// Full production preparation with actual paid Root/Mapper SDK, temporary creator data.
#[test]
fn executor_allocation_actual_three_sdk_preparation_preserves_original_reviewer_floor() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        if request.contains("You are the Root Coordinator") && request.contains("budget-allocation") {
            return (200,"application/json",fresh_multi_root_wire_response(&request).unwrap());
        }
        if !request.contains("You are the Root Coordinator") && !request.contains("SPA/API Mapper") {
            return (200,"application/json",model_round(&[],10));
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture_original_v5_limits(
        "executor-reviewer-floor",
        &format!("http://127.0.0.1:{port}/v1"),
        (60_000, 6),
    );
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "actual initial Root, Mapper, changed-fact Root; no target access"
    );
    let db = db::open(&f.context.db_path).unwrap();
    let remaining = crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
        &db,
        &session.lease.root_run_id,
    )
    .unwrap();
    assert_eq!(
        remaining,
        (Some(31_000), Some(2)),
        "Executor must preserve original Reviewer floor even with no spare proposal request"
    );
    let grant: (i64, i64) = db
        .query_row(
            "SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1",
            [&session.executor.assignment_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(grant, (28_940, 1));
    let window = f.context.run_budget.as_ref().unwrap();
    assert_eq!(
        (window.hard_tokens, window.hard_requests),
        grant,
        "execution window follows actually admitted budget"
    );
    let outcome = run_native_agent(&f.context);
    assert!(matches!(outcome,AgentTargetOutcome::Limited(_) | AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{}",outcome.detail());
    multi_agent_finish_execution(
        &f.context,
        &mut session,
        &outcome,
    )
    .unwrap();
    drop(session);
}

fn executor_allocation_closed_mapper_fixture(
    limits: (i64, i64),
) -> (
    RootTickFixture,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    NativeCoordinatorFrame,
    NativeCoordinatorTickReceipt,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (200,"application/json",changed_fact_response(&request,true))
    }));
    let f = root_tick_fixture_original_v5_limits("executor-original-v5",&format!("http://127.0.0.1:{port}/v1"),limits);
    let bootstrap = native_coordinator_tick(&f.context,&f.actor).unwrap();
    let mapper = native_coordinator_prepare_mapper(
        &f.context,
        &f.actor,
        &bootstrap,
        &bootstrap_mapper_task(&f),
    )
    .unwrap();
    let (text, usage) = multi_agent_child_round(
        &f.context,
        &f.actor,
        &mapper,
        "Independent Mapper",
        json!({"frozenEvidence":f.context.evidence}),
    )
    .unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    deliver_readonly_assessment(
        &db,
        &f.actor,
        &mapper,
        &usage,
        &json!({"summary":text}),
        None,
    )
    .unwrap();
    let (frame, decision) =
        native_coordinator_mapper_decision(&f.context, &f.actor, &mapper).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 3);
    (f, mapper, frame, decision, seen)
}
fn executor_allocation_admit(
    f: &RootTickFixture,
    mapper: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    frame: &NativeCoordinatorFrame,
    decision: &NativeCoordinatorTickReceipt,
) -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    use crate::agent_runtime::contract::{AgentLane, AgentRole};
    let caps = agent_tool_specs_for(&f.context)
        .iter()
        .map(|s| s.name.to_string())
        .collect::<Vec<_>>();
    native_coordinator_schedule_decision(
        &f.context,
        &f.actor,
        frame,
        decision,
        AgentRole::WebExecutor,
        AgentLane::TargetTouching,
        "mapper_handoff_ready",
        &json!({"target":f.context.target_url,"objective":"current paid Mapper follow-up","mapperAssignmentId":mapper.assignment_id}),
        1,
        &caps,
        1_000_000,
        1_000,
    )
}
#[test]
fn executor_allocation_actual_paid_frame_ignores_stale_hints_and_replays_original_grant_zero_write()
{
    let _real = RealSpecialistTransport::enter();
    let (f, mapper, frame, decision, seen) = executor_allocation_closed_mapper_fixture((60_000, 6));
    let child = executor_allocation_admit(&f, &mapper, &frame, &decision).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::model::original_web_grant(
            &db,
            &f.actor,
            &child.assignment_id
        )
        .unwrap(),
        (28_940, 1)
    );
    let before = web_mode_test_rows(&db);
    assert_eq!(
        executor_allocation_admit(&f, &mapper, &frame, &decision).unwrap(),
        child
    );
    web_mode_assert_rows(&db, &before);
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "same paid frame/grant needs no new SDK or permission"
    );
}
#[test]
fn executor_allocation_original_unlimited_dimensions_remain_unbounded_without_fake_execution() {
    let _real = RealSpecialistTransport::enter();
    for limits in [(0, 6), (60_000, 0), (0, 0)] {
        let (f, mapper, frame, decision, seen) = executor_allocation_closed_mapper_fixture(limits);
        let db = db::open(&f.context.db_path).unwrap();
        let before = web_mode_test_rows(&db);
        let expected = match limits {
            (0, 6) => (0, 1, true),
            (60_000, 0) => (20_940, 0, true),
            (0, 0) => (0, 0, true),
            _ => unreachable!(),
        };
        assert_eq!(
            native_coordinator_executor_allocation(&db, &f.actor, "mapper_handoff_ready", 1)
                .unwrap(),
            Some(expected)
        );
        web_mode_assert_rows(&db, &before);
        let child = executor_allocation_admit(&f, &mapper, &frame, &decision).unwrap();
        assert_eq!(crate::agent_runtime::multi_agent::budget::model::original_web_grant(&db,&f.actor,&child.assignment_id).unwrap(),(expected.0,expected.1));
        let before=web_mode_test_rows(&db);
        assert_eq!(executor_allocation_admit(&f,&mapper,&frame,&decision).unwrap(),child);
        web_mode_assert_rows(&db,&before);
        assert_eq!(seen.lock().unwrap().len(), 3);
    }
}

#[test]
fn executor_allocation_full_mixed_unlimited_preparation_binds_actual_runtime_windows() {
    let _real = RealSpecialistTransport::enter();
    for limits in [(0, 6), (60_000, 0)] {
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
            if request.contains("You are the Root Coordinator") && request.contains("budget-allocation") {
                return (200,"application/json",fresh_multi_root_wire_response(&request).unwrap());
            }
            if !request.contains("You are the Root Coordinator") && !request.contains("SPA/API Mapper") {
                return (200,"application/json",model_round(&[],10));
            }
            (
                200,
                "application/json",
                changed_fact_response(&request, true),
            )
        }));
        let mut f = root_tick_fixture_original_v5_limits(
            "executor-mixed-unlimited",
            &format!("http://127.0.0.1:{port}/v1"),
            limits,
        );
        drop(f.parent.take());
        let mut session = multi_agent_prepare(&mut f.context).unwrap();
        let db = db::open(&f.context.db_path).unwrap();
        let expected = if limits.0 == 0 { (0, 1) } else { (20_940, 0) };
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::model::original_web_grant(
                &db,
                &session.lease,
                &session.executor.assignment_id
            )
            .unwrap(),
            expected
        );
        let window = f.context.run_budget.as_ref().unwrap();
        assert_eq!((window.hard_tokens, window.hard_requests), expected);
        assert_eq!(seen.lock().unwrap().len(), 3);
        let outcome = run_native_agent(&f.context);
        assert!(matches!(outcome,AgentTargetOutcome::Limited(_) | AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{}",outcome.detail());
        multi_agent_finish_execution(
            &f.context,
            &mut session,
            &outcome,
        )
        .unwrap();
        drop(session);
    }
}
