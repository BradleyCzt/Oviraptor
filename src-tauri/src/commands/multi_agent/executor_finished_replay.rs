// Read-only reuse of an already delivered original WebExecutor completion.
// This is not a worker replacement, a new C grant or an unknown-outcome retry.
fn original_executor_finished_replay(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    payload: &JsonValue,
    summary: &str,
) -> Result<bool, String> {
    let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    original_executor_finished_replay_on(&tx, lease, child, payload, summary)
}

// Same original proof in an existing snapshot; no nested transaction or writes.
fn original_executor_finished_replay_on(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    payload: &JsonValue,
    summary: &str,
) -> Result<bool, String> {
    if db.is_autocommit() {
        return Err("executor_finished_replay_snapshot_required".into());
    }
    use crate::agent_runtime::multi_agent::lease as authority;
    if child.role != crate::agent_runtime::contract::AgentRole::WebExecutor {
        return Err("executor_finished_replay_role_invalid".into());
    }
    let tx = db;
    authority::validate_coordinator_lease(tx, lease)?;
    authority::require_executable_coordinator(tx, lease)?;
    original_executor_finished_historical_on(tx, lease, child, payload, summary)
}

// Immutable original delivery audit only. Expiry cannot erase a known result;
// this helper grants no execution, publication, lease renewal or replacement.
fn original_executor_finished_historical_on(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    payload: &JsonValue,
    summary: &str,
) -> Result<bool, String> {
    if db.is_autocommit() {
        return Err("executor_finished_replay_snapshot_required".into());
    }
    if child.role != crate::agent_runtime::contract::AgentRole::WebExecutor {
        return Err("executor_finished_replay_role_invalid".into());
    }
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    let tx = db;
    let completed: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments
        WHERE id=?1 AND state='completed')",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !completed {
        return Ok(false);
    }
    RootOwner::load_original(tx, &lease.root_run_id)?.require_original_coordinator(tx, lease)?;
    let mode = crate::agent_runtime::web_mode::root::read(tx, &lease.root_run_id)?
        .ok_or("executor_finished_replay_original_mode_missing")?;
    if mode.mode() != crate::agent_runtime::web_mode::WebMode::Multi {
        return Err("executor_finished_replay_role_invalid".into());
    }
    // Read the original child identity directly, not a live/renewed worker.
    let mut workers = tx.prepare(
        "SELECT x.id,x.lease_epoch,x.fencing_token,x.worker_id,x.state,x.finished_at
         FROM agent_assignment_attempts x JOIN agent_assignments a
           ON a.id=x.assignment_id AND a.child_run_id=x.child_run_id AND a.coordinator_run_id=x.root_run_id
         WHERE x.assignment_id=?1 AND x.child_run_id=?2 AND x.root_run_id=?3 AND a.target_key=?4
           AND x.coordinator_epoch=?5 AND x.coordinator_fencing_token=?6",
    ).map_err(|e|e.to_string())?;
    let workers = workers
        .query_map(
            params![
                child.assignment_id,
                child.run_id,
                lease.root_run_id,
                lease.target_key,
                lease.lease_epoch,
                lease.fencing_token
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let [(worker_id, worker_epoch, worker_fence, worker_name, state, finished)] =
        workers.as_slice()
    else {
        return Err("executor_finished_replay_worker_conflict".into());
    };
    if *worker_epoch <= 0
        || state != "completed"
        || finished.is_empty()
        || [worker_id, worker_fence, worker_name]
            .iter()
            .any(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err("executor_finished_replay_worker_conflict".into());
    }
    let payload = crate::agent_runtime::secrets::redact_json(payload).to_string();
    let summary = crate::agent_runtime::secrets::redact_text_with(summary, None);
    let correlation = format!("executor-result:{}", child.assignment_id);
    let message_count: i64 = tx.query_row(
        "SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",
        [&child.assignment_id], |r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if message_count != 1 {
        return Err("executor_finished_replay_receipt_conflict".into());
    }
    let mut messages = tx.prepare(
        "SELECT m.id FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         JOIN agent_messages m ON m.assignment_id=a.id
         WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND a.target_key=?4
           AND a.role='web_executor' AND a.lane='target_touching' AND a.state='completed'
           AND a.lease_epoch=?5 AND a.fencing_token=?6 AND a.budget_settled_at<>''
           AND a.reserved_tokens=0 AND a.reserved_requests=0
           AND r.assignment_id=a.id AND r.root_run_id=?3 AND r.parent_run_id=?3
           AND r.scan_id=?7 AND r.attempt_number=?8 AND r.target_url=?4 AND r.backend='native'
           AND r.role=a.role AND r.lane=a.lane AND r.status='terminal' AND r.terminal_state='completed'
           AND r.terminal_code='child_completed' AND r.terminal_reason=?9 AND r.finished_at<>''
           AND r.used_tokens=json_extract(?10,'$.usage.totalTokens')
           AND r.used_requests=json_extract(?10,'$.usage.modelRequests')
           AND m.run_id=?3 AND m.root_run_id=?3 AND m.from_run_id=?2 AND m.to_run_id=?3
           AND m.from_agent='web_executor' AND m.to_agent='coordinator' AND m.kind='execution_result'
           AND m.correlation_id=?11 AND m.evidence_revision=a.evidence_revision
           AND m.dedup_key=a.id||':execution_result:'||?11||':'||a.evidence_revision
           AND m.payload_json=?10 AND m.artifact_refs_json='[]'
           AND m.delivered_at<>'' AND m.acknowledged_at<>'' AND m.delivery_attempts=1
           AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='')
           AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
    ).map_err(|e|e.to_string())?;
    let messages = messages
        .query_map(
            params![
                child.assignment_id,
                child.run_id,
                lease.root_run_id,
                lease.target_key,
                lease.lease_epoch,
                lease.fencing_token,
                lease.scan_id,
                lease.attempt_number,
                summary,
                payload,
                correlation
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let [message] = messages.as_slice() else {
        return Err("executor_finished_replay_receipt_conflict".into());
    };
    let (events,kinds): (i64,i64) = tx.query_row(
        "SELECT count(*),COUNT(DISTINCT entity_type) FROM agent_collaboration_events
         WHERE scan_id=?1 AND attempt_number=?2 AND entity_type=event_type AND (
           (entity_type='assignment' AND entity_id=?3 AND payload_json=json_object('role','web_executor','state','completed'))
           OR (entity_type='agent_run' AND entity_id=?4 AND payload_json=json_object('role','web_executor','status','terminal','terminalState','completed'))
           OR (entity_type='mailbox_message' AND entity_id=?5 AND payload_json=(SELECT
             json_object('kind','execution_result','deliveredAt',delivered_at,'acknowledgedAt',acknowledged_at)
             FROM agent_messages WHERE id=?5)))",
        params![lease.scan_id,lease.attempt_number,child.assignment_id,child.run_id,message], |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|e| e.to_string())?;
    if (events, kinds) != (3, 3) {
        return Err("executor_finished_replay_event_conflict".into());
    }
    // No COMMIT, writes, grant, receipt replacement or source reconstruction.
    Ok(true)
}
