// Real executor outcomes and checkpoints, never a manufactured completion.
#[test]
fn budget_consumption_final_request_cap_retains_typed_code_despite_token_usage_summary() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_consumption_actual_fixture_request_limit(false, 12);
    let outcome = run_native_agent(&f.h.context);
    assert!(matches!(outcome, AgentTargetOutcome::Incomplete(_)), "{}", outcome.detail());
    assert!(outcome.detail().contains("模型调用达到 1 次硬上限"), "{}", outcome.detail());
    assert!(outcome.detail().contains("Token"), "the real ledger summary must retain its Token usage");
    assert_eq!(outcome.terminal_code(), AGENT_STOP_HARD_REQUESTS);
    let checkpoint = NativeAgentState::read(&f.h.db_path, &f.h.context.scan_id, &f.h.context.target_url).unwrap();
    assert_eq!(checkpoint.turns, 1);
    assert_eq!(checkpoint.token_usage.total_tokens, 50);
    assert!(checkpoint.terminal_reason.is_empty(), "the original incomplete checkpoint remains resumable");
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn budget_consumption_final_original_wall_cap_expiry_keeps_paid_fees_without_envelope() {
    let _real = RealSpecialistTransport::enter();
    let mut h = fresh_multi_production_harness("final-original-wall-cap");
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let response = if wire.contains("You are the Root Coordinator") && wire.contains("budget-consumption") {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original paid source"],
                "missing":[],"suggestions":["assess:budget_consumption"],"costNotes":[],"risks":[]}).to_string())
        } else if let Some(root) = fresh_multi_root_wire_response(&wire) { root }
        else if wire.contains("SPA/API Mapper") {
            proposal_model_response(r#"{"summary":"actual closed original Mapper","priorityContracts":[],"risks":[]}"#)
        } else { model_round(&[], 10) };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = 12;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = 8;
    // This is specifically the wall_time_ms dimension of original Root cap;
    // it is not evidence of exhausted model token/request dimensions.
    h.context.execution_plan.timeout_seconds = 10;
    freeze_fresh_multi_production_harness(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let mut f = BudgetTriggerFixture { h, session: Some(session),
        boundary: std::sync::Arc::new(std::sync::Mutex::new(None)), stop };
    let outcome = run_native_agent(&f.h.context);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let assignment = f.session.as_ref().unwrap().executor.assignment_id.clone();
    match &outcome {
        AgentTargetOutcome::Incomplete(stop) if stop.code == AGENT_STOP_HARD_REQUESTS => {
            assert!(outcome.detail().contains("模型调用达到 1 次硬上限"), "{}", outcome.detail());
        }
        AgentTargetOutcome::ResumeIncompatible(_) => {
            assert_eq!(outcome.detail(), "原模型账单已保存；此执行分支的权限已失效，停止使用该响应及派发后续动作");
            assert_eq!(crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &root).unwrap_err(),
                "budget_wall_time_exhausted", "the saved-bill rejection must be actual original deadline expiry");
        }
        _ => panic!("unexpected actual stop before deadline check: {}: {}", outcome.terminal_code(), outcome.detail()),
    }
    assert_eq!(db.query_row("SELECT count(*) FROM agent_web_model_journal WHERE root_run_id=?1 AND phase='received'",
        [&root], |r| r.get::<_, i64>(0)).unwrap(), 1);
    let paid_requests = crate::agent_runtime::multi_agent::budget::balance(&db, &root,
        Some(&assignment), "model_requests").unwrap();
    assert_eq!((paid_requests.consumed, paid_requests.reserved, paid_requests.indeterminate), (1, 0, 0));
    let paid = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'web-model:%' ORDER BY rowid", [&root]).unwrap();
    match crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &root) {
        Ok(remaining) => std::thread::sleep(remaining + Duration::from_millis(100)),
        Err(error) => assert_eq!(error, "budget_wall_time_exhausted"),
    }
    assert_eq!(crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &root).unwrap_err(),
        "budget_wall_time_exhausted");
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).is_err());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",
        [&assignment], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'web-model:%' ORDER BY rowid", [&root]).unwrap() == paid);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn budget_consumption_final_actual_executor_last_call_assessed_before_envelope_once() {
    let _real = RealSpecialistTransport::enter();
    let mut f = budget_consumption_actual_fixture(false);
    let outcome = run_native_agent(&f.h.context);
    assert!(matches!(outcome, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_) | AgentTargetOutcome::Limited(_)), "{}", outcome.detail());
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    let root = session.lease.root_run_id.clone();
    let assignment = session.executor.assignment_id.clone();
    let checkpoint = NativeAgentState::read(&f.h.db_path, &f.h.context.scan_id, &f.h.context.target_url).unwrap();
    assert!(checkpoint.turns > 0);
    let received: i64 = db.query_row("SELECT count(*) FROM agent_web_model_journal WHERE root_run_id=?1 AND phase='received'",
        [&root], |r| r.get(0)).unwrap();
    assert_eq!(received, checkpoint.turns);
    let count = |db: &rusqlite::Connection| db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='budget-consumption'",
        [&root], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count(&db), received - 1, "the final SDK has no subsequent action to trigger supervision");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",
        [&assignment], |r| r.get::<_, i64>(0)).unwrap(), 0);
    let original_calls = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid", [&root]).unwrap();
    let original_fees = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'web-model:%' ORDER BY rowid", [&root]).unwrap();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(count(&db), received);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",
        [&assignment], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid", [&root]).unwrap() == original_calls);
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'web-model:%' ORDER BY rowid", [&root]).unwrap() == original_fees);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn budget_consumption_final_actual_unknown_cost_cannot_publish_finished_envelope() {
    let _real = RealSpecialistTransport::enter();
    let mut f = budget_consumption_actual_fixture(true);
    let outcome = run_native_agent(&f.h.context);
    assert_eq!(outcome.terminal_code(), terminal_code::REQUEST_RECONCILIATION_REQUIRED, "{}", outcome.detail());
    let db = db::open(&f.h.db_path).unwrap();
    let assignment = f.session.as_ref().unwrap().executor.assignment_id.clone();
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).is_err());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",
        [&assignment], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn budget_consumption_final_withdrawn_original_coordinator_cannot_publish_finished_envelope() {
    let _real = RealSpecialistTransport::enter();
    let mut f = budget_consumption_actual_fixture(false);
    let outcome = run_native_agent(&f.h.context);
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    let assignment = session.executor.assignment_id.clone();
    assert_eq!(db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1 WHERE root_run_id=?2",
        params![Uuid::new_v4().to_string(), session.lease.root_run_id]).unwrap(), 1);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).is_err());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",
        [&assignment], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}
