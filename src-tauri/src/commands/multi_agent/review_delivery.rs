include!("review_attempt_closure.rs");

#[allow(clippy::too_many_arguments)]
fn persist_review_decision_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    reviewer: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    request_id: &str,
    candidate_id: &str,
    revision: i64,
    verdict: &str,
    reason_codes: &JsonValue,
    missing_evidence: &JsonValue,
    confidence: f64,
    summary: &str,
    target_dir: &Path,
    receipt_recovery: bool,
) -> Result<String, String> {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::mailbox};
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(transaction, lease)?;
    verify_review_snapshot(
        transaction,
        &lease.root_run_id,
        candidate_id,
        revision,
        target_dir,
    )?;
    let reason_codes_text = reason_codes.to_string();
    let missing_evidence_text = missing_evidence.to_string();
    transaction
        .execute(
            "INSERT INTO agent_review_decisions(root_run_id,candidate_id,candidate_revision,reviewer_run_id,verdict,reason_codes_json,missing_evidence_json,confidence) \
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(candidate_id,candidate_revision,reviewer_run_id) DO NOTHING",
            params![
                lease.root_run_id,
                candidate_id,
                revision,
                reviewer.run_id,
                verdict,
                reason_codes_text,
                missing_evidence_text,
                confidence
            ],
        )
        .map_err(|error| format!("无法保存 Reviewer 决策：{error}"))?;
    let (decision_id, stored_root, stored_verdict, stored_reasons, stored_missing, stored_confidence): (
        i64,
        String,
        String,
        String,
        String,
        f64,
    ) = transaction
        .query_row(
            "SELECT id,root_run_id,verdict,reason_codes_json,missing_evidence_json,confidence \
             FROM agent_review_decisions WHERE candidate_id=?1 AND candidate_revision=?2 AND reviewer_run_id=?3",
            params![candidate_id, revision, reviewer.run_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .map_err(|error| format!("无法确认 Reviewer 决策：{error}"))?;
    if stored_root != lease.root_run_id
        || stored_verdict != verdict
        || stored_reasons != reason_codes_text
        || stored_missing != missing_evidence_text
        || (stored_confidence - confidence).abs() > f64::EPSILON
    {
        return Err("review_decision_replay_conflict".into());
    }
    let request_changed = transaction
        .execute(
            "UPDATE agent_review_requests SET status=?1,decision_id=?2,finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
             WHERE id=?3 AND root_run_id=?4 AND assignment_id=?5 AND reviewer_run_id=?6 \
             AND candidate_id=?7 AND candidate_revision=?8 AND (status IN ('running',?1) OR (?11 AND status='failed')) \
             AND lease_epoch=?9 AND fencing_token=?10",
            params![
                verdict,
                decision_id,
                request_id,
                lease.root_run_id,
                reviewer.assignment_id,
                reviewer.run_id,
                candidate_id,
                revision,
                lease.lease_epoch,
                lease.fencing_token,
                receipt_recovery
            ],
        )
        .map_err(|error| format!("无法完成 Reviewer 请求：{error}"))?;
    if request_changed != 1 {
        return Err("review_request_fencing_or_state_conflict".into());
    }
    let send = if receipt_recovery {
        mailbox::send_saved_specialist
    } else {
        mailbox::send
    };
    let message_id = send(
        transaction,
        lease,
        &reviewer.run_id,
        &lease.root_run_id,
        AgentRole::EvidenceReviewer.as_str(),
        AgentRole::Coordinator.as_str(),
        "review_decision",
        request_id,
        &reviewer.assignment_id,
        revision,
        &serde_json::json!({
            "verdict": verdict,
            "summary": summary,
            "candidateId": candidate_id,
            "candidateRevision": revision,
        }),
    )?;
    Ok(message_id)
}

// Costs are settled separately because the model call already happened, even
// when its business result cannot be published. Everything visible as a
// successful review, however, must commit together or not at all.
#[allow(clippy::too_many_arguments)]
fn complete_review_delivery(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    reviewer: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    request_id: &str,
    candidate_id: &str,
    revision: i64,
    decision: &ValidatedReviewDecision,
    target_dir: &Path,
) -> Result<(), String> {
    let tx =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("review_delivery_lock:{error}"))?;
    complete_review_delivery_in_transaction(
        &tx,
        lease,
        reviewer,
        request_id,
        candidate_id,
        revision,
        decision,
        target_dir,
        false,
    )?;
    tx.commit()
        .map_err(|error| format!("review_delivery_commit:{error}"))
}

