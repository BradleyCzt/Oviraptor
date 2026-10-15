// Runtime artifacts are compared by path/type/content hash, never printed as
// raw task data or private HMAC key bytes when an assertion fails.
fn paid_pause_work_files(root: &Path) -> Vec<(PathBuf, String)> {
    fn collect(root: &Path, path: &Path, files: &mut Vec<(PathBuf, String)>) {
        use sha2::{Digest, Sha256};
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(!metadata.file_type().is_symlink());
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        if metadata.is_dir() {
            files.push((relative, "directory".into()));
            for entry in fs::read_dir(path).unwrap() {
                collect(root, &entry.unwrap().path(), files);
            }
        } else {
            assert!(metadata.is_file());
            files.push((
                relative,
                format!(
                    "file:{}:{:x}",
                    metadata.len(),
                    Sha256::digest(fs::read(path).unwrap())
                ),
            ));
        }
    }
    let mut files = Vec::new();
    collect(root, root, &mut files);
    files.sort();
    files
}

fn assert_paid_pause_startup_refused(root: &Path, db_path: &Path, scan: &str, mode: WebStartMode) {
    let mut db = db::open(db_path).unwrap();
    let before = web_mode_test_rows(&db);
    let changes = db.total_changes();
    let work = root.join(SCAN_WORK_DIRECTORY).join(scan);
    let files = paid_pause_work_files(&work);
    assert_eq!(
        web_mode_test_start(root, &mut db, scan, mode).unwrap_err(),
        "web_start_paid_history_requires_explicit_financial_contract"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        db.total_changes(),
        changes,
        "paid startup refusal must precede every database write"
    );
    assert_eq!(
        paid_pause_work_files(&work),
        files,
        "paid startup refusal must preserve every runtime artifact"
    );
    assert!(
        !work.join("attempt-0002").exists(),
        "refusal must not allocate another attempt"
    );
}

