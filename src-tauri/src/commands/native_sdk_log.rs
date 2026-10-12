// Separate SDK stage channel: identity/cursor hints only, never model content.
pub(crate) fn notify_native_sdk_log(
    hint: &crate::agent_runtime::model::diagnostics::Hint,
) -> Result<(), String> {
    let Some(app) = NEST_LOG_APP.get() else {
        return Ok(());
    };
    app.emit("nest-native-sdk-log", hint)
        .map_err(|_| "native_sdk_log_notification_unavailable".into())
}

#[tauri::command]
pub async fn read_native_sdk_log(
    state: State<'_, AppState>,
    scan_id: String,
    attempt: i64,
    cursor_attempt: Option<i64>,
    owner_id: Option<String>,
    after_sequence: i64,
    limit: Option<i64>,
) -> Result<crate::agent_runtime::model::diagnostics::replay::Snapshot, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::agent_runtime::model::diagnostics::replay::read(
            &db_path,
            &scan_id,
            attempt,
            cursor_attempt,
            owner_id.as_deref(),
            after_sequence,
            limit.unwrap_or(300),
        )
    })
    .await
    .map_err(|_| "native_sdk_log_read_failed".to_string())?
}

// Original history is a projection, never a reconstructed live session.
fn read_native_mapper_prepared_history_at(
    db_path: &Path, scan_id: &str, attempt: i64, root_run_id: &str,
) -> Result<Option<JsonValue>, String> {
    if scan_id.is_empty() || root_run_id.is_empty() || attempt < 1 {
        return Err("mapper_prepared_history_scope_invalid".into());
    }
    let db = rusqlite::Connection::open_with_flags(db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|error| error.to_string())?;
    db.pragma_update(None, "query_only", true).map_err(|error| error.to_string())?;
    let snapshot = db.unchecked_transaction().map_err(|error| error.to_string())?;
    let matched: bool = snapshot.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND root_run_id=?1 AND scan_id=?2 AND attempt_number=?3 AND role='coordinator' AND backend='native' AND orchestration_policy='multi' AND parent_run_id IS NULL)",
        params![root_run_id, scan_id, attempt], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if !matched { return Err("mapper_prepared_history_original_scope_mismatch".into()); }
    let raw: String = snapshot.query_row(
        "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
        [root_run_id], |row| row.get(0),
    ).map_err(|_| "mapper_prepared_history_original_control_missing")?;
    let contract: JsonValue = serde_json::from_str(&raw).map_err(|_| "mapper_prepared_history_original_control_invalid")?;
    let control = &contract["coordinator"];
    let text = |key: &str| control[key].as_str().map(str::to_owned)
        .ok_or_else(|| "mapper_prepared_history_original_control_invalid".to_string());
    let actor = crate::agent_runtime::multi_agent::lease::CoordinatorLease {
        scan_id: text("scan")?,
        attempt_number: control["attempt"].as_i64().ok_or("mapper_prepared_history_original_control_invalid")?,
        target_key: text("target")?, root_run_id: root_run_id.into(),
        lease_epoch: control["epoch"].as_i64().ok_or("mapper_prepared_history_original_control_invalid")?,
        fencing_token: text("fence")?, lease_expires_at: String::new(),
    };
    if actor.scan_id != scan_id || actor.attempt_number != attempt {
        return Err("mapper_prepared_history_original_scope_mismatch".into());
    }
    native_mapper_prepared_history_read_on(&snapshot, &actor)
        .map(|original| original.map(|projection| crate::agent_runtime::secrets::redact_json(projection.fact())))
}

#[tauri::command]
pub async fn read_native_mapper_prepared_history(
    state: State<'_, AppState>, scan_id: String, attempt: i64, root_run_id: String,
) -> Result<Option<JsonValue>, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move ||
        read_native_mapper_prepared_history_at(&db_path, &scan_id, attempt, &root_run_id))
        .await.map_err(|_| "mapper_prepared_history_read_failed".to_string())?
}
