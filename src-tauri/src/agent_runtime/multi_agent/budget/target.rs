//! Target admission is durable before I/O. Until a bound response receipt is
//! recorded the slot is indeterminate, never free or replayable.
use super::{append, historical::CostOwner, Kind};
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::{OptionalExtension, Transaction};

/// Brokers recheck their full execution grant separately. The original child
/// owner only supplies the shared time ceiling, including response body reads.
pub(crate) fn transport_timeout(
    db: &rusqlite::Connection,
    run: &str,
    ceiling: std::time::Duration,
) -> Result<std::time::Duration, String> {
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let Some((lease, _)) = child_owner(&tx, run)? else {
        return Err("budget_target_original_call_required".into());
    };
    Ok(super::clock::remaining(&tx, &lease.root_run_id)?.min(ceiling))
}

/// Derive authority from the original child attempt, never adopt a newer root
/// fence when an old request's headers arrive. Single execution still uses its
/// existing request journal; it has no coordinator assignment to fabricate.
pub(crate) fn child_owner(
    tx: &Transaction<'_>,
    run: &str,
) -> Result<Option<(CoordinatorLease, String)>, String> {
    let policy: String = tx
        .query_row(
            "SELECT orchestration_policy FROM agent_runs WHERE id=?1 AND backend='native'",
            [run],
            |r| r.get(0),
        )
        .map_err(|_| "budget_run_binding_missing")?;
    match policy.as_str() {
        "single" => return Ok(None),
        "multi" => {}
        _ => return Err("budget_execution_policy_invalid".into()),
    }
    let owner=tx.query_row("SELECT r.scan_id,r.attempt_number,r.target_url,a.coordinator_run_id,a.lease_epoch,a.fencing_token,c.lease_expires_at,a.id
        FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id
        JOIN agent_coordinator_leases c ON c.root_run_id=a.coordinator_run_id
        WHERE r.id=?1 AND r.root_run_id=a.coordinator_run_id AND r.target_url=a.target_key
        AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url
        AND c.lease_epoch=a.lease_epoch AND c.fencing_token=a.fencing_token",
        [run],|r|Ok((CoordinatorLease{scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,
            root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?},r.get(7)?)))
        .optional().map_err(|e|e.to_string())?;
    owner
        .map(Some)
        .ok_or("budget_child_authority_conflict".into())
}

pub(crate) fn claim(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    source: &str,
) -> Result<(), String> {
    crate::agent_runtime::target_protection::require_clear_on(tx, &lease.scan_id, &lease.target_key)?;
    super::continuation::migration::require_plain_root_boundary(tx, &lease.root_run_id)?;
    super::admission::require_determinate(tx, &lease.root_run_id)?;
    super::clock::sample(tx, lease, assignment)?;
    append(
        tx,
        lease,
        assignment,
        "target_requests",
        Kind::Reserve,
        1,
        &format!("target:{source}:reserve"),
        source,
    )?;
    append(
        tx,
        lease,
        assignment,
        "target_requests",
        Kind::Forfeit,
        1,
        &format!("target:{source}:dispatch"),
        source,
    )?;
    crate::agent_runtime::target_protection::require_clear_on(tx, &lease.scan_id, &lease.target_key)
}

pub(crate) fn receive(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    source: &str,
) -> Result<(), String> {
    let original = super::super::attempts::current(tx, lease, assignment)?;
    let owner = CostOwner::for_run(tx, &original.child_run_id)?
        .ok_or("budget_original_worker_binding_conflict")?;
    if owner.lease.root_run_id != lease.root_run_id
        || owner.lease.scan_id != lease.scan_id
        || owner.lease.attempt_number != lease.attempt_number
        || owner.lease.target_key != lease.target_key
        || owner.lease.lease_epoch != lease.lease_epoch
        || owner.lease.fencing_token != lease.fencing_token
        || owner.assignment != assignment
    {
        return Err("budget_original_worker_binding_conflict".into());
    }
    owner.receive_target(tx, source)
}

/// The transport owns a durable request receipt, never an execution grant.
pub(crate) fn receive_for_run(tx: &Transaction<'_>, run: &str, source: &str) -> Result<(), String> {
    if let Some(owner) = CostOwner::for_run(tx, run)? {
        owner.receive_target(tx, source)?;
    }
    Ok(())
}

include!("target/receipt.rs");
