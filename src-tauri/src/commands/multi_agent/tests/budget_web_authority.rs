#[test]
fn budget_web_model_inflight_root_and_child_cancellation_close_transport_keep_cost() {
    for root_cancel in [true, false] {
        let f = web_financial_producer_fixture();
        let path = f.context().db_path.clone();
        let child = f.context().run.as_ref().unwrap().run_id.clone();
        let accepted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let entered = accepted.clone();
        // Measure transport closure from the moment the provider holds the
        // request (cancel already durable), not from fixture/bootstrap work
        // that parallel test load can slow down.
        let accepted_at = std::sync::Arc::new(std::sync::Mutex::new(None));
        let accepted_clock = accepted_at.clone();
        f.set_web_reply(move |_| {
            let db = db::open(&path).unwrap();
            let cancelled = if root_cancel {
                db.query_row(
                    "SELECT root_run_id FROM agent_runs WHERE id=?1",
                    [&child],
                    |r| r.get::<_, String>(0),
                )
                .unwrap()
            } else {
                child.clone()
            };
            db.execute(
                "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",
                [cancelled],
            )
            .unwrap();
            *accepted_clock.lock().unwrap() = Some(std::time::Instant::now());
            entered.store(true, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_secs(2));
            (200, "application/json", model_round(&[], 10))
        });
        let start = std::time::Instant::now();
        let result = NativeAgentBackend.execute(f.context());
        let returned_at = std::time::Instant::now();
        assert!(
            accepted.load(std::sync::atomic::Ordering::SeqCst),
            "{}",
            result.detail()
        );
        let in_flight = returned_at.duration_since(accepted_at.lock().unwrap().unwrap());
        assert!(
            in_flight < std::time::Duration::from_secs(1),
            "cancel flag must close transport before delayed response: \
             {in_flight:?} after provider accepted, {:?} total",
            start.elapsed()
        );
        assert_eq!(
            result.terminal_code(),
            terminal_code::REQUEST_RECONCILIATION_REQUIRED
        );
        f.assert_calls(1);
        let db = db::open(&f.context().db_path).unwrap();
        let tx = db.unchecked_transaction().unwrap();
        let (lease, assignment) = crate::agent_runtime::multi_agent::budget::target::child_owner(
            &tx,
            &f.context().run.as_ref().unwrap().run_id,
        )
        .unwrap()
        .unwrap();
        let b = crate::agent_runtime::multi_agent::budget::balance(
            &tx,
            &lease.root_run_id,
            Some(&assignment),
            "model_requests",
        )
        .unwrap();
        assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 0, 1));
        assert_eq!(
            tx.query_row(
                "SELECT count(*) FROM agent_web_model_journal WHERE phase='uncertain'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        tx.rollback().unwrap();
    }
}

#[test]
fn budget_web_model_pending_claim_reopen_does_not_renew_any_lease() {
    let (root,mut context,_runtime,_request)=http_journal_fixture("http://127.0.0.1:9/",0);
    native_model_budget_admission(&context,1,"frozen-test-wire".into()).unwrap().unwrap();
    let db=db::open(&context.db_path).unwrap();
    db.execute_batch("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+120 seconds','localtime');
        UPDATE agent_assignments SET lease_expires_at=datetime('now','+120 seconds','localtime');
        UPDATE agent_capability_leases SET lease_expires_at=datetime('now','+120 seconds','localtime');
        UPDATE agent_runs SET lease_expires_at=datetime('now','+120 seconds','localtime');").unwrap();
    let leases=|db:&rusqlite::Connection| {
        ["agent_coordinator_leases","agent_assignments","agent_capability_leases","agent_runs"].iter().map(|table|
            db.prepare(&format!("SELECT lease_expires_at FROM {table} ORDER BY rowid")).unwrap()
                .query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap()).collect::<Vec<_>>()
    };
    let before=leases(&db);drop(db);context.resume=true;
    let result=NativeAgentBackend.execute(&context);
    assert_eq!(result.terminal_code(),terminal_code::REQUEST_RECONCILIATION_REQUIRED);
    let db=db::open(&context.db_path).unwrap();
    assert_eq!(leases(&db),before,"an unresolved call cannot regrant execution by restarting a heartbeat");
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_web_model_received_bill_missing_publication_cannot_renew_or_replay() {
    let f = web_financial_producer_fixture();
    let mut context = f.context().clone();
    let db = db::open(&context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_model_publication BEFORE INSERT ON agent_events WHEN NEW.event_type='model_round_completed' BEGIN SELECT RAISE(ABORT,'model_publication_unavailable'); END;").unwrap();
    assert!(NativeAgentBackend
        .execute(&context)
        .detail()
        .contains("model_publication_unavailable"));
    f.assert_calls(1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_web_model_journal WHERE phase='received'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER reject_model_publication;")
        .unwrap();
    let before: String = db
        .query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases",
            [],
            |r| r.get(0),
        )
        .unwrap();
    context.resume = true;
    assert_eq!(
        NativeAgentBackend.execute(&context).terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    assert_eq!(
        db.query_row(
            "SELECT lease_expires_at FROM agent_coordinator_leases",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    f.assert_calls(1);
}

#[test]
fn budget_web_model_heartbeat_cancelled_child_never_renews_authority() {
    let (root,context,_runtime,_request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let db=db::open(&context.db_path).unwrap();
    let child=&context.run.as_ref().unwrap().run_id;
    db.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",[child]).unwrap();
    let before=receipt_database_snapshot(&db);
    assert!(crate::agent_runtime::multi_agent::scheduler::refresh_running_executor_leases(&db,child).is_err());
    assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_web_model_heartbeat_partial_or_revoked_renewal_rolls_back_all_rows() {
    for table in ["agent_coordinator_leases","agent_assignments","agent_capability_leases","agent_runs","agent_assignment_attempts"] {
        for fault in ["ABORT","FAIL","IGNORE","cancel"] {
            let (root,context,_runtime,_request)=http_journal_fixture("http://127.0.0.1:9/",0);
            let db=db::open(&context.db_path).unwrap();
            db.execute_batch("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','+120 seconds','localtime');
                UPDATE agent_assignments SET lease_expires_at=datetime('now','+120 seconds','localtime');
                UPDATE agent_capability_leases SET lease_expires_at=datetime('now','+120 seconds','localtime');
                UPDATE agent_runs SET lease_expires_at=datetime('now','+120 seconds','localtime');").unwrap();
            let trigger=if fault=="cancel" {
                format!("CREATE TRIGGER break_heartbeat AFTER UPDATE ON {table} BEGIN
                    UPDATE agent_runs SET cancel_requested_at='cancelled-during-renewal' WHERE role='coordinator'; END;")
            } else {
                let raise=if fault=="IGNORE" {"IGNORE".to_string()} else {format!("{fault},'heartbeat-write-fault'")};
                format!("CREATE TRIGGER break_heartbeat BEFORE UPDATE ON {table} BEGIN SELECT RAISE({raise}); END;")
            };
            db.execute_batch(&trigger).unwrap();
            let before=receipt_database_snapshot(&db);
            assert!(crate::agent_runtime::multi_agent::scheduler::refresh_running_executor_leases(&db,&context.run.as_ref().unwrap().run_id).is_err(),"{table} {fault} regranted partial authority");
            assert_eq!(receipt_database_snapshot(&db),before,"{table} {fault} failed to restore all original rows");
            drop(db);fs::remove_dir_all(root).unwrap();
        }
    }
}
