// Foreground boundaries consume only already sealed HTTP header bills.
fn native_coordinator_http_budget_consumption_before_action(
    context: &AgentRunContext,
) -> Result<(), String> {
    use crate::agent_runtime::{
        contract::{AgentRole, MultiAgentPolicy},
        multi_agent::scheduler,
        store,
    };
    let Some(run) = context.run.as_ref() else {
        return Ok(());
    };
    if run.db_path != context.db_path {
        return Err("tool_authorization_unavailable".into());
    }
    let db = db::open(&context.db_path)?;
    let current = store::load_run(&db, &run.run_id)?.ok_or("root_http_budget_child_missing")?;
    if current.role != AgentRole::WebExecutor
        || current.orchestration_policy != MultiAgentPolicy::Multi
    {
        return Ok(());
    }
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if !native_http_budget_protocol_enabled_on(&tx, &current.root_run_id)? {
        return Ok(());
    }
    let actor = context
        .supervision
        .as_ref()
        .ok_or("worker_supervisor_stopped")?
        .original_actor_for_run(
            &tx,
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
    scheduler::verify_original_scheduled_child(&tx, &actor, &child)?;
    let unsettled: bool = tx
        .query_row(
            "SELECT budget_settled_at='' FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !unsettled {
        return Err("root_http_budget_worker_already_settled".into());
    }
    let sources = native_http_budget_received_sources_on(&tx, &actor, &child)?;
    tx.commit().map_err(|e| e.to_string())?;
    native_http_budget_assess_sources(&db, context, &actor, &child, sources)
}

fn native_coordinator_http_budget_consumption_after_settlement(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    payload: &JsonValue,
    summary: &str,
) -> Result<(), String> {
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if !native_http_budget_protocol_enabled_on(&tx, &actor.root_run_id)? {
        return Ok(());
    }
    let root = native_coordinator_root_context(context, actor);
    native_coordinator_tick_authority(&tx, &root, actor)?;
    let sources = native_http_budget_received_sources_on(&tx, actor, child)?;
    if original_executor_finished_historical_on(&tx, actor, child, payload, summary)? {
        for source in sources {
            let frame = NativeCoordinatorFrame::http_budget_consumption_original_on(
                &tx, context, actor, child, &source,
            )?;
            native_http_budget_verify_final_publication(&tx, actor, &frame)?;
        }
        return Ok(());
    }
    let settled:bool=tx.query_row("SELECT state='running' AND budget_settled_at<>'' FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2",params![child.assignment_id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !settled {
        return Err("root_http_budget_settled_worker_required".into());
    }
    crate::agent_runtime::multi_agent::scheduler::verify_original_scheduled_child(
        &tx, actor, child,
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    native_http_budget_assess_sources(&db, context, actor, child, sources)
}

fn native_http_budget_received_sources_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<Vec<String>, String> {
    if db.is_autocommit() {
        return Err("root_http_budget_snapshot_required".into());
    }
    crate::agent_runtime::multi_agent::budget::admission::require_determinate(
        db,
        &actor.root_run_id,
    )?;
    let root_owned = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        db,
        &actor.root_run_id,
    )?;
    root_owned.require_original_coordinator(db, actor)?;
    let mut sources=db.prepare("SELECT 'http:'||run_id||':'||invocation_id||':'||request_index||':'||request_hash FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND attempt_number=?3 AND run_id=?4 AND response_status>0 AND received_at<>'' ORDER BY budget_attempt,ordinal").map_err(|e|e.to_string())?
        .query_map(params![actor.scan_id,actor.target_key,actor.attempt_number,child.run_id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    // A deleted/cold-moved claim cannot erase an immutable sealed bill or its
    // original worker's charge from the drain inventory.
    crate::agent_runtime::web_mode::http_budget_storage::verify_schema_on(db)?;
    let worker =
        crate::agent_runtime::multi_agent::attempts::current(db, actor, &child.assignment_id)?;
    if worker.child_run_id != child.run_id {
        return Err("root_http_budget_original_worker_conflict".into());
    }
    let prefix = format!("http:{}:", child.run_id);
    let other_sources=db.prepare("SELECT source_id FROM main.agent_http_budget_receipts WHERE root_run_id=?1 AND run_id=?2 UNION SELECT source_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?3 AND lease_attempt_id=?4 AND dimension='target_requests' AND substr(source_id,1,length(?5))=?5 ORDER BY source_id")
        .map_err(|e|e.to_string())?.query_map(params![actor.root_run_id,child.run_id,child.assignment_id,worker.id,prefix],|r|r.get::<_,String>(0))
        .map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    for source in other_sources {
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
    for source in &sources {
        let receipt = crate::agent_runtime::web_mode::http_budget_storage::read_original_on(
            db, actor, source,
        )?
        .ok_or("root_http_budget_original_receipt_missing")?;
        if receipt.fact()["claim"]["runId"] != child.run_id {
            return Err("root_http_budget_original_scope_conflict".into());
        }
    }
    Ok(sources)
}

fn native_http_budget_assess_sources(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    sources: Vec<String>,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget;
    let root = native_coordinator_root_context(context, actor);
    for source in sources {
        let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        budget::admission::require_determinate(&tx, &actor.root_run_id)?;
        let frame = NativeCoordinatorFrame::http_budget_consumption_on(
            &tx, context, actor, child, &source,
        )?;
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
            return Err("root_http_budget_publication_changed".into());
        }
        let summary = decision.summary.as_json();
        let suggestions = summary["suggestions"]
            .as_array()
            .ok_or("root_decision_schema_invalid")?;
        if !suggestions
            .iter()
            .all(|s| s.as_str() == Some("assess:http_budget_consumption"))
        {
            return Err("root_http_budget_step_not_bounded".into());
        }
        if suggestions.is_empty() {
            return Err("root_http_budget_deferred".into());
        }
        tx.commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn native_http_budget_verify_final_publication(
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
            .ok_or("root_http_budget_publication_changed")?;
        let projected=crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(db,&actor.root_run_id,event)?;
        let suggestions = projected.record["summary"]["suggestions"]
            .as_array()
            .ok_or("root_decision_schema_invalid")?;
        if projected.record.get("localTools").is_some()
            || suggestions.is_empty()
            || !suggestions
                .iter()
                .all(|s| s.as_str() == Some("assess:http_budget_consumption"))
        {
            return Err("root_http_budget_step_not_bounded".into());
        }
        return Ok(());
    }
    Err("root_http_budget_final_publication_missing".into())
}
