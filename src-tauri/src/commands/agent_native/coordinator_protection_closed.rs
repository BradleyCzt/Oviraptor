// Historical Web closure is separate from its signed first-response source.
// The original inode guards remain owned by this proof, without recreating an
// invocation, renewing C, settling costs or granting another target/model call.
struct NativeClosedProtectionResponse {
    bounded: JsonValue,
    rows: Vec<NativeCoordinatorFrozenRows>,
    _original_sdk_exit: crate::agent_runtime::execution_owner::NativeInvocationOwner,
    _original_execution_exit: crate::agent_runtime::execution_owner::NativeInvocationOwner,
}

impl NativeClosedProtectionResponse {
    #[cfg_attr(not(test), allow(dead_code))]
    fn bounded_fact(&self) -> JsonValue {
        self.bounded.clone()
    }
    fn verification_rows(&self) -> &[NativeCoordinatorFrozenRows] {
        &self.rows
    }
}

// Pure historical audit for a paid frame that already holds the two original
// inode guards. It never probes the same locks a second time.
struct NativeClosedProtectionAudit {
    bounded: JsonValue,
    rows: Vec<NativeCoordinatorFrozenRows>,
}

#[cfg_attr(not(test), allow(dead_code))]
fn native_closed_protection_response_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    receipt_id: &str,
) -> Result<NativeClosedProtectionResponse, String> {
    native_protection_closed_context_on(db, context, actor)?;
    native_closed_protection_response_at_on(db, &context.db_path, actor, receipt_id)
}

fn native_closed_protection_response_at_on(
    db: &rusqlite::Connection,
    db_path: &Path,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    receipt_id: &str,
) -> Result<NativeClosedProtectionResponse, String> {
    use crate::agent_runtime::{execution_owner, multi_agent::budget};
    if db.is_autocommit() {
        return Err("protection_closed_snapshot_required".into());
    }
    native_protection_closed_database_on(db, db_path)?;
    let source = native_original_protection_response_on(db, actor, receipt_id)?;
    let fact = source.bounded_fact();
    let author = native_protection_text(&fact, "authorRunId")?;
    // These probes only open the original inode. Absence and a running owner
    // both reject; neither is a reason to create/retry an execution.
    let sdk_exit = execution_owner::probe_native_invocation(
        db_path,
        &actor.scan_id,
        actor.attempt_number,
        budget::web_lifetime::WEB_MODEL_INVOCATION_KIND,
        author,
    )
    .map_err(|e| format!("protection_closed_sdk_not_idle:{e}"))?
    .ok_or("protection_closed_original_sdk_exit_missing")?;
    let execution_exit = execution_owner::probe_native_invocation(
        db_path,
        &actor.scan_id,
        actor.attempt_number,
        "web-executor-original-execution",
        author,
    )
    .map_err(|e| format!("protection_closed_execution_not_idle:{e}"))?
    .ok_or("protection_closed_original_execution_exit_missing")?;
    let audited = native_audit_closed_protection_response_at_on(db, db_path, actor, receipt_id)?;
    let mut original = audited.bounded.clone();
    original["closedAuthorVerified"] = false.into();
    original
        .as_object_mut()
        .ok_or("protection_closed_original_source_changed")?
        .remove("closedAuthor");
    if original != fact {
        return Err("protection_closed_original_source_changed".into());
    }
    for row in source.verification_rows() {
        row.verify(db)?;
    }
    Ok(NativeClosedProtectionResponse {
        bounded: audited.bounded,
        rows: audited.rows,
        _original_sdk_exit: sdk_exit,
        _original_execution_exit: execution_exit,
    })
}

fn native_audit_closed_protection_response_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    receipt_id: &str,
) -> Result<NativeClosedProtectionAudit, String> {
    native_protection_closed_context_on(db, context, actor)?;
    native_audit_closed_protection_response_at_on(db, &context.db_path, actor, receipt_id)
}

fn native_protection_closed_context_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    if context.scan_id != actor.scan_id
        || context.attempt_number != actor.attempt_number
        || context.target_url != actor.target_key
        || resolve_agent_evidence_location(db, &actor.root_run_id)? != context.target_dir
    {
        return Err("protection_closed_context_conflict".into());
    }
    Ok(())
}

fn native_protection_closed_database_on(
    db: &rusqlite::Connection,
    db_path: &Path,
) -> Result<(), String> {
    if std::fs::canonicalize(db.path().ok_or("protection_closed_database_missing")?)
        .map_err(|_| "protection_closed_database_unavailable")?
        != std::fs::canonicalize(db_path).map_err(|_| "protection_closed_database_unavailable")?
    {
        return Err("protection_closed_context_conflict".into());
    }
    Ok(())
}

