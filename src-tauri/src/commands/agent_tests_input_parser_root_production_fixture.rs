// U176: actual fresh creator -> paid Root -> paid Mapper -> SDK executor Broker GET
// of an observed `?page=1` request -> U175 frozen InputParser frame -> one
// independently paid original-Root tick that Rust bounds to `dispatch:input_parser`.
// Nothing is minted (U177); BL/AZ/ES role frames stay not_ready.
type InputParserRootBoundary =
    std::sync::Arc<std::sync::Mutex<Option<Box<dyn FnOnce(&rusqlite::Connection) + Send>>>>;
type InputParserTestReply = std::sync::Arc<std::sync::Mutex<Option<Box<dyn FnMut(&str) -> String + Send>>>>;
struct InputParserRootFixture {
    h: AgentHarness,
    session: Option<MultiAgentSession>,
    root: String,
    boundary: InputParserRootBoundary,
    model_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    outcome: AgentTargetOutcome,
    new_evidence_reply: InputParserTestReply,
}
impl Drop for InputParserRootFixture {
    fn drop(&mut self) {
        drop(self.session.take());
        self.model_stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = fs::remove_dir_all(&self.h.root);
    }
}
impl InputParserRootFixture {
    fn lease(&self) -> &crate::agent_runtime::multi_agent::lease::CoordinatorLease {
        &self.session.as_ref().unwrap().lease
    }
    fn feedback(&self) -> Result<NativeCoordinatorTickReceipt, String> {
        native_coordinator_input_parser_feedback(&self.h.context, self.lease())
    }
    fn calls(&self) -> usize {
        self.h.model_seen.lock().unwrap().len()
    }
    fn requests(&self, db: &rusqlite::Connection) -> crate::agent_runtime::multi_agent::budget::Balance {
        crate::agent_runtime::multi_agent::budget::balance(db, &self.root, Some(""), "model_requests").unwrap()
    }
    fn record(&self, db: &rusqlite::Connection) -> (String, std::path::PathBuf) {
        let (id, refs): (String, String) = db.query_row(
            "SELECT id,artifact_refs_json FROM agent_evidence_nodes WHERE root_run_id=?1 AND kind='request_record' AND provenance='observed'",
            [&self.root], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let refs: Vec<String> = serde_json::from_str(&refs).unwrap();
        let name = refs.iter().find_map(|r| r.strip_prefix("agent-http/")).unwrap().to_string();
        (id, self.h.context.target_dir.join(AGENT_HTTP_DIRECTORY).join(name))
    }
    fn input_parser_rows(&self, db: &rusqlite::Connection) -> i64 {
        db.query_row("SELECT (SELECT count(*) FROM agent_assignments WHERE role='input_parser')+(SELECT count(*) FROM agent_runs WHERE role='input_parser')",
            [], |r| r.get(0)).unwrap()
    }
}
fn input_parser_root_fixture(tag: &str, suggestions: &'static [&'static str]) -> InputParserRootFixture {
    input_parser_root_fixture_limits(tag, suggestions, (60000, 20))
}
// U177: same actual fixture with explicit frozen hard (tokens, model requests).
fn input_parser_root_fixture_limits(tag: &str, suggestions: &'static [&'static str], limits: (i64, i64)) -> InputParserRootFixture {
    input_parser_root_fixture_inner(tag, suggestions, limits, false, input_parser_child_valid_reply, 1)
}
// Original v5 calibrated token/request boundaries; defaults still create v6.
fn input_parser_root_fixture_original_v5_limits(tag: &str, suggestions: &'static [&'static str], limits: (i64, i64)) -> InputParserRootFixture {
    input_parser_root_fixture_protocol(tag, suggestions, limits, false, input_parser_child_valid_reply, 1, true)
}
// U180: same real fixture, but drive the production finish hook end-to-end.
// U181 (fixture changed, reason: the flag is flipped): the finish is driven with NO
// test-only dispatch override — the real production gate (the flipped flag) opens the
// InputParser branch.
fn input_parser_production_fixture(tag: &str) -> InputParserRootFixture {
    input_parser_root_fixture_inner(tag, &["dispatch:input_parser"], (60000, 20), true, input_parser_child_valid_reply, 1)
}
// U181: same real fixture (finish not driven) whose provider answers the dispatched
// readonly InputParser child with `child_reply` (status, body) — used to inject SDK
// failures / schema-invalid output into the flipped production entry.
fn input_parser_root_fixture_reply(tag: &str, child_reply: fn() -> (u16, String)) -> InputParserRootFixture {
    input_parser_root_fixture_inner(tag, &["dispatch:input_parser"], (60000, 20), false, child_reply, 1)
}
fn input_parser_child_valid_reply() -> (u16, String) {
    (200, proposal_model_response(IP_SDK_VALID_ASSESSMENT))
}
fn input_parser_root_fixture_inner(tag: &str, suggestions: &'static [&'static str], limits: (i64, i64), dispatch_in_production: bool, child_reply: fn() -> (u16, String), http_nodes: usize) -> InputParserRootFixture {
    input_parser_root_fixture_protocol(tag, suggestions, limits, dispatch_in_production, child_reply, http_nodes, false)
}
fn input_parser_root_fixture_protocol(tag: &str, suggestions: &'static [&'static str], limits: (i64, i64), dispatch_in_production: bool, child_reply: fn() -> (u16, String), http_nodes: usize, original_v5: bool) -> InputParserRootFixture {
    use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};
    let mut h = fresh_multi_production_harness(tag);
    let target = h.context.target_url.clone();
    let database = h.db_path.clone();
    let boundary: InputParserRootBoundary = Arc::new(std::sync::Mutex::new(None));
    let fault = boundary.clone();
    let new_evidence_reply: InputParserTestReply = Arc::new(std::sync::Mutex::new(None));
    let evidence_reply = new_evidence_reply.clone();
    let executor_calls = AtomicUsize::new(0);
    let (port, seen, model_stop) = spawn_endpoint(Arc::new(move |wire| {
        if wire.contains("你是独立 InputParser 只读输入面专家") {
            // U180: answer the dispatched readonly InputParser child (by default with a
            // valid, strictly-schematized parameter assessment; U181: `child_reply`).
            let (status, body) = child_reply();
            return (status, "application/json", body);
        }
        let response = if wire.contains("你是独立ClientSide配置事实专家") {
            // U180: the already-shipped ClientSide readonly child runs first inside the
            // production finish hook; answer it with a valid, observation-grounded fact
            // so the pipeline proceeds to the InputParser branch under test.
            let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let refs: Vec<_> = input["observations"].as_array().map(|a| a.iter().map(|o| o["id"].clone()).collect()).unwrap_or_default();
            proposal_model_response(&json!({"summary":"冻结本地配置观察已记录；无浏览器影响证明",
                "observationRefs":refs,"gaps":["missing_browser_validation"],"candidates":[]}).to_string())
        } else if wire.contains("You are the Root Coordinator") {
            // Consumption and normal-close capability supervision are real
            // paid Root requests. Match the actual frozen phase, rather than
            // protocol words nested in bootstrap/changed-fact metadata.
            let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let phase = input["basis"]["phase"].as_str().unwrap_or("");
            if let Some(step) = match phase {
                "budget-consumption" => Some("assess:budget_consumption"),
                "http-budget-consumption" => Some("assess:http_budget_consumption"),
                "capability-changed" => Some("assess:capability_changed"),
                _ => None,
            } {
                proposal_model_response(&json!({"schemaVersion":1,
                    "observed":["original closed paid Web source"],"missing":[],
                    "suggestions":[step],"costNotes":[],"risks":[]}).to_string())
            } else if wire.contains("lease-expired") {
                if let Some(apply) = fault.lock().unwrap().take() {
                    let db = rusqlite::Connection::open_with_flags(&database,rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
                    apply(&db);
                }
                proposal_model_response(&json!({"schemaVersion":1,"observed":["original signed expired InputParser delivery"],"missing":[],"suggestions":["assess:lease_expired"],"costNotes":[],"risks":[]}).to_string())
            } else if wire.contains("input-parser-output") {
                // The paid InputParser-frame Root SDK has actually arrived.
                if let Some(apply) = fault.lock().unwrap().take() {
                    let db = rusqlite::Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
                    apply(&db);
                }
                proposal_model_response(&json!({"schemaVersion":1,"observed":["frozen observed query parameter"],
                    "missing":["parameter semantics unassessed"],"suggestions":suggestions,"costNotes":[],"risks":[]}).to_string())
            } else if wire.contains("new-evidence") {
                if let Some(apply) = fault.lock().unwrap().take() {
                    let db = rusqlite::Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
                    apply(&db);
                }
                if let Some(reply) = evidence_reply.lock().unwrap().as_mut() {
                    return (200, "application/json", reply(&wire));
                }
                proposal_model_response(&json!({"schemaVersion":1,"observed":["original paid closed HTTP observation"],
                    "missing":[],"suggestions":["assess:new_evidence"],"costNotes":[],"risks":[]}).to_string())
            } else if wire.contains("client-side-output") {
                // U180: the already-shipped ClientSide readonly path also runs a paid Root
                // tick inside the production finish hook; bound its client step so the
                // pipeline proceeds to the InputParser branch under test.
                proposal_model_response(&json!({"schemaVersion":1,"observed":["paid readonly configuration observations"],
                    "missing":["missing_browser_validation"],"suggestions":["assess:client_side_configuration"],"costNotes":[],"risks":[]}).to_string())
            } else {
                fresh_multi_root_wire_response(&wire).unwrap()
            }
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(r#"{"summary":"frozen frontend map only","priorityContracts":[],"risks":[]}"#)
        } else if executor_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            let tools: Vec<_> = (1..=http_nodes).map(|page| ("replay_http", json!({"identity":"anonymous","method":"GET",
                "url":format!("{target}/api/orders?page={page}"),"family":"authorization"}))).collect();
            model_round(&tools, 10)
        } else {
            model_round(&[], 10)
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = limits.0 as _;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = limits.1 as _;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = 8;
    if original_v5 { freeze_fresh_multi_production_harness_original_v5(&mut h); }
    else { freeze_fresh_multi_production_harness(&mut h); }
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    let mut session = multi_agent_prepare(&mut h.context).unwrap();
    let outcome = run_native_agent(&h.context);
    assert!(matches!(outcome, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{}", outcome.detail());
    if dispatch_in_production {
        // U180: drive the real production finish hook (Executor settle -> client_side ->
        // input_parser). U181: no test-only override — the dispatch is opened by the
        // flipped production flag alone.
        assert!(!input_parser_test_flag_forced_off(), "U181: no simulated flag-off");
        assert!(native_root_role_prepare_implemented(crate::agent_runtime::contract::AgentRole::InputParser));
        assert!(input_parser_dispatch_enabled(), "the production gate itself is open");
        multi_agent_finish_execution(&h.context, &mut session, &outcome).unwrap();
    }
    InputParserRootFixture { h, session: Some(session), root, boundary, model_stop, outcome, new_evidence_reply }
}
fn input_parser_root_wire_input(f: &InputParserRootFixture) -> JsonValue {
    let wires = f.h.model_seen.lock().unwrap();
    let wire = wires.iter().rev().find(|w| w.contains("input-parser-output")).unwrap();
    let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
    serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap()
}