#[allow(clippy::too_many_arguments)]
fn complete_review_delivery_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    reviewer: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    request_id: &str,
    candidate_id: &str,
    revision: i64,
    decision: &ValidatedReviewDecision,
    target_dir: &Path,
    receipt_recovery: bool,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{
        lease::{require_executable_coordinator, validate_coordinator_lease},
        scheduler,
    };
    require_executable_coordinator(tx, lease)?;
    let frozen:String=tx.query_row("SELECT candidate_json FROM agent_review_requests WHERE id=?1 AND root_run_id=?2 AND candidate_id=?3 AND candidate_revision=?4",params![request_id,lease.root_run_id,candidate_id,revision],|r|r.get(0)).map_err(|_|"review_authorization_original_candidate_missing")?;
    let frozen:JsonValue=serde_json::from_str(&frozen).map_err(|_|"review_authorization_original_candidate_invalid")?;
    native_authorization_original_candidate_on(tx,lease,target_dir,&frozen)?;
    let event_floor: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("review_delivery_event_cursor:{error}"))?;
    let expired = crate::agent_runtime::multi_agent::attempts::ExpiredSavedProof::capture(
        tx, lease, reviewer,
    )?;
    let saved_closure = if receipt_recovery {
        prepare_review_receipt_completion(
            tx,
            lease,
            reviewer,
            request_id,
            candidate_id,
            revision,
            decision,
            target_dir,
        )?;
        Some(ReviewClosedAttemptProof::capture(tx, lease, reviewer)?)
    } else {
        None
    };
    let message = persist_review_decision_in_transaction(
        tx,
        lease,
        reviewer,
        request_id,
        candidate_id,
        revision,
        &decision.verdict,
        &decision.reason_codes,
        &decision.missing_evidence,
        decision.confidence,
        &decision.summary,
        target_dir,
        receipt_recovery,
    )
    .map_err(|error| format!("review_gate_decision:{error}"))?;
    let payload = crate::agent_runtime::secrets::redact_json(&serde_json::json!({
        "verdict":decision.verdict, "summary":decision.summary,
        "candidateId":candidate_id, "candidateRevision":revision,
    }));
    consume_proposal_mailbox_in_transaction(
        tx,
        lease,
        &lease.root_run_id,
        &message,
        "review_decision",
        &payload,
    )
    .map_err(|error| format!("review_decision_delivery_failed:{error}"))?;
    let closure = if let Some(proof) = saved_closure {
        proof
    } else {
        scheduler::finish_child_in_transaction(tx, lease, reviewer, true, &decision.summary)
            .map_err(|error| format!("review_gate_finish:{error}"))?;
        ReviewClosedAttemptProof::capture(tx, lease, reviewer)?
    };
    settle_agent_finding_candidates_in_transaction(
        tx,
        lease,
        &reviewer.run_id,
        revision,
        &decision.verdict,
        target_dir,
    )
    .map_err(|error| format!("review_gate_publish:{error}"))?;
    let complete: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         JOIN agent_messages m ON m.assignment_id=a.id WHERE a.id=?1 AND a.child_run_id=?2 \
         AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 \
         AND r.status='terminal' AND r.terminal_state='completed' AND m.id=?3 \
         AND m.delivered_at<>'' AND m.acknowledged_at<>'') \
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='') \
         AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
        params![reviewer.assignment_id,reviewer.run_id,message], |row| row.get(0),
    ).map_err(|error| format!("review_delivery_postcondition:{error}"))?;
    if !complete {
        return Err("review_delivery_postcondition_failed".into());
    }
    let visible: i64 = tx.query_row(
        "SELECT COUNT(DISTINCT event_type) FROM agent_collaboration_events e \
         WHERE sequence>?1 AND scan_id=?2 AND attempt_number=?3 AND entity_type=event_type AND ( \
           (event_type='assignment' AND entity_id=?4 AND json_extract(payload_json,'$.state')='completed') \
           OR (event_type='agent_run' AND entity_id=?5 AND json_extract(payload_json,'$.status')='terminal' \
             AND json_extract(payload_json,'$.terminalState')='completed') \
           OR (event_type='review_gate' AND entity_id=?6 AND json_extract(payload_json,'$.status')=?7 \
             AND json_extract(payload_json,'$.candidateId')=?8) \
           OR (event_type='mailbox_message' AND entity_id=?9 AND json_extract(payload_json,'$.kind')='review_decision' \
             AND EXISTS(SELECT 1 FROM agent_messages m WHERE m.id=e.entity_id \
               AND m.delivered_at<>'' AND m.acknowledged_at<>'' \
               AND json_extract(e.payload_json,'$.deliveredAt')=m.delivered_at \
               AND json_extract(e.payload_json,'$.acknowledgedAt')=m.acknowledged_at)))",
        params![event_floor,lease.scan_id,lease.attempt_number,reviewer.assignment_id,reviewer.run_id,
            request_id,decision.verdict,candidate_id,message], |row| row.get(0),
    ).map_err(|error| format!("review_delivery_event_postcondition:{error}"))?;
    if visible != 4 {
        return Err("review_delivery_event_postcondition_failed".into());
    }
    persist_gap_review_receipt(tx, &lease.root_run_id, request_id, decision, target_dir)?;
    closure.verify(tx, lease, reviewer)?;
    if let Some(proof) = expired {
        proof.verify(tx, lease, reviewer)?;
    }
    native_authorization_original_candidate_on(tx,lease,target_dir,&frozen)?;
    validate_coordinator_lease(tx, lease)?;
    require_executable_coordinator(tx, lease)
}

// A bounded local retry is allowed only after the failed delivery has paused
// the child and a verified received receipt can drive completion. It contains
// no transport/scheduler call; an unknown model outcome never reaches here.
