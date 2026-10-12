struct PreparedFrontendTarget {
    position: usize,
    route: FrontendRoute,
    target_dir: PathBuf,
    proxy: Option<String>,
    /// The bundled CDP runtime probe plus its process environment, so an
    /// interactive tool can drive a real browser session instead of only
    /// replaying HTTP. `None` when this build has no runtime helper.
    browser: Option<AgentBrowserRuntime>,
}
enum FrontendQueueItem {
    Ready(PreparedFrontendTarget),
    Limited {
        position: usize,
        url: String,
        reason: String,
    },
    Failed {
        position: usize,
        url: String,
        reason: String,
    },
}

struct FrontendQueueAck<'a>(Option<&'a mpsc::SyncSender<()>>);

impl Drop for FrontendQueueAck<'_> {
    fn drop(&mut self) {
        if let Some(sender) = self.0 {
            let _ = sender.send(());
        }
    }
}

fn cached_frontend_recon(db_path: &Path, scan_id: &str, url: &str) -> Option<JsonValue> {
    let connection = db::open(db_path).ok()?;
    let raw = connection
        .query_row(
            "SELECT raw_json FROM sentinel_checkpoints WHERE scan_id=?1 AND url=?2 AND stage='frontend_recon'",
            params![scan_id, url],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .ok()
        .flatten()?;
    let target = serde_json::from_str::<JsonValue>(&raw).ok()?;
    // Version 4 includes native collection, independent A/B replay, the stricter sensitive-value
    // semantic pass and the replayable request/response baseline used by the
    // AI packet. Reusing an older checkpoint would preserve both the old
    // identity UI defects and false sensitive/API routing evidence.
    if target
        .get("analysisSummary")
        .and_then(|value| value.get("reconCacheVersion"))
        .and_then(JsonValue::as_i64)
        .unwrap_or(0) < 4
    {
        return None;
    }
    Some(serde_json::json!({"targets":[target]}))
}

#[allow(clippy::too_many_arguments)]
fn launch_frontend_recon_producer(
    db_path: PathBuf,
    scan_id: String,
    attempt_number: i64,
    worker: PathBuf,
    work_dir: PathBuf,
    targets: Vec<(String, String)>,
    proxies: Vec<(String, String)>,
    no_proxy: String,
    full_power: bool,
    serialize_for_local: bool,
    runtime_path: OsString,
    adaptive: AgentBudgetSettings,
    packet_budget: usize,
    log_path: PathBuf,
    auth_session_path: Option<PathBuf>,
    recon_config: FrontendReconConfig,
) -> Result<(
    mpsc::Receiver<FrontendQueueItem>,
    Option<mpsc::SyncSender<()>>,
),String> {
    // This producer performs direct HTTP/JS/CDP before Root publication.
    // Recovery scopes require their dedicated shared-budget consumer first.
    let route_db = rusqlite::Connection::open_with_flags(
        &db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).map_err(|e| e.to_string())?;
    if ordinary_web_recovery_scope_on(&route_db, &scan_id)? {
        return Err("protection_recovery_requires_dedicated_frontend_budget".into());
    }
    drop(route_db);
    let producer_owner = ScanWorkerOwner::claim(&db_path,&scan_id,attempt_number,"frontend_producer","web")?;
    let (sender, receiver) = mpsc::channel();
    let (ack_sender, ack_receiver) = mpsc::sync_channel(0);
    thread::Builder::new().name("native-frontend-producer".into()).spawn(move || {
        let _producer_owner = producer_owner;
        let total = targets.len();
        let queue_root = work_dir.join("url-pipeline");
        let _ = fs::create_dir_all(&queue_root);
        let browser_session_ids = auth_session_path
            .as_ref()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<JsonValue>(&text).ok())
            .map(|document| {
                let mut ids = document
                    .get("sessions")
                    .and_then(JsonValue::as_array)
                    .into_iter()
                    .flatten()
                    .map(|session| value_first(session, &["id"]))
                    .filter(|id| !id.is_empty())
                    .collect::<Vec<_>>();
                let id = value_first(&document, &["id"]);
                if !id.is_empty() {
                    ids.push(id);
                }
                ids.sort();
                ids.dedup();
                ids
            })
            .unwrap_or_default();
        for (offset, (company, url)) in targets.into_iter().enumerate() {
            let position = offset + 1;
            if !native_web_attempt_active(&db_path, &scan_id, attempt_number) {
                break;
            }
            if !browser_session_ids.is_empty() {
                let all_sessions_invalid = db::open(&db_path).ok().is_some_and(|connection| {
                    browser_session_ids.iter().all(|session_id| {
                        connection.query_row(
                            "SELECT status FROM browser_auth_sessions WHERE id=?1",
                            [session_id],
                            |row| row.get::<_, String>(0),
                        ).is_ok_and(|status| matches!(status.as_str(), "invalid" | "expired"))
                    })
                });
                if all_sessions_invalid {
                    let _ = sender.send(FrontendQueueItem::Limited {
                        position,
                        url,
                        reason: "所选浏览器身份均已明确失效；剩余认证探测已停止，请重新登录后在当前任务继续执行".into(),
                    });
                    continue;
                }
            }
            let target_dir = queue_root.join(format!("target-{position:05}"));
            append_runner_log(
                &log_path,
                &format!("frontend target {position}/{total} started: {url}"),
            );
            if let Err(error) = fs::create_dir_all(&target_dir) {
                let _ = sender.send(FrontendQueueItem::Failed {
                    position,
                    url,
                    reason: format!("无法创建前端探测目录：{error}"),
                });
                continue;
            }
            let _ = fs::write(target_dir.join(".oviraptor-scan-id"), &scan_id);
            let target_auth_session_path = auth_session_path.as_ref().and_then(|source| {
                let destination = target_dir.join("auth-session.json");
                fs::copy(source, &destination).ok()?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = fs::set_permissions(&destination, fs::Permissions::from_mode(0o600));
                }
                Some(destination)
            });
            let recon_output = target_dir.join("oviraptor_recon.json");
            let cached = if target_auth_session_path.is_none() {
                cached_frontend_recon(&db_path, &scan_id, &url)
            } else {
                None
            };
            let recon_note = if let Some(recon) = cached {
                if fs::write(
                    &recon_output,
                    serde_json::to_vec_pretty(&recon).unwrap_or_default(),
                )
                .is_err()
                {
                    "已复用前端探测检查点（证据文件写入失败）".to_string()
                } else {
                    "已复用前端探测检查点".to_string()
                }
            } else {
                update_batch_targets(
                    &db_path,
                    &scan_id,
                    std::slice::from_ref(&url),
                    "frontend_recon",
                );
                native_web_attempt_progress(
                    &db_path,
                    &scan_id,
                    attempt_number,
                    &format!("前端探测 {position}/{total} · {url}"),
                );
                let targets_json = target_dir.join("targets.json");
                let payload = serde_json::json!([{"company":company,"url":url}]);
                if let Err(error) = fs::write(
                    &targets_json,
                    serde_json::to_vec_pretty(&payload).unwrap_or_default(),
                ) {
                    let _ = sender.send(FrontendQueueItem::Failed {
                        position,
                        url,
                        reason: format!("无法写入前端探测目标：{error}"),
                    });
                    continue;
                }
                let mut current_recon_published = false;
                let result = (|| -> Result<(), String> {
                    let hard_timeout_seconds =
                        frontend_recon_hard_timeout_seconds(browser_session_ids.len(), &recon_config);
                    let exploration_timeout_seconds = frontend_recon_exploration_timeout_seconds(
                        browser_session_ids.len(),
                        &recon_config,
                    );
                    let proxy = proxies
                        .get(offset % proxies.len().max(1))
                        .map(|item| item.1.as_str());
                    append_runner_log(
                        &log_path,
                        &format!(
                            "frontend target {position}/{total}: Rust native recon started"
                        ),
                    );
                    append_runner_log(
                        &log_path,
                        &format!(
                            "frontend target {position}/{total}: watchdog={}s exploration-per-identity={}s identities={}",
                            hard_timeout_seconds,
                            exploration_timeout_seconds,
                            browser_session_ids.len().max(1)
                        ),
                    );
                    let (native_sender, native_receiver) = mpsc::sync_channel(1);
                    let native_company = company.clone();
                    let native_url = url.clone();
                    // Worker threads only write an attempt-specific staging file.
                    // The producer publishes it after accepting this attempt's result.
                    let staged_output = target_dir.join(format!("recon-staged-{}.json", Uuid::new_v4()));
                    let native_output = staged_output.clone();
                    let control = NativeReconControl::new_logged(Duration::from_secs(hard_timeout_seconds),&db_path,&scan_id,attempt_number,&url)?;
                    let native_control = control.clone();
                    let native_worker = worker.clone();
                    let native_auth = target_auth_session_path.clone();
                    let native_proxy = proxy.map(str::to_string);
                    let native_no_proxy = no_proxy.clone();
                    let native_runtime_path = runtime_path.clone();
                    let recon_owner = ScanWorkerOwner::claim(&db_path,&scan_id,attempt_number,"frontend_recon",&url)?;
                    let native_db_path = db_path.clone();
                    let native_scan_id = scan_id.clone();
                    thread::Builder::new().name("native-frontend-recon".into()).spawn(move || {
                        let _recon_owner = recon_owner;
                        if !native_web_attempt_active(&native_db_path,&native_scan_id,attempt_number) {
                            let _ = native_sender.send(Err("本轮侦察已停止；未启动采集".into()));
                            return;
                        }
                        let result = run_native_frontend_recon(
                            &native_company, &native_url, &native_output, &native_worker,
                            native_auth.as_deref(), recon_config.browser_request_timeout_seconds,
                            exploration_timeout_seconds, if serialize_for_local { "local" } else { "cloud" },
                            native_proxy.as_deref(), &native_no_proxy, &native_runtime_path, &native_control,
                        );
                        let _ = native_sender.send(result);
                    }).map_err(|_| "frontend_recon_thread_start_failed".to_string())?;
                    let started = Instant::now();
                    let mut last_heartbeat = Instant::now() - Duration::from_secs(5);
                    let result = loop {
                        match native_receiver.recv_timeout(Duration::from_millis(500)) {
                            Ok(result) => break result,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break Err("Rust 原生前端侦察线程异常退出".into()),
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if sentinel_scan_pause_requested(&db_path, &scan_id) { break Err("已暂停前端探测".into()); }
                                if !native_web_attempt_active(&db_path, &scan_id, attempt_number) { break Err("任务已停止或执行尝试已替换".into()); }
                                if started.elapsed() >= Duration::from_secs(hard_timeout_seconds) { break Err(format!("单个 URL 前端探测达到 {hard_timeout_seconds} 秒硬上限")); }
                                if last_heartbeat.elapsed() >= Duration::from_secs(5) {
                                    let elapsed = started.elapsed().as_secs();
                                    native_web_attempt_progress(&db_path, &scan_id, attempt_number, &format!("前端与接口侦察 {position}/{total} · 已运行 {elapsed}/{hard_timeout_seconds} 秒 · Rust 原生归并 + CDP 身份隔离采集 · 本阶段新增 Token 0"));
                                    last_heartbeat = Instant::now();
                                }
                            }
                        }
                    };
                    let attempt_alive = native_web_attempt_active(&db_path, &scan_id, attempt_number)
                        && !sentinel_scan_pause_requested(&db_path, &scan_id);
                    let timed_out = control.check().is_err();
                    control.cancel();
                    if !attempt_alive {
                        return Err("本轮侦察已停止，晚到结果未发布到当前任务".into());
                    }
                    // A timeout must not throw away a file this attempt already wrote.
                    // The next stage can use that evidence instead of failing the URL.
                    if staged_output.is_file() {
                        fs::rename(&staged_output, &recon_output)
                            .map_err(|error| format!("发布本轮侦察结果失败：{error}"))?;
                        current_recon_published = true;
                        if timed_out {
                            append_runner_log(
                                &log_path,
                                &format!("frontend target {position}/{total}: 已到时限，已保存当前证据并继续"),
                            );
                        }
                        return Ok(());
                    }
                    if result.is_ok() {
                        return Err("侦察线程结束但缺少本轮结果文件；未复用历史结果".into());
                    }
                    append_runner_log(
                        &log_path,
                        &format!(
                            "frontend target {position}/{total}: 浏览器采集阶段结束（watchdog {hard_timeout_seconds}s）· {}",
                            match &result {
                                Ok(()) => "采集成功".to_string(),
                                Err(error) => format!(
                                    "采集失败：{}",
                                    error.chars().take(1200).collect::<String>()
                                ),
                            }
                        ),
                    );
                    result
                })();
                match result {
                    Ok(()) => "前端探测完成".to_string(),
                    Err(_) if !native_web_attempt_active(&db_path, &scan_id, attempt_number) => break,
                    Err(error) => {
                        // Retain only this accepted attempt's partial evidence;
                        // never load an older canonical file after a failed run.
                        let retained = current_recon_published
                            && fs::read(&recon_output).ok()
                                .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok())
                                .filter(|_| frontend_recon_target(&recon_output, &url).is_some())
                                .is_some_and(|recon| insert_native_frontend_recon(&db_path, &scan_id, attempt_number, &recon).is_ok());
                        append_runner_log(
                            &log_path,
                            &format!(
                                "frontend target {position}/{total}: 证据整理阶段：{}",
                                if retained {
                                    "部分侦察证据已保存，任务不标记为采集成功"
                                } else {
                                    "本轮证据未通过当前 URL 校验，未保存"
                                }
                            ),
                        );
                        let _ = sender.send(FrontendQueueItem::Failed {
                            position,
                            url,
                            reason: if retained { format!("{error}；本轮已采集证据已保存，未标记为完整成功") } else { error },
                        });
                        continue;
                    }
                }
            };
            let recon = fs::read(&recon_output)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<JsonValue>(&bytes).ok());
            let Some(recon) =
                recon.filter(|_| frontend_recon_target(&recon_output, &url).is_some())
            else {
                let _ = sender.send(FrontendQueueItem::Failed {
                    position,
                    url,
                    reason: "前端探测结果缺少当前 URL，未启动自动验证".into(),
                });
                continue;
            };
            if insert_native_frontend_recon(&db_path, &scan_id, attempt_number, &recon).is_err() { break; }
            let validation = recon
                .get("targets")
                .and_then(JsonValue::as_array)
                .and_then(|items| {
                    items.iter().find(|target| {
                        target.get("url").and_then(JsonValue::as_str) == Some(url.as_str())
                            || target.get("finalUrl").and_then(JsonValue::as_str)
                                == Some(url.as_str())
                    })
                })
                .and_then(|target| target.get("authSessionValidation"));
            if validation
                .and_then(|value| value.get("wafDetected"))
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
            {
                let reason =
                    "前端运行时确认出现 WAF/机器人挑战/验证码或持续限流特征；已立即停止当前目标";
                add_target_to_fuse_zone(&db_path, &scan_id, &url, reason);
                let _ = sender.send(FrontendQueueItem::Limited {
                    position,
                    url,
                    reason: reason.into(),
                });
                continue;
            }
            let invalid_identity_keys = validation
                .and_then(|value| value.get("invalidIdentityKeys"))
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            let mut invalid_ids = invalid_identity_keys
                .iter()
                .filter_map(JsonValue::as_str)
                .filter_map(|identity| identity.strip_prefix("session:"))
                .filter_map(|identity| identity.split(':').next())
                .map(str::to_string)
                .collect::<Vec<_>>();
            for session_id in &invalid_ids {
                crate::auth_session::mark_session_invalid(
                    &db_path,
                    session_id,
                    "前端运行时确认该身份已失效；仅熄灭对应会话，其他身份继续探测",
                );
            }
            if validation
                .and_then(|value| value.get("clearSessionInvalid"))
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
            {
                let reason = "登录后探测被明确重定向回登录页且没有成功业务请求；会话已熄灯，停止后续认证探测";
                if invalid_ids.is_empty() {
                    invalid_ids = browser_session_ids.clone();
                    for session_id in &invalid_ids {
                        crate::auth_session::mark_session_invalid(
                            &db_path,
                            session_id,
                            reason,
                        );
                    }
                }
                let _ = sender.send(FrontendQueueItem::Limited {
                    position,
                    url,
                    reason: reason.into(),
                });
                continue;
            }
            append_runner_log(
                &log_path,
                &format!(
                    "frontend target {position}/{total}: recon result persisted; routing target"
                ),
            );
            let mut route = frontend_routes(&recon_output, std::slice::from_ref(&url), &adaptive)
                .into_iter()
                .next()
                .unwrap_or_else(|| FrontendRoute::fallback(&url, &adaptive, "前端路由结果缺失"));
            if full_power {
                annotate_local_full_power_routes(std::slice::from_mut(&mut route));
            }
            apply_investigation_route_gate(&db_path, &scan_id, &mut route);
            let _ = fs::write(
                target_dir.join("adaptive-routing.json"),
                serde_json::to_vec_pretty(&route.as_json()).unwrap_or_default(),
            );
            update_target_route(&db_path, &scan_id, &route, "routed");
            write_frontend_evidence(
                &recon_output,
                &url,
                &target_dir,
                &route,
                packet_budget,
                Some(&db_path),
                &scan_id,
            );
            native_web_attempt_progress(
                &db_path,
                &scan_id,
                attempt_number,
                &format!(
                    "前端队列已准备 {position}/{total} · {recon_note} · {} 分 · {}",
                    route.score, route.mode
                ),
            );
            let proxy = proxies
                .get(offset % proxies.len().max(1))
                .map(|item| item.1.clone());
            if sender
                .send(FrontendQueueItem::Ready(PreparedFrontendTarget {
                    position,
                    route,
                    target_dir,
                    proxy,
                    browser: Some(AgentBrowserRuntime {
                        helper: worker.clone(),
                        runtime_path: runtime_path.clone(),
                        no_proxy: no_proxy.clone(),
                    }),
                }))
                .is_err()
            {
                break;
            }
            // A local model can already consume every available CPU/GPU core.
            // Do not overlap it with Chrome/Node analysis of the next URL.
            if serialize_for_local && ack_receiver.recv().is_err() {
                break;
            }
        }
    }).map_err(|_| "frontend_producer_thread_start_failed".to_string())?;
    Ok((receiver, serialize_for_local.then_some(ack_sender)))
}
