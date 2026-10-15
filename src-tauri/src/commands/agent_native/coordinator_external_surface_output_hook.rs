// Foreground advisory after normal anonymous surface closure and signed source.
// Historical publication replay confers no current permission or fresh SDK grant.
fn native_coordinator_external_surface_output_feedback(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget;
    let db = db::open(&context.db_path)?;
    let root = native_coordinator_root_context(context, actor);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // Replay first: a previous paid publication is historical evidence only.
    if native_coordinator_external_surface_output_publication_replay_on(&tx, actor, child)?
        .is_some()
    {
        return Ok(());
    }
    native_coordinator_tick_authority(&tx, &root, actor)?;
    budget::admission::require_determinate(&tx, &actor.root_run_id)?;
    let frame =
        NativeCoordinatorFrame::external_surface_output_closed_on(&tx, &root, actor, child)?;
    native_supervision_frame_admission(frame.kind)?;
    native_coordinator_basis_for_frame_on(&tx, &root, actor, Some(&frame))?;
    tx.commit().map_err(|e| e.to_string())?;
    let decision = native_coordinator_tick_for_frame(&root, actor, &frame)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    frame.verify(&tx, &root, actor)?;
    decision.tick.require_executable(&tx)?;
    budget::admission::require_determinate(&tx, &actor.root_run_id)?;
    if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
        return Err("root_external_surface_output_publication_changed".into());
    }
    native_external_surface_output_require_bounded(&decision.summary.as_json())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

// Historical publication audit only. It returns an original sequence, never
// permission to resume, a fresh Tick, a worker or a new business publication.
pub(crate) fn native_coordinator_external_surface_output_publication_replay_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<Option<i64>, String> {
    if db.is_autocommit() {
        return Err("root_external_surface_output_snapshot_required".into());
    }
    // Genuine receipt absence is not a producer error. Bad signed sources,
    // unknown fees and scope errors must propagate, never become absence.
    if crate::scanning::external_surface_output_receipt::read_original_on(db, actor, child)?
        .is_none()
    {
        return Ok(None);
    }
    let frame = NativeCoordinatorFrame::external_surface_output_original_on(db, actor, child)?;
    native_external_surface_output_published_on(db, actor, &frame)
}

fn native_external_surface_output_published_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: &NativeCoordinatorFrame,
) -> Result<Option<i64>, String> {
    let rows=db.prepare("SELECT q.fact_json,p.fact_json FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.root_run_id=q.root_run_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request'").map_err(|e|e.to_string())?
        .query_map([&actor.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut found = None;
    for (request, publication) in rows {
        let request: JsonValue = serde_json::from_str(&request).map_err(|e| e.to_string())?;
        if request["request"]["basis"]["phase"] != "external-surface-output"
            || request["request"]["basis"]["changedFact"] != frame.fact()
        {
            continue;
        }
        let publication: JsonValue =
            serde_json::from_str(&publication).map_err(|e| e.to_string())?;
        let event = publication["eventSequence"]
            .as_i64()
            .ok_or("root_external_surface_output_publication_changed")?;
        let projected=crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(db,&actor.root_run_id,event)?;
        if projected.record.get("localTools").is_some() {
            return Err("root_external_surface_output_step_not_bounded".into());
        }
        native_external_surface_output_require_bounded(&projected.record["summary"])?;
        if found.replace(event).is_some() {
            return Err("root_external_surface_output_publication_conflict".into());
        }
    }
    Ok(found)
}
fn native_external_surface_output_require_bounded(summary: &JsonValue) -> Result<(), String> {
    let suggestions = summary["suggestions"]
        .as_array()
        .ok_or("root_decision_schema_invalid")?;
    if !suggestions
        .iter()
        .all(|s| s.as_str() == Some("assess:external_surface_output"))
    {
        return Err("root_external_surface_output_step_not_bounded".into());
    }
    if suggestions.is_empty() {
        return Err("root_external_surface_output_deferred".into());
    }
    Ok(())
}
