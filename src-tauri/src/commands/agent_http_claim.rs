// Same private journal transaction: original claim and budget become durable together.
fn claim_agent_http_request_on(
    tx: &rusqlite::Transaction<'_>,
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    request: &AgentHttpRequest,
    url: &str,
    headers: &[(String, String)],
) -> Result<AgentHttpClaim, String> {
    if agent_protection_stop_on(tx, context)?.is_some() {
        return Err("http_protection_stop_active".into());
    }
    let run = context.run.as_ref().ok_or("http_journal_run_required")?;
    agent_authorize_tool_on(tx, context, &request.tool).map_err(str::to_owned)?;
    let (root_owner, _prior_single_http_idle) = if native_single_policy(tx, context)? {
        let owner = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(
            tx,
            &run.run_id,
        )?;
        owner.require_live(tx)?;
        let idle=crate::agent_runtime::multi_agent::budget::root::target::lifetime::require_idle_original(tx,&owner)?;
        (Some(owner),idle)
    } else {
        (None,vec![])
    };
    if runtime.cancelled() {
        return Err("cancelled".into());
    }
    let running: bool = tx.query_row(
        "SELECT COUNT(*)=1 FROM tool_invocations WHERE run_id=?1 AND invocation_id=?2 AND tool_name=?3 AND status='running' AND policy_decision='allow'",
        params![run.run_id,runtime.current_invocation,request.tool],|r|r.get(0),
    ).map_err(|_| "http_journal_invocation_unavailable")?;
    if !running {
        return Err("http_journal_invocation_not_running".into());
    }
    let usage = agent_request_accounting(
        tx,
        &context.scan_id,
        context.attempt_number,
        &context.target_url,
    )?;
    let maximum = context
        .execution_plan
        .hard_model_requests
        .max(1)
        .saturating_mul(4)
        .min(400);
    if usage.budget_committed >= maximum {
        return Err("request_budget_exhausted".into());
    }
    let runtime_count =
        i64::try_from(runtime.target_requests).map_err(|_| "http_journal_counter_overflow")?;
    if usage.executor_recorded != runtime_count {
        return Err("http_journal_checkpoint_requires_reconciliation".into());
    }
    if let Some((lease, assignment)) =
        crate::agent_runtime::multi_agent::budget::target::child_owner(tx, &run.run_id)?
    {
        crate::agent_runtime::multi_agent::budget::supervision_capacity::require_http_headroom(
            tx, &lease, &assignment, &run.run_id,
        )?;
    }
    let anchor = *usage
        .attempts
        .last()
        .ok_or("http_journal_lineage_missing")?;
    // Hash logical wire inputs only. No raw URL queries, bodies, cookies or
    // authentication material are copied into this bookkeeping table.
    let mut wire_url = reqwest::Url::parse(url).map_err(|_| "http_journal_url_invalid")?;
    wire_url.set_fragment(None);
    let request_hash = crate::agent_runtime::store::stable_hash(&serde_json::json!({
        "method":request.method.to_ascii_uppercase(),"url":wire_url.as_str(),"identity":request.identity.key,
        "body":request.body.as_deref().filter(|body|!body.is_empty()),
        "headers":agent_http_valid_wire_headers(headers),
    }).to_string());
    let pending: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=?3 AND request_hash=?4 AND response_status=0)",
        params![context.scan_id,context.target_url,anchor,request_hash],|r|r.get(0),
    ).map_err(|_| "http_journal_pending_unavailable")?;
    if pending {
        return Err("http_journal_unknown_request_not_replayable".into());
    }
    tx.execute(
        "INSERT INTO agent_http_budget_origins(scan_id,target_url,budget_attempt,baseline_requests) VALUES(?1,?2,?3,?4) ON CONFLICT(scan_id,target_url,budget_attempt) DO NOTHING",
        params![context.scan_id,context.target_url,anchor,usage.executor_recorded],
    ).map_err(|_| "http_journal_origin_write_failed")?;
    let mut claim = AgentHttpClaim {
        run_id: run.run_id.clone(),
        invocation_id: runtime.current_invocation.clone(),
        request_index: i64::try_from(runtime.current_request_index)
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or("http_journal_counter_overflow")?,
        ordinal: usage
            .executor_recorded
            .checked_add(1)
            .ok_or("http_journal_counter_overflow")?,
        budget_attempt: anchor,
        request_hash,
        root_target: None,
        _single_target_lifetime: None,
    };
    let changed = tx.execute(
        "INSERT INTO agent_http_request_claims(scan_id,target_url,budget_attempt,ordinal,attempt_number,run_id,invocation_id,request_index,tool_name,identity_handle,request_hash)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![context.scan_id,context.target_url,anchor,claim.ordinal,context.attempt_number,run.run_id,
            claim.invocation_id,claim.request_index,request.tool,request.identity.key,claim.request_hash],
    ).map_err(|_| "http_journal_claim_write_failed")?;
    let bound: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=?3 AND ordinal=?4
         AND attempt_number=?5 AND run_id=?6 AND invocation_id=?7 AND request_index=?8 AND tool_name=?9 AND identity_handle=?10 AND request_hash=?11
         AND response_status=0 AND received_at='')",
        params![context.scan_id,context.target_url,anchor,claim.ordinal,context.attempt_number,run.run_id,
            claim.invocation_id,claim.request_index,request.tool,request.identity.key,claim.request_hash],|r|r.get(0),
    ).map_err(|_| "http_journal_claim_verification_failed")?;
    if changed != 1
        || !bound
        || agent_http_journal_usage(tx, &context.scan_id, &context.target_url, &usage.attempts)?
            != Some((claim.ordinal, usage.executor_unresolved + 1))
    {
        return Err("http_journal_claim_verification_failed".into());
    }
    if let Some((lease, assignment)) =
        crate::agent_runtime::multi_agent::budget::target::child_owner(tx, &claim.run_id)?
    {
        crate::agent_runtime::multi_agent::budget::target::claim(
            tx,
            &lease,
            &assignment,
            &agent_http_budget_source(&claim),
        )?;
    } else if let Some(owner) = root_owner {
        use crate::agent_runtime::multi_agent::budget::root::target::{
            RootTargetCall, RootTargetRequest,
        };
        let (original,lifetime) = RootTargetCall::claim(
            tx,
            owner,
            RootTargetRequest {
                scan: context.scan_id.clone(),
                attempt: context.attempt_number,
                target: context.target_url.clone(),
                budget_attempt: claim.budget_attempt,
                ordinal: claim.ordinal,
                invocation: claim.invocation_id.clone(),
                request_index: claim.request_index,
                request_hash: claim.request_hash.clone(),
            },
            !AGENT_READ_METHODS.contains(&request.method.as_str()),
            agent_http_upload_bytes(headers, request.body.as_deref()),
        )?;
        claim.root_target=Some(original);
        claim._single_target_lifetime=Some(lifetime);
    }
    agent_authorize_tool_on(tx, context, &request.tool).map_err(str::to_owned)?;
    if let Some(original) = &claim.root_target {
        original.require_executable(tx)?;
    }
    if agent_protection_stop_on(tx, context)?.is_some() {
        return Err("http_protection_stop_active".into());
    }
    Ok(claim)
}

