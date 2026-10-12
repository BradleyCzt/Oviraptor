// Pure admission of the exact payload; no hook, reservation or SDK.
fn native_financial_continuation_require_admission_in(
    db: &rusqlite::Connection,
    db_path: &Path,
    scan: &str,
    new_root: &str,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget;
    let (context, actor) = native_financial_continuation_context_in(db, db_path, scan, new_root)?;
    let basis = native_coordinator_basis_on(db, &context, &actor)?;
    if basis.get("financialContinuation").is_none() {
        return Err("financial_continuation_request_not_issued".into());
    }
    let (messages, specs) = native_coordinator_messages_for_basis(&basis);
    if !specs.is_empty() {
        return Err("financial_continuation_requires_plain_root".into());
    }
    let request = agent_model_request(messages, &specs);
    let tools: Vec<JsonValue> = specs.iter().map(AgentToolSpec::as_function_spec).collect();
    let bytes = serde_json::to_vec(&json!({"messages":request.messages,"tools":tools}))
        .map_err(|e| e.to_string())?
        .len();
    let profile = agent_model_profile(&context.environment, context.proxy.as_deref())?;
    let output = profile.max_output_tokens.unwrap_or(1024).min(2048);
    if output == 0 {
        return Err("root_tick_output_invalid".into());
    }
    let estimate = i64::try_from(bytes)
        .ok()
        .and_then(|b| b.checked_add(i64::try_from(output).ok()?))
        .filter(|n| *n > 0)
        .ok_or("root_tick_estimate_invalid")?;
    budget::admission::require_determinate(db, new_root)?;
    budget::limits::verify_root_contract(db, new_root)?;
    budget::root::require_child_capacity(db, new_root, estimate, 1)?;
    // RootModelCall reserves the same gross estimate independently in input,
    // cached and output classes, plus one request. Do not sum the classes as
    // another shared allowance or assume the eventual bill can skip a reserve.
    for (dimension, amount) in budget::DIMENSIONS[..4]
        .iter()
        .zip([estimate, estimate, estimate, 1])
    {
        let cap: Option<i64> = db
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits
            WHERE root_run_id=?1 AND dimension=?2",
                params![new_root, dimension],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let balance = budget::balance(db, new_root, None, dimension)?;
        let occupied = balance
            .consumed
            .checked_add(balance.reserved)
            .and_then(|n| n.checked_add(balance.indeterminate))
            .and_then(|n| n.checked_add(amount))
            .ok_or("budget_amount_overflow")?;
        if cap.is_some_and(|cap| occupied > cap) {
            return Err("budget_hard_limit_exceeded".into());
        }
    }
    budget::clock::remaining(db, new_root)?;
    Ok(())
}
