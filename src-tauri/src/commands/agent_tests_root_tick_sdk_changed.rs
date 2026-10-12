// G3 only: gated actual changed Mapper fact SDK, then local original-frame replay.
#[test]
fn native_sdk_log_coordinator_changed_fact_real_gate_uses_next_original_call_and_zero_sdk_replay() {
    use std::sync::{mpsc, Arc, Mutex};
    let (arrive, arrived) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |request| {
        if specialist_original_is_web_sdk(&request) {
            return (200, "application/json", model_round(&[], 10));
        }
        if request.contains("You are the Root Coordinator") && request.contains("mapper-output") {
            let _ = arrive.send(());
            let _ = released
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5));
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture_from_web_mode_turns(
        "sdk-changed-fact-gate",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        None,
        web_mode_fixture("multi", "https://authorized.example.test"),
        Some(1),
    );
    drop(f.parent.take());
    let path = f.context.db_path.clone();
    let scan = f.context.scan_id.clone();
    let (live, result) = std::thread::scope(|scope| {
        let (done, rx) = mpsc::channel();
        let context = &mut f.context;
        scope.spawn(move || {
            let _real = RealSpecialistTransport::enter();
            let result = (|| {
                let mut session = multi_agent_prepare(context)?;
                let db = db::open(&context.db_path)?;
                let root = native_coordinator_root_context(context, &session.lease);
                let tx = rusqlite::Transaction::new_unchecked(
                    &db,
                    rusqlite::TransactionBehavior::Immediate,
                )
                .unwrap();
                let frame = NativeCoordinatorFrame::mapper(&tx, &session.lease, &session.mapper)?;
                tx.commit().unwrap();
                let before = root_tick_sdk_page(&context.db_path, &context.scan_id);
                let replay = native_coordinator_tick_for_frame(&root, &session.lease, &frame)?;
                let after = root_tick_sdk_page(&context.db_path, &context.scan_id);
                assert!(replay.replayed);
                assert_eq!(
                    before, after,
                    "same saved paid frame produces no fresh SDK diagnostic"
                );
                let root_id = session.lease.root_run_id.clone();
                // Keep the original Root trace replay assertions above, then
                // produce the real paid Web exit required by production delivery.
                let outcome = run_native_agent(context);
                assert!(
                    matches!(outcome, AgentTargetOutcome::Incomplete(_)),
                    "{outcome:?}"
                );
                multi_agent_finish_execution(context, &mut session, &outcome)?;
                Ok::<_, String>((after, root_id))
            })();
            let _ = done.send(result);
        });
        arrived.recv_timeout(Duration::from_secs(5)).unwrap();
        let live = root_tick_sdk_page(&path, &scan);
        assert!(rx.try_recv().is_err());
        let _ = release.send(());
        (live, rx.recv_timeout(Duration::from_secs(10)).unwrap())
    });
    let (done, root) = result.unwrap();
    assert_eq!(
        root_tick_sdk_stages(&root_tick_sdk_rows(&live, 1)),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "validated",
            "terminal"
        ]
    );
    let before = root_tick_sdk_rows(&live, 2);
    assert_eq!(root_tick_sdk_stages(&before), ["prepared", "sent"]);
    let after = root_tick_sdk_rows(&done, 2);
    assert_eq!(
        root_tick_sdk_stages(&after),
        [
            "prepared",
            "sent",
            "response_received",
            "cost_saved",
            "validated",
            "terminal"
        ]
    );
    assert_ne!(
        before[0]["dispatchKey"],
        root_tick_sdk_rows(&live, 1)[0]["dispatchKey"]
    );
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|wire| specialist_original_is_web_sdk(wire))
            .count(),
        1
    );
    let db = db::open(&path).unwrap();
    root_tick_sdk_assert_original(&db, &root, &before);
    root_tick_sdk_assert_original(&db, &root, &after);
    let mut requests = db
        .prepare(
            "SELECT json_extract(fact_json,'$.request.basis.phase') FROM agent_root_tick_receipts
         WHERE root_run_id=?1 AND phase='request' ORDER BY round",
        )
        .unwrap();
    let phases = requests
        .query_map([&root], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        phases,
        [
            "bootstrap",
            "mapper-output",
            "budget-allocation",
            "budget-consumption"
        ]
    );
    for phase in ["budget-allocation", "budget-consumption"] {
        let publications: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_root_tick_receipts p
             JOIN agent_root_tick_receipts q ON q.root_run_id=p.root_run_id AND q.round=p.round
             WHERE p.root_run_id=?1 AND p.phase='publication' AND q.phase='request'
               AND json_extract(q.fact_json,'$.request.basis.phase')=?2",
                params![root, phase],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            publications, 1,
            "each original budget fact publishes exactly once: {phase}"
        );
    }
    assert_eq!(
        seen.lock().unwrap().len(),
        6,
        "four original Root calls (allocation and consumption are distinct) + Mapper + actual Web exit; Root replay adds no provider call"
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, &root, None, "model_requests")
            .unwrap()
            .consumed,
        seen.lock().unwrap().len() as i64,
        "the original shared ledger must retain every real provider request"
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, &root, Some(""), "model_requests")
            .unwrap()
            .consumed,
        4,
        "bootstrap, changed Mapper, original allocation and original consumption are independently paid; replay is free"
    );
}
