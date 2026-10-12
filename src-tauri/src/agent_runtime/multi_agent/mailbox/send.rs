use super::*;

#[allow(clippy::too_many_arguments)]
pub fn send(
    connection: &Connection,
    lease: &CoordinatorLease,
    from_run_id: &str,
    to_run_id: &str,
    from_role: &str,
    to_role: &str,
    kind: &str,
    correlation_id: &str,
    assignment_id: &str,
    evidence_revision: i64,
    payload: &JsonValue,
) -> Result<String, String> {
    let write = |db: &Connection| {
        with_authority(
            db,
            lease,
            from_run_id,
            to_run_id,
            from_role,
            to_role,
            kind,
            correlation_id,
            assignment_id,
            evidence_revision,
            payload,
            || {
                let worker = authority::route(
                    db,
                    lease,
                    from_run_id,
                    to_run_id,
                    from_role,
                    to_role,
                    assignment_id,
                    evidence_revision,
                )?;
                let live = super::super::attempts::require_live_for_run(db, &worker.child_run_id)?;
                if worker != live {
                    return Err("mailbox_worker_authority_conflict".into());
                }
                Ok(worker)
            },
        )
    };
    if !connection.is_autocommit() {
        return write(connection);
    }
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let id = write(&tx)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}

#[allow(clippy::too_many_arguments)]
fn with_authority(
    connection: &Connection,
    lease: &CoordinatorLease,
    from_run_id: &str,
    to_run_id: &str,
    from_role: &str,
    to_role: &str,
    kind: &str,
    correlation_id: &str,
    assignment_id: &str,
    evidence_revision: i64,
    payload: &JsonValue,
    authorize: impl Fn() -> Result<super::super::attempts::AssignmentAttempt, String>,
) -> Result<String, String> {
    if connection.is_autocommit() {
        return Err("mailbox_write_transaction_required".into());
    }
    let original = authorize()?;
    validate_coordinator_lease(connection, lease)?;
    if from_run_id.trim().is_empty() || to_run_id.trim().is_empty() || kind.trim().is_empty() {
        return Err("mailbox_route_invalid".into());
    }
    let child: Option<(String, String)> = connection.query_row(
        "SELECT child.id,a.role FROM agent_assignments a \
         JOIN agent_runs child ON child.id=a.child_run_id \
         JOIN agent_runs root ON root.id=a.coordinator_run_id \
         WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.target_key=?3 AND a.evidence_revision=?4 \
           AND root.role='coordinator' AND root.scan_id=?5 AND root.attempt_number=?6 AND root.target_url=?3 \
           AND child.root_run_id=root.id AND child.role=a.role AND child.scan_id=root.scan_id \
           AND child.attempt_number=root.attempt_number AND child.target_url=root.target_url",
        params![assignment_id, lease.root_run_id, lease.target_key, evidence_revision,
            lease.scan_id, lease.attempt_number],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|error| format!("mailbox_route_lookup_failed:{error}"))?;
    let bound = child.as_ref().is_some_and(|(child_run_id, child_role)| {
        (from_run_id == lease.root_run_id
            && to_run_id == child_run_id
            && from_role == "coordinator"
            && to_role == child_role)
            || (to_run_id == lease.root_run_id
                && from_run_id == child_run_id
                && from_role == child_role
                && to_role == "coordinator")
    });
    if !bound {
        return Err("mailbox_route_not_assignment_bound".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let dedup_key = format!("{assignment_id}:{kind}:{correlation_id}:{evidence_revision}");
    let payload_text = crate::agent_runtime::multi_agent::homogeneous::material_policy::json(connection, &lease.root_run_id, payload)?.to_string();
    let inserted = connection
        .execute(
            "INSERT INTO agent_messages(id,run_id,from_agent,to_agent,kind,correlation_id,dedup_key,payload_json,root_run_id,from_run_id,to_run_id,assignment_id,evidence_revision) \
             SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13 \
             WHERE EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE scan_id=?14 AND attempt_number=?15 \
               AND target_key=?16 AND root_run_id=?9 AND lease_epoch=?17 AND fencing_token=?18 \
               AND lease_expires_at>datetime('now','localtime')) \
               AND EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?14 AND attempt_count=?15 AND status='scanning') \
               AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?14) \
               AND EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs child ON child.id=a.child_run_id \
                 JOIN agent_runs root ON root.id=a.coordinator_run_id \
                 WHERE a.id=?12 AND a.coordinator_run_id=?9 AND a.target_key=?16 AND a.evidence_revision=?13 \
                   AND child.root_run_id=root.id AND child.role=a.role AND child.scan_id=?14 \
                   AND child.attempt_number=?15 AND child.target_url=?16 \
                   AND root.role='coordinator' AND root.scan_id=?14 AND root.attempt_number=?15 AND root.target_url=?16 \
                   AND ((?10=root.id AND ?11=child.id AND ?3='coordinator' AND ?4=a.role) \
                     OR (?10=child.id AND ?11=root.id AND ?3=a.role AND ?4='coordinator'))) \
             ON CONFLICT(run_id,dedup_key) DO NOTHING",
            params![
                id,
                lease.root_run_id,
                from_role,
                to_role,
                kind,
                correlation_id,
                dedup_key,
                payload_text,
                lease.root_run_id,
                from_run_id,
                to_run_id,
                assignment_id,
                evidence_revision,
                lease.scan_id,
                lease.attempt_number,
                lease.target_key,
                lease.lease_epoch,
                lease.fencing_token,
            ],
        )
        .map_err(|error| format!("无法发送 agent 消息：{error}"))?;
    validate_coordinator_lease(connection, lease)?;
    require_active_attempt(connection, &lease.scan_id, lease.attempt_number)?;
    let (existing_id, existing_from, existing_to, existing_from_role, existing_to_role,
        existing_kind, existing_correlation, existing_assignment, existing_revision, existing_payload):
        (String, String, String, String, String, String, String, String, i64, String) = connection
        .query_row(
            "SELECT id,from_run_id,to_run_id,from_agent,to_agent,kind,correlation_id,assignment_id,evidence_revision,payload_json \
             FROM agent_messages WHERE run_id=?1 AND dedup_key=?2",
            params![lease.root_run_id, dedup_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?)),
        )
        .map_err(|error| format!("无法读取幂等重放的 agent 消息：{error}"))?;
    if inserted == 1 && existing_id != id
        || existing_from != from_run_id
        || existing_to != to_run_id
        || existing_from_role != from_role
        || existing_to_role != to_role
        || existing_kind != kind
        || existing_correlation != correlation_id
        || existing_assignment != assignment_id
        || existing_revision != evidence_revision
        || existing_payload != payload_text
    {
        return Err("mailbox_replay_conflict".into());
    }
    let stored = record::Record::load(connection, lease, &existing_id, to_run_id)?;
    if inserted == 1
        && (!stored.delivered.is_empty()
            || !stored.acknowledged.is_empty()
            || stored.message.delivery_attempts != 0)
    {
        return Err("mailbox_insert_receipt_conflict".into());
    }
    authority::route(
        connection,
        lease,
        from_run_id,
        to_run_id,
        from_role,
        to_role,
        assignment_id,
        evidence_revision,
    )?;
    if authorize()? != original {
        return Err("mailbox_worker_authority_conflict".into());
    }
    Ok(existing_id)
}

// Coordinator-only delivery of an already recorded specialist result. This
// path cannot publish an assignment or authorize another model/tool request.
#[allow(clippy::too_many_arguments)]
pub(crate) fn send_saved_specialist(
    db: &Connection,
    lease: &CoordinatorLease,
    from: &str,
    to: &str,
    from_role: &str,
    to_role: &str,
    kind: &str,
    correlation: &str,
    assignment: &str,
    revision: i64,
    payload: &JsonValue,
) -> Result<String, String> {
    with_authority(
        db,
        lease,
        from,
        to,
        from_role,
        to_role,
        kind,
        correlation,
        assignment,
        revision,
        payload,
        || {
            let worker = authority::route(
                db, lease, from, to, from_role, to_role, assignment, revision,
            )?;
            super::saved_message::verify(
                db,
                lease,
                &worker,
                from,
                to,
                from_role,
                to_role,
                kind,
                correlation,
                payload,
            )?;
            Ok(worker)
        },
    )
}
