// Actual creator, paid Web SDKs and independent advisory Root SDKs.
#[test]
fn root_budget_consumption_trigger_cold_protocol_removal_cannot_downgrade_to_legacy() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_consumption_actual_fixture(false);
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    let raw: String = db.query_row("SELECT definition_json FROM agent_root_mode_definitions WHERE root_run_id=?1",
        [root], |r| r.get(0)).unwrap();
    let fragment = "\"budgetConsumptionRule\":\"original_settled_web_call_advisory_v1\"";
    assert_eq!(raw.matches(fragment).count(), 1);
    let removed = if raw.contains(&format!("{fragment},")) {
        raw.replacen(&format!("{fragment},"), "", 1)
    } else {
        assert!(raw.contains(&format!(",{fragment}")));
        raw.replacen(&format!(",{fragment}"), "", 1)
    };
    let _: JsonValue = serde_json::from_str(&removed).unwrap();
    let trigger: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='root_mode_immutable_update'",
        [], |r| r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER root_mode_immutable_update").unwrap();
    assert_eq!(db.execute("UPDATE agent_root_mode_definitions SET definition_json=?1 WHERE root_run_id=?2",
        params![removed, root]).unwrap(), 1);
    db.execute_batch(&trigger).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let calls = f.h.model_seen.lock().unwrap().len();
    assert!(native_coordinator_budget_consumption_before_action(&f.h.context).is_err(),
        "a cold damaged v7 ticket cannot become a successful legacy no-op");
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn root_budget_consumption_trigger_cold_original_call_cost_cannot_buy_new_assessment() {
    let _real = RealSpecialistTransport::enter();
    for damage in ["inputTokens", "cachedInputTokens", "outputTokens", "modelRequests", "responseHash"] {
        let f = budget_consumption_actual_fixture(false);
        native_coordinator_budget_before_executor(&f.h.context).unwrap();
        let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
        native_model_transport(&f.h.context, &client,
            vec![json!({"role":"user","content":"actual Web bill before cold source damage"})], &[], 1)
            .unwrap_or_else(|_| panic!("original actual bill must be received"));
        let db = db::open(&f.h.db_path).unwrap();
        let root = &f.session.as_ref().unwrap().lease.root_run_id;
        let (call, raw): (String, String) = db.query_row("SELECT call_id,receipt_json FROM agent_web_model_journal WHERE root_run_id=?1 AND phase='received'",
            [root], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let mut receipt: JsonValue = serde_json::from_str(&raw).unwrap();
        if damage == "responseHash" { receipt[damage] = json!("f".repeat(64)); }
        else {
            let old = receipt["usage"][damage].as_i64().unwrap();
            receipt["usage"][damage] = json!(old + 1);
            if matches!(damage, "inputTokens" | "outputTokens") {
                receipt["usage"]["totalTokens"] = json!(receipt["usage"]["totalTokens"].as_i64().unwrap() + 1);
            }
        }
        let trigger: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='web_model_journal_no_update'",
            [], |r| r.get(0)).unwrap();
        db.execute_batch("DROP TRIGGER web_model_journal_no_update").unwrap();
        assert_eq!(db.execute("UPDATE agent_web_model_journal SET receipt_json=?1 WHERE call_id=?2 AND phase='received'",
            params![receipt.to_string(), call]).unwrap(), 1);
        db.execute_batch(&trigger).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let calls = f.h.model_seen.lock().unwrap().len();
        assert!(native_coordinator_budget_consumption_before_action(&f.h.context).is_err(), "{damage}");
        assert_eq!(super::tests::application_table_snapshot(&db), before, "{damage}");
        assert_eq!(f.h.model_seen.lock().unwrap().len(), calls, "{damage}");
        assert!(f.h.site_seen.lock().unwrap().is_empty());
    }
}

fn budget_consumption_actual_fixture(unknown: bool) -> BudgetTriggerFixture {
    budget_consumption_actual_fixture_request_limit(unknown, 20)
}

