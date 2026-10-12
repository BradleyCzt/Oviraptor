// Admission, preparation and publication of an ordinary Web attempt share one
// IMMEDIATE transaction. No worker may be dispatched from this module.
include!("native_web_resume_finance.rs");
#[derive(Clone, Copy)]
enum WebStartMode { Confirm, Resume, Retry }

fn claim_scan_control(db_path: &Path, scan_id: &str) -> Result<NativeInvocationOwner, String> {
    claim_native_invocation(db_path, scan_id, 0, "scan_control", "lifecycle")
}

fn validate_web_start_id(scan_id: &str) -> Result<(), String> {
    if scan_id.is_empty() || scan_id.len() > 160
        || !scan_id.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    { return Err("invalid_web_start_scan_id".into()); }
    Ok(())
}

// A known pre-commit failure removes only this exclusively created directory's
// known files. Unknown commit outcomes preserve artifacts for reconciliation.
struct WebStartFiles {
    path: PathBuf,
    scan_id: String,
    preserve: bool,
    #[cfg(unix)]
    identity: (u64, u64),
}

impl WebStartFiles {
    fn allocate(root: &Path, scan_id: &str, attempt: u32) -> Result<Self, String> {
        let path = root.join(format!("attempt-{attempt:04}"));
        fs::create_dir(&path).map_err(|error| format!("web_start_directory_claim:{error}"))?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o700)) {
                let _ = fs::remove_dir(&path);
                return Err(error.to_string());
            }
        }
        if let Err(error) = fs::write(path.join(".oviraptor-scan-id"), scan_id) {
            let _ = fs::remove_dir(&path);
            return Err(error.to_string());
        }
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            (metadata.dev(), metadata.ino())
        };
        Ok(Self { path, scan_id: scan_id.into(), preserve: false, #[cfg(unix)] identity })
    }
}

impl Drop for WebStartFiles {
    fn drop(&mut self) {
        if self.preserve { return; }
        #[cfg(unix)] {
            use std::os::unix::fs::MetadataExt;
            if !fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(),m.ino()) == self.identity) { return; }
        }
        if !fs::symlink_metadata(&self.path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
            || fs::read_to_string(self.path.join(".oviraptor-scan-id")).ok().as_deref() != Some(&self.scan_id)
        { return; }
        // Never recursively remove a directory, historical attempt, arbitrary
        // task_path or unexpected worker output during startup compensation.
        for name in ["task.json", "auth-sessions.json", "auth-session.json", "targets.json", "targets.txt",
            "agent-instruction.md", "model-prompt-audit.json", ".web-dispatch-key", ".oviraptor-scan-id"] {
            let path = self.path.join(name);
            if fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && !m.file_type().is_symlink()) {
                let _ = fs::remove_file(path);
            }
        }
        let _ = fs::remove_dir(&self.path);
    }
}

fn web_start_root(app_data: &Path, scan_id: &str) -> Result<PathBuf, String> {
    validate_web_start_id(scan_id)?;
    let base = fs::canonicalize(app_data).map_err(|e| e.to_string())?;
    let mut path = base;
    for segment in [SCAN_WORK_DIRECTORY, scan_id] {
        path.push(segment);
        match fs::create_dir(&path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {},
            Err(error) => return Err(error.to_string()),
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("web_start_root_not_regular".into());
        }
    }
    Ok(path)
}

fn web_start_attempt_number(connection: &rusqlite::Connection, scan_id: &str, root: &Path, previous: i64) -> Result<u32, String> {
    let ledger_max: i64 = connection.query_row(
        "SELECT COALESCE(MAX(attempt_number),0) FROM sentinel_scan_attempts WHERE scan_id=?1",
        [scan_id], |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    let mut maximum = u32::try_from(previous.max(ledger_max)).map_err(|_| "web_start_attempt_invalid")?;
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if let Some(number) = entry.file_name().to_string_lossy().strip_prefix("attempt-") {
            let number = number.parse::<u32>().map_err(|_| "web_start_attempt_directory_invalid")?;
            maximum = maximum.max(number);
        }
    }
    for name in ["url-pipeline", "batches", "oviraptor-runner.log", "llm-hook.jsonl", "targets.json", "targets.txt"] {
        match fs::symlink_metadata(root.join(name)) {
            Ok(_) => maximum = maximum.max(1),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => return Err(error.to_string()),
        }
    }
    maximum.checked_add(1).ok_or_else(|| "web_start_attempt_exhausted".into())
}

