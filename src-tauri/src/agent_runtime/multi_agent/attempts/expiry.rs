//! Withdraw only a precisely bound original worker's expired grant.
use super::expiry_proof::ExpiryProof;
use super::*;

pub(crate) fn try_expire_original_in_transaction(
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<bool, String> {
    if tx.is_autocommit() {
        return Err("assignment_expiry_transaction_required".into());
    }
    super::super::lease::validate_coordinator_lease(tx, actor)?;
    let (epoch, fence): (i64, String) = tx.query_row(
        "SELECT a.lease_epoch,a.fencing_token FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND a.target_key=?4
           AND a.role=?5 AND r.role=a.role AND r.lane=a.lane AND r.assignment_id=a.id
           AND r.root_run_id=?3 AND r.parent_run_id=?3 AND r.scan_id=?6 AND r.attempt_number=?7
           AND r.target_url=?4 AND r.backend='native' AND r.orchestration_policy='multi'
           AND EXISTS(SELECT 1 FROM agent_runs root WHERE root.id=?3 AND root.root_run_id=root.id
             AND root.role='coordinator' AND root.backend='native' AND root.orchestration_policy='multi')",
        params![child.assignment_id,child.run_id,actor.root_run_id,actor.target_key,child.role.as_str(),actor.scan_id,actor.attempt_number],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).map_err(|e| format!("assignment_expiry_original_binding:{e}"))?;
    let mut original_scope = actor.clone();
    original_scope.lease_epoch = epoch;
    original_scope.fencing_token = fence;
    let worker = current(tx, &original_scope, &child.assignment_id)?;
    if worker.child_run_id != child.run_id {
        return Err("assignment_expiry_original_worker_conflict".into());
    }
    let deadline: i64 = tx.query_row(
        "SELECT CASE WHEN datetime(?1) IS NULL THEN -1 WHEN datetime(?1)<=datetime('now','localtime') THEN 1 ELSE 0 END",
        [&worker.expires_at], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if deadline < 0 {
        return Err("assignment_expiry_deadline_invalid".into());
    }
    if worker.state != "expired"
        && (deadline == 0 || !matches!(worker.state.as_str(), "leased" | "running" | "paused"))
    {
        return Ok(false);
    }
    super::super::lease::require_executable_coordinator(tx, actor)?;
    let mut proof = ExpiryProof::capture(tx, actor, child, &worker)?;
    if worker.state == "expired" {
        let complete: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts x JOIN agent_assignments a ON a.id=x.assignment_id
             JOIN agent_runs r ON r.id=x.child_run_id WHERE x.id=?1 AND x.failure_class='worker_lease_expired'
               AND datetime(x.expires_at)<=datetime(x.finished_at) AND datetime(x.finished_at)<=datetime('now','localtime')
               AND ((a.state='paused' AND a.failure_class='worker_lease_expired' AND a.budget_settled_at=''
                 AND a.finished_at='' AND r.status='paused' AND r.finished_at='' AND r.terminal_state=''
                 AND EXISTS(SELECT 1 FROM agent_lane_leases l WHERE l.assignment_id=a.id AND l.scan_id=r.scan_id
                   AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane))
                 OR (a.state='completed' AND a.budget_settled_at<>'' AND a.finished_at<>''
                   AND a.reserved_tokens=0 AND a.reserved_requests=0
                   AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>''
                   AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id))))
             AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?2 OR child_run_id=?3) AND revoked_at='')",
            params![worker.id,child.assignment_id,child.run_id], |r|r.get(0),
        ).map_err(|e|e.to_string())?;
        if !complete {
            let web_closed = child.role == crate::agent_runtime::contract::AgentRole::WebExecutor
                && crate::agent_runtime::web_mode::web_expiry_delivery::read_closed_original_on(
                    tx, actor, child,
                )?
                .is_some();
            if !web_closed {
                return Err("assignment_expiry_existing_audit_invalid".into());
            }
        }
        proof.verify(tx, actor, child, &worker)?;
        return Ok(true);
    }
    let open: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts x JOIN agent_assignments a ON a.id=x.assignment_id
         JOIN agent_runs r ON r.id=x.child_run_id JOIN agent_budget_ledger b ON b.root_run_id=x.root_run_id
         WHERE x.id=?1 AND x.finished_at='' AND a.finished_at='' AND a.budget_settled_at=''
           AND a.lease_epoch=x.coordinator_epoch AND a.fencing_token=x.coordinator_fencing_token
           AND b.lease_epoch=x.coordinator_epoch AND b.fencing_token=x.coordinator_fencing_token
           AND a.state IN ('leased','running','waiting_review','paused') AND r.status IN ('prepared','running','waiting_review','paused')
           AND r.finished_at='' AND r.terminal_state=''
           AND EXISTS(SELECT 1 FROM agent_lane_leases l WHERE l.assignment_id=a.id AND l.scan_id=r.scan_id
             AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane))",
        [&worker.id], |r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if !open {
        return Err("assignment_expiry_original_state_conflict".into());
    }
    let now: String = tx
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    proof.expect_withdrawal(child, &worker, &now)?;
    let changed = tx.execute(
        "UPDATE agent_assignment_attempts SET state='expired',finished_at=?6,failure_class='worker_lease_expired'
         WHERE id=?1 AND worker_id=?2 AND fencing_token=?3 AND lease_epoch=?4 AND expires_at=?5
           AND datetime(expires_at)<=datetime('now','localtime') AND finished_at='' AND state IN ('leased','running','paused')",
        params![worker.id,worker.worker_id,worker.fencing_token,worker.lease_epoch,worker.expires_at,now],
    ).map_err(|e|format!("assignment_expiry_worker_write:{e}"))?;
    let assignment = tx.execute(
        "UPDATE agent_assignments SET state='paused',failure_class='worker_lease_expired',updated_at=?5
         WHERE id=?1 AND child_run_id=?2 AND lease_epoch=?3 AND fencing_token=?4 AND budget_settled_at=''
           AND state IN ('leased','running','waiting_review','paused')",
        params![child.assignment_id,child.run_id,epoch,original_scope.fencing_token,now],
    ).map_err(|e|e.to_string())?;
    let run = tx.execute(
        "UPDATE agent_runs SET status='paused',updated_at=?3 WHERE id=?1 AND root_run_id=?2
           AND status IN ('prepared','running','waiting_review','paused') AND finished_at='' AND terminal_state=''",
        params![child.run_id,actor.root_run_id,now],
    ).map_err(|e|e.to_string())?;
    tx.execute(
        "UPDATE agent_capability_leases SET revoked_at=?6 WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3
           AND lease_epoch=?4 AND fencing_token=?5 AND revoked_at=''",
        params![child.assignment_id,child.run_id,actor.root_run_id,epoch,original_scope.fencing_token,now],
    ).map_err(|e|e.to_string())?;
    let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='')",
        params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if changed != 1 || assignment != 1 || run != 1 || active {
        return Err("assignment_expiry_withdrawal_unconfirmed".into());
    }
    proof.verify(tx, actor, child, &worker)?;
    Ok(true)
}
