// Separate each current financial advisory from the original business flow,
// binding all four usage dimensions to its original paid publication and fee.
fn unlimited_execution_financial_usage(
    f: &BudgetTriggerFixture,
    expected_web: usize,
) -> std::collections::BTreeMap<&'static str, i64> {
    let db = db::open(&f.h.db_path).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    let session = f.session.as_ref().unwrap();
    let root = &session.lease.root_run_id;
    let phases = followup_original_root_phase_counts(&snapshot, root, &f.h.model_seen.lock().unwrap());
    let expected: std::collections::BTreeMap<String, usize> = [
        ("bootstrap", 1), ("mapper-output", 1), ("budget-allocation", 1),
        ("new-evidence", 1), ("client-side-output", 1),
        ("budget-consumption", expected_web), ("http-budget-consumption", 1),
    ].into_iter().map(|(phase, count)| (phase.into(), count)).collect();
    assert_eq!(phases, expected, "only the original business phases and original financial advisories may be paid");
    let web_calls: Vec<String> = snapshot.prepare(
        "SELECT call_id FROM agent_web_model_journal WHERE root_run_id=?1 AND child_run_id=?2 AND phase='received' ORDER BY round"
    ).unwrap().query_map(params![root, session.executor.run_id], |row| row.get(0)).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(web_calls.len(), expected_web);
    let advisories: Vec<(String, String, String)> = snapshot.prepare(
        "SELECT q.call_id,q.fact_json,p.fact_json FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.root_run_id=q.root_run_id AND p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase') IN ('budget-consumption','http-budget-consumption') ORDER BY q.round"
    ).unwrap().query_map([root], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(advisories.len(), expected_web + 1);
    let mut totals = std::collections::BTreeMap::from([
        ("model_input_tokens", 0), ("model_cached_tokens", 0),
        ("model_output_tokens", 0), ("model_requests", 0),
    ]);
    let mut original_calls = std::collections::HashSet::new();
    let mut original_http = std::collections::HashSet::new();
    for (call, raw, publication) in advisories {
        let saved: JsonValue = serde_json::from_str(&raw).unwrap();
        let phase = saved["request"]["basis"]["phase"].as_str().unwrap();
        let fact = &saved["request"]["basis"]["changedFact"]["semantic"];
        assert_eq!(fact["rootRunId"], root.as_str());
        assert_eq!(fact["assignmentId"], session.executor.assignment_id);
        assert_eq!(fact["childRunId"], session.executor.run_id);
        assert_eq!(fact["advisoryOnly"], true);
        assert_eq!(fact["executionGrant"], false);
        assert_eq!(fact["additionalScopeGranted"], false);
        assert_eq!(fact["targetRequestsGranted"], 0);
        if phase == "budget-consumption" {
            let web_call = fact["callId"].as_str().unwrap();
            assert!(original_calls.insert(web_call.to_string()), "each original Web bill is assessed once");
            assert!(web_calls.iter().any(|original| original == web_call));
            let raw: String = snapshot.query_row(
                "SELECT receipt_json FROM agent_web_model_journal WHERE root_run_id=?1 AND call_id=?2 AND child_run_id=?3 AND assignment_id=?4 AND phase='received'",
                params![root, web_call, session.executor.run_id, session.executor.assignment_id], |row| row.get(0),
            ).unwrap();
            let received: JsonValue = serde_json::from_str(&raw).unwrap();
            assert_eq!(received["usageReported"], true);
            assert_eq!(received["usage"], fact["usage"]);
            assert_eq!(received["responseHash"], fact["responseHash"]);
            assert_eq!(fact["usage"]["modelRequests"], 1);
            let source = format!("web-model:{}:{}:{}:receipt:{}",
                session.executor.assignment_id, fact["round"].as_i64().unwrap(),
                fact["requestHash"].as_str().unwrap(), fact["responseHash"].as_str().unwrap());
            for (dimension, field) in [
                ("model_input_tokens", "inputTokens"), ("model_cached_tokens", "cachedInputTokens"),
                ("model_output_tokens", "outputTokens"), ("model_requests", "modelRequests"),
            ] {
                let amount = fact["usage"][field].as_i64().unwrap();
                let original_fee: (i64, i64) = snapshot.query_row(
                    "SELECT count(*),coalesce(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 AND dimension=?5 AND kind='consume'",
                    params![root, session.executor.assignment_id, fact["workerAttemptId"].as_str().unwrap(), source, dimension],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                ).unwrap();
                assert_eq!(original_fee, (i64::from(amount > 0), amount));
            }
        } else {
            assert_eq!(phase, "http-budget-consumption");
            assert_eq!(fact["protocol"], "original_received_http_target_cost_advisory_v1");
            assert_eq!(fact["targetRequests"], 1);
            let source = fact["sourceId"].as_str().unwrap();
            assert!(original_http.insert(source.to_string()));
            let receipt: i64 = snapshot.query_row(
                "SELECT count(*) FROM agent_http_budget_receipts WHERE root_run_id=?1 AND receipt_id=?2 AND source_id=?3 AND run_id=?4 AND scan_id=?5 AND attempt_number=?6 AND target_url=?7 AND response_status=?8",
                params![root, fact["receiptId"].as_str().unwrap(), source, session.executor.run_id,
                    session.lease.scan_id, session.lease.attempt_number, session.lease.target_key,
                    fact["status"].as_i64().unwrap()], |row| row.get(0),
            ).unwrap();
            assert_eq!(receipt, 1);
            let target_fees: Vec<(String, i64)> = snapshot.prepare(
                "SELECT kind,amount FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 AND dimension='target_requests' ORDER BY rowid"
            ).unwrap().query_map(params![root, session.executor.assignment_id, fact["workerAttemptId"].as_str().unwrap(), source],
                |row| Ok((row.get(0)?, row.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
            assert_eq!(target_fees, vec![("reserve".into(), 1), ("forfeit".into(), 1), ("reconcile".into(), 1)]);
        }
        let publication: JsonValue = serde_json::from_str(&publication).unwrap();
        let usage = &publication["event"]["usage"];
        let received: String = snapshot.query_row(
            "SELECT receipt_json FROM agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 AND phase='received'",
            params![root, call], |row| row.get(0),
        ).unwrap();
        let received: JsonValue = serde_json::from_str(&received).unwrap();
        assert_eq!(received["usageReported"], true);
        assert_eq!(received["usage"], *usage, "publication must retain the original known Root bill");
        assert_eq!(usage["modelRequests"], 1);
        for (dimension, field) in [
            ("model_input_tokens", "inputTokens"), ("model_cached_tokens", "cachedInputTokens"),
            ("model_output_tokens", "outputTokens"), ("model_requests", "modelRequests"),
        ] {
            let amount = usage[field].as_i64().unwrap();
            assert!(amount >= 0);
            let fee: (i64, i64) = snapshot.query_row(
                "SELECT count(*),coalesce(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='' AND source_id=?2 AND dimension=?3 AND kind='consume'",
                params![root, call, dimension], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(fee, (i64::from(amount > 0), amount), "one original Root fee per nonzero dimension");
            *totals.get_mut(dimension).unwrap() += amount;
        }
        let projected = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(
            &snapshot, root, publication["eventSequence"].as_i64().unwrap(),
        ).unwrap();
        assert_eq!(projected.record["advisoryOnly"], true);
        assert_eq!(projected.record["summary"]["suggestions"], json!([
            if phase == "budget-consumption" { "assess:budget_consumption" } else { "assess:http_budget_consumption" }
        ]));
        assert!(projected.record.get("localTools").is_none());
    }
    assert_eq!(original_calls.len(), expected_web);
    assert_eq!(original_http.len(), 1);
    assert_eq!(totals["model_requests"], (expected_web + 1) as i64);
    snapshot.rollback().unwrap();
    totals
}

// A real original Web SDK bill, before any advisory or target request. This
// exercises the same foreground frame construction as the complete chain.
fn unlimited_execution_original_paid_web_fixture(limits: (i64, i64)) -> BudgetTriggerFixture {
    let f = unlimited_execution_fixture(limits, 0);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
    let response = native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"one actual original Web bill before reserve verification"})], &[], 1)
        .unwrap_or_else(|_| panic!("original Web SDK bill must be received"));
    f.h.context.run.as_ref().unwrap().model_round(1, &response.usage, &[]).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 5);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    f
}

