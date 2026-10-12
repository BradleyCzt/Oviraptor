// Genuine pre-birth v8, Web SDK and production localhost HTTP headers writer.
fn http_budget_receipt_actual_fixture() -> BudgetTriggerFixture {
    http_budget_receipt_fixture(false)
}

fn http_budget_receipt_fixture(final_executor: bool) -> BudgetTriggerFixture {
    let mut h = fresh_multi_production_harness("http-budget-receipt-v8");
    let target = h.context.target_url.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let response = if wire.contains("You are the Root Coordinator") && wire.contains("client-side-output") {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original HTTP configuration"],
                "missing":["missing_browser_validation"],"suggestions":["assess:client_side_configuration"],"costNotes":[],"risks":[]}).to_string())
        } else if wire.contains("You are the Root Coordinator") && wire.contains("http-budget-consumption") {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original sealed HTTP cost"],
                "missing":[],"suggestions":["assess:http_budget_consumption"],"costNotes":[],"risks":[]}).to_string())
        } else if wire.contains("You are the Root Coordinator") && wire.contains("budget-consumption") {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original paid cost"],
                "missing":[],"suggestions":["assess:budget_consumption"],"costNotes":[],"risks":[]}).to_string())
        } else if let Some(root) = fresh_multi_root_wire_response(&wire) { root }
        else if wire.contains("SPA/API Mapper") {
            proposal_model_response(r#"{"summary":"original observed Mapper","priorityContracts":[],"risks":[]}"#)
        } else if wire.contains("独立ClientSide配置事实专家") {
            let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let refs: Vec<_> = input["observations"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect();
            proposal_model_response(&json!({"summary":"original observed HTTP configuration", "observationRefs":refs,
                "gaps":["missing_browser_validation"],"candidates":[]}).to_string())
        } else if final_executor {
            model_round(&[("replay_http", json!({"identity":"anonymous","method":"GET",
                "url":format!("{target}/"),"family":"authorization"}))], 10)
        } else { model_round(&[], 10) };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = 20;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = if final_executor { 1 } else { 8 };
    freeze_fresh_multi_production_harness_http_v8(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let f = BudgetTriggerFixture { h, session: Some(session),
        boundary: std::sync::Arc::new(std::sync::Mutex::new(None)), stop };
    if final_executor { return f; }
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
    let reply = native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"original paid Web before sealed HTTP"})], &[], 1)
        .unwrap_or_else(|_| panic!("actual Web SDK must settle"));
    let run = f.h.context.run.as_ref().unwrap();
    run.model_round(1, &reply.usage, &["replay_http".into()]).unwrap();
    let arguments = json!({"identity":"anonymous","method":"GET",
        "url":format!("{}/", f.h.context.target_url),"family":"authorization"});
    let mut runtime = AgentToolRuntime::default();
    let invocation = runtime.begin_invocation(1);
    run.begin_tool("replay_http", &arguments, &invocation).unwrap();
    let result = agent_execute_tool_at(&f.h.context, &mut runtime, "replay_http", &arguments, &invocation);
    assert!(value_first(&result.model_view, &["code"]).is_empty(), "{}", result.model_view);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    f
}

