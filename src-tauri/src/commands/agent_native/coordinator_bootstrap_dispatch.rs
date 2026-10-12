// Mapper issuance consumes a paid initial Root choice, never advisory prose alone.
#[cfg(test)]
fn native_coordinator_prepare_mapper(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    decision: &NativeCoordinatorTickReceipt,
    task: &JsonValue,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    native_coordinator_prepare_mapper_with_loop_bounds(context, actor, decision, task, None)
}

fn native_coordinator_prepare_mapper_with_loop_bounds(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    decision: &NativeCoordinatorTickReceipt,
    task: &JsonValue,
    loop_bounds: Option<&mapper_loop_request::VerifiedMapperLoopRequestBounds>,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler, store};
    let db = db::open(&context.db_path)?;
    if crate::agent_runtime::multi_agent::budget::root::model::tick::financial_purpose::binding_on(
        &db,
        &actor.root_run_id,
    )?
    .is_some()
    {
        return Err("financial_continuation_child_dispatch_denied".into());
    }
    let mode = crate::agent_runtime::web_mode::root::read(&db, &actor.root_run_id)?
        .ok_or("web_mode_root_missing")?;
    if mode.bootstrap_dispatch().is_none() {
        // Original Native contracts keep their frozen scheduling semantics.
        return scheduler::prepare_supervised_readonly_child(
            &db,
            actor,
            AgentRole::SpaApiMapper,
            "frontend_evidence_ready",
            task,
            8_000,
        );
    }
    let step = "dispatch:spa_api_mapper";
    let summary = decision.summary.as_json();
    if decision.saved.local_step.is_some()
        || !summary["suggestions"]
            .as_array()
            .is_some_and(|xs| xs.iter().any(|x| x.as_str() == Some(step)))
    {
        return Err("root_bootstrap_did_not_select_mapper".into());
    }
    let mut original = decision.tick.basis().clone();
    let object = original
        .as_object_mut()
        .ok_or("root_bootstrap_basis_invalid")?;
    for key in [
        "localParents",
        "localHistory",
        "localRound",
        "localCallsUsed",
        "budgetSnapshot",
    ] {
        object.remove(key);
    }
    if original["phase"] != "bootstrap"
        || original["permittedSuggestion"] != step
        || original["bootstrapDispatch"] != mode.bootstrap_dispatch().unwrap()
    {
        return Err("root_bootstrap_original_choice_conflict".into());
    }
    if original["bootstrapDispatch"]["schemaVersion"] == 15 {
        return native_coordinator_prepare_mapper_loop_allocated(
            &db, context, actor, decision, task, &original,
            loop_bounds.ok_or("mapper_loop_original_request_bounds_required")?,
        );
    }
    if loop_bounds.is_some() {
        return Err("mapper_loop_legacy_contract_upgrade_denied".into());
    }
    if original["bootstrapDispatch"]["allocationRule"] == "current_balance_after_reviewer_floor" {
        return native_coordinator_prepare_mapper_allocated(
            &db, context, actor, decision, task, &original,
        );
    }
    let mut task = task.clone();
    let object = task.as_object_mut().ok_or("root_decision_task_invalid")?;
    if object.contains_key("rootDecision") {
        return Err("root_decision_task_shadow".into());
    }
    object.insert("rootDecision".into(),json!({"step":step,"eventSequence":decision.event_sequence,
        "summaryHash":store::stable_hash(&summary.to_string()),"basisHash":store::stable_hash(&original.to_string()),
        "rustPolicy":{"schemaVersion":1,"trigger":"initial_frozen_evidence",
            "selection":"accepted","reasonCode":"frozen_frontend_inventory_requires_independent_mapper",
            "targetRequestsGranted":0,"reservedModelTokens":8000,"reservedModelRequests":1,
            "targetEvidenceProven":false}}));
    let verify = |db: &rusqlite::Connection| -> Result<(), String> {
        crate::collaboration_events::dispatch_schema::verify(db)?;
        native_coordinator_tick_authority(db, context, actor)?;
        native_coordinator_verify_original_basis(db, context, actor, &original)?;
        decision.tick.require_executable(db)?;
        if decision.tick.published(db, &decision.saved)? != Some(decision.event_sequence) {
            return Err("root_decision_publication_changed".into());
        }
        Ok(())
    };
    db.authorizer(Some(native_coordinator_bootstrap_authorize))
        .map_err(|e| e.to_string())?;
    let result = scheduler::prepare_readonly_child_checked(
        &db,
        actor,
        AgentRole::SpaApiMapper,
        "frontend_evidence_ready",
        &task,
        8000,
        verify,
    );
    use rusqlite::hooks::{AuthContext, Authorization};
    let clear = db
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let child = result?;
    clear?;
    Ok(child)
}

fn native_coordinator_bootstrap_authorize(
    c: rusqlite::hooks::AuthContext<'_>,
) -> rusqlite::hooks::Authorization {
    use rusqlite::hooks::{AuthAction, Authorization};
    let direct = c.database_name == Some("main") && c.accessor.is_none();
    let start = direct
        && matches!(
            c.action,
            AuthAction::Update {
                table_name: "agent_runs" | "agent_assignments",
                column_name: "started_at",
            } | AuthAction::Update {
                table_name: "agent_assignment_attempts",
                column_name: "state" | "heartbeat_at",
            }
        );
    if start {
        Authorization::Allow
    } else {
        native_coordinator_dispatch_authorize(c)
    }
}

include!("coordinator_mapper_allocation.rs");
