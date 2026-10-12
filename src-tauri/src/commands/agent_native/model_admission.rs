// Fresh Web model work shares the original root's budget, deadline and worker
// authority. A durable claim commits under the same authority lock before I/O.
struct NativeModelBudgetAdmission {
    lease: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    assignment: String,
    capability: String,
    remaining: Duration,
    round: i64,
    request_hash: String,
    unbounded_estimate: Option<i64>,
    call_id: String,
    _invocation: Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
}

/// A Web restart currently cannot prove atomic recovery of the provider bill,
/// legacy usage/event/checkpoint and local tools. Preserve the original facts;
/// do not renew permissions merely by reentering the executor. New attempts
/// have different children; the already-running worker is not a restart.
fn native_model_reentry_guard(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<(), String> {
    agent_require_frozen_web_plan(db, context).map_err(str::to_string)?;
    let child = &context.run.as_ref().ok_or("tool_run_not_found")?.run_id;
    let root:String=db.query_row("SELECT root_run_id FROM agent_runs WHERE id=?1 AND scan_id=?2
        AND attempt_number=?3 AND target_url=?4 AND backend='native' AND role='web_executor' AND orchestration_policy='multi'",
        params![child,context.scan_id,context.attempt_number,context.target_url],|r|r.get(0)).map_err(|_|"tool_run_not_found")?;
    crate::agent_runtime::multi_agent::budget::admission::require_determinate(db, &root)?;
    let history:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE child_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed')",
        [child],|r|r.get(0)).map_err(|e|e.to_string())?;
    if history {
        return Err("budget_history_requires_reconciliation".into());
    }
    Ok(())
}

fn native_model_budget_admission(
    context: &AgentRunContext,
    round: i64,
    request_hash: String,
) -> Result<Option<NativeModelBudgetAdmission>, String> {
    native_model_budget_admission_with_estimate(context, round, request_hash, None)
}
fn native_model_budget_admission_with_estimate(
    context: &AgentRunContext,
    round: i64,
    request_hash: String,
    unbounded_estimate: Option<i64>,
) -> Result<Option<NativeModelBudgetAdmission>, String> {
    let Some(run) = &context.run else {
        #[cfg(test)]
        {
            return Ok(None);
        }
        #[cfg(not(test))]
        {
            return Err("tool_run_not_found".into());
        }
    };
    if run.db_path != context.db_path {
        return Err("tool_authorization_unavailable".into());
    }
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let Some((lease, assignment)) =
        crate::agent_runtime::multi_agent::budget::target::child_owner(&tx, &run.run_id)?
    else {
        return Ok(None);
    };
    let capability = native_model_authorized_capability_on(&tx, context, &lease, &assignment)?;
    native_model_require_authority_on(&tx, context, &capability)?;
    crate::agent_runtime::multi_agent::budget::admission::require_determinate(
        &tx,
        &lease.root_run_id,
    )?;
    crate::agent_runtime::multi_agent::budget::clock::sample(&tx, &lease, &assignment)?;
    let remaining = native_model_remaining_capacity_on(&tx, &lease, &assignment)?;
    let mut admission=NativeModelBudgetAdmission {
        call_id:crate::agent_runtime::store::stable_hash(&serde_json::json!({"root":lease.root_run_id,
            "assignment":assignment,"child":run.run_id,"epoch":lease.lease_epoch,"fence":lease.fencing_token,
            "round":round,"requestHash":request_hash}).to_string()),
        lease,assignment,capability:capability.clone(),remaining,round,request_hash,unbounded_estimate,_invocation:None,
    };
    admission._invocation=Some(native_model_claim_on(&tx, context, &admission)?);
    native_model_require_authority_on(&tx, context, &capability)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(Some(admission))
}

