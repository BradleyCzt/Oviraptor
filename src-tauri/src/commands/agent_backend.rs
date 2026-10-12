/// §11: everything the task detail "执行计划" panel shows, from the plan and
/// checkpoint rows. Raw request/response JSON stays in the existing viewer.
#[tauri::command]
pub fn get_agent_target_execution(
    state: State<'_, AppState>,
    scan_id: String,
    url: String,
) -> Result<JsonValue, String> {
    read_agent_target_execution(&state.db_path, &scan_id, &url)
}

fn read_agent_target_execution(db_path: &Path, scan_id: &str, url: &str) -> Result<JsonValue, String> {
    let database = db::open(db_path)?;
    let connection = database.unchecked_transaction().map_err(|e|e.to_string())?;
    let attempt: i64 = connection.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1",
        [scan_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    let accounting = agent_request_accounting_view(&connection,scan_id,attempt,url);
    let plan = read_agent_checkpoint(db_path, scan_id, url, "agent_execution_plan");
    let native = NativeAgentState::read(db_path, scan_id, url);
    let mut budget_usage = native.as_ref().map(|value|value.budget_usage.clone())
        .filter(JsonValue::is_object).unwrap_or(serde_json::json!({}));
    budget_usage["targetRequests"] = accounting["recordedRequests"].clone();
    let (target_status, target_mode): (String, String) = connection
        .query_row(
            "SELECT status,scan_mode FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
            params![scan_id, url],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or_else(|_| ("queued".to_string(), String::new()));
    let coverage: String = connection
        .query_row(
            "SELECT record_json FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage=?3 AND record_key='coverage' ORDER BY updated_at DESC LIMIT 1",
            params![scan_id, url, AGENT_COVERAGE_STAGE],
            |row| row.get(0),
        )
        .unwrap_or_default();
    let confirmed: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage=?3 AND kind='vulnerability'",
            params![scan_id, url, AGENT_VULNERABILITY_STAGE],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let required: Vec<&str> = agent_required_families();
    let covered: Vec<String> = native
        .as_ref()
        .map(|value| {
            let mut rows = value.covered_families.clone();
            rows.sort();
            rows
        })
        .unwrap_or_default();
    let usage_from_plan = plan.get("budgets").cloned().unwrap_or(serde_json::json!({}));
    Ok(serde_json::json!({
        "backend": plan.get("backend").cloned().unwrap_or_else(|| serde_json::json!("native")),
        "mode": plan.get("mode").cloned().unwrap_or(serde_json::json!(target_mode)),
        "surface": plan.get("surface"),
        "timeoutSeconds": plan.get("timeoutSeconds"),
        "budgets": usage_from_plan,
        "hardLimits": {
            "hardTotalTokens": plan.pointer("/budgets/hardTotalTokens"),
            "hardModelRequests": plan.pointer("/budgets/hardModelRequests"),
            "maxTurns": plan.pointer("/budgets/maxTurns"),
            "noProgressWindow": plan.pointer("/stopping/noProgressWindow")
        },
        "coverage": {
            "required": required,
            "requiredLabels": required.iter().map(|family| agent_coverage_family_label(family)).collect::<Vec<_>>(),
            "covered": covered,
            "completedRatio": if required.is_empty() { 0.0 } else { covered.len() as f64 / required.len() as f64 },
            "ledger": json(coverage),
            "ledgerReported": native.is_some(),
            "confirmedFindings": confirmed
        },
        "runtime": {
            "turns": native.as_ref().map(|value| value.turns).unwrap_or(0),
            "noProgressStreak": native.as_ref().map(|value| value.no_progress_streak).unwrap_or(0),
            "currentAction": native.as_ref().and_then(|value| value.pending_queue.first().cloned()),
            "progressSignature": native.as_ref().map(|value| value.progress_signature.clone()).unwrap_or_default(),
            "lastExpansionReason": native.as_ref().map(|value| value.last_expansion_reason.clone()).unwrap_or_default(),
            "budgetUsage": budget_usage,
            "requestAccounting": accounting,
            "tokenUsage": native.as_ref().map(|value| value.token_usage.as_json()).unwrap_or_else(|| serde_json::json!({})),
            "terminalReason": native.as_ref().map(|value| value.terminal_reason.clone()).unwrap_or_default()
        },
        "targetStatus": target_status,
        "targetStatusText": agent_terminal_status_label(&target_status),
    }))
}

fn agent_terminal_status_label(status: &str) -> String {
    match status {
        "completed" => "已完成",
        "partial" | "paused" | "completed_with_gaps" => "待补充验证",
        "limited" | "protected_stop" => "已熔断",
        "failed" => "执行失败",
        "recon_only" => "仅完成确定性侦察",
        "manual_review" => "转人工分析",
        "resume_incompatible" => "续跑状态不兼容，需要重新执行",
        "persistence_failure" => "本地记录失败，已停止以避免重复消耗",
        "cancelled" => "已取消",
        "scanning" => "正在执行",
        other => other,
    }
    .to_string()
}

/// §12 Stage 4: web, greybox, code and CI are all owned by the native runtime. A
/// source-carrying scan freezes a repository snapshot instead of handing the work to a
/// sandboxed engine, and a capability it cannot have is reported as a gap.
fn agent_native_eligible(scan_type: &str, source_path: &str, urls: &[String]) -> bool {
    if !matches!(scan_type, "web" | "greybox" | "code" | "cicd") {
        return false;
    }
    !urls.is_empty() || !source_path.trim().is_empty()
}

/// Runtime inputs the shared web pipeline needs. Both the Asset task entry and
/// the Agent dialog entry build them here so there is only one set of
/// budget, proxy and worker resolution rules.
#[derive(serde::Serialize)]
struct AgentWebPipelineRuntime {
    worker: PathBuf,
    proxies: Vec<(String, String)>,
    no_proxy: String,
    adaptive: AgentBudgetSettings,
    packet_budget: usize,
    web_policy: JsonValue,
    skill_names: String,
    skill_instructions: String,
}

fn agent_web_pipeline_runtime(
    app: &tauri::AppHandle,
    connection: &rusqlite::Connection,
    scan_id: &str,
    deployment: &str,
) -> Result<AgentWebPipelineRuntime, String> {
    let runtime = resolve_agent_web_pipeline_runtime(
        connection, scan_id, deployment, resolve_frontend_recon_worker(app)?,
    )?;
    connection.execute(
        "INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2) ON CONFLICT(scan_id) DO UPDATE SET policy_json=excluded.policy_json,updated_at=datetime('now','localtime')",
        params![scan_id, runtime.web_policy.to_string()],
    ).map_err(|error| error.to_string())?;
    Ok(runtime)
}

// Recovery must resolve the current configuration without silently rewriting
// the frozen policy to fit it. Only the startup wrapper above publishes policy.
fn resolve_agent_web_pipeline_runtime(
    connection: &rusqlite::Connection,
    scan_id: &str,
    deployment: &str,
    worker: PathBuf,
) -> Result<AgentWebPipelineRuntime, String> {
    let settings = sentinel_settings(connection);
    let mut adaptive = AgentBudgetSettings::from_json(&settings);
    let stored_web_policy = connection
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .map(|text| serde_json::from_str(&text).map_err(|_| "web_runtime_policy_invalid".to_string()))
        .transpose()?
        .unwrap_or_else(|| serde_json::json!({"webModeCeiling": "standard"}));
    let (web_policy, skill_names, skill_instructions) =
        effective_web_policy(connection, &stored_web_policy, &settings)?;
    adaptive.apply_web_policy(&web_policy);
    adaptive.apply_deployment(deployment);
    Ok(AgentWebPipelineRuntime {
        worker,
        proxies: approved_agent_proxies(&settings),
        no_proxy: settings
            .get("noProxy")
            .and_then(JsonValue::as_str)
            .unwrap_or("127.0.0.1,localhost")
            .to_string(),
        packet_budget: frontend_packet_budget(&settings, deployment),
        adaptive,
        web_policy,
        skill_names,
        skill_instructions,
    })
}

/// One target's place in the attempt's backend matrix (Phase 2 §3.2).
#[derive(Clone, Debug, PartialEq)]
struct ScanTargetBackend {
    url: String,
    backend: AgentBackendKind,
    selection_reason: String,
}

/// The immutable per-attempt backend matrix. Dependency resolution happens only
/// after every target has been classified, so a mixed scan cannot start half of
/// its targets before discovering a required Native dependency is unavailable.
#[derive(Clone, Debug, PartialEq)]
struct ScanBackendPlan {
    scan_id: String,
    attempt_number: i64,
    targets: Vec<ScanTargetBackend>,
    requires_node: bool,
    requires_browser: bool,
}

/// Shared dependency boundary for both task-creation commands. Only the neutral
/// retirement marker remains decodable; unsupported aliases are rejected earlier.
/// This validation never probes an executable, container daemon or network host.
fn prepare_scan_dependencies(plan: &ScanBackendPlan) -> Result<(), String> {
    let legacy_targets = plan.targets.iter()
        .filter(|target| target.backend == AgentBackendKind::LegacyRemoved)
        .map(|target| target.url.as_str())
        .collect::<Vec<_>>();
    if legacy_targets.is_empty() {
        return Ok(());
    }
    // A retirement marker must never revive an executable, even for a version probe.
    Err(format!(
        "backend_retired: 本任务冻结了已停用的旧后端（{}）；旧执行计划不受支持，不能继续或自动转换为 Native",
        legacy_targets.join(", ")
    ))
}

impl ScanBackendPlan {
    fn as_json(&self) -> JsonValue {
        serde_json::json!({
            "schemaVersion": 1,
            "scanId": self.scan_id,
            "attemptNumber": self.attempt_number,
            "targets": self.targets.iter().map(|target| serde_json::json!({
                "url": target.url,
                "backend": target.backend.as_str(),
                "selectionReason": target.selection_reason,
            })).collect::<Vec<_>>(),
            "requiresNode": self.requires_node,
            "requiresBrowser": self.requires_browser,
        })
    }

    fn from_json(value: &JsonValue) -> Option<Self> {
        if value.get("schemaVersion").and_then(JsonValue::as_i64) != Some(1) {
            return None;
        }
        let targets = value
            .get("targets")
            .and_then(JsonValue::as_array)?
            .iter()
            .map(|row| {
                Some(ScanTargetBackend {
                    url: row.get("url")?.as_str()?.to_string(),
                    backend: AgentBackendKind::parse(row.get("backend")?.as_str()?)?,
                    selection_reason: row
                        .get("selectionReason")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let flag = |key: &str| {
            value
                .get(key)
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
        };
        Some(Self {
            scan_id: value
                .get("scanId")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string(),
            attempt_number: value
                .get("attemptNumber")
                .and_then(JsonValue::as_i64)
                .unwrap_or(1),
            targets,
            requires_node: flag("requiresNode"),
            requires_browser: flag("requiresBrowser"),
        })
    }

    fn backend_of(&self, url: &str) -> Option<ScanTargetBackend> {
        self.targets.iter().find(|target| target.url == url).cloned()
    }

    fn refresh_requirements(&mut self, node_needed: bool) {
        self.requires_node = node_needed && !self.targets.is_empty();
        self.requires_browser = self.requires_node;
    }
}

// Historical matrix fixtures only. Live Web/workbench startup freezes both
// projection and attempt in its own publication transaction.
#[cfg(test)]
fn write_scan_backend_plan(db_path: &Path, plan: &ScanBackendPlan) -> Result<(), String> {
    let payload = plan.as_json();
    // Never insert an attempt row from here: the pipeline owns that row, and a
    // phantom row would show up as a fake attempt in the UI.
    // A matrix that cannot be stored still governs this start-up decision, because
    // the caller holds the value; the failure is returned so the entry can report it.
    let projection = write_agent_checkpoint(db_path, &plan.scan_id, "", "scan_backend_plan", &payload);
    let attempt_row = db::open(db_path)
        .map_err(|error| error.to_string())
        .and_then(|connection| {
            connection
                .execute(
                    "UPDATE sentinel_scan_attempts SET backend_plan_json=?1,updated_at=datetime('now','localtime') WHERE scan_id=?2 AND attempt_number=?3",
                    params![payload.to_string(), plan.scan_id, plan.attempt_number],
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
        });
    projection.and(attempt_row)
}

/// Decide the backend of every target in this attempt, once. A stored matrix is
/// returned untouched, so editing settings mid-run cannot move a target between
/// backends (Phase 2 §3.2).
#[cfg(test)]
fn plan_scan_backends(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    urls: &[String],
    _settings: &JsonValue,
    native_eligible: bool,
) -> Result<ScanBackendPlan, String> {
    if !native_eligible && urls.iter().any(|url| !url.trim().is_empty()) {
        return Err(
            "unsupported_capability: 当前任务没有 Native 执行入口，且不会回退到已移除后端"
                .into(),
        );
    }
    let mut plan = read_scan_backend_plan(db_path, scan_id, attempt_number)?.unwrap_or_else(|| {
        ScanBackendPlan {
            scan_id: scan_id.to_string(),
            attempt_number,
            targets: Vec::new(),
            requires_node: false,
            requires_browser: false,
        }
    });
    let mut changed = false;
    for url in urls {
        if url.trim().is_empty() || plan.backend_of(url).is_some() {
            continue;
        }
        let (backend, reason) = agent_backend_choice(
            db_path,
            scan_id,
            attempt_number,
            url,
            &JsonValue::Null,
            native_eligible,
        );
        plan.targets.push(ScanTargetBackend {
            url: url.clone(),
            backend,
            selection_reason: reason,
        });
        changed = true;
    }
    if changed {
        plan.refresh_requirements(native_eligible);
        // §5.2: the frozen matrix is a durable record, not a suggestion.
        write_scan_backend_plan(db_path, &plan)?;
    }
    Ok(plan)
}

/// The ONE backend-selection function. Both task-creating entries call it, and the
/// answer depends on the **current attempt's** execution mode (Phase 2 §2.2):
///
/// * `initial` / `fresh` — today's settings decide, and the previous attempt's
///   pinned plan is deliberately ignored, so switching policy and re-running works;
/// * `resume` — only the parent attempt's frozen plan may decide;
/// * an attempt that already recorded its own plan — that plan decides, so a
///   settings edit can never switch a running attempt.
fn agent_select_backend(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    url: &str,
    settings: &JsonValue,
    native_eligible: bool,
) -> AgentBackendKind {
    agent_backend_choice(db_path, scan_id, attempt_number, url, settings, native_eligible).0
}

/// Same decision, plus the reason the matrix records for the UI and the logs.
fn agent_backend_choice(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    url: &str,
    _settings: &JsonValue,
    _native_eligible: bool,
) -> (AgentBackendKind, String) {
    // A target that already appears in this attempt's frozen matrix never changes.
    let rejected = || (AgentBackendKind::LegacyRemoved,
        "backend_plan_rejected: 已保存的执行后端不可验证，不得默认为 Native".to_string());
    match read_scan_backend_plan(db_path, scan_id, attempt_number) {
        Ok(Some(plan)) => {
            if let Some(target) = plan.backend_of(url) {
                // A matrix cannot overrule a conflicting or unreadable target row.
                return match persisted_attempt_backend(db_path, scan_id, attempt_number, url) {
                    Ok(None) => (target.backend, target.selection_reason),
                    Ok(Some(backend)) if backend == target.backend => (backend, target.selection_reason),
                    _ => rejected(),
                };
            }
        }
        Err(_) => return rejected(),
        Ok(None) => {},
    }
    let native_default = || {
        (
            AgentBackendKind::Native,
            format!("attempt {attempt_number} 使用唯一可执行的 native 后端"),
        )
    };
    let lineage = agent_attempt_lineage(db_path, scan_id, attempt_number);
    let inherited_from = if lineage.resume_kind == AgentResumeKind::ContinueIncomplete {
        Some(lineage.parent_attempt_number.unwrap_or(attempt_number))
    } else if match persisted_attempt_backend(db_path, scan_id, attempt_number, url) {
        Ok(backend) => backend.is_some(),
        Err(_) => return rejected(),
    } {
        // This very attempt already started on a backend; keep it.
        Some(attempt_number)
    } else {
        None
    };
    let Some(attempt) = inherited_from else {
        return native_default();
    };
    match persisted_attempt_backend(db_path, scan_id, attempt, url) {
        // The attempt (or its parent, for a continuation) froze this backend.
        Ok(Some(backend)) => (
            backend,
            format!("继承 attempt {attempt} 的冻结后端（{}）", backend.as_str()),
        ),
        // Missing/corrupt parent records never authorize a fresh Native start.
        Ok(None) | Err(_) => rejected(),
    }
}

/// `execution_mode` of the current attempt: only a resumed attempt may pick up
/// the unfinished queue of its own previous run.
fn agent_attempt_is_resume(db_path: &Path, scan_id: &str, attempt_number: i64) -> bool {
    let Ok(connection) = db::open(db_path) else {
        return false;
    };
    connection
        .query_row(
            "SELECT execution_mode FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
            params![scan_id, attempt_number],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_default()
        == "resume"
}

fn agent_attempt_number(db_path: &Path, scan_id: &str) -> i64 {
    let Ok(connection) = db::open(db_path) else {
        return 1;
    };
    connection
        .query_row(
            "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
            [scan_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(1)
        .max(1)
}

#[allow(clippy::too_many_arguments)]
fn agent_build_run_context_with_tool_configuration(
    db_path: &Path,
    scan_id: &str,
    attempt_number: i64,
    prepared: &PreparedFrontendTarget,
    environment: &ModelRuntimeEnv,
    adaptive: &AgentBudgetSettings,
    backend: AgentBackendKind,
    log_path: &Path,
    tool_configuration: Option<&crate::tool_supply::ToolAdministratorConfiguration>,
) -> AgentRunContext {
    let evidence_path = prepared.target_dir.join("frontend-evidence.json");
    let evidence = json(
        fs::read_to_string(&evidence_path)
            .unwrap_or_else(|_| "{\"error\":\"frontend_evidence_unavailable\"}".into()),
    );
    let capabilities = json(
        fs::read_to_string(prepared.target_dir.join(SRC_ASSURANCE_ADAPTER_NAME))
            .unwrap_or_else(|_| "{\"schemaVersion\":0}".into()),
    );
    let identities = match db::open(db_path) {
        Ok(connection) => scan_identity_keys(&connection, scan_id)
            .unwrap_or_default()
            .into_iter()
            .map(|key| agent_identity_from_key(&key))
            .collect(),
        Err(_) => vec![AgentIdentity::anonymous()],
    };
    let mut identities = if identities.is_empty() {
        vec![AgentIdentity::anonymous()]
    } else {
        identities
    };
    // A captured account may compare against a credential-free control on
    // the same frozen Web target. This is not another captured account and
    // does not alter the task's authenticated session policy or identity mode.
    if !identities.iter().any(|identity| identity.anonymous) {
        identities.push(AgentIdentity::anonymous());
    }
    // A continuation must run on the parent's frozen plan; only a fresh attempt
    // reads today's settings into a new one (§3.5).
    let plan_and_rejection = agent_frozen_plan_with_tool_configuration(
        db_path,
        scan_id,
        &prepared.route.url,
        &prepared.route,
        environment,
        adaptive,
        backend,
        attempt_number,
        tool_configuration,
    )
    .map(|plan| (plan, None)).unwrap_or_else(|rejection| {
        // Nothing is executed in this case: the loop reports the rejection as the
        // attempt's terminal state, and the parent's own plan row stays intact.
        append_runner_log(
            log_path,
            &format!("{}；{}", AGENT_STOP_RESUME_INCOMPATIBLE, rejection.message()),
        );
        (
            build_agent_execution_plan(adaptive, &prepared.route, environment, backend, db_path, scan_id)
                .with_attempt(attempt_number),
            Some(rejection),
        )
    });
    let (plan, plan_rejection) = plan_and_rejection;
    AgentRunContext {
        tool_configuration: tool_configuration.cloned(),
        supervision: None,
        external_surface: true,
        scan_id: scan_id.to_string(),
        attempt_number,
        target_url: prepared.route.url.clone(),
        target_dir: prepared.target_dir.clone(),
        db_path: db_path.to_path_buf(),
        #[cfg(test)]
        route: prepared.route.clone(),
        execution_plan: plan,
        evidence,
        capabilities,
        identities,
        proxy: prepared.proxy.clone(),
        log_path: log_path.to_path_buf(),
        environment: environment.clone(),
        resume: agent_attempt_is_resume(db_path, scan_id, attempt_number),
        browser: prepared.browser.clone(),
        plan_rejection,
        // Filled in by the orchestrator once the run row is registered.
        run: None,
        run_budget: None,
    }
}

/// `scan_identity_keys` yields ordered `session:{id}:{name}` / `anonymous` keys.
fn agent_identity_from_key(key: &str) -> AgentIdentity {
    let mut parts = key.split(':');
    match (parts.next(), parts.next()) {
        (Some("session"), Some(session_id)) if !session_id.is_empty() => {
            AgentIdentity::scoped(session_id)
        }
        _ => AgentIdentity::anonymous(),
    }
}

/// Shared per-target entry used by every web pipeline.
struct AgentTargetExecution<'a> {
    db_path: &'a Path,
    scan_id: &'a str,
    attempt_number: i64,
    settings: &'a JsonValue,
    environment: &'a ModelRuntimeEnv,
    adaptive: &'a AgentBudgetSettings,
    log_path: &'a Path,
}

#[cfg(test)]
fn run_agent_target(
    prepared: &PreparedFrontendTarget,
    execution: AgentTargetExecution<'_>,
) -> Result<OwnedAgentTargetOutcome, String> {
    run_agent_target_with_tool_configuration(prepared, execution, None)
}

fn run_agent_target_with_tool_configuration(
    prepared: &PreparedFrontendTarget,
    execution: AgentTargetExecution<'_>,
    tool_configuration: Option<&crate::tool_supply::ToolAdministratorConfiguration>,
) -> Result<OwnedAgentTargetOutcome, String> {
    let invocation = claim_native_invocation(
        execution.db_path, execution.scan_id, execution.attempt_number,
        "target", &prepared.route.url,
    )?;
    let connection = db::open(execution.db_path)?;
    if !native_source_attempt_active(&connection, execution.scan_id, execution.attempt_number) {
        return Err("native_attempt_stopped_or_replaced".into());
    }
    let terminal: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 \
         AND target_url=?3 AND role='coordinator' AND status IN ('terminal','legacy_backend_removed'))",
        params![execution.scan_id, execution.attempt_number, prepared.route.url], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if terminal { return Err("agent_target_already_terminal".into()); }
    drop(connection);
    let mut original_terminal = OriginalAgentTerminalIdentity::pending(
        execution.db_path, execution.scan_id, execution.attempt_number, &prepared.route.url,
    );
    let log_path = execution.log_path.to_path_buf();
    let outcome = run_owned_agent_target(prepared, execution, &invocation, &mut original_terminal, tool_configuration);
    Ok(OwnedAgentTargetOutcome {
        outcome, original_terminal, log_path, _invocation: invocation,
    })
}

fn run_owned_agent_target(
    prepared: &PreparedFrontendTarget,
    execution: AgentTargetExecution<'_>,
    invocation: &NativeInvocationOwner,
    original_terminal: &mut OriginalAgentTerminalIdentity,
    tool_configuration: Option<&crate::tool_supply::ToolAdministratorConfiguration>,
) -> AgentTargetOutcome {
    let AgentTargetExecution {
        db_path,
        scan_id,
        attempt_number,
        settings,
        environment,
        adaptive,
        log_path,
    } = execution;
    if !native_web_attempt_active(db_path, scan_id, attempt_number) {
        return AgentTargetOutcome::Cancelled;
    }
    let backend = agent_select_backend(
        db_path,
        scan_id,
        attempt_number,
        &prepared.route.url,
        settings,
        true,
    );
    if backend != AgentBackendKind::Native {
        return AgentTargetOutcome::Failed(AgentStop::new(
            AGENT_STOP_CONFIGURATION,
            "backend_retired: 执行后端缺失、不受支持或无法验证；不能沿用旧计划或自动转换为 Native",
        ));
    }
    // Native capabilities are materialized directly beside the target evidence.
    // Keep the receiver alive for the whole model loop; dropping it earlier would
    // advertise callback URLs that can no longer accept evidence.
    let _assurance_receiver = match stage_builtin_src_assurance(
        &prepared.route.url,
        &prepared.target_dir,
    ) {
        Ok(receiver) => Some(receiver),
        Err(error) => {
            append_runner_log(
                log_path,
                &format!("原生 SRC/OAST 能力不可用，继续执行不依赖该能力的合同：{error}"),
            );
            None
        }
    };
    let mut context = agent_build_run_context_with_tool_configuration(
        db_path,
        scan_id,
        attempt_number,
        prepared,
        environment,
        adaptive,
        backend,
        log_path,
        tool_configuration,
    );
    if let Some(rejection)=context.plan_rejection.as_ref() {return rejection_outcome(&context,rejection);}
    update_target_route(db_path, scan_id, &prepared.route, "scanning");
    // §11: the run row exists with its plan and budgets from the moment the
    // backend starts, so a pause, crash or cancel leaves something to recover.
    let run = runtime_open_run_for_attempt(db_path, scan_id, &prepared.route, attempt_number);
    context.run = run;
    if let Err(error) = original_terminal.bind_root(&context) {
        return AgentTargetOutcome::persistence_failure(error);
    }
    if context.evidence.get("error").is_some() {
        return AgentTargetOutcome::failed(
            "原生 Agent 缺少本地证据包，已停止；不会切换到任何已停用后端重跑",
        );
    }
    let mode=match native_frozen_web_root_mode(&context) {
        Ok(mode)=>mode,Err(error)=>return AgentTargetOutcome::persistence_failure(format!("web_mode_execution_denied:{error}")),
    };
    if let Err(error)=db::open(&context.db_path).and_then(|db|native_web_original_finance_on(&db,&context)) {
        return AgentTargetOutcome::persistence_failure(format!("web_original_finance_denied:{error}"));
    }
    if mode==crate::agent_runtime::web_mode::WebMode::Single {
        if let Err(error)=bind_agent_evidence_location(&context) {return AgentTargetOutcome::persistence_failure(error);}
        append_runner_log(&context.log_path,"single native execution with original frozen Root budget");
        return NativeAgentBackend.execute(&context);
    }
    let parent_owner=match context.run.as_ref().ok_or_else(||"multi_agent_root_run_missing".to_string()).and_then(|run|
        crate::agent_runtime::multi_agent::parent_invocation_owner::ParentInvocationOwner::claim(
            &context.db_path,&context.scan_id,context.attempt_number,&run.run_id)) {
        Ok(owner)=>owner,Err(error)=>return AgentTargetOutcome::persistence_failure(error),
    };
    // This is the first owner of this target after any preceding invocation
    // released its OS lock. Reconcile only abandoned provider dispatches;
    // the normal in-flight call is never touched, and no HTTP is replayed.
    let proposal_reentry = (|| {
        let run = context.run.as_ref().ok_or("directive_reentry_root_missing")?;
        let connection = db::open(&context.db_path)?;
        let lease = crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
            &connection, &context.scan_id, context.attempt_number, &context.target_url,
            &run.run_id, 600,
        )?;
        reconcile_abandoned_human_proposals(&connection, &lease, invocation)
    })();
    if let Err(error) = proposal_reentry {
        return AgentTargetOutcome::persistence_failure(format!(
            "directive_reentry_reconciliation_failed:{error}"
        ));
    }
    append_runner_log(
        &context.log_path,
        &format!(
            "{} backend selected for {} · plan {}",
            NativeAgentBackend.kind().as_str(),
            context.target_url,
            context.execution_plan.hash()
        ),
    );
    // §19: a native attempt never restarts the same target on a retired backend,
    // so a retry cannot double-spend. The reducer receives exactly what the
    // target-touching child reports.
    let mut session = match bind_agent_evidence_location(&context)
        .and_then(|()| multi_agent_prepare_owned(&mut context,parent_owner)) {
        Ok(session) => session,
        Err(error) if error == "homogeneous_scan_legacy_bootstrap_stepped_aside" => {
            return run_homogeneous_agent_target(&mut context);
        }
        Err(error) => {
            let failed = multi_agent_bootstrap_outcome(&error);
            let cleanup_lease = (|| {
                let run = context.run.as_ref().ok_or("bootstrap_coordinator_run_missing")?;
                let connection = db::open(&context.db_path)?;
                crate::agent_runtime::multi_agent::lease::acquire_coordinator_lease(
                    &connection,
                    &context.scan_id,
                    context.attempt_number,
                    &context.target_url,
                    &run.run_id,
                    600,
                )
            })();
            return match cleanup_lease {
                Ok(lease) => finalize_agent_target(&context, &lease, failed),
                Err(error) => coordinator_finalize_failure(&context, &failed, &error),
            };
        }
    };
    append_runner_log(
        &context.log_path,
        &format!(
            "multi-agent active: coordinator={} mapper={} executor={} assignment={}",
            session.lease.root_run_id,
            session.mapper.run_id,
            session.executor.run_id,
            session.executor.assignment_id
        ),
    );
    let outcome = NativeAgentBackend.execute(&context);
    if let Err(error) = multi_agent_finish_execution(&context, &mut session, &outcome) {
        let failed = executor_settlement_outcome(&outcome,&error);
        return finalize_agent_target(&context, &session.lease, failed);
    }
    if matches!(
        outcome,
        AgentTargetOutcome::Failed(_)
            | AgentTargetOutcome::ResumeIncompatible(_)
            | AgentTargetOutcome::Cancelled
    ) || outcome.terminal_code()==terminal_code::REQUEST_RECONCILIATION_REQUIRED {
        return finalize_agent_target(&context, &session.lease, outcome);
    }
    if let Err(error) = multi_agent_authorization(&context, &mut session) {
        let failed = AgentTargetOutcome::failed(format!("authorization_control_failed:{error}"));
        return finalize_agent_target(&context, &session.lease, failed);
    }
    let reviewed = multi_agent_review(&context, &mut session, outcome);
    finalize_agent_target(&context, &session.lease, reviewed)
}

include!("agent_backend_homogeneous.rs");
include!("agent_homogeneous_workers.rs");

/// Backend selection, the per-target orchestrator and the pipeline tally.
///
/// Every executor reports an `AgentTargetOutcome`; only this file turns it into
/// persisted target status and the scan summary, so execution paths cannot grow
/// separate "部分完成" wording.
#[derive(Clone, Default)]
struct AgentPipelineTally {
    projected_roots: HashSet<String>,
    completed: usize,
    /// Targets that closed inside their bounds **with named coverage gaps** (§10).
    /// Counting them as `completed` is what let a scan claim a clean finish while
    /// whole families were never verified.
    completed_with_gaps: usize,
    partial: usize,
    skipped: usize,
    manual_review: usize,
    limited: usize,
    failed: usize,
    failure_details: Vec<String>,
}

impl AgentPipelineTally {
    fn counted(&self) -> usize {
        self.completed
            + self.completed_with_gaps
            + self.partial
            + self.skipped
            + self.manual_review
            + self.limited
            + self.failed
    }

    fn deferred(&self, total: usize) -> usize {
        total.saturating_sub(self.counted())
    }

    fn push_detail(&mut self, detail: String) {
        if self.failure_details.len() < 5 {
            self.failure_details.push(detail);
        }
    }

    /// The single scan-level terminal state writer.
    fn collapse_checkpoint_semicolons(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        let mut prev_semi = false;
        for ch in value.chars() {
            if ch == '；' {
                if prev_semi {
                    continue;
                }
                prev_semi = true;
                out.push(ch);
            } else {
                prev_semi = false;
                out.push(ch);
            }
        }
        out
    }

    #[cfg(test)]
    fn finalize(&self, db_path: &Path, scan_id: &str, total: usize) {
        let (status, summary) = self.terminal_summary(total);
        sentinel_scan_update(db_path, scan_id, status, &summary);
    }

    fn terminal_summary(&self, total: usize) -> (&'static str, String) {
        let deferred = self.deferred(total);
        let gaps = self.completed_with_gaps;
        let failure_suffix = {
            let cleaned = self
                .failure_details
                .iter()
                .map(|detail| detail.trim().trim_start_matches('；').trim())
                .filter(|detail| !detail.is_empty())
                .map(|detail| detail.to_string())
                .collect::<Vec<_>>();
            if cleaned.is_empty() {
                String::new()
            } else {
                format!("；报错细节：{}", cleaned.join("；"))
            }
        };
        let uninterrupted = self.failed == 0 && self.limited == 0 && self.partial == 0 && deferred == 0;
        if uninterrupted && gaps == 0 {
            (
                "completed",
                Self::collapse_checkpoint_semicolons(&format!(
                    "本轮执行完成：自动验证 {}，确定性侦察收口 {}，复杂前端自动收口 {}，无异常中断",
                    self.completed, self.skipped, self.manual_review
                )),
            )
        } else if uninterrupted {
            // §10: the run answered every target, but some of them closed with
            // declared holes. That is finished work with visible gaps, and the
            // wording has to say so instead of "无异常中断".
            (
                "completed_with_gaps",
                Self::collapse_checkpoint_semicolons(&format!(
                    "本轮执行完成但存在覆盖缺口：自动验证 {}，带覆盖缺口完成 {}，确定性侦察收口 {}，复杂前端自动收口 {}；缺口未计入无异常完成{failure_suffix}",
                    self.completed, gaps, self.skipped, self.manual_review
                )),
            )
        } else if self.completed + gaps + self.partial + self.skipped + self.manual_review > 0 {
            let gap_note = if gaps > 0 {
                format!("，带覆盖缺口完成 {gaps}")
            } else {
                String::new()
            };
            let summary = if self.failed == 0 && self.limited == 0 && deferred == 0 {
                format!(
                    "本轮未完整结束：自动验证 {}{gap_note}，待补充验证 {}，确定性侦察收口 {}，复杂前端自动收口 {}；无执行失败，待补充与缺口项未计入自动验证完成{failure_suffix}",
                    self.completed, self.partial, self.skipped, self.manual_review
                )
            } else {
                format!(
                    "本轮执行异常：自动验证 {}{gap_note}，待补充验证 {}，确定性侦察收口 {}，复杂前端自动收口 {}，熔断 {}，执行失败 {}，未处理 {deferred}{failure_suffix}",
                    self.completed,
                    self.partial,
                    self.skipped,
                    self.manual_review,
                    self.limited,
                    self.failed
                )
            };
            ("partial", Self::collapse_checkpoint_semicolons(&summary))
        } else {
            (
                "failed",
                Self::collapse_checkpoint_semicolons(&format!(
                    "流水线没有有效完成目标：可重试无进展 {}，失败 {}，未处理 {deferred}{failure_suffix}",
                    self.limited, self.failed
                )),
            )
        }
    }
}

/// Route modes that never reach a backend; recorded here so both pipelines
/// share one implementation.
fn record_agent_routing_skip(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
    position: usize,
    total: usize,
    tally: &mut AgentPipelineTally,
) -> bool {
    if route.mode == "manual_review" {
        tally.manual_review += 1;
        update_target_route(db_path, scan_id, route, "manual_review");
        sentinel_scan_update(
            db_path,
            scan_id,
            "scanning",
            &format!(
                "目标 {position}/{total} · 复杂前端已完成本地证据提取，转人工分析 · {}",
                route.reason_text()
            ),
        );
        return false;
    }
    if route.mode == "skip" {
        tally.skipped += 1;
        update_target_route(db_path, scan_id, route, "recon_only");
        sentinel_scan_update(
            db_path,
            scan_id,
            "scanning",
            &format!(
                "目标 {position}/{total} · 前端价值 {} 分，跳过模型调查 · {}",
                route.score,
                route.reason_text()
            ),
        );
        return false;
    }
    true
}

/// Facts this attempt owes the runtime store, independent of the outcome: the
/// frozen plan, its hashes and its four budget ceilings. §11 requires the plan
/// budgets to land on the run row, otherwise `agent_runs` reports a 0 ceiling and
/// the usage columns cannot be read as a ratio.
#[cfg(test)]
fn runtime_report(db_path: &Path, scan_id: &str, route: &FrontendRoute) -> crate::agent_runtime::runtime_adapter::BackendReport {
    let attempt_number: i64 = db::open(db_path)
        .ok()
        .and_then(|connection| {
            connection
                .query_row(
                    "SELECT attempt_count FROM sentinel_scans WHERE id=?1",
                    [scan_id],
                    |row| row.get::<_, i64>(0),
                )
                .ok()
        })
        .unwrap_or(1)
        .max(1);
    runtime_report_for_attempt(db_path,scan_id,route,attempt_number)
}

/// Register the run before the backend starts, so a target that crashes, is
/// paused or is cancelled still has a `running` row with its plan and budgets.
/// Re-opening an unfinished run reuses the same row instead of forking history,
/// and hands back the ledger the loop mirrors its live facts into.
fn runtime_open_run_for_attempt(
    db_path: &Path,
    scan_id: &str,
    route: &FrontendRoute,
    attempt_number: i64,
) -> Option<AgentRunLedger> {
    use crate::agent_runtime::runtime_adapter;
    let Ok(connection) = db::open(db_path) else {
        return None;
    };
    let report = runtime_report_for_attempt(db_path, scan_id, route, attempt_number);
    runtime_adapter::open_run(&connection, &report)
        .ok()
        .map(|run_id| AgentRunLedger {
            db_path: db_path.to_path_buf(),
            run_id,
        })
}

/// Phase 0 wiring: the runtime reduces the terminal state into
/// `agent_runs`/`agent_events`. Legacy status columns are written elsewhere and
/// unchanged, so current scan behaviour stays as it was.
// Compatibility for existing production-entry tests; the live pipeline consumes
// checked errors below and never counts failed publication as completion.
#[cfg(test)]
fn record_runtime_terminal_facts(db_path: &Path, scan_id: &str, route: &FrontendRoute, outcome: &AgentTargetOutcome) {
    let _ = record_runtime_terminal_facts_checked(db_path, scan_id, route, outcome);
}
include!("agent_original_terminal_identity.rs");
include!("agent_terminal_report.rs");
include!("agent_terminal_projection.rs");

// Apply the original outcome; stop on cancellation or projection failure.
include!("agent_terminal_legacy_consumer.rs");
include!("agent_terminal_legacy_writer.rs");
include!("agent_terminal_legacy_fuse.rs");
