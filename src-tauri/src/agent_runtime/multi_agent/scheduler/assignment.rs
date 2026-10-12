#[allow(clippy::too_many_arguments)]
pub fn schedule_child(
    connection: &Connection,
    lease: &CoordinatorLease,
    role: AgentRole,
    lane: AgentLane,
    trigger_code: &str,
    task_slice: &JsonValue,
    evidence_revision: i64,
    capabilities: &[String],
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<ScheduledChild, String> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定多智能体调度：{error}"))?;
    let child = schedule_child_in_transaction(
        &transaction,
        lease,
        role,
        lane,
        trigger_code,
        task_slice,
        evidence_revision,
        capabilities,
        reserved_tokens,
        reserved_requests,
    )?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交多智能体调度：{error}"))?;
    Ok(child)
}

/// The caller commits the assignment with its source instruction and outbox.
/// Never accept a plain Connection here: a partial handoff must roll back the
/// child, lane, capabilities and budget together with the directive state.
#[allow(clippy::too_many_arguments)]
pub(crate) fn schedule_child_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    role: AgentRole,
    lane: AgentLane,
    trigger_code: &str,
    task_slice: &JsonValue,
    evidence_revision: i64,
    capabilities: &[String],
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<ScheduledChild, String> {
    let connection: &Connection = transaction;
    // DESIGN §5.1 / §5.2：同质任务由 homogeneous::apply 的调度分支负责（含 slice_id 的
    // dedupe + lane_slots 配额 + 不写旧 lane 互斥表）。legacy 分支一律不受影响。
    if crate::agent_runtime::multi_agent::homogeneous::apply::task_exists_for_root(
        connection,
        &lease.root_run_id,
    )
    .unwrap_or(false)
    {
        return Err("homogeneous_worker_requires_homogeneous_scheduler".into());
    }
    validate_coordinator_lease(connection, lease)?;
    let mut issued_lease = lease.clone();
    issued_lease.lease_expires_at = connection.query_row(
        "SELECT lease_expires_at FROM agent_coordinator_leases WHERE scan_id=?1 AND attempt_number=?2
         AND target_key=?3 AND root_run_id=?4 AND lease_epoch=?5 AND fencing_token=?6",
        params![lease.scan_id,lease.attempt_number,lease.target_key,lease.root_run_id,lease.lease_epoch,lease.fencing_token],
        |r|r.get(0),
    ).map_err(|e|format!("scheduled_deadline_read:{e}"))?;
    let lease = &issued_lease;
    validate_child_contract(
        connection,
        lease,
        role,
        lane,
        task_slice,
        evidence_revision,
        capabilities,
        reserved_tokens,
        reserved_requests,
    )?;
    let dedup_key = format!(
        "{}:{}:{}:{}",
        role.as_str(),
        trigger_code,
        lease.target_key,
        evidence_revision
    );
    let assignment_id = format!(
        "asg-{}",
        &store::stable_hash(&format!("{}:{dedup_key}", lease.root_run_id))[..24]
    );
    let run_id = format!("run-{assignment_id}");
    let mut assignment = AgentAssignment::new(
        assignment_id.clone(),
        lease.root_run_id.clone(),
        role,
        lane,
        lease.target_key.clone(),
        dedup_key,
    );
    assignment.child_run_id = run_id.clone();
    assignment.trigger_code = trigger_code.to_string();
    assignment.task_slice = task_slice.clone();
    assignment.evidence_revision = evidence_revision;
    assignment.reserved_tokens = reserved_tokens.max(0);
    assignment.reserved_requests = reserved_requests.max(0);
    assignment.capability_lease = capabilities.to_vec();
    validate_coordinator_lease(transaction, lease)?;
    require_executable_coordinator(transaction, lease)?;
    if role == AgentRole::Authorization && super::assignment::load_assignment(transaction, &assignment_id)?.is_none() {
        crate::agent_runtime::multi_agent::budget::supervision_capacity::require_authorization_issue_headroom_on(transaction, lease)?;
    }
    if role == AgentRole::ExternalSurface && super::assignment::load_assignment(transaction, &assignment_id)?.is_none() {
        crate::agent_runtime::multi_agent::budget::supervision_capacity::require_external_surface_issue_headroom_on(transaction, lease, reserved_requests)?;
    }
    if role == AgentRole::WebExecutor {
        if super::assignment::load_assignment(transaction,&assignment_id)?.is_some() {
            let original=verify_scheduled_authority(transaction,lease,&assignment)?;
            ensure_fresh_web_executor_attempt_except(transaction,lease,Some(&original))?;
        } else { ensure_fresh_web_executor_attempt(transaction, lease)?; }
    }
    // Upgraded databases can contain active assignments written before the
    // lane table existed. Treat those as occupying their lane too; never
    // silently reclaim a slot on expiry, particularly a target-touching one.
    // DESIGN §6：同质 Worker 的行用 agent_lane_slots 计配额，不参与这里的互斥盘点，
    // 否则同 target 的多切片并行会被误判成 agent_lane_occupied。
    let legacy_occupant: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.coordinator_run_id \
             WHERE r.scan_id=?1 AND r.attempt_number=?2 AND a.target_key=?3 AND a.lane=?4 \
             AND a.id<>?5 AND a.state IN ('prepared','leased','running','waiting_review') \
             AND a.scheduling_model='legacy_role_pipeline')",
            params![
                lease.scan_id,
                lease.attempt_number,
                lease.target_key,
                lane.as_str(),
                assignment_id
            ],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法检查旧版 lane 占用：{error}"))?;
    if legacy_occupant {
        return Err("agent_lane_occupied".into());
    }
    // An idempotent lookup retains the original worker's authority. Rotation
    // requires a distinct attempt; never repair or renew an existing child here.
    if super::assignment::load_assignment(transaction, &assignment_id)?.is_some() {
        return verify_scheduled_authority(transaction, lease, &assignment);
    }
    initialize_child_budget(transaction, lease)?;
    let inserted = matches!(
        insert_assignment(transaction, &assignment)?,
        AssignmentInsert::Inserted(_)
    );
    let active: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 AND state IN ('prepared','leased','running','waiting_review'))",
            [&assignment_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法检查 assignment 生命周期：{error}"))?;
    if !active {
        return Err("assignment_replay_not_active".into());
    }
    // §5.3 Loop6: every scheduled child holds exactly one contract row in the
    // same transaction that takes the lane and reserves budget, so a failure
    // here rolls everything back together. The v1 key binds attempt + target +
    // role + trigger + revision; cross-trigger logical dedup (dropping the
    // trigger) waits until task_slice carries a stable action field.
    let contract = super::contract_owner::schedule_contract_key(
        lease.attempt_number,
        &lease.target_key,
        role.as_str(),
        trigger_code,
        evidence_revision,
    );
    super::contract_owner::acquire_contract_owner(
        transaction,
        &lease.root_run_id,
        &contract,
        &assignment_id,
        lease.lease_epoch,
        &lease.fencing_token,
    )?;
    let claimed = transaction
        .execute(
            "INSERT INTO agent_lane_leases(scan_id,attempt_number,target_key,lane,assignment_id) \
             VALUES(?1,?2,?3,?4,?5) ON CONFLICT DO NOTHING",
            params![
                lease.scan_id,
                lease.attempt_number,
                lease.target_key,
                lane.as_str(),
                assignment_id
            ],
        )
        .map_err(|error| format!("无法占用 lane：{error}"))?;
    if claimed == 0 {
        let owner: String = transaction
            .query_row(
                "SELECT assignment_id FROM agent_lane_leases WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND lane=?4",
                params![lease.scan_id, lease.attempt_number, lease.target_key, lane.as_str()],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法读取 lane 持有者：{error}"))?;
        if owner != assignment_id {
            return Err("agent_lane_occupied".into());
        }
    }
    if inserted {
        super::attempts::issue_first(transaction, lease, &assignment_id, &run_id)?;
        reserve_child_budget(transaction, lease, &assignment)?;
    }
    transaction
        .execute(
            "UPDATE agent_assignments SET lease_epoch=?1,fencing_token=?2,lease_expires_at=?3,\
             state=CASE WHEN state='prepared' THEN 'leased' ELSE state END,leased_at=CASE WHEN leased_at='' THEN datetime('now','localtime') ELSE leased_at END,\
             updated_at=datetime('now','localtime') WHERE id=?4 AND coordinator_run_id=?5",
            params![
                lease.lease_epoch,
                lease.fencing_token,
                lease.lease_expires_at,
                assignment_id,
                lease.root_run_id
            ],
        )
        .map_err(|error| format!("无法租用 assignment：{error}"))?;
    super::budget::clock::sample(transaction, lease, &assignment_id)?;
    create_child_grant(transaction, lease, &assignment)?;
    // INSERT triggers must not change the frozen surface while authority is
    // being issued. Failure rolls back child, lane, capabilities and budget.
    super::source::validate_surface_role(transaction, lease, role)?;
    if role == AgentRole::EvidenceReviewer
        && super::source::uses_source_runtime(transaction, lease, role)?
    {
        super::source_review_subject::validate_slice(
            transaction,
            lease,
            task_slice,
            evidence_revision,
            capabilities,
        )?;
        super::source_review_subject::verify_assignment(
            transaction,
            lease,
            &ScheduledChild {
                assignment_id: assignment_id.clone(),
                run_id: run_id.clone(),
                role,
            },
        )?;
    }
    let child = verify_scheduled_authority(transaction, lease, &assignment)?;
    verify_fresh_grant_deadline(transaction, lease, &child)?;
    Ok(child)
}
