/// A response receipt here records receipt of HTTP headers, not complete body
/// capture, successful execution, or verified evidence. Claimed requests always
/// remain charged, including crashes before send and responses lost in flight.
#[derive(Debug)]
struct AgentHttpClaim {
    run_id: String,
    invocation_id: String,
    request_index: i64,
    ordinal: i64,
    budget_attempt: i64,
    request_hash: String,
    root_target: Option<crate::agent_runtime::multi_agent::budget::root::target::RootTargetCall>,
    _single_target_lifetime: Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
}
include!("agent_http_claim.rs");
include!("agent_http_wire.rs");

fn agent_http_budget_source(claim: &AgentHttpClaim) -> String {
    format!(
        "http:{}:{}:{}:{}",
        claim.run_id, claim.invocation_id, claim.request_index, claim.request_hash
    )
}

fn agent_http_claim_error(reason: String) -> JsonValue {
    let code = match reason.as_str() {
        "http_journal_checkpoint_requires_reconciliation"
        | "http_journal_unknown_request_not_replayable"
        | "budget_indeterminate_requires_reconciliation"
        | "budget_history_requires_reconciliation"
        | "budget_clock_origin_missing" => terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        "budget_hard_limit_exceeded" => "request_budget_exhausted",
        "budget_wall_time_exhausted" => "wall_time_budget_exhausted",
        "budget_child_authority_conflict"
        | "budget_root_authority_conflict"
        | "budget_assignment_scope_conflict"
        | "budget_operation_not_granted"
        | "budget_execution_policy_invalid" => "tool_capability_or_fencing_denied",
        "cancelled"
        | "root_supervision_request_capacity_exhausted"
        | "request_budget_exhausted"
        | "agent_attempt_not_active"
        | "tool_execution_surface_denied"
        | "tool_execution_plan_unavailable"
        | "tool_authorization_unavailable"
        | "tool_run_not_found"
        | "coordinator_not_executable"
        | "tool_policy_or_role_denied"
        | "tool_capability_or_fencing_denied"
        | "tool_identity_binding_denied" => reason.as_str(),
        "http_journal_context_invalid"
        | "http_journal_identity_invalid"
        | "http_journal_invocation_not_running"
        | "http_journal_version_invalid"
        | "http_journal_baseline_invalid"
        | "http_journal_claim_invalid"
        | "http_journal_lineage_missing"
        | "http_journal_counter_overflow"
        | "http_journal_run_required"
        | "http_journal_checkpoint_exceeds_ledger"
        | "request_accounting_overflow"
        | "budget_clock_origin_conflict"
        | "budget_clock_origin_invalid"
        | "budget_clock_moved_backwards"
        | "budget_clock_unsettled_sample"
        | "budget_root_original_owner_conflict"
        | "budget_target_claim_invalid"
        | "budget_target_http_binding_conflict"
        | "budget_target_receipt_invalid"
        | "budget_target_original_call_required"
        | "budget_frozen_limit_changed"
        | "budget_dimension_contract_incomplete" => terminal_code::EVIDENCE_INTEGRITY,
        _ => "evidence_write_failed",
    };
    serde_json::json!({"error":reason,"code":code})
}

fn agent_http_journal_usage(
    connection: &rusqlite::Connection,
    scan: &str,
    target: &str,
    attempts: &[i64],
) -> Result<Option<(i64, i64)>, String> {
    let anchor = *attempts.last().ok_or("http_journal_lineage_missing")?;
    let baseline: Option<(i64, i64)> = connection.query_row(
        "SELECT baseline_requests,schema_version FROM agent_http_budget_origins WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=?3",
        params![scan,target,anchor], |r|Ok((r.get(0)?,r.get(1)?)),
    ).optional().map_err(|_| "http_journal_origin_unavailable")?;
    let Some((baseline, 1)) = baseline else {
        return if baseline.is_none() {
            Ok(None)
        } else {
            Err("http_journal_version_invalid".into())
        };
    };
    if baseline < 0 {
        return Err("http_journal_baseline_invalid".into());
    }
    let mut statement = connection.prepare(
        "SELECT c.ordinal,c.attempt_number,c.response_status,c.received_at,c.request_hash,
         EXISTS(SELECT 1 FROM agent_runs r JOIN tool_invocations i ON i.run_id=r.id
           WHERE r.id=c.run_id AND r.scan_id=c.scan_id AND r.target_url=c.target_url
             AND r.attempt_number=c.attempt_number AND i.invocation_id=c.invocation_id
             AND i.tool_name=c.tool_name AND i.policy_decision='allow')
         FROM agent_http_request_claims c WHERE c.scan_id=?1 AND c.target_url=?2 AND c.budget_attempt=?3 ORDER BY c.ordinal"
    ).map_err(|_| "http_journal_claims_unavailable")?;
    let rows = statement
        .query_map(params![scan, target, anchor], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })
        .map_err(|_| "http_journal_claims_unavailable")?;
    let mut total = baseline;
    let mut unresolved = 0;
    for row in rows {
        let (ordinal, attempt, status, received, hash, bound) =
            row.map_err(|_| "http_journal_claim_invalid")?;
        total = total.checked_add(1).ok_or("request_accounting_overflow")?;
        if ordinal != total
            || !attempts.contains(&attempt)
            || !bound
            || hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || (status == 0) != received.is_empty()
            || (status != 0 && !(100..=599).contains(&status))
        {
            return Err("http_journal_claim_invalid".into());
        }
        if status == 0 {
            unresolved += 1;
        }
    }
    Ok(Some((total, unresolved)))
}

