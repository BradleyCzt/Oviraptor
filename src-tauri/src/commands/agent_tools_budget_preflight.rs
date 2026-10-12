// Pure preflight before paid supervision. Physical claims still recheck every guard.
fn agent_target_budget_preflight(
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    name: &str,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget;
    let required: i64 = match name {
        "compare_identities" => 2,
        "replay_http" | "targeted_discovery" | "browser_action" => 1,
        _ => return Ok(()),
    };
    let connection = db::open(&context.db_path)?;
    let tx = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    // This preflight precedes only the new Multi Web supervision protocol.
    // Other paths retain their original physical admission contract.
    let Some(run) = context.run.as_ref() else {
        return Ok(());
    };
    if run.db_path != context.db_path {
        return Err("tool_authorization_unavailable".into());
    }
    let original =
        crate::agent_runtime::store::load_run(&tx, &run.run_id)?.ok_or("tool_run_not_found")?;
    if original.orchestration_policy != crate::agent_runtime::contract::MultiAgentPolicy::Multi
        || original.role != crate::agent_runtime::contract::AgentRole::WebExecutor
    {
        return Ok(());
    }
    agent_authorize_tool_on(&tx, context, name).map_err(str::to_owned)?;
    let usage = agent_request_accounting(
        &tx,
        &context.scan_id,
        context.attempt_number,
        &context.target_url,
    )?;
    let runtime_count =
        i64::try_from(runtime.target_requests).map_err(|_| "http_journal_counter_overflow")?;
    if usage.executor_recorded != runtime_count {
        return Err("http_journal_checkpoint_requires_reconciliation".into());
    }
    let ceiling = context
        .execution_plan
        .hard_model_requests
        .max(1)
        .saturating_mul(4)
        .min(400);
    let prospective = usage
        .budget_committed
        .checked_add(required)
        .ok_or("request_accounting_overflow")?;
    if prospective > ceiling {
        return Err(if required == 2 {
            "comparison_request_budget_unavailable"
        } else {
            "request_budget_exhausted"
        }
        .into());
    }
    let (lease, assignment) =
        budget::target::child_owner(&tx, &run.run_id)?.ok_or("budget_child_authority_conflict")?;
    budget::supervision_capacity::require_http_headroom_for_count(
        &tx,
        &lease,
        &assignment,
        &run.run_id,
        required,
    )?;
    budget::root::RootOwner::load_original(&tx, &lease.root_run_id)?
        .require_original_coordinator(&tx, &lease)?;
    let root = lease.root_run_id;
    budget::admission::require_determinate(&tx, &root)?;
    budget::clock::remaining(&tx, &root)?;
    let balance = budget::balance(&tx, &root, None, "target_requests")?;
    let committed = balance
        .reserved
        .checked_add(balance.consumed)
        .and_then(|n| n.checked_add(balance.indeterminate))
        .and_then(|n| n.checked_add(required))
        .ok_or("budget_amount_overflow")?;
    let limit: Option<i64> = tx.query_row(
        "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='target_requests'",
        [&root], |row|row.get(0),
    ).map_err(|_| "budget_dimension_not_configured")?;
    if limit.is_some_and(|limit| committed > limit) {
        return Err("budget_hard_limit_exceeded".into());
    }
    // No clock sample, invocation, claim, reservation or permission is created.
    // Multiple-request tools retain per-request atomic physical admission.
    Ok(())
}