fn budget_consumption_actual_fixture_request_limit(unknown: bool, requests: i64) -> BudgetTriggerFixture {
    let target_db = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let target_reader = target_db.clone();
    let mut h = fresh_multi_production_harness_with_site("budget-consumption-original", move |wire| {
        let db = db::open(target_reader.lock().unwrap().as_ref().unwrap()).unwrap();
        let assessments: i64 = db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='budget-consumption'",
            [], |r| r.get(0)).unwrap();
        assert!(assessments > 0, "the original paid consumption assessment must publish before the target receives HTTP");
        mock_site(wire)
    });
    *target_db.lock().unwrap() = Some(h.db_path.clone());
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let response = if wire.contains("You are the Root Coordinator") && wire.contains("budget-consumption") {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original settled Web cost"],
                "missing":[],"suggestions":["assess:budget_consumption"],"costNotes":[],"risks":[]}).to_string())
        } else if let Some(root) = fresh_multi_root_wire_response(&wire) { root }
        else if wire.contains("SPA/API Mapper") {
            proposal_model_response(r#"{"summary":"actual closed original Mapper","priorityContracts":[],"risks":[]}"#)
        } else if unknown {
            let mut response: JsonValue = serde_json::from_str(&model_round(&[], 10)).unwrap();
            response.as_object_mut().unwrap().remove("usage");
            response.to_string()
        } else { model_round(&[], 10) };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    // Same existing production fixture budgets; no extra capacity to green.
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = requests;
    h.context.execution_plan.soft_model_requests = 10.min(requests);
    h.context.execution_plan.max_turns = 8;
    freeze_fresh_multi_production_harness_original_v7(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    BudgetTriggerFixture { h, session: Some(session),
        boundary: std::sync::Arc::new(std::sync::Mutex::new(None)), stop }
}

#[test]
fn root_budget_consumption_trigger_real_tool_publishes_assessment_before_actual_http() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_consumption_actual_fixture(false);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
    let reply = native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"actual Web bill before production tool entry"})], &[], 1)
        .unwrap_or_else(|_| panic!("original Web bill must be settled"));
    let run = f.h.context.run.as_ref().unwrap();
    run.model_round(1, &reply.usage, &["replay_http".into()]).unwrap();
    let args = json!({"identity":"anonymous","method":"GET",
        "url":format!("{}/", f.h.context.target_url),"family":"authorization"});
    let mut runtime = AgentToolRuntime::default();
    let invocation = runtime.begin_invocation(f.h.context.attempt_number);
    run.begin_tool("replay_http", &args, &invocation).unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let actor = &f.session.as_ref().unwrap().lease;
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 3);
    let calls = f.h.model_seen.lock().unwrap().len();
    let result = agent_execute_tool_at(&f.h.context, &mut runtime, "replay_http", &args, &invocation);
    assert!(value_first(&result.model_view, &["code"]).is_empty(), "{}", result.model_view);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls + 1);
    assert_eq!(root_tick_count(&db, &actor.root_run_id, "publication"), 4);
    let child = &f.session.as_ref().unwrap().executor;
    let root_models = crate::agent_runtime::multi_agent::budget::balance(&db,
        &actor.root_run_id, Some(""), "model_requests").unwrap();
    let web_models = crate::agent_runtime::multi_agent::budget::balance(&db,
        &actor.root_run_id, Some(&child.assignment_id), "model_requests").unwrap();
    let targets = crate::agent_runtime::multi_agent::budget::balance(&db,
        &actor.root_run_id, Some(&child.assignment_id), "target_requests").unwrap();
    assert_eq!(root_models.consumed, 4);
    assert_eq!(web_models.consumed, 1);
    assert_eq!(targets.consumed, 1);
    let before = super::tests::application_table_snapshot(&db);
    // Reconstruct the original claim inputs. Reusing the live runtime would
    // advance request_index and legitimately describe a different HTTP call.
    // The original journal rejects a duplicate claim; it does not cache/send
    // the prior response as another successful tool execution.
    let mut replay_runtime = AgentToolRuntime { target_requests: runtime.target_requests,
        ..Default::default() };
    let replay = agent_execute_tool_at(&f.h.context, &mut replay_runtime, "replay_http", &args, &invocation);
    assert_eq!(replay.model_view["code"], "evidence_write_failed", "{}", replay.model_view);
    assert_eq!(replay.model_view["error"], "http_journal_claim_write_failed");
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls + 1);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[test]
fn root_budget_consumption_trigger_sdk_preflight_revoked_capability_spends_no_root_call() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_consumption_actual_fixture(false);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
    let reply = native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"actual bill before original capability withdrawal"})], &[], 1)
        .unwrap_or_else(|_| panic!("original call must be paid"));
    let run = f.h.context.run.as_ref().unwrap();
    run.model_round(1, &reply.usage, &[]).unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    assert!(db.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1",
        [&run.run_id]).unwrap() > 0);
    let before = super::tests::application_table_snapshot(&db);
    let calls = f.h.model_seen.lock().unwrap().len();
    assert_eq!(native_model_budget_preflight(&f.h.context).unwrap_err(), "tool_capability_or_fencing_denied");
    let failure = native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"no Root bill before rejected new SDK"})], &[], 2);
    match failure {
        Err(NativeModelRoundFailure::Outcome(outcome)) => assert!(outcome.detail().contains("tool_capability_or_fencing_denied"), "{}", outcome.detail()),
        _ => panic!("revoked original capability must reject the SDK"),
    }
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn root_budget_consumption_trigger_sdk_preflight_exhausted_real_grant_spends_no_root_call() {
    let _real = RealSpecialistTransport::enter();
    // Smaller original birth cap yields one original Web request after the
    // existing supervision/reviewer floors. Never rewrite a live budget.
    let f = budget_consumption_actual_fixture_request_limit(false, 12);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
    let reply = native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"actually consume the original last Web request"})], &[], 1)
        .unwrap_or_else(|_| panic!("the original granted call must be paid"));
    let run = f.h.context.run.as_ref().unwrap();
    run.model_round(1, &reply.usage, &[]).unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    let balance = crate::agent_runtime::multi_agent::budget::balance(&db,
        &session.lease.root_run_id, Some(&session.executor.assignment_id), "model_requests").unwrap();
    assert_eq!((balance.consumed, balance.reserved, balance.indeterminate), (1, 0, 0));
    let before = super::tests::application_table_snapshot(&db);
    let calls = f.h.model_seen.lock().unwrap().len();
    assert_eq!(native_model_budget_preflight(&f.h.context).unwrap_err(), "budget_model_requests_exhausted");
    match native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"exhaustion must precede any Root assessment"})], &[], 2) {
        Err(NativeModelRoundFailure::Outcome(outcome)) => assert_eq!(outcome.terminal_code(), AGENT_STOP_HARD_REQUESTS),
        _ => panic!("an exhausted actual grant must reject the next SDK"),
    }
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn root_budget_consumption_trigger_original_paid_calls_each_assess_once_without_target_grants() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_consumption_actual_fixture(false);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    let declaration = crate::agent_runtime::web_mode::root::read(&db, root).unwrap().unwrap();
    assert_eq!(declaration.bootstrap_dispatch().unwrap()["schemaVersion"], 7);
    let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
    let mut prior_call = String::new();
    let mut cumulative_usage = AgentTokenUsage::default();
    for round in 1..=2 {
        let reply = native_model_transport(&f.h.context, &client,
            vec![json!({"role":"user","content":format!("actual original Web paid round {round}")})], &[], round)
            .unwrap_or_else(|failure| match failure {
                NativeModelRoundFailure::Model(error) => panic!(
                    "actual Web round {round} must settle before supervision: {}: {}",
                    error.code(), error.detail()),
                NativeModelRoundFailure::Outcome(outcome) => panic!(
                    "actual Web round {round} must settle before supervision: {}: {}",
                    outcome.terminal_code(), outcome.detail()),
            });
        // Preserve the actual executor's received -> model completion order.
        // A later round may not bypass the original published usage event.
        cumulative_usage = agent_add_usage(&cumulative_usage, &reply.usage);
        f.h.context.run.as_ref().unwrap().model_round(round, &cumulative_usage,
            &reply.deduplicated_tool_calls().iter().map(|call| call.name.clone()).collect::<Vec<_>>())
            .unwrap();
        let call: String = db.query_row("SELECT call_id FROM agent_web_model_journal WHERE root_run_id=?1 AND phase='received' ORDER BY round DESC LIMIT 1",
            [root], |row| row.get(0)).unwrap();
        assert_ne!(call, prior_call);
        prior_call = call;
        let fees = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
            "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id<>'' ORDER BY rowid", []).unwrap();
        let calls = f.h.model_seen.lock().unwrap().len();
        native_coordinator_budget_consumption_before_action(&f.h.context).unwrap();
        assert_eq!(f.h.model_seen.lock().unwrap().len(), calls + 1);
        assert_eq!(root_tick_count(&db, root, "publication"), 3 + round);
        assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
            "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id<>'' ORDER BY rowid", []).unwrap() == fees);
        let before = super::tests::application_table_snapshot(&db);
        native_coordinator_budget_consumption_before_action(&f.h.context).unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), calls + 1);
        assert!(f.h.site_seen.lock().unwrap().is_empty());
    }
    let phases: Vec<String> = f.h.model_seen.lock().unwrap().iter()
        .filter(|wire| wire.contains("You are the Root Coordinator"))
        .map(|wire| {
            let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            input["basis"]["phase"].as_str().unwrap().to_string()
        }).collect();
    assert_eq!(phases, ["bootstrap", "mapper-output", "budget-allocation", "budget-consumption", "budget-consumption"]);
}

