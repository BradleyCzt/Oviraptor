#[tauri::command]
pub fn create_sentinel_scan(
    state: State<AppState>,
    project_id: i64,
    asset_ids: Vec<i64>,
    scan_mode: Option<String>,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    create_sentinel_asset_scan_in(&connection, project_id, asset_ids, scan_mode)
}

fn create_sentinel_asset_scan_in(
    connection: &rusqlite::Connection,
    project_id: i64,
    mut asset_ids: Vec<i64>,
    scan_mode: Option<String>,
) -> Result<SentinelScan, String> {
    // Project membership, fuse filtering, policy and the complete target set
    // belong to one draft snapshot. A failed late insert must leave no draft.
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定资产任务草稿：{error}"))?;
    let window = crate::agent_runtime::web_mode::draft::DraftCreationWindow::begin(&transaction)?;
    let connection = &transaction;
    let project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id=?1 AND status='active'",
            [project_id],
            |r| r.get(0),
        )
        .map_err(|_| "项目不存在或已归档；恢复工作空间后才能创建新任务".to_string())?;
    asset_ids.sort_unstable();
    asset_ids.dedup();
    let ids_json = serde_json::to_string(&asset_ids).map_err(|e| e.to_string())?;
    let mut statement = connection.prepare("SELECT a.id,a.company,COALESCE(NULLIF(a.link,''),a.host) FROM assets a JOIN project_assets pa ON pa.asset_id=a.id WHERE pa.project_id=?1 AND pa.is_deleted=0 AND a.id IN (SELECT value FROM json_each(?2)) AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.project_id=pa.project_id AND f.normalized_url=lower(rtrim(trim(COALESCE(NULLIF(a.link,''),a.host)),'/'))) ORDER BY a.id").map_err(|e| e.to_string())?;
    let mut targets = statement
        .query_map(params![project_id, ids_json], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(statement);
    if targets.is_empty() {
        return Err("没有可发送的资产；所选 URL 可能都在熔断区".into());
    }
    let excluded_count = asset_ids.len().saturating_sub(targets.len());
    // Duplicate assets may refer to the exact same URL. Select the first asset
    // deterministically instead of using INSERT OR IGNORE to hide write errors.
    let original_count = targets.len();
    let mut urls = std::collections::HashSet::new();
    targets.retain(|(_, _, url)| urls.insert(url.clone()));
    let duplicate_count = original_count - targets.len();
    let mut checkpoint = format!("待确认：已加入 {} 个 URL", targets.len());
    if excluded_count > 0 {
        checkpoint.push_str(&format!(
            "；{excluded_count} 个所选资产因熔断、已移出或不属于当前工作空间而排除"
        ));
    }
    if duplicate_count > 0 {
        checkpoint.push_str(&format!("；合并 {duplicate_count} 个重复 URL"));
    }
    let scan_id = fresh_web_draft_id();
    let scan_mode = normalized_web_scan_mode(scan_mode.as_deref());
    let policy = build_web_investigation_policy(
        Some(scan_mode),
        None,
        Vec::new(),
        &[],
        "",
        "asset-workspace",
        Some("breadth"),
    )?;
    let inserted = connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name) VALUES(?1,?2,?3,'draft',?4,'','web',?3)", params![scan_id,project_id,project_name,checkpoint]).map_err(|e| e.to_string())?;
    if inserted != 1 {
        return Err("资产任务草稿写入未完成".into());
    }
    let inserted = connection.execute(
        "INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2)",
        params![scan_id, policy.to_string()],
    ).map_err(|error| error.to_string())?;
    if inserted != 1 {
        return Err("资产任务策略写入未完成".into());
    }
    for (asset_id, company, url) in &targets {
        let inserted = connection
            .execute(
                "INSERT INTO sentinel_targets(project_id,scan_id,asset_id,company,url) VALUES(?1,?2,?3,?4,?5)",
                params![project_id, scan_id, asset_id, company, url],
            )
            .map_err(|e| e.to_string())?;
        if inserted != 1 {
            return Err("资产任务目标写入未完成".into());
        }
    }
    // Check persisted values as well as affected rows: an AFTER trigger can
    // remove or change a row while the INSERT still reports success.
    let valid: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans s JOIN sentinel_scan_contexts c ON c.scan_id=s.id WHERE s.id=?1 AND s.project_id=?2 AND s.project_name=?3 AND s.task_name=?3 AND s.status='draft' AND s.current_checkpoint=?4 AND s.task_path='' AND s.scan_type='web' AND s.attempt_count=0 AND c.environment='internal' AND c.policy_json=?5)",
        params![scan_id, project_id, project_name, checkpoint, policy.to_string()], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_targets WHERE scan_id=?1",
            [&scan_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !valid || count != targets.len() as i64 {
        return Err("资产任务草稿持久化校验失败".into());
    }
    for (asset_id, company, url) in &targets {
        let valid: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_targets WHERE scan_id=?1 AND project_id=?2 AND asset_id=?3 AND company=?4 AND url=?5 AND status='queued' AND last_attempt_number=0)",
            params![scan_id, project_id, asset_id, company, url], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        if !valid {
            return Err("资产任务目标持久化校验失败".into());
        }
    }
    let result = sentinel_scan_by_id(connection, &scan_id)?;
    crate::agent_runtime::web_mode::draft::register_new(
        window,
        &scan_id,
        crate::agent_runtime::web_mode::WebMode::Multi,
    )?;
    transaction
        .commit()
        .map_err(|error| format!("资产任务草稿提交失败：{error}"))?;
    Ok(result)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_sentinel_url_scan(
    state: State<AppState>,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    max_budget_usd: Option<f64>,
    auth_session_id: Option<String>,
    auth_session_ids: Option<Vec<String>>,
    auth_session_scope_id: Option<String>,
    skill_ids: Option<Vec<i64>>,
    instruction: Option<String>,
    closure: Option<String>,
    orchestration_mode: Option<String>,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    create_sentinel_url_scan_with_mode_in(
        &connection,
        project_id,
        task_name,
        urls,
        scan_mode,
        max_budget_usd,
        auth_session_id,
        auth_session_ids,
        auth_session_scope_id,
        skill_ids,
        instruction,
        closure,
        orchestration_mode,
    )
}

