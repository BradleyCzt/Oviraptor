#[test]
fn specialist_cancelled_in_model_queue_has_durable_no_send_proof() {
    use crate::agent_runtime::model::{admission, CancelToken};
    use std::{sync::mpsc, time::{Duration, Instant}};

    let (root, mut context, lease, child) = specialist_journal_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", proposal_model_response("unexpected"))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    context.environment.deployment = "local".into();
    let gate = admission::gate_for(true, &context.environment.api_base);
    let occupied = gate.acquire(&CancelToken::new()).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let replay_context = context.clone();
    let replay_lease = lease.clone();
    let replay_child = child.clone();
    let path = context.db_path.clone();
    let assignment = child.assignment_id.clone();
    let child_id = child.run_id.clone();
    let worker = std::thread::spawn(move || {
        let result = multi_agent_child_round_transport(
            &context, &lease, &child, "readonly", serde_json::json!({}),
        );
        done_tx.send(result).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while gate.queued() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(gate.queued(), 1, "specialist did not wait for admission");
    let connection = db::open(&path).unwrap();
    connection.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1", [&child_id]).unwrap();
    assert!(done_rx.recv_timeout(Duration::from_secs(3)).unwrap().is_err());
    worker.join().unwrap();
    assert!(seen.lock().unwrap().is_empty(), "the model provider was never contacted");
    let (state, code): (String, String) = connection.query_row(
        "SELECT state,failure_code FROM agent_specialist_calls WHERE assignment_id=?1",
        [&assignment], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(state, "executing");
    assert_eq!(code, "model_cancelled_before_transport");
    connection.execute("UPDATE agent_runs SET cancel_requested_at='' WHERE id=?1", [&child_id]).unwrap();
    drop(occupied);
    let replay = multi_agent_child_round_transport(
        &replay_context, &replay_lease, &replay_child, "readonly", serde_json::json!({}),
    ).unwrap_err();
    assert!(replay.contains("specialist_model_not_sent_new_assignment_required"), "{replay}");
    assert!(seen.lock().unwrap().is_empty(), "old assignment must not replay the provider call");
    assert_eq!(failed_specialist_error(&connection, &replay_lease, &replay_child, "model cancelled before transport"),
        "model cancelled before transport");
    let (assignment_state, reserved): (String, i64) = connection.query_row(
        "SELECT state,reserved_requests FROM agent_assignments WHERE id=?1",
        [&assignment], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!((assignment_state.as_str(), reserved), ("failed", 0));
    let global_reserved: i64 = connection.query_row(
        "SELECT reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [&replay_lease.root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(global_reserved, 0);
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn specialist_cancel_after_provider_arrival_keeps_unknown_usage_reserved() {
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    let (root, mut context, lease, child) = specialist_journal_fixture();
    let (arrived_tx, arrived_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Arc::new(Mutex::new(release_rx));
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        arrived_tx.send(()).unwrap();
        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(5));
        (200, "application/json", proposal_model_response("late"))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let path = context.db_path.clone();
    let assignment = child.assignment_id.clone();
    let child_id = child.run_id.clone();
    let replay_context = context.clone();
    let replay_lease = lease.clone();
    let replay_child = child.clone();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        done_tx.send(multi_agent_child_round_transport(
            &context, &lease, &child, "readonly", serde_json::json!({}),
        )).unwrap();
    });
    arrived_rx.recv_timeout(Duration::from_secs(5)).expect("provider received request");
    let connection = db::open(&path).unwrap();
    connection.execute(
        "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",
        [&child_id],
    ).unwrap();
    let cancelled = done_rx.recv_timeout(Duration::from_secs(3));
    release_tx.send(()).unwrap();
    assert!(cancelled.expect("in-flight cancellation must abort").is_err());
    worker.join().unwrap();
    let recorded = Instant::now() + Duration::from_secs(2);
    while seen.lock().unwrap().is_empty() && Instant::now() < recorded {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    let (state, code): (String, String) = connection.query_row(
        "SELECT state,failure_code FROM agent_specialist_calls WHERE assignment_id=?1",
        [&assignment], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_ne!(code, "model_cancelled_before_transport");
    assert_ne!(state, "received");
    connection.execute("UPDATE agent_runs SET cancel_requested_at='' WHERE id=?1", [&child_id]).unwrap();
    assert!(multi_agent_child_round_transport(
        &replay_context, &replay_lease, &replay_child, "readonly", serde_json::json!({}),
    ).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1, "unknown request cannot replay");
    failed_specialist_error(&connection, &replay_lease, &replay_child, "in-flight cancellation");
    assert_specialist_usage_pending(&connection, "spa_api_mapper", 8_000);
    drop(connection);
    let _ = fs::remove_dir_all(root);
}
