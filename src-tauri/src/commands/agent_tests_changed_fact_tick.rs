fn changed_fact_response(request: &str, allow: bool) -> String {
    if request.contains("You are the Root Coordinator") {
        let body: JsonValue = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        let suggestion = if input["basis"]["phase"] == "http-budget-consumption" {
            "assess:http_budget_consumption"
        } else if input["basis"]["phase"] == "budget-consumption" {
            "assess:budget_consumption"
        } else if request.contains("budget-allocation") {
            "assess:budget_allocation"
        } else if request.contains("identity-session-output") {
            "assess:identity_session_metadata"
        } else if request.contains("mapper-output") {
            "dispatch:web_executor"
        } else if request.contains("reviewer-decision") {
            "dispatch:deep_investigator"
        } else if request.contains("bootstrapDispatch") {
            "dispatch:spa_api_mapper"
        } else {
            "bootstrap is advisory"
        };
        proposal_model_response(
            &json!({"schemaVersion":1,"observed":["actual original frozen fact"],"missing":[],
            "suggestions":if allow || suggestion=="dispatch:spa_api_mapper" {vec![suggestion]}else {vec![]},"costNotes":[],"risks":[]})
            .to_string(),
        )
    } else if request.contains("IdentitySession") {
        proposal_model_response(&json!({"summary":"actual paid identity metadata only",
            "observedAuthentication":["one validated handle"],"evidenceGaps":["no target identity comparison"],
            "authorizationProven":false}).to_string())
    } else {
        proposal_model_response(
            r#"{"summary":"actual paid Mapper envelope","priorityContracts":[],"risks":[]}"#,
        )
    }
}

#[test]
fn coordinator_changed_mapper_fact_actual_sdk_and_same_fact_zero_sdk_replay() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            if request.contains("You are the Root Coordinator") || request.contains("SPA/API Mapper") {
                changed_fact_response(&request, true)
            } else { model_round(&[], 10) },
        )
    }));
    let fixture = web_mode_fixture("multi", "https://authorized.example.test");
    let mut f = root_tick_fixture_from_web_mode_turns(
        "changed-fact-mapper", &format!("http://127.0.0.1:{port}/v1"),
        true, (60_000,20), None, fixture, Some(1),
    );
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "actual bootstrap Root, independent Mapper, changed-fact Root SDK"
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 2);
    let root = native_coordinator_root_context(&f.context, &session.lease);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let frame = NativeCoordinatorFrame::mapper(&tx, &session.lease, &session.mapper).unwrap();
    tx.commit().unwrap();
    let replay = native_coordinator_tick_for_frame(&root, &session.lease, &frame).unwrap();
    assert!(replay.replayed);
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "no new fact means no negotiation/provider call"
    );
    let task: String = db
        .query_row(
            "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
            [&session.executor.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    let task: JsonValue = serde_json::from_str(&task).unwrap();
    assert_eq!(task["rootDecision"]["eventSequence"], replay.event_sequence);
    assert_eq!(
        task["rootDecision"]["frameHash"],
        crate::agent_runtime::store::stable_hash(&frame.fact().to_string())
    );
    assert_eq!(task["rootDecision"]["step"], "dispatch:web_executor");
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        2
    );
    let outcome = run_native_agent(&f.context);
    assert!(matches!(outcome, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{}", outcome.detail());
    multi_agent_finish_execution(&f.context, &mut session, &outcome).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 5, "original budget feedback and one actual empty Web round exit naturally");
    drop(session);
}

#[test]
fn coordinator_changed_mapper_fact_paid_defer_does_not_dispatch_web_executor() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, false),
        )
    }));
    let mut f = root_tick_fixture("changed-fact-defer", &format!("http://127.0.0.1:{port}/v1"));
    drop(f.parent.take());
    let error = multi_agent_prepare(&mut f.context)
        .err()
        .expect("Root did not select target execution");
    assert_eq!(error, "root_decision_did_not_select_step");
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(
        root_tick_count(&db, &f.actor.root_run_id, "publication"),
        2,
        "actual paid Root semantics remain visible"
    );
    let web: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE role='web_executor'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(web, 0);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        2
    );
}

