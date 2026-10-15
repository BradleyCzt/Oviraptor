// Frozen in the real workbench before HMAC and Source Root/C/control INSERT.
fn source_born_deadline_checkpoint_fixture(
    schema: i64,
    calls: usize,
    checkpoint: &str,
) -> (
    PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    Result<JsonValue, String>,
) {
    source_reviewer_execution_fixture_configured_before_birth(
        "valid",
        None,
        schema,
        None,
        calls,
        (None, |db, record| {
            record.policy["webModeCeiling"] = json!("quick");
            db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentQuickTimeout',30)", []).unwrap();
        }),
        |root, db, record, actor| {
            if checkpoint == "before_coverage" {
                source_coverage_checkpoint_fixture(root, db, record, actor, checkpoint);
            } else {
                source_reviewer_checkpoint_fixture(root, db, record, actor, checkpoint);
            }
            assert_eq!(
                db.query_row(
                    "SELECT hard_limit FROM agent_budget_limits
                WHERE root_run_id=?1 AND dimension='wall_time_ms'",
                    [&actor.root_run_id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
                30_000
            );
            let before = crate::scanning::web_mode_test_rows(db);
            let deadline = std::time::Instant::now() + Duration::from_secs(40);
            loop {
                match crate::agent_runtime::multi_agent::budget::clock::remaining(
                    db,
                    &actor.root_run_id,
                ) {
                    Err(code) if code == "budget_wall_time_exhausted" => break,
                    Ok(_) => {
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    other => panic!("invalid original deadline: {other:?}"),
                }
            }
            assert!(
                crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(db, actor)
                    .is_err(),
                "the test must not extend the born C beyond its Root clock"
            );
            crate::scanning::web_mode_assert_rows(db, &before);
            let original =
                crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
                    db,
                    &actor.root_run_id,
                )
                .unwrap();
            original.require_original_coordinator(db, actor).unwrap();
            assert!(original.require_executable(db).is_err());
            let financial_before = source_exit_snapshot(db);
            let result = run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            );
            assert!(
                result.is_err(),
                "natural Root/C expiry cannot reenter saved business publication"
            );
            source_exit_assert_snapshot(db, actor, &financial_before);
            source_exit_require_fact(db, actor, true);
            result
        },
    )
}
