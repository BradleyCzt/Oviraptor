// True transport + paid Mapper receipt; derived coordinator output is not input drift.
#[test]
fn coordinator_saved_mapper_reentry_ignores_derived_advice_and_preserves_original_request() {
    use crate::agent_runtime::multi_agent::{budget, specialist};
    let _real = RealSpecialistTransport::enter();
    // Keep the original legacy Native birth: this test makes no HTTP request
    // and must not acquire a newer HTTP-consumption protocol merely to close.
    let mut h = root_v2_slot_harness("stable-mapper-input", 1);
    let db = db::open(&h.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER stable_mapper_wait BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='evidence_summary' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(multi_agent_prepare(&mut h.context).is_err());
    assert_eq!(h.model_seen.lock().unwrap().len(), 2);
    let (assignment, child, original_request): (String, String, String) = db.query_row(
        "SELECT assignment_id,child_run_id,request_json FROM agent_specialist_calls WHERE role='spa_api_mapper' AND state='received'",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    db.execute_batch("DROP TRIGGER stable_mapper_wait;")
        .unwrap();
    // These are derived display/advice projections, never new frontend source facts.
    h.context.evidence["multiAgentCoordinatorHandoff"] =
        json!({"replayed":true,"note":"later derived advice"});
    h.context.evidence["multiAgentExternalSurface"] = json!({"advisoryOnly":true});
    let mut session = multi_agent_prepare(&mut h.context)
        .unwrap_or_else(|error| panic!("same original paid Mapper input must recover: {error}"));
    assert_eq!(session.mapper.assignment_id, assignment);
    assert_eq!(session.mapper.run_id, child);
    assert_eq!(
        h.model_seen.lock().unwrap().len(),
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
    assert_eq!(h.model_seen.lock().unwrap().len(), 3);
    let outcome = AgentTargetOutcome::incomplete("stable original Mapper receipt");
    multi_agent_finish_execution(
        &h.context,
        &mut session,
        &outcome,
    )
    .unwrap();
    // The legacy finish entry renews only its original Coordinator lease's
    // three time fields. Original business rows and all paid calls stay fixed.
    let paid = root_v2_slot_legacy_replay_snapshot(&db, &session.lease);
    let sdk = h.model_seen.lock().unwrap().len();
    for _ in 0..2 {
        multi_agent_finish_execution(&h.context, &mut session, &outcome).unwrap();
        assert_eq!(root_v2_slot_legacy_replay_snapshot(&db, &session.lease), paid);
        assert_eq!(h.model_seen.lock().unwrap().len(), sdk);
    }
    assert!(h.site_seen.lock().unwrap().is_empty());
    let http_rows: (i64, i64) = db.query_row(
        "SELECT (SELECT count(*) FROM agent_http_request_claims),(SELECT count(*) FROM agent_http_budget_receipts)",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(http_rows, (0, 0), "legacy Mapper recovery must remain free of HTTP claims and receipts");
    drop(session);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
