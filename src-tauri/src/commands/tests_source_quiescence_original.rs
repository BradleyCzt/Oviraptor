#[test]
fn source_quiescence_actual_launcher_pause_keeps_paid_sdk_and_finishes_after_original_exit() {
    use std::sync::{atomic::Ordering, mpsc, Arc, Mutex};
    let (arrive, arrived) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let released = Arc::new(Mutex::new(released));
    let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(move |_| {
        arrive.send(()).unwrap();
        let _ = released
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
        (200,"application/json",json!({"choices":[{"message":{"role":"assistant","content":"late response"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let (root, db, record) =
        source_dispatch_fixture(&source_specialist_test_environment(port), None);
    let path = root.join("oviraptor.sqlite3");
    let work = root.join("attempt-0001");
    launch_native_source_pipeline(
        path.clone(),
        root.clone(),
        record.scan_id.clone(),
        1,
        work.clone(),
        record.source_path.clone(),
        record.scan_type.clone(),
        record.diff_base.clone(),
    )
    .unwrap();
    arrived
        .recv_timeout(Duration::from_secs(8))
        .expect("production Source launcher must reach actual SDK");
    let (run, target, plan): (String, String, String) = db
        .query_row(
            "SELECT id,target_url,plan_json FROM agent_runs WHERE role='coordinator'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<JsonValue>(&plan).unwrap()["surface"],
        "source"
    );
    assert!(
        crate::agent_runtime::execution_owner::probe_native_invocation(
            &path,
            &record.scan_id,
            1,
            "branch",
            "source"
        )
        .is_err()
    );
    assert!(
        crate::agent_runtime::execution_owner::probe_native_invocation(
            &path,
            &record.scan_id,
            1,
            "source-model",
            "source"
        )
        .is_err()
    );
    assert!(
        crate::agent_runtime::execution_owner::probe_native_invocation(
            &path,
            &record.scan_id,
            1,
            "target",
            &target
        )
        .unwrap()
        .is_none(),
        "Source never owns a Web target invocation"
    );
    let before = source_exit_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        claim_scan_quiescence_in(&tx, &path, &record.scan_id).is_err(),
        "actual caller must return before quiescence"
    );
    tx.rollback().unwrap();
    assert_eq!(source_exit_snapshot(&db), before);
    request_sentinel_pause(&path, &record.scan_id).unwrap();
    let _ = release.send(());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let status = loop {
        let status: String = db
            .query_row(
                "SELECT status FROM sentinel_scans WHERE id=?1",
                [&record.scan_id],
                |r| r.get(0),
            )
            .unwrap();
        if status == "paused" || std::time::Instant::now() >= deadline {
            break status;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    stop.store(true, Ordering::SeqCst);
    assert_eq!(
        status,
        "paused",
        "actual Source branch must finalize pause: {:?}",
        finish_sentinel_pause(&path, &record.scan_id, 1)
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(arrived.try_recv().is_err());
    let fee = crate::agent_runtime::multi_agent::budget::balance(&db, &run, None, "model_requests")
        .unwrap();
    assert_eq!(fee.consumed + fee.indeterminate + fee.reserved, 1);
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&run],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        plan
    );
    assert_ne!(
        db.query_row("SELECT status FROM agent_runs WHERE id=?1", [&run], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "terminal"
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_messages", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let after = source_exit_snapshot(&db);
    assert!(!finish_sentinel_pause(&path, &record.scan_id, 1).unwrap());
    assert_eq!(source_exit_snapshot(&db), after);
    assert!(
        crate::agent_runtime::execution_owner::probe_native_invocation(
            &path,
            &record.scan_id,
            1,
            "target",
            &target
        )
        .unwrap()
        .is_none()
    );
    // Only original identities are accepted, even after the business pause.
    // Holding the real producer's existing Source inode independently of the
    // outer branch must still prevent aggregate quiescence.
    let source_owner = crate::agent_runtime::execution_owner::probe_native_invocation(
        &path,
        &record.scan_id,
        1,
        "source-model",
        "source",
    )
    .unwrap()
    .unwrap();
    let before = source_exit_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        claim_scan_quiescence_in(&tx, &path, &record.scan_id).unwrap_err(),
        "scan_quiescence_worker_active_or_unverifiable"
    );
    tx.rollback().unwrap();
    assert_eq!(source_exit_snapshot(&db), before);
    drop(source_owner);
    for sql in [
        "UPDATE agent_runs SET plan_json=json_set(plan_json,'$.surface','web') WHERE role='coordinator'",
        "UPDATE agent_runs SET plan_json=json_set(plan_json,'$.targetRequestsGranted',1) WHERE role='coordinator'",
        "UPDATE agent_runs SET target_url='https://renamed.example.test/' WHERE role='coordinator'",
        "UPDATE agent_runs SET attempt_number=99 WHERE role='coordinator'",
        "UPDATE agent_runs SET orchestration_policy='single' WHERE role='coordinator'",
        "UPDATE agent_runs SET plan_hash='changed' WHERE role='coordinator'",
    ] {
        db.execute_batch("SAVEPOINT source_scope").unwrap();
        db.execute_batch(sql).unwrap();
        let damaged=source_exit_snapshot(&db);
        assert_eq!(claim_scan_quiescence_in(&db,&path,&record.scan_id).unwrap_err(),"scan_quiescence_original_scope_unverifiable","{sql}");
        assert_eq!(source_exit_snapshot(&db),damaged,"denial changed original rows: {sql}");
        db.execute_batch("ROLLBACK TO source_scope; RELEASE source_scope").unwrap();
        assert_eq!(source_exit_snapshot(&db),before,"rollback did not restore original rows: {sql}");
    }
    // Absence of the original Source inode must never be repaired by the
    // finalizer. This damage is confined to the disposable test directory.
    use sha2::Digest;
    let canonical = fs::canonicalize(&path).unwrap();
    let mut name = canonical.file_name().unwrap().to_os_string();
    name.push(".invocations");
    let key = serde_json::to_vec(&(&record.scan_id, 1, "source-model", "source")).unwrap();
    let original = canonical
        .with_file_name(name)
        .join(format!("{:x}.lock", sha2::Sha256::digest(key)));
    assert!(original.is_file());
    fs::remove_file(&original).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        claim_scan_quiescence_in(&tx, &path, &record.scan_id).unwrap_err(),
        "scan_quiescence_original_source_exit_missing"
    );
    tx.rollback().unwrap();
    assert!(!original.exists());
    assert_eq!(source_exit_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_quiescence_unborn_and_old_attempt_models_block_pause_without_row_changes() {
    for attempt in [0, 1] {
        let (root, path, db) = pause_fixture();
        let owner = claim_native_invocation(&path, "start-test", attempt, "source-model", "source")
            .unwrap();
        request_sentinel_pause(&path, "start-test").unwrap();
        let before = source_exit_snapshot(&db);
        assert_eq!(
            finish_sentinel_pause(&path, "start-test", 1).unwrap_err(),
            "scan_quiescence_worker_active_or_unverifiable"
        );
        assert_eq!(source_exit_snapshot(&db), before);
        drop(owner);
        assert!(finish_sentinel_pause(&path, "start-test", 1).unwrap());
        assert_eq!(pause_status(&db), "paused");
        assert_eq!(
            db.query_row("SELECT count(*) FROM agent_root_budget_attempts", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
