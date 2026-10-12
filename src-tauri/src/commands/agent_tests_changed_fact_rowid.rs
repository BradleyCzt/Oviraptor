// Replacing a physical closed row cannot reuse its original fact proof.
#[test]
fn coordinator_changed_fact_physical_rowid_replacement_denies_replay_without_sdk() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        if specialist_original_is_web_sdk(&request) {
            return (200,"application/json",proposal_model_response("actual original Web exit; no target tool"));
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture_from_web_mode_turns("changed-fact-rowid", &format!("http://127.0.0.1:{port}/v1"),
        true,(60_000,20),None,web_mode_fixture("multi","https://authorized.example.test"),Some(1));
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    let root = native_coordinator_root_context(&f.context, &session.lease);
    let db = db::open(&root.db_path).unwrap();
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let frame = NativeCoordinatorFrame::mapper(&tx, &session.lease, &session.mapper).unwrap();
    tx.commit().unwrap();
    let old: i64 = db
        .query_row(
            "SELECT rowid FROM agent_runs WHERE id=?1",
            [&session.mapper.run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        db.execute(
            "UPDATE agent_runs SET rowid=?1 WHERE id=?2",
            params![old + 1000000, session.mapper.run_id]
        )
        .unwrap(),
        1
    );
    let before = web_mode_test_rows(&db);
    let result = native_coordinator_tick_for_frame(&root, &session.lease, &frame);
    assert!(
        matches!(&result, Err(code) if code=="root_frame_original_fact_changed"),
        "changed physical row must deny original replay: {:?}",
        result.err()
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &session.lease.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        2
    );
    assert_eq!(
        db.execute(
            "UPDATE agent_runs SET rowid=?1 WHERE id=?2",
            params![old, session.mapper.run_id]
        )
        .unwrap(),
        1
    );
    let replay = native_coordinator_tick_for_frame(&root, &session.lease, &frame).unwrap();
    assert!(replay.replayed);
    assert_eq!(seen.lock().unwrap().len(), 3);
    // Only after exact physical restoration and zero-SDK original replay,
    // execute and settle the genuinely issued original Web request.
    let outcome=run_native_agent(&f.context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    multi_agent_finish_execution(&f.context,&mut session,&outcome).unwrap();
    assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
    let usage:(i64,i64,i64)=db.query_row("SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(usage,(20,0,1));
    drop(session);
}
