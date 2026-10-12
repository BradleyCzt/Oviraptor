// Real original 600-second worker deadline; no clock/state/fee mutation.
#[test]
fn mapper_loop_natural_expiry_two_round_delivery_faults_preserve_original_paid_source() {
    use crate::agent_runtime::multi_agent::{attempts, specialist::mapper_loop};
    use std::time::{Duration, Instant};
    let rounds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = rounds.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |request| {
        let wire: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let reply = if request.contains("You are the Root Coordinator") {
            let input: JsonValue =
                serde_json::from_str(wire["messages"][1]["content"].as_str().unwrap()).unwrap();
            let step = match input["basis"]["phase"].as_str().unwrap() {
                "bootstrap" => "dispatch:spa_api_mapper",
                "lease-expired" => "assess:lease_expired",
                other => panic!("unexpected natural-expiry Root phase {other}"),
            };
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original signed expired Mapper output"],"missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string())
        } else {
            assert_eq!(wire["max_tokens"], 512);
            assert_eq!(wire["tools"][0]["function"]["name"], "evidence.lookup");
            match counted.fetch_add(1,std::sync::atomic::Ordering::SeqCst) {
                0 => json!({"choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"original_mapper_lookup","type":"function","function":{"name":"evidence.lookup","arguments":"{\"evidenceId\":\"frontend-evidence\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":7,"completion_tokens":5,"total_tokens":12,"prompt_tokens_details":{"cached_tokens":2}}}).to_string(),
                1 => {
                    let messages=wire["messages"].as_array().unwrap();
                    assert_eq!(messages.len(),4);
                    assert_eq!(messages[3]["tool_call_id"],"original_mapper_lookup");
                    let observation:JsonValue=serde_json::from_str(messages[3]["content"].as_str().unwrap()).unwrap();
                    assert_eq!(observation["ok"],true);
                    assert_eq!(observation["evidenceId"],"frontend-evidence");
                    assert!(observation["preview"].as_str().is_some_and(|v|!v.is_empty()));
                    sdk_exact_model_body(r#"{"summary":"original frozen lookup observed; no target access","priorityContracts":[],"risks":[]}"#)
                }
                n=>panic!("unexpected Mapper round {n}"),
            }
        };
        (200, "application/json", reply)
    }));
    let f = root_tick_fixture_from_web_mode_evidence_protocol(
        "mapper-loop-natural-expiry-two-round-faults",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        Some(1200),
        web_mode_fixture("multi", "https://authorized.example.test"),
        RootFixtureEvidence {
            mapper_v15: true,
            ..Default::default()
        },
    );
    let decision = native_coordinator_tick(&f.context, &f.actor).unwrap();
    let bounds = mapper_loop_allocation_bounds(&f);
    let db = db::open(&f.context.db_path).unwrap();
    let child = native_coordinator_prepare_mapper_loop_allocated(
        &db,
        &f.context,
        &f.actor,
        &decision,
        &bootstrap_mapper_task(&f),
        &mapper_loop_original_basis(&decision),
        &bounds,
    )
    .unwrap();
    let output = super::mapper_executor_adapter::native_run_mapper_controlled_executor(
        &f.context, &f.actor, &child, &bounds,
    )
    .unwrap();
    assert_eq!(output.usage().model_requests, 2);
    assert_eq!(seen.lock().unwrap().len(), 3);
    let worker = attempts::current(&db, &f.actor, &child.assignment_id).unwrap();
    let original_deadline = worker.expires_at;
    let stop = Instant::now() + Duration::from_secs(630);
    loop {
        let current = attempts::current(&db, &f.actor, &child.assignment_id).unwrap();
        assert_eq!(
            current.expires_at, original_deadline,
            "only the parent C may renew"
        );
        if current.state == "expired" {
            break;
        }
        assert!(
            Instant::now() < stop,
            "original signed expiry sweep did not arrive"
        );
        f.parent.as_ref().unwrap().check().unwrap();
        assert_eq!(
            seen.lock().unwrap().len(),
            3,
            "waiting cannot issue SDK work"
        );
        std::thread::sleep(Duration::from_secs(1));
    }
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        mapper_loop::expired_seal::read_original_on(&tx, &f.actor, &child)
            .unwrap()
            .is_none()
    );
    let state: String = tx
        .query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "paused");
    drop(tx);
    let payload = json!({"summary":output.text()});
    let original_paid:Vec<(String,i64)>=db.prepare("SELECT dimension,sum(amount) FROM agent_budget_entries WHERE lease_attempt_id=?1 AND kind='consume' GROUP BY dimension ORDER BY dimension").unwrap().query_map([&worker.id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='received'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='lookup'",[&child.run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    // Faults affect only this temporary original DB after its natural signed sweep.
    db.execute_batch("CREATE TRIGGER actual_expired_mapper_sidewrite AFTER UPDATE ON agent_assignments WHEN NEW.state='completed' BEGIN UPDATE sentinel_scans SET project_name='forbidden expiry sidewrite' WHERE id=(SELECT scan_id FROM agent_runs WHERE id=NEW.child_run_id); END").unwrap();
    let before_fault = super::tests::application_table_snapshot(&db);
    let error = complete_expired_mapper_assessment(&db, &f.actor, &child, output.usage(), &payload)
        .unwrap_err();
    assert!(
        error.starts_with("mapper_loop_expired_delivery_write_rejected:")
            && (error.contains("not authorized") || error.contains("authorization denied")),
        "restricted original writer must reject trigger sidewrite: {error}"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before_fault);
    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name='actual_expired_mapper_sidewrite'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    db.execute_batch("DROP TRIGGER actual_expired_mapper_sidewrite")
        .unwrap();
    db.execute_batch("CREATE TRIGGER actual_expired_mapper_insert_fault BEFORE INSERT ON agent_mapper_loop_expired_receipts BEGIN SELECT RAISE(IGNORE); END").unwrap();
    let before_fault = super::tests::application_table_snapshot(&db);
    assert_eq!(
        complete_expired_mapper_assessment(&db, &f.actor, &child, output.usage(), &payload)
            .unwrap_err(),
        "mapper_loop_expired_insert_unconfirmed"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before_fault);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_mapper_loop_expired_receipts WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='evidence_summary'",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let paid_after_fault:Vec<(String,i64)>=db.prepare("SELECT dimension,sum(amount) FROM agent_budget_entries WHERE lease_attempt_id=?1 AND kind='consume' GROUP BY dimension ORDER BY dimension").unwrap().query_map([&worker.id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(paid_after_fault, original_paid);
    assert_eq!(seen.lock().unwrap().len(), 3);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name='actual_expired_mapper_insert_fault'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    db.execute_batch("DROP TRIGGER actual_expired_mapper_insert_fault")
        .unwrap();

    assert_eq!(
        complete_expired_mapper_assessment(&db, &f.actor, &child, output.usage(), &payload)
            .unwrap(),
        payload
    );
    let tx = db.unchecked_transaction().unwrap();
    let receipt = mapper_loop::expired_seal::read_original_on(&tx, &f.actor, &child)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.worker_attempt_id(), worker.id);
    assert_eq!(
        mapper_loop::expired_saved::read_closed_original_on(&tx, &f.actor, &child)
            .unwrap()
            .output()
            .usage(),
        output.usage()
    );
    assert_eq!(
        attempts::current(&tx, &f.actor, &child.assignment_id)
            .unwrap()
            .state,
        "expired"
    );
    drop(tx);
    let paid = native_coordinator_lease_expiry_feedback(&f.context, &f.actor, &child).unwrap();
    assert_eq!(
        paid.summary.as_json()["suggestions"],
        json!(["assess:lease_expired"])
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        4,
        "bootstrap, two original Mapper SDKs, expired-output Root"
    );
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        complete_expired_mapper_assessment(&db, &f.actor, &child, output.usage(), &payload)
            .unwrap(),
        payload
    );
    let replay = native_coordinator_lease_expiry_feedback(&f.context, &f.actor, &child).unwrap();
    assert_eq!(replay.event_sequence, paid.event_sequence);
    assert_eq!(seen.lock().unwrap().len(), 4);
    let replay_output = super::mapper_executor_adapter::native_run_mapper_controlled_executor(
        &f.context, &f.actor, &child, &bounds,
    )
    .unwrap();
    assert_eq!(replay_output.fact(), output.fact());
    let history = read_native_mapper_prepared_history_at(
        &f.context.db_path,
        &f.actor.scan_id,
        f.actor.attempt_number,
        &f.actor.root_run_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(history["stage"], "original expired mapper delivered");
    assert_eq!(history["rootFinalOutcomeVerified"], false);
    assert_eq!(history["executionAllowed"], false);
    assert_eq!(history["usage"]["modelRequests"], 2);
    assert_eq!(history["supervisionEventSequence"], paid.event_sequence);
    assert_eq!(seen.lock().unwrap().len(), 4);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}
