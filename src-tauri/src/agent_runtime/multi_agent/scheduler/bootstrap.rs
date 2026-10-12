/// A new Coordinator fence does not own the previous generation's child,
/// model receipt, capability or reserved budget. Even a completed bootstrap
/// result needs an explicit, receipt-bound import contract before it may be
/// reused under a different fence. Human directive proposals have their own
/// uncertain-outcome reconciliation and must not block unrelated bootstrap
/// work. Reject before the caller rewrites the ledger fence; a new attempt
/// has its own independent accounting and scope.
pub fn ensure_fresh_readonly_fence(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<(), String> {
    validate_coordinator_lease(connection, lease)?;
    let previous: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         WHERE r.scan_id=?1 AND r.attempt_number=?2 AND r.target_url=?3 AND a.target_key=?3
           AND a.role IN ('spa_api_mapper','identity_session','repo_mapper','source_analyst','client_side')
           AND a.trigger_code NOT LIKE 'human_directive:%'
           AND (a.coordinator_run_id<>?4 OR a.lease_epoch<>?5 OR a.fencing_token<>?6))",
            params![lease.scan_id, lease.attempt_number, lease.target_key,
                lease.root_run_id, lease.lease_epoch, lease.fencing_token],
            |row| row.get(0),
        )
        .map_err(|error| format!("readonly_fencing_history:{error}"))?;
    if previous {
        Err("readonly_fencing_changed_requires_fresh_attempt".into())
    } else {
        Ok(())
    }
}

/// Bootstrap analysis is resumable only as the same frozen assignment. Existing
/// workers are read, not rescheduled: no budget reservation or capability grant
/// is repeated. New assignment + start is a single transaction.
pub fn prepare_readonly_child(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    trigger: &str,
    task_slice: &JsonValue,
    reserved_tokens: i64,
) -> Result<ScheduledChild, String> {
    prepare_readonly_child_checked(
        connection,
        lease,
        role,
        trigger,
        task_slice,
        reserved_tokens,
        |_| Ok(()),
    )
}

