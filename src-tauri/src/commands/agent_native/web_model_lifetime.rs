// Original per-child SDK proof lives with the financial runtime.
use crate::agent_runtime::multi_agent::budget::web_lifetime::WEB_MODEL_INVOCATION_KIND;
fn native_web_model_idle_original(
    db: &rusqlite::Connection,
    scope: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>, String> {
    crate::agent_runtime::multi_agent::budget::web_lifetime::require_idle_original(db, scope)
}

// New-protocol worker lifetime spans every model/tool round, including the gaps
// between SDK calls. A probe of its original inode proves actual loop return.
fn native_web_original_execution_owner(
    context: &AgentRunContext,
) -> Result<Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>, String> {
    let Some(run) = &context.run else { return Ok(None); };
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e|e.to_string())?;
    let Some((actor, assignment)) = crate::agent_runtime::multi_agent::budget::target::child_owner(&tx, &run.run_id)?
        else { return Ok(None); };
    let mode = crate::agent_runtime::web_mode::root::read(&tx, &actor.root_run_id)?;
    if mode.is_none_or(|m|m.new_evidence_supervision().is_none()) { return Ok(None); }
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&tx, &actor)?;
    crate::agent_runtime::multi_agent::lease::require_executable_coordinator(&tx, &actor)?;
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&tx, &actor.root_run_id)?
        .require_original_coordinator(&tx, &actor)?;
    native_model_reentry_guard(&tx, context)?;
    crate::agent_runtime::multi_agent::budget::receipts::original_owner(
        &tx, &run.run_id, &actor, &assignment,
    )?.verify(&tx)?;
    let owner = crate::agent_runtime::execution_owner::claim_native_invocation(
        &context.db_path, &context.scan_id, context.attempt_number,
        "web-executor-original-execution", &run.run_id,
    )?;
    tx.commit().map_err(|e|e.to_string())?;
    Ok(Some(owner))
}
