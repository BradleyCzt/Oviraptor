#[tauri::command]
pub fn start_workbench_scan(
    app: AppHandle,
    state: State<AppState>,
    input: WorkbenchScanInput,
) -> Result<SentinelScan, String> {
    start_workbench_scan_impl(&app, &state, WorkbenchStartRequest::New(Box::new(input)))
}

#[tauri::command]
pub fn rescan_workbench_scan(
    app: AppHandle,
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    start_workbench_scan_impl(&app, &state, WorkbenchStartRequest::Retry(scan_id))
}

#[tauri::command]
pub fn confirm_sentinel_scan(
    app: AppHandle,
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    start_web_scan(app, state, scan_id, WebStartMode::Confirm)
}

fn start_web_scan(
    app: AppHandle,
    state: State<AppState>,
    scan_id: String,
    mode: WebStartMode,
) -> Result<SentinelScan, String> {
    let _owner = claim_scan_control(&state.db_path, &scan_id)?;
    let tool_configuration = crate::tool_supply::ToolAdministratorConfiguration::from_app(&app)?;
    let mut database = db::open(&state.db_path)?;
    database.pragma_update(None, "synchronous", "FULL").map_err(|error| error.to_string())?;
    let connection = database.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    // Retain all previous-attempt worker locks until admission is committed.
    // A paused/failed label is not proof that old callers have actually exited.
    let _previous_workers = claim_scan_quiescence_in(&connection,&state.db_path,&scan_id)?;
    let mut startup = prepare_web_startup_in(&connection, &state.app_data_dir, &scan_id, mode)?;
    let project_id = startup.project_id;
    let project_name = &startup.project_name;
    let targets = startup.targets.clone();
    let attempt_number = startup.attempt;
    let work_dir = startup.files.path.clone();
    let settings = sentinel_settings(&connection);
    let home = state
        .app_data_dir
        .parent()
        .unwrap_or(&state.app_data_dir)
        .to_path_buf();
    let model_environment = model_runtime_env(&settings)?;
    // One shared runtime resolution for both web task entries.
    let agent_runtime =
        agent_web_pipeline_runtime(&app, &connection, &scan_id, &model_environment.deployment)?;
    let web_policy = agent_runtime.web_policy.clone();
    let skill_names = agent_runtime.skill_names.clone();
    let skill_instructions = agent_runtime.skill_instructions.clone();
    let adaptive = agent_runtime.adaptive.clone();
    let proxies = agent_runtime.proxies.clone();
    let no_proxy = agent_runtime.no_proxy.clone();
    let packet_budget = agent_runtime.packet_budget;
    let worker = agent_runtime.worker.clone();
    let runtime_path = sentinel_runtime_path(&home);
    let (startup_idle_timeout, startup_hard_timeout) =
        model_startup_timeouts(&model_environment);
    let task_path = work_dir.join("task.json");
    // This file is an immutable execution plan. Runtime status and checkpoints
    // live only in sentinel_scans/sentinel_scan_attempts; duplicating them here
    // left every completed plan permanently saying `queued` and encouraged
    // accidental reuse of stale state during diagnostics.
    let payload = serde_json::json!({"scanId":scan_id,"projectId":project_id,"projectName":project_name,"targets":targets.iter().map(|(company,url)|serde_json::json!({"company":company,"url":url})).collect::<Vec<_>>(),"frontendReconStrategy":"coverage-led-browser-exploration+evidence-validation","queueOrder":"fifo","effectiveWebPolicy":web_policy.clone(),"skills":skill_names.clone(),"adaptiveRouting":{"enabled":true,"forcedMode":"coverage-led","modeCeiling":adaptive.max_mode.clone(),"maxBudgetUsd":adaptive.max_budget_usd,"quickScore":adaptive.quick_score,"standardScore":adaptive.standard_score,"deepScore":adaptive.deep_score,"quickTimeout":adaptive.quick_timeout,"standardTimeout":adaptive.standard_timeout,"deepTimeout":adaptive.deep_timeout,"quickTokenLimit":adaptive.quick_tokens,"standardTokenLimit":adaptive.standard_tokens,"deepTokenLimit":adaptive.deep_tokens,"quickRequestLimit":adaptive.quick_requests,"standardRequestLimit":adaptive.standard_requests,"deepRequestLimit":adaptive.deep_requests,"noToolTurnLimit":adaptive.no_tool_turn_limit,"startupIdleTimeout":startup_idle_timeout,"startupHardTimeout":startup_hard_timeout},"llmPolicy":{"model":model_environment.llm,"deployment":model_environment.deployment,"fullPower":model_environment.full_power,"promptAuditMode":model_environment.prompt_audit_mode},"runtimePolicy":{"backend":"native-agent"},"authorizedProxyPool":!proxies.is_empty(),"createdAt":chrono::Utc::now().to_rfc3339()});
    fs::write(
        &task_path,
        serde_json::to_vec_pretty(&payload).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let auth_session_path = if let Some(document) = web_dispatch_auth_document(&connection, project_id, &web_policy)? {
        let path = work_dir.join("auth-sessions.json");
        crate::auth_session::write_session_document(&path, &document)?;
        Some(path)
    } else { None };
    let targets_json = work_dir.join("targets.json");
    fs::write(
        &targets_json,
        serde_json::to_vec_pretty(
            &targets
                .iter()
                .map(|(company, url)| serde_json::json!({"company":company,"url":url}))
                .collect::<Vec<_>>(),
        )
        .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let targets_txt = work_dir.join("targets.txt");
    fs::write(
        &targets_txt,
        targets
            .iter()
            .map(|(_, url)| url.replace(['\r', '\n'], ""))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .map_err(|error| error.to_string())?;
    let instruction = build_web_investigation_instruction(
        &web_policy,
        &skill_instructions,
        model_environment.deployment == "local",
    );
    instruction.write_to(&work_dir)?;
    write_model_prompt_audit(&work_dir, instruction.as_str(), &model_environment)?;
    let checkpoint = if model_environment.full_power { format!("第 {attempt_number} 次执行：{} 个 URL；逐 URL 前端探测后进入后端队列；本地火力全开仅放宽普通 Web，现代前端仍执行定向验证硬上限",targets.len()) } else { format!("第 {attempt_number} 次执行：{} 个 URL；逐 URL 探测后立即进入任务队列",targets.len()) };
    let result = persist_web_startup_in(&connection, &startup, &checkpoint, &skill_names, &web_policy)?;
    let runtime_binding = web_dispatch_runtime_binding(&connection, &agent_runtime, &model_environment, &runtime_path)?;
    register_web_dispatch_binding_in(&connection, &startup, &runtime_binding)?;
    register_private_web_mode_on(&connection,&startup,&runtime_binding)?;
    let admission = WebDispatchAdmission { home, settings, claimed_guard: None,
        recon_config: serde_json::from_value(runtime_binding["frontendConfig"].clone())
            .map_err(|_| "web_binding_frontend_config_invalid")? };
    // A commit error can be ambiguous. Keep the owned files, never dispatch a
    // worker on error, and do not compensate by overwriting another attempt.
    startup.files.preserve = true;
    connection.commit().map_err(|error| format!("web_start_commit_unconfirmed:{error}"))?;
    launch_sentinel_url_pipeline_with_tool_configuration(
        state.db_path.clone(),
        scan_id,
        attempt_number as i64,
        worker,
        work_dir,
        targets,
        proxies,
        no_proxy,
        model_environment,
        runtime_path,
        adaptive,
        packet_budget,
        auth_session_path,
        Some(admission),
        Some(tool_configuration),
    )?;
    Ok(result)
}

#[tauri::command]
pub fn pause_sentinel_scan(
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    let attempt = request_sentinel_pause(&state.db_path,&scan_id)?;
    // Native callers observe cancellation and stop their own process handles.
    // Do not signal unverified historical PIDs or erase cleanup evidence here.
    let runner_log = scan_work_dir_for(&state.app_data_dir, &scan_id).join("oviraptor-runner.log");
    let finalization = finish_sentinel_pause(&state.db_path,&scan_id,attempt);
    append_runner_log(&runner_log,&format!("pause requested; local worker quiescence result: {finalization:?}; no automatic replay"));
    let connection = db::open(&state.db_path)?;
    sentinel_scan_by_id(&connection, &scan_id)
}

#[tauri::command]
pub fn resume_sentinel_scan(
    app: AppHandle,
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    let db_path = state.db_path.clone();
    let connection = db::open(&db_path)?;
    let (status, scan_type): (String, String) = connection
        .query_row(
            "SELECT status,scan_type FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| "任务不存在".to_string())?;
    if !matches!(status.as_str(), "paused" | "partial") {
        return Err(format!("任务当前状态为 {status}，不能恢复"));
    }
    // Never reinterpret an unsealed retired run as Native after startup stopped
    // rewriting historical rows. Database lookup failures also block resume.
    if let Some(refusal) = retired_backend_resume_refusal(&connection, &scan_id)? {
        return Err(refusal);
    }
    if scan_type != "web" {
        drop(connection);
        return rescan_workbench_scan(app, state, scan_id);
    }
    drop(connection);
    start_web_scan(app, state, scan_id, WebStartMode::Resume)
}

#[tauri::command]
pub fn cancel_sentinel_scan(state: State<AppState>, scan_id: String) -> Result<(), String> {
    let _owner = claim_scan_control(&state.db_path, &scan_id)?;
    let database = db::open(&state.db_path)?;
    cancel_draft_scan_in(&database, &scan_id)
}

fn cancel_draft_scan_in(database: &rusqlite::Connection, scan_id: &str) -> Result<(),String> {
    let connection = rusqlite::Transaction::new_unchecked(database, rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let status: String = connection
        .query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&scan_id],
            |r| r.get(0),
        )
        .map_err(|_| "任务不存在".to_string())?;
    if connection.query_row("SELECT EXISTS(SELECT 1 FROM main.native_protection_recovery_drafts WHERE scan_id=?1)",
        [scan_id], |r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {
        native_protection_recovery_cancel_draft_in(&connection, scan_id)?;
        connection.commit().map_err(|e|format!("protection_recovery_cancel_commit_unconfirmed:{e}"))?;
        return Ok(());
    }
    if status != "draft" {
        return Err("只有未确认任务可以删除；已确认任务请保留审计记录".into());
    }
    connection
        .execute(
            "DELETE FROM browser_auth_sessions WHERE owner_scan_id=?1",
            [&scan_id],
        )
        .map_err(|error| error.to_string())?;
    let changed = connection
        .execute("DELETE FROM sentinel_scans WHERE id=?1 AND status='draft'", [&scan_id])
        .map_err(|e| e.to_string())?;
    if changed != 1 { return Err("cancel_scan_state_changed".into()); }
    let remains: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1) OR EXISTS(SELECT 1 FROM browser_auth_sessions WHERE owner_scan_id=?1)", [&scan_id], |r| r.get(0)).map_err(|e| e.to_string())?;
    if remains { return Err("cancel_scan_not_persisted".into()); }
    connection.commit().map_err(|error| error.to_string())?;
    // Task paths may belong to historical attempts. Deleting a draft is not
    // authority to remove an arbitrary persisted path or immutable history.
    Ok(())
}

#[cfg(not(unix))]
fn request_stop_sentinel_process(process_id: i64) {
    if process_id <= 0 {
        return;
    }
    if cfg!(target_os = "windows") {
        let _ = Command::new("taskkill")
            .args(["/PID", &process_id.to_string(), "/T"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        let pid = process_id.to_string();
        let process_group = format!("-{pid}");
        let _ = Command::new("kill")
            .args(["-TERM", "--", &process_group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("pkill")
            .args(["-TERM", "-P", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-TERM", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(not(unix))]
fn force_stop_sentinel_process(process_id: i64) {
    if process_id <= 0 {
        return;
    }
    request_stop_sentinel_process(process_id);
    thread::sleep(Duration::from_millis(250));
    if cfg!(target_os = "windows") {
        let _ = Command::new("taskkill")
            .args(["/PID", &process_id.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        let pid = process_id.to_string();
        let process_group = format!("-{pid}");
        let _ = Command::new("kill")
            .args(["-KILL", "--", &process_group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("pkill")
            .args(["-KILL", "-P", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-KILL", &pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[tauri::command]
pub fn delete_sentinel_scan(state: State<AppState>, scan_id: String) -> Result<(), String> {
    delete_sentinel_scan_inner(&state.db_path, &scan_id)
}

pub(crate) fn list_sentinel_scans_inner(
    db_path: &Path,
    project_id: Option<i64>,
    limit: Option<i64>,
    offset: Option<i64>,
    before: Option<(&str, &str)>,
) -> Result<Vec<SentinelScan>, String> {
    let connection = db::open(db_path)?;
    let mut s = connection
        .prepare(&format!(
            "SELECT {SENTINEL_SCAN_COLUMNS} FROM sentinel_scans WHERE (?1 IS NULL OR project_id=?1) AND (?4 IS NULL OR updated_at < ?4 OR (updated_at=?4 AND id < ?5)) ORDER BY updated_at DESC,id DESC LIMIT ?2 OFFSET ?3"
        ))
        .map_err(|e| e.to_string())?;
    let rows = s
        .query_map(
            params![project_id, limit.unwrap_or(300).clamp(1, 2000), offset.unwrap_or(0).max(0), before.map(|pair| pair.0), before.map(|pair| pair.1)],
            sentinel_scan_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string());
    rows
}

#[tauri::command]
pub async fn list_sentinel_scans(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    limit: Option<i64>,
    offset: Option<i64>,
    before_updated_at: Option<String>,
    before_id: Option<String>,
) -> Result<Vec<SentinelScan>, String> {
    if before_updated_at.is_some() != before_id.is_some() {
        return Err("历史任务游标必须包含更新时间和任务编号".into());
    }
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        list_sentinel_scans_inner(&db_path, project_id, limit, offset, before_updated_at.as_deref().zip(before_id.as_deref()))
    })
    .await
    .map_err(|error| format!("任务列表读取线程失败：{error}"))?
}

fn search_sentinel_scan_page_inner(
    db_path: &Path,
    project_id: Option<i64>,
    search: &str,
    view: &str,
    limit: i64,
    before: Option<(&str, &str)>,
) -> Result<Vec<SentinelScan>, String> {
    if !matches!(view, "attention" | "history" | "all") {
        return Err("未知任务视图".into());
    }
    let search = search.trim();
    if search.is_empty() || search.chars().count() > 200 {
        return Err("搜索词长度必须在 1 到 200 字符之间".into());
    }
    // Treat %, _ and the escape character literally; the search is a substring
    // match, not a caller-controlled SQL pattern.
    let pattern = format!("%{}%", search.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let connection = db::open(db_path)?;
    let mut statement = connection.prepare(&format!(
        "SELECT {SENTINEL_SCAN_COLUMNS} FROM sentinel_scans WHERE (?1 IS NULL OR project_id=?1) \
         AND (?2='all' OR (?2='history' AND status IN ('completed','completed_with_gaps','cancelled','failed')) \
         OR (?2='attention' AND status NOT IN ('completed','completed_with_gaps','cancelled','failed'))) \
         AND (id LIKE ?3 ESCAPE '\\' OR task_name LIKE ?3 ESCAPE '\\' \
         OR project_name LIKE ?3 ESCAPE '\\' OR scan_type LIKE ?3 ESCAPE '\\' \
         OR EXISTS(SELECT 1 FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id \
         AND (t.company LIKE ?3 ESCAPE '\\' OR t.url LIKE ?3 ESCAPE '\\')) \
         OR EXISTS(SELECT 1 FROM sentinel_findings f WHERE f.scan_id=sentinel_scans.id \
         AND f.target_url LIKE ?3 ESCAPE '\\') \
         OR EXISTS(SELECT 1 FROM sentinel_scan_attempts a WHERE a.scan_id=sentinel_scans.id \
         AND a.stop_reason LIKE ?3 ESCAPE '\\')) \
         AND (?4 IS NULL OR updated_at < ?4 OR (updated_at=?4 AND id < ?5)) \
         ORDER BY updated_at DESC,id DESC LIMIT ?6"
    )).map_err(|error| error.to_string())?;
    let rows = statement.query_map(
        params![project_id, view, pattern, before.map(|pair| pair.0), before.map(|pair| pair.1), limit.clamp(1, 300)],
        sentinel_scan_row,
    ).map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string());
    rows
}

#[tauri::command]
pub async fn search_sentinel_scan_page(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    search: String,
    view: String,
    limit: i64,
    before_updated_at: Option<String>,
    before_id: Option<String>,
) -> Result<Vec<SentinelScan>, String> {
    if before_updated_at.is_some() != before_id.is_some() {
        return Err("搜索游标必须包含更新时间和任务编号".into());
    }
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        search_sentinel_scan_page_inner(&db_path, project_id, &search, &view, limit,
            before_updated_at.as_deref().zip(before_id.as_deref()))
    }).await.map_err(|error| format!("任务搜索线程失败：{error}"))?
}

fn archive_sentinel_scan_inner(
    db_path: &Path,
    scan_id: &str,
    project_id: i64,
    archive: bool,
) -> Result<SentinelScan, String> {
    let connection = db::open(db_path)?;
    let changed = connection.execute(
        "UPDATE sentinel_scans SET archived_at=CASE WHEN ?3=1 THEN datetime('now','localtime') ELSE '' END \
         WHERE id=?1 AND project_id=?2 AND (?3=0 OR status IN ('completed','completed_with_gaps','cancelled','failed'))",
        params![scan_id, project_id, archive as i64],
    ).map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("任务不存在、项目不匹配，或仍有未完成义务；不能归档".into());
    }
    sentinel_scan_by_id(&connection, scan_id)
}

#[tauri::command]
pub async fn archive_sentinel_scan(
    state: State<'_, AppState>, scan_id: String, project_id: i64, archive: bool,
) -> Result<SentinelScan, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || archive_sentinel_scan_inner(&db_path, &scan_id, project_id, archive))
        .await.map_err(|error| format!("任务归档线程失败：{error}"))?
}

#[tauri::command]
pub fn list_sentinel_scan_attempts(
    state: State<AppState>,
    scan_id: String,
) -> Result<Vec<SentinelScanAttempt>, String> {
    let connection = db::open(&state.db_path)?;
    sync_sentinel_attempt(&connection, &scan_id);
    let mut statement = connection
        .prepare(
            "SELECT scan_id,attempt_number,execution_mode,status,stage,checkpoint,stop_reason,work_dir,COALESCE(backend_plan_json,''),llm_requests_delta,input_tokens_delta,output_tokens_delta,cached_tokens_delta,total_tokens_delta,started_at,finished_at,updated_at FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([scan_id], |row| {
            Ok(SentinelScanAttempt {
                scan_id: row.get(0)?,
                attempt_number: row.get(1)?,
                execution_mode: row.get(2)?,
                status: row.get(3)?,
                stage: row.get(4)?,
                checkpoint: row.get(5)?,
                stop_reason: row.get(6)?,
                work_dir: row.get(7)?,
                backend_plan_json: row.get(8)?,
                llm_requests: row.get(9)?,
                input_tokens: row.get(10)?,
                output_tokens: row.get(11)?,
                cached_tokens: row.get(12)?,
                total_tokens: row.get(13)?,
                started_at: row.get(14)?,
                finished_at: row.get(15)?,
                updated_at: row.get(16)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_sentinel_vulnerability_scan_ids(
    state: State<'_, AppState>,
    project_id: Option<i64>,
) -> Result<Vec<String>, String> {
    let database = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = db::open(&database)?;
        sentinel_vulnerability_scan_ids_in(&connection, project_id)
    }).await.map_err(|_| "findings_index_reader_failed".to_string())?
}

fn sentinel_vulnerability_scan_ids_in(
    connection: &rusqlite::Connection, project_id: Option<i64>,
) -> Result<Vec<String>, String> {
    // Stream one query snapshot. Use the same typed visibility predicate as
    // the detail reader, not SQLite coercion of arbitrary JSON into strings.
    let mut statement = connection
        .prepare(
            "SELECT f.scan_id,c.policy_json,f.record_json FROM sentinel_findings f \
             JOIN sentinel_scans s ON s.id=f.scan_id \
             LEFT JOIN sentinel_scan_contexts c ON c.scan_id=f.scan_id \
             WHERE f.kind='vulnerability' AND (?1 IS NULL OR s.project_id=?1) \
               AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id) \
             ORDER BY s.updated_at DESC,s.id DESC,f.id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement.query_map([project_id], |row| Ok((
        row.get::<_,String>(0)?, row.get::<_,Option<String>>(1)?, row.get::<_,String>(2)?,
    ))).map_err(|error| error.to_string())?;
    let mut selected = Vec::new();
    let mut current: Option<(String, bool)> = None;
    for row in rows {
        let (scan_id, policy, record) = row.map_err(|error| error.to_string())?;
        if current.as_ref().is_none_or(|(id,_)| id != &scan_id) {
            current = Some((scan_id.clone(), findings_require_proof(policy.as_deref())?));
        }
        if selected.last() == Some(&scan_id) { continue; }
        if !current.as_ref().is_some_and(|(_,proof)| *proof) || proof_finding_is_visible(&record) {
            selected.push(scan_id);
        }
    }
    Ok(selected)
}

/// A runner-log read located through the attempt row instead of a guessed
/// directory, so "no logs yet" and "the log could not be read" stay distinct.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SentinelRunnerLogView {
    scan_id: String,
    attempt: i64,
    attempt_status: String,
    stage: String,
    /// `attempt_work_dir` when the path came from the database, `task_root`
    /// when the scan has no attempt row or the stored directory is unusable.
    source: String,
    work_dir: String,
    updated_at: String,
    /// `ready` | `not_created` | `empty` | `read_failed`.
    status: String,
    message: String,
    lines: Vec<String>,
}

fn runner_log_scan_error(scan_id: &str) -> Option<&'static str> {
    if scan_id.is_empty() {
        return Some("缺少任务 ID");
    }
    if scan_id
        .chars()
        .any(|character| matches!(character, '/' | '\\' | ':'))
        || scan_id.contains("..")
    {
        return Some("invalid scan id");
    }
    None
}

/// The attempt row a runner-log read is anchored to.
#[derive(Clone, Debug, Default)]
struct AttemptLogRow {
    attempt_number: i64,
    status: String,
    stage: String,
    work_dir: String,
    updated_at: String,
}

fn attempt_log_from_row(row: &Row<'_>) -> rusqlite::Result<AttemptLogRow> {
    Ok(AttemptLogRow {
        attempt_number: row.get(0)?,
        status: row.get(1)?,
        stage: row.get(2)?,
        work_dir: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

/// The stored attempt to read, or the newest one when the caller did not pick.
fn sentinel_attempt_log_row(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt: Option<i64>,
) -> Result<Option<AttemptLogRow>, String> {
    const SELECT: &str = "SELECT attempt_number,status,stage,work_dir,updated_at \
        FROM sentinel_scan_attempts WHERE scan_id=?1";
    let failure = |error: rusqlite::Error| format!("无法读取 attempt 记录：{error}");
    if let Some(number) = attempt {
        return connection
            .query_row(
                &format!("{SELECT} AND attempt_number=?2"),
                params![scan_id, number],
                attempt_log_from_row,
            )
            .optional()
            .map_err(failure);
    }
    connection
        .query_row(
            &format!("{SELECT} ORDER BY attempt_number DESC LIMIT 1"),
            [scan_id],
            attempt_log_from_row,
        )
        .optional()
        .map_err(failure)
}

fn read_sentinel_runner_log_inner(
    db_path: &Path,
    app_data_dir: &Path,
    scan_id: &str,
    attempt: Option<i64>,
    limit: Option<usize>,
) -> Result<SentinelRunnerLogView, String> {
    let scan_id = scan_id.trim();
    if let Some(reason) = runner_log_scan_error(scan_id) {
        return Err(reason.into());
    }
    let connection = db::open(db_path)?;
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1)",
            [scan_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err("任务不存在".into());
    }
    let requested = attempt.filter(|number| *number > 0);
    let stored = sentinel_attempt_log_row(&connection, scan_id, requested)?;
    if let (None, Some(number)) = (&stored, requested) {
        return Err(format!("任务 {scan_id} 没有第 {number} 次执行的记录"));
    }
    let stored = stored.unwrap_or_default();
    let stored_dir = stored.work_dir.trim();
    let (path, source) = if !stored_dir.is_empty() && Path::new(stored_dir).is_absolute() {
        (
            Path::new(stored_dir).join("oviraptor-runner.log"),
            "attempt_work_dir",
        )
    } else {
        (
            scan_work_dir_for(app_data_dir, scan_id).join("oviraptor-runner.log"),
            "task_root",
        )
    };
    let read = read_runner_log_tail(&path, limit.unwrap_or(300).clamp(1, 1000));
    let attempt_number = stored.attempt_number;
    let message = match read.status {
        RunnerLogStatus::Ready => String::new(),
        RunnerLogStatus::NotCreated if attempt_number == 0 => {
            "该任务没有执行记录，已按任务目录查找；日志尚未生成".to_string()
        }
        RunnerLogStatus::NotCreated => format!("第 {attempt_number} 次执行的日志尚未生成"),
        RunnerLogStatus::Empty => format!("日志已生成但没有可显示的内容：{}", read.detail),
        RunnerLogStatus::ReadFailed => format!("日志读取失败：{}", read.detail),
    };
    Ok(SentinelRunnerLogView {
        scan_id: scan_id.to_string(),
        attempt: attempt_number,
        attempt_status: stored.status,
        stage: stored.stage,
        source: source.to_string(),
        work_dir: stored.work_dir,
        updated_at: stored.updated_at,
        status: read.status.as_str().to_string(),
        message,
        lines: read.lines,
    })
}

#[tauri::command]
pub async fn read_sentinel_runner_log(
    state: State<'_, AppState>,
    scan_id: String,
    attempt: Option<i64>,
    limit: Option<usize>,
) -> Result<SentinelRunnerLogView, String> {
    let db_path = state.db_path.clone();
    let app_data_dir = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        read_sentinel_runner_log_inner(&db_path, &app_data_dir, &scan_id, attempt, limit)
    })
    .await
    .map_err(|error| format!("任务日志读取线程失败：{error}"))?
}

fn get_sentinel_runner_log_inner(
    db_path: &Path,
    app_data_dir: &Path,
    scan_id: String,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    read_sentinel_runner_log_inner(
        db_path,
        app_data_dir,
        &scan_id,
        None,
        limit,
    )
    .map(|view| view.lines)
}

#[tauri::command]
pub async fn get_sentinel_runner_log(
    state: State<'_, AppState>,
    scan_id: String,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    let db_path = state.db_path.clone();
    let app_data_dir = state.app_data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        get_sentinel_runner_log_inner(&db_path, &app_data_dir, scan_id, limit)
    })
    .await
    .map_err(|error| format!("任务日志读取线程失败：{error}"))?
}

fn search_sentinel_scan_ids_inner(db_path: &Path, search: String) -> Result<Vec<String>, String> {
    let needle = format!("%{}%", search.trim());
    if search.trim().is_empty() {
        return Ok(Vec::new());
    }
    let connection = db::open(db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT s.id FROM sentinel_scans s LEFT JOIN sentinel_targets t ON t.scan_id=s.id LEFT JOIN sentinel_findings f ON f.scan_id=s.id WHERE s.project_name LIKE ?1 OR s.task_name LIKE ?1 OR s.source_path LIKE ?1 OR s.id LIKE ?1 OR t.company LIKE ?1 OR t.url LIKE ?1 OR f.target_url LIKE ?1 ORDER BY s.updated_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([needle], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn search_sentinel_scan_ids(
    state: State<'_, AppState>,
    search: String,
) -> Result<Vec<String>, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || search_sentinel_scan_ids_inner(&db_path, search))
        .await
        .map_err(|error| format!("任务搜索线程失败：{error}"))?
}

#[tauri::command]
pub async fn list_sentinel_targets(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<SentinelTarget>, String> {
    let connection = db::open(&state.db_path)?;
    // Aggregate URL history once. The previous correlated COUNT re-scanned the
    // entire target table for every row and degraded quadratically.
    let mut s = connection.prepare(
        "WITH filtered AS (
           SELECT *,lower(rtrim(trim(url),'/')) AS normalized_url
           FROM sentinel_targets WHERE (?1 IS NULL OR project_id=?1)
         ), history AS (
           SELECT project_id,normalized_url,COUNT(DISTINCT scan_id) AS scan_count
           FROM filtered GROUP BY project_id,normalized_url
         )
         SELECT t.id,t.project_id,t.scan_id,t.company,t.url,t.status,t.value_score,t.scan_mode,t.routing_reason,t.last_attempt_number,t.created_at,t.updated_at,COALESCE(h.scan_count,0)
         FROM filtered t LEFT JOIN history h ON h.project_id=t.project_id AND h.normalized_url=t.normalized_url
         ORDER BY t.updated_at DESC,t.id DESC LIMIT ?2"
    ).map_err(|e| e.to_string())?;
    let rows = s
        .query_map(
            params![project_id, limit.unwrap_or(5000).clamp(100, 20_000)],
            |r| {
                Ok(SentinelTarget {
                    id: r.get(0)?,
                    project_id: r.get(1)?,
                    scan_id: r.get(2)?,
                    company: r.get(3)?,
                    url: r.get(4)?,
                    status: r.get(5)?,
                    value_score: r.get(6)?,
                    scan_mode: r.get(7)?,
                    routing_reason: r.get(8)?,
                    last_attempt_number: r.get(9)?,
                    created_at: r.get(10)?,
                    updated_at: r.get(11)?,
                    scan_count: r.get(12)?,
                })
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string());
    rows
}

#[tauri::command]
pub fn list_sentinel_fuse_zone(
    state: State<AppState>,
    project_id: Option<i64>,
) -> Result<Vec<SentinelFuseEntry>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(
        "SELECT id,project_id,asset_id,company,url,source_scan_id,reason,verdict,note,evidence,archived,created_at,updated_at FROM sentinel_fuse_zone WHERE (?1 IS NULL OR project_id=?1) ORDER BY archived,updated_at DESC,id DESC",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([project_id], |row| {
            Ok(SentinelFuseEntry {
                id: row.get(0)?,
                project_id: row.get(1)?,
                asset_id: row.get(2)?,
                company: row.get(3)?,
                url: row.get(4)?,
                source_scan_id: row.get(5)?,
                reason: row.get(6)?,
                verdict: row.get(7)?,
                note: row.get(8)?,
                evidence: row.get(9)?,
                archived: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn save_sentinel_fuse_review(
    state: State<AppState>,
    input: SentinelFuseReviewInput,
) -> Result<(), String> {
    let verdict = input.verdict.trim();
    if ![
        "pending",
        "manual_verified",
        "needs_followup",
        "not_reproducible",
    ]
    .contains(&verdict)
    {
        return Err("熔断区人工结论无效".into());
    }
    let connection = db::open(&state.db_path)?;
    let changed = connection.execute(
        "UPDATE sentinel_fuse_zone SET verdict=?1,note=?2,evidence=?3,archived=?4,updated_at=datetime('now','localtime') WHERE id=?5",
        params![verdict, input.note.trim(), input.evidence.trim(), input.archived as i64, input.id],
    ).map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("熔断记录不存在".into());
    }
    Ok(())
}

#[tauri::command]
pub fn remove_sentinel_fuse_entry(
    app: AppHandle,
    state: State<AppState>,
    entry_id: i64,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    let (project_id, company, url, source_scan_id): (i64, String, String, String) = connection
        .query_row(
            "SELECT z.project_id,z.company,z.url,z.source_scan_id FROM sentinel_fuse_zone z JOIN projects p ON p.id=z.project_id AND p.status='active' WHERE z.id=?1",
            [entry_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| "熔断记录不存在，或工作空间已归档；请先恢复工作空间再重试".to_string())?;
    let lifecycle = claim_scan_control(&state.db_path,&source_scan_id)?;
    let (project_name, scan_type, status): (String, String, String) = connection
        .query_row(
            "SELECT project_name,scan_type,status FROM sentinel_scans WHERE id=?1",
            [&source_scan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "熔断来源任务不存在，无法自动重试".to_string())?;
    if scan_type != "web" {
        return Err("熔断 URL 只能回到原 Web 任务继续执行".into());
    }
    if matches!(status.as_str(), "scanning" | "pausing") {
        return Err("来源任务仍在运行，请先暂停后再移出熔断区".into());
    }
    if status == "paused" {
        return Err("来源任务已暂停，请先继续或结束该任务再移出熔断区".into());
    }
    // Keep compatibility with historical retry children that may still be
    // active, but never create another child. New retries always continue the
    // source task in place.
    let existing = connection
        .query_row(
            "SELECT s.id,s.status FROM sentinel_scans s JOIN sentinel_targets t ON t.scan_id=s.id WHERE s.previous_scan_id=?1 AND t.url=?2 AND s.status IN ('draft','queued','scanning','pausing') ORDER BY s.updated_at DESC LIMIT 1",
            params![source_scan_id, url],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if let Some((scan_id, status)) = existing {
        connection
            .execute("DELETE FROM sentinel_fuse_zone WHERE id=?1", [entry_id])
            .map_err(|error| error.to_string())?;
        let scan = sentinel_scan_by_id(&connection, &scan_id)?;
        drop(connection);
        drop(lifecycle);
        return if status == "draft" {
            confirm_sentinel_scan(app, state, scan_id)
        } else {
            Ok(scan)
        };
    }
    let transaction = rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let previous_workers=claim_scan_quiescence_in(&transaction,&state.db_path,&source_scan_id)?;
    transaction
        .execute("DELETE FROM sentinel_fuse_zone WHERE id=?1", [entry_id])
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,'queued') ON CONFLICT(project_id,scan_id,url) DO UPDATE SET company=excluded.company,status='queued',value_score=0,scan_mode='',routing_reason='',updated_at=datetime('now','localtime')",
            params![project_id, source_scan_id, company, url],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE sentinel_scans SET project_name=?1,status='draft',current_checkpoint='已移出熔断区；正在当前任务中复用前端证据并自动重试',previous_scan_id='',updated_at=datetime('now','localtime') WHERE id=?2",
            params![project_name, source_scan_id],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    drop(connection);
    drop(previous_workers);
    drop(lifecycle);
    confirm_sentinel_scan(app, state, source_scan_id)
}

#[tauri::command]
pub async fn list_sentinel_checkpoints(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<Vec<SentinelCheckpoint>, String> {
    let connection = db::open(&state.db_path)?;
    let mut stmt = connection.prepare("SELECT scan_id,url,stage,raw_json,updated_at FROM sentinel_checkpoints WHERE scan_id=?1 ORDER BY updated_at DESC").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([scan_id], |r| {
            Ok(SentinelCheckpoint {
                scan_id: r.get(0)?,
                url: r.get(1)?,
                stage: r.get(2)?,
                raw_json: r.get(3)?,
                updated_at: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub async fn list_sentinel_findings(
    state: State<'_, AppState>,
    scan_id: String,
    kind: Option<String>,
) -> Result<Vec<SentinelFinding>, String> {
    let database = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let connection = db::open(&database)?;
        sentinel_findings_in(&connection, &scan_id, kind.as_deref())
    }).await.map_err(|_| "findings_reader_failed".to_string())?
}

fn sentinel_findings_in(
    connection: &rusqlite::Connection, scan_id: &str, kind: Option<&str>,
) -> Result<Vec<SentinelFinding>, String> {
    // Reading saved results must never scan source_path or synthesize/replace
    // inventory. Authorized startup owns that work, not a UI read or poll.
    // Existence, deletion, policy and rows belong to one read snapshot.
    let transaction = if connection.is_autocommit() {
        Some(connection.unchecked_transaction().map_err(|_| "findings_snapshot_failed")?)
    } else { None };
    let connection = transaction.as_deref().unwrap_or(connection);
    let policy: Option<Option<String>> = connection.query_row(
        "SELECT c.policy_json FROM sentinel_scans s LEFT JOIN sentinel_scan_contexts c ON c.scan_id=s.id
         WHERE s.id=?1 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=s.id)",
        [scan_id], |row| row.get(0),
    ).optional().map_err(|_| "findings_scope_lookup_failed")?;
    let Some(policy) = policy else { return Ok(Vec::new()) };
    let require_proof = findings_require_proof(policy.as_deref())?;
    let mut stmt = connection.prepare("SELECT id,scan_id,target_url,stage,kind,record_key,title,severity,record_json,updated_at FROM sentinel_findings WHERE scan_id=?1 AND (?2 IS NULL OR kind=?2) ORDER BY stage,kind,id").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![scan_id, kind], |r| {
            Ok(SentinelFinding {
                id: r.get(0)?,
                scan_id: r.get(1)?,
                target_url: r.get(2)?,
                stage: r.get(3)?,
                kind: r.get(4)?,
                record_key: r.get(5)?,
                title: r.get(6)?,
                severity: r.get(7)?,
                record_json: r.get(8)?,
                updated_at: r.get(9)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let rows = if require_proof {
        rows.into_iter()
            .filter(|finding| {
                finding.kind != "vulnerability" || proof_finding_is_visible(&finding.record_json)
            })
            .collect()
    } else {
        rows
    };
    Ok(rows)
}

fn findings_require_proof(policy: Option<&str>) -> Result<bool, String> {
    // Context-free historical tasks predate closure policy. A missing field in
    // a valid object also keeps that compatibility; malformed/unknown explicit
    // policy must not silently broaden visibility.
    let Some(policy) = policy else { return Ok(false) };
    let policy: JsonValue = serde_json::from_str(policy).map_err(|_| "findings_policy_invalid")?;
    let policy = policy.as_object().ok_or("findings_policy_invalid")?;
    match policy.get("closure") {
        None => Ok(false),
        Some(JsonValue::String(value)) if value == "breadth" => Ok(false),
        Some(JsonValue::String(value)) if value == "proof" => Ok(true),
        _ => Err("findings_policy_invalid".into()),
    }
}

fn proof_finding_is_visible(record_json: &str) -> bool {
    let Ok(value) = serde_json::from_str::<JsonValue>(record_json) else {
        return false;
    };
    let control = value
        .get("controlRequestId")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim();
    let test = value
        .get("testRequestId")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim();
    let impact = value
        .get("impact")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim();
    !control.is_empty() && !test.is_empty() && control != test && !impact.is_empty()
}

#[tauri::command]
pub async fn list_sentinel_opportunities(
    state: State<'_, AppState>,
    project_id: Option<i64>,
    scan_id: Option<String>,
    status: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<SentinelOpportunity>, String> {
    let connection = db::open(&state.db_path)?;
    let limit = limit.unwrap_or(500).clamp(1, 5000);
    let query_limit = limit.saturating_mul(4).min(5000);
    let mut statement = connection.prepare(
        "SELECT id,project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,why_json,evidence_json,recommended_action_json,source,record_json,first_seen,last_seen FROM sentinel_opportunities WHERE (?1 IS NULL OR project_id=?1) AND (?2 IS NULL OR scan_id=?2) AND (?3 IS NULL OR status=?3) ORDER BY CASE status WHEN 'ready' THEN 0 WHEN 'in_progress' THEN 1 WHEN 'queued' THEN 2 ELSE 3 END,score DESC,last_seen DESC,id DESC LIMIT ?4",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id, scan_id, status, query_limit], |row| {
            Ok(SentinelOpportunity {
                id: row.get(0)?,
                project_id: row.get(1)?,
                scan_id: row.get(2)?,
                target_url: row.get(3)?,
                opportunity_key: row.get(4)?,
                category: row.get(5)?,
                title: row.get(6)?,
                score: row.get(7)?,
                status: row.get(8)?,
                confidence: row.get(9)?,
                why: json(row.get(10)?),
                evidence: json(row.get(11)?),
                recommended_action: json(row.get(12)?),
                source: row.get(13)?,
                record: json(row.get(14)?),
                first_seen: row.get(15)?,
                last_seen: row.get(16)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut grouped: Vec<SentinelOpportunity> = Vec::new();
    let mut indexes: HashMap<String, usize> = HashMap::new();
    for mut row in rows {
        let method = value_first(&row.record, &["method", "httpMethod"]).to_ascii_uppercase();
        let endpoint = value_first(
            &row.record,
            &["normalizedPath", "endpoint", "url", "path"],
        );
        let key = format!(
            "{}|{}|{}|{}|{}",
            row.scan_id,
            row.target_url,
            row.category.to_ascii_lowercase(),
            method,
            normalized_investigation_path(&endpoint)
        );
        if let Some(index) = indexes.get(&key).copied() {
            let current = &mut grouped[index];
            current.score = current.score.max(row.score);
            merge_opportunity_record(&mut current.record, &row.record);
            merge_json_array(&mut current.why, &row.why);
            merge_json_array(&mut current.evidence, &row.evidence);
            if opportunity_status_rank(&row.status) > opportunity_status_rank(&current.status) {
                current.status = std::mem::take(&mut row.status);
                current.id = row.id;
                current.opportunity_key = std::mem::take(&mut row.opportunity_key);
            }
        } else {
            indexes.insert(key, grouped.len());
            grouped.push(row);
        }
    }
    grouped.truncate(limit as usize);
    Ok(grouped)
}

fn opportunity_status_rank(status: &str) -> u8 {
    match status {
        "validated" => 7,
        "in_progress" => 6,
        "ready" => 5,
        "queued" => 4,
        "needs_more_evidence" => 3,
        "blocked_by_authorization" => 2,
        _ => 1,
    }
}

fn merge_json_array(target: &mut JsonValue, source: &JsonValue) {
    let mut values = target.as_array().cloned().unwrap_or_default();
    let mut seen = values
        .iter()
        .map(JsonValue::to_string)
        .collect::<HashSet<_>>();
    for item in source.as_array().into_iter().flatten() {
        if seen.insert(item.to_string()) {
            values.push(item.clone());
        }
    }
    *target = JsonValue::Array(values);
}

fn merge_opportunity_record(target: &mut JsonValue, source: &JsonValue) {
    let Some(target_object) = target.as_object_mut() else {
        return;
    };
    for field in ["identityKeys", "identityScopeKeys"] {
        let mut merged = target_object
            .get(field)
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let mut seen = merged
            .iter()
            .map(JsonValue::to_string)
            .collect::<HashSet<_>>();
        for value in source
            .get(field)
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            if seen.insert(value.to_string()) {
                merged.push(value.clone());
            }
        }
        target_object.insert(field.into(), JsonValue::Array(merged));
    }
    let mut runs = target_object
        .get("identityRuns")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    for incoming in source
        .get("identityRuns")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let identity = value_first(incoming, &["identityKey"]);
        if let Some(existing) = runs
            .iter_mut()
            .find(|value| value_first(value, &["identityKey"]) == identity)
        {
            let existing_observed =
                existing.get("observed").and_then(JsonValue::as_bool) == Some(true);
            let incoming_observed =
                incoming.get("observed").and_then(JsonValue::as_bool) == Some(true);
            if incoming_observed && !existing_observed {
                *existing = incoming.clone();
            }
        } else {
            runs.push(incoming.clone());
        }
    }
    target_object.insert("identityRuns".into(), JsonValue::Array(runs));
}

#[tauri::command]
pub fn update_sentinel_opportunity_status(
    state: State<AppState>,
    opportunity_id: i64,
    status: String,
) -> Result<(), String> {
    let status = status.trim().to_ascii_lowercase();
    if ![
        "queued",
        "ready",
        "in_progress",
        "validated",
        "dismissed",
        "exhausted",
        "needs_more_evidence",
        "blocked_by_authorization",
        "closed",
    ]
    .contains(&status.as_str())
    {
        return Err("不支持的机会状态".into());
    }
    let connection = db::open(&state.db_path)?;
    let (record, scan_id, target_url, category) = connection
        .query_row(
            "SELECT record_json,scan_id,target_url,category FROM sentinel_opportunities WHERE id=?1",
            [opportunity_id],
            |row| {
                Ok((
                    json(row.get::<_, String>(0)?),
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(|_| "机会记录不存在".to_string())?;
    if matches!(status.as_str(), "ready" | "in_progress") {
        let (eligible, reason) = opportunity_agent_readiness(&record);
        if !eligible {
            return Err(format!(
                "该线索尚缺少可复现请求契约或新鲜响应，不能进入自动验证队列：{reason}"
            ));
        }
    }
    let method = value_first(&record, &["method", "httpMethod"]).to_ascii_uppercase();
    let normalized_path = value_first(
        &record,
        &["normalizedPath", "endpoint", "url", "path"],
    );
    let normalized_path = normalized_investigation_path(&normalized_path).to_ascii_lowercase();
    let changed = connection
        .execute(
            "UPDATE sentinel_opportunities SET status=?1,last_seen=datetime('now','localtime') WHERE scan_id=?2 AND target_url=?3 AND lower(category)=lower(?4) AND upper(COALESCE(json_extract(record_json,'$.method'),''))=?5 AND lower(COALESCE(json_extract(record_json,'$.normalizedPath'),''))=?6",
            params![status, scan_id, target_url, category, method, normalized_path],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("机会记录不存在".into());
    }
    // Keep terminal/manual opportunity actions visible in the investigation
    // graph and Action Center. Updating only the opportunity row made a card
    // appear to disappear without leaving an auditable next step.
    if matches!(status.as_str(), "validated" | "dismissed" | "exhausted" | "needs_more_evidence" | "blocked_by_authorization") {
        connection.execute(
            "INSERT INTO investigation_actions(project_id,scan_id,target_url,action_key,state_key,action_type,label,outcome,value_score,protocol_json) SELECT project_id,scan_id,target_url,?1,'opportunity-status','opportunity_follow_up',?2,?3,?4,?5 FROM sentinel_opportunities WHERE id=?6 ON CONFLICT(scan_id,target_url,action_key) DO UPDATE SET label=excluded.label,outcome=excluded.outcome,value_score=excluded.value_score,protocol_json=excluded.protocol_json,updated_at=datetime('now','localtime')",
            params![
                format!("opportunity-status:{}", opportunity_id),
                match status.as_str() {
                    "validated" => "验证已完成 · 查看结论与证据",
                    "dismissed" => "已排除 · 保留排除依据",
                    "exhausted" => "无新增证据 · 停止重复验证",
                    "needs_more_evidence" => "需要更多证据 · 继续同请求核查",
                    "blocked_by_authorization" => "旧授权停点 · 已交由自动策略收口",
                    _ => "机会状态已更新",
                },
                status.clone(),
                match status.as_str() { "validated" => 95, "needs_more_evidence" => 70, "blocked_by_authorization" => 40, _ => 20 },
                serde_json::json!({"opportunityId": opportunity_id, "status": status}).to_string(),
                opportunity_id,
            ],
        ).map_err(|error| error.to_string())?;
        connection.execute(
            "UPDATE sentinel_opportunities SET record_json=json_set(CASE WHEN json_valid(record_json) THEN record_json ELSE '{}' END,'$.lastOpportunityStatus',?1,'$.lastOpportunityStatusAt',datetime('now','localtime')) WHERE id=?2",
            params![status, opportunity_id],
        ).map_err(|error| error.to_string())?;
    }
    Ok(())
}
