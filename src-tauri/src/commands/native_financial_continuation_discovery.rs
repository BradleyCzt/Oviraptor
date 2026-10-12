// Recover an already issued grant's immutable history, never a new proposal or
// permission. Both closed and pending attempts retain their original signatures.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinancialContinuationDiscoveredIssue {
    original_receipt: JsonValue,
    proposal: JsonValue,
    status: FinancialContinuationStatus,
    historical_proposal: bool,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinancialContinuationDiscovery {
    schema: u8,
    scan_id: String,
    attempt_number: i64,
    issued: Option<FinancialContinuationDiscoveredIssue>,
}

// Historical signature verification deliberately uses the signed task's policy,
// not today's policy/session/C. Current authority is rechecked by status before
// it can report workerStartAllowed; expired history remains readable.
fn financial_discovery_mode_in(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    root: &str,
    birth: Option<&JsonValue>,
) -> Result<ModeFact, String> {
    private_historical_web_mode_in(db, scan, attempt, root, birth).map(|p| p.fact().clone())
}

fn private_historical_web_mode_in(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    root: &str,
    birth: Option<&JsonValue>,
) -> Result<VerifiedMode, String> {
    private_historical_web_mode_for_mode_in(db, scan, attempt, root, birth, WebMode::Multi)
}

fn private_historical_web_mode_for_mode_in(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    root: &str,
    birth: Option<&JsonValue>,
    expected_mode: WebMode,
) -> Result<VerifiedMode, String> {
    if birth.is_some() && expected_mode != WebMode::Multi {
        return Err("financial_continuation_discovery_mode_changed".into());
    }
    let (fact, signature) = web_mode::stored_fact(db, scan, attempt)?;
    let work: String = db
        .query_row(
            "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
            params![scan, attempt],
            |r| r.get(0),
        )
        .map_err(|_| "financial_continuation_discovery_mode_missing")?;
    if fact.work_dir != work {
        return Err("financial_continuation_discovery_mode_changed".into());
    }
    let directory = WebBindingDirectory::open(Path::new(&work))?;
    let marker = directory
        .read(".oviraptor-scan-id", 1024, false, false)?
        .ok_or("web_mode_marker_missing")?;
    let key = directory
        .read(WEB_DISPATCH_KEY_FILE, 32, false, false)?
        .ok_or("web_mode_key_missing")?;
    let task = directory
        .read("task.json", WEB_DISPATCH_BINDING_LIMIT, false, false)?
        .ok_or("web_mode_task_missing")?;
    let document: JsonValue = serde_json::from_slice(&task).map_err(|_| "web_mode_task_invalid")?;
    let original = web_mode::root::read(db, root)?.ok_or("financial_continuation_mode_missing")?;
    let policy = &document["effectiveWebPolicy"];
    if marker != scan.as_bytes()
        || document["scanId"] != scan
        || fact.task_hash != format!("{:x}", Sha256::digest(&task))
        || fact.targets != mode_task_targets(&document)?
        || fact.policy_hash != crate::agent_runtime::store::stable_hash(&policy.to_string())
        || WebMode::from_policy(policy)? != fact.mode
        || original.fact() != &fact
        || original.mode() != expected_mode
    {
        return Err("financial_continuation_discovery_mode_changed".into());
    }
    let verified = VerifiedMode::verify(fact.clone(), &key, &signature)?;
    if let Some(birth) = birth {
        let ordinary: i64 = db.query_row(
            "SELECT count(*) FROM native_web_dispatch_bindings WHERE scan_id=?1 AND attempt_number=?2 AND branch='web'",
            params![scan, attempt], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        let (approval, tag, raw): (String, Vec<u8>, String) = db.query_row(
            "SELECT approval_id,binding_tag,binding_fact_json FROM agent_financial_startup_bindings WHERE scan_id=?1 AND attempt_number=?2",
            params![scan, attempt], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).map_err(|_| "financial_continuation_discovery_startup_missing")?;
        let binding: JsonValue = serde_json::from_str(&raw)
            .map_err(|_| "financial_continuation_discovery_startup_changed")?;
        let canonical = binding.to_string();
        let expected = json!({"domain":"oviraptor.financial-continuation.v1",
            "approvalId":birth["approvalId"],"proposalHash":birth["proposalHash"],
            "frontendEvidenceHash":birth["frontendEvidenceHash"],
            "newRootRunId":birth["newRootRunId"],"newPlanHash":birth["newPlanHash"]});
        let tags = tag.iter().map(|b| format!("{b:02x}")).collect::<String>();
        if ordinary != 0
            || approval != birth["approvalId"]
            || raw != canonical
            || binding != expected
            || tag.len() != 32
            || fact.startup_tag != tags
            || crate::agent_runtime::store::stable_hash(&raw) != fact.runtime_hash
            || document["attemptNumber"] != attempt
            || document["taskPurpose"] != "new_work_after_original_cost_confirmation"
            || document["financialContinuation"]["approvalId"] != birth["approvalId"]
            || document["financialContinuation"]["proposalHash"] != birth["proposalHash"]
            || document["financialContinuation"]["taskDescription"] != birth["taskDescription"]
        {
            return Err("financial_continuation_discovery_startup_changed".into());
        }
        aws_lc_rs::hmac::verify(
            &aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, &key),
            raw.as_bytes(),
            &tag,
        )
        .map_err(|_| "financial_continuation_discovery_startup_signature_invalid")?;
    } else {
        let (schema, tag): (i64, Vec<u8>) = db.query_row(
            "SELECT schema_version,binding_tag FROM native_web_dispatch_bindings WHERE scan_id=?1 AND attempt_number=?2 AND branch='web'",
            params![scan, attempt], |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|_| "financial_continuation_discovery_startup_missing")?;
        let financial: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_financial_startup_bindings WHERE scan_id=?1 AND attempt_number=?2)",
            params![scan, attempt], |r|r.get(0),
        ).map_err(|e|e.to_string())?;
        let tags = tag.iter().map(|b| format!("{b:02x}")).collect::<String>();
        if schema != 1 || financial || tag.len() != 32 || fact.startup_tag != tags {
            return Err("financial_continuation_discovery_startup_changed".into());
        }
    }
    directory.check_identity()?;
    Ok(verified)
}

