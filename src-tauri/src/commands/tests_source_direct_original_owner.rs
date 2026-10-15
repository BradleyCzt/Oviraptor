// Actual direct Source SDK path; a healthy current C/worker is not original finance.
#[test]
fn source_direct_sdk_missing_original_owner_refuses_send_and_keeps_all_rows() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{scheduler, source, supervisor::WorkerSupervisor},
    };
    let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(
        |_| {
            (200, "application/json", json!({
            "choices":[{"message":{"role":"assistant","content":"Old Source Root reached SDK"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}
        }).to_string())
        },
    ));
    let model = source_specialist_test_environment(port);
    // Intentionally historical; never switch this negative to a fresh issuer.
    let (root, db, record, actor) = source_specialist_fixture_model(Some(&model));
    let role = AgentRole::RepoMapper;
    let slice = source::task_slice(&db, &actor, role).unwrap();
    let input = source::assessment_input(&db, &actor, role).unwrap();
    let (tokens, _) = source_assessment_budget(
        &source_assessment_messages(SOURCE_ASSESSMENT_SYSTEM, &input),
        &agent_model_profile(&model, None).unwrap(),
    )
    .unwrap();
    let child = scheduler::prepare_readonly_child(
        &db,
        &actor,
        role,
        "source_results_ready",
        &slice,
        tokens,
    )
    .unwrap();
    let path = root.join("oviraptor.sqlite3");
    let supervisor = WorkerSupervisor::start(&path, &actor).unwrap();
    let context = SpecialistTransportContext {
        supervision: Some(supervisor.ticket()),
        db_path: &path,
        scan_id: &record.scan_id,
        attempt_number: 1,
        target_key: &actor.target_key,
        run_id: &actor.root_run_id,
        environment: &model,
        proxy: None,
        usage_dir: &root,
        deadline: None,
    };
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&db, &actor).unwrap();
    crate::agent_runtime::multi_agent::attempts::require_live_for_run(&db, &child.run_id).unwrap();
    let before = crate::scanning::web_mode_test_rows(&db);
    let result =
        specialist_round_transport(&context, &actor, &child, SOURCE_ASSESSMENT_SYSTEM, input);
    let sent = seen.lock().unwrap().len();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let unchanged = crate::scanning::web_mode_test_rows(&db) == before;
    drop(supervisor);
    drop(db);
    fs::remove_dir_all(root).unwrap();
    assert!(
        result.is_err(),
        "current C/worker sent SDK without original financial owner: {result:?}"
    );
    assert_eq!(
        sent, 0,
        "missing original finance must refuse physical model IO"
    );
    assert!(
        unchanged,
        "no invoice, fee, heartbeat, event or business write may escape rejection"
    );
}
