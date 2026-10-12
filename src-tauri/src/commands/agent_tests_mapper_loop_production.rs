// Failure-only metadata from the actual temporary original DB; no invented rows.
fn mapper_loop_production_failure_inventory(context: &AgentRunContext) {
    let db = db::open(&context.db_path).expect("actual fixture DB must remain readable");
    for (name,sql) in [
        ("assignments", "SELECT json_object('id',id,'child',child_run_id,'state',state,'settledAt',budget_settled_at,'reservedTokens',reserved_tokens,'reservedRequests',reserved_requests) FROM agent_assignments WHERE role='spa_api_mapper' ORDER BY rowid"),
        ("workers", "SELECT json_object('id',id,'child',child_run_id,'state',state,'finishedAt',finished_at) FROM agent_assignment_attempts WHERE child_run_id IN (SELECT child_run_id FROM agent_assignments WHERE role='spa_api_mapper') ORDER BY rowid"),
        ("runs", "SELECT json_object('id',id,'status',status,'terminalState',terminal_state,'finishedAt',finished_at) FROM agent_runs WHERE role='spa_api_mapper' ORDER BY rowid"),
        ("events", "SELECT json_object('run',run_id,'sequence',sequence,'type',event_type) FROM agent_events WHERE run_id IN (SELECT child_run_id FROM agent_assignments WHERE role='spa_api_mapper') ORDER BY rowid"),
        ("snapshots", "SELECT json_object('run',run_id,'count',count(*)) FROM agent_snapshots WHERE run_id IN (SELECT child_run_id FROM agent_assignments WHERE role='spa_api_mapper') GROUP BY run_id"),
        ("journal", "SELECT json_object('call',call_id,'worker',worker_attempt_id,'round',round_number,'phase',phase,'requestHash',request_hash,'factHash',fact_hash) FROM agent_mapper_loop_journal ORDER BY round_number,phase"),
        ("finalReceipt", "SELECT json_object('receipt',receipt_id,'child',child_run_id,'worker',worker_attempt_id,'message',message_id,'factHash',fact_hash,'signatureBytes',length(signature)) FROM agent_mapper_loop_final_receipts ORDER BY rowid"),
        ("messages", "SELECT json_object('id',id,'assignment',assignment_id,'kind',kind,'from',from_run_id,'to',to_run_id,'created',created_at,'delivered',delivered_at,'ack',acknowledged_at,'attempts',delivery_attempts) FROM agent_messages WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE role='spa_api_mapper') ORDER BY rowid"),
        ("budgetEntries", "SELECT json_object('worker',lease_attempt_id,'dimension',dimension,'kind',kind,'amount',amount,'source',source_id,'key',idempotency_key) FROM agent_budget_entries WHERE assignment_id IN (SELECT id FROM agent_assignments WHERE role='spa_api_mapper') ORDER BY rowid"),
        ("collaboration", "SELECT json_object('sequence',sequence,'type',event_type,'entityType',entity_type,'entity',entity_id,'payload',json(payload_json)) FROM agent_collaboration_events WHERE entity_id IN (SELECT id FROM agent_assignments WHERE role='spa_api_mapper' UNION SELECT child_run_id FROM agent_assignments WHERE role='spa_api_mapper') ORDER BY sequence"),
    ] {
        let rows=db.prepare(sql).and_then(|mut query|query.query_map([],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>());
        match rows { Ok(rows)=>eprintln!("actual Mapper failure inventory {name}: count={} {rows:?}",rows.len()),Err(error)=>eprintln!("actual Mapper failure inventory {name}: query error {error}") }
    }
}

// Actual production prepare, original v15 paid allocation, two SDKs and one frozen lookup.
fn mapper_loop_full_production_fixture() -> (
    RootTickFixture,
    MultiAgentSession,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let rounds = Arc::new(AtomicUsize::new(0));
    let counted = rounds.clone();
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |request| {
        let wire: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        if wire["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("You are the Root Coordinator")
        {
            let input: JsonValue =
                serde_json::from_str(wire["messages"][1]["content"].as_str().unwrap()).unwrap();
            let step = match input["basis"]["phase"].as_str().unwrap() {
                "bootstrap" => "dispatch:spa_api_mapper",
                "mapper-output" => "dispatch:web_executor",
                phase => panic!("unexpected original Root phase: {phase}"),
            };
            return (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":["original frozen Mapper source"],"missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string()));
        }
        assert_eq!(wire["max_tokens"], 512, "original frozen output cap");
        assert_eq!(wire["tools"].as_array().unwrap().len(), 1);
        assert_eq!(wire["tools"][0]["function"]["name"], "evidence.lookup");
        match counted.fetch_add(1, Ordering::SeqCst) {
            0 => {
                assert_eq!(wire["messages"].as_array().unwrap().len(), 2);
                (200,"application/json",json!({"choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"original_mapper_lookup","type":"function","function":{"name":"evidence.lookup","arguments":"{\"evidenceId\":\"frontend-evidence\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":7,"completion_tokens":5,"total_tokens":12,"prompt_tokens_details":{"cached_tokens":2}}}).to_string())
            }
            1 => {
                let messages = wire["messages"].as_array().unwrap();
                assert_eq!(messages.len(), 4);
                assert_eq!(messages[2]["tool_calls"][0]["id"], "original_mapper_lookup");
                assert_eq!(messages[3]["role"], "tool");
                assert_eq!(messages[3]["tool_call_id"], "original_mapper_lookup");
                let observation: JsonValue =
                    serde_json::from_str(messages[3]["content"].as_str().unwrap()).unwrap();
                assert_eq!(observation["ok"], true);
                assert_eq!(observation["evidenceId"], "frontend-evidence");
                assert!(observation["preview"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()));
                (
                    200,
                    "application/json",
                    sdk_exact_model_body(
                        r#"{"summary":"original frozen lookup observed; no target access","priorityContracts":[],"risks":[]}"#,
                    ),
                )
            }
            n => panic!("unexpected new Mapper SDK round {n}"),
        }
    }));
    let mut f = root_tick_fixture_from_web_mode_evidence_protocol(
        "mapper-loop-production-two-round",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        None,
        web_mode_fixture("multi", "https://authorized.example.test"),
        RootFixtureEvidence {
            mapper_v15: true,
            ..Default::default()
        },
    );
    drop(f.parent.take());
    let session = multi_agent_prepare(&mut f.context).unwrap_or_else(|error| {
        mapper_loop_production_failure_inventory(&f.context);
        panic!("actual v15 full prepare failed: {error}");
    });
    assert_eq!(rounds.load(Ordering::SeqCst), 2);
    (f, session, seen)
}