// Pure-read denial before a budget-consumption assessment can spend Root fees.
// This is not an admission or a claim: the atomic admission repeats every check.
fn native_model_budget_preflight(context: &AgentRunContext) -> Result<(), String> {
    let Some(run) = &context.run else {
        #[cfg(test)]
        { return Ok(()); }
        #[cfg(not(test))]
        { return Err("tool_run_not_found".into()); }
    };
    if run.db_path != context.db_path {
        return Err("tool_authorization_unavailable".into());
    }
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Deferred)
        .map_err(|e| e.to_string())?;
    let Some((lease, assignment)) =
        crate::agent_runtime::multi_agent::budget::target::child_owner(&tx, &run.run_id)?
    else {
        return Ok(());
    };
    let capability = native_model_authorized_capability_on(&tx, context, &lease, &assignment)?;
    native_model_require_authority_on(&tx, context, &capability)?;
    crate::agent_runtime::multi_agent::budget::admission::require_determinate(
        &tx, &lease.root_run_id,
    )?;
    native_model_remaining_capacity_on(&tx, &lease, &assignment)?;
    tx.commit().map_err(|e| e.to_string())
}

fn native_model_authorized_capability_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    assignment: &str,
) -> Result<String, String> {
    let run = context.run.as_ref().ok_or("tool_run_not_found")?;
    // Reuse the broker's exact run/plan/identity/fence checks with an existing
    // capability. Each eventual tool still requires its own named capability.
    db.query_row("SELECT capability FROM agent_capability_leases
        WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5
        AND revoked_at='' AND lease_expires_at>datetime('now','localtime') ORDER BY capability LIMIT 1",
        params![assignment,run.run_id,lease.root_run_id,lease.lease_epoch,lease.fencing_token],|r|r.get(0))
        .map_err(|_|"tool_capability_or_fencing_denied".into())
}

fn native_model_require_authority_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    capability: &str,
) -> Result<(), String> {
    let run = context.run.as_ref().ok_or("tool_run_not_found")?;
    agent_authorize_tool_on(db, context, capability).map_err(str::to_string)?;
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_runs root ON root.id=r.root_run_id
        JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id WHERE r.id=?1
        AND r.cancel_requested_at='' AND root.cancel_requested_at='' AND a.budget_settled_at='')",
        [&run.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {
        return Err("tool_capability_or_fencing_denied".into());
    }
    Ok(())
}

fn native_model_remaining_capacity_on(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    assignment: &str,
) -> Result<Duration, String> {
    let remaining =
        crate::agent_runtime::multi_agent::budget::clock::remaining(db, &lease.root_run_id)?;
    let remaining_requests = crate::agent_runtime::multi_agent::budget::balance(
        db, &lease.root_run_id, Some(assignment), "model_requests",
    )?.reserved;
    let unbounded = native_model_unbounded_dimensions_on(db, &lease.root_run_id)?;
    if remaining_requests < 1 && !unbounded.is_some_and(|flags|flags[3]) {
        return Err("budget_model_requests_exhausted".into());
    }
    Ok(remaining)
}

// Unknown original Root obligations precede local Web limit/completion exits.
// Pure reads: no claim, reserve, lease renewal, refund or execution authority.
fn native_web_require_determinate(context: &AgentRunContext) -> Result<(), String> {
    let Some(run) = context.run.as_ref() else {
        return Ok(());
    };
    let db = db::open(&context.db_path)?;
    let row =
        crate::agent_runtime::store::load_run(&db, &run.run_id)?.ok_or("tool_run_not_found")?;
    if row.orchestration_policy != crate::agent_runtime::contract::MultiAgentPolicy::Multi
        || row.role != crate::agent_runtime::contract::AgentRole::WebExecutor
    {
        return Ok(());
    }
    if row.scan_id != context.scan_id
        || row.attempt_number != context.attempt_number
        || row.target_url != context.target_url
    {
        return Err("tool_run_scope_mismatch".into());
    }
    crate::agent_runtime::multi_agent::budget::admission::require_determinate(&db, &row.root_run_id)
}