type WebStartTarget = (i64, i64, String, String, String, i64);
fn web_start_target_snapshot(connection: &rusqlite::Connection, scan_id: &str) -> Result<Vec<WebStartTarget>, String> {
    let mut statement = connection.prepare("SELECT id,project_id,company,url,status,last_attempt_number FROM sentinel_targets WHERE scan_id=?1 ORDER BY id").map_err(|e| e.to_string())?;
    let rows = statement.query_map([scan_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
        .map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    Ok(rows)
}

struct WebStartup {
    scan_id: String,
    project_id: i64,
    project_name: String,
    previous_attempt: i64,
    attempt: u32,
    execution_mode: String,
    targets: Vec<(String,String)>,
    target_snapshot: Vec<WebStartTarget>,
    plan: ScanBackendPlan,
    files: WebStartFiles,
}

// New attempts always use Native. Continuations must carry the exact parent's
// frozen decision; malformed/missing/retired parent plans are not fresh starts.
fn web_start_backend_plan(connection: &rusqlite::Connection, scan_id: &str, previous: i64, attempt: u32, mode: &str, urls: &[String]) -> Result<ScanBackendPlan,String> {
    if !agent_native_eligible("web", "", urls) { return Err("unsupported_web_start_capability".into()); }
    let parent = if mode == "resume" {
        let raw: String = connection.query_row("SELECT backend_plan_json FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
            params![scan_id,previous], |r| r.get(0)).map_err(|e| format!("resume_backend_plan_missing:{e}"))?;
        let value: JsonValue = serde_json::from_str(&raw).map_err(|e| format!("resume_backend_plan_corrupt:{e}"))?;
        let plan = ScanBackendPlan::from_json(&value).ok_or("resume_backend_plan_corrupt")?;
        let source_count = value.get("targets").and_then(JsonValue::as_array).map(Vec::len);
        let unique = plan.targets.iter().map(|t| &t.url).collect::<std::collections::HashSet<_>>();
        if plan.scan_id != scan_id || value.get("attemptNumber").and_then(JsonValue::as_i64) != Some(previous)
            || plan.attempt_number != previous || source_count != Some(plan.targets.len())
            || unique.len() != plan.targets.len() || plan.targets.iter().any(|t| t.url.trim().is_empty())
        { return Err("resume_backend_plan_mismatch".into()); }
        Some(plan)
    } else { None };
    let mut targets = Vec::new();
    for url in urls {
        if targets.iter().any(|t: &ScanTargetBackend| t.url == *url) { continue; }
        let target = match &parent {
            Some(parent) => parent.backend_of(url).ok_or("resume_backend_target_missing")?,
            None => ScanTargetBackend { url: url.clone(), backend: AgentBackendKind::Native,
                selection_reason: format!("attempt {attempt} 使用唯一可执行的 native 后端") },
        };
        targets.push(target);
    }
    let mut plan = ScanBackendPlan { scan_id: scan_id.into(), attempt_number: attempt.into(), targets,
        requires_node: false, requires_browser: false };
    plan.refresh_requirements(true);
    prepare_scan_dependencies(&plan)?;
    Ok(plan)
}

fn prepare_web_startup_in(connection: &rusqlite::Connection, app_data: &Path, scan_id: &str, mode: WebStartMode) -> Result<WebStartup,String> {
    if connection.is_autocommit() { return Err("web_start_requires_transaction".into()); }
    validate_web_start_id(scan_id)?;
    let (project_id, project_name, status, scan_type, previous_attempt): (i64,String,String,String,i64) = connection.query_row(
        "SELECT s.project_id,s.project_name,s.status,s.scan_type,s.attempt_count FROM sentinel_scans s JOIN projects p ON p.id=s.project_id AND p.status='active' WHERE s.id=?1 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=s.id)",
        [scan_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).map_err(|e| format!("任务不存在、已删除或工作空间已归档：{e}"))?;
    if scan_type != "web" { return Err("web_start_wrong_scan_type".into()); }
    // A staged recovery scope has no execution consumer yet. Never let ordinary
    // confirm/resume/retry substitute current defaults for its signed contract.
    if connection.query_row("SELECT EXISTS(SELECT 1 FROM main.native_protection_recovery_drafts WHERE scan_id=?1)",
        [scan_id], |r|r.get::<_,bool>(0)).map_err(|e|e.to_string())? {
        return Err("protection_recovery_requires_dedicated_fresh_birth".into());
    }
    if verified_administrative_closure(connection,scan_id)?.is_some() {
        return Err("administrative_closure_requires_independent_task".into());
    }
    require_web_start_financial_history_in(connection,scan_id)?;
    require_web_start_mode_on(connection,scan_id,previous_attempt,
        matches!(mode,WebStartMode::Resume) || (matches!(mode,WebStartMode::Retry) && retry_only_incomplete_targets(&status)))?;
    match mode {
        WebStartMode::Confirm if status != "draft" => return Err(format!("任务当前状态为 {status}，不能重复确认")),
        WebStartMode::Resume => {
            if !matches!(status.as_str(), "paused" | "partial") { return Err(format!("任务当前状态为 {status}，不能恢复")); }
            let sealed: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND status='legacy_backend_removed')", [scan_id], |r| r.get(0)).map_err(|e| e.to_string())?;
            if sealed { return Err("legacy_backend_removed: 历史执行不能恢复，请发起全新 Native 执行".into()); }
            let changed = connection.execute("UPDATE sentinel_scans SET status='draft' WHERE id=?1 AND status=?2 AND attempt_count=?3", params![scan_id,status,previous_attempt]).map_err(|e| e.to_string())?;
            if changed != 1 { return Err("web_resume_state_changed".into()); }
            connection.execute("INSERT INTO app_settings(key,value) VALUES(?1,'resume') ON CONFLICT(key) DO UPDATE SET value='resume'", [format!("sentinel-next-attempt-mode:{scan_id}")]).map_err(|e| e.to_string())?;
        },
        WebStartMode::Retry => { prepare_web_scan_retry_in(connection, scan_id, &status)?; },
        WebStartMode::Confirm => {},
    }
    let execution_mode = connection.query_row("SELECT value FROM app_settings WHERE key=?1", [format!("sentinel-next-attempt-mode:{scan_id}")], |r| r.get::<_,String>(0))
        .optional().map_err(|e| e.to_string())?.unwrap_or_else(|| "initial".into());
    if !matches!(execution_mode.as_str(), "initial" | "fresh" | "resume") { return Err("web_start_invalid_mode".into()); }
    if matches!(mode,WebStartMode::Resume) && execution_mode != "resume" { return Err("web_resume_mode_not_persisted".into()); }
    if matches!(mode,WebStartMode::Retry) && execution_mode != if retry_only_incomplete_targets(&status) { "resume" } else { "fresh" } {
        return Err("web_retry_mode_not_persisted".into());
    }
    if execution_mode == "resume" {
        let sealed: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND status='legacy_backend_removed')", [scan_id], |r| r.get(0)).map_err(|e| e.to_string())?;
        if sealed { return Err("legacy_backend_removed: 历史执行不能恢复，请发起全新 Native 执行".into()); }
    }
    // The authorization preflight predicts the upcoming directory. It MUST run
    // before allocation, otherwise its attempt calculation advances twice.
    let followup_attempt = gap_followup_control_preflight(connection, app_data, scan_id)?;
    connection.execute("UPDATE sentinel_targets SET status='fuse_excluded',routing_reason='该 URL 位于熔断区；移出熔断区后才会恢复自动扫描',updated_at=datetime('now','localtime') WHERE scan_id=?1 AND EXISTS(SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=sentinel_targets.project_id AND f.normalized_url=lower(rtrim(trim(sentinel_targets.url),'/')))", [scan_id]).map_err(|e| e.to_string())?;
    let missed: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_targets t JOIN sentinel_fuse_zone f ON f.project_id=t.project_id AND f.normalized_url=lower(rtrim(trim(t.url),'/')) WHERE t.scan_id=?1 AND f.archived=0 AND t.status<>'fuse_excluded')", [scan_id], |r| r.get(0)).map_err(|e| e.to_string())?;
    if missed { return Err("web_start_fuse_not_persisted".into()); }
    let targets = {
        let mut statement = connection.prepare(SENTINEL_RESUME_TARGETS_SQL).map_err(|e| e.to_string())?;
        let rows = statement.query_map([scan_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))
            .map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
        rows
    };
    if targets.is_empty() { return Err("任务没有可扫描目标；URL 可能都在熔断区".into()); }
    let root = web_start_root(app_data, scan_id)?;
    let attempt = web_start_attempt_number(connection, scan_id, &root, previous_attempt)?;
    if followup_attempt.is_some_and(|expected| expected != i64::from(attempt)) { return Err("followup_control_attempt_changed_reconfigure_controls".into()); }
    let urls = targets.iter().map(|(_,url)| url.clone()).collect::<Vec<_>>();
    let plan = web_start_backend_plan(connection,scan_id,previous_attempt,attempt,&execution_mode,&urls)?;
    let target_snapshot = web_start_target_snapshot(connection,scan_id)?;
    if target_snapshot.iter().any(|target| target.1 != project_id) { return Err("web_start_target_project_mismatch".into()); }
    let files = WebStartFiles::allocate(&root,scan_id,attempt)?;
    Ok(WebStartup { scan_id:scan_id.into(),project_id,project_name,previous_attempt,attempt,execution_mode,targets,target_snapshot,plan,files })
}

