// Commands map the shared pure denial to their existing terminal presentation.
fn agent_protection_stop_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<Option<AgentStop>, String> {
    crate::agent_runtime::target_protection::read_on(db, &context.scan_id, &context.target_url)
        .map(|stop| stop.map(|stop| AgentStop::new(stop.code, stop.reason)))
}

fn protection_stop_original_claim_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    claim: &AgentHttpClaim,
    status: u16,
) -> Result<String, String> {
    if context
        .run
        .as_ref()
        .is_none_or(|run| run.run_id != claim.run_id || run.db_path != context.db_path)
        || !(100..=599).contains(&status)
    {
        return Err("protection_stop_claim_scope_invalid".into());
    }
    db.query_row(
        "SELECT json_object('rowid',c.rowid,'scanId',c.scan_id,'targetUrl',c.target_url,
          'budgetAttempt',c.budget_attempt,'ordinal',c.ordinal,'attemptNumber',c.attempt_number,
          'originRowid',o.rowid,'originBaselineRequests',o.baseline_requests,'originSchema',o.schema_version,
          'runId',c.run_id,'invocationId',c.invocation_id,'requestIndex',c.request_index,
          'toolName',c.tool_name,'identityHandle',c.identity_handle,'requestHash',c.request_hash,
          'responseStatus',c.response_status,'createdAt',c.created_at,'receivedAt',c.received_at)
         FROM main.agent_http_request_claims c
         JOIN main.agent_http_budget_origins o ON o.scan_id=c.scan_id
          AND o.target_url=c.target_url AND o.budget_attempt=c.budget_attempt
         JOIN main.tool_invocations i ON i.run_id=c.run_id AND i.invocation_id=c.invocation_id
          AND i.tool_name=c.tool_name AND i.policy_decision='allow'
         WHERE c.scan_id=?1 AND c.target_url=?2 AND c.attempt_number=?3 AND c.run_id=?4
          AND c.invocation_id=?5 AND c.request_index=?6 AND c.request_hash=?7
          AND c.budget_attempt=?8 AND c.ordinal=?9 AND c.response_status=?10
          AND c.received_at<>''",
        params![
            context.scan_id,
            context.target_url,
            context.attempt_number,
            claim.run_id,
            claim.invocation_id,
            claim.request_index,
            claim.request_hash,
            claim.budget_attempt,
            claim.ordinal,
            status
        ],
        |r| r.get(0),
    )
    .map_err(|_| "protection_stop_original_headers_missing".into())
}

fn agent_persist_protection_stop(
    context: &AgentRunContext,
    claim: &AgentHttpClaim,
    status: u16,
    stop: &AgentStop,
) -> Result<(), String> {
    use rusqlite::{
        hooks::{AuthAction, AuthContext, Authorization},
        OptionalExtension,
    };
    crate::agent_runtime::target_protection::validate_code(stop.code)?;
    if stop.reason.is_empty()
        || stop.reason.chars().count() > 2000
        || (stop.code == AGENT_STOP_RATE_LIMIT && status != 429)
    {
        return Err("protection_stop_invalid".into());
    }
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "protection_stop_lock_failed")?;
    crate::agent_runtime::target_protection::verify_schema_on(&tx)?;
    let original = protection_stop_original_claim_on(&tx, context, claim, status)?;
    let hash = crate::agent_runtime::store::stable_hash(&original);
    let prior: Option<(String,String,String,String,i64,String)> = tx.query_row(
        "SELECT stop_code,reason,claim_json,claim_hash,source_attempt_number,source_run_id FROM main.agent_protection_stops
         WHERE scan_id=?1 AND target_url=?2", params![context.scan_id,context.target_url],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).optional().map_err(|_| "protection_stop_read_failed")?;
    if let Some(prior) = prior {
        if prior
            != (
                stop.code.to_string(),
                stop.reason.clone(),
                original,
                hash,
                context.attempt_number,
                claim.run_id.clone(),
            )
        {
            return Err("protection_stop_original_conflict".into());
        }
        // Exact original retries are read-only; no current-time repair or refresh.
        return Ok(());
    }
    db.authorizer(Some(|a: AuthContext<'_>| match a.action {
        AuthAction::Insert { table_name }
            if a.database_name == Some("main")
                && table_name == "agent_protection_stops"
                && a.accessor.is_none() =>
        {
            Authorization::Allow
        }
        AuthAction::Insert { .. } | AuthAction::Update { .. } | AuthAction::Delete { .. } => {
            Authorization::Deny
        }
        _ => Authorization::Allow,
    }))
    .map_err(|_| "protection_stop_write_guard_unavailable")?;
    let guard = ProtectionStopWriteGuard(&db);
    let n = tx
        .execute(
            "INSERT INTO main.agent_protection_stops(scan_id,target_url,source_attempt_number,
          source_run_id,stop_code,reason,claim_json,claim_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                context.scan_id,
                context.target_url,
                context.attempt_number,
                claim.run_id,
                stop.code,
                stop.reason,
                original,
                hash
            ],
        )
        .map_err(|_| "protection_stop_write_failed")?;
    if n != 1 || protection_stop_original_claim_on(&tx, context, claim, status)? != original {
        return Err("protection_stop_original_headers_changed".into());
    }
    let after: (String, String, String, String, i64, String) = tx
        .query_row(
            "SELECT stop_code,reason,claim_json,claim_hash,source_attempt_number,source_run_id
         FROM main.agent_protection_stops WHERE scan_id=?1 AND target_url=?2",
            params![context.scan_id, context.target_url],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .map_err(|_| "protection_stop_postcondition_failed")?;
    if after
        != (
            stop.code.to_string(),
            stop.reason.clone(),
            original,
            hash,
            context.attempt_number,
            claim.run_id.clone(),
        )
    {
        return Err("protection_stop_postcondition_failed".into());
    }
    drop(guard);
    tx.commit()
        .map_err(|_| "protection_stop_commit_failed".into())
}

struct ProtectionStopWriteGuard<'a>(&'a rusqlite::Connection);
impl Drop for ProtectionStopWriteGuard<'_> {
    fn drop(&mut self) {
        use rusqlite::hooks::{AuthContext, Authorization};
        let _ = self
            .0
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}
