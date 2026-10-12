// Foreground only, after original financial closure and dedicated delivery.
// The borrowed proof keeps original Web SDK/execution exit inodes held across
// every paid read. Do not reacquire those same inodes inside frame verification.
fn native_coordinator_web_expiry_feedback(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    financial:&crate::agent_runtime::multi_agent::budget::model::expired_web_receipt::ExpiredWebFinancialProof,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget;
    let db = db::open(&context.db_path)?;
    let root = native_coordinator_root_context(context, actor);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    financial.require_on(&tx, actor, child)?;
    budget::admission::require_determinate(&tx, &actor.root_run_id)?;
    let frame = NativeCoordinatorFrame::web_expiry_closed_on(&tx, &root, actor, child, financial)?;
    native_supervision_frame_admission(frame.kind)?;
    // An already published original assessment is a read-only replay. Never
    // route it through a possibly fresh paid SDK admission.
    if native_web_expiry_published_on(&tx, actor, &frame)?.is_some() {
        return Ok(());
    }
    native_coordinator_basis_for_frame_on(&tx, &root, actor, Some(&frame))?;
    tx.commit().map_err(|e| e.to_string())?;
    let decision = native_coordinator_tick_for_frame(&root, actor, &frame)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    financial.require_on(&tx, actor, child)?;
    native_coordinator_tick_authority(&tx, &root, actor)?;
    frame.verify(&tx, &root, actor)?;
    decision.tick.require_executable(&tx)?;
    budget::admission::require_determinate(&tx, &actor.root_run_id)?;
    if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
        return Err("root_web_expiry_publication_changed".into());
    }
    native_web_expiry_require_bounded(&decision.summary.as_json())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

// Historical publication audit only. It returns an original sequence, never
// permission to resume, a fresh Tick, a worker or a new business publication.
pub(crate) fn native_coordinator_web_expiry_publication_replay_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<Option<i64>, String> {
    if db.is_autocommit() {
        return Err("root_web_expiry_snapshot_required".into());
    }
    // A first legitimate financial/delivery pass has no saved delivery yet.
    // The reader still verifies original birth/worker first; only its genuine
    // absence becomes None. Signature/financial/schema errors propagate.
    if crate::agent_runtime::web_mode::web_expiry_delivery::read_closed_original_on(
        db, actor, child,
    )?
    .is_none()
    {
        return Ok(None);
    }
    let frame = NativeCoordinatorFrame::web_expiry_original_on(db, actor, child)?;
    native_web_expiry_published_on(db, actor, &frame)
}

fn native_web_expiry_published_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: &NativeCoordinatorFrame,
) -> Result<Option<i64>, String> {
    let rows=db.prepare("SELECT q.fact_json,p.fact_json FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.root_run_id=q.root_run_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request'").map_err(|e|e.to_string())?
        .query_map([&actor.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut found = None;
    for (request, publication) in rows {
        let request: JsonValue = serde_json::from_str(&request).map_err(|e| e.to_string())?;
        if request["request"]["basis"]["phase"] != "lease-expired"
            || request["request"]["basis"]["changedFact"] != frame.fact()
        {
            continue;
        }
        let publication: JsonValue =
            serde_json::from_str(&publication).map_err(|e| e.to_string())?;
        let event = publication["eventSequence"]
            .as_i64()
            .ok_or("root_web_expiry_publication_changed")?;
        let projected=crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(db,&actor.root_run_id,event)?;
        if projected.record.get("localTools").is_some() {
            return Err("root_web_expiry_step_not_bounded".into());
        }
        native_web_expiry_require_bounded(&projected.record["summary"])?;
        if found.replace(event).is_some() {
            return Err("root_web_expiry_publication_conflict".into());
        }
    }
    Ok(found)
}
fn native_web_expiry_require_bounded(summary: &JsonValue) -> Result<(), String> {
    let suggestions = summary["suggestions"]
        .as_array()
        .ok_or("root_decision_schema_invalid")?;
    if !suggestions
        .iter()
        .all(|s| s.as_str() == Some("assess:lease_expired"))
    {
        return Err("root_web_expiry_step_not_bounded".into());
    }
    if suggestions.is_empty() {
        return Err("root_web_expiry_deferred".into());
    }
    Ok(())
}