fn persist_web_startup_in(connection: &rusqlite::Connection, startup: &WebStartup, checkpoint: &str, skills: &str, policy: &JsonValue) -> Result<SentinelScan,String> {
    if connection.is_autocommit() { return Err("web_start_requires_transaction".into()); }
    let scan_id = &startup.scan_id;
    let attempt = i64::from(startup.attempt);
    let work_dir = startup.files.path.to_string_lossy();
    let task_path = startup.files.path.join("task.json");
    let task_path = task_path.to_string_lossy();
    let changed = connection.execute("UPDATE sentinel_scans SET status='scanning',current_checkpoint=?1,task_path=?2,skill_names=?3,attempt_count=?4,updated_at=datetime('now','localtime') WHERE id=?5 AND status='draft' AND attempt_count=?6 AND scan_type='web' AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?5) AND EXISTS(SELECT 1 FROM projects WHERE id=sentinel_scans.project_id AND status='active')",
        params![checkpoint,task_path,skills,attempt,scan_id,startup.previous_attempt]).map_err(|e| e.to_string())?;
    if changed != 1 { return Err("web_start_state_changed".into()); }
    let mut expected_targets = startup.target_snapshot.clone();
    for target in &mut expected_targets {
        if startup.targets.iter().any(|(company,url)| company == &target.2 && url == &target.3)
            && !matches!(target.4.as_str(), "completed"|"completed_with_gaps"|"partial"|"recon_only"|"manual_review"|"limited"|"protected_stop"|"failed"|"fuse_excluded"|"resume_incompatible"|"persistence_failure") {
            let changed = connection.execute("UPDATE sentinel_targets SET last_attempt_number=?1 WHERE id=?2 AND scan_id=?3", params![attempt,target.0,scan_id]).map_err(|e| e.to_string())?;
            if changed != 1 { return Err("web_start_target_not_persisted".into()); }
            target.5 = attempt;
        }
    }
    let backend = startup.plan.as_json().to_string();
    let stage = sentinel_attempt_stage("scanning",checkpoint);
    let changed = connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status,stage,checkpoint,work_dir,llm_requests_start,input_tokens_start,output_tokens_start,cached_tokens_start,total_tokens_start,backend_plan_json) SELECT id,?2,?3,'scanning',?4,?5,?6,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,?7 FROM sentinel_scans WHERE id=?1",
        params![scan_id,attempt,startup.execution_mode,stage,checkpoint,work_dir,backend]).map_err(|e| e.to_string())?;
    if changed != 1 { return Err("web_start_ledger_not_persisted".into()); }
    connection.execute("DELETE FROM app_settings WHERE key=?1", [format!("sentinel-next-attempt-mode:{scan_id}")]).map_err(|e| e.to_string())?;
    prepare_current_attempt_surface(connection,scan_id,attempt)?;
    // A new attempt inherits the explicit scheduling contract, rather than
    // silently returning a homogeneous task to the legacy role pipeline.
    if attempt>1 && crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(connection,scan_id,1)? {
        let text:String=connection.query_row("SELECT plan_json FROM native_scan_plans WHERE scan_id=?1 AND attempt_number=1",[scan_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        let mut frozen:JsonValue=serde_json::from_str(&text).map_err(|e|e.to_string())?;
        frozen["attemptNumber"]=json!(attempt);
        let text=frozen.to_string();
        connection.execute("INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash) VALUES(?1,?2,'web','native',?3,?4)",params![scan_id,attempt,text,crate::agent_runtime::store::stable_hash(&text)]).map_err(|e|e.to_string())?;
    }
    connection.execute("INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json) VALUES(?1,'','scan_backend_plan',?2) ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=datetime('now','localtime')", params![scan_id,backend]).map_err(|e| e.to_string())?;
    register_native_branches_in(connection,scan_id,attempt,&["web"])?;
    // Validate persisted rows AFTER all writes/triggers, not just row counts.
    let valid: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN projects p ON p.id=s.project_id WHERE s.id=?1 AND s.project_id=?2 AND p.status='active' AND s.status='scanning' AND s.scan_type='web' AND s.attempt_count=?3 AND s.task_path=?4 AND s.skill_names=?5 AND s.current_checkpoint=?6) AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        params![scan_id,startup.project_id,attempt,task_path,skills,checkpoint], |r| r.get(0)).map_err(|e| e.to_string())?;
    let ledger_valid: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts a JOIN sentinel_scans s ON s.id=a.scan_id WHERE a.scan_id=?1 AND a.attempt_number=?2 AND a.execution_mode=?3 AND a.status='scanning' AND a.stage=?4 AND a.checkpoint=?5 AND a.work_dir=?6 AND a.backend_plan_json=?7 AND a.finished_at='' AND a.stop_reason='' AND a.llm_requests_start=s.llm_requests AND a.input_tokens_start=s.input_tokens AND a.output_tokens_start=s.output_tokens AND a.cached_tokens_start=s.cached_tokens AND a.total_tokens_start=s.total_tokens)",
        params![scan_id,attempt,startup.execution_mode,stage,checkpoint,work_dir,backend], |r| r.get(0)).map_err(|e| e.to_string())?;
    let projection_valid: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_checkpoints WHERE scan_id=?1 AND url='' AND stage='scan_backend_plan' AND raw_json=?2) AND EXISTS(SELECT 1 FROM app_settings WHERE key=?3 AND value=?4) AND NOT EXISTS(SELECT 1 FROM app_settings WHERE key=?5)",
        params![scan_id,backend,format!("agent-current-attempt:{scan_id}"),attempt.to_string(),format!("sentinel-next-attempt-mode:{scan_id}")], |r| r.get(0)).map_err(|e| e.to_string())?;
    let stored_policy: String = connection.query_row("SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1", [scan_id], |r| r.get(0)).map_err(|e| e.to_string())?;
    if !valid || !ledger_valid || !projection_valid || web_start_target_snapshot(connection,scan_id)? != expected_targets
        || serde_json::from_str::<JsonValue>(&stored_policy).map_err(|e| e.to_string())? != *policy {
        return Err("web_start_persistence_postcondition".into());
    }
    sentinel_scan_by_id(connection,scan_id)
}