fn unlimited_execution_assert_bad_reserve_refused(f: &BudgetTriggerFixture) {
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    let before = web_mode_test_rows(&db);
    let calls = f.h.model_seen.lock().unwrap().len();
    assert!(crate::agent_runtime::multi_agent::budget::model::original_web_grant(
        &db, &session.lease, &session.executor.assignment_id).is_err());
    assert!(native_coordinator_budget_consumption_before_action(&f.h.context).is_err());
    web_mode_assert_rows(&db, &before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn unlimited_execution_zero_original_grant_assesses_same_actual_bill_once() {
    let _real = RealSpecialistTransport::enter();
    let f = unlimited_execution_original_paid_web_fixture((0, 0));
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    assert_eq!(crate::agent_runtime::multi_agent::budget::model::original_web_grant(
        &db, &session.lease, &session.executor.assignment_id).unwrap(), (0, 0));
    let call: String = db.query_row(
        "SELECT call_id FROM agent_web_model_journal WHERE root_run_id=?1 AND phase='received'",
        [&session.lease.root_run_id], |row| row.get(0),
    ).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    let frame = NativeCoordinatorFrame::budget_consumption_on(&snapshot, &f.h.context,
        &session.lease, &session.executor, &call).unwrap();
    assert_eq!(frame.semantic["originalTokens"], 0);
    assert_eq!(frame.semantic["originalRequests"], 0);
    frame.verify(&snapshot, &f.h.context, &session.lease).unwrap();
    snapshot.rollback().unwrap();
    native_coordinator_budget_consumption_before_action(&f.h.context).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 6);
    let before = web_mode_test_rows(&db);
    for _ in 0..2 {
        native_coordinator_budget_consumption_before_action(&f.h.context).unwrap();
        let snapshot = db.unchecked_transaction().unwrap();
        frame.verify(&snapshot, &f.h.context, &session.lease).unwrap();
        snapshot.rollback().unwrap();
        web_mode_assert_rows(&db, &before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), 6);
        assert!(f.h.site_seen.lock().unwrap().is_empty());
    }
}

