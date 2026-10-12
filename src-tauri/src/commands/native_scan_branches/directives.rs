fn directive_recipient_roots(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt: i64,
    thread_key: &str,
) -> Result<Vec<(String, String)>, String> {
    if thread_key.is_empty() || (thread_key != "team" && thread_key.chars().count() > 200) {
        return Err("directive_thread_not_in_current_root".into());
    }
    let mut statement = connection.prepare(
        "SELECT run.id,run.target_url FROM agent_runs AS run \
         WHERE run.scan_id=?1 AND run.attempt_number=?2 AND run.role='coordinator' \
         AND run.status IN ('prepared','running','paused') \
         AND (run.root_run_id=run.id OR (run.root_run_id='' AND run.orchestration_policy='single' AND run.parent_run_id IS NULL AND run.assignment_id='')) \
         AND (?3='team' OR run.target_url=?3 OR 'coordinator:' || run.id=?3 \
           OR EXISTS(SELECT 1 FROM agent_assignments a WHERE a.coordinator_run_id=run.id AND a.id=?3) \
           OR EXISTS(SELECT 1 FROM agent_messages m WHERE m.root_run_id=run.id AND m.correlation_id=?3)) \
         ORDER BY run.id",
    ).map_err(|error| error.to_string())?;
    let roots = statement
        .query_map(params![scan_id, attempt, thread_key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(roots)
}

fn scan_directive_scope(
    connection: &rusqlite::Connection,
    scan_id: &str,
    thread_key: &str,
) -> Result<(i64, String, String, i64, String), String> {
    let attempt: i64 = connection
        .query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1 \
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => "任务不存在".to_string(),
            other => format!("无法读取指令任务尝试：{other}"),
        })?;
    let roots = directive_recipient_roots(connection, scan_id, attempt, thread_key)?;
    let (root_run_id, target_key) = match roots.as_slice() {
        [root] => root.clone(),
        [] => return Err("directive_thread_has_no_coordinator".into()),
        _ => return Err("directive_thread_coordinator_ambiguous".into()),
    };
    let lease: Option<(i64, String)> = if root_run_id.is_empty() {
        None
    } else {
        connection.query_row(
            "SELECT lease_epoch,fencing_token FROM agent_coordinator_leases WHERE scan_id=?1 AND attempt_number=?2 \
             AND target_key=?3 AND root_run_id=?4 AND lease_expires_at>datetime('now','localtime')",
            params![scan_id, attempt, target_key, root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(|error| error.to_string())?
    };
    let (lease_epoch, fencing_token) = lease.unwrap_or_default();
    Ok((attempt, root_run_id, target_key, lease_epoch, fencing_token))
}

#[tauri::command]
pub fn draft_scan_directive(
    state: State<'_, AppState>,
    scan_id: String,
    text: String,
    thread_key: Option<String>,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    draft_scan_directive_in(
        &connection,
        &scan_id,
        &text,
        thread_key.as_deref().unwrap_or("team"),
    )
}

fn draft_scan_directive_in(
    connection: &rusqlite::Connection,
    scan_id: &str,
    text: &str,
    thread_key: &str,
) -> Result<JsonValue, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("请输入要修正的内容".into());
    }
    if text.chars().count() > 2_000 {
        return Err("一次修正最多 2000 个字".into());
    }
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定用户指令草案：{error}"))?;
    let (attempt, root_run_id, target_key, lease_epoch, fencing_token) =
        scan_directive_scope(&transaction, scan_id, thread_key)?;
    let draft = crate::agent_runtime::multi_agent::directive::create_draft_in_transaction(
        &transaction,
        scan_id,
        attempt,
        &root_run_id,
        &target_key,
        "coordinator",
        &text,
        thread_key,
        lease_epoch,
        &fencing_token,
    )?;
    let value = crate::agent_runtime::multi_agent::homogeneous::material_policy::json(&transaction, &root_run_id,
        &serde_json::to_value(draft).map_err(|error| error.to_string())?,
    )?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交用户指令草案：{error}"))?;
    Ok(value)
}

/// Backward-compatible command name. It deliberately creates only a draft.
#[tauri::command]
pub fn post_scan_directive(
    state: State<'_, AppState>,
    scan_id: String,
    text: String,
) -> Result<JsonValue, String> {
    draft_scan_directive(state, scan_id, text, None)
}

#[tauri::command]
pub fn confirm_scan_directive(
    state: State<'_, AppState>,
    scan_id: String,
    draft_id: String,
    revision: i64,
    draft_hash: String,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    let directive = crate::agent_runtime::multi_agent::directive::confirm_bound_draft(
        &connection,
        &scan_id,
        &draft_id,
        revision,
        &draft_hash,
    )?;
    Ok(json!({
        "id": directive.id,
        "status": directive.status,
        "accepted": directive.status == "pending",
        "message": "已确认并进入当前尝试的 Coordinator 队列",
    }))
}

#[tauri::command]
pub fn cancel_scan_directive(
    state: State<'_, AppState>,
    scan_id: String,
    draft_id: String,
    revision: i64,
    draft_hash: String,
) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    crate::agent_runtime::multi_agent::directive::cancel_draft(
        &connection,
        &scan_id,
        &draft_id,
        revision,
        &draft_hash,
    )
}

/// Explicit local-only recovery; never uses the active-task execution scope or
/// renews a lease. The frozen scan/attempt/directive binding is checked in SQL.
#[tauri::command]
pub fn reconcile_scan_directive_receipt(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    directive_id: String,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_received(
        &connection,
        &scan_id,
        attempt_number,
        &directive_id,
    )
}

/// Deliberately separate from current-attempt reconciliation. It does not
/// renew a lease, resume a run, dispatch a child or issue a target/model call.
#[tauri::command]
pub fn reconcile_historical_scan_directive_receipt(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    directive_id: String,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    crate::agent_runtime::multi_agent::directive::reconciliation::reconcile_historical_received(
        &connection,
        &scan_id,
        attempt_number,
        &directive_id,
    )
}

fn review_scan_directive_in(
    connection: &rusqlite::Connection,
    scan_id: &str,
    draft_id: &str,
    revision: i64,
    draft_hash: &str,
    kind: &str,
    argument: &str,
) -> Result<JsonValue, String> {
    crate::agent_runtime::multi_agent::directive::human_review::review(
        connection, scan_id, draft_id, revision, draft_hash, kind, argument,
    )
}

#[tauri::command]
pub fn revise_scan_directive(
    state: State<'_, AppState>,
    scan_id: String,
    draft_id: String,
    revision: i64,
    draft_hash: String,
    text: String,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    review_scan_directive_in(
        &connection,
        &scan_id,
        &draft_id,
        revision,
        &draft_hash,
        "revise",
        &text,
    )
}

#[tauri::command]
pub fn reject_scan_directive(
    state: State<'_, AppState>,
    scan_id: String,
    draft_id: String,
    revision: i64,
    draft_hash: String,
    reason: String,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    review_scan_directive_in(
        &connection,
        &scan_id,
        &draft_id,
        revision,
        &draft_hash,
        "reject",
        &reason,
    )
}
