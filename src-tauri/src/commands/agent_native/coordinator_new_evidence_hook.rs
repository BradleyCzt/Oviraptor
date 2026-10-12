// Original closed HTTP facts authorize only an advisory paid Root assessment.
// No new executor, capability, target action or C renewal follows this hook.
fn native_coordinator_new_evidence_feedback(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    evidence_id: &str,
) -> Result<NativeCoordinatorTickReceipt, String> {
    let root = native_coordinator_root_context(context, actor);
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    let frame = NativeCoordinatorFrame::new_closed_http_evidence(&tx, &root, actor, evidence_id)?;
    // The paid basis separately requires the original creation-only protocol.
    native_coordinator_basis_for_frame_on(&tx, &root, actor, Some(&frame))?;
    tx.commit().map_err(|e| e.to_string())?;
    let decision = native_coordinator_tick_for_frame(&root, actor, &frame)?;
    if !decision.summary.as_json()["suggestions"]
        .as_array()
        .is_some_and(|items| items.iter().all(|item| item.as_str() == Some(frame.step())))
    {
        return Err("root_new_evidence_step_not_bounded".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    frame.verify(&tx, &root, actor)?;
    decision.tick.require_executable(&tx)?;
    if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
        return Err("root_new_evidence_publication_changed".into());
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(decision)
}

fn multi_agent_new_evidence_supervision(
    context: &AgentRunContext,
    session: &MultiAgentSession,
) -> Result<(), String> {
    let db = db::open(&context.db_path)?;
    let mode = crate::agent_runtime::web_mode::root::read(&db, &session.lease.root_run_id)?;
    if !mode.is_some_and(|mode| mode.new_evidence_supervision().is_some()) {
        return Ok(());
    }
    // Every physical node is independent, including nodes at the same revision.
    // Tick re-verifies each immutable source and replays its original paid basis.
    let mut query = db
        .prepare(
            "SELECT id FROM agent_evidence_nodes
        WHERE root_run_id=?1 AND created_by_run_id=?2
        AND kind='request_record' AND provenance='observed' ORDER BY rowid,id",
        )
        .map_err(|e| e.to_string())?;
    let nodes = query
        .query_map(
            params![session.lease.root_run_id, session.executor.run_id],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for node in nodes {
        session.supervisor.check()?;
        native_coordinator_new_evidence_feedback(context, &session.lease, &node)?;
    }
    Ok(())
}
