// Real original paid model evidence; historical HTTP counters below are
// explicit boundary fixtures, not a claim of 79 actual target requests.
fn target_budget_preflight_paid_fixture() -> BudgetTriggerFixture {
    let f = budget_consumption_actual_fixture(false);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let client = AgentModelClient::new(
        agent_model_profile(&f.h.context.environment, None).unwrap(),
        &[],
    );
    let reply = native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"actual settled original Web bill before HTTP preflight"})],
        &[], 1).unwrap_or_else(|_|panic!("the original Web call must really settle"));
    f.h.context
        .run
        .as_ref()
        .unwrap()
        .model_round(1, &reply.usage, &[])
        .unwrap();
    let mut state = NativeAgentState::fresh(
        f.h.context.attempt_number,
        AgentBackendKind::Native,
        &agent_stable_hash(&f.h.context.evidence),
        &f.h.context.execution_plan.hash(),
        agent_initial_queue(&f.h.context),
    );
    state.token_usage = reply.usage;
    state.turns = 1;
    state
        .persist(&f.h.db_path, &f.h.context.scan_id, &f.h.context.target_url)
        .unwrap();
    f
}

#[test]
fn target_budget_preflight_two_sided_comparison_with_one_persistent_slot_is_pure_and_unpaid() {
    let _real = RealSpecialistTransport::enter();
    let f = target_budget_preflight_paid_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    agent_authorize_tool_on(&db, &f.h.context, "compare_identities").unwrap();
    let raw: String = db
        .query_row(
            "SELECT raw_json FROM sentinel_checkpoints
        WHERE scan_id=?1 AND url=?2 AND stage=?3",
            params![
                f.h.context.scan_id,
                f.h.context.target_url,
                NATIVE_AGENT_STATE_STAGE
            ],
            |r| r.get(0),
        )
        .unwrap();
    let original: JsonValue = serde_json::from_str(&raw).unwrap();
    let mut state = NativeAgentState::from_json(&original).unwrap();
    let ceiling =
        f.h.context
            .execution_plan
            .hard_model_requests
            .max(1)
            .saturating_mul(4)
            .min(400);
    state.target_requests = ceiling - 1;
    let mut normalized = state.as_json();
    normalized["targetRequests"] = original["targetRequests"].clone();
    assert_eq!(
        normalized, original,
        "only the explicit historical request boundary changes"
    );
    state
        .persist(&f.h.db_path, &f.h.context.scan_id, &f.h.context.target_url)
        .unwrap();
    let runtime = AgentToolRuntime {
        target_requests: usize::try_from(ceiling - 1).unwrap(),
        ..Default::default()
    };
    let before = super::tests::application_table_snapshot(&db);
    let calls = f.h.model_seen.lock().unwrap().len();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    let paid = root_tick_count(&db, root, "publication");
    assert_eq!(
        agent_target_budget_preflight(&f.h.context, &runtime, "compare_identities").unwrap_err(),
        "comparison_request_budget_unavailable"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(root_tick_count(&db, root, "publication"), paid);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn target_budget_preflight_revoked_original_capability_is_pure_and_unpaid() {
    let _real = RealSpecialistTransport::enter();
    let f = target_budget_preflight_paid_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let run = &f.h.context.run.as_ref().unwrap().run_id;
    agent_authorize_tool_on(&db, &f.h.context, "replay_http").unwrap();
    assert!(
        db.execute(
            "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime')
        WHERE child_run_id=?1",
            [run]
        )
        .unwrap()
            > 0
    );
    let runtime = AgentToolRuntime::default();
    let before = super::tests::application_table_snapshot(&db);
    let calls = f.h.model_seen.lock().unwrap().len();
    assert_eq!(
        agent_target_budget_preflight(&f.h.context, &runtime, "replay_http").unwrap_err(),
        "tool_capability_or_fencing_denied"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn target_budget_preflight_rejects_unbounded_batch_count_without_writes() {
    let _real = RealSpecialistTransport::enter();
    let f = target_budget_preflight_paid_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let calls = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    for count in [0, -1, 3, i64::MAX] {
        assert_eq!(crate::agent_runtime::multi_agent::budget::supervision_capacity::require_http_headroom_for_count(
            &tx,&session.lease,&session.executor.assignment_id,&session.executor.run_id,count).unwrap_err(),
            "root_supervision_request_count_invalid");
    }
    tx.commit().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}
