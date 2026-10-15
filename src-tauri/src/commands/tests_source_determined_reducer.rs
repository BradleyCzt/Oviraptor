#[test]
fn source_determined_reducer_actual_independent_gap_free_coverage_completes_once() {
    let (root, db, actor, result) = source_reviewer_execution_fixture_using_calls(
        "no_candidates_separate_git",
        None,
        4,
        None,
        7,
        |root, _, record, _| {
            run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            )
        },
    );
    let report = result.unwrap();
    assert_eq!(
        report["sourceCoverageDecision"]["decision"]["coverageSufficient"],
        true
    );
    assert_eq!(
        report["sourceCoverageDecision"]["decision"]["outstandingGaps"],
        json!([])
    );
    assert_eq!(report["independentCandidateReviewCompleted"], false);
    assert_eq!(report["gate"]["status"], "passed");
    let terminal: (String, String, String, i64) = db
        .query_row(
            "SELECT status,terminal_state,terminal_code,(SELECT count(*) FROM agent_events
            WHERE run_id=?1 AND event_type='terminal_reduced') FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(terminal, ("terminal".into(), "completed".into(), AGENT_STOP_FINISH.into(), 1),
        "all original obligations known and coverage independently sufficient must use the shared reducer");
    let paid = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap()
    .consumed;
    assert_eq!(
        paid, 7,
        "seven actual independent physical provider receipts, no Root SDK added"
    );
    let before = crate::scanning::web_mode_test_rows(&db);
    assert!(run_native_source_assessments(
        &root.join("oviraptor.sqlite3"),
        &actor.scan_id,
        1,
        &root.join("attempt-0001")
    )
    .is_err());
    crate::scanning::web_mode_assert_rows(&db, &before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn source_determined_reducer_actual_outstanding_gap_stays_bounded() {
    let (root, db, actor, result) = source_coverage_execution_fixture("valid", None);
    let report = result.unwrap();
    assert_eq!(
        report["sourceCoverageDecision"]["decision"]["coverageSufficient"],
        false
    );
    assert!(
        !report["sourceCoverageDecision"]["decision"]["outstandingGaps"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let state: (String, String) = db
        .query_row(
            "SELECT terminal_state,terminal_code FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        ("completed_with_gaps".into(), AGENT_STOP_DERIVED.into())
    );
    assert_eq!(report["modelRequests"], 8);
    assert_ne!(report["gate"]["status"], "passed");
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn source_determined_reducer_final_completed_write_ignore_cannot_claim_success() {
    let fault = "CREATE TRIGGER source_reducer_ignore BEFORE UPDATE ON agent_runs
        WHEN NEW.role='coordinator' AND NEW.terminal_state='completed' AND OLD.status<>'terminal'
        BEGIN SELECT RAISE(IGNORE); END;";
    let (root, db, actor, result) = source_reviewer_execution_fixture_using_calls(
        "no_candidates_separate_git",
        Some(fault),
        4,
        None,
        7,
        |root, _, record, _| {
            run_native_source_assessments(
                &root.join("oviraptor.sqlite3"),
                &record.scan_id,
                1,
                &root.join("attempt-0001"),
            )
        },
    );
    assert!(
        result.is_err(),
        "ignored completed publication must not escape as a successful report"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE id=?1 AND terminal_state='completed'",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_events WHERE run_id=?1 AND event_type='terminal_reduced'",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        7,
        "normal paid child entries survive failed local publication"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
