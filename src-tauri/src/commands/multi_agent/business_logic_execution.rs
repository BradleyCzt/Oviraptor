// Original v14 readonly candidate followup. Full General ReAct remains separate.
#[cfg(test)]
thread_local! {
    static BUSINESS_LOGIC_FOLLOWUP_DISABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn with_business_logic_followup_disabled_for_test<T>(f: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            BUSINESS_LOGIC_FOLLOWUP_DISABLED.with(|flag| flag.set(self.0));
        }
    }
    let reset = Reset(BUSINESS_LOGIC_FOLLOWUP_DISABLED.with(|flag| flag.replace(true)));
    let value = f();
    drop(reset);
    value
}

fn multi_agent_business_logic_followup(
    context: &AgentRunContext,
    session: &MultiAgentSession,
) -> Result<(), String> {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    #[cfg(test)]
    if BUSINESS_LOGIC_FOLLOWUP_DISABLED.with(|flag| flag.get()) { return Ok(()); }
    session.supervisor.check()?;
    if session.mapper.role != AgentRole::SpaApiMapper { return Ok(()); }
    let db = db::open(&context.db_path)?;
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let Some(mode) = crate::agent_runtime::web_mode::root::read(&tx, &session.lease.root_run_id)? else {
        return Ok(());
    };
    if !mode.bootstrap_dispatch().is_some_and(|p| matches!(p["schemaVersion"].as_i64(), Some(14 | 15))) {
        return Ok(());
    }
    let actor = &session.lease;
    let input = native_business_logic_input_collection_on(&tx, actor, &context.target_dir)?;
    if !native_business_logic_dispatch_eligible(&input) { return Ok(()); }
    let missing: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE (a.coordinator_run_id=?1 OR r.root_run_id=?1) AND (a.role='business_logic' OR r.role='business_logic') AND NOT EXISTS(SELECT 1 FROM agent_business_logic_dispatch_receipts d WHERE d.root_run_id=?1 AND d.assignment_id=a.id AND d.child_run_id=r.id))",
        [&actor.root_run_id], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if missing { return Err("business_logic_dispatch_binding_missing".into()); }
    let existing = tx.prepare("SELECT assignment_id,child_run_id FROM agent_business_logic_dispatch_receipts WHERE root_run_id=?1")
        .map_err(|e| e.to_string())?
        .query_map([&actor.root_run_id], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?)))
        .map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())?;
    if existing.len() > 1 { return Err("business_logic_original_worker_conflict".into()); }
    if let Some((assignment_id, run_id)) = existing.into_iter().next() {
        let child = scheduler::ScheduledChild { assignment_id, run_id, role: AgentRole::BusinessLogic };
        business_logic_dispatch_receipt::verify_original_on(&tx, actor, &child)?;
        let state: String = tx.query_row("SELECT state FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        tx.rollback().map_err(|e| e.to_string())?;
        if state == "completed" {
            multi_agent_business_logic_readonly(context, actor)?;
        } else {
            multi_agent_business_logic_execute_prepared(context, actor, &child)?;
            native_coordinator_business_logic_output_feedback(context, actor, &child)?;
        }
        return session.supervisor.check();
    }
    let frame = NativeCoordinatorFrame::business_logic_input_on(&tx, actor, &context.target_dir)?;
    tx.rollback().map_err(|e| e.to_string())?;
    let root = native_coordinator_root_context(context, actor);
    let decision = native_coordinator_tick_for_frame(&root, actor, &frame)?;
    if decision.saved.local_step.is_some()
        || !native_root_paid_suggestion_selected(&decision, "dispatch:business_logic") {
        return Ok(());
    }
    session.supervisor.check()?;
    let child = native_coordinator_mint_business_logic_child(context, actor, &decision)?;
    session.supervisor.check()?;
    multi_agent_business_logic_execute_prepared(context, actor, &child)?;
    native_coordinator_business_logic_output_feedback(context, actor, &child)?;
    session.supervisor.check()
}