#[test]
fn mapper_loop_production_prepare_two_actual_sdks_original_lookup_signed_ack_and_paid_root_replay()
{
    use crate::agent_runtime::multi_agent::specialist::mapper_loop;
    let (f, session, seen) = mapper_loop_full_production_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let output =
        mapper_loop::verified_original_output_on(&tx, &session.lease, &session.mapper).unwrap();
    assert_eq!(output.usage().model_requests, 2);
    assert_eq!(output.usage().input_tokens, 14);
    assert_eq!(output.usage().cached_input_tokens, 4);
    assert_eq!(output.usage().output_tokens, 10);
    let receipt =
        mapper_loop::final_seal::read_final_original_on(&tx, &session.lease, &session.mapper)
            .unwrap()
            .unwrap();
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_messages WHERE id=?1 AND acknowledged_at<>'' AND delivery_attempts=1",[receipt.message_id()],|r|r.get::<_,i64>(0)).unwrap(),1);
    let lookup:String=tx.query_row("SELECT fact_json FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='lookup'",[&session.mapper.run_id],|r|r.get(0)).unwrap();
    let lookup: JsonValue = serde_json::from_str(&lookup).unwrap();
    let wires = seen.lock().unwrap();
    let second = wires
        .iter()
        .filter_map(|request| {
            serde_json::from_str::<JsonValue>(request.split_once("\r\n\r\n")?.1).ok()
        })
        .find(|wire| {
            wire["messages"]
                .as_array()
                .is_some_and(|messages| messages.iter().any(|m| m["role"] == "tool"))
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<JsonValue>(second["messages"][3]["content"].as_str().unwrap())
            .unwrap(),
        lookup["observation"]
    );
    drop(wires);
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='received'",[&session.mapper.run_id],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='lookup'",[&session.mapper.run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM agent_specialist_calls WHERE child_run_id=?1",
            [&session.mapper.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        tx.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1",
            [&session.mapper.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let phases =
        followup_original_root_phase_counts(&tx, &f.actor.root_run_id, &seen.lock().unwrap());
    assert_eq!(
        phases,
        std::collections::BTreeMap::from([
            ("bootstrap".to_owned(), 1),
            ("mapper-output".to_owned(), 1)
        ])
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        phases.values().sum::<usize>() + usize::try_from(output.usage().model_requests).unwrap()
    );
    let frame = NativeCoordinatorFrame::mapper(&tx, &session.lease, &session.mapper).unwrap();
    tx.commit().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let count = seen.lock().unwrap().len();
    let root = native_coordinator_root_context(&f.context, &session.lease);
    assert!(
        native_coordinator_tick_for_frame(&root, &session.lease, &frame)
            .unwrap()
            .replayed
    );
    assert_eq!(
        complete_readonly_assessment(
            &db,
            &session.lease,
            &session.mapper,
            output.usage(),
            &json!({"summary":output.text()})
        )
        .unwrap(),
        json!({"summary":output.text()})
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), count);
}
