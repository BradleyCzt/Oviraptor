fn root_v2_slot_declaration(
    context: &AgentRunContext,
    batches: i64,
) -> crate::agent_runtime::multi_agent::budget::root_definition::NewRootBudgetDeclaration {
    use crate::agent_runtime::multi_agent::budget::root_definition::NewRootBudgetDeclaration;
    let plan = &context.execution_plan;
    let finite = |n| (n > 0).then_some(n);
    NewRootBudgetDeclaration {
        schema_version: 2,
        declaration_id: Uuid::new_v4().to_string(),
        scan_id: context.scan_id.clone(),
        attempt_number: context.attempt_number,
        target_url: context.target_url.clone(),
        plan_hash: plan.hash(),
        execution_slot_capacity: 3,
        limits: [
            finite(plan.hard_total_tokens),
            finite(plan.hard_total_tokens),
            finite(plan.hard_total_tokens),
            finite(plan.hard_model_requests),
            Some(plan.hard_model_requests.max(1).saturating_mul(4).min(400)),
            Some(0),
            Some(0),
            Some(0),
            Some(batches),
            Some(i64::try_from(plan.timeout_seconds).unwrap() * 1000),
        ],
    }
}

include!("agent_tests_root_v2_slot_fresh_fixture.rs");
include!("agent_tests_root_v2_slot_http_legacy.rs");

fn root_v2_slot_harness(tag: &str, batches: i64) -> AgentHarness {
    let mut h = root_v2_slot_fresh_harness(tag, batches);
    root_v2_slot_retarget_model(&mut h, vec![proposal_model_response(
        "{\"summary\":\"frozen local evidence\",\"priorityContracts\":[],\"risks\":[]}")]);
    h
}

