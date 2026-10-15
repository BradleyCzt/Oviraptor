// This helper runs real work only when the parent supplies a controlled fixture.
#[test]
fn a02_test_subprocess_driver() {
    let Ok(path) = std::env::var("OVIRAPTOR_TEST_FINANCIAL_DRIVER_DB") else {
        return;
    };
    let scan = std::env::var("OVIRAPTOR_TEST_FINANCIAL_DRIVER_SCAN").unwrap();
    let root = std::env::var("OVIRAPTOR_TEST_FINANCIAL_EXIT_AFTER_PUBLICATION").unwrap();
    let _real = RealSpecialistTransport::enter();
    native_financial_continuation_work(Path::new(&path), &scan, &root).unwrap();
    panic!("child must exit at the exact paid publication fault boundary");
}

#[test]
fn financial_continuation_actual_subprocess_exit_after_publication_reentry_closes_without_sdk() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("issue-publication-process-exit");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let issued = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        preview["proposalHash"].as_str().unwrap(),
    )
    .unwrap();
    let root = issued["newRootRunId"].as_str().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "scanning::agent_tests::a02_test_subprocess_driver",
            "--test-threads=1",
        ])
        .env("OVIRAPTOR_TEST_FINANCIAL_DRIVER_DB", &f.context.db_path)
        .env("OVIRAPTOR_TEST_FINANCIAL_DRIVER_SCAN", &f.context.scan_id)
        .env("OVIRAPTOR_TEST_FINANCIAL_EXIT_AFTER_PUBLICATION", root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(86),
        "child stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    let pending = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        preview["proposalHash"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        pending["assessmentCompleted"], true,
        "paid publication exists before closure"
    );
    assert_eq!(
        pending["attemptClosed"], false,
        "process exit must not fabricate an exit proof"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&f.context.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "scanning"
    );
    let before = web_mode_test_rows(&db);
    let result =
        native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, root).unwrap();
    assert_eq!(result["assessmentCompleted"], true);
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "closure reentry must read the paid publication, not resend its SDK"
    );
    let closed = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        preview["proposalHash"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(closed["approvalId"], issued["approvalId"]);
    assert_eq!(closed["attemptClosed"], true);
    assert_ne!(
        web_mode_test_rows(&db),
        before,
        "reentry must actually write the missing closure"
    );
    let snapshot = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::budget::clock::FinalClock::verify_original_exit(
        &snapshot, root,
    )
    .unwrap();
    drop(snapshot);
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&f.context.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "partial"
    );
}