/// Task-external approved inventory selection for a genuinely new Multi draft.
/// Empty selection preserves the old creation path without loading trust files.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_sentinel_url_scan_with_tool_selection(
    app: tauri::AppHandle,
    state: State<AppState>,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    max_budget_usd: Option<f64>,
    auth_session_id: Option<String>,
    auth_session_ids: Option<Vec<String>>,
    auth_session_scope_id: Option<String>,
    skill_ids: Option<Vec<i64>>,
    instruction: Option<String>,
    closure: Option<String>,
    orchestration_mode: Option<String>,
    approved_ids: Option<Vec<String>>,
) -> Result<SentinelScan, String> {
    let ids = approved_ids.unwrap_or_default();
    let connection = db::open(&state.db_path)?;
    if ids.is_empty() {
        return create_sentinel_url_scan_with_mode_in(
            &connection,
            project_id,
            task_name,
            urls,
            scan_mode,
            max_budget_usd,
            auth_session_id,
            auth_session_ids,
            auth_session_scope_id,
            skill_ids,
            instruction,
            closure,
            orchestration_mode,
        );
    }
    let configuration = crate::tool_supply::ToolAdministratorConfiguration::from_app(&app)?;
    create_sentinel_url_scan_with_tool_selection_in(
        &connection,
        &configuration,
        project_id,
        task_name,
        urls,
        scan_mode,
        max_budget_usd,
        auth_session_id,
        auth_session_ids,
        auth_session_scope_id,
        skill_ids,
        instruction,
        closure,
        orchestration_mode,
        ids,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_sentinel_url_scan_with_tool_selection_in(
    connection: &rusqlite::Connection,
    configuration: &crate::tool_supply::ToolAdministratorConfiguration,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    max_budget_usd: Option<f64>,
    auth_session_id: Option<String>,
    auth_session_ids: Option<Vec<String>>,
    auth_session_scope_id: Option<String>,
    skill_ids: Option<Vec<i64>>,
    instruction: Option<String>,
    closure: Option<String>,
    orchestration_mode: Option<String>,
    approved_ids: Vec<String>,
) -> Result<SentinelScan, String> {
    create_sentinel_url_scan_with_tool_selection_execution_in(connection,configuration,project_id,task_name,urls,scan_mode,max_budget_usd,auth_session_id,auth_session_ids,auth_session_scope_id,skill_ids,instruction,closure,orchestration_mode,approved_ids,false)
}

#[allow(clippy::too_many_arguments)]
fn create_sentinel_url_scan_with_tool_selection_execution_in(
    connection:&rusqlite::Connection,configuration:&crate::tool_supply::ToolAdministratorConfiguration,
    project_id:i64,task_name:String,urls:Vec<String>,scan_mode:Option<String>,max_budget_usd:Option<f64>,
    auth_session_id:Option<String>,auth_session_ids:Option<Vec<String>>,auth_session_scope_id:Option<String>,
    skill_ids:Option<Vec<i64>>,instruction:Option<String>,closure:Option<String>,orchestration_mode:Option<String>,
    mut approved_ids:Vec<String>,execution:bool,
) -> Result<SentinelScan, String> {
    if approved_ids.is_empty()
        || approved_ids.len() > 16
        || approved_ids
            .iter()
            .any(|id| !Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == *id))
    {
        return Err("tool_selection_approved_ids_invalid".into());
    }
    approved_ids.sort();
    if approved_ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("tool_selection_approved_ids_invalid".into());
    }
    let mode = crate::agent_runtime::web_mode::WebMode::new_input(orchestration_mode.as_deref())?;
    if orchestration_mode.is_none() || mode != crate::agent_runtime::web_mode::WebMode::Multi {
        return Err("tool_selection_explicit_multi_required".into());
    }
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定 Web 工具选择草稿：{error}"))?;
    let window = crate::agent_runtime::web_mode::draft::DraftCreationWindow::begin(&transaction)?;
    let scan = create_sentinel_url_draft_rows(
        &transaction,
        project_id,
        task_name,
        urls,
        scan_mode,
        max_budget_usd,
        auth_session_id,
        auth_session_ids,
        auth_session_scope_id,
        skill_ids,
        instruction,
        closure,
        None,
    )?;
    if execution{freeze_homogeneous_metadata_in(&transaction,&scan.id,true)?;}
    let fresh = crate::agent_runtime::web_mode::draft::register_new(window, &scan.id, mode)?;
    let targets = {
        let mut statement = transaction
            .prepare("SELECT url FROM main.sentinel_targets WHERE scan_id=?1 ORDER BY url")
            .map_err(|e| e.to_string())?;
        let rows = statement
            .query_map([&scan.id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?
    };
    let request =
        crate::tool_supply::selection::DeclaredSelectionRequest::parse(&serde_json::json!({
            "schemaVersion":1,"requestId":Uuid::new_v4().to_string(),
            "scope":{"scanId":scan.id,"attemptNumber":1,"targetUrls":targets},
            "selection":{"schemaVersion":1,"approvalIds":approved_ids},
        }))?;
    crate::tool_supply::selection::stage_for_new_draft(&fresh, configuration, Some(&request))?
        .ok_or("tool_selection_stage_insert_unconfirmed")?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交 Web 工具选择草稿：{error}"))?;
    Ok(scan)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn create_sentinel_url_scan_in(
    connection: &rusqlite::Connection,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    max_budget_usd: Option<f64>,
    auth_session_id: Option<String>,
    auth_session_ids: Option<Vec<String>>,
    auth_session_scope_id: Option<String>,
    skill_ids: Option<Vec<i64>>,
    instruction: Option<String>,
    closure: Option<String>,
) -> Result<SentinelScan, String> {
    create_sentinel_url_scan_with_mode_in(
        connection,
        project_id,
        task_name,
        urls,
        scan_mode,
        max_budget_usd,
        auth_session_id,
        auth_session_ids,
        auth_session_scope_id,
        skill_ids,
        instruction,
        closure,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_sentinel_url_scan_with_mode_in(
    connection: &rusqlite::Connection,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    max_budget_usd: Option<f64>,
    auth_session_id: Option<String>,
    auth_session_ids: Option<Vec<String>>,
    auth_session_scope_id: Option<String>,
    skill_ids: Option<Vec<i64>>,
    instruction: Option<String>,
    closure: Option<String>,
    orchestration_mode: Option<String>,
) -> Result<SentinelScan, String> {
    create_sentinel_url_scan_scheduling_in(connection, project_id, task_name, urls, scan_mode,
        max_budget_usd, auth_session_id, auth_session_ids, auth_session_scope_id, skill_ids,
        instruction, closure, orchestration_mode, false)
}

/// An explicit product opt-in, frozen before the draft is signed. Legacy callers
/// keep their original mode and never acquire homogeneous execution implicitly.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn create_sentinel_homogeneous_url_scan(
    app:AppHandle,
    state: State<AppState>, project_id: i64, task_name: String, urls: Vec<String>,
    scan_mode: Option<String>, max_budget_usd: Option<f64>, auth_session_id: Option<String>,
    auth_session_ids: Option<Vec<String>>, auth_session_scope_id: Option<String>,
    skill_ids: Option<Vec<i64>>, instruction: Option<String>, closure: Option<String>,
    execution:Option<bool>,approved_ids:Option<Vec<String>>,
) -> Result<SentinelScan, String> {
    let connection = db::open(&state.db_path)?;
    let ids=approved_ids.unwrap_or_default();
    if execution.unwrap_or(false){
        let configuration=crate::tool_supply::ToolAdministratorConfiguration::from_app(&app)?;
        return create_sentinel_url_scan_with_tool_selection_execution_in(&connection,&configuration,project_id,task_name,urls,scan_mode,max_budget_usd,auth_session_id,auth_session_ids,auth_session_scope_id,skill_ids,instruction,closure,Some("multi".into()),ids,true);
    }
    if !ids.is_empty(){return Err("homogeneous_readonly_draft_cannot_select_execution_tools".into());}
    create_sentinel_url_scan_scheduling_in(&connection, project_id, task_name, urls, scan_mode,
        max_budget_usd, auth_session_id, auth_session_ids, auth_session_scope_id, skill_ids,
        instruction, closure, Some("multi".into()), true)
}

#[allow(clippy::too_many_arguments)]
fn create_sentinel_url_scan_scheduling_in(
    connection: &rusqlite::Connection, project_id: i64, task_name: String, urls: Vec<String>,
    scan_mode: Option<String>, max_budget_usd: Option<f64>, auth_session_id: Option<String>,
    auth_session_ids: Option<Vec<String>>, auth_session_scope_id: Option<String>,
    skill_ids: Option<Vec<i64>>, instruction: Option<String>, closure: Option<String>,
    orchestration_mode: Option<String>, homogeneous: bool,
) -> Result<SentinelScan, String> {
    let mode = crate::agent_runtime::web_mode::WebMode::new_input(orchestration_mode.as_deref())?;
    // Identity ownership, policy and all targets are one draft. In particular,
    // a late target insert failure must not consume a captured login identity.
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定 Web 任务草稿：{error}"))?;
    let window = crate::agent_runtime::web_mode::draft::DraftCreationWindow::begin(&transaction)?;
    let scan = create_sentinel_url_draft_rows(
        &transaction,
        project_id,
        task_name,
        urls,
        scan_mode,
        max_budget_usd,
        auth_session_id,
        auth_session_ids,
        auth_session_scope_id,
        skill_ids,
        instruction,
        closure,
        None,
    )?;
    if homogeneous {
        freeze_homogeneous_metadata_in(&transaction,&scan.id,false)?;
    }
    crate::agent_runtime::web_mode::draft::register_new(window, &scan.id, mode)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交 Web 任务草稿：{error}"))?;
    Ok(scan)
}

fn freeze_homogeneous_metadata_in(transaction:&rusqlite::Transaction<'_>,scan:&str,execution:bool)->Result<(),String>{
        let goal:String=transaction.query_row("SELECT COALESCE(json_extract(policy_json,'$.additionalInstruction'),'') FROM sentinel_scan_contexts WHERE scan_id=?1",[scan],|r|r.get(0)).map_err(|e|e.to_string())?;
        let mut plan = json!({"schemaVersion":1,"scanId":scan,"attemptNumber":1,
            "scheduling_model":"homogeneous_workers","workerExecution":"model_read_only_v1",
            "allowedParallel":3,"lane":"read_only_analysis","goal":goal,
            "credentialVisibility":"plaintext"});
        if execution{plan["workerExecution"]=json!(crate::agent_runtime::multi_agent::homogeneous::execution_policy::MODE);plan["targetParallel"]=json!(1);plan["reviewParallel"]=json!(1);}
        transaction.execute("INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash) VALUES(?1,1,'web','native',?2,?3)",
            params![scan, plan.to_string(), crate::agent_runtime::store::stable_hash(&plan.to_string())])
            .map_err(|e| format!("homogeneous_plan_freeze:{e}"))?;
        transaction.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.scheduling_model','homogeneous_workers','$.workerExecution',?2) WHERE scan_id=?1",params![scan,plan["workerExecution"].as_str().unwrap()])
            .map_err(|e| format!("homogeneous_draft_policy:{e}"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn create_sentinel_url_draft_rows(
    connection: &rusqlite::Transaction<'_>,
    project_id: i64,
    task_name: String,
    urls: Vec<String>,
    scan_mode: Option<String>,
    max_budget_usd: Option<f64>,
    auth_session_id: Option<String>,
    auth_session_ids: Option<Vec<String>>,
    auth_session_scope_id: Option<String>,
    skill_ids: Option<Vec<i64>>,
    instruction: Option<String>,
    closure: Option<String>,
    exact_target: Option<&str>,
) -> Result<SentinelScan, String> {
    let project_name: String = connection
        .query_row(
            "SELECT name FROM projects WHERE id=?1 AND status='active'",
            [project_id],
            |r| r.get(0),
        )
        .map_err(|_| "项目不存在或已归档；恢复工作空间后才能创建新任务".to_string())?;
    let mut normalized = Vec::new();
    for value in urls {
        for piece in value.split(|character: char| {
            character.is_whitespace() || matches!(character, ',' | '，' | ';' | '；')
        }) {
            let piece = piece
                .trim()
                .trim_matches(|character| matches!(character, '"' | '\'' | '<' | '>' | '，' | '。'))
                .trim_end_matches('/')
                .to_string();
            if !piece.is_empty() {
                normalized.push(piece);
            }
        }
    }
    normalized.sort();
    normalized.dedup();
    if let Some(target) = exact_target {
        // A source binding fixes the literal URL too (including a trailing
        // slash). Never change its resource while preparing the next task.
        if normalized.len() != 1 || normalized[0] != target.trim_end_matches('/') {
            return Err("followup_exact_target_invalid".into());
        }
        normalized[0] = target.into();
    }
    if normalized.is_empty() {
        return Err("至少需要一个 URL".into());
    }
    if normalized.len() > 200 {
        return Err("单个 Web 任务最多 200 个 URL".into());
    }
    if normalized
        .iter()
        .any(|url| !(url.starts_with("http://") || url.starts_with("https://")))
    {
        return Err("URL 必须以 http:// 或 https:// 开头".into());
    }
    let scan_mode = normalized_web_scan_mode(scan_mode.as_deref()).to_string();
    if max_budget_usd.is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 10_000.0) {
        return Err("单任务费用上限必须大于 0 且不超过 10000 USD".into());
    }
    let auth_session_id = auth_session_id.unwrap_or_default().trim().to_string();
    let mut auth_session_ids = auth_session_ids
        .unwrap_or_default()
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if !auth_session_id.is_empty() {
        auth_session_ids.push(auth_session_id.clone());
    }
    auth_session_ids.sort();
    auth_session_ids.dedup();
    if auth_session_ids.len() > 5 {
        return Err("单个任务最多比较 5 个登录身份".into());
    }
    let auth_session_scope_id = auth_session_scope_id.unwrap_or_default();
    crate::auth_session::validate_draft_sessions_for_task(
        connection,
        &auth_session_ids,
        project_id,
        &auth_session_scope_id,
    )?;
    crate::auth_session::distinct_session_documents_for_scan(
        connection,
        &auth_session_ids,
        project_id,
    )?;
    let mut targets = Vec::new();
    let mut fused = Vec::new();
    for url in normalized {
        let blocked = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sentinel_fuse_zone WHERE project_id=?1 AND normalized_url=lower(rtrim(trim(?2),'/')))",
                params![project_id, url],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| format!("无法检查目标熔断状态：{error}"))?;
        if blocked {
            fused.push(url);
        } else {
            targets.push(url);
        }
    }
    if targets.is_empty() {
        return Err(format!(
            "输入 URL 全部位于熔断区，未创建任务：{}",
            fused.join("，")
        ));
    }
    let scan_id = fresh_web_draft_id();
    let title = if task_name.trim().is_empty() {
        format!("{} · Web", project_name)
    } else {
        task_name.trim().chars().take(120).collect()
    };
    let inserted = connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name) VALUES(?1,?2,?3,'draft',?5,'','web',?4)",
            params![
                scan_id,
                project_id,
                project_name,
                title,
                if fused.is_empty() {
                    format!("待确认：已加入 {} 个 URL", targets.len())
                } else {
                    format!(
                        "待确认：已加入 {} 个 URL；{} 个在熔断区，未加入：{}",
                        targets.len(),
                        fused.len(),
                        fused.join("，")
                    )
                }
            ],
        )
        .map_err(|error| error.to_string())?;
    if inserted != 1 {
        return Err("Web 任务草稿写入未完成".into());
    }
    let policy = build_web_investigation_policy(
        Some(&scan_mode),
        max_budget_usd,
        auth_session_ids.clone(),
        &skill_ids.unwrap_or_default(),
        instruction.as_deref().unwrap_or(""),
        "agent-dialog",
        closure.as_deref(),
    )?;
    let inserted = connection.execute(
        "INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2) ON CONFLICT(scan_id) DO UPDATE SET policy_json=excluded.policy_json,updated_at=datetime('now','localtime')",
        params![scan_id, policy.to_string()],
    ).map_err(|error| error.to_string())?;
    if inserted != 1 {
        return Err("Web 任务策略写入未完成".into());
    }
    crate::auth_session::bind_draft_sessions_to_scan(
        connection,
        &auth_session_ids,
        project_id,
        &auth_session_scope_id,
        &scan_id,
    )?;
    for url in targets {
        let inserted = connection
            .execute(
                "INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,'queued')",
                params![project_id, scan_id, project_name, url],
            )
            .map_err(|error| error.to_string())?;
        if inserted != 1 {
            return Err("Web 任务目标写入未完成".into());
        }
    }
    sentinel_scan_by_id(connection, &scan_id)
}

const SENTINEL_SCAN_COLUMNS: &str = "id,project_id,project_name,status,current_checkpoint,task_path,previous_scan_id,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,scan_type,task_name,source_path,skill_names,attempt_count,created_at,updated_at,CASE WHEN scan_type='web' THEN COALESCE((SELECT json_extract(policy_json,'$.webModeCeiling') FROM sentinel_scan_contexts WHERE scan_id=sentinel_scans.id),'standard') ELSE '' END,COALESCE((SELECT attempt_number FROM sentinel_scan_attempts WHERE scan_id=sentinel_scans.id ORDER BY attempt_number DESC LIMIT 1),0),COALESCE((SELECT status FROM sentinel_scan_attempts WHERE scan_id=sentinel_scans.id ORDER BY attempt_number DESC LIMIT 1),''),COALESCE((SELECT checkpoint FROM sentinel_scan_attempts WHERE scan_id=sentinel_scans.id ORDER BY attempt_number DESC LIMIT 1),''),COALESCE((SELECT stop_reason FROM sentinel_scan_attempts WHERE scan_id=sentinel_scans.id ORDER BY attempt_number DESC LIMIT 1),''),archived_at,EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=sentinel_scans.id),EXISTS(SELECT 1 FROM native_web_closure_handoffs WHERE scan_id=sentinel_scans.id)";

fn scan_llm_policy(task_path: &str) -> (String, String, bool) {
    let Some(value) = fs::read(task_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok())
    else {
        return (String::new(), "unknown".into(), false);
    };
    let policy = value.get("llmPolicy").unwrap_or(&JsonValue::Null);
    (
        policy
            .get("model")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string(),
        match policy.get("deployment").and_then(JsonValue::as_str) {
            Some("local") => "local".into(),
            Some("cloud") => "cloud".into(),
            _ => "unknown".into(),
        },
        policy
            .get("fullPower")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
    )
}

fn sentinel_scan_row(row: &Row<'_>) -> rusqlite::Result<SentinelScan> {
    let task_path: String = row.get(5)?;
    let (llm_model, llm_deployment, llm_full_power) = scan_llm_policy(&task_path);
    Ok(SentinelScan {
        id: row.get(0)?,
        project_id: row.get(1)?,
        project_name: row.get(2)?,
        status: row.get(3)?,
        current_checkpoint: row.get(4)?,
        task_path,
        previous_scan_id: row.get(6)?,
        llm_requests: row.get(7)?,
        input_tokens: row.get(8)?,
        output_tokens: row.get(9)?,
        cached_tokens: row.get(10)?,
        total_tokens: row.get(11)?,
        scan_type: row.get(12)?,
        task_name: row.get(13)?,
        source_path: row.get(14)?,
        skill_names: row.get(15)?,
        attempt_count: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        requested_scan_mode: row.get(19)?,
        llm_model,
        llm_deployment,
        llm_full_power,
        latest_attempt_number: row.get(20)?,
        latest_attempt_status: row.get(21)?,
        latest_attempt_checkpoint: row.get(22)?,
        latest_attempt_stop_reason: row.get(23)?,
        archived_at: row.get(24)?,
        administrative_closure_recorded: row.get(25)?,
        closure_handoff_recorded: row.get(26)?,
    })
}

fn sentinel_scan_by_id(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<SentinelScan, String> {
    connection
        .query_row(
            &format!("SELECT {SENTINEL_SCAN_COLUMNS} FROM sentinel_scans WHERE id=?1"),
            [scan_id],
            sentinel_scan_row,
        )
        .map_err(|error| error.to_string())
}

fn sentinel_attempt_stage(status: &str, checkpoint: &str) -> String {
    let text = checkpoint.to_lowercase();
    if matches!(status, "completed" | "partial" | "recon_only") {
        return "complete".into();
    }
    if matches!(status, "failed" | "cancelled") {
        return "stopped".into();
    }
    if matches!(status, "paused" | "pausing") {
        return "paused".into();
    }
    if text.contains("漏洞")
        || text.contains("finding")
        || text.contains("结果")
        || text.contains("同步")
    {
        return "evidence".into();
    }
    if text.contains("agent")
        || text.contains("模型")
        || text.contains("验证")
        || text.contains("poc")
    {
        return "validation".into();
    }
    if text.contains("前端")
        || text.contains("浏览器")
        || text.contains("javascript")
        || text.contains(" js")
        || text.contains("探测")
        || text.contains("接口")
    {
        return "frontend_recon".into();
    }
    if text.contains("队列") || text.contains("准备") || text.contains("docker") {
        return "preparing".into();
    }
    "running".into()
}

fn record_sentinel_attempt_start(
    connection: &rusqlite::Connection,
    scan_id: &str,
    attempt_number: i64,
    work_dir: &Path,
) -> Result<(), String> {
    let mode_key = format!("sentinel-next-attempt-mode:{scan_id}");
    let execution_mode = connection
        .query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [&mode_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .filter(|value| matches!(value.as_str(), "fresh" | "resume"))
        .unwrap_or_else(|| "initial".into());
    let (status, checkpoint, requests, input, output, cached, total): (
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = connection
        .query_row(
            "SELECT status,current_checkpoint,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
        )
        .map_err(|error| error.to_string())?;
    let stage = sentinel_attempt_stage(&status, &checkpoint);
    connection
        .execute(
            "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status,stage,checkpoint,work_dir,llm_requests_start,input_tokens_start,output_tokens_start,cached_tokens_start,total_tokens_start) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(scan_id,attempt_number) DO UPDATE SET execution_mode=excluded.execution_mode,status=excluded.status,stage=excluded.stage,checkpoint=excluded.checkpoint,work_dir=excluded.work_dir,llm_requests_start=excluded.llm_requests_start,input_tokens_start=excluded.input_tokens_start,output_tokens_start=excluded.output_tokens_start,cached_tokens_start=excluded.cached_tokens_start,total_tokens_start=excluded.total_tokens_start,llm_requests_delta=0,input_tokens_delta=0,output_tokens_delta=0,cached_tokens_delta=0,total_tokens_delta=0,stop_reason='',finished_at='',started_at=datetime('now','localtime'),updated_at=datetime('now','localtime')",
            params![scan_id, attempt_number, execution_mode, status, stage, checkpoint, work_dir.to_string_lossy(), requests, input, output, cached, total],
        )
        .map_err(|error| error.to_string())?;
    // The backend matrix is written before this row exists, so the update then
    // matches nothing. Copy the already stored plan onto the attempt.
    let _ = connection.execute(
        "UPDATE sentinel_scan_attempts SET backend_plan_json=(
            SELECT raw_json FROM sentinel_checkpoints
            WHERE scan_id=?1 AND stage='scan_backend_plan' AND trim(url)=''
            ORDER BY updated_at DESC LIMIT 1
         ) WHERE scan_id=?1 AND attempt_number=?2 AND trim(COALESCE(backend_plan_json,''))=''",
        params![scan_id, attempt_number],
    );
    connection
        .execute("DELETE FROM app_settings WHERE key=?1", [mode_key])
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn sync_sentinel_attempt(connection: &rusqlite::Connection, scan_id: &str) {
    let current = connection.query_row(
        "SELECT status,current_checkpoint,attempt_count FROM sentinel_scans WHERE id=?1",
        [scan_id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    );
    let Ok((status, checkpoint, attempt_number)) = current else {
        return;
    };
    if attempt_number <= 0 {
        return;
    }
    let stage = sentinel_attempt_stage(&status, &checkpoint);
    let terminal = matches!(
        status.as_str(),
        "completed"
            | "completed_with_gaps"
            | "partial"
            | "recon_only"
            | "failed"
            | "cancelled"
            | "paused"
    );
    let _ = connection.execute(
        "UPDATE sentinel_scan_attempts SET status=?1,stage=?2,checkpoint=CASE WHEN ?4=1 AND trim(stop_reason)<>'' THEN checkpoint ELSE ?3 END,stop_reason=CASE WHEN ?4=1 AND trim(stop_reason)='' THEN ?3 ELSE stop_reason END,llm_requests_delta=MAX(0,(SELECT llm_requests FROM sentinel_scans WHERE id=?5)-llm_requests_start),input_tokens_delta=MAX(0,(SELECT input_tokens FROM sentinel_scans WHERE id=?5)-input_tokens_start),output_tokens_delta=MAX(0,(SELECT output_tokens FROM sentinel_scans WHERE id=?5)-output_tokens_start),cached_tokens_delta=MAX(0,(SELECT cached_tokens FROM sentinel_scans WHERE id=?5)-cached_tokens_start),total_tokens_delta=MAX(0,(SELECT total_tokens FROM sentinel_scans WHERE id=?5)-total_tokens_start),finished_at=CASE WHEN ?4=1 AND finished_at='' THEN datetime('now','localtime') WHEN ?4=0 THEN '' ELSE finished_at END,updated_at=datetime('now','localtime') WHERE scan_id=?5 AND attempt_number=?6 AND NOT EXISTS(SELECT 1 FROM native_web_attempt_closures WHERE scan_id=?5 AND attempt_number=?6)",
        params![status, stage, checkpoint, terminal as i64, scan_id, attempt_number],
    );
}

const SENTINEL_RESCAN_COUNT_SQL: &str = "SELECT COUNT(*) FROM sentinel_targets t WHERE scan_id=?1 AND (?2=0 OR status NOT IN ('completed','recon_only','manual_review')) AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=t.project_id AND f.normalized_url=lower(rtrim(trim(t.url),'/')))";
#[cfg(test)]
const SENTINEL_RESCAN_COPY_SQL: &str = "INSERT INTO sentinel_targets(project_id,scan_id,asset_id,company,url,status) SELECT t.project_id,?1,t.asset_id,t.company,t.url,'queued' FROM sentinel_targets t WHERE t.scan_id=?2 AND (?3=0 OR t.status NOT IN ('completed','recon_only','manual_review')) AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=t.project_id AND f.normalized_url=lower(rtrim(trim(t.url),'/')))";
#[cfg(test)]
const SENTINEL_RESCAN_RECON_COPY_SQL: &str = "INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json,updated_at) SELECT ?1,c.url,c.stage,c.raw_json,datetime('now','localtime') FROM sentinel_checkpoints c JOIN sentinel_targets t ON t.scan_id=?1 AND t.url=c.url WHERE c.scan_id=?2 AND c.stage='frontend_recon' ON CONFLICT(scan_id,url,stage) DO UPDATE SET raw_json=excluded.raw_json,updated_at=excluded.updated_at";
#[cfg(test)]
const SENTINEL_RESUME_COUNT_SQL: &str = "SELECT COUNT(*) FROM sentinel_targets WHERE scan_id=?1 AND status NOT IN ('completed','completed_with_gaps','partial','recon_only','manual_review','limited','protected_stop','failed','fuse_excluded','resume_incompatible','persistence_failure')";
const SENTINEL_RESUME_TARGETS_SQL: &str = "SELECT company,url FROM sentinel_targets WHERE scan_id=?1 AND status NOT IN ('completed','completed_with_gaps','partial','recon_only','manual_review','limited','protected_stop','failed','fuse_excluded','resume_incompatible','persistence_failure') ORDER BY id";

#[cfg(test)]
fn prepare_web_scan_retry(
    connection: &mut rusqlite::Connection,
    scan_id: &str,
    status: &str,
) -> Result<i64, String> {
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let count = prepare_web_scan_retry_in(&transaction, scan_id, status)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(count)
}

fn prepare_web_scan_retry_in(
    connection: &rusqlite::Connection,
    scan_id: &str,
    status: &str,
) -> Result<i64, String> {
    if connection.is_autocommit() {
        return Err("web_retry_requires_transaction".into());
    }
    let registered: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_processes WHERE scan_id=?1)
         OR EXISTS(SELECT 1 FROM analyzer_container_receipts WHERE scan_id=?1 AND cleanup_status<>'confirmed')",
        [scan_id],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if registered {
        return Err("scan_quiescence_cleanup_unconfirmed".into());
    }
    match status {
        "scanning" | "pausing" => return Err("任务仍在运行，请先暂停后再继续".into()),
        "paused" => return Err("暂停任务请使用“继续扫描”，无需重新执行".into()),
        "draft" => return Err("任务仍待确认，不能重复准备".into()),
        _ => {}
    }
    let retry_incomplete_only = retry_only_incomplete_targets(status);
    // Archived fuse rows are immutable history. Only active protection-system
    // entries may exclude a target from a new attempt.
    let target_count: i64 = connection
        .query_row(
            SENTINEL_RESCAN_COUNT_SQL,
            params![scan_id, retry_incomplete_only as i64],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if target_count == 0 {
        return Err("当前任务没有可继续执行的 URL；目标可能都在熔断区".into());
    }
    let fuse_excluded: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sentinel_targets t WHERE t.scan_id=?1 AND (?2=0 OR status NOT IN ('completed','recon_only','manual_review')) AND EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=t.project_id AND f.normalized_url=lower(rtrim(trim(t.url),'/')))",
        params![scan_id, retry_incomplete_only as i64],
        |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    let next_attempt = connection
        .query_row(
            "SELECT attempt_count+1 FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?
        .max(1);
    let execution_mode = if retry_incomplete_only {
        "resume"
    } else {
        "fresh"
    };
    let checkpoint = if !retry_incomplete_only && fuse_excluded > 0 {
        format!("已建立第 {next_attempt} 次全新执行计划：清空当前结果面后重新处理 {target_count} 个 URL，排除熔断区 {fuse_excluded} 个；旧结果仅保留在执行历史中")
    } else if !retry_incomplete_only {
        format!("已建立第 {next_attempt} 次全新执行计划：清空当前结果面后重新处理 {target_count} 个 URL；旧结果仅保留在执行历史中")
    } else if fuse_excluded > 0 {
        format!("已建立第 {next_attempt} 次续跑计划：仅处理 {target_count} 个未完成 URL，排除熔断区 {fuse_excluded} 个；保留可复用证据，旧状态仅保留在执行历史中")
    } else {
        format!("已建立第 {next_attempt} 次续跑计划：仅处理 {target_count} 个未完成 URL；保留可复用证据，旧状态仅保留在执行历史中")
    };
    let target_ids = {
        let mut statement = connection.prepare("SELECT t.id FROM sentinel_targets t WHERE scan_id=?1 AND (?2=0 OR status NOT IN ('completed','recon_only','manual_review')) AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=t.project_id AND f.normalized_url=lower(rtrim(trim(t.url),'/'))) ORDER BY t.id").map_err(|e| e.to_string())?;
        let rows = statement
            .query_map(params![scan_id, retry_incomplete_only as i64], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    let routing_reason = format!("第 {next_attempt} 次执行待重新分流；上一轮结束原因见执行历史");
    let changed = connection
        .execute(
            "UPDATE sentinel_targets SET status='queued',value_score=0,scan_mode='',routing_reason=?3,updated_at=datetime('now','localtime') WHERE scan_id=?1 AND (?2=0 OR status NOT IN ('completed','recon_only','manual_review')) AND NOT EXISTS (SELECT 1 FROM sentinel_fuse_zone f WHERE f.archived=0 AND f.project_id=sentinel_targets.project_id AND f.normalized_url=lower(rtrim(trim(sentinel_targets.url),'/')))",
            params![scan_id, retry_incomplete_only as i64, routing_reason],
        )
        .map_err(|error| error.to_string())?;
    if changed as i64 != target_count {
        return Err("web_retry_targets_not_persisted".into());
    }
    connection
        .execute(
            "INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![format!("sentinel-next-attempt-mode:{scan_id}"), execution_mode],
        )
        .map_err(|error| error.to_string())?;
    let changed = connection
        .execute(
            "UPDATE sentinel_scans SET status='draft',current_checkpoint=?1,previous_scan_id='',updated_at=datetime('now','localtime') WHERE id=?2 AND status=?3",
            params![checkpoint, scan_id, status],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("web_retry_state_changed".into());
    }
    for id in target_ids {
        let valid: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_targets WHERE id=?1 AND scan_id=?2 AND status='queued' AND value_score=0 AND scan_mode='' AND routing_reason=?3)", params![id,scan_id,routing_reason], |r| r.get(0)).map_err(|e| e.to_string())?;
        if !valid {
            return Err("web_retry_target_postcondition".into());
        }
    }
    let valid: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND status='draft' AND current_checkpoint=?2 AND previous_scan_id='') AND EXISTS(SELECT 1 FROM app_settings WHERE key=?3 AND value=?4) AND NOT EXISTS(SELECT 1 FROM sentinel_processes WHERE scan_id=?1)",params![scan_id,checkpoint,format!("sentinel-next-attempt-mode:{scan_id}"),execution_mode],|r| r.get(0)).map_err(|e| e.to_string())?;
    if !valid {
        return Err("web_retry_preparation_postcondition".into());
    }
    Ok(target_count)
}

fn retry_only_incomplete_targets(status: &str) -> bool {
    matches!(status, "partial" | "failed" | "limited" | "cancelled")
}

fn next_scan_attempt_number(scan_root: &Path, minimum_attempt: u32) -> u32 {
    let mut max_attempt = 0u32;
    if let Ok(entries) = fs::read_dir(scan_root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(number) = name.strip_prefix("attempt-") else {
                continue;
            };
            if entry.path().is_dir() {
                max_attempt = max_attempt.max(number.parse::<u32>().unwrap_or(0));
            }
        }
    }
    let has_legacy_attempt = [
        "url-pipeline",
        "batches",
        "oviraptor-runner.log",
        "llm-hook.jsonl",
        "targets.json",
        "targets.txt",
    ]
    .iter()
    .any(|name| scan_root.join(name).exists());
    let next_discovered_attempt = if max_attempt > 0 {
        max_attempt.saturating_add(1)
    } else if has_legacy_attempt {
        2
    } else {
        1
    };
    next_discovered_attempt.max(minimum_attempt.max(1))
}

#[cfg(test)]
fn next_scan_attempt_work_dir(scan_root: &Path, minimum_attempt: u32) -> Result<PathBuf, String> {
    fs::create_dir_all(scan_root).map_err(|error| error.to_string())?;
    let attempt = next_scan_attempt_number(scan_root, minimum_attempt);
    let work_dir = scan_root.join(format!("attempt-{attempt:04}"));
    fs::create_dir_all(&work_dir).map_err(|error| error.to_string())?;
    Ok(work_dir)
}

#[tauri::command]
pub fn rescan_sentinel_scan(
    app: AppHandle,
    state: State<AppState>,
    scan_id: String,
) -> Result<SentinelScan, String> {
    start_web_scan(app, state, scan_id, WebStartMode::Retry)
}

fn sentinel_settings(connection: &rusqlite::Connection) -> JsonValue {
    connection
        .query_row(
            "SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub fn list_agent_instructions(state: State<AppState>) -> Result<Vec<AgentSkill>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection.prepare(
        "SELECT id,name,description,instructions,builtin,enabled,created_at,updated_at FROM agent_skills ORDER BY builtin DESC,updated_at DESC,id DESC"
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(AgentSkill {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                instructions: row.get(3)?,
                builtin: row.get::<_, i64>(4)? != 0,
                enabled: row.get::<_, i64>(5)? != 0,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn save_agent_instruction(
    state: State<AppState>,
    input: AgentSkillInput,
) -> Result<i64, String> {
    let name = input.name.trim();
    let instructions = input.instructions.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return Err("技能名称不能为空且不能超过 80 个字符".into());
    }
    if instructions.is_empty() || instructions.chars().count() > 30_000 {
        return Err("技能指令不能为空且不能超过 30000 个字符".into());
    }
    let connection = db::open(&state.db_path)?;
    if let Some(id) = input.id {
        let builtin: i64 = connection
            .query_row(
                "SELECT builtin FROM agent_skills WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(|_| "技能不存在".to_string())?;
        if builtin != 0 {
            return Err("内置技能不可覆盖；请新建自定义技能".into());
        }
        connection.execute("UPDATE agent_skills SET name=?1,description=?2,instructions=?3,enabled=?4,updated_at=datetime('now','localtime') WHERE id=?5", params![name,input.description.trim(),instructions,input.enabled as i64,id]).map_err(|error| error.to_string())?;
        Ok(id)
    } else {
        connection.execute("INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,0,?4)", params![name,input.description.trim(),instructions,input.enabled as i64]).map_err(|error| error.to_string())?;
        Ok(connection.last_insert_rowid())
    }
}

#[tauri::command]
pub fn delete_agent_instruction(state: State<AppState>, skill_id: i64) -> Result<(), String> {
    let connection = db::open(&state.db_path)?;
    let deleted = connection
        .execute(
            "DELETE FROM agent_skills WHERE id=?1 AND builtin=0",
            [skill_id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("内置技能不可删除，或技能不存在".into());
    }
    Ok(())
}

/// task name, project, status, scan type, path, then the five usage counters and
/// the two timestamps.
type AgentTraceRow = (
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    String,
    String,
);

fn agent_trace_base(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<AgentTraceRow, String> {
    connection
        .query_row(
            "SELECT task_name,project_name,status,scan_type,task_path,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,created_at,updated_at FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?,row.get(8)?,row.get(9)?,row.get(10)?,row.get(11)?)),
        )
        .map_err(|_| "Agent 任务不存在".to_string())
}

fn retained_trace_value(value: &JsonValue) -> JsonValue {
    crate::agent_runtime::secrets::redact_json_with(value, None)
}

fn retained_trace_text(value: &str) -> String {
    crate::agent_runtime::secrets::redact_text_with(value, None)
}

fn trace_preview(value: &str, limit: usize) -> (String, i64, bool) {
    let retained = retained_trace_text(value);
    let size = value.len() as i64;
    let mut preview = retained.chars().take(limit).collect::<String>();
    let truncated = retained.chars().count() > limit;
    if truncated {
        preview.push_str("\n… [preview truncated]");
    }
    (preview, size, truncated)
}

fn collect_agent_trace(
    connection: &rusqlite::Connection,
    scan_id: &str,
    include_events: bool,
    latest_attempt_only: bool,
) -> Result<(AgentTraceSummary, Vec<AgentTraceEvent>), String> {
    if native_trace_available(connection, scan_id)? {
        collect_native_agent_trace(connection, scan_id, include_events, latest_attempt_only)
    } else {
        collect_historical_agent_trace(connection, scan_id, include_events, latest_attempt_only)
    }
}

#[tauri::command]
pub fn list_agent_traces(state: State<AppState>) -> Result<Vec<AgentTraceSummary>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT id FROM sentinel_scans s WHERE task_path<>'' OR EXISTS(\
             SELECT 1 FROM agent_runs r WHERE r.scan_id=s.id AND r.backend='native')\
             ORDER BY created_at DESC LIMIT 120",
        )
        .map_err(|error| error.to_string())?;
    let ids = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .flatten()
        .collect::<Vec<_>>();
    drop(statement);
    Ok(ids
        .iter()
        .filter_map(|id| {
            collect_agent_trace(&connection, id, false, false)
                .ok()
                .map(|value| value.0)
        })
        .collect())
}

fn read_agent_trace_detail(
    connection: &rusqlite::Connection,
    scan_id: &str,
) -> Result<AgentTraceDetail, String> {
    let fallback_task_path = agent_trace_base(connection, scan_id)?.4;
    let task_path = connection
        .query_row(
            "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 ORDER BY attempt_number DESC LIMIT 1",
            [scan_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback_task_path);
    let (summary, events) = collect_agent_trace(connection, scan_id, true, true)?;
    let mut prompt_audit = if native_trace_available(connection, scan_id)? {
        let path = Path::new(&task_path).join("model-prompt-audit.json");
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ModelPromptAudit>(&bytes).ok())
            .map(|mut audit| {
                audit.instruction = audit.instruction.as_deref().map(retained_trace_text);
                audit
            })
    } else {
        historical_prompt_audit(connection, scan_id)?
    };
    if let Some(audit) = prompt_audit.as_mut() {
        // A captured request object is still a redacted historical projection,
        // never proof that the instruction is the exact provider request.
        audit.exact_model_request = false;
        if summary.source_authority == "historical_external" && summary.exact_request_capture {
            audit.capture_level = "generated_instruction_and_redacted_model_requests".into();
            audit.notice = "历史外部 instruction 与 Hook 请求来自 canonical 导入记录：只读、未复核、脱敏展示；不代表 Native 请求或磁盘上的历史源文件已被清理。".into();
        }
    }
    Ok(project_trace_display_privacy(AgentTraceDetail {
        summary,
        events,
        prompt_audit,
    }))
}

#[tauri::command]
pub fn get_agent_trace(
    state: State<AppState>,
    scan_id: String,
) -> Result<AgentTraceDetail, String> {
    read_agent_trace_detail(&db::open(&state.db_path)?, &scan_id)
}