fn native_audit_closed_protection_response_at_on(
    db: &rusqlite::Connection,
    db_path: &Path,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    receipt_id: &str,
) -> Result<NativeClosedProtectionAudit, String> {
    use crate::agent_runtime::multi_agent::budget;
    if db.is_autocommit() {
        return Err("protection_closed_snapshot_required".into());
    }
    let shadow: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name IN
         ('agent_messages','agent_web_model_journal','agent_lane_leases','agent_collaboration_events'))",
        [], |r|r.get(0),
    ).map_err(|_|"protection_closed_schema_unavailable")?;
    if shadow {
        return Err("protection_closed_schema_shadowed".into());
    }
    let source = native_original_protection_response_on(db, actor, receipt_id)?;
    let mut bounded = source.bounded_fact();
    let author = native_protection_text(&bounded, "authorRunId")?.to_string();
    let assignment = native_protection_text(&bounded, "assignmentId")?.to_string();
    let worker = native_protection_text(&bounded, "workerAttemptId")?.to_string();
    native_protection_closed_database_on(db, db_path)?;
    let unsettled: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1 AND status='running')
          OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1
             AND (response_status=0 OR received_at=''))",
            [&author],
            |r| r.get(0),
        )
        .map_err(|_| "protection_closed_pending_work_unavailable")?;
    if unsettled {
        return Err("protection_closed_original_work_pending".into());
    }
    let journal_conflict: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_web_model_journal j WHERE j.child_run_id=?1
          AND (j.root_run_id<>?2 OR j.assignment_id<>?3 OR j.lease_epoch<>?4 OR j.fencing_token<>?5
            OR (j.phase<>'dispatch' AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal d
              WHERE d.phase='dispatch' AND d.call_id=j.call_id AND d.root_run_id=j.root_run_id
                AND d.assignment_id=j.assignment_id AND d.child_run_id=j.child_run_id
                AND d.lease_epoch=j.lease_epoch AND d.fencing_token=j.fencing_token
                AND d.round=j.round AND d.request_hash=j.request_hash))))",
            params![
                author,
                actor.root_run_id,
                assignment,
                actor.lease_epoch,
                actor.fencing_token
            ],
            |r| r.get(0),
        )
        .map_err(|_| "protection_closed_model_journal_unavailable")?;
    if journal_conflict {
        return Err("protection_closed_original_model_scope_conflict".into());
    }
    let usage = budget::web_lifetime::child_usage_original(db, actor, &assignment, &author)?;
    let invoice = [
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.model_requests,
    ];
    if invoice.iter().any(|v| *v < 0)
        || usage.cached_input_tokens > usage.input_tokens
        || usage.input_tokens.checked_add(usage.output_tokens) != Some(usage.total_tokens)
        || usage.model_requests <= 0
    {
        return Err("protection_closed_original_usage_invalid".into());
    }
    for (index, dimension) in budget::DIMENSIONS.iter().enumerate() {
        let balance =
            budget::balance_for_attempt(db, &actor.root_run_id, &assignment, &worker, dimension)?;
        if balance.reserved != 0
            || balance.indeterminate != 0
            || index < 4 && balance.consumed != invoice[index]
            || index == 4 && balance.consumed != 1
            || (5..8).contains(&index) && balance.consumed != 0
        {
            return Err("protection_closed_original_budget_unresolved".into());
        }
        // Concurrency and elapsed wall cost are real ledger facts, not assumed
        // zero just because the source has closed.
    }
    let used: (i64, i64, i64) = db
        .query_row(
            "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs
         WHERE id=?1 AND assignment_id=?2 AND root_run_id=?3 AND cancel_requested_at=''",
            params![author, assignment, actor.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "protection_closed_child_usage_unavailable")?;
    if used
        != (
            usage.total_tokens,
            usage.cached_input_tokens,
            usage.model_requests,
        )
    {
        return Err("protection_closed_child_usage_conflict".into());
    }
    // The stop row is already signed and physically reverified by the source.
    // Its original reason is the exact summary used by the closing writer.
    let summary: String = db
        .query_row(
            "SELECT reason FROM agent_protection_stops WHERE scan_id=?1 AND target_url=?2
         AND source_attempt_number=?3 AND source_run_id=?4
         AND stop_code='confirmed_waf_or_challenge'
         AND json_extract(claim_json,'$.rowid')=?5",
            params![
                actor.scan_id,
                actor.target_key,
                actor.attempt_number,
                author,
                bounded["claimRowid"].as_i64()
            ],
            |r| r.get(0),
        )
        .map_err(|_| "protection_closed_original_summary_missing")?;
    let summary = crate::agent_runtime::secrets::redact_text_with(&summary, None);
    let expected = crate::agent_runtime::secrets::redact_json(&json!({
        "terminalCode":AGENT_STOP_WAF,"summary":summary,
        "usage":{"totalTokens":usage.total_tokens,"modelRequests":usage.model_requests}
    }));
    let mut q = db
        .prepare(
            "SELECT id,payload_json FROM agent_messages WHERE assignment_id=?1
         AND kind='execution_result' ORDER BY rowid",
        )
        .map_err(|_| "protection_closed_original_ack_unavailable")?;
    let messages = q
        .query_map([&assignment], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|_| "protection_closed_original_ack_unavailable")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| "protection_closed_original_ack_unavailable")?;
    let [(message, raw)] = messages.as_slice() else {
        return Err("protection_closed_original_ack_conflict".into());
    };
    let expected_raw = expected.to_string();
    if bounded["protectionCode"] != AGENT_STOP_WAF || *raw != expected_raw {
        return Err("protection_closed_original_payload_conflict".into());
    }
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment.clone(),
        run_id: author.clone(),
        role: crate::agent_runtime::contract::AgentRole::WebExecutor,
    };
    crate::collaboration_events::client_delivery_schema::verify(db)?;
    if !original_executor_finished_historical_on(db, actor, &child, &expected, &summary)? {
        return Err("protection_closed_original_author_not_closed".into());
    }
    let mut rows = source.verification_rows().to_vec();
    let attempt = actor.attempt_number.to_string();
    for (sql, keys) in [
        ("SELECT rowid,* FROM agent_runs WHERE id=?1",vec![author.as_str()]),
        ("SELECT rowid,* FROM agent_assignments WHERE id=?1",vec![assignment.as_str()]),
        ("SELECT rowid,* FROM agent_assignment_attempts WHERE child_run_id=?1 ORDER BY rowid",vec![author.as_str()]),
        ("SELECT rowid,* FROM agent_web_model_journal WHERE child_run_id=?1 ORDER BY rowid",vec![author.as_str()]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 ORDER BY rowid",vec![actor.root_run_id.as_str(),assignment.as_str(),worker.as_str()]),
        ("SELECT rowid,* FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result' ORDER BY rowid",vec![assignment.as_str()]),
        ("SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2 ORDER BY rowid",vec![assignment.as_str(),author.as_str()]),
        ("SELECT rowid,* FROM tool_invocations WHERE run_id=?1 ORDER BY rowid",vec![author.as_str()]),
        ("SELECT rowid,* FROM agent_http_request_claims WHERE run_id=?1 ORDER BY rowid",vec![author.as_str()]),
        ("SELECT rowid,* FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=CAST(?2 AS INTEGER) AND entity_id IN (?3,?4,?5) ORDER BY sequence",vec![actor.scan_id.as_str(),attempt.as_str(),assignment.as_str(),author.as_str(),message.as_str()]),
    ] {
        rows.push(NativeCoordinatorFrozenRows::capture(db, sql, &keys)?);
    }
    rows.push(NativeCoordinatorFrozenRows::capture_optional(
        db,
        "SELECT rowid,* FROM agent_lane_leases WHERE assignment_id=?1",
        &[&assignment],
    )?);
    let again = native_original_protection_response_on(db, actor, receipt_id)?;
    if again.bounded_fact() != source.bounded_fact()
        || budget::web_lifetime::child_usage_original(db, actor, &assignment, &author)? != usage
        || !original_executor_finished_historical_on(db, actor, &child, &expected, &summary)?
    {
        return Err("protection_closed_original_source_changed".into());
    }
    bounded["closedAuthorVerified"] = true.into();
    bounded["closedAuthor"] = json!({"runId":author,"assignmentId":assignment,
        "workerAttemptId":worker,"messageId":message,"originalInvoice":usage.as_json(),
        "savedOutput":expected,"executionGrant":false,"targetRequestsGranted":0});
    for row in &rows {
        row.verify(db)?;
    }
    Ok(NativeClosedProtectionAudit { bounded, rows })
}

