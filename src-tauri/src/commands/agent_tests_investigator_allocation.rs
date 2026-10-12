// Original v5 allocation boundaries keep their born contract and exact limits.
// Investigator must not consume Reviewer/Root floors after Executor took surplus.
#[test]
fn investigator_allocation_denies_when_only_retained_floor_remains_after_prepare() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture_original_v5_limits(
        "investigator-floor-deny",
        &format!("http://127.0.0.1:{port}/v1"),
        (60_000, 6),
    );
    drop(f.parent.take());
    let session = multi_agent_prepare(&mut f.context).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 3);
    let db = db::open(&f.context.db_path).unwrap();
    let remaining = crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
        &db,
        &session.lease.root_run_id,
    )
    .unwrap();
    assert_eq!(
        remaining,
        (Some(31_000), Some(2)),
        "prepare leaves only Reviewer+Root retained floor"
    );
    let err = native_coordinator_investigator_allocation(
        &db,
        &session.lease,
        "reviewer_insufficient_evidence",
        1,
    )
    .unwrap_err();
    assert_eq!(err, "multi_agent_investigator_budget_unavailable");
    drop(session);
}

#[test]
fn investigator_allocation_admits_fixed_proposal_when_surplus_exceeds_floors() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        if specialist_original_is_web_sdk(&request) {
            return (200, "application/json", model_round(&[], 10));
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture_from_web_mode_evidence_protocol(
        "investigator-floor-admit",
        &format!("http://127.0.0.1:{port}/v1"),
        true, (80_000, 8), None,
        web_mode_fixture("multi", "https://authorized.example.test"),
        RootFixtureEvidence { max_turns: Some(1), original_v5: true, ..Default::default() },
    );
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 3);
    let outcome = run_native_agent(&f.context);
    assert!(matches!(outcome, AgentTargetOutcome::Incomplete(_)), "{outcome:?}");
    assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(), 1);
    multi_agent_finish_execution(&f.context, &mut session, &outcome).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let remaining = crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
        &db,
        &session.lease.root_run_id,
    )
    .unwrap();
    let (token_floor, request_floor) =
        native_coordinator_executor_retained_floor(&db, &session.lease).unwrap();
    assert!(
        remaining
            .0
            .is_some_and(|n| n >= token_floor + crate::agent_runtime::multi_agent::directive::proposals::PROPOSAL_TOKENS)
            && remaining.1.is_some_and(|n| n > request_floor),
        "finished executor must leave proposal headroom above floors: remaining={remaining:?} floor=({token_floor},{request_floor})"
    );
    assert_eq!(
        native_coordinator_investigator_allocation(
            &db,
            &session.lease,
            "reviewer_insufficient_evidence",
            1,
        )
        .unwrap(),
        Some((
            crate::agent_runtime::multi_agent::directive::proposals::PROPOSAL_TOKENS,
            1,
            true
        ))
    );
    drop(session);
}