#[test]
fn root_budget_consumption_trigger_unknown_original_cost_stops_without_assessment_or_target() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_consumption_actual_fixture(true);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
    assert!(native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"actual unknown original Web bill"})], &[], 1).is_err());
    let db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let calls = f.h.model_seen.lock().unwrap().len();
    assert!(native_coordinator_budget_consumption_before_action(&f.h.context).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn root_budget_consumption_trigger_revoked_original_coordinator_cannot_assess_or_continue() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_consumption_actual_fixture(false);
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let client = AgentModelClient::new(agent_model_profile(&f.h.context.environment, None).unwrap(), &[]);
    native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"paid before original Coordinator withdrawal"})], &[], 1)
        .unwrap_or_else(|_| panic!("actual original paid Web bill must exist"));
    let db = db::open(&f.h.db_path).unwrap();
    let actor = &f.session.as_ref().unwrap().lease;
    assert_eq!(db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1 WHERE root_run_id=?2",
        params![Uuid::new_v4().to_string(), actor.root_run_id]).unwrap(), 1);
    let before = super::tests::application_table_snapshot(&db);
    let calls = f.h.model_seen.lock().unwrap().len();
    assert!(native_coordinator_budget_consumption_before_action(&f.h.context).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert!(native_model_transport(&f.h.context, &client,
        vec![json!({"role":"user","content":"old Coordinator cannot authorize another Web bill"})], &[], 2).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}
