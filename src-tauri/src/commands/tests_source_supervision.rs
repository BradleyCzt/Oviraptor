include!("tests_source_supervision_born_fixture.rs");

#[test]
fn assignment_attempt_supervisor_source_inflight_stop_preserves_claim_without_regrant() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{budget, scheduler, source, supervisor::WorkerSupervisor},
    };
    use std::sync::{atomic::Ordering, mpsc, Arc, Mutex};
    let (arrived_tx, arrived_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Arc::new(Mutex::new(release_rx));
    let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(move |_| {
        let _ = arrived_tx.send(());
        let _ = release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
        (200, "application/json", json!({"choices":[{"message":{"content":"late response","role":"assistant"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}).to_string())
    }));
    let environment = source_specialist_test_environment(port);
    let (root, db, record, lease) = source_specialist_true_born_model_fixture(&environment);
    let role = AgentRole::RepoMapper;
    let slice = source::task_slice(&db, &lease, role).unwrap();
    let input = source::assessment_input(&db, &lease, role).unwrap();
    let (tokens, _) = source_assessment_budget(
        &source_assessment_messages("source assessment", &input),
        &agent_model_profile(&environment, None).unwrap(),
    )
    .unwrap();
    let child = scheduler::prepare_readonly_child(
        &db,
        &lease,
        role,
        "source_results_ready",
        &slice,
        tokens,
    )
    .unwrap();
    let database = root.join("oviraptor.sqlite3");
    let guard = WorkerSupervisor::start(&database, &lease).unwrap();
    let context = SpecialistTransportContext {
        supervision: Some(guard.ticket()),
        db_path: &database,
        scan_id: &record.scan_id,
        attempt_number: 1,
        target_key: &lease.target_key,
        run_id: &lease.root_run_id,
        environment: &environment,
        proxy: None,
        usage_dir: &root,
        deadline: None,
    };
    std::thread::scope(|scope| {
        let (done_tx, done_rx) = mpsc::channel();
        let (ctx, actor, worker, material) = (&context, &lease, &child, &input);
        scope.spawn(move || {
            let _ = done_tx.send(specialist_round_transport(
                ctx,
                actor,
                worker,
                "source assessment",
                material.clone(),
            ));
        });
        arrived_rx.recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|error| panic!("original Source provider arrival: {error}; executor returned: {:?}", done_rx.try_recv()));
        drop(guard);
        let result = done_rx.recv_timeout(Duration::from_secs(3));
        let _ = release_tx.send(());
        assert!(result
            .expect("Source SDK must cancel before provider response")
            .is_err());
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while seen.lock().unwrap().is_empty() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_messages WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let cost = budget::balance(
        &db,
        &lease.root_run_id,
        Some(&child.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!((cost.consumed, cost.indeterminate), (0, 1));
    let before = application_table_snapshot(&db);
    assert!(
        specialist_round_transport(&context, &lease, &child, "source assessment", input).is_err()
    );
    assert!(application_table_snapshot(&db) == before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    stop.store(true, Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_source_tool_stop_cancels_before_waiting_for_database_writer() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{scheduler, source, source_rounds, supervisor::WorkerSupervisor},
    };
    let environment = source_specialist_test_environment(9);
    let (root, db, record, lease) = source_specialist_true_born_model_fixture(&environment);
    db.execute(
        "UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",
        [&lease.root_run_id],
    )
    .unwrap();
    let role = AgentRole::RepoMapper;
    let slice = source::tool_task_slice(&db, &lease, role, 1).unwrap();
    let capabilities = source::tool_capabilities(role).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &lease,
        role,
        AgentLane::ReadOnlyAnalysis,
        "source_tools_ready",
        &slice,
        1,
        &capabilities,
        24_000,
        3,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &lease, &child).unwrap();
    let database = root.join("oviraptor.sqlite3");
    let guard = WorkerSupervisor::start(&database, &lease).unwrap();
    let context = SpecialistTransportContext {
        supervision: Some(guard.ticket()),
        db_path: &database,
        scan_id: &record.scan_id,
        attempt_number: 1,
        target_key: &lease.target_key,
        run_id: &lease.root_run_id,
        environment: &environment,
        proxy: None,
        usage_dir: &root,
        deadline: None,
    };
    let request = json!({"messages":[{"role":"user","content":json!({"sourceTask":slice}).to_string()}],
        "tools":capabilities.iter().map(|name|json!({"type":"function","function":{"name":name}})).collect::<Vec<_>>()});
    assert!(matches!(
        source_rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, |tx| {
            authorize_source_tool_phase(tx, &context, &lease, &child)
        })
        .unwrap(),
        source_rounds::Start::Dispatch(_)
    ));
    let cancel = source_tool_model_cancel_token(&context, &lease, &child);
    assert!(!cancel.is_cancelled());
    let writer =
        rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    let before = application_table_snapshot(&writer);
    let healthy_check = std::time::Instant::now();
    assert!(
        !cancel.is_cancelled(),
        "a healthy writer cannot cancel a live Source worker or block its parent polling"
    );
    assert!(healthy_check.elapsed() < Duration::from_millis(250));
    drop(guard);
    let start = std::time::Instant::now();
    assert!(cancel.is_cancelled());
    assert!(
        start.elapsed() < Duration::from_millis(250),
        "a stopped parent cannot wait for the ten-second SQLite writer timeout"
    );
    assert!(application_table_snapshot(&writer) == before);
    writer.rollback().unwrap();
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_source_stopped_parent_cannot_schedule_initial_business_work() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let environment = source_specialist_test_environment(9);
    let (root, db, record, lease) = source_specialist_fixture_model(Some(&environment));
    let database = root.join("oviraptor.sqlite3");
    let guard = WorkerSupervisor::start(&database, &lease).unwrap();
    let context = SpecialistTransportContext {
        supervision: Some(guard.ticket()),
        db_path: &database,
        scan_id: &record.scan_id,
        attempt_number: 1,
        target_key: &lease.target_key,
        run_id: &lease.root_run_id,
        environment: &environment,
        proxy: None,
        usage_dir: &root,
        deadline: None,
    };
    drop(guard);
    let before = application_table_snapshot(&db);
    let profile = agent_model_profile(&environment, None).unwrap();
    let result = run_source_initial_phases_from(&db, &context, &lease, &profile, 0);
    assert!(result.is_err());
    assert!(
        application_table_snapshot(&db) == before,
        "a stopped parent must not freeze guidance or issue a worker before checking supervision"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