/// The captured paid Root proof is checked in the same child issuance transaction.
pub fn prepare_readonly_child_checked(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    trigger: &str,
    task_slice: &JsonValue,
    reserved_tokens: i64,
    verify: impl Fn(&Connection) -> Result<(), String>,
) -> Result<ScheduledChild, String> {
    if !role.is_readonly_bootstrap_role() {
        return Err("readonly_bootstrap_role_invalid".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|e| format!("readonly_bootstrap_lock:{e}"))?;
    let (child, fresh) = prepare_readonly_child_in_transaction(
        &tx,
        lease,
        role,
        trigger,
        task_slice,
        reserved_tokens,
        verify,
    )?;
    // Replay stays a read-only lookup: even callback writes roll back on Drop.
    if fresh {
        tx.commit()
            .map_err(|e| format!("readonly_bootstrap_commit:{e}"))?;
    }
    Ok(child)
}

/// Allocation and original paid authority can share the same issuance lock.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_readonly_child_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    role: AgentRole,
    trigger: &str,
    task_slice: &JsonValue,
    reserved_tokens: i64,
    verify: impl Fn(&Connection) -> Result<(), String>,
) -> Result<(ScheduledChild, bool), String> {
    if !role.is_readonly_bootstrap_role() {
        return Err("readonly_bootstrap_role_invalid".into());
    }
    verify(tx)?;
    validate_coordinator_lease(tx, lease)?;
    require_executable_coordinator(tx, lease)?;
    // Source specialists also enter through this shared scheduler, without
    // Web's outer bootstrap guard. Keep the fence check inside the scheduling
    // transaction so an old completed role cannot be followed by a new role
    // under a replacement generation of the same attempt.
    ensure_fresh_readonly_fence(tx, lease)?;
    super::source::validate_surface_role(tx, lease, role)?;
    if super::source::is_source_role(role) {
        super::source::validate_task_slice(tx, lease, role, task_slice, 1)?;
    }
    if role == AgentRole::ClientSide {
        super::client_side::validate_scheduled(
            task_slice,
            lease,
            1,
            &["evidence.read".into(), "mailbox.write".into()],
        )?;
    }
    let dedup = format!("{}:{trigger}:{}:1", role.as_str(), lease.target_key);
    let id = format!(
        "asg-{}",
        &store::stable_hash(&format!("{}:{dedup}", lease.root_run_id))[..24]
    );
    let existing: Option<(String, String, String)> = tx
        .query_row(
            "SELECT child_run_id,state,task_slice_json FROM agent_assignments WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((run_id, state, stored_slice)) = existing {
        let child = ScheduledChild {
            assignment_id: id,
            run_id,
            role,
        };
        let bound: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
             WHERE a.id=?1 AND a.coordinator_run_id=?2 AND r.root_run_id=?2 AND r.assignment_id=a.id \
               AND a.role=?3 AND r.role=?3 AND a.lane='read_only_analysis' AND r.lane=a.lane \
               AND a.target_key=?4 AND r.target_url=?4 AND r.scan_id=?5 AND r.attempt_number=?6 \
               AND a.lease_epoch=?7 AND a.fencing_token=?8 AND a.dedup_key=?9 AND a.trigger_code=?10 \
               AND a.evidence_revision=1 AND ((a.state='running' AND r.status='running') \
                 OR (a.state='paused' AND r.status='paused') \
                 OR (a.state='completed' AND r.status='terminal' AND r.terminal_state='completed')))",
            params![child.assignment_id,lease.root_run_id,role.as_str(),lease.target_key,lease.scan_id,
                lease.attempt_number,lease.lease_epoch,lease.fencing_token,dedup,trigger],|r|r.get(0),
        ).map_err(|e|e.to_string())?;
        if !bound
            || serde_json::from_str::<JsonValue>(&stored_slice).ok()
                != Some(crate::agent_runtime::secrets::redact_json(task_slice))
        {
            return Err("readonly_bootstrap_replay_conflict".into());
        }
        if state != "running" {
            super::specialist::received_for_reconciliation(tx, lease, &child)?;
        }
        verify(tx)?;
        return Ok((child, false));
    }
    // ClientSide/Identity pre-check when a ledger already exists. SpaApiMapper
    // is often the first child after a paid Root journal with no coarse ledger
    // yet, so it only post-checks in the same transaction (after reserve).
    // RepoMapper/SourceAnalyst stay on the source budget surface.
    if role.requires_web_reviewer_floor_on_bootstrap() {
        super::client_side::require_reviewer_floor(tx, lease, reserved_tokens, 1)?;
    }
    let child = schedule_child_in_transaction(
        tx,
        lease,
        role,
        AgentLane::ReadOnlyAnalysis,
        trigger,
        task_slice,
        1,
        &["evidence.read".into(), "mailbox.write".into()],
        reserved_tokens,
        1,
    )?;
    mark_child_running_in_transaction(tx, lease, &child)?;
    // Web-root Reviewer floor: see AgentRole::requires_web_reviewer_floor_on_bootstrap; SpaApiMapper only when
    // this root actually has a web_mode projection (source-fixture siblings that
    // reuse the Mapper role name stay on the source budget surface).
    if role.requires_web_reviewer_floor_on_bootstrap()
        || (role == AgentRole::SpaApiMapper
            && crate::agent_runtime::web_mode::root::read(tx, &lease.root_run_id)?.is_some())
    {
        super::client_side::require_reviewer_floor(tx, lease, 0, 0)?;
    }
    verify(tx)?;
    Ok((child, true))
}

/// The old build dispatched target requests under `deep_investigator`. A
/// resumed attempt must not silently start another target loop under the new
/// `web_executor` name. Until a checkpoint-aware handoff can prove exactly
/// which requests were committed, require a fresh attempt for either history.
/// Check both tables: an interrupted write may have persisted only one side.
pub fn ensure_fresh_web_executor_attempt(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<(), String> {
    ensure_fresh_web_executor_attempt_except(connection, lease, None)
}

// Exclude only the verified original physical run and assignment; other
// target loops, including legacy or partial one-sided records, still refuse.
fn ensure_fresh_web_executor_attempt_except(
    connection: &Connection,
    lease: &CoordinatorLease,
    original: Option<&ScheduledChild>,
) -> Result<(), String> {
    validate_coordinator_lease(connection, lease)?;
    let prior: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs c ON c.id=a.coordinator_run_id \
             WHERE c.scan_id=?1 AND c.attempt_number=?2 AND c.target_url=?3 \
             AND a.role IN ('deep_investigator','web_executor') AND a.lane='target_touching' AND (?4 IS NULL OR a.id<>?4) \
             UNION ALL SELECT 1 FROM agent_runs r WHERE r.scan_id=?1 AND r.attempt_number=?2 \
             AND r.target_url=?3 AND r.role IN ('deep_investigator','web_executor') \
             AND r.lane='target_touching' AND (?5 IS NULL OR r.id<>?5))",
            params![lease.scan_id, lease.attempt_number, lease.target_key, original.map(|c|&c.assignment_id), original.map(|c|&c.run_id)],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法检查既有目标执行任务：{error}"))?;
    if prior {
        Err("target_execution_recovery_requires_fresh_attempt".into())
    } else {
        Ok(())
    }
}

