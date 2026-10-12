// Red-first contracts: registered Single, original frozen origin, real HTTP.
// All APIs below already exist in the preceding Root-model batch.
fn single_target_harness_with(
    tag: &str,
    timeout: u64,
    site: impl Fn(String) -> (u16, &'static str, String) + Send + Sync + 'static,
) -> AgentHarness {
    use crate::agent_runtime::multi_agent::budget::root::RootOwner;
    // The Root wall clock origin is the Root run's `created_at` (written while
    // `runtime_open_run` creates the Root), a whole-second SQLite datetime, while
    // elapsed is sampled at millisecond precision. A Root created at
    // hh:mm:ss.900 is therefore charged ~900ms before any work (conservative,
    // never under-billed). With a 1-4s hard limit that truncation, not the
    // behavior under test, could exhaust the budget before the financial
    // fixture below is even initialized. Create the task just after a second
    // boundary so the frozen origin equals the real start; limits unchanged.
    let mut harness = agent_harness(tag, site, vec![AgentIdentity::anonymous()]);
    harness.context.execution_plan.timeout_seconds = timeout;
    // Align after all expensive setup, before plan persistence inserts the original Root
    // is born. Never rewrite an already born clock or enlarge its deadline.
    if timeout <= 4 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        std::thread::sleep(Duration::from_millis(1002 - u64::from(now.subsec_millis())));
    }
    freeze_harness_plan(&harness);
    harness.context.run = runtime_open_run(
        &harness.db_path,
        &harness.context.scan_id,
        &harness.context.route,
    );
    let db = db::open(&harness.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    RootOwner::initialize_financial_fixture_for_test(&tx, &harness.context.run.as_ref().unwrap().run_id).unwrap();
    tx.commit().unwrap();
    drop(db);
    harness
}

fn single_target_harness(tag: &str, timeout: u64) -> AgentHarness {
    single_target_harness_with(tag, timeout, |_| {
        (200, "text/plain", "single-target-response".into())
    })
}

fn single_target_cleanup(harness: AgentHarness) {
    let root = harness.root.clone();
    let port = reqwest::Url::parse(&harness.context.target_url)
        .unwrap()
        .port()
        .unwrap();
    drop(harness);
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::net::TcpListener::bind(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "single fixture leaked listener {port}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    fs::remove_dir_all(root).unwrap();
}

fn single_target_start(context: &AgentRunContext) -> (AgentToolRuntime, AgentHttpRequest) {
    let mut runtime = AgentToolRuntime::default();
    let invocation = runtime.begin_invocation(context.attempt_number);
    context
        .run
        .as_ref()
        .unwrap()
        .begin_tool("replay_http", &json!({}), &invocation)
        .unwrap();
    runtime.cancel = Some(agent_scan_cancel_token(
        &context.db_path,
        &context.scan_id,
        context.attempt_number,
    ));
    let request = AgentHttpRequest {
        identity: AgentIdentity::anonymous(),
        method: "GET".into(),
        url: format!("{}/plain", context.target_url),
        extra_headers: vec![],
        body: None,
        content_type: None,
        contract_key: String::new(),
        family: "authorization".into(),
        source: ScopeSource::HttpReplay,
        tool: "replay_http".into(),
        timeout_seconds: 15,
    };
    (runtime, request)
}

#[test]
fn single_budget_actual_http_cost_is_original_root_and_shared_counter_is_not_doubled() {
    use crate::agent_runtime::multi_agent::budget;
    let harness = single_target_harness("single-http-known-cost", 30);
    let (mut runtime, request) = single_target_start(&harness.context);
    let result = agent_http_exchange(&harness.context, &mut runtime, &request);
    let requests = harness.site_seen.lock().unwrap().len();
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let b = budget::balance(&db, root, Some(""), "target_requests").unwrap();
    let committed = agent_request_accounting(
        &db,
        &harness.context.scan_id,
        harness.context.attempt_number,
        &harness.context.target_url,
    )
    .unwrap()
    .budget_committed;
    let child_rows: Vec<i64> = [
        "agent_assignments",
        "agent_assignment_attempts",
        "agent_coordinator_leases",
    ]
    .iter()
    .map(|table| {
        db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    })
    .collect();
    drop(db);
    drop(runtime);
    single_target_cleanup(harness);
    assert_eq!(result.unwrap()["status"], 200);
    assert_eq!(requests, 1);
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 1, 0));
    assert_eq!(
        committed, 1,
        "shared executor journal is not a second Root charge"
    );
    assert_eq!(child_rows, vec![0, 0, 0]);
}

