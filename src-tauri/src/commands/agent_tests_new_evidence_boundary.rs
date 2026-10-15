// Preserve real HTTP/SDK original invoices and use the production delivery writer.
// This fixture stops immediately after delivery so the first assessment is unpaid.
fn new_evidence_closed_original_fixture(tag: &str) -> InputParserRootFixture {
    new_evidence_closed_original_fixture_with_financial_drain(tag, false)
}

fn new_evidence_closed_original_fixture_with_financial_drain(
    tag: &str,
    drain_financial: bool,
) -> InputParserRootFixture {
    let f = input_parser_root_fixture(tag, &["dispatch:input_parser"]);
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    let original_grant = if drain_financial {
        let grant = crate::agent_runtime::multi_agent::budget::model::original_web_grant(
            &db, &session.lease, &session.executor.assignment_id,
        ).unwrap();
        assert_eq!(grant.1, 3, "the original finite request grant fixes this fixture's Web bill count");
        Some(grant)
    } else { None };
    let tx = db.unchecked_transaction().unwrap();
    let (usage, idle) =
        settle_original_web_child_usage(&tx, &session.lease, &session.executor).unwrap();
    tx.commit().unwrap();
    drop(idle);
    let payload = json!({"terminalCode":f.outcome.terminal_code(),"summary":f.outcome.detail(),
        "usage":{"totalTokens":usage.total_tokens,"modelRequests":usage.model_requests}});
    if drain_financial {
        // A later full finish may only replay assessments already published
        // before logical closure. Keep the first new-evidence assessment unpaid.
        native_coordinator_budget_consumption_after_settlement(
            &f.h.context, &session.lease, &session.executor, &payload, &f.outcome.detail(),
        ).unwrap();
        native_coordinator_http_budget_consumption_after_settlement(
            &f.h.context, &session.lease, &session.executor, &payload, &f.outcome.detail(),
        ).unwrap();
        let expected_web = usize::try_from(original_grant.unwrap().1).unwrap();
        assert_eq!(usage.model_requests, expected_web as i64);
        let phases = followup_original_root_phase_counts(&db, &f.root, &f.h.model_seen.lock().unwrap());
        let expected_phases: std::collections::BTreeMap<String, usize> = [
            ("bootstrap", 1), ("mapper-output", 1), ("budget-allocation", 1),
            ("budget-consumption", expected_web), ("http-budget-consumption", 1),
        ].into_iter().map(|(phase, count)| (phase.into(), count)).collect();
        assert_eq!(phases, expected_phases);
        assert_eq!(phases.get("http-budget-consumption"), Some(&1));
        assert!(!phases.contains_key("new-evidence"));
        assert_eq!(f.calls(), phases.values().sum::<usize>() + expected_web + 1,
            "original Root phases, the original Web grant and the one original Mapper account for every SDK");
        let snapshot = db.unchecked_transaction().unwrap();
        let worker = crate::agent_runtime::multi_agent::attempts::current(
            &snapshot, &session.lease, &session.executor.assignment_id,
        ).unwrap();
        let web_bills: Vec<(String, i64, String, String)> = snapshot.prepare(
            "SELECT call_id,round,request_hash,receipt_json FROM agent_web_model_journal WHERE root_run_id=?1 AND assignment_id=?2 AND child_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 AND phase='received' ORDER BY round"
        ).unwrap().query_map(params![f.root, session.executor.assignment_id, session.executor.run_id,
            session.lease.lease_epoch, session.lease.fencing_token],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(web_bills.len(), expected_web);
        assert_eq!(web_bills.iter().map(|bill| bill.1).collect::<Vec<_>>(), vec![1, 2, 3]);
        let assessments: Vec<(String, String, String)> = snapshot.prepare(
            "SELECT q.call_id,q.fact_json,p.fact_json FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.root_run_id=q.root_run_id AND p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase') IN ('budget-consumption','http-budget-consumption') ORDER BY q.round"
        ).unwrap().query_map([&f.root], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(assessments.len(), expected_web + 1);
        let mut assessed_web = std::collections::HashSet::new();
        let mut assessed_http = std::collections::HashSet::new();
        let mut paid_web_requests = 0;
        for (call, raw, published) in assessments {
            let request: JsonValue = serde_json::from_str(&raw).unwrap();
            let basis = &request["request"]["basis"];
            let fact = &basis["changedFact"]["semantic"];
            assert_eq!(fact["rootRunId"], f.root);
            assert_eq!(fact["assignmentId"], session.executor.assignment_id);
            assert_eq!(fact["childRunId"], session.executor.run_id);
            assert_eq!(fact["workerAttemptId"], worker.id);
            assert_eq!(fact["advisoryOnly"], true);
            assert_eq!(fact["executionGrant"], false);
            assert_eq!(fact["additionalScopeGranted"], false);
            assert_eq!(fact["targetRequestsGranted"], 0);
            let fee: (i64, i64) = snapshot.query_row(
                "SELECT count(*),coalesce(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='' AND source_id=?2 AND dimension='model_requests' AND kind='consume'",
                params![f.root, call], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(fee, (1, 1));
            if basis["phase"] == "budget-consumption" {
                let original_call = fact["callId"].as_str().unwrap();
                assert!(assessed_web.insert(original_call.to_string()), "an original Web bill is assessed once");
                let bill = web_bills.iter().find(|bill| bill.0 == original_call).unwrap();
                assert_eq!(fact["round"], bill.1);
                assert_eq!(fact["requestHash"], bill.2);
                let received: JsonValue = serde_json::from_str(&bill.3).unwrap();
                assert_eq!(received["usageReported"], true);
                assert_eq!(received["usage"], fact["usage"]);
                assert_eq!(received["responseHash"], fact["responseHash"]);
                assert_eq!(fact["usage"]["modelRequests"], 1);
                paid_web_requests += fact["usage"]["modelRequests"].as_i64().unwrap();
                let dispatch: i64 = snapshot.query_row(
                    "SELECT count(*) FROM agent_web_model_journal WHERE root_run_id=?1 AND assignment_id=?2 AND child_run_id=?3 AND call_id=?4 AND round=?5 AND request_hash=?6 AND lease_epoch=?7 AND fencing_token=?8 AND phase='dispatch' AND receipt_json='{}'",
                    params![f.root, session.executor.assignment_id, session.executor.run_id, original_call, bill.1,
                        bill.2, session.lease.lease_epoch, session.lease.fencing_token], |row| row.get(0),
                ).unwrap();
                assert_eq!(dispatch, 1);
                let source = format!("web-model:{}:{}:{}:receipt:{}", session.executor.assignment_id,
                    bill.1, bill.2, fact["responseHash"].as_str().unwrap());
                for (dimension, field) in [
                    ("model_input_tokens", "inputTokens"), ("model_cached_tokens", "cachedInputTokens"),
                    ("model_output_tokens", "outputTokens"), ("model_requests", "modelRequests"),
                ] {
                    let amount = fact["usage"][field].as_i64().unwrap();
                    assert!(amount >= 0);
                    let fee: (i64, i64) = snapshot.query_row(
                        "SELECT count(*),coalesce(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 AND dimension=?5 AND kind='consume'",
                        params![f.root, session.executor.assignment_id, worker.id, source, dimension],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    ).unwrap();
                    assert_eq!(fee, (i64::from(amount > 0), amount));
                }
            } else {
                assert_eq!(basis["phase"], "http-budget-consumption");
                assert_eq!(fact["protocol"], "original_received_http_target_cost_advisory_v1");
                assert_eq!(fact["targetRequests"], 1);
                let source = fact["sourceId"].as_str().unwrap();
                assert!(assessed_http.insert(source.to_string()));
                let receipt: i64 = snapshot.query_row(
                    "SELECT count(*) FROM agent_http_budget_receipts WHERE root_run_id=?1 AND receipt_id=?2 AND source_id=?3 AND run_id=?4 AND scan_id=?5 AND attempt_number=?6 AND target_url=?7 AND response_status=?8",
                    params![f.root, fact["receiptId"].as_str().unwrap(), source, session.executor.run_id,
                        session.lease.scan_id, session.lease.attempt_number, session.lease.target_key,
                        fact["status"].as_i64().unwrap()], |row| row.get(0),
                ).unwrap();
                assert_eq!(receipt, 1);
                let target_fees: Vec<(String, i64)> = snapshot.prepare(
                    "SELECT kind,amount FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 AND dimension='target_requests' ORDER BY rowid"
                ).unwrap().query_map(params![f.root, session.executor.assignment_id, worker.id, source],
                    |row| Ok((row.get(0)?, row.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
                assert_eq!(target_fees, vec![("reserve".into(), 1), ("forfeit".into(), 1), ("reconcile".into(), 1)]);
            }
            let published: JsonValue = serde_json::from_str(&published).unwrap();
            let invoice: String = snapshot.query_row(
                "SELECT receipt_json FROM agent_root_model_journal WHERE root_run_id=?1 AND call_id=?2 AND phase='received'",
                params![f.root, call], |row| row.get(0),
            ).unwrap();
            let invoice: JsonValue = serde_json::from_str(&invoice).unwrap();
            assert_eq!(invoice["usageReported"], true);
            assert_eq!(invoice["usage"], published["event"]["usage"]);
            assert_eq!(invoice["usage"]["modelRequests"], 1);
            for (dimension, field) in [
                ("model_input_tokens", "inputTokens"), ("model_cached_tokens", "cachedInputTokens"),
                ("model_output_tokens", "outputTokens"), ("model_requests", "modelRequests"),
            ] {
                let amount = invoice["usage"][field].as_i64().unwrap();
                assert!(amount >= 0);
                let fee: (i64, i64) = snapshot.query_row(
                    "SELECT count(*),coalesce(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='' AND source_id=?2 AND dimension=?3 AND kind='consume'",
                    params![f.root, call, dimension], |row| Ok((row.get(0)?, row.get(1)?)),
                ).unwrap();
                assert_eq!(fee, (i64::from(amount > 0), amount));
            }
            let publication = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(
                &snapshot, &f.root, published["eventSequence"].as_i64().unwrap(),
            ).unwrap();
            let step = if basis["phase"] == "budget-consumption" {
                "assess:budget_consumption"
            } else { "assess:http_budget_consumption" };
            assert_eq!(publication.record["summary"]["suggestions"], json!([step]));
            assert!(publication.record.get("localTools").is_none());
        }
        assert_eq!(assessed_web.len(), expected_web);
        assert_eq!(assessed_http.len(), 1);
        assert_eq!(paid_web_requests, usage.model_requests);
        snapshot.rollback().unwrap();
    }
    complete_target_child_delivery(
        &db,
        &session.lease,
        &session.executor,
        "execution_result",
        &format!("executor-result:{}", session.executor.assignment_id),
        &payload,
        true,
        &f.outcome.detail(),
    )
    .unwrap();
    f
}

#[test]
fn new_evidence_actual_first_sdk_concurrent_owner_blocks_duplicate_and_later_replays_zero() {
    let _real = RealSpecialistTransport::enter();
    let f = new_evidence_closed_original_fixture("ne-concurrent");
    let db = db::open(&f.h.db_path).unwrap();
    let (id, _) = f.record(&db);
    let calls = f.calls();
    let consumed = f.requests(&db).consumed;
    let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *f.boundary.lock().unwrap() = Some(Box::new(move |_| {
        arrived_tx.send(()).unwrap();
        release_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    }));
    let context = f.h.context.clone();
    let actor = f.lease().clone();
    let node = id.clone();
    let first = std::thread::spawn(move || {
        let _real = RealSpecialistTransport::enter();
        native_coordinator_new_evidence_feedback(&context, &actor, &node).map(|r| r.replayed)
    });
    arrived_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let second = native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &id);
    release_tx.send(()).unwrap();
    assert!(
        second.is_err(),
        "concurrent original paid invocation must refuse"
    );
    assert!(!first.join().unwrap().unwrap());
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(f.requests(&db).consumed, consumed + 1);
    let before = client_delivery_all_rows(&db);
    assert!(
        native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &id)
            .unwrap()
            .replayed
    );
    assert_eq!(client_delivery_all_rows(&db), before);
    assert_eq!(f.calls(), calls + 1);
}

#[test]
fn new_evidence_actual_local_snapshot_round_then_final_replays_all_original_fees() {
    let _real = RealSpecialistTransport::enter();
    let mut f = new_evidence_closed_original_fixture_with_financial_drain("ne-local-react", true);
    let db = db::open(&f.h.db_path).unwrap();
    let (id, _) = f.record(&db);
    let calls = f.calls();
    let consumed = f.requests(&db).consumed;
    let mut rounds = 0;
    *f.new_evidence_reply.lock().unwrap() = Some(Box::new(move |_| {
        rounds += 1;
        if rounds == 1 {
            root_local_wire(
                json!([root_local_call("ne-snapshot", "snapshot.read", json!({}))]),
                "",
                true,
            )
        } else {
            proposal_model_response(
                &json!({"schemaVersion":1,"observed":["original local snapshot checked"],
                "missing":[],"suggestions":["assess:new_evidence"],"costNotes":[],"risks":[]})
                .to_string(),
            )
        }
    }));
    let decision = native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &id).unwrap();
    assert!(native_root_paid_suggestion_selected(
        &decision,
        "assess:new_evidence"
    ));
    assert_eq!(f.calls(), calls + 2);
    assert_eq!(f.requests(&db).consumed, consumed + 2);
    let outcome = f.outcome.clone();
    // Re-enter the original completed delivery, then run real Client/InputParser.
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    let completed_calls = f.calls();
    let before = client_delivery_all_rows(&db);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(client_delivery_all_rows(&db), before);
    assert_eq!(f.calls(), completed_calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn new_evidence_expired_before_first_sdk_and_expired_during_sdk_preserve_original_author_and_fees()
{
    let _real = RealSpecialistTransport::enter();
    for during in [false, true] {
        let f = new_evidence_closed_original_fixture(if during {
            "ne-expire-during"
        } else {
            "ne-expire-before"
        });
        let db = db::open(&f.h.db_path).unwrap();
        let (id, _) = f.record(&db);
        let root = f.root.clone();
        let expire = move |db: &rusqlite::Connection| {
            db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1", [&root]).unwrap();
        };
        if during {
            *f.boundary.lock().unwrap() = Some(Box::new(expire));
        } else {
            expire(&db);
        }
        let calls = f.calls();
        let before = client_delivery_all_rows(&db);
        let consumed = f.requests(&db).consumed;
        let publications = root_tick_count(&db, &f.root, "publication");
        assert!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &id).is_err());
        assert_eq!(f.calls(), calls + usize::from(during));
        assert_eq!(root_tick_count(&db, &f.root, "publication"), publications);
        assert_eq!(f.requests(&db).consumed, consumed + i64::from(during));
        if !during {
            assert_eq!(client_delivery_all_rows(&db), before);
        }
        let paid = client_delivery_all_rows(&db);
        assert!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &id).is_err());
        assert_eq!(client_delivery_all_rows(&db), paid);
        assert_eq!(f.calls(), calls + usize::from(during));
    }
}
