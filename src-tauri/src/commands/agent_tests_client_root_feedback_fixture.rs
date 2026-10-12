// Actual fresh creator + independent Mapper + SDK executor loop + Broker GET.
type ClientRootBoundary = std::sync::Arc<std::sync::Mutex<Option<Box<dyn FnOnce(&rusqlite::Connection) + Send>>>>;
struct ClientRootFixture {
    h: AgentHarness,
    session: Option<MultiAgentSession>,
    outcome: AgentTargetOutcome,
    root: String,
    boundary: ClientRootBoundary,
    model_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for ClientRootFixture {
    fn drop(&mut self) {
        drop(self.session.take());
        self.model_stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = fs::remove_dir_all(&self.h.root);
    }
}
impl ClientRootFixture {
    fn finish(&mut self) -> Result<(), String> {
        multi_agent_finish_execution(&self.h.context, self.session.as_mut().unwrap(), &self.outcome)
    }
    fn child(&self, db: &rusqlite::Connection) -> crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        let (assignment_id,run_id) = db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='client_side'", [&self.root], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id,run_id,role:crate::agent_runtime::contract::AgentRole::ClientSide}
    }
    fn feedback(&self, db: &rusqlite::Connection) -> Result<(), String> {
        native_coordinator_client_feedback(&self.h.context, self.session.as_ref().unwrap(), &self.child(db))
    }
    fn root_cost(&self, db: &rusqlite::Connection) -> i64 {
        crate::agent_runtime::multi_agent::budget::balance(db,&self.root,Some(""),"model_requests").unwrap().consumed
    }
}
fn client_root_fixture(tag: &str, forbidden: bool) -> ClientRootFixture {
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
    let mut h = fresh_multi_production_harness(tag);
    let target = h.context.target_url.clone();
    let database = h.db_path.clone();
    let boundary: ClientRootBoundary = Arc::new(std::sync::Mutex::new(None));
    let fault = boundary.clone();
    let executor_calls = AtomicUsize::new(0);
    let (port, seen, model_stop) = spawn_endpoint(Arc::new(move |wire| {
        let response = if wire.contains("You are the Root Coordinator") {
            if wire.contains("client-side-output") {
                // The feedback SDK has actually arrived; all earlier grant,
                // executor, receipt, Client SDK and ACK operations succeeded.
                if let Some(apply) = fault.lock().unwrap().take() {
                    let db = rusqlite::Connection::open_with_flags(&database, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE).unwrap();
                    apply(&db);
                }
                proposal_model_response(&json!({"schemaVersion":1,"observed":["paid readonly configuration observations"],
                    "missing":["missing_browser_validation"],"suggestions":if forbidden {vec!["dispatch:web_executor"]}else{vec!["assess:client_side_configuration"]},"costNotes":[],"risks":[]}).to_string())
            } else { fresh_multi_root_wire_response(&wire).unwrap() }
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(r#"{"summary":"frozen frontend map only","priorityContracts":[],"risks":[]}"#)
        } else if wire.contains("独立ClientSide配置事实专家") {
            let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let refs: Vec<_> = input["observations"].as_array().unwrap().iter().map(|o|o["id"].clone()).collect();
            proposal_model_response(&json!({"summary":"readonly original HTTP configuration; no impact proof", "observationRefs":refs,
                "gaps":["missing_browser_validation"],"candidates":[]}).to_string())
        } else if executor_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            model_round(&[("replay_http", json!({"identity":"anonymous","method":"GET","url":format!("{target}/"),"family":"authorization"}))], 10)
        } else { model_round(&[], 10) };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = 20;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = 8;
    freeze_fresh_multi_production_harness_original_v6(&mut h);
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let outcome = run_native_agent(&h.context);
    assert!(matches!(outcome, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{}", outcome.detail());
    ClientRootFixture {h,session:Some(session),outcome,root,boundary,model_stop}
}
