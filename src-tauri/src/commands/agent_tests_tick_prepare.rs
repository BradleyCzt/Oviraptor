#[test]
fn coordinator_tick_actual_multi_prepare_uses_root_sdk_then_mapper_and_original_headroom() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        if specialist_original_is_web_sdk(&request) {
            return (200,"application/json",proposal_model_response("actual bounded Web exit; no target tool"));
        }
        if request.contains("You are the Root Coordinator") && request.contains("budget-allocation") {
            return (200,"application/json",changed_fact_response(&request,true));
        }
        let text = if request.contains("You are the Root Coordinator") {
            if request.contains("mapper-output") {
                json!({"schemaVersion":1,"observed":["new frozen bootstrap decision"],"missing":[],"suggestions":["dispatch:web_executor"],"costNotes":[],"risks":[]}).to_string()
            } else {json!({"schemaVersion":1,"observed":["new frozen bootstrap decision"],"missing":[],"suggestions":["dispatch:spa_api_mapper"],"costNotes":[],"risks":[]}).to_string()}
        } else {
            r#"{"summary":"actual Mapper input","priorityContracts":[],"risks":[]}"#.into()
        };
        (200, "application/json", proposal_model_response(&text))
    }));
    let mut f = root_tick_fixture_from_web_mode_turns(
        "tick-production-prepare",
        &format!("http://127.0.0.1:{port}/v1"), true, (60_000,20), None,
        web_mode_fixture("multi","https://authorized.example.test"), Some(1),
    );
    drop(f.parent.take());
    let mut session = multi_agent_prepare(&mut f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        seen.lock().unwrap().len(),
        3,
        "actual bootstrap Root, Mapper, then changed-fact Root SDK"
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 2);
    assert!(
        f.context.evidence["multiAgentCoordinator"]["decisionSummary"]["observed"]
            .to_string()
            .contains("new frozen bootstrap decision")
    );
    let root_tokens = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        Some(""),
        "model_input_tokens",
    )
    .unwrap()
    .consumed
        + crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            Some(""),
            "model_output_tokens",
        )
        .unwrap()
        .consumed;
    let root_requests = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        Some(""),
        "model_requests",
    )
    .unwrap()
    .consumed;
    let (total_tokens,total_requests,spent_tokens,spent_requests):(i64,i64,i64,i64)=db.query_row(
        "SELECT total_tokens,total_requests,spent_tokens,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    let grants: (i64, i64) = db
        .query_row(
            "SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1",
            [&session.executor.assignment_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    // Creation contract v6 (`supervisionRequestAllocationRule`): token grants
    // keep the original retained floors; request grants retain half the surplus
    // after the budget assessment and six minimum follow-ups. The current balance
    // after Root's paid calls, minus the retained Reviewer floor (15_000/1),
    // the retained Root supervision floor (16_000/1) and two ordered
    // human-proposal reservations (2 x PROPOSAL_TOKENS / 2 requests).
    let task: JsonValue = serde_json::from_str(
        &db.query_row(
            "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
            [&session.executor.assignment_id],
            |r| r.get::<_, String>(0),
        )
        .unwrap(),
    )
    .unwrap();
    let policy = &task["rootDecision"]["rustPolicy"];
    assert_eq!(policy["reviewerFloor"], json!({"modelRequests":1,"modelTokens":15_000}));
    assert_eq!(policy["rootSupervisionFloor"], json!({"modelRequests":1,"modelTokens":16_000}));
    assert_eq!(
        (policy["costBasis"]["reservedModelTokens"].as_i64(), policy["costBasis"]["reservedModelRequests"].as_i64()),
        (Some(grants.0), Some(grants.1)),
        "the paid decision records exactly the issued grant"
    );
    assert_eq!(
        grants,
        (
            total_tokens
                - spent_tokens
                - root_tokens
                - 15_000
                - 16_000
                - 2 * crate::agent_runtime::multi_agent::directive::proposals::PROPOSAL_TOKENS,
            (total_requests - spent_requests - root_requests - 7) / 2 - 2
        ),
        "Root's paid calls preserve original token floors, v6 request headroom and both ordered human reservations"
    );
    // After the preparation assertions, settle a genuinely paid original Web
    // request and owner/SDK exit; a fabricated outcome is no longer a proof.
    let outcome=run_native_agent(&f.context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    multi_agent_finish_execution(&f.context,&mut session,&outcome).unwrap();
    assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
    let original_web:(i64,i64,i64)=db.query_row(
        "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(original_web,(20,0,1));
    drop(session);
}