/// Dynamic allocation replay keeps the first worker's still-live grant.
pub(crate) fn verify_running_mapper_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    task: &JsonValue,
    tokens: i64,
) -> Result<(), String> {
    let trigger = "frontend_evidence_ready";
    let mut expected = AgentAssignment::new(
        child.assignment_id.clone(),
        lease.root_run_id.clone(),
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        lease.target_key.clone(),
        format!("spa_api_mapper:{trigger}:{}:1", lease.target_key),
    );
    expected.child_run_id = child.run_id.clone();
    expected.trigger_code = trigger.into();
    expected.task_slice = task.clone();
    expected.evidence_revision = 1;
    expected.reserved_tokens = tokens;
    expected.reserved_requests = 1;
    expected.capability_lease = vec!["evidence.read".into(), "mailbox.write".into()];
    if verify_scheduled_authority(tx, lease, &expected)? != *child {
        return Err("mapper_allocation_original_worker_conflict".into());
    }
    Ok(())
}

/// Explicit original v15 only. The old one-call readonly issuer is unchanged.
pub(crate) fn prepare_mapper_loop_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    task: &JsonValue,
    input: &super::budget::model::mapper_loop_contract::FrozenMapperLoopInput,
    verify: impl Fn(&Connection) -> Result<(), String>,
) -> Result<(ScheduledChild, bool), String> {
    input.require_on(tx, lease)?;
    verify(tx)?;
    validate_coordinator_lease(tx, lease)?;
    require_executable_coordinator(tx, lease)?;
    ensure_fresh_readonly_fence(tx, lease)?;
    super::source::validate_surface_role(tx, lease, AgentRole::SpaApiMapper)?;
    if task["mapperLoop"] != *input.fact() || task["mapperLoopHash"] != input.hash() {
        return Err("mapper_loop_task_source_changed".into());
    }
    let trigger = "frontend_evidence_ready";
    let dedup = format!("spa_api_mapper:{trigger}:{}:1", lease.target_key);
    let id = format!(
        "asg-{}",
        &store::stable_hash(&format!("{}:{dedup}", lease.root_run_id))[..24]
    );
    let existing: Option<String> = tx
        .query_row(
            "SELECT child_run_id FROM agent_assignments WHERE id=?1",
            [&id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(run_id) = existing {
        let child = ScheduledChild {
            assignment_id: id,
            run_id,
            role: AgentRole::SpaApiMapper,
        };
        let grant =
            super::budget::model::original_mapper_loop_grant_on(tx, lease, &child.assignment_id)?;
        let stored: String = tx
            .query_row(
                "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
                [&child.assignment_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if grant.input().fact() != input.fact()
            || stored.as_bytes() != crate::agent_runtime::secrets::redact_json(task).to_string().as_bytes()
        {
            return Err("mapper_loop_original_grant_conflict".into());
        }
        let running: bool = tx
            .query_row(
                "SELECT state='running' FROM agent_assignments WHERE id=?1",
                [&child.assignment_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if running {
            verify_running_mapper_loop_in_transaction(tx, lease, &child, task, input)?;
        }
        verify(tx)?;
        return Ok((child, false));
    }
    super::budget::supervision_capacity::require_mapper_loop_issue_headroom_on(tx, lease)?;
    let child = schedule_child_in_transaction(
        tx,
        lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        trigger,
        task,
        1,
        &["evidence.read".into(), "mailbox.write".into()],
        input.reserved_tokens(),
        2,
    )?;
    mark_child_running_in_transaction(tx, lease, &child)?;
    verify_running_mapper_loop_in_transaction(tx, lease, &child, task, input)?;
    let grant =
        super::budget::model::original_mapper_loop_grant_on(tx, lease, &child.assignment_id)?;
    if grant.input().fact() != input.fact() {
        return Err("mapper_loop_original_grant_conflict".into());
    }
    super::budget::supervision_capacity::require_mapper_loop_reserved_headroom_on(tx, lease)?;
    verify(tx)?;
    Ok((child, true))
}

pub(crate) fn verify_running_mapper_loop_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    task: &JsonValue,
    input: &super::budget::model::mapper_loop_contract::FrozenMapperLoopInput,
) -> Result<(), String> {
    input.require_on(tx, lease)?;
    let trigger = "frontend_evidence_ready";
    let mut expected = AgentAssignment::new(
        child.assignment_id.clone(),
        lease.root_run_id.clone(),
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        lease.target_key.clone(),
        format!("spa_api_mapper:{trigger}:{}:1", lease.target_key),
    );
    expected.child_run_id = child.run_id.clone();
    expected.trigger_code = trigger.into();
    expected.task_slice = task.clone();
    expected.evidence_revision = 1;
    expected.reserved_tokens = input.reserved_tokens();
    expected.reserved_requests = 2;
    expected.capability_lease = vec!["evidence.read".into(), "mailbox.write".into()];
    if verify_scheduled_authority(tx, lease, &expected)? != *child {
        return Err("mapper_loop_original_worker_conflict".into());
    }
    Ok(())
}
