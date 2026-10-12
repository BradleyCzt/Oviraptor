#[test]
fn scan_deletion_paid_single_real_sdk_unknown_cost_keeps_original_rows_after_branch_finalizer() {
    let mut reply: JsonValue = serde_json::from_str(&model_round(
        &[(
            "finish_target",
            json!({"coverage":closing_ledger(&[]),"stopReason":"unknown fee"}),
        )],
        100,
    ))
    .unwrap();
    reply.as_object_mut().unwrap().remove("usage");
    let (f, context, owned, calls) =
        original_terminal_dispatch_reply(true, Some(reply.to_string()));
    assert_eq!(calls, 1);
    let root = owned
        .original_terminal
        .root_run_id
        .as_ref()
        .unwrap()
        .clone();
    let mut tally = AgentPipelineTally::default();
    let published =
        record_owned_agent_target_outcome(&f.path, &f.scan, &context.route, &owned, &mut tally);
    let report = json!({"targets":tally.counted(),"published":published,"originalRoot":root,"stopCode":owned.outcome.terminal_code(),"detail":owned.outcome.detail()});
    drop(owned);
    assert!(finish_native_branch(
        &f.path,
        &f.scan,
        1,
        "web",
        "partial",
        "original unknown fee retained",
        &report
    )
    .unwrap());
    let db = db::open(&f.path).unwrap();
    let known =
        crate::agent_runtime::multi_agent::budget::balance(&db, &root, None, "model_requests")
            .unwrap();
    assert_eq!((known.consumed, known.indeterminate), (1, 0));
    for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS[..3]
        .iter()
        .copied()
    {
        let fee = crate::agent_runtime::multi_agent::budget::balance(&db, &root, None, dimension)
            .unwrap();
        assert!(fee.indeterminate > 0);
    }
    let before = single_finally_physical(&db);
    let file = fs::read(context.target_dir.join("frontend-evidence.json")).unwrap();
    assert!(delete_sentinel_scan_inner(&f.path, &f.scan).is_err());
    assert_eq!(single_finally_physical(&db), before);
    assert_eq!(
        fs::read(context.target_dir.join("frontend-evidence.json")).unwrap(),
        file
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM native_deleted_scan_audits", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_deleted_scan_audits_v2",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn scan_deletion_paid_single_implicit_fk_business_owner_is_preserved() {
    let (f, _context, _root) = paid_single_deletion_fixture();
    let db = db::open(&f.path).unwrap();
    db.execute_batch("CREATE TABLE protected_business(owner TEXT REFERENCES sentinel_scans ON DELETE CASCADE,value BLOB);").unwrap();
    db.execute(
        "INSERT INTO protected_business VALUES(?1,X'00FF41')",
        [&f.scan],
    )
    .unwrap();
    let before = single_finally_physical(&db);
    assert!(delete_sentinel_scan_inner(&f.path, &f.scan).is_err());
    assert_eq!(single_finally_physical(&db), before);
}

#[test]
fn scan_deletion_paid_single_original_financial_control_without_exit_cannot_use_old_label() {
    let (f, context) = single_http_inflight_fixture(65530);
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(finish_native_branch(
        &f.path,
        &f.scan,
        1,
        "web",
        "partial",
        "never dispatched",
        &json!({"originalRoot":root})
    )
    .unwrap());
    // Isolated corruption of an old label, never an execution or exit receipt.
    db.execute(
        "UPDATE agent_runs SET status='completed' WHERE id=?1",
        [root],
    )
    .unwrap();
    let before = single_finally_physical(&db);
    assert!(delete_sentinel_scan_inner(&f.path, &f.scan).is_err());
    assert_eq!(single_finally_physical(&db), before);
}

#[test]
fn scan_deletion_paid_single_deleted_id_cannot_start_freeze_or_dispatch_native_sdk() {
    let (f, mut context, root) = paid_single_deletion_fixture();
    let plan = web_mode_test_plan(&f);
    let mut db = db::open(&f.path).unwrap();
    delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
    let before = single_finally_physical(&db);
    assert!(web_mode_test_start(&f.root, &mut db, &f.scan, WebStartMode::Confirm).is_err());
    assert_eq!(single_finally_physical(&db), before);
    assert!(persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &f.target, &plan).is_err());
    assert_eq!(single_finally_physical(&db), before);
    assert!(runtime_open_run_for_attempt(&f.path, &f.scan, &context.route, 1).is_none());
    assert_eq!(single_finally_physical(&db), before);
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", model_round(&[], 20))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&context.environment, None).unwrap(),
        &[],
    );
    assert!(native_model_transport(
        &context,
        &client,
        vec![json!({"role":"user","content":"deleted scope must not execute"})],
        &[],
        13
    )
    .is_err());
    assert_eq!(seen.lock().unwrap().len(), 0);
    assert_eq!(single_finally_physical(&db), before);
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, &root)
            .is_err()
    );
    assert!(crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &f.scan).unwrap());
}
