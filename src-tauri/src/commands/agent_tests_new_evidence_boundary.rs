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
        let phases = followup_original_root_phase_counts(&db, &f.root, &f.h.model_seen.lock().unwrap());
        assert_eq!(phases.get("budget-consumption"), Some(&2));
        assert_eq!(phases.get("http-budget-consumption"), Some(&1));
        assert!(!phases.contains_key("new-evidence"));
        let snapshot = db.unchecked_transaction().unwrap();
        let assessments: Vec<(String, String, i64)> = snapshot.prepare(
            "SELECT q.call_id,q.fact_json,json_extract(p.fact_json,'$.eventSequence') FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.root_run_id=q.root_run_id AND p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase') IN ('budget-consumption','http-budget-consumption') ORDER BY q.round"
        ).unwrap().query_map([&f.root], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(assessments.len(), 3);
        for (call, raw, event) in assessments {
            let request: JsonValue = serde_json::from_str(&raw).unwrap();
            let basis = &request["request"]["basis"];
            let fact = &basis["changedFact"]["semantic"];
            assert_eq!(fact["rootRunId"], f.root);
            assert_eq!(fact["advisoryOnly"], true);
            assert_eq!(fact["executionGrant"], false);
            assert_eq!(fact["additionalScopeGranted"], false);
            assert_eq!(fact["targetRequestsGranted"], 0);
            let fee: (i64, i64) = snapshot.query_row(
                "SELECT count(*),coalesce(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='' AND source_id=?2 AND dimension='model_requests' AND kind='consume'",
                params![f.root, call], |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(fee, (1, 1));
            let publication = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(&snapshot, &f.root, event).unwrap();
            let step = if basis["phase"] == "budget-consumption" {
                "assess:budget_consumption"
            } else { "assess:http_budget_consumption" };
            assert_eq!(publication.record["summary"]["suggestions"], json!([step]));
            assert!(publication.record.get("localTools").is_none());
        }
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
