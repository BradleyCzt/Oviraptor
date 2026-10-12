fn multi_agent_finish_execution(
    context: &AgentRunContext,
    session: &mut MultiAgentSession,
    outcome: &AgentTargetOutcome,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::scheduler;
    let connection = db::open(&context.db_path)?;
    let original_supervision =
        crate::agent_runtime::web_mode::root::read(&connection, &session.lease.root_run_id)?
            .is_some_and(|mode| mode.new_evidence_supervision().is_some());
    let capability_supervision =
        crate::agent_runtime::web_mode::root::read(&connection, &session.lease.root_run_id)?
            .is_some_and(|mode| mode.bootstrap_dispatch().is_some_and(|b|
                b["capabilityTransitionRule"] == crate::agent_runtime::web_mode::capability_transition_receipt::RULE));
    if !original_supervision {
        session.lease = crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
            &connection,
            &context.scan_id,
            context.attempt_number,
            &context.target_url,
            &session.lease.root_run_id,
            600,
        )?;
    }
    // Expired Web workers can deliver original fees, never a successful result.
    let expired_web: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts WHERE assignment_id=?1 AND child_run_id=?2 AND state='expired' AND failure_class='worker_lease_expired')",
        params![session.executor.assignment_id, session.executor.run_id], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if original_supervision && expired_web {
        let tx = rusqlite::Transaction::new_unchecked(
            &connection, rusqlite::TransactionBehavior::Immediate,
        ).map_err(|e| e.to_string())?;
        if native_coordinator_web_expiry_publication_replay_on(
            &tx, &session.lease, &session.executor,
        )?.is_some() {
            return Ok(());
        }
        session.supervisor.check()?;
        let issuer = original_withdrawal_issuer_on(&tx, &session.lease)?
            .ok_or("web_expiry_original_issuer_required")?;
        let financial = crate::agent_runtime::multi_agent::budget::model::expired_web_receipt::settle_expired_web_receipt_on(
            &tx, &session.lease, &session.executor, &issuer,
        )?;
        crate::agent_runtime::web_mode::web_expiry_delivery::issue_on(
            &tx, &session.lease, &session.executor, &issuer, &financial,
        )?;
        tx.commit().map_err(|e| e.to_string())?;
        session.supervisor.check()?;
        return native_coordinator_web_expiry_feedback(
            context, &session.lease, &session.executor, &financial,
        );
    }
    let settlement = (|| -> Result<AgentTokenUsage, String> {
        let tx = rusqlite::Transaction::new_unchecked(
            &connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(|e| e.to_string())?;
        let (usage, _original_idle) = if original_supervision {
            settle_original_web_child_usage(&tx, &session.lease, &session.executor)?
        } else {
            let usage =
                crate::agent_runtime::multi_agent::budget::web_lifetime::child_usage_original(
                    &tx,
                    &session.lease,
                    &session.executor.assignment_id,
                    &session.executor.run_id,
                )?;
            settle_child_usage_in_transaction(&tx, &session.lease, &session.executor, &usage)?;
            (usage, vec![])
        };
        tx.commit().map_err(|e| e.to_string())?;
        Ok(usage)
    })();
    let usage = match settlement {
        Ok(usage) => usage,
        Err(error) => {
            if let Err(cleanup) = stop_failed_child_preserving_usage(
                &connection,
                &session.lease,
                &session.executor,
                &error,
            ) {
                return Err(format!("{error};executor_child_cleanup:{cleanup}"));
            }
            return Err(error);
        }
    };
    // Financial settlement above remains valid even if supervision failed.
    // A stopped supervisor cannot authorize subsequent business publication.
    session.supervisor.check()?;
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(
        &connection,
        &session.lease,
    )?;
    crate::agent_runtime::multi_agent::lease::require_executable_coordinator(
        &connection,
        &session.lease,
    )?;
    let success = matches!(
        outcome,
        AgentTargetOutcome::Completed(_)
            | AgentTargetOutcome::BoundedCompleted(_)
            | AgentTargetOutcome::Incomplete(_)
            | AgentTargetOutcome::Limited(_)
    );
    let correlation_id = format!("executor-result:{}", session.executor.assignment_id);
    let payload = serde_json::json!({
        "terminalCode": outcome.terminal_code(), "summary": outcome.detail(),
        "usage": { "totalTokens": usage.total_tokens, "modelRequests": usage.model_requests }
    });
    native_coordinator_budget_consumption_after_settlement(
        context,
        &session.lease,
        &session.executor,
        &payload,
        &outcome.detail(),
    )
    .map_err(|error| format!("executor_followup_failed:budget_consumption:{error}"))?;
    native_coordinator_http_budget_consumption_after_settlement(
        context, &session.lease, &session.executor, &payload, &outcome.detail(),
    ).map_err(|error| format!("executor_followup_failed:http_budget_consumption:{error}"))?;
    if success
        && original_executor_finished_replay(
            &connection,
            &session.lease,
            &session.executor,
            &payload,
            &outcome.detail(),
        )?
    {
        if capability_supervision {
            native_coordinator_capability_transition_feedback(context, &session.lease, &session.executor)?;
        }
        native_verify_delivered_protection_source(&connection, context, &session.lease, outcome)?;
        multi_agent_protection_supervision(context, session, outcome)?;
        if !outcome_requires_manual_execution_resolution(outcome) {
            multi_agent_new_evidence_supervision(context, session)
                .map_err(|error| format!("executor_followup_failed:new_evidence:{error}"))?;
            multi_agent_client_side_readonly(context, session)
                .map_err(|error| format!("executor_followup_failed:client_side:{error}"))?;
            multi_agent_input_parser(context, session)
                .map_err(|error| format!("executor_followup_failed:input_parser:{error}"))?;
            multi_agent_business_logic_followup(context, session)
                .map_err(|error| format!("executor_followup_failed:business_logic:{error}"))?;
        }
        return Ok(());
    }
    let finalize = complete_target_child_delivery(
        &connection,
        &session.lease,
        &session.executor,
        "execution_result",
        &correlation_id,
        &payload,
        success,
        &outcome.detail(),
    );
    let _capability_idle = match finalize {
        Ok(guard) => guard,
        Err(error) => {
        if let Err(cleanup) = scheduler::finish_child(
            &connection,
            &session.lease,
            &session.executor,
            false,
            &error,
        ) {
            return Err(format!("{error};executor_child_cleanup:{cleanup}"));
        }
        return Err(error);
        }
    };
    if success {
        if capability_supervision {
            native_coordinator_capability_transition_feedback(context, &session.lease, &session.executor)?;
        }
        native_verify_delivered_protection_source(&connection, context, &session.lease, outcome)?;
        multi_agent_protection_supervision(context, session, outcome)?;
    }
    if success && !outcome_requires_manual_execution_resolution(outcome) {
        multi_agent_new_evidence_supervision(context, session)
            .map_err(|error| format!("executor_followup_failed:new_evidence:{error}"))?;
        multi_agent_client_side_readonly(context, session)
            .map_err(|error| format!("executor_followup_failed:client_side:{error}"))?;
        multi_agent_input_parser(context, session)
            .map_err(|error| format!("executor_followup_failed:input_parser:{error}"))?;
        multi_agent_business_logic_followup(context, session)
            .map_err(|error| format!("executor_followup_failed:business_logic:{error}"))?;
    }
    Ok(())
}

