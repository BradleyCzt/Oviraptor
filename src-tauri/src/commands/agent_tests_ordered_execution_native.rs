// Both ordered assessments must fit the original pool while the real executor is active.
#[test]
fn ordered_execution_native_loop_dispatches_both_assessments_under_original_capacity() {
    let _real = RealSpecialistTransport::enter();
    let mut h = fresh_multi_production_harness("ordered-native-capacity");
    h.context.execution_plan.max_turns = 2;
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        let system = fresh_multi_wire_system(&request);
        let response = if system.contains("independent deep_investigator specialist")
            || system.contains("independent spa_api_mapper specialist") {
            proposal_model_response(valid_proposal_text())
        } else {
            fresh_directive_closure_wire_response(&request).unwrap_or_else(|| model_round(&[], 10))
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    freeze_fresh_multi_production_harness(&mut h);
    let root_id = h.context.run.as_ref().unwrap().run_id.clone();
    let connection = db::open(&h.db_path).unwrap();
    let native = ordered_exec_native(&connection, &root_id);
    let mut session = multi_agent_prepare(&mut h.context).unwrap();
    let id = confirm_queue_directive(&connection, &session.lease,
        "@investigator 先评估原证据，再由 @mapper 评估建议");
    let outcome = NativeAgentBackend.execute(&h.context);
    let receipts = ordered_exec_receipts(&connection, &id);
    assert_eq!(receipts.len(), 2, "both actual actions must fit alongside the live executor: {outcome:?}");
    assert_eq!(receipts[0]["role"], "deep_investigator");
    assert_eq!(receipts[1]["role"], "spa_api_mapper");
    assert_eq!(receipts[1]["predecessor"]["receiptId"], receipts[0]["receiptId"]);
    assert_ne!(receipts[0]["workerId"], receipts[1]["workerId"]);
    assert_eq!(ordered_exec_native(&connection, &root_id), native);
    let budget_assessment = coordinator_original_budget_assessment_count(
        &connection, &root_id, &seen.lock().unwrap(),
        &["bootstrap", "mapper-output", "budget-allocation", "human-directive", "budget-consumption"],
    );
    assert_eq!(seen.lock().unwrap().len() - budget_assessment, 9,
        "four original paid Root including HumanDirective, one bootstrap Mapper, two actual assessments, two actual executor rounds, excluding the separately proven original Web-cost advisory: {outcome:?}");
    let executor_calls: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'",
        [&session.executor.run_id], |r| r.get(0)).unwrap();
    assert_eq!(executor_calls, 2);
    for receipt in &receipts {
        assert_eq!(receipt["usage"]["totalTokens"], 20);
        assert_eq!(receipt["usage"]["modelRequests"], 1);
        assert_eq!(receipt["targetRequests"], 0);
        assert_eq!(receipt["independentReviewApproved"], false);
    }
    assert_eq!(ordered_exec_projection(&connection, &id)["state"], "completed");
    multi_agent_finish_execution(&h.context, &mut session, &outcome).unwrap();
    coordinator_finished_web_replay_spends_zero(&h.context, &mut session, &outcome,
        || seen.lock().unwrap().len());
    drop(session);
    drop(connection);
    let _ = fs::remove_dir_all(h.root);
}

#[test]
fn ordered_execution_tight_original_pool_defers_unstarted_plan_without_aborting_native_executor() {
    let _real = RealSpecialistTransport::enter();
    let mut h = fresh_multi_production_harness("ordered-native-tight");
    h.context.execution_plan.max_turns = 2;
    h.context.execution_plan.hard_model_requests = 6;
    h.context.execution_plan.soft_model_requests = 6;
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (200, "application/json", fresh_directive_closure_wire_response(&request)
            .unwrap_or_else(|| model_round(&[], 10)))
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    // Six-request legacy regression retains its original v5 birth contract.
    freeze_fresh_multi_production_harness_original_v5(&mut h);
    let connection = db::open(&h.db_path).unwrap();
    let root_id = h.context.run.as_ref().unwrap().run_id.clone();
    let native = ordered_exec_native(&connection, &root_id);
    let mut session = multi_agent_prepare(&mut h.context).unwrap();
    let id = confirm_queue_directive(&connection, &session.lease,
        "@investigator 先评估原证据，再由 @mapper 评估建议");
    let outcome = NativeAgentBackend.execute(&h.context);
    let (status, reason): (String, String) = connection.query_row(
        "SELECT status,rejection_code FROM agent_user_directives WHERE id=?1", [&id],
        |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((status.as_str(), reason.as_str()), ("deferred", "root_human_assessment_budget_unavailable"), "{outcome:?}");
    assert_eq!(ordered_exec_count(&connection, "agent_directive_ordered_actions"), 0);
    assert_eq!(ordered_exec_count(&connection, "agent_directive_ordered_receipts"), 0);
    assert_eq!(ordered_exec_native(&connection, &root_id), native);
    assert_eq!(seen.lock().unwrap().len(), 5, "three original paid Root/Mapper and its one granted executor request continue: {outcome:?}");
    let calls: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'",
        [&session.executor.run_id], |r| r.get(0)).unwrap();
    assert_eq!(calls, 1);
    assert_eq!(ordered_exec_projection(&connection, &id)["actions"][0]["state"], "not_started");
    multi_agent_finish_execution(&h.context, &mut session, &outcome).unwrap();
    drop(session);
    drop(connection);
    let _ = fs::remove_dir_all(h.root);
}
