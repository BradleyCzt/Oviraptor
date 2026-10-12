fn multi_agent_review(
    context: &AgentRunContext,
    session: &mut MultiAgentSession,
    outcome: AgentTargetOutcome,
) -> AgentTargetOutcome {
    let execution_stop =
        outcome_requires_manual_execution_resolution(&outcome).then(|| outcome.clone());
    let reviewed = multi_agent_review_inner(context, session, outcome);
    if let Some(stop) = execution_stop {
        if matches!(
            reviewed,
            AgentTargetOutcome::Completed(_)
                | AgentTargetOutcome::BoundedCompleted(_)
                | AgentTargetOutcome::Incomplete(_)
                | AgentTargetOutcome::Limited(_)
        ) {
            // Reviewer decisions still persist independently, but reviewing
            // evidence cannot settle unknown request effects or renew a grant.
            return stop;
        }
    }
    reviewed
}

fn outcome_requires_manual_execution_resolution(outcome: &AgentTargetOutcome) -> bool {
    matches!(
        outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
            | terminal_code::EXECUTION_AUTHORIZATION_DENIED
    )
}

fn multi_agent_review_inner(
    context: &AgentRunContext,
    session: &mut MultiAgentSession,
    outcome: AgentTargetOutcome,
) -> AgentTargetOutcome {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
        store,
    };
    if let Err(error) = session.supervisor.check() {
        // The original supervisor rejects an inactive attempt before any
        // Reviewer work. Preserve the same cancellation outcome as the later
        // original authority guard; no other supervisor failure is remapped.
        if error == "agent_attempt_not_active" {
            return AgentTargetOutcome::Cancelled;
        }
        return AgentTargetOutcome::failed(format!("review_supervisor:{error}"));
    }
    let connection = match db::open(&context.db_path) {
        Ok(connection) => connection,
        Err(error) => {
            return AgentTargetOutcome::failed(format!("review_gate_persistence:{error}"))
        }
    };
    // Verify any original birth seal before a missing optional field can select
    // the legacy renewal path. Reading a declaration alone is not provenance.
    if let Err(error) = original_withdrawal_issuer_on(&connection, &session.lease) {
        return AgentTargetOutcome::failed(format!("review_gate_birth:{error}"));
    }
    let original_supervision = match crate::agent_runtime::web_mode::root::read(
        &connection, &session.lease.root_run_id,
    ) {
        Ok(mode) => mode.is_some_and(|mode| mode.lease_expiry_supervision().is_some()),
        Err(error) => return AgentTargetOutcome::failed(format!("review_gate_birth:{error}")),
    };
    if original_supervision {
        // This Root's supervision contract cannot adopt or renew a C. Review
        // starts only with the original execution authority still valid.
        let root = native_coordinator_root_context(context, &session.lease);
        if let Err(error) = native_coordinator_tick_authority(&connection, &root, &session.lease) {
            // The original authority guard still runs in full. A paused or
            // cancelled attempt must retain its cancellation outcome rather
            // than becoming a review failure; neither outcome permits work.
            if error == "agent_attempt_not_active" {
                return AgentTargetOutcome::Cancelled;
            }
            return AgentTargetOutcome::failed(format!("review_gate_lease:{error}"));
        }
    } else {
    session.lease = match crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
        &connection,
        &context.scan_id,
        context.attempt_number,
        &context.target_url,
        &session.lease.root_run_id,
        600,
    ) {
        Ok(lease) => lease,
        Err(error) => return AgentTargetOutcome::failed(format!("review_gate_lease:{error}")),
    };
    }
    match review_attempt_active(&connection, &session.lease) {
        Ok(true) => {}
        Ok(false) => return AgentTargetOutcome::Cancelled,
        Err(error) => return AgentTargetOutcome::failed(format!("review_gate_status:{error}")),
    }
    let finding_candidates =
        match pending_agent_finding_candidates(&connection, &session.lease.root_run_id) {
            Ok(candidates) => candidates,
            Err(error) => {
                return AgentTargetOutcome::failed(format!("review_gate_candidates:{error}"))
            }
        };
    let has_pending_findings = !finding_candidates.is_empty();
    let persisted_fact_refs =
        match review_fact_refs(&connection, &session.lease.root_run_id, &context.target_dir) {
            Ok(refs) => refs,
            Err(error) => return AgentTargetOutcome::failed(format!("review_gate_facts:{error}")),
        };
    let mut candidate = crate::agent_runtime::secrets::redact_json(&serde_json::json!({
        "target": context.target_url,
        "outcome": outcome.detail(),
        "terminalCode": outcome.terminal_code(),
        "completion": outcome.completion().map(|completion| serde_json::json!({
            "coveredFamilies": completion.covered_families,
            "uncoveredFamilies": completion.uncovered_families,
            "confirmedFindings": completion.confirmed_findings,
            "verifiedToolResults": completion.verified_tool_results,
        })),
        "evidence": context.evidence,
        "persistedFactRefs": persisted_fact_refs,
        "findingCandidates": finding_candidates,
    }));
    let original_authorization=(|| -> Result<(),String> {
        let tx=connection.unchecked_transaction().map_err(|e|e.to_string())?;
        native_authorization_original_candidate_on(&tx,&session.lease,&context.target_dir,&candidate)?;
        Ok(())
    })();
    if let Err(error)=original_authorization { return AgentTargetOutcome::failed(format!("review_gate_authorization_original:{error}")); }
    let followup = (|| -> Result<Option<JsonValue>, String> {
        let tx = connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        gap_followup_review_context(&tx, &session.lease.root_run_id, &context.target_dir)
    })();
    let followup = match followup {
        Ok(value) => value,
        Err(error) => return AgentTargetOutcome::incomplete(format!("review_gap_source:{error}")),
    };
    let reviews_followup = followup.is_some();
    if let Some(followup) = followup {
        candidate["gapFollowup"] = followup;
    }
    let candidate_id = format!(
        "candidate-{}",
        &store::stable_hash(&format!(
            "{}:{}",
            session.lease.root_run_id, context.target_url
        ))[..24]
    );
    let candidate_text = candidate.to_string();
    let prior: Option<(i64, String, String)> = match connection
        .query_row(
            "SELECT candidate_revision,candidate_json,status FROM agent_review_requests WHERE root_run_id=?1 AND candidate_id=?2 ORDER BY candidate_revision DESC LIMIT 1",
            params![session.lease.root_run_id, candidate_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
    {
        Ok(prior) => prior,
        Err(error) => return AgentTargetOutcome::failed(format!("review_gate_revision_read:{error}")),
    };
    if !has_pending_findings && prior.is_none() && !reviews_followup {
        let any_findings: bool = match connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_finding_candidates WHERE root_run_id=?1)",
            [&session.lease.root_run_id],
            |row| row.get(0),
        ) {
            Ok(value) => value,
            Err(error) => {
                return AgentTargetOutcome::failed(format!("review_gate_candidates:{error}"))
            }
        };
        if any_findings {
            return AgentTargetOutcome::failed("review_gate_orphaned_findings_without_request");
        }
        // A coverage-only close has no CandidateFinding to review. Its
        // terminal state remains the executor/coverage reducer's decision;
        // do not spend a model call to manufacture a confirmed verdict.
        // Overall coverage is not a receipt for individual human actions.
        // Unresolved directives close atomically with the Coordinator later.
        return outcome;
    }
    let revision = match prior {
        Some((revision, previous, _)) if previous == candidate_text => {
            return match replay_frozen_review(
                context,
                session,
                &candidate_id,
                revision,
                &candidate_text,
                outcome,
            ) {
                Ok(outcome) => outcome,
                Err(error) => AgentTargetOutcome::incomplete(format!("review_gate_replay:{error}")),
            };
        }
        Some((revision, previous, _))
            if candidate_diff_only_settled_findings(&previous, &candidate) =>
        {
            return match replay_frozen_review(
                context,
                session,
                &candidate_id,
                revision,
                &previous,
                outcome,
            ) {
                Ok(outcome) => outcome,
                Err(error) => AgentTargetOutcome::incomplete(format!("review_gate_replay:{error}")),
            };
        }
        Some((revision, previous, status))
            if status == "insufficient_evidence" && !has_new_review_fact(&previous, &candidate) =>
        {
            return match replay_frozen_review(
                context,
                session,
                &candidate_id,
                revision,
                &previous,
                outcome,
            ) {
                Ok(outcome) => outcome,
                Err(error) => AgentTargetOutcome::incomplete(format!("review_gate_replay:{error}")),
            };
        }
        Some((revision, _, _)) => revision.saturating_add(1),
        None => 1,
    };
    let reviewer = match scheduler::schedule_child(
        &connection,
        &session.lease,
        AgentRole::EvidenceReviewer,
        AgentLane::Review,
        "candidate_ready",
        &serde_json::json!({"candidateId":candidate_id,"candidateRevision":revision}),
        revision,
        &["evidence.read".into(), "review.write".into()],
        8_000,
        1,
    ) {
        Ok(child) => child,
        Err(error) => return AgentTargetOutcome::failed(format!("review_gate_schedule:{error}")),
    };
    if let Err(error) = scheduler::start_child_or_release(&connection, &session.lease, &reviewer) {
        return AgentTargetOutcome::failed(format!("review_gate_start:{error}"));
    }
    let request_id = format!("review-{}-{revision}", candidate_id);
    if let Err(error) = open_review_request(
        &connection,
        &session.lease,
        &reviewer,
        &request_id,
        &candidate_id,
        revision,
        &candidate_text,
        &context.target_dir,
    ) {
        let cleanup =
            scheduler::finish_child(&connection, &session.lease, &reviewer, false, &error);
        return AgentTargetOutcome::failed(match cleanup {
            Ok(()) => format!("review_gate_persistence:{error}"),
            Err(cleanup_error) => {
                format!("review_gate_persistence:{error};cleanup:{cleanup_error}")
            }
        });
    }
    let review_round = multi_agent_child_round(
        context,
        &session.lease,
        &reviewer,
        if reviews_followup {
            GAP_FOLLOWUP_REVIEW_PROMPT
        } else {
            "你是独立 Evidence Reviewer。你没有网络、浏览器、上传或目标访问能力，只能审查冻结 CandidateBundle。输出严格 JSON：{\"verdict\":\"confirmed|rejected|insufficient_evidence\",\"reasonCodes\":[\"...\"],\"missingEvidence\":[\"...\"],\"confidence\":0.0,\"summary\":\"...\"}。没有可验证工具证据时不得 confirmed。"
        },
        candidate.clone(),
    );
    let (review_text, usage) = match review_round {
        Ok(value) => value,
        Err(error) => {
            if let Err(cleanup_error) =
                finish_failed_review(&connection, &session.lease, &reviewer, &request_id, &error)
            {
                return AgentTargetOutcome::failed(format!(
                    "review_gate_failed:{error};cleanup:{cleanup_error}"
                ));
            }
            return AgentTargetOutcome::incomplete(format!("review_gate_failed:{error}"));
        }
    };
    if let Err(error) = settle_child_usage(&connection, &session.lease, &reviewer, &usage) {
        if let Err(cleanup_error) =
            finish_failed_review(&connection, &session.lease, &reviewer, &request_id, &error)
        {
            return AgentTargetOutcome::failed(format!("{error};cleanup:{cleanup_error}"));
        }
        if let Err(recovery_error) = recover_received_review(
            &connection,
            &session.lease,
            &candidate_id,
            revision,
            &candidate_text,
            &context.target_dir,
        ) {
            return AgentTargetOutcome::failed(format!("{error};recovery:{recovery_error}"));
        }
        return match replay_frozen_review(
            context,
            session,
            &candidate_id,
            revision,
            &candidate_text,
            outcome,
        ) {
            Ok(outcome) => outcome,
            Err(replay_error) => {
                AgentTargetOutcome::incomplete(format!("review_gate_replay:{replay_error}"))
            }
        };
    }
    let decision = match validated_review_decision(&review_text) {
        Ok(decision) => decision,
        Err(error) => {
            if let Err(cleanup_error) =
                finish_failed_review(&connection, &session.lease, &reviewer, &request_id, &error)
            {
                return AgentTargetOutcome::failed(format!(
                    "review_gate_invalid_decision:{error};cleanup:{cleanup_error}"
                ));
            }
            return AgentTargetOutcome::incomplete(format!("review_gate_invalid_decision:{error}"));
        }
    };
    let verdict = decision.verdict.as_str();
    let summary = decision.summary.as_str();
    let missing_evidence = &decision.missing_evidence;
    if let Err(error) = complete_review_delivery(
        &connection,
        &session.lease,
        &reviewer,
        &request_id,
        &candidate_id,
        revision,
        &decision,
        &context.target_dir,
    ) {
        if let Err(cleanup_error) =
            finish_failed_review(&connection, &session.lease, &reviewer, &request_id, &error)
        {
            return AgentTargetOutcome::failed(format!("{error};cleanup:{cleanup_error}"));
        }
        if let Err(recovery_error) = recover_received_review(
            &connection,
            &session.lease,
            &candidate_id,
            revision,
            &candidate_text,
            &context.target_dir,
        ) {
            return AgentTargetOutcome::failed(format!("{error};recovery:{recovery_error}"));
        }
    }
    if let Err(error) = native_coordinator_original_review_expired_feedback(
        context, &session.lease, &candidate_id, revision,
    ) {
        return AgentTargetOutcome::incomplete(format!("review_lease_supervision:{error}"));
    }
    let success = verdict == "confirmed";
    let gap_error = if verdict == "insufficient_evidence"
        && !outcome_requires_manual_execution_resolution(&outcome)
    {
        multi_agent_investigate_review_gap(
            context,
            session,
            &candidate_id,
            revision,
            missing_evidence,
        )
        .err()
    } else {
        None
    };
    if let Some(error) = gap_error {
        return AgentTargetOutcome::incomplete(format!("review_gap_investigation_failed:{error}"));
    }
    if success {
        outcome
    } else {
        AgentTargetOutcome::incomplete(format!(
            "review_gate_{verdict}:{}",
            agent_text_truncated(summary, 500)
        ))
    }
}
