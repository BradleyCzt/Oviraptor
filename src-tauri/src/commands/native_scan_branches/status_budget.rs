/// A bounded, attempt-scoped projection. This is an accounting diagnostic,
/// not a budget authority or a reason to stop/resume a task.
fn native_budget_diagnostics(
    snapshot: &rusqlite::Connection,
    scan_id: &str,
    attempt: i64,
) -> Result<JsonValue, String> {
    let total: i64 = snapshot.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 \
         AND backend='native' AND role='coordinator' AND (root_run_id='' OR root_run_id=id)",
        params![scan_id, attempt],
        |row| row.get(0),
    ).map_err(|error| format!("无法盘点预算 root：{error}"))?;
    let mut query = snapshot.prepare(
        "SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 \
         AND backend='native' AND role='coordinator' AND (root_run_id='' OR root_run_id=id) \
         ORDER BY id LIMIT 50",
    ).map_err(|error| format!("无法读取预算 root：{error}"))?;
    let roots = query.query_map(params![scan_id, attempt], |row| row.get::<_, String>(0))
        .map_err(|error| format!("无法读取预算 root：{error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("无法读取预算 root：{error}"))?;
    let reports = roots.into_iter().map(|root| {
        let gaps = crate::agent_runtime::multi_agent::budget_gaps::budget_ledger_gaps_in_snapshot(snapshot, &root)?;
        let single_original_history = native_single_original_history(snapshot, &root)?;
        Ok(json!({"rootRunId":root,"gaps":gaps,"singleOriginalHistory":single_original_history}))
    }).collect::<Result<Vec<_>, String>>()?;
    Ok(json!({"roots":reports,"totalRoots":total,"truncated":total > 50,
        "authoritative":false,"schema":"summary_gap_v1"}))
}

/// Deliberately projects fixed diagnostic codes, never raw audit/SQLite errors.
/// A closed original invoice is not consent, a dollar invoice or new capacity.
fn native_single_original_history(db: &rusqlite::Connection, root: &str) -> Result<JsonValue, String> {
    let single: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.agent_runs r JOIN main.sentinel_scans s ON s.id=r.scan_id
         WHERE r.id=?1 AND r.backend='native' AND r.role='coordinator'
         AND r.parent_run_id IS NULL AND r.assignment_id='' AND r.orchestration_policy='single'
         AND s.scan_type='web')", [root], |row| row.get(0),
    ).map_err(|_| "budget_single_history_scope_unavailable".to_owned())?;
    if !single { return Ok(JsonValue::Null); }
    let verified = crate::agent_runtime::multi_agent::budget::continuation::verify_settled_history_on(db, root).is_ok();
    Ok(json!({"schemaVersion":1,"rootRunId":root,
        "state":if verified {"verified_closed_history"} else {"not_verified"},
        "authoritative":false,"executionAllowed":false,"targetRequestsGranted":0,
        "basis":"original_runtime_records_not_provider_dollar_invoice"}))
}

fn native_budget_diagnostics_for_attempt(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt: i64,
) -> Result<JsonValue, String> {
    if attempt <= 0 { return Err("无效的任务轮次".into()); }
    let snapshot = connection.unchecked_transaction().map_err(|error| error.to_string())?;
    let visible: bool = snapshot.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2) \
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|error| format!("无法核验预算任务作用域：{error}"))?;
    if !visible { return Err("任务不存在或尝试已变化".into()); }
    let result = native_budget_diagnostics(&snapshot, scan_id, attempt)?;
    Ok(crate::agent_runtime::secrets::redact_json(&result))
}

#[tauri::command]
pub fn get_native_budget_diagnostics(
    state: State<'_, AppState>, scan_id: String, attempt_number: i64,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    native_budget_diagnostics_for_attempt(&connection, &scan_id, attempt_number)
}
