// Typed gateway before-transport cancellation must not acquire business writes.
#[test]
fn specialist_unsent_actual_queue_cancellation_denies_receipt_trigger_business_write() {
    use crate::agent_runtime::model::{admission, CancelToken};
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    let (root, mut context, lease, child) = specialist_financial_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response("must not send"),
        )
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    context.environment.deployment = "local".into();
    let gate = admission::gate_for(true, &context.environment.api_base);
    let occupied = gate.acquire(&CancelToken::new()).unwrap();
    let path = context.db_path.clone();
    let replay_context = context.clone();
    let replay_lease = lease.clone();
    let replay_child = child.clone();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        done_tx
            .send(multi_agent_child_round_transport(
                &context,
                &lease,
                &child,
                "readonly",
                json!({}),
            ))
            .unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while gate.queued() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(gate.queued(), 1, "model call did not reach admission queue");
    let db = db::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER unsent_collateral AFTER UPDATE OF failure_code ON agent_specialist_calls WHEN NEW.failure_code='model_cancelled_before_transport' BEGIN UPDATE projects SET status='archived'; END;").unwrap();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute(
        "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",
        [&replay_child.run_id],
    )
    .unwrap();
    // Capture cancellation inside its transaction; receipt cannot race ahead of this snapshot.
    let before = web_mode_test_rows(&tx);
    tx.commit().unwrap();
    let error = done_rx
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap_err();
    worker.join().unwrap();
    let after = web_mode_test_rows(&db);
    assert_eq!(
        after.iter().find(|(n, _)| n == "projects").unwrap().1,
        before.iter().find(|(n, _)| n == "projects").unwrap().1,
        "typed no-send receipt mutated business records"
    );
    assert!(
        error.contains("未发出") && error.contains("specialist_no_send_receipt:"),
        "{error}"
    );
    specialist_financial_unchanged(&db, &before, false);
    assert!(seen.lock().unwrap().is_empty());
    drop(occupied);
    let before_replay = web_mode_test_rows(&db);
    assert!(multi_agent_child_round_transport(
        &replay_context,
        &replay_lease,
        &replay_child,
        "readonly",
        json!({})
    )
    .is_err());
    web_mode_assert_rows(&db, &before_replay);
    assert!(seen.lock().unwrap().is_empty());
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn specialist_unsent_local_expired_worker_fact_denies_collateral_and_keeps_all_rows() {
    use crate::agent_runtime::multi_agent::specialist;
    let (root, context, lease, child) = specialist_financial_fixture();
    let db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(call) =
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap()
    else {
        panic!()
    };
    db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','-1 second') WHERE assignment_id=?1",[&child.assignment_id]).unwrap();
    db.execute_batch("CREATE TRIGGER unsent_fact_collateral AFTER INSERT ON agent_model_cost_facts BEGIN UPDATE projects SET status='archived'; END;").unwrap();
    let before = web_mode_test_rows(&db);
    assert!(specialist::record_not_sent(&db, &call, "user_cancelled").is_err());
    web_mode_assert_rows(&db, &before);
    let cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &lease.root_run_id,
        Some(&child.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!((cost.consumed, cost.indeterminate), (0, 0));
    assert_eq!(cost.reserved, 1);
    assert!(specialist::record_not_sent(&db, &call, "user_cancelled").is_err());
    web_mode_assert_rows(&db, &before);
    let tx = db.unchecked_transaction().unwrap();
    let error = specialist::record_not_sent(&tx, &call, "user_cancelled").unwrap_err();
    assert!(
        error.contains("specialist_phase_private_transaction_required"),
        "{error}"
    );
    assert!(!db.is_autocommit());
    web_mode_assert_rows(&tx, &before);
    tx.rollback().unwrap();
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
