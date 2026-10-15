// Actual process boundaries reuse the original private transaction test barrier.
fn paid_multi_deletion_child(h: &AgentHarness, mode: &str) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().unwrap()).args([
        "scanning::agent_tests::scan_deletion_paid_multi_process_death_before_commit_keeps_all_original_sources","--exact","--test-threads=1"])
        .env("OVIRAPTOR_PAID_DELETE_FIXTURE_BINDING",json!([h.db_path.to_string_lossy(),h.context.scan_id]).to_string())
        .env("OVIRAPTOR_PAID_DELETE_FIXTURE_MODE",mode).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap()
}
#[test]
fn scan_deletion_paid_multi_process_death_before_commit_keeps_all_original_sources() {
    if paid_scan_deletion_subprocess_entry() {
        return;
    }
    let (h, root, stop) = paid_multi_archive_fixture();
    let db = db::open(&h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let mut child = paid_multi_deletion_child(&h, "before_commit");
    let marker = h.db_path.with_extension("deletion-before-commit");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !marker.exists() && Instant::now() < deadline {
        assert!(
            child.try_wait().unwrap().is_none(),
            "child stopped before verified deletion transaction"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    if !marker.exists() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("verified precommit boundary missing");
    }
    assert!(super::tests::application_table_snapshot(&db) == before);
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    assert!(super::tests::application_table_snapshot(&db) == before);
    multi_exit_receipt_read(&db, &root).unwrap();
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    assert!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &h.context.scan_id).unwrap()
    );
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
#[test]
fn scan_deletion_paid_multi_lost_commit_reply_keeps_one_archive_and_original_costs() {
    let (h, _root, stop) = paid_multi_archive_fixture();
    let db = db::open(&h.db_path).unwrap();
    let fees = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid",
        [],
    )
    .unwrap();
    let mut child = paid_multi_deletion_child(&h, "lost_reply");
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("commit reply child timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!status.success());
    assert!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &h.context.scan_id).unwrap()
    );
    let after = super::tests::application_table_snapshot(&db);
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == after);
    assert!(
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid",
            []
        )
        .unwrap()
            == fees
    );
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