/// A real paid Single attempt pauses with its original financial history.
/// Ordinary Resume/Retry cannot create another allowance: a reviewed financial
/// continuation contract is required and its execution is not implemented yet.
#[test]
fn e2e_paid_pause_refuses_resume_and_retry_without_financial_contract() {
    let mut harness = fresh_single_production_harness("e2e-pause", mock_site);
    freeze_fresh_single_production_harness(&mut harness);
    let AgentHarness {
        root,
        db_path,
        mut context,
        site_seen,
        ..
    } = harness;
    let target_url = context.target_url.clone();
    let pause_db = db_path.clone();
    let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let replay = model_round(
        &[(
            "replay_http",
            serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"}),
        )],
        500,
    );
    let (port, model_seen, _model_stop) = spawn_endpoint(std::sync::Arc::new(move |request| {
        if !request.contains("POST /v1/chat/completions") {
            return (404, "text/plain", "unexpected path".to_string());
        }
        let body = if served.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            replay.clone()
        } else {
            // The user pauses during a text-only round; no target request is
            // in flight when the executor observes the pause boundary.
            request_sentinel_pause(&pause_db, "agent-scan").unwrap();
            model_round(&[], 10)
        };
        (200, "application/json", body)
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let outcome = NativeAgentBackend.execute(&context);
    assert!(
        matches!(outcome, AgentTargetOutcome::Cancelled),
        "{:?}",
        outcome.detail()
    );
    assert_eq!(
        record_runtime_terminal_facts_checked(&db_path, "agent-scan", &context.route, &outcome),
        Ok(None)
    );
    assert!(finish_sentinel_pause(&db_path, "agent-scan", 1).unwrap());
    let parent = NativeAgentState::read(&db_path, "agent-scan", &target_url).unwrap();
    assert_eq!(
        (
            parent.attempt_number,
            parent.parent_attempt_number,
            parent.target_requests
        ),
        (1, None, 1)
    );
    assert!(
        parent.terminal_reason.is_empty(),
        "a pause does not settle the original runtime"
    );
    assert_eq!(site_seen.lock().unwrap().len(), 1);
    assert_eq!(model_seen.lock().unwrap().len(), 2);
    let db = db::open(&db_path).unwrap();
    let run = &context.run.as_ref().unwrap().run_id;
    let paused_run: String = db
        .query_row("SELECT status FROM agent_runs WHERE id=?1", [run], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        paused_run, "paused",
        "the runtime row must not settle on a pause"
    );
    let paid_models: i64 = db.query_row("SELECT COALESCE(SUM(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND dimension='model_requests' AND kind='consume'", [run], |row| row.get(0)).unwrap();
    assert_eq!(
        paid_models, 2,
        "both actual provider rounds retain their recorded cost"
    );
    let rounds = {
        let mut q = db.prepare("SELECT round FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch' ORDER BY round").unwrap();
        q.query_map([run], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(rounds, vec![1, 2]);
    drop(db);
    for mode in [WebStartMode::Resume, WebStartMode::Retry] {
        assert_paid_pause_startup_refused(&root, &db_path, "agent-scan", mode);
        let unchanged = NativeAgentState::read(&db_path, "agent-scan", &target_url).unwrap();
        assert_eq!(
            (
                unchanged.attempt_number,
                unchanged.parent_attempt_number,
                unchanged.target_requests
            ),
            (1, None, 1)
        );
        assert_eq!(
            site_seen.lock().unwrap().len(),
            1,
            "refused startup sends no target request"
        );
        assert_eq!(
            model_seen.lock().unwrap().len(),
            2,
            "refused startup sends no model request"
        );
    }
}

/// Paid startup refusal must not be bypassed to manufacture a resumed Root.
/// An independent task's real fresh Single Root starts at round 1; the paid
/// parent's turn number and skipped rounds cannot authorize its first call.
#[test]
fn fresh_single_root_refuses_inherited_or_skipped_model_round_after_paid_resume_refusal() {
    use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
    let mut harness = fresh_single_production_harness("e2e-root-round", mock_site);
    freeze_fresh_single_production_harness(&mut harness);
    let AgentHarness {
        root,
        db_path,
        mut context,
        site_seen,
        ..
    } = harness;
    let target_url = context.target_url.clone();
    let pause_db = db_path.clone();
    let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let round_one = model_round(
        &[(
            "replay_http",
            serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"}),
        )],
        500,
    );
    let (port, model_seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |request| {
        if !request.contains("POST /v1/chat/completions") {
            return (404, "text/plain", "unexpected path".to_string());
        }
        if served.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            return (200, "application/json", round_one.clone());
        }
        request_sentinel_pause(&pause_db, "agent-scan").unwrap();
        (200, "application/json", model_round(&[], 10))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let outcome = NativeAgentBackend.execute(&context);
    assert!(
        matches!(outcome, AgentTargetOutcome::Cancelled),
        "{:?}",
        outcome.detail()
    );
    assert_eq!(
        record_runtime_terminal_facts_checked(&db_path, "agent-scan", &context.route, &outcome),
        Ok(None)
    );
    assert!(finish_sentinel_pause(&db_path, "agent-scan", 1).unwrap());
    let parent = NativeAgentState::read(&db_path, "agent-scan", &target_url).unwrap();
    assert_eq!(
        (parent.attempt_number, parent.turns, parent.target_requests),
        (1, 1, 1)
    );
    assert_paid_pause_startup_refused(&root, &db_path, "agent-scan", WebStartMode::Resume);

    // This is a separately created task with no inherited charge or resume
    // permission. Do not clear the paid parent's ledger or invoke a lower-level
    // issuer to make the blocked Resume above appear to have succeeded.
    let mut fresh = fresh_single_production_harness("e2e-independent-root-round", mock_site);
    freeze_fresh_single_production_harness(&mut fresh);
    native_prepare_single_budget(&fresh.context).unwrap();
    let db = db::open(&fresh.db_path).unwrap();
    let run = fresh.context.run.as_ref().unwrap().run_id.clone();
    let before = web_mode_test_rows(&db);
    let changes = db.total_changes();
    let work = fresh
        .root
        .join(SCAN_WORK_DIRECTORY)
        .join(&fresh.context.scan_id);
    let files = paid_pause_work_files(&work);
    let models_before = fresh.model_seen.lock().unwrap().len();
    for (round, expected) in [
        (parent.turns + 1, "budget_history_requires_reconciliation"),
        (parent.turns + 2, "budget_history_requires_reconciliation"),
        (0, "budget_root_call_invalid"),
    ] {
        let tx = db.unchecked_transaction().unwrap();
        let error =
            RootModelCall::claim_single_transport(&tx, &run, round, &"a".repeat(64), 1_000).err();
        drop(tx);
        assert_eq!(
            error.as_deref(),
            Some(expected),
            "round {round} must be refused"
        );
    }
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        db.total_changes(),
        changes,
        "a refused round must precede every write"
    );
    assert_eq!(paid_pause_work_files(&work), files);
    assert_eq!(
        fresh.model_seen.lock().unwrap().len(),
        models_before,
        "a refused round sends no provider request"
    );
    assert_eq!(
        fresh.site_seen.lock().unwrap().len(),
        0,
        "an independent fresh task sends no target request"
    );
    assert_eq!(site_seen.lock().unwrap().len(), 1);
    assert_eq!(model_seen.lock().unwrap().len(), 2);
}

    /// A local deployment must keep working through the LLM hook that makes
    /// cancellation real: the hook forwards to the configured server and leaves its
    /// own usage ledger beside the target.
    #[test]
    fn e2e_local_deployment_runs_through_the_cancellable_hook() {
        let mut harness = agent_harness(
            "e2e-local-hook",
            mock_site,
            vec![
                AgentIdentity::anonymous(),
                AgentIdentity::scoped("session-a"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        let target_url = harness.context.target_url.clone();
        let replay = |identity: &str| {
            serde_json::json!({"identity":identity,"method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"})
        };
        retarget_model(
            &mut harness,
            vec![
                model_round(&[("replay_http", replay("session-a"))], 900),
                model_round(&[("replay_http", replay("anonymous"))], 900),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({
                            "coverage": closing_ledger(&["authorization"]),
                            "stopReason":"本地模型收口"
                        }),
                    )],
                    900,
                ),
            ],
        );
        harness.context.environment.deployment = "local".to_string();
        let target_dir = harness.context.target_dir.clone();
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert!(
            matches!(
                outcome,
                AgentTargetOutcome::Completed(_) | AgentTargetOutcome::BoundedCompleted(_)
            ),
            "{:?}",
            outcome.detail()
        );
        assert!(
            harness.model_seen.lock().unwrap().len() >= 3,
            "the hook forwards every round to the configured server"
        );
        assert!(
            target_dir.join("llm-hook.jsonl").is_file(),
            "the run went through the hook"
        );
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            2,
            "both sides of the identity contrast reached the target"
        );
    }

    /// A stand-in for the bundled CDP probe: reads the scripted input on stdin and
    /// answers with a runtime capture, so the browser path is testable without a
    /// browser installed here.
    fn scripted_cdp_helper(root: &Path, available: bool) -> PathBuf {
        let path = root.join("fake_runtime_probe.cjs");
        let script = if available {
            r#"
const input = JSON.parse(require("fs").readFileSync(0, "utf8"));
const origin = new URL(String(input.url || "http://127.0.0.1")).origin;
const out = { available: true, actions: [], requests: [], stopReason: "scripted", captureStatus: "complete" };
if (input.targetAction) {
  out.actions = [{ id: "action-1", label: String(input.targetAction), stateChanged: true, afterUrl: origin + "/orders" }];
  out.requests = [
    { url: origin + "/api/orders?page=2", method: "GET", status: 200, actionId: "action-1", resourceType: "xhr" },
    { url: "https://cdn.example.invalid/static/app.js", method: "GET", status: 200, actionId: "navigation", resourceType: "script" },
    { url: "https://tracker.example.com/collect", method: "POST", status: 204, actionId: "action-1", resourceType: "xhr" },
    { url: origin + "/legacy/report", method: "GET", status: 301, actionId: "action-1", resourceType: "document", redirectedTo: "https://other-host.example.org/legacy/report" }
  ];
}
process.stdout.write(JSON.stringify(out));
"#
        } else {
            r#"
JSON.parse(require("fs").readFileSync(0, "utf8"));
process.stdout.write(JSON.stringify({ available: false, errors: ["no browser binary"], actions: [], requests: [] }));
"#
        };
        fs::write(&path, script).unwrap();
        path
    }

    fn browser_runtime(helper: PathBuf) -> AgentBrowserRuntime {
        AgentBrowserRuntime {
            helper,
            runtime_path: std::env::var_os("PATH").unwrap_or_default(),
            no_proxy: "127.0.0.1,localhost".to_string(),
        }
    }

    /// `browser_action` performs the located control in a real browser session and
    /// reports only the requests that step produced — counted against the target
    /// budget, because the target really served them.
    #[test]
    fn browser_action_drives_the_probe_and_reports_its_delta() {
        let harness = agent_harness("agent-browser-act", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            root,
            mut context,
            site_seen,
            ..
        } = harness;
        context.browser = Some(browser_runtime(scripted_cdp_helper(&root, true)));
        let mut runtime = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut runtime,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"}),
        ).model_view;
        assert_eq!(
            report.get("browserDriven").and_then(JsonValue::as_bool),
            Some(true),
            "{report}"
        );
        let delta = report
            .get("networkDelta")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        assert_eq!(delta.len(), 1, "only the action's own request: {report}");
        assert_eq!(value_first(&delta[0], &["path"]), "/api/orders");
        assert_eq!(
            delta[0]
                .get("parameters")
                .and_then(JsonValue::as_array)
                .map(|rows| rows.len()),
            Some(1)
        );
        assert_eq!(runtime.target_requests, 1, "browser traffic spent the budget");
        assert!(runtime.touched("GET", "/api/orders"));
        assert!(runtime.families.contains("business_flow"));
        assert!(
            runtime.queue_key_satisfied(&context, "api:GET|/api/orders"),
            "a browser-observed endpoint closes the queue"
        );
        assert_eq!(
            site_seen.lock().unwrap().len(),
            0,
            "the tool itself makes no extra HTTP request"
        );
    }

    /// An isolated browser with per-request Broker enforcement is not shipped
    /// yet. A live multi-agent child cannot silently use the host CDP helper.
    #[test]
    fn multi_agent_browser_action_uses_only_brokered_http_without_host_browser() {
        let harness = agent_harness("agent-browser-sandbox", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness { root, mut context, site_seen, .. } = harness;
        context.browser = Some(browser_runtime(scripted_cdp_helper(&root, true)));
        context.run_budget = Some(AgentRunBudgetWindow {
            starting_tokens: 0,
            starting_requests: 0,
            hard_tokens: 4_000,
            hard_requests: 10,
        });
        let mut runtime = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut runtime,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"}),
        ).model_view;
        assert_eq!(report.get("browserDriven").and_then(JsonValue::as_bool), Some(false), "{report}");
        assert_eq!(value_first(&report, &["browserAttempt"]),
            "unsupported_sandbox:browser_network_broker_unavailable");
        assert!(report.get("networkDelta").is_none(), "{report}");
        assert_eq!(site_seen.lock().unwrap().len(), 1, "only the brokered entry GET may reach the target");
        assert_eq!(runtime.target_requests, 1);
        assert!(runtime.touched("GET", "/orders"));
        assert!(!runtime.touched("GET", "/api/orders"), "capture attribution is not an executed request");
        assert!(!runtime.endpoints.contains("GET|/api/orders"), "capture attribution must not advance discovery");
        assert!(runtime.parameter_signatures.is_empty(), "capture attribution must not advance parameter progress");
        assert!(!runtime.coverage.iter().any(|entry| entry.evidence_kind == "browser_action"),
            "the HTTP fallback is not a browser execution");
    }

    /// With no browser runtime the tool must say it did not drive a browser,
    /// and must label the capture-derived APIs as attribution, not observation.
    #[test]
    fn browser_action_says_when_it_could_not_drive_a_browser() {
        let harness =
            agent_harness("agent-browser-none", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            root,
            mut context,
            site_seen,
            ..
        } = harness;
        let mut runtime = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut runtime,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"}),
        ).model_view;
        assert_eq!(
            report.get("browserDriven").and_then(JsonValue::as_bool),
            Some(false),
            "{report}"
        );
        assert_eq!(value_first(&report, &["browserAttempt"]), "no_browser_runtime");
        assert!(report.get("attributedApis").is_some());
        assert!(report.get("newApiDelta").is_none(), "attribution is not observation");
        assert!(!runtime.endpoints.contains("GET|/api/orders"));
        assert!(runtime.parameter_signatures.is_empty());
        assert!(!runtime.coverage.iter().any(|entry| entry.evidence_kind == "browser_action"));

        // A probe that reports no browser also degrades, and says what happened.
        context.browser = Some(browser_runtime(scripted_cdp_helper(&root, false)));
        let mut retry = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut retry,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"}),
        ).model_view;
        assert_eq!(
            report.get("browserDriven").and_then(JsonValue::as_bool),
            Some(false),
            "{report}"
        );
        assert_eq!(value_first(&report, &["browserAttempt"]), "no_browser_runtime");
        assert!(!retry.endpoints.contains("GET|/api/orders"));
        assert!(retry.parameter_signatures.is_empty());
        assert!(!retry.coverage.iter().any(|entry| entry.evidence_kind == "browser_action"));
        assert_eq!(
            site_seen.lock().unwrap().len(),
            2,
            "each degraded call contributes exactly one direct navigation"
        );
    }

    /// Case 17: the runtime store mirrors the loop while it runs. The pause is
    /// flipped from inside the second model round, so round one is already fully
    /// audited in `agent_events`/`tool_invocations`/`agent_snapshots` before the
    /// loop stops, and the refused round-two call is audited too.
    #[test]
    fn live_run_facts_land_before_the_terminal_close() {
        let harness = single_target_harness_with("e2e-live-events", 90, mock_site);
        let AgentHarness {
            db_path,
            mut context,
            site_seen,
            ..
        } = harness;
        let target_url = context.target_url.clone();
        // Explicit lower-level original Single financial fixture, frozen before
        // the real Native loop; no backfill by the executor or SDK transport.
        let ledger = context.run.as_ref().unwrap().clone();

        let pause_db = db_path.clone();
        let served = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let served_counter = served.clone();
        let replay_url = target_url.clone();
        let (model_port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_request| {
            if served_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 1 {
                if let Ok(connection) = db::open(&pause_db) {
                    let _ = connection.execute(
                        "UPDATE sentinel_scans SET status='pausing' WHERE id='agent-scan'",
                        [],
                    );
                }
            }
            (
                200,
                "application/json",
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{replay_url}/api/orders"),"family":"authorization"}),
                    )],
                    500,
                ),
            )
        }));
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");

        let outcome = NativeAgentBackend.execute(&context);
        assert!(
            matches!(outcome, AgentTargetOutcome::Cancelled),
            "{:?}",
            outcome.detail()
        );
        assert_eq!(site_seen.lock().unwrap().len(), 1, "the pause stops later rounds");

        let connection = db::open(&db_path).unwrap();
        let rounds: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_events WHERE run_id=?1 AND event_type='model_round_completed'",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rounds, 1, "a paid response after pause cannot produce a semantic model-round event");
        let completed: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tool_invocations WHERE run_id=?1 AND tool_name='replay_http' AND status='completed'",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(completed, 1, "the executed call is audited live");
        let refused: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM tool_invocations WHERE run_id=?1 AND status='refused' AND error_class='cancelled'",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(refused, 0, "unaccepted late model output cannot invent a tool invocation");
        let root = &ledger.run_id;
        let requests = crate::agent_runtime::multi_agent::budget::balance(&connection, root, None, "model_requests").unwrap();
        assert_eq!(requests.consumed + requests.indeterminate, 2, "both physical SDK calls remain charged or explicitly unknown");
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'", [root], |r| r.get::<_,i64>(0)).unwrap(), 2);
        let native_before = crate::agent_runtime::single_projection::source_hash(&connection, &runtime_report(&db_path, "agent-scan", &context.route)).unwrap();
        assert!(record_runtime_terminal_facts_checked(&db_path, "agent-scan", &context.route, &outcome).unwrap().is_none());
        assert_eq!(crate::agent_runtime::single_projection::source_hash(&connection, &runtime_report(&db_path, "agent-scan", &context.route)).unwrap(), native_before);
        let snapshot: String = connection
            .query_row(
                "SELECT snapshot_json FROM agent_snapshots WHERE run_id=?1",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        let snapshot = json(snapshot);
        assert_eq!(
            snapshot.get("targetRequests").and_then(JsonValue::as_i64),
            Some(1)
        );
        assert_eq!(snapshot.get("terminal").map(|v| v.is_null()), Some(true));
        let status: String = connection
            .query_row(
                "SELECT status FROM agent_runs WHERE id=?1",
                [&ledger.run_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "paused", "real SDK pause saves a yield boundary without a terminal verdict");
    }

    // ------------------------------------------------------------------
    // Remediation §13.4 — one scope gate for HTTP, browser entry and CDP output
    // ------------------------------------------------------------------

    /// A browser entry outside the frozen origins must be refused before Chrome is
    /// ever started.
    #[test]
    fn browser_entry_outside_frozen_scope_is_refused() {
        let harness =
            agent_harness("remediation-entry", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            root,
            mut context,
            site_seen,
            ..
        } = harness;
        context.browser = Some(browser_runtime(scripted_cdp_helper(&root, true)));
        // The evidence action points at a host that was never approved.
        context.evidence["investigation"]["actions"] = serde_json::json!([
            {"key": "external-panel", "type": "link", "label": "第三方客服", "url": "https://vendor-support.example.com/chat"}
        ]);
        let mut runtime = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut runtime,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"external-panel"}),
        ).model_view;
        assert_eq!(value_first(&report, &["code"]), "scope_denied", "{report}");
        assert_eq!(runtime.target_requests, 0);
        assert_eq!(site_seen.lock().unwrap().len(), 0);
    }

    /// Requests a browser observed on a third party must never become formal APIs,
    /// even when the page itself is authorized.
    #[test]
    fn cdp_observed_third_party_never_enters_the_ledger() {
        let harness =
            agent_harness("remediation-cdp", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            root,
            mut context,
            site_seen,
            ..
        } = harness;
        context.browser = Some(browser_runtime(scripted_cdp_helper(&root, true)));
        let mut runtime = AgentToolRuntime::default();
        let report = agent_execute_tool(
            &context,
            &mut runtime,
            "browser_action",
            &serde_json::json!({"identity":"anonymous","actionKey":"open-orders","family":"business_flow"}),
        ).model_view;
        let rejected = report
            .get("rejectedObservations")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            rejected.iter().any(|row| value_first(row, &["host"]) == "tracker.example.com"),
            "the third-party XHR must be audited and refused: {report}"
        );
        assert!(!runtime.touched("POST", "/collect"), "no third-party endpoint in the ledger");
        assert_eq!(
            runtime.target_requests, 1,
            "only the authorized business API counts as spend"
        );
        assert_eq!(site_seen.lock().unwrap().len(), 0);
    }

    /// A cross-origin redirect out of the frozen scope stops that action; an
    /// identity-provider jump is recorded but never auto-tested.
    #[test]
    fn cross_origin_redirects_stop_the_action_and_idp_jumps_are_only_observed() {
        let (_root, db_path) = temp_database("remediation-redirect");
        seed_scan(&db_path, "agent-scan", "scanning");
        let (port, _redirect_history) = spawn_redirect_server(
            "https://unrelated-host.example.org/stolen",
            "https://login.idp.example.com/o/authorize?client_id=abc&redirect_uri=x&response_type=code",
        );
        let base = format!("http://127.0.0.1:{port}");
        let context = test_context(&db_path, &base, vec![AgentIdentity::anonymous()]);
        let mut runtime = AgentToolRuntime::default();
        let hijack = agent_execute_tool(
            &context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{base}/api/hijack"),"family":"authorization"}),
        ).model_view;
        assert_eq!(value_first(&hijack, &["redirectCode"]), "redirect_out_of_scope", "{hijack}");
        assert!(
            !runtime.families.contains("authorization"),
            "a refused redirect cannot credit coverage"
        );

        let sso = agent_execute_tool(
            &context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{base}/api/sso"),"family":"authentication_session"}),
        ).model_view;
        assert_eq!(value_first(&sso, &["redirectCode"]), "identity_provider_redirect", "{sso}");
        assert!(
            !runtime.families.contains("authentication_session"),
            "an identity-provider jump is observation, not coverage"
        );
    }

    /// §13.5: a tool result that carries a credential must leave nothing of it in
    /// the model view, and the raw copy must be a different file with different
    /// content — the split the whole redaction contract depends on.
    #[test]
    fn tool_results_are_redacted_before_reaching_the_model() {
        const LEAKY_JWT: &str =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.6-6Sp7A_qFVoXcCwZDmenP";
        let site = move |request: String| {
            if request.contains("/api/whoami") {
                return (
                    200,
                    "application/json",
                    format!(
                        "{{\"token\":\"{LEAKY_JWT}\",\"authorization\":\"Bearer {LEAKY_JWT}\",\"email\":\"ops@example.com\",\"phone\":\"13800138000\",\"role\":\"admin\",\"orders\":12}}"
                    ),
                );
            }
            (404, "text/plain", "not found".to_string())
        };
        let harness = agent_harness("redact-view", site, vec![AgentIdentity::anonymous()]);
        let target_url = harness.context.target_url.clone();
        let mut runtime = AgentToolRuntime::default();
        let result = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/whoami"),"family":"authentication_session"}),
        );
        let view = result.model_view.to_string();
        for secret in [LEAKY_JWT, "ops@example.com", "13800138000"] {
            assert!(
                !view.contains(secret),
                "the model view leaked {secret}: {view}"
            );
        }
        assert!(
            view.contains("<redacted:"),
            "markers must stay comparable: {view}"
        );
        // What the contract says the model still needs: structure, not payloads.
        assert_eq!(result.model_view["status"].as_i64(), Some(200));
        assert_eq!(result.model_view["contentType"].as_str(), Some("application/json"));
        assert!(view.contains("\"role\""), "field paths survive: {view}");
        assert!(view.contains("\"bodySha256\""), "hashes survive: {view}");
        assert!(view.contains("\"truncated\""), "length facts survive: {view}");
        assert!(!result.artifact_id.is_empty());
        assert_eq!(
            result.model_view["rawArtifactId"].as_str(),
            Some(result.artifact_id.as_str())
        );

        let directory = harness.context.target_dir.join("agent-http");
        let metadata = fs::read_to_string(directory.join(&result.artifact_id)).unwrap();
        assert!(!metadata.contains(LEAKY_JWT), "raw bytes leaked into metadata");
        assert!(!metadata.contains("rawBody"));
        let raw = fs::read_to_string(directory.join("0001.body")).unwrap();
        assert!(raw.contains(LEAKY_JWT), "the audit copy must stay complete");
        assert_ne!(raw, view, "raw evidence and model view cannot be the same text");
        // The request metadata keeps names, never values, and a bodyless GET has
        // no raw request payload at all.
        let stored: JsonValue = json(metadata);
        let names = stored["request"]["headerNames"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            names.iter().all(|name| !name
                .as_str()
                .unwrap_or_default()
                .contains("Bearer")),
            "header values must not be persisted as metadata: {stored}"
        );
        assert_eq!(stored["request"]["bodyBytes"], JsonValue::Null);
        assert!(!directory.join("0001.request").exists());
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(directory.join("0001.body")).unwrap().permissions().mode() & 0o777
        };
        assert_eq!(permissions, 0o600, "raw evidence is readable only by this user");
    }

    /// §14.10: the loop must never hand a credential to the model, whichever
    /// provider class is configured — so what the mock endpoint actually received
    /// is the thing under test, not what the tool returned.
    #[test]
    fn e2e_model_history_never_carries_a_raw_credential() {
        const LEAKY_JWT: &str =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.6-6Sp7A_qFVoXcCwZDmenP";
        let mut harness = agent_harness(
            "e2e-redact",
            move |request: String| {
                if request.contains("/api/orders") {
                    return (
                        200,
                        "application/json",
                        format!(
                            "{{\"sessionToken\":\"{LEAKY_JWT}\",\"owner\":\"ops@example.com\",\"items\":[{{\"id\":1}}]}}"
                        ),
                    );
                }
                (404, "text/plain", "not found".to_string())
            },
            vec![AgentIdentity::anonymous()],
        );
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"}),
                    )],
                    900,
                ),
                model_round(&[], 1_100),
            ],
        );
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert!(
            matches!(
                outcome,
                AgentTargetOutcome::Incomplete(_)
                    | AgentTargetOutcome::Completed(_)
                    | AgentTargetOutcome::BoundedCompleted(_)
            ),
            "unexpected outcome: {:?}",
            outcome.detail()
        );
        let prompts = harness.model_seen.lock().unwrap().join("\n");
        assert!(
            prompts.contains("POST /v1/chat/completions"),
            "the second round must have run"
        );
        assert!(
            !prompts.contains(LEAKY_JWT),
            "a raw session token reached the model provider"
        );
        assert!(!prompts.contains("ops@example.com"));
        assert!(
            prompts.contains("<redacted:"),
            "the tool result the model saw must be the marked view"
        );
        // The credential is still recoverable for audit, from the raw side only.
        let raw = fs::read_to_string(
            harness
                .context
                .target_dir
                .join("agent-http")
                .join("0001.body"),
        )
        .unwrap();
        assert!(raw.contains(LEAKY_JWT));
    }
