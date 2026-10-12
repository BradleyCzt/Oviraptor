// Read-only original target receipt verification; never a new execution grant.
pub(crate) fn original_target_receipt(
    db: &rusqlite::Connection,
    actor: &CoordinatorLease,
    run: &str,
    source: &str,
) -> Result<(String, String), String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return original_target_receipt(&tx, actor, run, source);
    }
    let owner = CostOwner::for_run(db, run)?.ok_or("budget_original_worker_binding_conflict")?;
    owner.verify(db)?;
    if owner.lease.root_run_id != actor.root_run_id
        || owner.lease.scan_id != actor.scan_id
        || owner.lease.attempt_number != actor.attempt_number
        || owner.lease.target_key != actor.target_key
        || owner.lease.lease_epoch != actor.lease_epoch
        || owner.lease.fencing_token != actor.fencing_token
    {
        return Err("budget_original_worker_binding_conflict".into());
    }
    for (kind, suffix) in [
        ("reserve", "reserve"),
        ("forfeit", "dispatch"),
        ("reconcile", "receipt"),
    ] {
        let key = super::scope::stored_key(
            db,
            &actor.root_run_id,
            &owner.assignment,
            &owner.attempt.id,
            &format!("target:{source}:{suffix}"),
        )?;
        let exact: bool = db
            .query_row(
                "SELECT count(*)=1 FROM agent_budget_entries WHERE root_run_id=?1
             AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests'
             AND kind=?4 AND amount=1 AND source_id=?5 AND idempotency_key=?6",
                rusqlite::params![
                    actor.root_run_id,
                    owner.assignment,
                    owner.attempt.id,
                    kind,
                    source,
                    key
                ],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !exact {
            return Err("budget_original_target_receipt_conflict".into());
        }
    }
    let count: i64 = db.query_row(
        "SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1
         AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4",
        rusqlite::params![actor.root_run_id,owner.assignment,owner.attempt.id,source],|r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if count != 3 {
        return Err("budget_original_target_receipt_conflict".into());
    }
    Ok((owner.assignment, owner.attempt.id))
}
