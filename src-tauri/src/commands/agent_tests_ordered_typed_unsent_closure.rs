// A gateway BeforeTransport receipt is distinct from a missing response.
fn ordered_closure_actual_queued_cancel(f: &RootTickFixture, id: &str) -> crate::agent_runtime::multi_agent::directive::ordered_execution::ActionJob {
    use crate::agent_runtime::model::{admission, CancelToken};
    let gate = admission::gate_for(true, &f.context.environment.api_base);
    let occupied = gate.acquire(&CancelToken::new()).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let order = std::thread::scope(|scope| {
        let (done_tx, done) = std::sync::mpsc::channel();
        scope.spawn(move || { let _ = done_tx.send(ordered_exec_apply(f)); });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while gate.queued()==0 && std::time::Instant::now()<deadline { std::thread::sleep(Duration::from_millis(5)); }
        assert_eq!(gate.queued(), 1, "original ordered SDK did not reach admission queue");
        let (child, order): (String,i64) = db.query_row("SELECT child_run_id,action_order FROM agent_directive_ordered_actions WHERE directive_id=?1 AND state='executing'", [id], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        db.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1", [&child]).unwrap();
        let error = done.recv_timeout(Duration::from_secs(3)).unwrap().unwrap_err();
        assert!(error.contains("未发出"), "{error}");
        drop(occupied);
        order
    });
    crate::agent_runtime::multi_agent::directive::ordered_execution::load(&db, id, order).unwrap().unwrap()
}
#[test]
fn ordered_typed_unsent_closure_actual_gateway_queue_cancel_releases_only_original_unused_budget() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", proposal_model_response(valid_proposal_text()))));
    let (mut f, id) = ordered_exec_fixture("ordered-typed-unsent", &format!("http://127.0.0.1:{port}/v1"));
    f.context.environment.deployment = "local".into();
    let job = ordered_closure_actual_queued_cancel(&f, &id);
    let db = db::open(&f.context.db_path).unwrap();
    let saved = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_specialist_calls WHERE assignment_id=?1", [&job.child.assignment_id]).unwrap();
    assert_eq!(db.query_row("SELECT failure_code FROM agent_specialist_calls WHERE assignment_id=?1", [&job.child.assignment_id], |r|r.get::<_,String>(0)).unwrap(), "model_cancelled_before_transport");
    let outcome = AgentTargetOutcome::incomplete("original ordered gateway proved before transport cancellation");
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    ordered_closure_assert_cancelled(&db, &job);
    for dimension in ["model_input_tokens", "model_cached_tokens", "model_output_tokens", "model_requests"] {
        let balance = crate::agent_runtime::multi_agent::budget::balance(&db, &f.actor.root_run_id, Some(&job.child.assignment_id), dimension).unwrap();
        assert_eq!((balance.reserved, balance.consumed, balance.indeterminate), (0,0,0));
    }
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_specialist_calls WHERE assignment_id=?1", [&job.child.assignment_id]).unwrap()==saved);
    assert_eq!(ordered_exec_projection(&db, &id)["actions"][0]["state"], "cancelled_before_dispatch");
    assert!(ordered_exec_receipts(&db, &id).is_empty());
    assert_eq!(seen.lock().unwrap().len(), 0);
    let before = super::tests::application_table_snapshot(&db);
    finish_coordinator_run(&db, &f.actor, &outcome).unwrap();
    assert!(super::tests::application_table_snapshot(&db)==before);
}