fn native_financial_continuation_discovery_on(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> Result<JsonValue, String> {
    if db.is_autocommit() {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return native_financial_continuation_discovery_on(&snapshot, scan, attempt);
    }
    let current: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN sentinel_scan_attempts a ON a.scan_id=s.id
        AND a.attempt_number=s.attempt_count WHERE s.id=?1 AND s.attempt_count=?2 AND ?2>0)
        AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        params![scan, attempt], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if !current {
        return Err("financial_continuation_discovery_scope_invalid".into());
    }
    // Columns alone are insufficient: a damaged scope column must not hide a
    // durable birth as `issued:null`. JSON and independent startup/Root scope
    // also discover candidates; the immutable verifier then checks all columns.
    let mut q = db.prepare("SELECT new_root_run_id FROM agent_financial_continuations
        WHERE (scan_id=?1 AND new_attempt_number=?2)
        OR CASE WHEN json_valid(fact_json) THEN json_extract(fact_json,'$.scanId')=?1
          AND json_extract(fact_json,'$.newAttemptNumber')=?2 ELSE 0 END
        OR new_root_run_id IN(SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2)
        OR approval_id IN(SELECT approval_id FROM agent_financial_startup_bindings WHERE scan_id=?1 AND attempt_number=?2)")
        .map_err(|e|e.to_string())?;
    let roots = q
        .query_map(params![scan, attempt], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if roots.len() > 1 {
        return Err("financial_continuation_discovery_ambiguous".into());
    }
    let issued = if let Some(root) = roots.first() {
        let birth = verify_financial_continuation_birth_on(db, root)?;
        if birth["scanId"] != scan || birth["newAttemptNumber"] != attempt {
            return Err("financial_continuation_discovery_binding_conflict".into());
        }
        let source = financial_status_string(&birth, "sourceRootRunId")?;
        let source_attempt = birth["sourceAttemptNumber"]
            .as_i64()
            .ok_or("financial_continuation_birth_invalid")?;
        let mode = financial_discovery_mode_in(db, scan, source_attempt, &source, None)?;
        let source_mode_rows = NativeCoordinatorFrozenRows::capture_optional(db,
            "SELECT rowid,* FROM native_web_mode_receipts WHERE scan_id=?1 AND attempt_number=CAST(?2 AS INTEGER)",
            &[scan, &source_attempt.to_string()])?;
        if birth["proposal"]["originalModeReceiptId"] != mode.receipt_id
            || birth["proposal"]["evidenceHashes"]["mode"] != source_mode_rows.hash
        {
            return Err("financial_continuation_discovery_original_mode_changed".into());
        }
        financial_discovery_mode_in(db, scan, attempt, root, Some(&birth))?;
        let call: String = db
            .query_row(
                "SELECT call_id FROM agent_root_cost_reconciliation_receipts WHERE receipt_id=?1
             AND scan_id=?2 AND attempt_number=?3 AND root_run_id=?4",
                params![
                    birth["sourceReceiptId"].as_str(),
                    scan,
                    source_attempt,
                    source
                ],
                |r| r.get(0),
            )
            .map_err(|_| "financial_continuation_discovery_receipt_missing")?;
        // The normal user-confirmation reader requires the source attempt to be
        // current. Issuance has advanced it. This historical reader reuses the
        // complete original invoice/terminal/cost/receipt verifier, never a raw
        // JSON lookup or relaxed confirmation/authorization entry point.
        let receipt = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::
            read_original_cost_reconciliation(db, &source, &call)?
            .ok_or("financial_continuation_discovery_receipt_missing")?;
        if receipt["receiptId"] != birth["sourceReceiptId"]
            || receipt["scanId"] != scan
            || receipt["attemptNumber"] != source_attempt
            || receipt["rootRunId"] != source
            || receipt["callId"] != call
            || receipt["actualCostConfirmed"] != true
            || receipt["executionAllowed"] != false
            || receipt["automaticResumeAllowed"] != false
        {
            return Err("financial_continuation_discovery_receipt_changed".into());
        }
        let proposal = birth["proposal"].clone();
        if !proposal["issuanceImplemented"].is_boolean()
            || proposal["executionAllowed"] != false
            || proposal["automaticResumeAllowed"] != false
            || proposal["originalSemanticReplayAllowed"] != false
        {
            return Err("financial_continuation_discovery_proposal_invalid".into());
        }
        let status = serde_json::from_value(financial_continuation_status_in(db, &birth)?)
            .map_err(|_| "financial_continuation_discovery_status_invalid")?;
        Some(FinancialContinuationDiscoveredIssue {
            original_receipt: receipt,
            proposal,
            status,
            historical_proposal: true,
        })
    } else {
        let marker: bool = db.query_row("SELECT
            EXISTS(SELECT 1 FROM agent_financial_startup_bindings WHERE scan_id=?1 AND attempt_number=?2)
            OR EXISTS(SELECT 1 FROM agent_financial_root_budget_definitions d JOIN agent_runs r ON r.id=d.root_run_id
                WHERE r.scan_id=?1 AND r.attempt_number=?2)",
            params![scan, attempt], |r|r.get(0)).map_err(|e|e.to_string())?;
        if marker {
            return Err("financial_continuation_discovery_birth_missing".into());
        }
        None
    };
    serde_json::to_value(FinancialContinuationDiscovery {
        schema: 1,
        scan_id: scan.into(),
        attempt_number: attempt,
        issued,
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn discover_native_root_financial_continuation(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
) -> Result<JsonValue, String> {
    let db = rusqlite::Connection::open_with_flags(
        &state.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    native_financial_continuation_discovery_on(&db, &scan_id, attempt_number)
}

fn original_withdrawal_issuer_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Option<crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer>, String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return original_withdrawal_issuer_on(&tx, actor);
    }
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_mode_birth_seals WHERE root_run_id=?1)",
            [&actor.root_run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        if web_mode::root::read(db, &actor.root_run_id)?
            .is_some_and(|d| d.lease_expiry_supervision().is_some())
        {
            return Err("withdrawal_issuer_required_birth_seal_missing".into());
        }
        return Ok(None);
    }
    let proof = private_historical_web_mode_in(
        db,
        &actor.scan_id,
        actor.attempt_number,
        &actor.root_run_id,
        None,
    )?;
    web_mode::root::verify_birth(db, &actor.root_run_id, &proof)?;
    let declaration =
        web_mode::root::read(db, &actor.root_run_id)?.ok_or("withdrawal_issuer_root_missing")?;
    if declaration.new_evidence_supervision().is_none() {
        return Ok(None);
    }
    let issuer = crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, proof)?;
    issuer.require_actor(db, actor)?;
    Ok(Some(issuer))
}

// Original private mode proof only: no approval, renewal, or financial grant.
pub(crate) fn original_root_capture_mode(db: &rusqlite::Connection, root: &str) -> Result<VerifiedMode, String> {
    if db.is_autocommit() {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return original_root_capture_mode(&snapshot, root);
    }
    let declaration = web_mode::root::read(db, root)?.ok_or("root_cache_original_mode_missing")?;
    let fact = declaration.fact();
    let financial: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_financial_continuations WHERE new_root_run_id=?1)", [root], |r|r.get(0)).map_err(|e|e.to_string())?;
    let birth = if financial { Some(verify_financial_continuation_birth_on(db, root)?) } else { None };
    let proof = private_historical_web_mode_for_mode_in(db, &fact.scan_id, fact.attempt_number, root, birth.as_ref(), declaration.mode())?;
    web_mode::root::verify_birth(db, root, &proof)?;
    Ok(proof)
}