#[test]
fn unlimited_execution_finite_or_mixed_original_reserve_damage_refuses_assessment() {
    let _real = RealSpecialistTransport::enter();
    for limits in [(60_000, 20), (0, 20), (60_000, 0)] {
        for damage in ["missing", "amount"] {
            let f = unlimited_execution_original_paid_web_fixture(limits);
            let db = db::open(&f.h.db_path).unwrap();
            let child = &f.session.as_ref().unwrap().executor;
            let row: i64 = db.query_row(
                "SELECT min(rowid) FROM agent_budget_entries WHERE assignment_id=?1 AND kind='reserve' AND source_id=?2",
                params![child.assignment_id, format!("assignment:{}", child.assignment_id)], |row| row.get(0),
            ).unwrap();
            let guard = if damage == "missing" { "budget_entry_no_delete" } else { "budget_entry_no_update" };
            let trigger: String = db.query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1", [guard], |row| row.get(0),
            ).unwrap();
            db.execute_batch(&format!("DROP TRIGGER {guard}")).unwrap();
            assert_eq!(db.execute(if damage == "missing" {
                "DELETE FROM agent_budget_entries WHERE rowid=?1"
            } else {
                "UPDATE agent_budget_entries SET amount=amount+1 WHERE rowid=?1"
            }, [row]).unwrap(), 1);
            db.execute_batch(&trigger).unwrap();
            unlimited_execution_assert_bad_reserve_refused(&f);
        }
    }
}

#[test]
fn unlimited_execution_zero_original_grant_forged_or_duplicate_reserves_refuse_assessment() {
    let _real = RealSpecialistTransport::enter();
    for rows in [1, 2] {
        let f = unlimited_execution_original_paid_web_fixture((0, 0));
        let db = db::open(&f.h.db_path).unwrap();
        let session = f.session.as_ref().unwrap();
        let worker = crate::agent_runtime::multi_agent::attempts::current(
            &db, &session.lease, &session.executor.assignment_id).unwrap();
        for index in 0..rows {
            let key = format!("reserve:{}:model_requests{}", session.executor.assignment_id,
                if index == 0 { "" } else { ":cold-duplicate" });
            db.execute(
                "INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id) VALUES(?1,?2,?3,?4,'model_requests','reserve',1,?5,?6)",
                params![Uuid::new_v4().to_string(), session.lease.root_run_id, session.executor.assignment_id,
                    worker.id, key, format!("assignment:{}", session.executor.assignment_id)],
            ).unwrap();
        }
        unlimited_execution_assert_bad_reserve_refused(&f);
    }
}