fn receive_agent_http_headers_on(
    tx: &rusqlite::Transaction<'_>,
    context: &AgentRunContext,
    claim: &AgentHttpClaim,
    status: u16,
) -> Result<(), String> {
    // A late response may settle a claim after cancellation; this does not grant
    // permission to dispatch anything, reactivate the task, or publish evidence.
    let changed = tx.execute(
        "UPDATE agent_http_request_claims SET response_status=?1,received_at=datetime('now','localtime')
         WHERE scan_id=?2 AND target_url=?3 AND budget_attempt=?4 AND ordinal=?5 AND run_id=?6 AND invocation_id=?7
           AND request_index=?8 AND request_hash=?9 AND attempt_number=?10 AND response_status=0 AND received_at=''",
        params![status,context.scan_id,context.target_url,claim.budget_attempt,claim.ordinal,claim.run_id,claim.invocation_id,claim.request_index,claim.request_hash,context.attempt_number],
    ).map_err(|_| "http_journal_receipt_write_failed")?;
    if changed != 1 {
        return Err("http_journal_receipt_verification_failed".into());
    }
    let proof = agent_http_header_receipt(tx, context, claim, status)?;
    if let Some(original) = &claim.root_target {
        original.receive(tx, status)?;
    } else {
        crate::agent_runtime::multi_agent::budget::target::receive_for_run(
            tx,
            &claim.run_id,
            &agent_http_budget_source(claim),
        )?;
    }
    if agent_http_header_receipt(tx, context, claim, status)? != proof {
        return Err("http_journal_receipt_verification_failed".into());
    }
    Ok(())
}
