#[test]
fn source_financial_exit_pause_between_live_check_and_closure_cannot_publish_terminal() {
    use std::sync::{atomic::Ordering, Arc};
    let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(|_| {
        (
            500,
            "application/json",
            "{\"error\":\"provider unavailable\"}".into(),
        )
    }));
    let (root, db, record) =
        source_dispatch_fixture(&source_specialist_test_environment(port), None);
    analysis_view_run(&root, &record, |_, engine, _, _, scratch, _| {
        Ok(source_regression_outcome(engine, scratch))
    })
    .unwrap();
    let path = root.join("oviraptor.sqlite3");
    let work = root.join("attempt-0001");
    let registered = prepare_native_source_coordinator(&db, &record.scan_id, 1, &work).unwrap();
    let original =
        native_source_fresh_finance::original_for_execution(&db, &registered.run_id).unwrap();
    let plan = NativeSourcePlan::load(&db, &record.scan_id, 1).unwrap();
    let (pause_path, pause_scan) = (path.clone(), record.scan_id.clone());
    source_failure_cutover_once_for_test(move || {
        assert_eq!(request_sentinel_pause(&pause_path, &pause_scan).unwrap(), 1);
    });
    let result = run_native_source_assessments(&path, &record.scan_id, 1, &work);
    assert!(result.is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&record.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "pausing"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM agent_runs WHERE id=?1",
            [&original.root_run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "running",
        "pause cutover must not publish a new business terminal"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_messages WHERE root_run_id=?1",
            [&original.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let fee = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &original.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(fee.consumed + fee.reserved + fee.indeterminate, 1);
    assert_eq!(
        NativeSourcePlan::load(&db, &record.scan_id, 1).unwrap(),
        plan
    );
    assert!(finish_sentinel_pause(&path, &record.scan_id, 1).unwrap());
    stop.store(true, Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
