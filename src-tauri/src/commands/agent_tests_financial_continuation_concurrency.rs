// Production approval/worker calls race under real OS ownership and SQLite.
#[test]
fn financial_continuation_concurrent_approval_and_worker_use_one_birth_and_new_sdk() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("issue-concurrent");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let hash = preview["proposalHash"].as_str().unwrap().to_string();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let (path, data, scan, root, receipt, hash, barrier) = (
                f.context.db_path.clone(),
                f.f.root.clone(),
                f.context.scan_id.clone(),
                f.actor.root_run_id.clone(),
                receipt.clone(),
                hash.clone(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                barrier.wait();
                native_financial_continuation_approve_on(
                    &path, &data, &scan, 1, &root, &receipt, &hash,
                )
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(results.iter().any(Result::is_ok));
    for error in results.iter().filter_map(|r| r.as_ref().err()) {
        assert!(error.starts_with("native_invocation_not_owned:"), "{error}");
    }
    let issued = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        &hash,
    )
    .unwrap();
    for result in results.iter().filter_map(|r| r.as_ref().ok()) {
        assert_eq!(result["approvalId"], issued["approvalId"]);
        assert_eq!(result["newRootRunId"], issued["newRootRunId"]);
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_financial_continuations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE scan_id=?1 AND attempt_number=2",
            [&f.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "approval never dispatches an SDK request by itself"
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let (path, scan, root, barrier) = (
                f.context.db_path.clone(),
                f.context.scan_id.clone(),
                issued["newRootRunId"].as_str().unwrap().to_string(),
                barrier.clone(),
            );
            std::thread::spawn(move || {
                barrier.wait();
                native_financial_continuation_work(&path, &scan, &root)
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(results.iter().any(Result::is_ok), "{results:?}");
    for error in results.iter().filter_map(|r| r.as_ref().err()) {
        assert!(error.starts_with("native_invocation_not_owned:"), "{error}");
    }
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "concurrent workers must not duplicate the new paid request"
    );
    let final_receipt = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        &hash,
    )
    .unwrap();
    assert_eq!(final_receipt["approvalId"], issued["approvalId"]);
    assert_eq!(final_receipt["assessmentCompleted"], true);
    assert_eq!(final_receipt["attemptClosed"], true);
}

#[test]
fn financial_continuation_startup_column_and_signed_runtime_tamper_refuse_before_sdk() {
    for case in ["approval-column", "signed-runtime"] {
        let _real = RealSpecialistTransport::enter();
        let (f, seen, receipt) = financial_issuance_fixture(&format!("startup-{case}"));
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
        assert!(private_web_mode_on(&db, &f.context.scan_id, 2).is_ok());
        db.execute_batch("DROP TRIGGER financial_startup_no_update")
            .unwrap();
        if case == "approval-column" {
            db.execute("UPDATE agent_financial_startup_bindings SET approval_id=?1 WHERE scan_id=?2 AND attempt_number=2",
                params![Uuid::new_v4().to_string(), f.context.scan_id]).unwrap();
        } else {
            let raw: String = db.query_row("SELECT binding_fact_json FROM agent_financial_startup_bindings WHERE scan_id=?1 AND attempt_number=2",
                [&f.context.scan_id], |r| r.get(0)).unwrap();
            let mut binding: JsonValue = serde_json::from_str(&raw).unwrap();
            binding["newPlanHash"] = "f".repeat(64).into();
            db.execute("UPDATE agent_financial_startup_bindings SET binding_fact_json=?1 WHERE scan_id=?2 AND attempt_number=2",
                params![binding.to_string(), f.context.scan_id]).unwrap();
        }
        let before = web_mode_test_rows(&db);
        assert_eq!(
            private_web_mode_on(&db, &f.context.scan_id, 2).unwrap_err(),
            "financial_continuation_startup_changed"
        );
        web_mode_assert_rows(&db, &before);
        assert!(native_financial_continuation_work(
            &f.context.db_path,
            &f.context.scan_id,
            issued["newRootRunId"].as_str().unwrap()
        )
        .is_err());
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "bad startup proof must not send a new SDK request"
        );
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_runs WHERE scan_id=?1 AND attempt_number=2",
                [&f.context.scan_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
}
