// A read-only proposal, never a replacement Coordinator, new Root or SDK call.
fn native_financial_continuation_model_on(
    db: &rusqlite::Connection,
    root: &str,
) -> Result<String, String> {
    use crate::agent_runtime::store::stable_hash;
    let settings = sentinel_settings(db);
    let environment = model_runtime_env(&settings)?;
    let mut candidates = vec![None];
    candidates.extend(
        approved_agent_proxies(&settings)
            .into_iter()
            .map(|(_, url)| Some(url)),
    );
    for proxy in candidates {
        let profile = agent_model_profile(&environment, proxy.as_deref())?;
        let request = json!({"model":profile.model,"endpointBinding":stable_hash(&profile.endpoint),
            "deployment":if profile.local {"local"}else {"cloud"},
            "credentialBinding":stable_hash(profile.bearer().unwrap_or("")),
            "proxyBinding":stable_hash(profile.proxy.as_deref().unwrap_or("")),
            "maxContextTokens":profile.max_context_tokens,
            "maxOutputTokens":profile.max_output_tokens.unwrap_or(1024).min(2048),
            "temperature":crate::agent_runtime::model::gateway::ModelRequest::default().temperature,
            "timeoutPolicy":"original_root_clock",
            "schemaHash":stable_hash(&crate::agent_runtime::multi_agent::budget::root::model::tick::decision::DecisionSummary::schema().to_string())});
        // This validator compares original provider/model/key/proxy/profile.
        // Do not start the local model hook merely to inspect those bindings.
        if native_coordinator_require_original_model_on(db, root, &request).is_ok() {
            return Ok(stable_hash(&request.to_string()));
        }
    }
    Err("financial_continuation_original_model_changed".into())
}

