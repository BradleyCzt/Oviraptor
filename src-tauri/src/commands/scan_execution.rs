fn claim_web_pipeline_branch(
    db_path: &Path, scan_id: &str, attempt_number: i64, work_dir: &Path, worker: &Path,
    admission: &mut Option<WebDispatchAdmission>,
) -> Result<NativeBranchGuard,String> {
    if let Some(mut guard) = admission.as_mut().and_then(|frozen| frozen.claimed_guard.take()) {
        if guard.db_path != db_path || guard.scan_id != scan_id || guard.attempt != attempt_number || guard.branch != "web" {
            guard.disarm();
            return Err("web_recovery_owned_guard_mismatch".into());
        }
        Ok(guard)
    } else if let Some(frozen) = admission {
        NativeBranchGuard::claim_with_preflight(db_path,scan_id,attempt_number,"web", |connection| {
            verify_live_web_dispatch_binding_in(connection,scan_id,attempt_number,work_dir,worker,&frozen.home)
        })
    } else { claim_workbench_pipeline_branch(db_path,scan_id,attempt_number,"web") }
}

#[allow(clippy::too_many_arguments)]
fn launch_sentinel_url_pipeline(
    db_path: PathBuf,
    scan_id: String,
    attempt_number: i64,
    worker: PathBuf,
    work_dir: PathBuf,
    targets: Vec<(String, String)>,
    proxies: Vec<(String, String)>,
    no_proxy: String,
    model_environment: ModelRuntimeEnv,
    runtime_path: OsString,
    adaptive: AgentBudgetSettings,
    packet_budget: usize,
    auth_session_path: Option<PathBuf>,
    admission: Option<WebDispatchAdmission>,
) -> Result<(), String> {
    launch_sentinel_url_pipeline_with_tool_configuration(db_path, scan_id, attempt_number, worker, work_dir, targets, proxies, no_proxy, model_environment, runtime_path, adaptive, packet_budget, auth_session_path, admission, None)
}