#[test]
fn single_budget_expired_root_never_starts_target_or_local_tool_work() {
    let harness = single_target_harness("single-expired-target", 4);
    let (mut runtime, request) = single_target_start(&harness.context);
    std::thread::sleep(Duration::from_millis(4200));
    let db = db::open(&harness.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let result = agent_http_exchange(&harness.context, &mut runtime, &request);
    let local = agent_execute_tool_at(
        &harness.context,
        &mut runtime,
        "inspect_evidence",
        &json!({"kind":"api"}),
        "blocked-local-inspection",
    );
    let unchanged = super::tests::application_table_snapshot(&db) == before;
    let requests = harness.site_seen.lock().unwrap().len();
    drop(db);
    drop(runtime);
    single_target_cleanup(harness);
    assert!(result.is_err(), "expired Root must deny target evidence");
    assert_eq!(requests, 0);
    assert!(!value_first(&local.model_view, &["code"]).is_empty());
    assert!(unchanged);
}

#[test]
fn single_budget_actual_unknown_model_debt_blocks_fresh_tool_and_target() {
    let mut harness = single_target_harness("single-unknown-model-target", 30);
    let (port, model_seen, stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            500,
            "application/json",
            r#"{"error":{"message":"unknown-single-fee"}}"#.into(),
        )
    }));
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&harness.context.environment, None).unwrap(),
        &[],
    );
    let model = native_model_transport(
        &harness.context,
        &client,
        vec![json!({"role":"user","content":"one unknown Root model bill"})],
        &[],
        1,
    );
    let (mut runtime, request) = single_target_start(&harness.context);
    let db = db::open(&harness.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let http = agent_http_exchange(&harness.context, &mut runtime, &request);
    let local = agent_execute_tool_at(
        &harness.context,
        &mut runtime,
        "inspect_evidence",
        &json!({"kind":"api"}),
        "unknown-model-local",
    );
    let unchanged = super::tests::application_table_snapshot(&db) == before;
    let requests = harness.site_seen.lock().unwrap().len();
    let models = model_seen.lock().unwrap().len();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(client);
    drop(model_seen);
    drop(stop);
    drop(db);
    drop(runtime);
    single_target_cleanup(harness);
    assert!(model.is_err());
    assert_eq!(models, 1);
    assert!(http.is_err());
    assert!(!value_first(&local.model_view, &["code"]).is_empty());
    assert_eq!(requests, 0);
    assert!(unchanged);
}

#[test]
fn single_budget_known_headers_after_actual_scan_pause_save_fee_and_deny_output() {
    use crate::agent_runtime::multi_agent::budget;
    let binding = std::sync::Arc::new(std::sync::Mutex::new(None::<(PathBuf, String)>));
    let handler_binding = binding.clone();
    let harness = single_target_harness_with("single-paused-http-output", 30, move |_| {
        let (path, scan) = handler_binding.lock().unwrap().clone().unwrap();
        let db = db::open(&path).unwrap();
        db.execute(
            "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
            [&scan],
        )
        .unwrap();
        drop(db);
        (200, "text/plain", "late-original-http".into())
    });
    *binding.lock().unwrap() = Some((harness.db_path.clone(), harness.context.scan_id.clone()));
    let (mut runtime, request) = single_target_start(&harness.context);
    let result = agent_http_exchange(&harness.context, &mut runtime, &request);
    let requests = harness.site_seen.lock().unwrap().len();
    let empty_output = runtime.coverage.is_empty() && runtime.requests.is_empty();
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let b = budget::balance(&db, root, Some(""), "target_requests").unwrap();
    let headers: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE response_status=200",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let findings: i64 = db
        .query_row(
            "SELECT count(*) FROM sentinel_findings WHERE stage=?1",
            [AGENT_EVIDENCE_STAGE],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    drop(runtime);
    drop(binding);
    single_target_cleanup(harness);
    assert!(
        result.is_err(),
        "known original cost must deny paused output"
    );
    assert_eq!(requests, 1);
    assert!(empty_output);
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 1, 0));
    assert_eq!((headers, findings), (1, 0));
}

#[test]
fn single_budget_actual_http_timeout_uses_original_root_remainder() {
    let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let release = std::sync::Arc::new(std::sync::Mutex::new(release_rx));
    let harness = single_target_harness_with("single-http-original-deadline", 2, move |_| {
        let _ = arrived_tx.send(());
        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(5));
        let _ = finished_tx.send(());
        (200, "text/plain", "too-late".into())
    });
    let (mut runtime, request) = single_target_start(&harness.context);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let (arrived, completed_before_release, outcome, handler_finished) =
        std::thread::scope(|scope| {
            let context = &harness.context;
            let worker = scope.spawn(move || {
                let start = Instant::now();
                let failed = agent_http_exchange(context, &mut runtime, &request).is_err();
                let outcome = (failed, start.elapsed());
                let _ = done_tx.send(outcome);
                outcome
            });
            let arrived = arrived_rx.recv_timeout(Duration::from_secs(2)).is_ok();
            let early = done_rx.recv_timeout(Duration::from_millis(2500)).ok();
            // Release even on the red baseline, then join before any assertion.
            let _ = release_tx.send(());
            let finished = finished_rx.recv_timeout(Duration::from_secs(2)).is_ok();
            (arrived, early.is_some(), worker.join().unwrap(), finished)
        });
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let b =
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "target_requests")
            .unwrap();
    drop(db);
    single_target_cleanup(harness);
    assert!(arrived && handler_finished);
    assert!(
        completed_before_release && outcome.0,
        "Root deadline must end real HTTP before server release"
    );
    assert!(
        outcome.1 < Duration::from_millis(2500),
        "original deadline, not a fresh 15-second ceiling"
    );
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, 1));
}
