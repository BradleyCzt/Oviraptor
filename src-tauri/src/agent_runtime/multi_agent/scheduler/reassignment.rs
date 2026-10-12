// Explicit replacement of a revoked worker with no durable dispatch.
#[path = "reassignment/lineage.rs"]
mod lineage;
#[path = "reassignment/proof.rs"]
mod proof;
#[path = "reassignment/undispatched.rs"]
mod undispatched;

/// This entry does not adopt historical Coordinator generations, replay sent
/// work, or replace a canonical Reviewer. The logical work stays frozen.
pub fn reassign_undispatched_expired(
    db: &Connection,
    lease: &CoordinatorLease,
    original: &ScheduledChild,
) -> Result<ScheduledChild, String> {
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
        .map_err(|e| format!("assignment_replacement_lock:{e}"))?;
    validate_coordinator_lease(&tx, lease)?;
    require_executable_coordinator(&tx, lease)?;
    let mut issued = lease.clone();
    issued.lease_expires_at = tx.query_row(
        "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1 AND lease_epoch=?2 AND fencing_token=?3",
        params![lease.root_run_id,lease.lease_epoch,lease.fencing_token], |r|r.get(0),
    ).map_err(|e|e.to_string())?;
    let lease = &issued;
    let mut assignment = super::assignment::load_assignment(&tx, &original.assignment_id)?
        .ok_or("assignment_replacement_missing")?;
    if assignment.role != original.role
        || !original.role.allows_undispatched_worker_replacement()
    {
        return Err("assignment_replacement_role_requires_dedicated_recovery".into());
    }
    validate_child_contract(
        &tx,
        lease,
        assignment.role,
        assignment.lane,
        &assignment.task_slice,
        assignment.evidence_revision,
        &assignment.capability_lease,
        assignment.reserved_tokens,
        assignment.reserved_requests,
    )?;
    if assignment.child_run_id != original.run_id {
        let child = lineage::replay(&tx, lease, original, &assignment)?;
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(child);
    }
    let old = super::attempts::current(&tx, lease, &assignment.id)?;
    if !super::attempts::try_expire_original_in_transaction(&tx, lease, original)? {
        return Err("assignment_replacement_requires_expired_worker".into());
    }
    undispatched::verify(&tx, lease, original, false)?;
    super::budget::limits::initialize(&tx, lease)?;
    super::budget::admission::require_determinate(&tx, &lease.root_run_id)?;
    super::budget::clock::remaining(&tx, &lease.root_run_id)?;
    let now: String = tx
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let new_id = uuid::Uuid::new_v4().to_string();
    let new_fence = uuid::Uuid::new_v4().to_string();
    let new_worker = uuid::Uuid::new_v4().to_string();
    let new_run = format!("run-{}", uuid::Uuid::new_v4());
    let ordinal = old
        .lease_epoch
        .checked_add(1)
        .ok_or("assignment_replacement_ordinal_overflow")?;
    let proof = proof::ReplacementProof::capture(&tx, lease, original, &new_run, &new_id, &now)?;
    super::budget::model::release_unsent(&tx, lease, &assignment.id)?;
    super::budget::limits::release_slot(&tx, lease, &assignment.id)?;
    undispatched::verify(&tx, lease, original, true)?;
    let original_hash = lineage::original_hash(&tx, &old.id)?;
    let changed=tx.execute("INSERT INTO agent_assignment_attempts(id,root_run_id,assignment_id,child_run_id,
        coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,state,expires_at,leased_at,heartbeat_at)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'leased',?10,?11,?11)",
        params![new_id,lease.root_run_id,assignment.id,new_run,lease.lease_epoch,lease.fencing_token,ordinal,new_fence,new_worker,lease.lease_expires_at,now])
        .map_err(|e|format!("assignment_replacement_issue:{e}"))?;
    let moved=tx.execute("UPDATE agent_assignments SET child_run_id=?2,state='leased',failure_class='',lease_expires_at=?3,updated_at=?4
        WHERE id=?1 AND child_run_id=?5 AND state='paused' AND failure_class='worker_lease_expired'
          AND lease_epoch=?6 AND fencing_token=?7 AND budget_settled_at='' AND finished_at=''",
        params![assignment.id,new_run,lease.lease_expires_at,now,original.run_id,lease.lease_epoch,lease.fencing_token])
        .map_err(|e|e.to_string())?;
    if changed != 1 || moved != 1 {
        return Err("assignment_replacement_issue_unconfirmed".into());
    }
    assignment.child_run_id = new_run.clone();
    create_child_grant(&tx, lease, &assignment)?;
    super::budget::model::reserve(
        &tx,
        lease,
        &assignment.id,
        assignment.reserved_tokens,
        assignment.reserved_requests,
    )?;
    super::budget::limits::reserve_slot(&tx, lease, &assignment.id)?;
    let inserted=tx.execute("INSERT INTO agent_assignment_replacements(original_attempt_id,replacement_attempt_id,root_run_id,
        assignment_id,coordinator_epoch,coordinator_fencing_token,original_hash,assignment_hash)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![old.id,new_id,lease.root_run_id,assignment.id,lease.lease_epoch,lease.fencing_token,original_hash,store::stable_hash(&assignment.content_key().to_string())])
        .map_err(|e|format!("assignment_replacement_fact:{e}"))?;
    let worker = super::attempts::current(&tx, lease, &assignment.id)?;
    if inserted != 1
        || worker.id != new_id
        || worker.worker_id != new_worker
        || worker.fencing_token != new_fence
        || worker.lease_epoch != ordinal
        || worker.state != "leased"
        || !worker.finished_at.is_empty()
        || !worker.failure_class.is_empty()
    {
        return Err("assignment_replacement_issue_unconfirmed".into());
    }
    let child = verify_scheduled_authority(&tx, lease, &assignment)?;
    verify_fresh_grant_deadline(&tx, lease, &child)?;
    validate_child_contract(
        &tx,
        lease,
        assignment.role,
        assignment.lane,
        &assignment.task_slice,
        assignment.evidence_revision,
        &assignment.capability_lease,
        assignment.reserved_tokens,
        assignment.reserved_requests,
    )?;
    lineage::verify(&tx, lease, &assignment)?;
    undispatched::verify(&tx, lease, original, true)?;
    proof.verify(&tx, lease, original, &child, &old.id, &new_id)?;
    tx.commit()
        .map_err(|e| format!("assignment_replacement_commit:{e}"))?;
    Ok(child)
}