#[allow(clippy::too_many_arguments)]
fn launch_sentinel_url_pipeline_with_tool_configuration(
    db_path: PathBuf,
    scan_id: String,
    attempt_number: i64,
    worker: PathBuf,
    work_dir: PathBuf,
    targets: Vec<(String, String)>,
    proxies: Vec<(String, String)>,
    no_proxy: String,
    model_environment: ModelRuntimeEnv,
    runtime_path: OsString,
    adaptive: AgentBudgetSettings,
    packet_budget: usize,
    auth_session_path: Option<PathBuf>,
    mut admission: Option<WebDispatchAdmission>,
    tool_configuration: Option<crate::tool_supply::ToolAdministratorConfiguration>,
) -> Result<(), String> {
    thread::Builder::new().name("native-web-pipeline".into()).spawn(move || {
        let claim = claim_web_pipeline_branch(&db_path,&scan_id,attempt_number,&work_dir,&worker,&mut admission);
        let mut branch_guard = match claim {
            Ok(guard) => guard,
            Err(error) => {
                append_runner_log(&work_dir.join("oviraptor-runner.log"), &format!("Web 分支未取得执行权，不启动；未领取的工作台分支按准入结果登记，其他状态不改写：{error}"));
                return;
            }
        };
        let (settings, recon_config) = match admission {
            Some(frozen) => (frozen.settings, frozen.recon_config),
            None => (db::open(&db_path).map(|connection| sentinel_settings(&connection))
                .unwrap_or_else(|_| serde_json::json!({})), frontend_recon_config(&worker)),
        };
        if !sentinel_scan_is_active(&db_path, &scan_id) {
            return;
        }
        let log_path = work_dir.join("oviraptor-runner.log");
        let total_targets = targets.len();
        let model_policy = local_model_runtime_policy(&model_environment);
        let packet_budget = if model_environment.deployment == "local" {
            packet_budget.min(model_policy.frontend_packet_budget_bytes)
        } else {
            packet_budget
        };
        if model_environment.deployment == "local" {
            match apply_omlx_local_resource_policy(&model_environment) {
                Ok(Some(summary)) => append_runner_log(&log_path, &summary),
                Ok(None) => append_runner_log(
                    &log_path,
                    "local model resource policy: non-oMLX endpoint; Oviraptor request limits still apply",
                ),
                Err(error) => append_runner_log(
                    &log_path,
                    &format!("oMLX resource policy persisted with live-apply warning: {error}"),
                ),
            }
        }
        append_runner_log(
            &log_path,
            &format!(
                "frontend packet policy: {} KB total budget; evidence, parameters, sensitive clues, and code slices are priority-compacted",
                packet_budget / 1024
            ),
        );
        let producer = launch_frontend_recon_producer(
            db_path.clone(),
            scan_id.clone(),
            attempt_number,
            worker,
            work_dir.clone(),
            targets,
            proxies,
            no_proxy.clone(),
            model_environment.full_power,
            model_environment.deployment == "local",
            runtime_path.clone(),
            adaptive.clone(),
            packet_budget,
            log_path.clone(),
            auth_session_path,
            recon_config,
        );
        let (receiver, frontend_ack) = match producer {
            Ok(producer) => producer,
            Err(error) => {
                append_runner_log(&log_path,&format!("前端线程未取得执行权：{error}"));
                return;
            }
        };
        // Local context/memory admission errors retain the completed frontend
        // evidence and remain retryable after the automatic resource policy is
        // adjusted. They are partial results, not target execution failures.
        append_runner_log(&log_path, "agent backend: native (retired backends are read-only)");
        let mut tally = AgentPipelineTally::default();
        for item in receiver {
            if !native_web_attempt_current(&db_path, &scan_id, attempt_number) { return; }
            if sentinel_scan_pause_requested(&db_path, &scan_id) {
                append_runner_log(
                    &log_path,
                    "pipeline pause checkpoint reached; waiting for actual worker exit",
                );
                append_runner_log(
                    &log_path,
                    "pause requested; waiting for owned workers to exit; no automatic replay",
                );
                return;
            }
            if !sentinel_scan_is_active(&db_path, &scan_id) {
                return;
            }
            let prepared = match item {
                FrontendQueueItem::Ready(prepared) => prepared,
                FrontendQueueItem::Limited {
                    position,
                    url,
                    reason,
                } => {
                    tally.limited += 1;
                    update_batch_targets(&db_path, &scan_id, std::slice::from_ref(&url), "limited");
                    sentinel_scan_update(
                        &db_path,
                        &scan_id,
                        "scanning",
                        &format!("目标 {position}/{total_targets} · 自动停止 · {reason}"),
                    );
                    continue;
                }
                FrontendQueueItem::Failed {
                    position,
                    url,
                    reason,
                } => {
                    tally.failed += 1;
                    // A recon timeout is unfinished work, not a closed failure.
                    // Keep it resumable so 继续 only reruns this URL.
                    let resumable = reason.contains("硬上限") || reason.contains("时限") || reason.contains("晚到结果");
                    let stored_status = if resumable { "paused" } else { "failed" };
                    if let Ok(connection) = db::open(&db_path) {
                        let _ = connection.execute(
                            "UPDATE sentinel_targets SET status=?1,routing_reason=?2,updated_at=datetime('now','localtime') WHERE scan_id=?3 AND url=?4",
                            params![stored_status, reason, scan_id, url],
                        );
                    }
                    tally.push_detail(format!("{url}：{reason}"));
                    sentinel_scan_update(
                        &db_path,
                        &scan_id,
                        "scanning",
                        &format!("目标 {position}/{total_targets} · 前端探测失败 · {reason}"),
                    );
                    continue;
                }
            };
            // In local-model mode this acknowledges the prepared URL only after
            // its Native Agent work (or routing skip) has finished. The producer then
            // starts the next browser/AST pass without competing for CPU.
            let _frontend_ack = FrontendQueueAck(frontend_ack.as_ref());
            let position = prepared.position;
            let route = &prepared.route;
            if !record_agent_routing_skip(
                &db_path,
                &scan_id,
                route,
                position,
                total_targets,
                &mut tally,
            ) {
                continue;
            }
            // Plan selection/writes belong inside target admission. A repeated
            // target must not change the winner's frozen plan before it owns it.
            let dispatched = run_agent_target_with_tool_configuration(
                &prepared,
                AgentTargetExecution {
                    db_path: &db_path,
                    scan_id: &scan_id,
                    attempt_number,
                    settings: &settings,
                    environment: &model_environment,
                    adaptive: &adaptive,
                    log_path: &log_path,
                },
                tool_configuration.as_ref(),
            );
            let dispatched = match dispatched {
                Ok(dispatched) => dispatched,
                Err(error) => {
                    append_runner_log(&log_path, &format!("目标未取得执行权，不改写已有执行或终态，需检查当前执行记录：{error}"));
                    branch_guard.disarm();
                    return;
                }
            };
            if !native_web_attempt_current(&db_path, &scan_id, attempt_number) { return; }
            if !record_owned_agent_target_outcome(&db_path, &scan_id, route, &dispatched, &mut tally) {
                return;
            }
            if sentinel_scan_pause_requested(&db_path, &scan_id) {
                return;
            }
        }
        if !native_web_attempt_current(&db_path, &scan_id, attempt_number) { return; }
        if sentinel_scan_pause_requested(&db_path, &scan_id) {
            return;
        }
        let (status, summary) = tally.terminal_summary(total_targets);
        if let Err(error) = finish_native_branch(&db_path, &scan_id, attempt_number, "web", status, &summary, &json!({"targetCount": total_targets})) {
            append_runner_log(&log_path, &format!("Web 分支终态未写入：{error}"));
        }
        if scan_supports_automatic_learning(&db_path, &scan_id) {
            schedule_learning_candidate(
                db_path.clone(),
                scan_id.clone(),
                model_environment.clone(),
            );
        }
    }).map(|_| ()).map_err(|_| "web_dispatch_thread_start_failed_no_automatic_retry".into())
}
