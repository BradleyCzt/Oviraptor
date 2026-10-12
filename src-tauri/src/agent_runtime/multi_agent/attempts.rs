//! A worker lease is distinct from Coordinator authority and logical work.
//! Child-run IDs identify the original worker context. No read path issues a
//! replacement identity or adopts historical work lacking a worker grant.
use super::{lease::CoordinatorLease, scheduler::ScheduledChild};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub(crate) mod audit_rows;
mod cleanup;
mod expired_saved;
mod expiry;
mod new_withdrawal;
pub(crate) mod withdrawal_receipt;
pub(super) use new_withdrawal::withdraw_new_for_supervision;
pub(crate) mod expiry_proof;
pub(crate) use expired_saved::{verify_expired_closed_audit, ExpiredSavedProof};
pub(crate) use expiry::try_expire_original_in_transaction;
mod lifecycle;
mod reservation;
mod saved;
pub(crate) use cleanup::pause_original;
pub(crate) use lifecycle::{finish, pause, renew, start};
pub(crate) use reservation::require_reservation;
pub(crate) use saved::finish_saved;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AssignmentAttempt {
    pub id: String,
    pub root_run_id: String,
    pub assignment_id: String,
    pub child_run_id: String,
    pub coordinator_epoch: i64,
    pub coordinator_fencing_token: String,
    pub lease_epoch: i64,
    pub fencing_token: String,
    pub worker_id: String,
    pub state: String,
    pub expires_at: String,
    pub finished_at: String,
    pub failure_class: String,
}

