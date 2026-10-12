// Actual producer fixture: fresh creator/HMAC/Root and actual supplier Broker,
// SDK endpoint and supervisor. No injected controller decision or test issuer.
struct ClientHookFixture {
    directory: PathBuf,
    root: String,
    db: rusqlite::Connection,
    context: AgentRunContext,
    session: Option<MultiAgentSession>,
    site_seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    model_seen: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    site_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    model_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    model_boundary_sql: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    entry_floor: i64,
    event_floor: i64,
    collaboration_floor: i64,
}
impl Drop for ClientHookFixture {
    fn drop(&mut self) {
        // Join original supervision before the temporary database is removed.
        drop(self.session.take());
        self.site_stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.model_stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = fs::remove_dir_all(&self.directory);
    }
}
impl ClientHookFixture {
    fn finish(&mut self) -> Result<(), String> {
        multi_agent_finish_execution(
            &self.context,
            self.session.as_mut().unwrap(),
            &AgentTargetOutcome::incomplete("known captured config; no impact"),
        )
    }
}
fn client_hook_fixture(tag: &str, hard_tokens: i64, hard_requests: i64) -> ClientHookFixture {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{scheduler, supervisor::WorkerSupervisor},
    };
    let (site_port, site_seen, site_stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "text/html",
            "<html>readonly config source</html>".into(),
        )
    }));
    let target = format!("http://127.0.0.1:{site_port}");
    let (directory, path, root, lease) =
        multi_agent_task_root_for_target_protocol(tag, hard_tokens, hard_requests, &target, false);
    let mut context = test_context(&path, &target, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.target_dir = fs::canonicalize(&directory)
        .unwrap()
        .join("private-http-source");
    context.run = Some(AgentRunLedger {
        db_path: path.clone(),
        run_id: root.clone(),
    });
    // Read the actual born plan; do not freeze/adopt a Root after history.
    context.execution_plan =
        web_mode_positive_plan_or(&path, &target, context.execution_plan.clone());
    let db = db::open(&path).unwrap();
    assert!(crate::agent_runtime::web_mode::root::read(&db, &root).unwrap().unwrap()
        .new_evidence_supervision().is_none(), "direct Broker fixture uses original birth contract");
    let supplier = scheduler::schedule_child(
        &db,
        &lease,
        AgentRole::WebExecutor,
        AgentLane::TargetTouching,
        "client-source-capture",
        &json!({}),
        1,
        &["replay_http".into()],
        512,
        0,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &lease, &supplier).unwrap();
    context.run.as_mut().unwrap().run_id = supplier.run_id.clone();
    accounting_native(&context, 0);
    let supervisor = WorkerSupervisor::start(&path, &lease).unwrap();
    context.supervision = Some(supervisor.ticket());
    let mut runtime = AgentToolRuntime::default();
    let invocation = runtime.begin_invocation(1);
    let row = context
        .run
        .as_ref()
        .unwrap()
        .begin_tool("replay_http", &json!({}), &invocation)
        .unwrap();
    let request = AgentHttpRequest {
        identity: AgentIdentity::anonymous(),
        method: "GET".into(),
        url: target.clone(),
        extra_headers: vec![],
        body: None,
        content_type: None,
        contract_key: String::new(),
        family: "authorization".into(),
        source: ScopeSource::IdentityComparison,
        tool: "replay_http".into(),
        timeout_seconds: 5,
    };
    let view = agent_http_exchange(&context, &mut runtime, &request).unwrap();
    context
        .run
        .as_ref()
        .unwrap()
        .finish_tool(Some(row), "replay_http", &json!({}), &view, &invocation, 1)
        .unwrap();
    accounting_native(&context, 1);
    let model_boundary_sql = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let model_boundary = model_boundary_sql.clone();
    let model_database = path.clone();
    let (model_port, model_seen, model_stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let body = wire.split_once("\r\n\r\n").unwrap().1;
        let json: JsonValue = serde_json::from_str(body).unwrap();
        let input: JsonValue =
            serde_json::from_str(json["messages"][1]["content"].as_str().unwrap()).unwrap();
        let refs: Vec<_> = input["observations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["id"].clone())
            .collect();
        // Test-only fault timing: grant has committed and actual provider I/O
        // has arrived. Static fixture SQL cannot fail an earlier grant instead.
        if let Some(sql) = model_boundary.lock().unwrap().take() {
            rusqlite::Connection::open_with_flags(
                &model_database, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
            ).unwrap().execute_batch(&sql).unwrap();
        }
        let response = json!({"summary":"原捕获配置观察已记录；没有浏览器影响证明","observationRefs":refs,
            "gaps":["missing_browser_validation"],"candidates":[]});
        (
            200,
            "application/json",
            proposal_model_response(&response.to_string()),
        )
    }));
    context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
    let session = MultiAgentSession {
        supervisor,
        lease: lease.clone(),
        mapper: supplier.clone(),
        executor: supplier,
    };
    let entry_floor = db
        .query_row(
            "SELECT COALESCE(MAX(rowid),0) FROM agent_budget_entries",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let event_floor = db
        .query_row(
            "SELECT COALESCE(MAX(rowid),0) FROM agent_events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let collaboration_floor = db
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    ClientHookFixture {
        directory,
        root,
        db,
        context,
        session: Some(session),
        site_seen,
        model_seen,
        site_stop,
        model_stop,
        model_boundary_sql,
        entry_floor,
        event_floor,
        collaboration_floor,
    }
}
