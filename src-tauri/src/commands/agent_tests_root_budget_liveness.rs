#[test]
fn root_budget_original_invoice_survives_closed_root_without_reauthorizing_work() {
    use crate::agent_runtime::multi_agent::budget::{self, root::model::RootModelCall};
    for phase in ["received", "uncertain", "unsent"] {
        let harness = root_budget_owned_fixture_for_test("root-late-original-invoice");
        let db = db::open(&harness.db_path).unwrap();
        let root = &harness.context.run.as_ref().unwrap().run_id;
        let tx = db.unchecked_transaction().unwrap();
        let call = RootModelCall::claim(&tx, root, 1, &"a".repeat(64), 1000).unwrap();
        tx.commit().unwrap();
        db.execute("UPDATE agent_runs SET status='paused' WHERE id=?1", [root])
            .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let response = late_web_model_response();
        let tx = db.unchecked_transaction().unwrap();
        assert_eq!(
            call.terminal(
                &tx,
                phase,
                (phase == "received").then_some(&response),
                "original-root-fact"
            )
            .unwrap(),
            phase == "uncertain"
        );
        tx.commit().unwrap();
        let received: i64 = db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'", [&root], |r|r.get(0)).unwrap();
        let exact_cache: i64 = db.query_row("SELECT count(*) FROM agent_root_model_cache_provenance p JOIN agent_root_model_journal j
         ON p.call_id=j.call_id AND p.root_run_id=j.root_run_id AND p.lease_attempt_id=j.lease_attempt_id
         AND p.round=j.round AND p.request_hash=j.request_hash AND p.response_hash=json_extract(j.receipt_json,'$.responseHash')
         WHERE j.root_run_id=?1 AND j.phase='received' AND p.cached_usage_reported=0", [&root], |r|r.get(0)).unwrap();
        assert_eq!(exact_cache, received);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_cache_provenance", [], |r|r.get::<_,i64>(0)).unwrap(), received);
        let after = super::tests::application_table_snapshot(&db);
        for (table, rows) in &before {
            if !matches!(
                table.as_str(),
                "agent_root_model_journal" | "agent_budget_entries" | "agent_root_model_cache_provenance"
            ) {
                assert_eq!(
                    after
                        .iter()
                        .find(|(name, _)| name == table)
                        .map(|(_, values)| values),
                    Some(rows),
                    "{table}"
                );
            }
        }
        let b = budget::balance(&db, root, Some(""), "model_requests").unwrap();
        assert_eq!(
            (b.reserved, b.consumed, b.indeterminate),
            match phase {
                "received" => (0, 1, 0),
                "uncertain" => (0, 0, 1),
                _ => (0, 0, 0),
            }
        );
        let saved = super::tests::application_table_snapshot(&db);
        assert!(call.require_next_work(&db).is_err());
        let tx = db.unchecked_transaction().unwrap();
        assert!(RootModelCall::claim(&tx, root, 2, &"b".repeat(64), 1000).is_err());
        assert!(call
            .terminal(
                &tx,
                phase,
                (phase == "received").then_some(&response),
                "original-root-fact"
            )
            .is_err());
        tx.rollback().unwrap();
        assert!(super::tests::application_table_snapshot(&db) == saved);
        drop(db);
        fs::remove_dir_all(harness.root).unwrap();
    }
}

#[test]
fn root_budget_actual_inflight_scan_stop_cancels_once_and_retains_original_unknown_cost() {
    use crate::agent_runtime::multi_agent::budget;
    use std::sync::{mpsc, Arc, Mutex};
    let (arrived_tx, arrived_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Arc::new(Mutex::new(release_rx));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrived_tx.send(());
        let _ = release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
        (200, "application/json", model_round(&[], 20))
    }));
    let mut harness = root_budget_owned_fixture_for_test("root-inflight-scan-stop");
    harness.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&harness.context.environment, None).unwrap(),
        &[],
    );
    let db = db::open(&harness.db_path).unwrap();
    std::thread::scope(|scope| {
        let (done_tx, done_rx) = mpsc::channel();
        let context = &harness.context;
        let client = &client;
        scope.spawn(move || {
            let _ = done_tx.send(native_model_transport(
                context,
                client,
                vec![json!({"role":"user","content":"stop original Root in flight"})],
                &[],
                1,
            ));
        });
        arrived_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        db.execute(
            "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
            [&context.scan_id],
        )
        .unwrap();
        let result = done_rx.recv_timeout(Duration::from_secs(3));
        let _ = release_tx.send(());
        assert!(result
            .expect("original scan stop must cancel before provider replies")
            .is_err());
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while seen.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let b = budget::balance(&db, root, Some(""), "model_requests").unwrap();
    assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, 1));
    assert_eq!(seen.lock().unwrap().len(), 1);
    let before = super::tests::application_table_snapshot(&db);
    assert!(native_model_transport(&harness.context, &client, vec![], &[], 2).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(db);
    fs::remove_dir_all(harness.root).unwrap();
}
