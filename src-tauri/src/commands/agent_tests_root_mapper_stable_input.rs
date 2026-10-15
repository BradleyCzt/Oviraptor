// True transport + paid Mapper receipt; derived coordinator output is not input drift.
#[test]
fn coordinator_saved_mapper_reentry_ignores_derived_advice_and_preserves_original_request() {
    use crate::agent_runtime::multi_agent::{budget, specialist};
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        let response = if request.contains("You are the Root Coordinator")
            || request.contains("SPA/API Mapper")
        {
            changed_fact_response(&request, true)
        } else {
            proposal_model_response("actual bounded Web exit; no target tool")
        };
        (200, "application/json", response)
    }));
    let mut f = root_tick_fixture_from_web_mode_turns(
        "stable-mapper-input",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        None,
        web_mode_fixture("multi", "https://authorized.example.test"),
        Some(1),
    );
    drop(f.parent.take());
    let db = db::open(&f.context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER stable_mapper_wait BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='evidence_summary' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(multi_agent_prepare(&mut f.context).is_err());
    assert_eq!(seen.lock().unwrap().len(), 2);
    let (assignment, child, original_request): (String, String, String) = db.query_row(
        "SELECT assignment_id,child_run_id,request_json FROM agent_specialist_calls WHERE role='spa_api_mapper' AND state='received'",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    db.execute_batch("DROP TRIGGER stable_mapper_wait;")
        .unwrap();
    // These are derived display/advice projections, never new frontend source facts.
    f.context.evidence["multiAgentCoordinatorHandoff"] =
        json!({"replayed":true,"note":"later derived advice"});
    f.context.evidence["multiAgentExternalSurface"] = json!({"advisoryOnly":true});
    let mut session = multi_agent_prepare(&mut f.context)
        .unwrap_or_else(|error| panic!("same original paid Mapper input must recover: {error}"));
    assert_eq!(session.mapper.assignment_id, assignment);
    assert_eq!(session.mapper.run_id, child);
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "no second Mapper SDK; one actual changed-fact Root decision"
    );
    let saved: String = db
        .query_row(
            "SELECT request_json FROM agent_specialist_calls WHERE child_run_id=?1",
            [&child],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        saved, original_request,
        "original paid request bytes are never rewritten"
    );
    assert_eq!(
        budget::balance(&db, &session.lease.root_run_id, Some(""), "model_requests")
            .unwrap()
            .consumed,
        2
    );
    assert_eq!(
        budget::balance(
            &db,
            &session.lease.root_run_id,
            Some(&assignment),
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    let mut changed: JsonValue = serde_json::from_str(&original_request).unwrap();
    changed["unpaidInput"] = true.into();
    let before = web_mode_test_rows(&db);
    assert!(
        specialist::start_authorized(&db, &session.lease, &session.mapper, &changed, |_| Ok(()))
            .is_err(),
        "a genuinely different request cannot adopt the received invoice"
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(seen.lock().unwrap().len(), 3);
    let outcome = run_native_agent(&f.context);
    assert!(
        matches!(outcome, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),
        "{}",
        outcome.detail()
    );
    multi_agent_finish_execution(&f.context, &mut session, &outcome).unwrap();
    assert_eq!(
        seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),
        1,
        "one actual original Web SDK exit"
    );
    let original_web: (i64, i64, i64) = db.query_row(
        "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(original_web, (20, 0, 1));
    coordinator_finished_web_replay_spends_zero(&f.context, &mut session, &outcome, || seen.lock().unwrap().len());
    drop(session);
    drop(db);
}
