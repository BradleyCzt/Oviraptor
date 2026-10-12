// Web control-plane recovery is distinct from the read-only schedule lookup.
// No gateway error retries a call: only an expired, undispatched worker may
// acquire a replacement before the model transport begins.
pub fn prepare_supervised_readonly_child(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    trigger: &str,
    task_slice: &JsonValue,
    reserved_tokens: i64,
) -> Result<ScheduledChild, String> {
    if !role.allows_supervised_readonly_bootstrap() {
        return Err("supervised_bootstrap_role_requires_dedicated_recovery".into());
    }
    let dedup = format!("{}:{trigger}:{}:1", role.as_str(), lease.target_key);
    let id = format!(
        "asg-{}",
        &store::stable_hash(&format!("{}:{dedup}", lease.root_run_id))[..24]
    );
    let mut frozen = AgentAssignment::new(
        id,
        lease.root_run_id.clone(),
        role,
        AgentLane::ReadOnlyAnalysis,
        lease.target_key.clone(),
        dedup,
    );
    frozen.trigger_code = trigger.into();
    frozen.task_slice = task_slice.clone();
    frozen.evidence_revision = 1;
    frozen.reserved_tokens = reserved_tokens.max(0);
    frozen.reserved_requests = 1;
    frozen.capability_lease = vec!["evidence.read".into(), "mailbox.write".into()];
    let prepared = prepare_readonly_child(
        connection,
        lease,
        role,
        trigger,
        task_slice,
        reserved_tokens,
    );
    if let Ok(child) = &prepared {
        let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if super::attempts::require_live_for_run(&tx, &child.run_id).is_ok() {
            let actual = super::assignment::load_assignment(&tx, &child.assignment_id)?
                .ok_or("readonly_bootstrap_replay_conflict")?;
            if actual.content_key() != frozen.content_key() {
                return Err("readonly_bootstrap_replay_conflict".into());
            }
            verify_scheduled_authority(&tx, lease, &actual)?;
            return Ok(child.clone());
        }
        // A durable saved receipt follows the local delivery contract, which
        // never restores execution or issues another provider request.
        if super::specialist::received_for_reconciliation(&tx, lease, child).is_ok() {
            return Ok(child.clone());
        }
    }
    ensure_fresh_readonly_fence(connection, lease)?;
    let actual = super::assignment::load_assignment(connection, &frozen.id)?
        .ok_or("readonly_bootstrap_recovery_missing")?;
    if actual.content_key() != frozen.content_key() {
        return Err("readonly_bootstrap_replay_conflict".into());
    }
    let original = ScheduledChild {
        assignment_id: actual.id,
        run_id: actual.child_run_id,
        role,
    };
    if actual.state.as_str() == "leased" {
        let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if super::attempts::require_live_for_run(&tx, &original.run_id).is_ok() {
            let expected = super::assignment::load_assignment(&tx, &original.assignment_id)?
                .ok_or("readonly_bootstrap_replay_conflict")?;
            if expected.content_key() != frozen.content_key()
                || expected.child_run_id != original.run_id
            {
                return Err("readonly_bootstrap_replay_conflict".into());
            }
            verify_scheduled_authority(&tx, lease, &expected)?;
            mark_child_running_in_transaction(&tx, lease, &original)?;
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(original);
        }
    }
    let replacement = reassign_undispatched_expired(connection, lease, &original)?;
    start_child_or_release(connection, lease, &replacement)?;
    Ok(replacement)
}
