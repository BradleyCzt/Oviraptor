// Gap Investigator: fixed proposal grant only when surplus remains after Reviewer/Root floors.
fn native_coordinator_investigator_allocation(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    trigger: &str,
    revision: i64,
) -> Result<Option<(i64, i64, bool)>, String> {
    use crate::agent_runtime::{
        multi_agent::{budget, directive::proposals::PROPOSAL_TOKENS},
        store,
    };
    let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("web_mode_root_missing")?;
    let bootstrap = mode.bootstrap_dispatch().unwrap_or(JsonValue::Null);
    if bootstrap["executorAllocationRule"]
        != "current_balance_after_reviewer_floor_and_ordered_proposals"
    {
        return Ok(None);
    }
    let local = mode
        .local_deliberation()
        .ok_or("investigator_allocation_original_floor_missing")?;
    if local["reviewerTokenFloor"] != 15_000 || local["reviewerRequestFloor"] != 1 {
        return Err("investigator_allocation_original_floor_conflict".into());
    }
    budget::admission::require_determinate(db, &actor.root_run_id)?;
    let dedup = format!(
        "deep_investigator:{trigger}:{}:{revision}",
        actor.target_key
    );
    let id = format!(
        "asg-{}",
        &store::stable_hash(&format!("{}:{dedup}", actor.root_run_id))[..24]
    );
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1)",
            [&id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        let (tokens, requests): (i64, i64) = db
            .query_row(
                "SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "investigator_allocation_original_grant_missing".to_string())?;
        return Ok(Some((tokens, requests, false)));
    }
    let (token_floor, request_floor) = native_coordinator_executor_retained_floor(db, actor)?;
    let (available_tokens, available_requests) =
        budget::root::remaining_child_capacity(db, &actor.root_run_id)?;
    let need_tokens = PROPOSAL_TOKENS;
    let need_requests = 1_i64;
    if available_tokens.is_some_and(|n| n < token_floor.saturating_add(need_tokens))
        || available_requests.is_some_and(|n| n < request_floor.saturating_add(need_requests))
    {
        return Err("multi_agent_investigator_budget_unavailable".into());
    }
    Ok(Some((need_tokens, need_requests, true)))
}
