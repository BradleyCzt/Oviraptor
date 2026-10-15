// Actual Source provider requests, original born budget and production executor.
#[test]
fn source_controlled_executor_actual_three_rounds_path_correction_and_finish() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler, source, supervisor::WorkerSupervisor},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let observed = Arc::new(std::sync::Mutex::new(Vec::<JsonValue>::new()));
    let requests = observed.clone();
    let (port, _, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(move |raw| {
        let body: JsonValue = serde_json::from_str(raw.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let number = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let messages = body["messages"].as_array().unwrap();
        let observations = messages
            .iter()
            .filter(|m| m["role"] == "tool")
            .map(|m| serde_json::from_str::<JsonValue>(m["content"].as_str().unwrap()).unwrap())
            .collect::<Vec<_>>();
        let tool = |id: &str, name: &str, args: JsonValue| json!({"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}});
        let tools = match number {
            0 => {
                assert!(observations.is_empty());
                vec![tool(
                    "source-round-1-inventory",
                    "repo.inventory",
                    json!({}),
                )]
            }
            1 => {
                assert_eq!(observations.len(), 1);
                assert!(
                    observations[0].to_string().contains("app.py"),
                    "actual inventory must reach next provider request"
                );
                vec![
                    tool(
                        "bad-path",
                        "repo.read_slice",
                        json!({"path":"../outside.py"}),
                    ),
                    tool(
                        "corrected-path",
                        "repo.read_slice",
                        json!({"path":"app.py","startLine":1,"maxLines":20}),
                    ),
                ]
            }
            2 => {
                assert_eq!(observations.len(), 3);
                assert_eq!(
                    observations[1]["code"], "path_escape",
                    "actual audited path denial must reach provider"
                );
                assert!(
                    observations[2].to_string().contains("print('changed')"),
                    "corrected read must use frozen repository bytes"
                );
                vec![tool(
                    "source-round-3-finish",
                    "assignment.finish",
                    json!({"summary":"Frozen inventory and corrected source read inspected; no confirmed finding","gaps":["independent review still required"]}),
                )]
            }
            _ => panic!("unexpected fourth actual Source SDK request"),
        };
        requests.lock().unwrap().push(body);
        (200,"application/json",json!({"choices":[{"message":{"role":"assistant","tool_calls":tools},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let model = source_specialist_test_environment(port);
    let (root, db, _, actor) = source_tool_true_born_fixture_model(Some(&model));
    db.execute(
        "UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",
        [&actor.root_run_id],
    )
    .unwrap();
    let role = AgentRole::SourceAnalyst;
    crate::agent_runtime::multi_agent::directive::source_guidance::freeze(&db, &actor, role, true)
        .unwrap();
    let slice = source::tool_task_slice(&db, &actor, role, 1).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &actor,
        role,
        AgentLane::ReadOnlyAnalysis,
        "source_tools_ready",
        &slice,
        1,
        &source::tool_capabilities(role).unwrap(),
        24_000,
        3,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &actor, &child).unwrap();
    let path = root.join("oviraptor.sqlite3");
    let work = root.join("attempt-0001");
    let parent = WorkerSupervisor::start(&path, &actor).unwrap();
    let context = SpecialistTransportContext {
        supervision: Some(parent.ticket()),
        db_path: &path,
        scan_id: &actor.scan_id,
        attempt_number: actor.attempt_number,
        target_key: &actor.target_key,
        run_id: &actor.root_run_id,
        environment: &model,
        proxy: None,
        usage_dir: &work,
        deadline: None,
    };
    let result = execute_source_tool_assignment(&db, &context, &actor, &child, &slice).unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_model_rounds WHERE child_run_id=?1 AND state='received'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),3);
    let rows=db.prepare("SELECT round_number,call_index,tool_name,state,output_json FROM agent_source_tool_receipts WHERE child_run_id=?1 ORDER BY round_number,call_index").unwrap().query_map([&child.run_id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|r| r.3 == "completed"));
    assert_eq!(
        rows.iter().map(|r| (r.0, r.1)).collect::<Vec<_>>(),
        vec![(1, 0), (2, 0), (2, 1), (3, 0)]
    );
    assert_eq!(
        serde_json::from_str::<JsonValue>(&rows[1].4).unwrap()["code"],
        "path_escape"
    );
    let provider = observed.lock().unwrap();
    for (request_index, receipt_indices) in [(1, vec![0usize]), (2, vec![0usize, 1, 2])] {
        let actual = provider[request_index]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "tool")
            .map(|m| serde_json::from_str::<JsonValue>(m["content"].as_str().unwrap()).unwrap())
            .collect::<Vec<_>>();
        let original = receipt_indices
            .into_iter()
            .map(|i| serde_json::from_str::<JsonValue>(&rows[i].4).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            actual, original,
            "provider observations must equal original committed tool receipts"
        );
    }
    drop(provider);
    let request_balance = budget::balance(
        &db,
        &actor.root_run_id,
        Some(&child.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (
            request_balance.reserved,
            request_balance.consumed,
            request_balance.indeterminate
        ),
        (0, 3, 0)
    );
    let target = budget::balance(
        &db,
        &actor.root_run_id,
        Some(&child.assignment_id),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (target.reserved, target.consumed, target.indeterminate),
        (0, 0, 0)
    );
    let usage:(i64,i64,i64,i64)=db.query_row("SELECT spent_tokens,spent_requests,reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(usage, (60, 3, 0, 0));
    let completed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND a.state='completed' AND a.budget_settled_at<>'' AND r.terminal_state='completed' AND r.used_requests=3 AND r.used_tokens=60) AND (SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='source_tool_result' AND acknowledged_at<>'' AND delivery_attempts=1)=1 AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1) AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND revoked_at='')",[&child.assignment_id],|r|r.get(0)).unwrap();
    assert!(completed, "{result}");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_evidence_nodes WHERE created_by_run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "reads and denied paths cannot become findings"
    );
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(parent);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

// Source has a stricter original cross-round ID contract than Web fallback IDs.
#[test]
fn source_controlled_executor_actual_reused_id_keeps_known_fee_without_finish() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{budget, scheduler, source, supervisor::WorkerSupervisor},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let (port, _, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(move |raw| {
        let request: JsonValue =
            serde_json::from_str(raw.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let number = observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (name, args) = match number {
            0 => ("repo.inventory", json!({})),
            1 => {
                let original = request["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["role"] == "tool")
                    .unwrap();
                assert!(original["content"].as_str().unwrap().contains("app.py"));
                (
                    "assignment.finish",
                    json!({"summary":"must not finish with a reused original call id","gaps":[]}),
                )
            }
            _ => panic!("rejected paid Source response cannot issue another SDK"),
        };
        (200,"application/json",json!({"choices":[{"message":{"role":"assistant","tool_calls":[{"id":"source-original-reused-id","type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let model = source_specialist_test_environment(port);
    let (root, db, _, actor) = source_tool_true_born_fixture_model(Some(&model));
    db.execute(
        "UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",
        [&actor.root_run_id],
    )
    .unwrap();
    let role = AgentRole::SourceAnalyst;
    crate::agent_runtime::multi_agent::directive::source_guidance::freeze(&db, &actor, role, true)
        .unwrap();
    let slice = source::tool_task_slice(&db, &actor, role, 1).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &actor,
        role,
        AgentLane::ReadOnlyAnalysis,
        "source_tools_ready",
        &slice,
        1,
        &source::tool_capabilities(role).unwrap(),
        24_000,
        3,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &actor, &child).unwrap();
    let path = root.join("oviraptor.sqlite3");
    let work = root.join("attempt-0001");
    let parent = WorkerSupervisor::start(&path, &actor).unwrap();
    let context = SpecialistTransportContext {
        supervision: Some(parent.ticket()),
        db_path: &path,
        scan_id: &actor.scan_id,
        attempt_number: actor.attempt_number,
        target_key: &actor.target_key,
        run_id: &actor.root_run_id,
        environment: &model,
        proxy: None,
        usage_dir: &work,
        deadline: None,
    };

    let error = execute_source_tool_assignment(&db, &context, &actor, &child, &slice).unwrap_err();
    assert_eq!(
        error,
        "source_tool_response_rejected:\"source_round_tool_id_reused\""
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    let (raw,usage):(String,String)=db.query_row("SELECT response_json,usage_json FROM agent_source_model_rounds WHERE child_run_id=?1 AND round_number=2 AND state='received'",[&child.run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let response: JsonValue = serde_json::from_str(&raw).unwrap();
    assert_eq!(response["rejection"], "source_round_tool_id_reused");
    assert_eq!(response["toolCalls"], json!([]));
    assert_eq!(response["usageReported"], true);
    let original_usage: JsonValue = serde_json::from_str(&usage).unwrap();
    assert_eq!(original_usage["totalTokens"], 20);
    let paid = budget::balance(
        &db,
        &actor.root_run_id,
        Some(&child.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (paid.reserved, paid.consumed, paid.indeterminate),
        (1, 2, 0)
    );
    for (dimension, expected) in [("model_input_tokens", 20), ("model_output_tokens", 20)] {
        let balance = budget::balance(
            &db,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )
        .unwrap();
        assert_eq!((balance.consumed, balance.indeterminate), (expected, 0));
    }
    let target = budget::balance(
        &db,
        &actor.root_run_id,
        Some(&child.assignment_id),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (target.reserved, target.consumed, target.indeterminate),
        (0, 0, 0)
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_tool_receipts WHERE child_run_id=?1 AND state='completed'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_source_tool_receipts WHERE child_run_id=?1 AND tool_name='assignment.finish'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_messages WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let live:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND a.state='running' AND a.budget_settled_at='' AND r.status='running' AND r.used_requests=0 AND r.used_tokens=0)",[&child.assignment_id],|r|r.get(0)).unwrap();
    assert!(live, "paid rejection is not a completed/settled child");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(parent);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