#[test]
fn http_budget_receipt_actual_next_sdk_pays_original_http_assessment_once() {
    let _real = RealSpecialistTransport::enter();
    let f = http_budget_receipt_actual_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    let count = |db: &rusqlite::Connection| db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='http-budget-consumption'",
        [root], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count(&db), 0);
    let before_sdk = f.h.model_seen.lock().unwrap().len();
    let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
    native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"next actual SDK after sealed HTTP"})], &[], 2)
        .unwrap_or_else(|_| panic!("new SDK must follow original HTTP assessment"));
    assert_eq!(count(&db), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), before_sdk + 2);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    native_coordinator_http_budget_consumption_before_action(&f.h.context).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn http_budget_receipt_missing_claim_and_seal_cannot_hide_original_paid_http() {
    let _real = RealSpecialistTransport::enter();
    let f = http_budget_receipt_actual_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let actor = &f.session.as_ref().unwrap().lease;
    let delete: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='http_budget_receipt_no_delete'",
        [], |r| r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER http_budget_receipt_no_delete").unwrap();
    assert_eq!(db.execute("DELETE FROM agent_http_budget_receipts WHERE root_run_id=?1", [&actor.root_run_id]).unwrap(), 1);
    db.execute_batch(&delete).unwrap();
    assert_eq!(db.execute("DELETE FROM agent_http_request_claims WHERE run_id=?1", [&f.session.as_ref().unwrap().executor.run_id]).unwrap(), 1);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert_eq!(native_coordinator_http_budget_consumption_before_action(&f.h.context).unwrap_err(),
        "root_http_budget_original_receipt_missing");
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn http_budget_receipt_actual_last_http_is_assessed_before_final_envelope_and_reentry_is_free() {
    let _real = RealSpecialistTransport::enter();
    let mut f = http_budget_receipt_fixture(true);
    let outcome = run_native_agent(&f.h.context);
    assert!(matches!(outcome, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{}", outcome.detail());
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let assignment = f.session.as_ref().unwrap().executor.assignment_id.clone();
    let count = |db: &rusqlite::Connection| db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='http-budget-consumption'",
        [&root], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count(&db), 0);
    let fees = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'http:%' ORDER BY rowid", [&root]).unwrap();
    // At the actual INSERT boundary, the last original HTTP assessment must
    // already have a paid publication; otherwise no envelope can be written.
    db.execute_batch("CREATE TRIGGER final_http_assessment_order BEFORE INSERT ON agent_messages WHEN NEW.kind='execution_result' BEGIN SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='http-budget-consumption') THEN RAISE(ABORT,'final_http_assessment_missing_before_envelope') END; END;").unwrap();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(count(&db), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",
        [&assignment], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'http:%' ORDER BY rowid", [&root]).unwrap() == fees);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn http_budget_receipt_actual_headers_seal_is_exact_readonly_and_non_authorizing() {
    use crate::agent_runtime::web_mode::http_budget_storage;
    let _real = RealSpecialistTransport::enter();
    let f = http_budget_receipt_actual_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let actor = &f.session.as_ref().unwrap().lease;
    let source: String = db.query_row("SELECT source_id FROM agent_http_budget_receipts WHERE root_run_id=?1",
        [&actor.root_run_id], |r| r.get(0)).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_http_budget_receipts WHERE length(signature)=32 AND response_status=200",
        [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let snapshot = db.unchecked_transaction().unwrap();
    let saved = http_budget_storage::read_original_on(&snapshot, actor, &source).unwrap().unwrap();
    assert_eq!(saved.fact()["status"], 200);
    assert_eq!(saved.fact()["targetRequests"], 1);
    assert_eq!(saved.fact()["rootRunId"], actor.root_run_id);
    assert_eq!(http_budget_storage::read_original_on(&snapshot, actor, &source).unwrap().unwrap().receipt_id(), saved.receipt_id());
    let mut foreign = actor.clone();
    foreign.scan_id = "foreign-scan".into();
    assert!(http_budget_storage::read_original_on(&snapshot, &foreign, &source).is_err());
    foreign = actor.clone();
    foreign.root_run_id = "foreign-root".into();
    assert!(http_budget_storage::read_original_on(&snapshot, &foreign, &source).unwrap().is_none());
    let child = &f.session.as_ref().unwrap().executor;
    assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&snapshot, &actor.root_run_id,
        Some(&child.assignment_id), "target_requests").unwrap().consumed, 1);
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert!(db.execute("UPDATE agent_http_budget_receipts SET signature=zeroblob(32)", []).is_err());
    assert!(db.execute("DELETE FROM agent_http_budget_receipts", []).is_err());
}

#[test]
fn http_budget_receipt_cold_signature_damage_and_missing_receipt_never_backfill() {
    use crate::agent_runtime::web_mode::http_budget_storage;
    let _real = RealSpecialistTransport::enter();
    let f = http_budget_receipt_actual_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let actor = &f.session.as_ref().unwrap().lease;
    let source: String = db.query_row("SELECT source_id FROM agent_http_budget_receipts WHERE root_run_id=?1",
        [&actor.root_run_id], |r| r.get(0)).unwrap();
    let update: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='http_budget_receipt_no_update'",
        [], |r| r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER http_budget_receipt_no_update").unwrap();
    assert_eq!(db.execute("UPDATE agent_http_budget_receipts SET signature=zeroblob(32) WHERE root_run_id=?1", [&actor.root_run_id]).unwrap(), 1);
    db.execute_batch(&update).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let snapshot = db.unchecked_transaction().unwrap();
    assert!(http_budget_storage::read_original_on(&snapshot, actor, &source).is_err());
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let delete: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='http_budget_receipt_no_delete'",
        [], |r| r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER http_budget_receipt_no_delete").unwrap();
    assert_eq!(db.execute("DELETE FROM agent_http_budget_receipts WHERE root_run_id=?1", [&actor.root_run_id]).unwrap(), 1);
    db.execute_batch(&delete).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert!(http_budget_storage::read_original_on(&snapshot, actor, &source).unwrap().is_none());
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_http_request_claims WHERE response_status=200", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
#[test]
fn http_budget_receipt_cold_original_fee_rows_cannot_validate_saved_seal() {
    use crate::agent_runtime::web_mode::http_budget_storage;
    let _real = RealSpecialistTransport::enter();
    for kind in ["reserve", "forfeit", "reconcile"] {
        for column in ["amount", "source_id", "lease_attempt_id"] {
            let f = http_budget_receipt_actual_fixture();
            let db = db::open(&f.h.db_path).unwrap();
            let actor = &f.session.as_ref().unwrap().lease;
            let source: String = db.query_row("SELECT source_id FROM agent_http_budget_receipts WHERE root_run_id=?1",
                [&actor.root_run_id], |r| r.get(0)).unwrap();
            let trigger: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='budget_entry_no_update'",
                [], |r| r.get(0)).unwrap();
            db.execute_batch("DROP TRIGGER budget_entry_no_update").unwrap();
            let expression = if column == "amount" { "amount+1" } else { "'foreign-original-source'" };
            assert_eq!(db.execute(&format!("UPDATE agent_budget_entries SET {column}={expression} WHERE root_run_id=?1 AND source_id=?2 AND kind=?3 AND dimension='target_requests'"),
                params![actor.root_run_id, source, kind]).unwrap(), 1);
            db.execute_batch(&trigger).unwrap();
            let before = super::tests::application_table_snapshot(&db);
            let sdk = f.h.model_seen.lock().unwrap().len();
            let snapshot = db.unchecked_transaction().unwrap();
            assert!(http_budget_storage::read_original_on(&snapshot, actor, &source).is_err(), "{kind}/{column}");
            snapshot.rollback().unwrap();
            assert_eq!(super::tests::application_table_snapshot(&db), before);
            assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
            assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
        }
    }
}

#[test]
fn http_budget_receipt_schema_shadow_and_extra_trigger_are_rejected() {
    use crate::agent_runtime::web_mode::http_budget_storage;
    let _real = RealSpecialistTransport::enter();
    let f = http_budget_receipt_actual_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let actor = &f.session.as_ref().unwrap().lease;
    let source: String = db.query_row("SELECT source_id FROM agent_http_budget_receipts WHERE root_run_id=?1",
        [&actor.root_run_id], |r| r.get(0)).unwrap();
    for (install, remove) in [
        ("CREATE TEMP TABLE agent_http_budget_receipts(fake TEXT)", "DROP TABLE temp.agent_http_budget_receipts"),
        ("CREATE TRIGGER http_extra_reader_trigger AFTER INSERT ON agent_http_budget_receipts BEGIN SELECT 1; END", "DROP TRIGGER http_extra_reader_trigger"),
    ] {
        db.execute_batch(install).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        assert_eq!(http_budget_storage::read_original_on(&snapshot, actor, &source).err().unwrap(), "http_budget_receipt_schema_conflict");
        snapshot.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        db.execute_batch(remove).unwrap();
    }
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn http_budget_receipt_replace_is_immutable_and_cold_rule_removal_cannot_fallback() {
    let _real = RealSpecialistTransport::enter();
    let f = http_budget_receipt_actual_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(db.execute("INSERT OR REPLACE INTO agent_http_budget_receipts SELECT * FROM agent_http_budget_receipts", []).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let actor = &f.session.as_ref().unwrap().lease;
    let raw: String = db.query_row("SELECT definition_json FROM agent_root_mode_definitions WHERE root_run_id=?1",
        [&actor.root_run_id], |r| r.get(0)).unwrap();
    let fragment = "\"httpBudgetConsumptionRule\":\"original_received_http_target_cost_advisory_v1\"";
    assert_eq!(raw.matches(fragment).count(), 1);
    let needle = if raw.contains(&format!("{fragment},")) { format!("{fragment},") } else { format!(",{fragment}") };
    assert_eq!(raw.matches(&needle).count(), 1);
    let damaged = raw.replacen(&needle, "", 1);
    let _: JsonValue = serde_json::from_str(&damaged).unwrap();
    let trigger: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='root_mode_immutable_update'", [], |r| r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER root_mode_immutable_update").unwrap();
    db.execute("UPDATE agent_root_mode_definitions SET definition_json=?1 WHERE root_run_id=?2", params![damaged, actor.root_run_id]).unwrap();
    db.execute_batch(&trigger).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(native_coordinator_http_budget_consumption_before_action(&f.h.context).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
#[test]
fn http_budget_receipt_fresh_witness_refuses_another_writer_connection() {
    let _real = RealSpecialistTransport::enter();
    let f = http_budget_receipt_actual_fixture();
    let run = f.h.context.run.as_ref().unwrap();
    let arguments = json!({"identity":"anonymous","method":"GET",
        "url":format!("{}/", f.h.context.target_url),"family":"authorization"});
    let mut runtime = AgentToolRuntime {target_requests:1, ..Default::default()};
    runtime.set_invocation("inv-1-002");
    run.begin_tool("replay_http", &arguments, "inv-1-002").unwrap();
    let request = AgentHttpRequest {identity:AgentIdentity::anonymous(), method:"GET".into(),
        url:format!("{}/", f.h.context.target_url), extra_headers:vec![], body:None,
        content_type:None,contract_key:String::new(),family:"authorization".into(),
        source:ScopeSource::IdentityComparison,tool:"replay_http".into(),timeout_seconds:5};
    // A legitimate pending pre-send claim owns its private original writer.
    // This test deliberately never sends its second target request.
    let transport = claim_agent_http_sealed_transport_with_headers(&f.h.context, &runtime,
        &request, &request.url, &[]).unwrap();
    let witness = transport.witness.as_ref().unwrap();
    let foreign_db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&foreign_db);
    let snapshot = foreign_db.unchecked_transaction().unwrap();
    assert_eq!(witness.received_fact_on(&snapshot, 200).err().unwrap(),
        "http_budget_receipt_original_writer_required");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&foreign_db), before);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
