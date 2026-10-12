// Historical first-response proof only. No current execution, closure, model
// bill settlement, repair or paid supervision frame is granted by this reader.
struct NativeOriginalProtectionResponse {
    bounded: JsonValue,
    rows: Vec<NativeCoordinatorFrozenRows>,
}
impl NativeOriginalProtectionResponse {
    fn bounded_fact(&self) -> JsonValue {
        self.bounded.clone()
    }
    fn verification_rows(&self) -> &[NativeCoordinatorFrozenRows] {
        &self.rows
    }
}

fn native_original_protection_response_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    receipt_id: &str,
) -> Result<NativeOriginalProtectionResponse, String> {
    if db.is_autocommit() {
        return Err("protection_original_snapshot_required".into());
    }
    let shadow: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name IN
        ('agent_runs','agent_assignments','agent_assignment_attempts','agent_capability_leases',
         'agent_budget_entries','agent_http_request_claims','agent_http_budget_origins','tool_invocations',
         'agent_protection_stops','agent_evidence_locations','agent_root_mode_definitions','agent_root_mode_birth_seals',
         'sentinel_scan_attempts','native_web_mode_receipts','native_web_dispatch_bindings','agent_financial_startup_bindings'))",
        [],|r|r.get(0)).map_err(|_|"protection_original_schema_unavailable")?;
    if shadow {
        return Err("protection_original_schema_shadowed".into());
    }
    let proof = private_historical_web_mode_in(
        db,
        &actor.scan_id,
        actor.attempt_number,
        &actor.root_run_id,
        None,
    )?;
    let issuer = crate::agent_runtime::web_mode::protection_issuer::ProtectionIssuer::from_verified_birth_on(
        db,&actor.root_run_id,proof.clone())?;
    // Authenticate before any receipt-derived SQL key or artifact path is used.
    let raw = issuer.verify_issued(db, receipt_id)?;
    let fact: JsonValue =
        serde_json::from_str(&raw).map_err(|_| "protection_original_fact_invalid")?;
    native_protection_exact_keys(
        &fact,
        &[
            "schemaVersion",
            "storageId",
            "receiptId",
            "createdAt",
            "rootRunId",
            "scanId",
            "attemptNumber",
            "targetUrl",
            "runId",
            "claimRowid",
            "workerAttemptId",
            "assignmentId",
            "artifactId",
            "status",
            "protectionCode",
            "coordinatorEpoch",
            "coordinatorFence",
            "classifierVersion",
            "contentType",
            "truncated",
            "preSendAuthorization",
            "originalFacts",
            "firstWriter",
        ],
    )?;
    if fact["rootRunId"] != actor.root_run_id
        || fact["scanId"] != actor.scan_id
        || fact["attemptNumber"] != actor.attempt_number
        || fact["targetUrl"] != actor.target_key
        || fact["coordinatorEpoch"] != actor.lease_epoch
        || fact["coordinatorFence"] != actor.fencing_token
    {
        return Err("protection_original_actor_scope_changed".into());
    }
    crate::agent_runtime::target_protection::verify_schema_on(db)?;
    let before = NativeCoordinatorFrozenRows::capture(
        db,
        "SELECT rowid,* FROM main.agent_protection_response_receipts WHERE receipt_id=?1",
        &[receipt_id],
    )?;
    let (mut rows, claim) = native_protection_original_rows(db, actor, &fact)?;
    let directory = resolve_agent_evidence_location(db, &actor.root_run_id)?;
    let artifact = native_protection_original_artifact(db, &directory, &fact, &claim)?;
    rows.push(NativeCoordinatorFrozenRows::capture(
        db,
        "SELECT rowid,* FROM agent_evidence_locations WHERE root_run_id=?1",
        &[&actor.root_run_id],
    )?);
    for row in &rows {
        row.verify(db)?;
    }
    before.verify(db)?;
    if issuer.verify_issued(db, receipt_id)? != raw {
        return Err("protection_original_receipt_changed".into());
    }
    if native_protection_original_artifact(db, &directory, &fact, &claim)? != artifact
        || resolve_agent_evidence_location(db, &actor.root_run_id)? != directory
        || private_historical_web_mode_in(
            db,
            &actor.scan_id,
            actor.attempt_number,
            &actor.root_run_id,
            None,
        )? != proof
    {
        return Err("protection_original_files_changed".into());
    }
    rows.push(before);
    Ok(NativeOriginalProtectionResponse {
        bounded: json!({"schemaVersion":1,"source":"original_signed_protection_response_receipt",
            "receiptId":receipt_id,"rootRunId":actor.root_run_id,"scanId":actor.scan_id,
            "attemptNumber":actor.attempt_number,"targetKey":actor.target_key,
            "authorRunId":fact["runId"],"assignmentId":fact["assignmentId"],
            "workerAttemptId":fact["workerAttemptId"],"claimRowid":fact["claimRowid"],
            "status":fact["status"],"protectionCode":fact["protectionCode"],
            "truncated":fact["truncated"],"artifact":artifact,
            "originalFactHash":crate::agent_runtime::store::stable_hash(&raw),
            "executionGrant":false,"targetRequestsGranted":0,"closedAuthorVerified":false}),
        rows,
    })
}

fn native_protection_exact_keys(value: &JsonValue, keys: &[&str]) -> Result<(), String> {
    if value
        .as_object()
        .is_none_or(|o| o.len() != keys.len() || keys.iter().any(|k| !o.contains_key(*k)))
    {
        return Err("protection_original_fact_shape_invalid".into());
    }
    Ok(())
}
fn native_protection_text<'a>(value: &'a JsonValue, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "protection_original_fact_invalid".into())
}
fn native_protection_positive(value: &JsonValue, key: &str) -> Result<i64, String> {
    value
        .get(key)
        .and_then(JsonValue::as_i64)
        .filter(|n| *n > 0)
        .ok_or_else(|| "protection_original_fact_invalid".into())
}