#[test]
fn root_v2_slot_real_native_prepare_does_not_steal_batch_one() {
    use crate::agent_runtime::multi_agent::budget;
    let mut h = root_v2_slot_harness("new-root-slot-real", 1);
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    let db = db::open(&h.db_path).unwrap();
    let plan_before: String = db
        .query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&root],
            |r| r.get(0),
        )
        .unwrap();
    let _real = RealSpecialistTransport::enter();
    let mut session = multi_agent_prepare(&mut h.context).unwrap();
    assert_eq!(
        h.model_seen.lock().unwrap().len(),
        3,
        "must use actual SDK/received receipt"
    );
    assert_eq!(h.model_seen.lock().unwrap().iter()
        .filter(|wire| wire.contains("You are the Root Coordinator")).count(), 2,
        "the new SDK request is an actual Root model call");
    assert_eq!(budget::balance(&db, &root, Some(""), "model_requests").unwrap().consumed, 2,
        "actual Root cost uses its original financial owner");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='publication'",
        [&root], |r|r.get::<_,i64>(0)).unwrap(), 2);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1 AND state='received'",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&session.executor.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "real executor must hold its TargetTouching execution slot"
    );
    assert_eq!(
        budget::balance(&db, &root, None, "concurrency_batches").unwrap(),
        budget::Balance::default(),
        "a child execution slot must not spend or reserve actual batch 1"
    );
    assert_eq!(db.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='concurrency_batches'",
        [&root],|r|r.get::<_,i64>(0)).unwrap(),1,"only explicit new-Root batch ceiling applies");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        10
    );
    assert_eq!(
        db.query_row(
            "SELECT plan_json FROM agent_runs WHERE id=?1",
            [&root],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        plan_before,
        "Native JSON bytes stay untouched"
    );
    multi_agent_finish_execution(
        &h.context,
        &mut session,
        &AgentTargetOutcome::incomplete("slot proof"),
    )
    .unwrap();
    assert_eq!(
        budget::balance(&db, &root, None, "concurrency_batches").unwrap(),
        budget::Balance::default()
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&session.executor.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(session);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn root_v2_slot_zero_batch_budget_still_allows_real_readonly_and_executor_slots() {
    use crate::agent_runtime::multi_agent::budget;
    let mut h = root_v2_slot_harness("new-root-zero-batches", 0);
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    let _real = RealSpecialistTransport::enter();
    let mut session = multi_agent_prepare(&mut h.context).unwrap();
    assert_eq!(h.model_seen.lock().unwrap().len(), 3);
    let db = db::open(&h.db_path).unwrap();
    assert_eq!(db.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='concurrency_batches'",
        [&root],|r|r.get::<_,i64>(0)).unwrap(),0,"zero actual race batches grants no racing");
    assert_eq!(
        budget::balance(&db, &root, None, "concurrency_batches").unwrap(),
        budget::Balance::default()
    );
    let before = super::tests::application_table_snapshot(&db);
    let caps = agent_tool_specs_for(&h.context)
        .iter()
        .map(|s| s.name.to_string())
        .collect::<Vec<_>>();
    let rejected = crate::agent_runtime::multi_agent::scheduler::schedule_child(
        &db,
        &session.lease,
        crate::agent_runtime::contract::AgentRole::WebExecutor,
        crate::agent_runtime::contract::AgentLane::TargetTouching,
        "other_target_touch",
        &serde_json::json!({"target":h.context.target_url,
            "objective":"must never receive a second TargetTouching slot",
            "mapperAssignmentId":session.mapper.assignment_id}),
        2,
        &caps,
        1000,
        1,
    );
    assert!(
        rejected.is_err(),
        "a separate batch ledger does not expand lane capacity"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(h.model_seen.lock().unwrap().len(), 3);
    multi_agent_finish_execution(
        &h.context,
        &mut session,
        &AgentTargetOutcome::incomplete("slot proof"),
    )
    .unwrap();
    drop(session);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn root_v2_slot_existing_native_root_cannot_gain_new_declaration() {
    let mut h = agent_harness(
        "old-root-no-v2-grant",
        mock_site,
        vec![AgentIdentity::anonymous()],
    );
    freeze_harness_plan(&h);
    h.context.run = runtime_open_run(&h.db_path, &h.context.scan_id, &h.context.route);
    let declaration = root_v2_slot_declaration(&h.context, 1);
    let db = db::open(&h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        persist_agent_execution_plan_with_budget(
            &h.db_path,
            &h.context.scan_id,
            h.context.attempt_number,
            &h.context.target_url,
            &h.context.execution_plan,
            Some(&declaration)
        )
        .unwrap_err(),
        "budget_declaration_requires_new_root"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(h.model_seen.lock().unwrap().len(), 0);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn root_v2_slot_declaration_collateral_write_rolls_back_root_and_projection() {
    let h = agent_harness(
        "new-root-decl-trigger",
        mock_site,
        vec![AgentIdentity::anonymous()],
    );
    seed_attempt_row(&h.db_path, h.context.attempt_number, "initial");
    let declaration = root_v2_slot_declaration(&h.context, 1);
    let db = db::open(&h.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER declaration_collateral AFTER INSERT ON agent_root_budget_definitions BEGIN INSERT INTO projects(name) VALUES('not-approved'); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(persist_agent_execution_plan_with_budget(
        &h.db_path,
        &h.context.scan_id,
        h.context.attempt_number,
        &h.context.target_url,
        &h.context.execution_plan,
        Some(&declaration)
    )
    .is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(h.model_seen.lock().unwrap().len(), 0);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn root_v2_slot_undeclared_native_root_prepare_rejects_without_any_grant() {
    let mut h = agent_harness("old-root-no-mode-or-v2-grant", mock_site,
        vec![AgentIdentity::anonymous()]);
    freeze_harness_plan(&h);
    h.context.run = runtime_open_run(&h.db_path, &h.context.scan_id, &h.context.route);
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    retarget_model(&mut h, vec![proposal_model_response("{\"summary\":\"historical\"}")]);
    let db = db::open(&h.db_path).unwrap();
    let original = super::tests::application_table_snapshot(&db);
    let caller_plan = h.context.execution_plan.as_json();
    let _real = RealSpecialistTransport::enter();
    let error = multi_agent_prepare(&mut h.context).err().expect("legacy Root must not gain mode or financial authority");
    assert_eq!(error, "web_mode_root_declaration_missing");
    assert!(super::tests::application_table_snapshot(&db) == original);
    assert_eq!(h.context.execution_plan.as_json(), caller_plan);
    assert_eq!(h.model_seen.lock().unwrap().len(), 0);
    for table in ["agent_root_mode_definitions", "agent_root_budget_definitions",
        "agent_root_budget_attempts", "agent_budget_limits", "agent_budget_clock_origins",
        "agent_budget_ledger", "agent_assignments", "agent_capability_leases",
        "agent_assignment_attempts", "agent_lane_leases", "agent_coordinator_leases"] {
        assert_eq!(db.query_row(&format!("SELECT count(*) FROM {table}"), [],
            |r|r.get::<_,i64>(0)).unwrap(), 0, "historical prepare filled {table}");
    }
    assert_eq!(db.query_row("SELECT count(*) FROM agent_runs WHERE id=?1", [&root],
        |r|r.get::<_,i64>(0)).unwrap(), 1);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