// A known signed protected response must retain its original closed author
// after delivery and on completion replay. Missing issuance remains a durable
// stopped result: this hook never constructs a replacement seal from files.
fn native_verify_delivered_protection_source(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    outcome: &AgentTargetOutcome,
) -> Result<(), String> {
    if !matches!(outcome, AgentTargetOutcome::Limited(stop) if stop.code == AGENT_STOP_WAF) {
        return Ok(());
    }
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut q = tx.prepare("SELECT receipt_id FROM main.agent_protection_response_receipts
        WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 ORDER BY storage_id")
        .map_err(|e| e.to_string())?;
    let ids = q
        .query_map(
            params![
                actor.root_run_id,
                actor.scan_id,
                actor.attempt_number,
                actor.target_key
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    match ids.as_slice() {
        [] => Ok(()),
        [id] => {
            let closed = native_closed_protection_response_on(&tx, context, actor, id)?;
            let fact = closed.bounded_fact();
            if fact["closedAuthorVerified"] != true
                || fact["executionGrant"] != false
                || fact["targetRequestsGranted"] != 0
            {
                return Err("protection_closed_postcondition_invalid".into());
            }
            for row in closed.verification_rows() {
                row.verify(&tx)?;
            }
            Ok(())
        }
        _ => Err("protection_closed_original_receipt_conflict".into()),
    }
}