fn native_financial_continuation_preview_in(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    root: &str,
    receipt: &str,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::{budget, lease};
    use crate::agent_runtime::{store, web_mode};
    if db.is_autocommit() {
        return Err("financial_continuation_requires_snapshot".into());
    }
    let (status, target, plan_hash): (String, String, String) = db.query_row(
        "SELECT s.status,r.target_url,r.plan_hash FROM sentinel_scans s JOIN projects p ON p.id=s.project_id
         JOIN agent_runs r ON r.scan_id=s.id AND r.attempt_number=s.attempt_count
         WHERE s.id=?1 AND s.attempt_count=?2 AND s.scan_type='web' AND s.status IN ('paused','partial')
           AND p.status='active' AND r.id=?3 AND r.backend='native' AND r.role='coordinator'
           AND r.orchestration_policy='multi' AND r.root_run_id=r.id AND r.parent_run_id IS NULL AND r.assignment_id=''
           AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=s.id)",
        params![scan, attempt, root], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).map_err(|_| "financial_continuation_current_scope_invalid")?;
    let proposed = attempt
        .checked_add(1)
        .filter(|v| *v > 1)
        .ok_or("financial_continuation_attempt_invalid")?;
    let source = budget::continuation::settled_source_on(db, root, receipt)?;
    let original = private_web_mode_on(db, scan, attempt)?;
    let declaration =
        web_mode::root::read(db, root)?.ok_or("financial_continuation_mode_missing")?;
    if declaration.fact() != original.fact() || declaration.mode() != web_mode::WebMode::Multi {
        return Err("financial_continuation_original_mode_changed".into());
    }
    declaration.validate_scope(scan, attempt, &target, &plan_hash)?;
    let actor = db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
        FROM agent_coordinator_leases WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4",
        params![scan,attempt,target,root], |r| Ok(lease::CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,
            target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?}))
        .map_err(|_|"financial_continuation_original_coordinator_missing")?;
    budget::root::RootOwner::load_original(db, root)?.require_original_coordinator(db, &actor)?;
    // This checks only the original C/fence and expiry, not executable status.
    // Normal execution gates remain closed while the task is paused.
    lease::validate_coordinator_lease(db, &actor)?;
    let model_hash = native_financial_continuation_model_on(db, root)?;
    let (identity_mode, identity_ids) =
        crate::auth_session::validated_scan_identities(db, scan, &target)?;
    let mut query = db
        .prepare(
            "SELECT id FROM agent_runs WHERE scan_id=?1
        AND backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id=''
        UNION SELECT root_run_id FROM agent_root_budget_attempts
        WHERE json_extract(contract_json,'$.root.scan')=?1 ORDER BY 1",
        )
        .map_err(|e| e.to_string())?;
    let roots = query
        .query_map([scan], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut historical_proofs = Vec::new();
    for historical in &roots {
        let original_scope: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND scan_id=?2
            AND attempt_number>0 AND attempt_number<=?3 AND backend='native' AND role='coordinator'
            AND parent_run_id IS NULL AND assignment_id='' AND (root_run_id=id
            OR (root_run_id='' AND orchestration_policy='single')))",
                params![historical, scan, attempt],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !original_scope {
            return Err("financial_continuation_history_scope_missing".into());
        }
        historical_proofs.push(budget::continuation::verify_settled_history_on(
            db, historical,
        )?);
    }
    let mut evidence = serde_json::Map::new();
    for (name, sql, keys) in [
        ("scope", "SELECT s.rowid,s.*,p.rowid,p.* FROM sentinel_scans s JOIN projects p ON p.id=s.project_id WHERE s.id=?1", vec![scan]),
        ("authority", "SELECT rowid,* FROM agent_coordinator_leases WHERE root_run_id=?1", vec![root]),
        ("mode", "SELECT rowid,* FROM native_web_mode_receipts WHERE scan_id=?1 AND attempt_number=CAST(?2 AS INTEGER)", vec![scan]),
        ("plan", "SELECT rowid,* FROM agent_runs WHERE id=?1", vec![root]),
        ("model", "SELECT rowid,* FROM agent_root_tick_receipts WHERE root_run_id=?1 AND round=1 AND phase='request'", vec![root]),
        ("context", "SELECT rowid,* FROM sentinel_scan_contexts WHERE scan_id=?1", vec![scan]),
        ("sessions", "SELECT rowid,* FROM browser_auth_sessions WHERE owner_scan_id=?1 ORDER BY rowid", vec![scan]),
    ] {
        let attempt_text = attempt.to_string();
        let mut keys = keys;
        if name == "mode" { keys.push(&attempt_text); }
        let rows = NativeCoordinatorFrozenRows::capture_optional(db, sql, &keys)?;
        evidence.insert(name.into(), json!(rows.hash));
    }
    evidence.insert(
        "history".into(),
        json!(store::stable_hash(&json!(historical_proofs).to_string())),
    );
    source.verify_on(db)?;
    lease::validate_coordinator_lease(db, &actor)?;
    if private_web_mode_on(db, scan, attempt)?.fact() != original.fact() {
        return Err("financial_continuation_original_mode_changed".into());
    }
    let mut value = json!({"schema":1,"scanId":scan,"attemptNumber":attempt,"rootRunId":root,"receiptId":receipt,
        "proposedAttemptNumber":proposed,"targetKey":target,"previousStatus":status,
        "financialSource":source.as_json(),"mode":"multi","originalModeReceiptId":original.fact().receipt_id,
        "modelBindingHash":model_hash,"identityMode":format!("{identity_mode:?}"),"identityCount":identity_ids.len(),
        "evidenceHashes":evidence,"historicalRootsChecked":roots.len(),
        "taskPurpose":"new_work_after_original_cost_confirmation","originalSemanticReplayAllowed":false,
        "taskDescription":{"purpose":"fresh_root_assessment_of_frozen_web_evidence","targetKey":target,
            "originalPlanHash":plan_hash,"originalSemanticReplayAllowed":false,"limits":"remaining_original_budget_and_deadline"},
        "executionAllowed":false,"automaticResumeAllowed":false,"issuanceImplemented":true});
    value["proposalHash"] = store::stable_hash(&value.to_string()).into();
    Ok(value)
}

fn native_financial_continuation_preview_on(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    root: &str,
    receipt: &str,
) -> Result<JsonValue, String> {
    let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
    native_financial_continuation_preview_in(&snapshot, scan, attempt, root, receipt)
}

#[tauri::command]
pub fn preview_native_root_financial_continuation(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    root_run_id: String,
    receipt_id: String,
) -> Result<JsonValue, String> {
    let db = db::open(&state.db_path)?;
    native_financial_continuation_preview_on(
        &db,
        &scan_id,
        attempt_number,
        &root_run_id,
        &receipt_id,
    )
}
