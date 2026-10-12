// Foreground guard before fresh Web SDK/tool actions, never inside bill writes.
fn native_coordinator_budget_consumption_before_action(
    context: &AgentRunContext,
) -> Result<(), String> {
    use crate::agent_runtime::{
        contract::{AgentRole, MultiAgentPolicy},
        multi_agent::{budget, scheduler},
        store,
    };
    let Some(run) = context.run.as_ref() else {
        return Ok(());
    };
    if run.db_path != context.db_path {
        return Err("tool_authorization_unavailable".into());
    }
    let db = db::open(&context.db_path)?;
    let current =
        store::load_run(&db, &run.run_id)?.ok_or("root_budget_consumption_child_missing")?;
    if current.role != AgentRole::WebExecutor
        || current.orchestration_policy != MultiAgentPolicy::Multi
    {
        return Ok(());
    }
    let declaration = crate::agent_runtime::web_mode::root::read(&db, &current.root_run_id)?
        .ok_or("root_budget_consumption_original_protocol_missing")?;
    // An unsigned cold rewrite into an older-looking declaration must not
    // disable a protocol that was sealed when this Root was born.
    let original = original_root_capture_mode(&db, &current.root_run_id)?;
    if original.fact() != declaration.fact()
        || declaration.mode() != crate::agent_runtime::web_mode::WebMode::Multi
    {
        return Err("root_budget_consumption_original_protocol_missing".into());
    }
    // Historical Roots retain their original protocol; never backfill it.
    if declaration
        .bootstrap_dispatch()
        .unwrap_or(JsonValue::Null)
        .get("budgetConsumptionRule")
        .is_none()
    {
        return Ok(());
    }
    let actor = context
        .supervision
        .as_ref()
        .ok_or("worker_supervisor_stopped")?
        .original_actor_for_run(
            &db,
            &context.db_path,
            &context.scan_id,
            context.attempt_number,
            &context.target_url,
            &run.run_id,
        )?;
    let child = scheduler::ScheduledChild {
        assignment_id: current.assignment_id,
        run_id: run.run_id.clone(),
        role: AgentRole::WebExecutor,
    };
    let root = native_coordinator_root_context(context, &actor);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_budget_consumption_protocol_on(&tx, &actor)?;
    native_coordinator_tick_authority(&tx, &root, &actor)?;
    budget::admission::require_determinate(&tx, &actor.root_run_id)?;
    let calls=tx.prepare("SELECT call_id FROM agent_web_model_journal
        WHERE root_run_id=?1 AND assignment_id=?2 AND child_run_id=?3 AND phase='received' ORDER BY round,call_id")
        .map_err(|e|e.to_string())?.query_map(params![actor.root_run_id,child.assignment_id,child.run_id],|r|r.get::<_,String>(0))
        .map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    let unsettled: bool = db
        .query_row(
            "SELECT budget_settled_at='' FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !unsettled {
        return Err("root_budget_consumption_worker_already_settled".into());
    }
    native_budget_consumption_assess_calls(&db, context, &actor, &child, calls)
}

fn native_budget_consumption_assess_calls(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    calls: Vec<String>,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget;
    let root = native_coordinator_root_context(context, actor);
    for call in calls {
        let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        budget::admission::require_determinate(&tx, &actor.root_run_id)?;
        let frame =
            NativeCoordinatorFrame::budget_consumption_on(&tx, context, actor, child, &call)?;
        native_supervision_frame_admission(frame.kind)?;
        native_coordinator_basis_for_frame_on(&tx, &root, actor, Some(&frame))?;
        tx.commit().map_err(|e| e.to_string())?;
        let decision = native_coordinator_tick_for_frame(&root, actor, &frame)?;
        let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        context
            .supervision
            .as_ref()
            .ok_or("worker_supervisor_stopped")?
            .check_run(
                &tx,
                &context.db_path,
                &context.scan_id,
                context.attempt_number,
                &context.target_url,
                &child.run_id,
            )?;
        native_coordinator_tick_authority(&tx, &root, actor)?;
        frame.verify(&tx, context, actor)?;
        decision.tick.require_executable(&tx)?;
        budget::admission::require_determinate(&tx, &actor.root_run_id)?;
        if decision.tick.published(&tx, &decision.saved)? != Some(decision.event_sequence) {
            return Err("root_budget_consumption_publication_changed".into());
        }
        let summary = decision.summary.as_json();
        let suggestions = summary["suggestions"]
            .as_array()
            .ok_or("root_decision_schema_invalid")?;
        if !suggestions
            .iter()
            .all(|s| s.as_str() == Some("assess:budget_consumption"))
        {
            return Err("root_budget_consumption_step_not_bounded".into());
        }
        if suggestions.is_empty() {
            return Err("root_budget_consumption_deferred".into());
        }
        tx.commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Final settlement supplies no permission for another Web action. A completed
// replay can only audit its already published advisory, never create a Root call.
fn native_coordinator_budget_consumption_after_settlement(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    payload: &JsonValue,
    summary: &str,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget;
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let original = original_root_capture_mode(&tx, &actor.root_run_id)?;
    let declaration = crate::agent_runtime::web_mode::root::read(&tx, &actor.root_run_id)?
        .ok_or("root_budget_consumption_original_protocol_missing")?;
    if original.fact() != declaration.fact()
        || declaration.mode() != crate::agent_runtime::web_mode::WebMode::Multi
    {
        return Err("root_budget_consumption_original_protocol_missing".into());
    }
    if declaration
        .bootstrap_dispatch()
        .unwrap_or(JsonValue::Null)
        .get("budgetConsumptionRule")
        .is_none()
    {
        return Ok(());
    }
    native_budget_consumption_protocol_on(&tx, actor)?;
    let root = native_coordinator_root_context(context, actor);
    native_coordinator_tick_authority(&tx, &root, actor)?;
    budget::admission::require_determinate(&tx, &actor.root_run_id)?;
    let calls=tx.prepare("SELECT call_id FROM agent_web_model_journal WHERE root_run_id=?1 AND assignment_id=?2 AND child_run_id=?3 AND phase='received' ORDER BY round,call_id").map_err(|e|e.to_string())?
        .query_map(params![actor.root_run_id,child.assignment_id,child.run_id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    if original_executor_finished_historical_on(&tx, actor, child, payload, summary)? {
        for call in calls {
            let frame = NativeCoordinatorFrame::budget_consumption_original_on(
                &tx, context, actor, child, &call,
            )?;
            native_budget_consumption_verify_final_publication(&tx, actor, &frame)?;
        }
        return Ok(());
    }
    let settled:bool=tx.query_row("SELECT budget_settled_at<>'' AND state='running' FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2",params![child.assignment_id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !settled {
        return Err("root_budget_consumption_settled_worker_required".into());
    }
    crate::agent_runtime::multi_agent::scheduler::verify_original_scheduled_child(
        &tx, actor, child,
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    native_budget_consumption_assess_calls(&db, context, actor, child, calls)
}

fn native_budget_consumption_verify_final_publication(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: &NativeCoordinatorFrame,
) -> Result<(), String> {
    let rows=db.prepare("SELECT q.fact_json,p.fact_json FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.root_run_id=q.root_run_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request'").map_err(|e|e.to_string())?
        .query_map([&actor.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    for (request, publication) in rows {
        let request: JsonValue = serde_json::from_str(&request).map_err(|e| e.to_string())?;
        if request["request"]["basis"]["changedFact"] != frame.fact() {
            continue;
        }
        let publication: JsonValue =
            serde_json::from_str(&publication).map_err(|e| e.to_string())?;
        let event = publication["eventSequence"]
            .as_i64()
            .ok_or("root_budget_consumption_publication_changed")?;
        let projected=crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(db,&actor.root_run_id,event)?;
        let suggestions = projected.record["summary"]["suggestions"]
            .as_array()
            .ok_or("root_decision_schema_invalid")?;
        if projected.record.get("localTools").is_some()
            || suggestions.is_empty()
            || !suggestions
                .iter()
                .all(|s| s.as_str() == Some("assess:budget_consumption"))
        {
            return Err("root_budget_consumption_step_not_bounded".into());
        }
        return Ok(());
    }
    Err("root_budget_consumption_final_publication_missing".into())
}