#[cfg(test)]
fn claim_agent_http_request(
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    request: &AgentHttpRequest,
    url: &str,
) -> Result<Option<AgentHttpClaim>, String> {
    if context.run.is_none() {
        return Ok(None);
    }
    if !agent_http_claim_identity_valid(context, request) {
        return Err("http_journal_identity_invalid".into());
    }
    let headers = agent_http_wire_headers(context, request)?;
    claim_agent_http_request_with_headers(context, runtime, request, url, &headers)
}

#[cfg(test)]
fn claim_agent_http_request_with_headers(
    context: &AgentRunContext,
    runtime: &AgentToolRuntime,
    request: &AgentHttpRequest,
    url: &str,
    headers: &[(String, String)],
) -> Result<Option<AgentHttpClaim>, String> {
    let Some(run) = &context.run else {
        // Unregistered contexts only exist in isolated unit tests. Production
        // never silently falls back to volatile accounting.
        #[cfg(test)]
        {
            return Ok(None);
        }
        #[cfg(not(test))]
        {
            return Err("http_journal_run_required".into());
        }
    };
    if run.db_path != context.db_path || runtime.current_invocation.is_empty() {
        return Err("http_journal_context_invalid".into());
    }
    if !agent_http_claim_identity_valid(context, request) {
        return Err("http_journal_identity_invalid".into());
    }
    let connection = db::open(&context.db_path)?;
    let tx =
        rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "http_journal_lock_failed")?;
    let single = native_single_policy(&tx, context)?;
    let work = || claim_agent_http_request_on(&tx, context, runtime, request, url, headers);
    let claim = if single {
        single_http_transaction(&tx, false, work)?
    } else {
        work()?
    };
    tx.commit()
        .map_err(|_| "http_journal_claim_commit_failed")?;
    Ok(Some(claim))
}

#[cfg(test)]
fn receive_agent_http_headers(
    context: &AgentRunContext,
    claim: &AgentHttpClaim,
    status: u16,
) -> Result<(), String> {
    if context
        .run
        .as_ref()
        .is_none_or(|run| run.run_id != claim.run_id || run.db_path != context.db_path)
    {
        return Err("http_journal_receipt_context_invalid".into());
    }
    let connection = db::open(&context.db_path)?;
    let tx =
        rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| "http_journal_receipt_lock_failed")?;
    let work = || receive_agent_http_headers_on(&tx, context, claim, status);
    if claim.root_target.is_some() {
        single_http_transaction(&tx, true, work)?;
    } else {
        work()?;
    }
    tx.commit()
        .map_err(|_| "http_journal_receipt_commit_failed".to_string())
}

fn agent_http_header_receipt(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    claim: &AgentHttpClaim,
    status: u16,
) -> Result<Vec<rusqlite::types::Value>, String> {
    let mut statement = db
        .prepare(
            "SELECT c.* FROM agent_http_request_claims c
        JOIN tool_invocations i ON i.run_id=c.run_id AND i.invocation_id=c.invocation_id
          AND i.tool_name=c.tool_name AND i.policy_decision='allow'
        WHERE c.scan_id=?1 AND c.target_url=?2 AND c.budget_attempt=?3 AND c.ordinal=?4
          AND c.run_id=?5 AND c.invocation_id=?6 AND c.request_index=?7 AND c.request_hash=?8
          AND c.response_status=?9 AND c.received_at<>'' AND c.attempt_number=?10",
        )
        .map_err(|_| "http_journal_receipt_verification_failed")?;
    let count = statement.column_count();
    statement
        .query_row(
            params![
                context.scan_id,
                context.target_url,
                claim.budget_attempt,
                claim.ordinal,
                claim.run_id,
                claim.invocation_id,
                claim.request_index,
                claim.request_hash,
                status,
                context.attempt_number
            ],
            |r| (0..count).map(|i| r.get(i)).collect(),
        )
        .map_err(|_| "http_journal_receipt_verification_failed".into())
}
