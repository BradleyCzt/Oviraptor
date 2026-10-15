#[test]
fn source_financial_exit_expired_original_c_has_financial_fact_without_dispatch_or_publication() {
    let (root, db, c, result) = source_reviewer_execution_fixture_using_calls(
        "valid",
        None,
        4,
        None,
        0,
        |root, db, record, c| {
            db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[&c.root_run_id]).unwrap();
            let before = source_exit_snapshot(db);
            let wall = source_exit_wall(db, &c.root_run_id);
            let result = run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            );
            assert!(result.is_err());
            source_exit_require_fact(db, c, false);
            source_exit_assert_snapshot(db, c, &before);
            assert_eq!(source_exit_wall(db, &c.root_run_id), wall);
            let before = crate::scanning::web_mode_test_rows(db);
            assert!(run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001")
            )
            .is_err());
            crate::scanning::web_mode_assert_rows(db, &before);
            result
        },
    );
    assert!(result.is_err());
    source_exit_require_fact(&db, &c, false);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn source_financial_exit_naturally_exhausted_original_clock_keeps_full_elapsed_and_zero_sdk() {
    let (root, db, c, result) = source_reviewer_execution_fixture_configured_before_birth(
        "valid",
        None,
        4,
        None,
        0,
        (None, |db, record| {
            record.policy["webModeCeiling"] = json!("quick");
            db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentQuickTimeout',30)",[]).unwrap();
        }),
        |root, db, record, c| {
            let deadline = std::time::Instant::now() + Duration::from_secs(40);
            loop {
                match crate::agent_runtime::multi_agent::budget::clock::remaining(
                    db,
                    &c.root_run_id,
                ) {
                    Err(e) if e == "budget_wall_time_exhausted" => break,
                    Ok(_) => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    other => panic!("{other:?}"),
                }
            }
            let before = source_exit_snapshot(db);
            let wall = source_exit_wall(db, &c.root_run_id);
            let result = run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            );
            assert!(result.is_err());
            source_exit_require_fact(db, c, true);
            source_exit_assert_snapshot(db, c, &before);
            assert_eq!(source_exit_wall(db, &c.root_run_id), wall);
            result
        },
    );
    assert!(result.is_err());
    source_exit_require_fact(&db, &c, true);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn source_financial_exit_pre_execute_unknown_receipt_error_samples_only_original_wall() {
    let (root, db, _, result) = source_reviewer_execution_fixture_using_calls(
        "valid",
        None,
        3,
        None,
        1,
        |root, db, record, c| {
            source_reviewer_checkpoint_fixture(root, db, record, c, "first_assessment_unknown");
            let before = source_exit_snapshot(db);
            let wall = source_exit_wall(db, &c.root_run_id);
            std::thread::sleep(Duration::from_millis(50));
            let result = run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            );
            assert!(result.is_err());
            assert!(
                source_exit_wall(db, &c.root_run_id) > wall,
                "early reentry error lost known elapsed"
            );
            source_exit_assert_snapshot(db, c, &before);
            assert!(
                crate::agent_runtime::multi_agent::budget::clock::elapsed_fact::read(
                    db,
                    &c.root_run_id
                )
                .unwrap()
                .is_none()
            );
            result
        },
    );
    assert!(result.is_err());
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn source_financial_exit_paid_pending_tool_failure_samples_wall_without_repeating_sdk() {
    let (root, db, _, result) = source_reviewer_execution_fixture_using_calls(
        "valid",
        None,
        3,
        None,
        3,
        |root, db, record, c| {
            source_reviewer_checkpoint_fixture(root, db, record, c, "first_tool_received");
            let paid = source_exit_paid_rows(db, &c.root_run_id);
            let wall = source_exit_wall(db, &c.root_run_id);
            db.execute_batch("CREATE TRIGGER source_exit_delivery_failure BEFORE UPDATE OF state ON agent_source_tool_receipts WHEN NEW.state='completed' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
            std::thread::sleep(Duration::from_millis(50));
            let result = run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            );
            assert!(result.is_err());
            assert!(
                source_exit_wall(db, &c.root_run_id) > wall,
                "ToolPending early return lost known elapsed"
            );
            assert_eq!(source_exit_paid_rows(db, &c.root_run_id), paid);
            assert_eq!(
                db.query_row(
                    "SELECT status FROM agent_runs WHERE id=?1",
                    [&c.root_run_id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
                "running"
            );
            result
        },
    );
    assert!(result.is_err());
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn source_financial_exit_private_writer_failures_keep_primary_error_and_roll_back_every_table() {
    for fault in [
        "BEFORE INSERT ON agent_budget_entries WHEN NEW.dimension='wall_time_ms' BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON agent_budget_entries WHEN NEW.dimension='wall_time_ms' BEGIN UPDATE projects SET status='archived'; END;",
    ] {
        let (root,db,_,result)=source_reviewer_execution_fixture_using_calls("valid",None,3,None,1,|root,db,record,c| {
            source_reviewer_checkpoint_fixture(root,db,record,c,"first_assessment_unknown");
            db.execute_batch(&format!("CREATE TRIGGER source_exit_financial_fault {fault}")).unwrap();
            let before=crate::scanning::web_mode_test_rows(db);
            std::thread::sleep(Duration::from_millis(50));
            let result=run_native_source_assessments(&root.join("oviraptor.sqlite3"),&record.scan_id,1,&root.join("attempt-0001"));
            let error=result.as_ref().unwrap_err();assert!(error.contains("source_financial_exit:"),"financial finally was skipped: {error}");
            assert!(!error.starts_with("source_financial_exit:"),"primary failure must remain");
            crate::scanning::web_mode_assert_rows(db,&before);result
        });
        assert!(result.is_err());drop(db);fs::remove_dir_all(root).unwrap();
    }
}