#[test]
fn ordered_typed_unsent_closure_second_queue_cancel_keeps_original_first_paid_receipt() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", proposal_model_response(valid_proposal_text()))));
    let (mut f, id) = ordered_exec_fixture("ordered-typed-unsent-second", &format!("http://127.0.0.1:{port}/v1"));
    f.context.environment.deployment = "local".into();
    let db = db::open(&f.context.db_path).unwrap();
    ordered_exec_apply(&f).unwrap();
    let original = ordered_exec_receipts(&db, &id);
    let job = ordered_closure_actual_queued_cancel(&f, &id);
    assert_eq!(job.action.order, 2);
    finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("second original call never sent")).unwrap();
    ordered_closure_assert_cancelled(&db, &job);
    assert_eq!(ordered_exec_receipts(&db, &id), original);
    assert_eq!(ordered_exec_projection(&db, &id)["actions"][1]["state"], "cancelled_before_dispatch");
    let used: (i64,i64) = db.query_row("SELECT spent_tokens,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1", [&f.actor.root_run_id], |r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(used, (20,1));
    assert_eq!(seen.lock().unwrap().len(), 1);
}
#[test]
fn ordered_typed_unsent_closure_damaged_receipt_or_foreign_log_owner_cannot_release_or_close_root() {
    for damage in [
        "UPDATE agent_specialist_calls SET finished_at=''",
        "UPDATE agent_specialist_calls SET event_sequence=1",
        "UPDATE agent_specialist_calls SET response_json='{\"content\":\"fake\"}'",
        "UPDATE agent_specialist_calls SET usage_json='{\"modelRequests\":1}'",
        "DROP TRIGGER native_sdk_log_owner_no_update; UPDATE native_sdk_log_owners SET worker_id=NULL WHERE domain='specialist'",
    ] {
        // Own never-contacted provider: a shared fixed endpoint would share
        // one admission domain with the sibling iteration-heavy test.
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (500, "application/json", "must not send".into())));
        let (mut f, id) = ordered_exec_fixture("ordered-typed-unsent-damaged", &format!("http://127.0.0.1:{port}/v1"));
        f.context.environment.deployment = "local".into();
        let _job = ordered_closure_actual_queued_cancel(&f, &id);
        assert!(seen.lock().unwrap().is_empty(), "queued typed-unsent call must never reach the provider");
        let db = db::open(&f.context.db_path).unwrap();
        db.execute_batch(damage).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("damaged typed receipt")).is_err(), "{damage}");
        assert!(super::tests::application_table_snapshot(&db)==before, "{damage}: original Root must entirely roll back");
    }
}
#[test]
fn ordered_typed_unsent_closure_silent_or_collateral_writes_roll_back_all_original_rows() {
    for fault in [
        "CREATE TRIGGER typed_unsent_close_fault BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='cancelled' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER typed_unsent_close_fault BEFORE INSERT ON agent_budget_entries WHEN NEW.kind='release' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER typed_unsent_close_fault AFTER UPDATE OF state ON agent_assignments WHEN NEW.state='cancelled' BEGIN UPDATE projects SET name='unauthorized-typed-unsent'; END;",
        "CREATE TRIGGER typed_unsent_close_fault AFTER UPDATE OF state ON agent_assignments WHEN NEW.state='cancelled' BEGIN UPDATE agent_specialist_calls SET failure_code=''; END;",
    ] {
        // Own never-contacted provider: a shared fixed endpoint would share
        // one admission domain with the sibling iteration-heavy test.
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (500, "application/json", "must not send".into())));
        let (mut f, id) = ordered_exec_fixture("ordered-typed-unsent-fault", &format!("http://127.0.0.1:{port}/v1"));
        f.context.environment.deployment = "local".into();
        let _job = ordered_closure_actual_queued_cancel(&f, &id);
        assert!(seen.lock().unwrap().is_empty(), "queued typed-unsent call must never reach the provider");
        let db = db::open(&f.context.db_path).unwrap();
        db.execute_batch(fault).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("faulted typed receipt release")).is_err(), "{fault}");
        assert!(super::tests::application_table_snapshot(&db)==before, "{fault}: original Root must entirely roll back");
    }
}
#[test]
fn ordered_typed_unsent_closure_forged_no_send_after_actual_503_cannot_refund_sent_cost() {
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (503, "application/json", r#"{"error":{"message":"fixture unavailable"}}"#.into())));
    let (f, _) = ordered_exec_fixture("ordered-typed-unsent-sent", &format!("http://127.0.0.1:{port}/v1"));
    let db = db::open(&f.context.db_path).unwrap();
    assert!(ordered_exec_apply(&f).is_err());
    // Temporary corrupt-history fixture only; production immutable writer stays unchanged.
    db.execute_batch("DROP TRIGGER agent_specialist_call_immutable; UPDATE agent_specialist_calls SET state='executing',failure_code='model_cancelled_before_transport'").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let error = finish_coordinator_run(&db, &f.actor, &AgentTargetOutcome::incomplete("forged no-send after provider failure")).unwrap_err();
    assert!(error.contains("proposal_unsent_transport_history_conflict"), "{error}");
    assert!(super::tests::application_table_snapshot(&db)==before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
