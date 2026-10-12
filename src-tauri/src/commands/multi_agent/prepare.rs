#[cfg(test)]
fn multi_agent_prepare(context:&mut AgentRunContext)->Result<MultiAgentSession,String> {
    let root=context.run.as_ref().ok_or("multi_agent_root_run_missing")?.run_id.clone();
    if native_frozen_web_root_mode(context)?!=crate::agent_runtime::web_mode::WebMode::Multi {return Err("multi_agent_requires_frozen_multi_root".into());}
    let owner=crate::agent_runtime::multi_agent::parent_invocation_owner::ParentInvocationOwner::claim(
        &context.db_path,&context.scan_id,context.attempt_number,&root)?;
    multi_agent_prepare_owned(context,owner)
}

fn multi_agent_prepare_owned(context:&mut AgentRunContext,owner:crate::agent_runtime::multi_agent::parent_invocation_owner::ParentInvocationOwner)->Result<MultiAgentSession,String> {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{lease, mailbox, scheduler},
    };
    let root_run_id = context
        .run
        .as_ref()
        .map(|run| run.run_id.clone())
        .ok_or_else(|| "multi_agent_root_run_missing".to_string())?;
    owner.validate_scope(&context.db_path,&context.scan_id,context.attempt_number,&root_run_id)?;
    let connection = db::open(&context.db_path)?;
    if crate::agent_runtime::multi_agent::budget::root::model::tick::financial_purpose::binding_on(&connection,&root_run_id)?.is_some() {
        return Err("financial_continuation_general_prepare_denied".into());
    }
    if native_frozen_web_root_mode_on(&connection,context)?!=crate::agent_runtime::web_mode::WebMode::Multi {return Err("multi_agent_requires_frozen_multi_root".into());}
    native_web_original_finance_on(&connection,context)?;
    native_coordinator_frozen_evidence(&connection,context)?;
    // New lease contracts authenticate their original issuer before acquiring
    // or refreshing C, modifying Root projections, or rebinding summary fees.
    let preflight_issuer = {
        let tx = connection.unchecked_transaction().map_err(|e| e.to_string())?;
        let required = crate::agent_runtime::web_mode::root::read(&tx, &root_run_id)?
            .is_some_and(|d| d.lease_expiry_supervision().is_some());
        let sealed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_mode_birth_seals WHERE root_run_id=?1)", [&root_run_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if required || sealed {
            let original = tx.query_row(
                "SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_key=?4",
                params![root_run_id,context.scan_id,context.attempt_number,context.target_url],
                |r| Ok(lease::CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?}),
            ).map_err(|_| "worker_supervisor_original_coordinator_missing")?;
            if required {
                lease::validate_coordinator_lease(&tx, &original)?;
                lease::require_executable_coordinator(&tx, &original)?;
            }
            let issuer = original_withdrawal_issuer_on(&tx, &original)?;
            if required && issuer.is_none() { return Err("worker_supervisor_original_signed_issuer_required".into()); }
            issuer
        } else { None }
    };

    // The linked gap is an advisory objective, never an inherited execution
    // grant. Revalidate it before scheduling any child, not only at review.
    let followup = {
        let tx = connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        gap_followup_review_context(&tx, &root_run_id, &context.target_dir)?
    };
    if let Some(followup) = followup {
        context.evidence["followupObjective"] = serde_json::json!({
            "source":followup["source"],"historicalCandidate":followup["historicalCandidate"],
            "targetRequestsGranted":0,"instruction":"仅在本次新授权合同内补足原缺证项；历史建议不是权限，历史证据不是本次新事实。",
        });
    }
    // The scan policy is only a list of handles. Re-check task binding,
    // validity, distinct credentials and this target's scope before any
    // child is scheduled or any target-touching capability is issued.
    let (identity_mode, session_ids) = crate::auth_session::validated_scan_identities(
        &connection,
        &context.scan_id,
        &context.target_url,
    )?;
    let authenticated_identity_count = session_ids.len();
    context.identities = if session_ids.is_empty() {
        vec![AgentIdentity::anonymous()]
    } else {
        session_ids.into_iter().map(AgentIdentity::scoped).collect()
    };
    // Preserve the fixed credential-free control alongside every validated
    // account; it is neither another captured account nor an identity grant.
    if authenticated_identity_count > 0 {
        context.identities.push(AgentIdentity::anonymous());
    }
    let coordinator_lease = lease::acquire_coordinator_lease(
        &connection,
        &context.scan_id,
        context.attempt_number,
        &context.target_url,
        &root_run_id,
        600,
    )?;
    // A crashed legacy Deep Investigator (or a Web Executor from a newer
    // build) may already have sent target requests. The same attempt cannot
    // safely dispatch another executor under a different role/revision.
    scheduler::ensure_fresh_web_executor_attempt(&connection, &coordinator_lease)?;
    // Do not rebind the budget ledger or replay an old specialist under a
    // replacement fence. Received and uncertain calls require distinct
    // recovery contracts; neither grants the new Coordinator old authority.
    scheduler::ensure_fresh_readonly_fence(&connection, &coordinator_lease)?;
    if context.external_surface {
        let prior: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 \
             AND target_url=?3 AND role='external_surface')",
                params![context.scan_id, context.attempt_number, context.target_url],
                |r| r.get(0),
            )
            .map_err(|e| format!("public_surface_history:{e}"))?;
        if prior {
            return Err("public_surface_recovery_requires_fresh_attempt".into());
        }
    }
    connection
        .execute(
            "UPDATE agent_runs SET root_run_id=id,orchestration_policy='multi',lane='read_only_analysis',\
             capability_lease_json='[\"coordination.control\",\"evidence.read\",\"mailbox.read\",\"mailbox.write\"]',\
             lease_expires_at=?1,heartbeat_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?2",
            params![coordinator_lease.lease_expires_at, root_run_id],
        )
        .map_err(|error| format!("无法激活多智能体 Coordinator：{error}"))?;
    connection
        .execute(
            "INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,lease_epoch,fencing_token) \
             SELECT id,hard_token_budget,hard_request_budget,?1,?2 FROM agent_runs WHERE id=?3 \
             ON CONFLICT(root_run_id) DO UPDATE SET lease_epoch=excluded.lease_epoch,fencing_token=excluded.fencing_token,updated_at=datetime('now','localtime')",
            params![coordinator_lease.lease_epoch, coordinator_lease.fencing_token, root_run_id],
        )
        .map_err(|error| format!("无法初始化多智能体预算账本：{error}"))?;
    // DESIGN §0.1 / §2.2：任务级 scheduling_model 开关。取值只来自冻结扫描计划，
    // 默认 legacy_role_pipeline；只有显式声明 homogeneous_workers 的任务才建立同质任务，
    // 固定角色管线的既有行为与测试完全不变。
    let scheduling_model =
        crate::agent_runtime::multi_agent::homogeneous::store::scheduling_model_from_plan(
            &connection,
            &context.scan_id,
            context.attempt_number,
        )
        .map_err(|error| format!("无法判定调度模式：{error}"))?;
    crate::agent_runtime::multi_agent::homogeneous::store::set_run_scheduling_model(
        &connection,
        &root_run_id,
        scheduling_model,
    )
    .map_err(|error| format!("无法写入调度模式：{error}"))?;
    crate::agent_runtime::multi_agent::homogeneous::store::activate_task_for_lease(
        &connection,
        &coordinator_lease,
    )
    .map_err(|error| format!("无法激活同质 Worker 任务：{error}"))?;
    // Phase 1.1 让路（REVIEW §8.1 第 3 条）：显式声明 homogeneous_workers 的 root 不再进入
    // legacy bootstrap dispatch——固定角色 child、旧 lane 互斥与 bootstrap 预算一个都不建，
    // 切片只由主管轮（mint 出的 homogeneous-decision 帧 + `homogeneous::apply`）派发。
    // 默认 legacy_role_pipeline 的 root 走原路径，本分支一次都不执行。
    if scheduling_model
        == crate::agent_runtime::multi_agent::homogeneous::types::SCHEDULING_HOMOGENEOUS
    {
        return Err("homogeneous_scan_legacy_bootstrap_stepped_aside".into());
    }
    let issuer = match preflight_issuer {
        Some(issuer) => {
            let tx = connection.unchecked_transaction().map_err(|e| e.to_string())?;
            issuer.require_actor(&tx, &coordinator_lease)?;
            Some(issuer)
        }
        None => original_withdrawal_issuer_on(&connection, &coordinator_lease)?,
    };
    let supervisor = match issuer {
        Some(issuer) => crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor::start_owned_signed(&context.db_path, &coordinator_lease, owner, issuer)?,
        None => crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor::start_owned(&context.db_path, &coordinator_lease, owner)?,
    };
    context.supervision = Some(supervisor.ticket());
    let bootstrap = native_coordinator_bootstrap(context,&coordinator_lease)?;
    context.evidence["multiAgentCoordinator"]=json!({"decisionSummary":bootstrap.summary.as_json(),
        "eventSequence":bootstrap.event_sequence,"replayed":bootstrap.replayed,"advisoryOnly":true});
    let mapper_loop_bounds = if crate::agent_runtime::web_mode::root::read(&connection, &root_run_id)?
        .and_then(|mode| mode.bootstrap_dispatch())
        .is_some_and(|contract| contract["schemaVersion"] == 15)
    {
        let snapshot = connection.unchecked_transaction().map_err(|error| error.to_string())?;
        let inventory = mapper_evidence_lookup::MapperEvidenceInventory::capture_on(
            &snapshot, &coordinator_lease, &context.target_dir, 1,
            mapper_evidence_lookup::MapperLookupLimits::new(256 * 1024, 512)?,
        )?;
        Some(mapper_loop_request::VerifiedMapperLoopRequestBounds::capture_on(
            &snapshot, &coordinator_lease, &context.target_dir, inventory,
            &context.environment, context.proxy.as_deref(),
        )?)
    } else { None };
    let mapper = native_coordinator_prepare_mapper_with_loop_bounds(context,&coordinator_lease,&bootstrap,&json!({
        "target":context.target_url,
        "objective":"从冻结前端证据中整理 API、参数、认证线索和优先验证路径，不访问目标",
    }), mapper_loop_bounds.as_ref())?;
    // Derived projections are not new frontend evidence. Keep the paid Root's
    // stable public summary/reference, while replay diagnostics stay outside
    // the Mapper's original request. Never rewrite/adopt an old paid request.
    let mut mapper_evidence = context.evidence.clone();
    if let Some(evidence) = mapper_evidence.as_object_mut() {
        evidence.retain(|key, _| !key.starts_with("multiAgent") || key == "multiAgentCoordinator");
        if let Some(coordinator) = evidence
            .get_mut("multiAgentCoordinator")
            .and_then(JsonValue::as_object_mut)
        {
            coordinator.remove("replayed");
        }
    }
    let mapper_input = serde_json::json!({
        "target": context.target_url,
        "evidence": mapper_evidence,
        "capabilities": context.capabilities,
        "identityMode": format!("{identity_mode:?}"),
        "identityCount": authenticated_identity_count,
        "requiredOutput": {"summary":"string","priorityContracts":["string"],"risks":["string"]}
    });
    let mapper_result = if let Some(bounds) = &mapper_loop_bounds {
        mapper_executor_adapter::native_run_mapper_controlled_executor(
            context, &coordinator_lease, &mapper, bounds,
        ).map(|output| (output.text().to_owned(), *output.usage()))
    } else { multi_agent_child_round(
        context,
        &coordinator_lease,
        &mapper,
        "你是独立的 SPA/API Mapper。你只能分析提供的冻结证据，不能声称访问了目标。输出严格 JSON，不要 Markdown；列出证据支持的 API、参数、认证线索和建议优先合同。",
        mapper_input,
    ) };
    context.evidence["multiAgentMapper"] = match mapper_result {
        Ok((text, usage)) => deliver_readonly_assessment(
            &connection,
            &coordinator_lease,
            &mapper,
            &usage,
            &serde_json::json!({"summary":text}),
            None,
        )?,
        Err(error) => {
            if mapper_loop_bounds.is_some() {
                // Per-round invoices retain their original financial stage.
                // Legacy specialist cleanup cannot reconcile or refund them.
                return Err(error);
            }
            return Err(failed_specialist_error(
                &connection,
                &coordinator_lease,
                &mapper,
                &error,
            ));
        }
    };
    // Original delivery has committed. Root assessment must not enter its
    // child cleanup/retry closure or the Supervisor's protected writer.
    if crate::agent_runtime::multi_agent::attempts::current(&connection, &coordinator_lease, &mapper.assignment_id)?.state == "expired"
        && crate::agent_runtime::web_mode::root::read(&connection, &root_run_id)?.is_some_and(|d| d.lease_expiry_supervision().is_some()) {
        native_coordinator_lease_expiry_feedback(context, &coordinator_lease, &mapper)?;
    }
    if identity_mode != crate::auth_session::ScanIdentityMode::AnonymousOnly {
        supervisor.check()?;
        let identity_child = scheduler::prepare_supervised_readonly_child(
            &connection,
            &coordinator_lease,
            AgentRole::IdentitySession,
            "validated_identity_metadata_ready",
            &serde_json::json!({
                "identityMode": format!("{identity_mode:?}"),
                "identityCount": authenticated_identity_count,
                "mapperAssignmentId": mapper.assignment_id,
                "objective": "只从冻结证据分析认证状态、身份隔离线索和缺口；不访问目标或判定越权",
            }),
            4_000,
        )?;
        let identity_result = multi_agent_child_round(
            context,
            &coordinator_lease,
            &identity_child,
            "你是独立的 IdentitySession 分析专家。只从冻结证据与已验证的身份数量推断认证线索和缺口；不得访问目标，不得声称双身份对照已执行或越权漏洞已确认。严格输出 JSON。",
            serde_json::json!({
                "target": context.target_url,
                "identityMode": format!("{identity_mode:?}"),
                "identityCount": authenticated_identity_count,
                "mapperSummary": context.evidence.get("multiAgentMapper"),
                "requiredOutput": {"summary":"string","observedAuthentication":["string"],"evidenceGaps":["string"],"authorizationProven":false},
            }),
        );
        context.evidence["multiAgentIdentitySession"] = match identity_result {
            Ok((text, usage)) => deliver_readonly_assessment(
                &connection,
                &coordinator_lease,
                &identity_child,
                &usage,
                &serde_json::json!({"summary":text,"identityMode":format!("{identity_mode:?}")}),
                None,
            )?,
            Err(error) => {
                return Err(failed_specialist_error(
                    &connection,
                    &coordinator_lease,
                    &identity_child,
                    &error,
                ));
            }
        };
        native_coordinator_original_expired_feedback(context,&coordinator_lease,&identity_child)?;
        // This is a distinct paid, closed worker fact, not Mapper's opinion or
        // a target authorization. Rust checks metadata again before Web grant.
        let feedback=native_coordinator_identity_feedback(context,&coordinator_lease,&mapper,&identity_child)?;
        context.evidence["multiAgentIdentityCoordinator"]=json!({"decisionSummary":feedback.summary.as_json(),
            "eventSequence":feedback.event_sequence,"replayed":feedback.replayed,"advisoryOnly":true,
            "targetRequestsGranted":0,"authorizationProven":false});
    }
    supervisor.check()?;
    if context.external_surface {
        context.evidence["multiAgentExternalSurface"] =
            multi_agent_external_surface(context, &coordinator_lease)?;
    }
    let (coordinator_frame,coordinator_decision)=native_coordinator_mapper_decision(context,&coordinator_lease,&mapper)?;
    context.evidence["multiAgentCoordinatorHandoff"]=json!({"decisionSummary":coordinator_decision.summary.as_json(),
        "eventSequence":coordinator_decision.event_sequence,"replayed":coordinator_decision.replayed});
    let executor_revision: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(evidence_revision),0)+1 FROM agent_assignments WHERE coordinator_run_id=?1 AND role='web_executor'",
            [&root_run_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法分配执行专家 revision：{error}"))?;
    let (remaining_tokens,remaining_requests)=crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(&connection,&root_run_id)?;
    let total_tokens=remaining_tokens.unwrap_or(0);
    let total_requests=remaining_requests.unwrap_or(0);
    let executor_tokens=remaining_tokens.map(|n|n.saturating_sub(8_000)).unwrap_or(0);
    let executor_requests=remaining_requests.map(|n|n.saturating_sub(1)).unwrap_or(0);
    if remaining_tokens.is_some_and(|n|n<=8_000) || remaining_requests.is_some_and(|n|n<=1) {return Err("multi_agent_executor_budget_unavailable".into());}
    if (total_tokens > 0 && executor_tokens <= 0) || (total_requests > 0 && executor_requests <= 0)
    {
        return Err("multi_agent_executor_budget_unavailable".into());
    }
    // Partition the original pool for two ordered assessments when it leaves
    // at least the previous 4000-token/one-request executor window. Smaller
    // pools retain the existing singleton reservation or executor-only split.
    let proposal_requests = if (total_tokens == 0 || executor_tokens >= 12_000)
        && (total_requests == 0 || executor_requests >= 3) {
        2
    } else if (total_tokens == 0 || executor_tokens >= 8_000)
        && (total_requests == 0 || executor_requests >= 2) {
        1
    } else {
        0
    };
    let executor_tokens = if total_tokens > 0 {
        executor_tokens - proposal_requests
            * crate::agent_runtime::multi_agent::directive::proposals::PROPOSAL_TOKENS
    } else {
        executor_tokens
    };
    let executor_requests = if total_requests > 0 {
        executor_requests - proposal_requests
    } else {
        executor_requests
    };
    let executor_capabilities = agent_tool_specs_for(context)
        .into_iter()
        .map(|spec| spec.name.to_string())
        .collect::<Vec<_>>();
    supervisor.check()?;
    let executor = native_coordinator_schedule_decision(
        context,
        &coordinator_lease,
        &coordinator_frame,
        &coordinator_decision,
        AgentRole::WebExecutor,
        AgentLane::TargetTouching,
        "mapper_handoff_ready",
        &serde_json::json!({
            "target": context.target_url,
            "objective": "在授权范围内执行冻结计划中的验证合同，所有目标访问都必须经过工具 Broker",
            "mapperAssignmentId": mapper.assignment_id,
        }),
        executor_revision,
        &executor_capabilities,
        executor_tokens,
        executor_requests,
    )?;
    // The creation-only dynamic rule was evaluated under the issuance lock.
    // Bind the runtime window to its immutable first-worker grant, not a prior estimate.
    let mode=crate::agent_runtime::web_mode::root::read(&connection,&root_run_id)?.ok_or("web_mode_root_missing")?;
    let (executor_tokens,executor_requests)=if mode.bootstrap_dispatch().unwrap_or(JsonValue::Null).get("executorAllocationRule").is_some() {
        crate::agent_runtime::multi_agent::budget::model::original_web_grant(&connection,&coordinator_lease,&executor.assignment_id)?
    } else {(executor_tokens,executor_requests)};
    scheduler::start_child_or_release(&connection, &coordinator_lease, &executor)?;
    let assignment_correlation = format!("executor:{}", executor.assignment_id);
    let handoff = (|| -> Result<(), String> {
        let assignment_message_id = mailbox::send(
            &connection,
            &coordinator_lease,
            &root_run_id,
            &executor.run_id,
            AgentRole::Coordinator.as_str(),
            AgentRole::WebExecutor.as_str(),
            "execution_assignment",
            &assignment_correlation,
            &executor.assignment_id,
            executor_revision,
            &serde_json::json!({
                "target": context.target_url,
                "mapperSummary": context.evidence.get("multiAgentMapper"),
                "capabilities": executor_capabilities,
                "publicSurface": context.evidence.get("multiAgentExternalSurface"),
            }),
        )?;
        receive_expected_child_message(
            &connection,
            &coordinator_lease,
            &executor.run_id,
            &assignment_message_id,
            "execution_assignment",
        )?;
        Ok(())
    })();
    if let Err(error) = handoff {
        if let Err(cleanup) =
            scheduler::finish_child(&connection, &coordinator_lease, &executor, false, &error)
        {
            return Err(format!("{error};executor_handoff_cleanup:{cleanup}"));
        }
        return Err(error);
    }
    let executor_start_usage =
        NativeAgentState::read(&context.db_path, &context.scan_id, &context.target_url)
            .map(|state| state.token_usage)
            .unwrap_or_default();
    context.run = Some(AgentRunLedger {
        db_path: context.db_path.clone(),
        run_id: executor.run_id.clone(),
    });
    context.run_budget = Some(AgentRunBudgetWindow {
        starting_tokens: executor_start_usage.total_tokens.max(0),
        starting_requests: executor_start_usage.model_requests.max(0),
        hard_tokens: executor_tokens,
        hard_requests: executor_requests,
    });
    supervisor.check()?;
    Ok(MultiAgentSession {
        supervisor,
        lease: coordinator_lease,
        mapper,
        executor,
    })
}