/// A separately leased, target-touching Authorization child runs only when an
/// operator prepared an immutable control group for this exact attempt. Its
/// three GETs are claimed individually by the broker before network I/O.
fn multi_agent_authorization(
    context: &AgentRunContext,
    session: &mut MultiAgentSession,
) -> Result<(), String> {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let connection = db::open(&context.db_path)?;
    session.supervisor.check()?;
    let controls = load_authorization_controls(&connection, context)?;
    if controls.is_empty() {
        return Ok(());
    }
    if controls.len() > 4 {
        return Err("authorization_control_group_limit_exceeded".into());
    }
    if crate::agent_runtime::web_mode::root::read(&connection, &session.lease.root_run_id)?
        .is_some_and(|mode| mode.bootstrap_dispatch().is_some_and(|b|
            b["authorizationOutputRule"] == authorization_output_receipt::RULE)) {
        let saved: Option<(String, String, String)> = connection.query_row(
            "SELECT id,child_run_id,task_slice_json FROM agent_assignments WHERE coordinator_run_id=?1 AND role='authorization' AND trigger_code='operator_authorization_control_ready' AND evidence_revision=1 AND state='completed'",
            [&session.lease.root_run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional().map_err(|e| e.to_string())?;
        if let Some((assignment_id, run_id, raw)) = saved {
            let task: JsonValue = serde_json::from_str(&raw).map_err(|_| "authorization_output_original_task_changed")?;
            let mut original_keys = task["contractKeys"].as_array().ok_or("authorization_output_original_task_changed")?
                .iter().map(|v| v.as_str().map(str::to_owned).ok_or("authorization_output_original_task_changed"))
                .collect::<Result<Vec<_>, _>>()?;
            let mut current_keys = controls.iter().map(|c| c.contract_key.clone()).collect::<Vec<_>>();
            original_keys.sort(); current_keys.sort();
            if original_keys != current_keys { return Err("authorization_output_original_task_changed".into()); }
            let child = scheduler::ScheduledChild { assignment_id, run_id, role: AgentRole::Authorization };
            native_coordinator_authorization_output_feedback(context, &session.lease, &child)?;
            return Ok(());
        }
    }
    // Verify the original birth before any optional protocol can select a
    // legacy renewal. Ordinary supervised Roots retain their original C.
    original_withdrawal_issuer_on(&connection, &session.lease)?;
    let supervised =
        crate::agent_runtime::web_mode::root::read(&connection, &session.lease.root_run_id)?
            .is_some_and(|mode| mode.lease_expiry_supervision().is_some());
    if supervised {
        let root = native_coordinator_root_context(context, &session.lease);
        native_coordinator_tick_authority(&connection, &root, &session.lease)?;
    } else {
        session.lease = crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
            &connection,
            &context.scan_id,
            context.attempt_number,
            &context.target_url,
            &session.lease.root_run_id,
            600,
        )?;
    }
    let child = scheduler::schedule_child(
        &connection,
        &session.lease,
        AgentRole::Authorization,
        AgentLane::TargetTouching,
        "operator_authorization_control_ready",
        &serde_json::json!({"contractKeys":controls.iter().map(|row| &row.contract_key).collect::<Vec<_>>(),
            "objective":"仅执行已登记的 A→X、B→X、B→Y GET 控制组"}),
        1,
        &["authorization_probe".into()],
        0,
        0,
    )?;
    scheduler::start_child_or_release(&connection, &session.lease, &child)?;
    let mut child_context = context.clone();
    child_context.run = Some(AgentRunLedger {
        db_path: context.db_path.clone(),
        run_id: child.run_id.clone(),
    });
    let result = (|| -> Result<usize, String> {
        let mut verified = 0;
        for control in &controls {
            session.supervisor.check()?;
            authorization_control_urls(control)?;
            let (owner, owner_artifact) = execute_authorization_side(
                &child_context,
                &child,
                control,
                "owner",
                &control.owner_identity,
                &control.owner_object_url,
            )?;
            let (cross, cross_artifact) = execute_authorization_side(
                &child_context,
                &child,
                control,
                "cross",
                &control.tester_identity,
                &control.owner_object_url,
            )?;
            let (tester, tester_artifact) = execute_authorization_side(
                &child_context,
                &child,
                control,
                "tester",
                &control.tester_identity,
                &control.tester_control_url,
            )?;
            if agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number)
                .is_cancelled()
            {
                return Err("authorization_attempt_cancelled".into());
            }
            match verify_authorization_control_group(control, [&owner, &cross, &tester]) {
                Ok(()) => {
                    let key = format!(
                        "authorization-control:{}",
                        &crate::agent_runtime::store::stable_hash(&format!(
                            "{}:{}:{}",
                            context.scan_id, context.attempt_number, control.contract_key
                        ))[..24]
                    );
                    stage_agent_finding_with_active_guard(
                        &child_context,
                        AGENT_VULNERABILITY_STAGE,
                        "vulnerability",
                        &key,
                        "跨账号访问已登记的他人对象",
                        "high",
                        &serde_json::json!({
                            "kind":"authorization_cross_object_read", "contractKey":control.contract_key,
                            "verification":"operator_bound_three_side_native_control",
                            "responseObjectPointer":control.response_object_pointer,
                            "method":"GET", "sides":[
                                {"name":"owner", "artifactId":owner_artifact},
                                {"name":"cross", "artifactId":cross_artifact},
                                {"name":"tester", "artifactId":tester_artifact},
                            ],
                            "attemptNumber":context.attempt_number,
                        }),
                        true,
                    )?;
                    verified += 1;
                }
                Err(code) => {
                    append_runner_log(
                        &context.log_path,
                        &format!(
                            "authorization control {} insufficient: {code}",
                            control.contract_key
                        ),
                    );
                }
            }
        }
        Ok(verified)
    })();
    let summary = match &result {
        Ok(count) => format!("authorization controls verified {count}/{}", controls.len()),
        Err(code) => format!("authorization control incomplete: {code}"),
    };
    // Once running, even a usage query or mailbox failure must attempt a
    // fenced terminal transition. Otherwise the target lane and reserved
    // request budget remain occupied after the coordinator has failed.
    let finalization = (|| -> Result<(), String> {
        let spent: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE child_run_id=?1",
                [&child.run_id],
                |row| row.get(0),
            )
            .map_err(|_| "authorization_usage_unavailable".to_string())?;
        let usage = AgentTokenUsage::default();
        settle_child_usage(&connection, &session.lease, &child, &usage)?;
        complete_target_child_delivery_on(
            &connection, &session.lease, &child,
            TargetChildDeliveryMessage {
                kind: "authorization_result",
                correlation_id: &format!("authorization:{}", child.assignment_id),
                payload: &serde_json::json!({"summary":summary,"verified":result.as_ref().ok(),"requestCount":spent}),
                success: result.is_ok(), summary: &summary,
            }, Some(&context.target_dir),
        ).map(|_| ())
    })();
    if let Err(error) = finalization {
        if let Err(cleanup) =
            stop_failed_child_preserving_usage(&connection, &session.lease, &child, &error)
        {
            return Err(format!("{error};authorization_child_cleanup:{cleanup}"));
        }
        return Err(error);
    }
    if result.is_ok() && crate::agent_runtime::web_mode::root::read(&connection, &session.lease.root_run_id)?
        .is_some_and(|mode| mode.bootstrap_dispatch().is_some_and(|b|
            b["authorizationOutputRule"] == authorization_output_receipt::RULE)) {
        native_coordinator_authorization_output_feedback(context, &session.lease, &child)?;
    }
    result.map(|_| ())
}
