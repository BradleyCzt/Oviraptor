// Foreground-only advisory assessment of an already saved original worker.
// Never called from the Supervisor writer or child-delivery retry transaction.
fn native_coordinator_lease_expiry_feedback(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<NativeCoordinatorTickReceipt, String> {
    let root = native_coordinator_root_context(context, actor);
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e|e.to_string())?;
    let frame = NativeCoordinatorFrame::lease_closed_mapper(&tx, &root, actor, child)?;
    let _original_sdk_exit = native_coordinator_lease_original_sdk_exit(&root, actor, child)?;
    // Typed source verification and original protocol precede every paid round.
    // A display-only frame cannot enter through this foreground producer.
    native_supervision_frame_admission(frame.kind)?;
    native_coordinator_basis_for_frame_on(&tx, &root, actor, Some(&frame))?;
    tx.commit().map_err(|e|e.to_string())?;
    let decision = native_coordinator_tick_for_frame(&root, actor, &frame)?;
    if !decision.summary.as_json()["suggestions"].as_array()
        .is_some_and(|items|items.iter().all(|item| item.as_str()==Some("assess:lease_expired"))) {
        return Err("root_lease_expired_step_not_bounded".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e|e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    frame.verify(&tx, &root, actor)?;
    decision.tick.require_executable(&tx)?;
    if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
        return Err("root_lease_expired_publication_changed".into());
    }
    tx.commit().map_err(|e|e.to_string())?;
    Ok(decision)
}

// Both first delivery and completed replay must assess the same original fact.
fn native_coordinator_original_expired_feedback(context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild) -> Result<(),String> {
    let db = db::open(&context.db_path)?;
    if crate::agent_runtime::multi_agent::attempts::current(&db,actor,&child.assignment_id)?.state == "expired"
        && crate::agent_runtime::web_mode::root::read(&db,&actor.root_run_id)?.is_some_and(|d|d.lease_expiry_supervision().is_some()) {
        native_coordinator_lease_expiry_feedback(context,actor,child)?;
    }
    Ok(())
}

// Candidate-scoped lookup shared by first publication and every frozen replay.
// Existing review delivery has committed before this foreground assessment.
fn native_coordinator_original_review_expired_feedback(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    candidate: &str,
    revision: i64,
) -> Result<(),String> {
    let db=db::open(&context.db_path)?;
    let child=db.query_row(
        "SELECT q.assignment_id,q.reviewer_run_id FROM agent_review_requests q JOIN agent_assignments a ON a.id=q.assignment_id AND a.child_run_id=q.reviewer_run_id AND a.coordinator_run_id=q.root_run_id AND a.role='evidence_reviewer' AND a.lane='review' WHERE q.root_run_id=?1 AND q.candidate_id=?2 AND q.candidate_revision=?3",
        params![actor.root_run_id,candidate,revision],
        |row|Ok(crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id:row.get(0)?,run_id:row.get(1)?,
            role:crate::agent_runtime::contract::AgentRole::EvidenceReviewer,
        }),
    ).map_err(|error|format!("review_lease_original_assignment:{error}"))?;
    native_coordinator_original_expired_feedback(context,actor,&child)
}