pub(crate) fn current(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<AssignmentAttempt, String> {
    let row = db.query_row(
        "SELECT x.id,x.root_run_id,x.assignment_id,x.child_run_id,x.coordinator_epoch,
         x.coordinator_fencing_token,x.lease_epoch,x.fencing_token,x.worker_id,x.state,x.expires_at,x.finished_at,x.failure_class
         FROM agent_assignment_attempts x JOIN agent_assignments a
           ON a.id=x.assignment_id AND a.child_run_id=x.child_run_id AND a.coordinator_run_id=x.root_run_id
         WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.target_key=?3",
        params![assignment,lease.root_run_id,lease.target_key], |r|Ok(AssignmentAttempt {
            id:r.get(0)?,root_run_id:r.get(1)?,assignment_id:r.get(2)?,child_run_id:r.get(3)?,
            coordinator_epoch:r.get(4)?,coordinator_fencing_token:r.get(5)?,lease_epoch:r.get(6)?,
            fencing_token:r.get(7)?,worker_id:r.get(8)?,state:r.get(9)?,expires_at:r.get(10)?,finished_at:r.get(11)?,failure_class:r.get(12)?,
        }),
    ).optional().map_err(|e|format!("assignment_attempt_read:{e}"))?
        .ok_or("assignment_attempt_missing_requires_reconciliation")?;
    if row.coordinator_epoch != lease.lease_epoch
        || row.coordinator_fencing_token != lease.fencing_token
        || row.lease_epoch <= 0
        || [&row.id, &row.worker_id, &row.fencing_token]
            .iter()
            .any(|v| uuid::Uuid::parse_str(v).is_err())
        || !matches!(
            row.state.as_str(),
            "leased" | "running" | "paused" | "completed" | "failed" | "cancelled" | "expired"
        )
    {
        return Err("assignment_attempt_binding_conflict".into());
    }
    Ok(row)
}

// Worker expiry closes response publication independently of the Coordinator.
// Existing non-expired local pause/no-send contracts keep their own checks.
pub(crate) fn require_unexpired_response_worker(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    super::lease::validate_coordinator_lease(db, lease)?;
    let worker = current(db, lease, &child.assignment_id)?;
    let unexpired: bool = db
        .query_row(
            "SELECT COALESCE(datetime(?1)>datetime('now','localtime'),0)",
            [&worker.expires_at],
            |row| row.get(0),
        )
        .map_err(|e| format!("response_worker_deadline:{e}"))?;
    if worker.child_run_id != child.run_id || worker.state == "expired" || !unexpired {
        return Err("response_worker_expired_or_fenced".into());
    }
    Ok(())
}

pub(crate) fn issue_first(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    child_run: &str,
) -> Result<AssignmentAttempt, String> {
    super::budget::continuation::migration::require_plain_root_boundary(tx, &lease.root_run_id)?;
    super::lease::validate_coordinator_lease(tx, lease)?;
    let fresh: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments
        WHERE id=?1 AND coordinator_run_id=?2 AND child_run_id=?3 AND target_key=?4
          AND state='prepared' AND lease_epoch=0 AND fencing_token='')
        AND NOT EXISTS(SELECT 1 FROM agent_assignment_attempts WHERE assignment_id=?1)",
            params![assignment, lease.root_run_id, child_run, lease.target_key],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !fresh {
        return Err("assignment_attempt_initial_claim_conflict".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let fence = uuid::Uuid::new_v4().to_string();
    let worker = uuid::Uuid::new_v4().to_string();
    let changed=tx.execute("INSERT INTO agent_assignment_attempts(id,root_run_id,assignment_id,child_run_id,
        coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,state,expires_at)
        VALUES(?1,?2,?3,?4,?5,?6,1,?7,?8,'leased',?9)",
        params![id,lease.root_run_id,assignment,child_run,lease.lease_epoch,lease.fencing_token,fence,worker,lease.lease_expires_at])
        .map_err(|e|format!("assignment_attempt_issue:{e}"))?;
    let actual = current(tx, lease, assignment)?;
    if changed != 1
        || actual.id != id
        || actual.fencing_token != fence
        || actual.worker_id != worker
        || actual.state != "leased"
        || actual.expires_at != lease.lease_expires_at
    {
        return Err("assignment_attempt_issue_unconfirmed".into());
    }
    Ok(actual)
}

pub(crate) fn require_live_for_run(
    db: &Connection,
    run_id: &str,
) -> Result<AssignmentAttempt, String> {
    let lease=db.query_row("SELECT c.scan_id,c.attempt_number,c.target_key,c.root_run_id,c.lease_epoch,c.fencing_token,c.lease_expires_at
        FROM agent_coordinator_leases c JOIN agent_runs r ON r.root_run_id=c.root_run_id
        WHERE r.id=?1 AND r.backend='native' AND r.orchestration_policy='multi'
          AND r.scan_id=c.scan_id AND r.attempt_number=c.attempt_number AND r.target_url=c.target_key",
        [run_id],|r|Ok(CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,
            root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?}))
        .map_err(|e|format!("assignment_attempt_run_binding:{e}"))?;
    super::lease::validate_coordinator_lease(db, &lease)?;
    super::lease::require_executable_coordinator(db, &lease)?;
    let assignment: String = db
        .query_row(
            "SELECT assignment_id FROM agent_runs WHERE id=?1",
            [run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let attempt = current(db, &lease, &assignment)?;
    let live:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts x
        JOIN agent_assignments a ON a.id=x.assignment_id AND a.child_run_id=x.child_run_id
        JOIN agent_runs r ON r.id=x.child_run_id AND r.assignment_id=a.id
        WHERE x.id=?1 AND r.id=?2 AND datetime(x.expires_at)>datetime('now','localtime')
          AND a.lease_epoch=x.coordinator_epoch AND a.fencing_token=x.coordinator_fencing_token
          AND a.lease_expires_at>datetime('now','localtime') AND r.lease_expires_at>datetime('now','localtime')
          AND r.cancel_requested_at='' AND r.role=a.role AND r.lane=a.lane
          AND ((x.state='leased' AND a.state='leased' AND r.status='prepared')
            OR (x.state='running' AND a.state IN ('running','waiting_review') AND r.status='running')
            -- Phase 1 审核 P0-2：同质 Worker 的执行载体。只对
            -- scheduling_model='homogeneous_workers' 生效，legacy 两支条件保持原样。
            OR (a.scheduling_model='homogeneous_workers' AND x.state='leased' AND a.state='assigned' AND r.status='prepared')
            OR (a.scheduling_model='homogeneous_workers' AND x.state='running' AND a.state='running' AND r.status='running')
            -- 重派：assignment 回到 assigned 时 attempt 仍是上一次存活的 running 行（不能违例回退状态）
            OR (a.scheduling_model='homogeneous_workers' AND x.state='running' AND a.state='assigned' AND r.status='running')))",
        params![attempt.id,run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !live {
        return Err("assignment_attempt_expired_or_not_executable".into());
    }
    Ok(attempt)
}