#[test]
fn coordinator_dispatch_last_write_cannot_change_original_closed_mapper() {
    let _real = RealSpecialistTransport::enter();
    let MapperBoundaryEndpoint {port,requests:seen,database:fault_db}=coordinator_mapper_boundary_endpoint("CREATE TRIGGER corrupt_original_mapper AFTER INSERT ON agent_assignments WHEN NEW.role='web_executor'
        BEGIN UPDATE agent_runs SET finished_at='' WHERE role='spa_api_mapper'; END;");
    let mut f = root_tick_fixture(
        "changed-fact-dispatch-trigger",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    *fault_db.lock().unwrap()=Some(f.context.db_path.clone());
    drop(f.parent.take());
    let db = db::open(&f.context.db_path).unwrap();

    assert!(
        multi_agent_prepare(&mut f.context).is_err(),
        "scheduler writes must recheck the same captured frame"
    );
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(
        root_tick_count(&db, &f.actor.root_run_id, "publication"),
        2,
        "paid Root publication precedes the failing grant transaction"
    );
    let row: (i64, i64, i64) = db
        .query_row(
            "SELECT (SELECT COUNT(*) FROM agent_assignments WHERE role='web_executor'),
        (SELECT COUNT(*) FROM agent_runs WHERE role='spa_api_mapper' AND finished_at<>''),
        (SELECT reserved_requests FROM agent_budget_ledger)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (0, 1, 0),
        "roll back child, lane/capabilities/grants and the collateral original fact edit"
    );
}

#[test]
fn coordinator_changed_fact_cannot_replace_original_provider_credential_before_new_fee() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        if specialist_original_is_web_sdk(&request) {
            return (200, "application/json", proposal_model_response("actual bounded Web exit; no target tool"));
        }
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let mut f = root_tick_fixture_from_web_mode_turns(
        "changed-fact-model-owner",
        &format!("http://127.0.0.1:{port}/v1"),
        true, (60_000, 20), None,
        web_mode_fixture("multi", "https://authorized.example.test"), Some(1),
    );
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let mut root = native_coordinator_root_context(&f.context, &session.lease);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let frame = NativeCoordinatorFrame::mapper(&tx, &session.lease, &session.mapper).unwrap();
    tx.commit().unwrap();
    root.environment.api_key = "different-isolated-test-credential".into();
    let before = web_mode_test_rows(&db);
    let error = native_coordinator_tick_for_frame(&root, &session.lease, &frame)
        .err()
        .expect("model owner cannot change");
    assert_eq!(error, "root_tick_original_model_changed");
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "no provider/credential adoption or new invoice"
    );
    web_mode_assert_rows(&db, &before);
    let original_web = run_native_agent(&f.context);
    assert!(matches!(original_web, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{original_web:?}");
    multi_agent_finish_execution(&f.context, &mut session, &original_web).unwrap();
    assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(), 1);
    let usage: (i64, i64, i64) = db.query_row(
        "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap();
    assert_eq!(usage, (20, 0, 1), "only the unchanged original context can pay and exit");
    drop(session);
}

#[test]
fn coordinator_changed_fact_dispatch_rejects_collateral_business_trigger() {
    let _real = RealSpecialistTransport::enter();
    let MapperBoundaryEndpoint {port,requests:seen,database:fault_db}=coordinator_mapper_boundary_endpoint("CREATE TRIGGER collateral_business AFTER INSERT ON agent_assignments
        WHEN NEW.role='web_executor'
        BEGIN UPDATE projects SET name='escaped collateral' WHERE id=9002; END;");
    let mut f = root_tick_fixture(
        "changed-fact-business-scope",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    *fault_db.lock().unwrap()=Some(f.context.db_path.clone());
    drop(f.parent.take());
    let db = db::open(&f.context.db_path).unwrap();
    db.execute(
        "INSERT INTO projects(id,name) VALUES(9002,'isolated business sentinel')",
        [],
    )
    .unwrap();

    assert!(multi_agent_prepare(&mut f.context).is_err());
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 2);
    let row: (String, i64, i64) = db
        .query_row(
            "SELECT name,
        (SELECT COUNT(*) FROM agent_assignments WHERE role='web_executor'),
        (SELECT reserved_requests FROM agent_budget_ledger) FROM projects WHERE id=9002",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(row, ("isolated business sentinel".into(), 0, 0));
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        2,
        "already observed Root invoices survive the denied child transaction"
    );
}

#[test]
fn coordinator_changed_fact_dispatch_rejects_replaced_event_emitter() {
    let _real = RealSpecialistTransport::enter();
    let MapperBoundaryEndpoint {port,requests:seen,database:fault_db}=coordinator_mapper_boundary_endpoint("DROP TRIGGER agent_collaboration_assignment_insert;
        CREATE TRIGGER agent_collaboration_assignment_insert AFTER INSERT ON agent_assignments
        WHEN NEW.role='web_executor' BEGIN
        INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json)
        VALUES('foreign-scope',99,'assignment','foreign-child','assignment','{}'); END;");
    let mut f = root_tick_fixture(
        "changed-fact-emitter-scope",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    *fault_db.lock().unwrap()=Some(f.context.db_path.clone());
    drop(f.parent.take());
    let db = db::open(&f.context.db_path).unwrap();

    assert!(multi_agent_prepare(&mut f.context).is_err());
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 2);
    let row: (i64, i64) = db
        .query_row(
            "SELECT
        (SELECT COUNT(*) FROM agent_assignments WHERE role='web_executor'),
        (SELECT COUNT(*) FROM agent_collaboration_events WHERE scan_id='foreign-scope')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (0, 0),
        "an allowed emitter name never authorizes a replaced SQL contract"
    );
}
