// Actual fresh Single producer with received SDK response and absent usage.
// Incoming completion reports must replay the original debt, never authorize work.
#[test]
fn fresh_single_unknown_usage_canonical_replay_retains_original_debt() {
    use crate::agent_runtime::multi_agent::budget;
    let mut h = fresh_single_production_harness("unknown-usage-canonical-replay", mock_site);
    freeze_fresh_single_production_harness(&mut h);
    let mut reply: JsonValue = serde_json::from_str(&model_round(
        &[(
            "finish_target",
            json!({"coverage":closing_ledger(&[]),"stopReason":"unknown usage"}),
        )],
        100,
    ))
    .unwrap();
    reply.as_object_mut().unwrap().remove("usage");
    retarget_model(&mut h, vec![reply.to_string()]);
    let owned = execute_fresh_single_production_harness(&mut h);
    assert!(
        matches!(owned.outcome, AgentTargetOutcome::Incomplete(_)),
        "{:?}",
        owned.outcome
    );
    assert_eq!(
        owned.outcome.terminal_code(),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    assert_eq!(h.model_seen.lock().unwrap().len(), 1);
    assert!(h.site_seen.lock().unwrap().is_empty());
    let db = db::open(&h.db_path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    budget::root::RootOwner::load_single(&db, root)
        .unwrap()
        .read_single_exit(&db)
        .unwrap();
    let balance = |dimension| budget::balance(&db, root, None, dimension).unwrap();
    let request = balance("model_requests");
    assert_eq!(
        (request.consumed, request.reserved, request.indeterminate),
        (1, 0, 0)
    );
    for dimension in budget::DIMENSIONS[..3].iter().copied() {
        assert!(balance(dimension).indeterminate > 0, "{dimension}");
    }
    assert_eq!(balance("target_requests").consumed, 0);
    assert!(
        NativeAgentState::load(&h.db_path, &h.context.scan_id, &h.context.target_url)
            .unwrap()
            .is_none()
    );
    assert!(findings_for(&h.db_path, AGENT_COVERAGE_STAGE).is_empty());
    assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert_eq!(tally.failed, 0);
    let terminal = read_agent_checkpoint(
        &h.db_path,
        &h.context.scan_id,
        &h.context.target_url,
        "agent_terminal",
    );
    assert_eq!(terminal["status"], "paused");
    assert_eq!(
        terminal["stop"]["code"],
        terminal_code::REQUEST_RECONCILIATION_REQUIRED
    );
    let original = single_finally_physical(&db);
    for _ in 0..2 {
        assert!(record_owned_agent_target_outcome(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            &owned,
            &mut tally
        ));
        assert!(record_agent_target_outcome(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
                "forged debt closure",
                AGENT_STOP_FINISH
            )),
            &mut tally
        ));
        let after = read_agent_checkpoint(
            &h.db_path,
            &h.context.scan_id,
            &h.context.target_url,
            "agent_terminal",
        );
        assert_eq!(
            after, terminal,
            "incoming completion cannot rewrite the original stop or completion"
        );
        assert_eq!(tally.counted(), 1);
        assert!(
            single_finally_physical(&db) == original,
            "all original typed rows/rowids/fees remain unchanged"
        );
    }
    assert_eq!(h.model_seen.lock().unwrap().len(), 1);
    assert!(h.site_seen.lock().unwrap().is_empty());
    drop(owned);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

// Actual paid Single transport: conflicting aliases cannot become a known
// receipt merely because the old parser selected the first valid field.
#[test]
fn fresh_single_ambiguous_usage_keeps_original_paid_request_and_blocks_response_tools() {
    use crate::agent_runtime::multi_agent::budget;
    for (label, usage) in [
        (
            "conflicting-input",
            json!({"prompt_tokens":80,"input_tokens":800,"completion_tokens":20,"total_tokens":100}),
        ),
        (
            "malformed-output",
            json!({"prompt_tokens":80,"completion_tokens":20,"output_tokens":"20","total_tokens":100}),
        ),
        (
            "conflicting-cache",
            json!({"prompt_tokens":80,"completion_tokens":20,"total_tokens":100,"prompt_tokens_details":{"cached_tokens":10},"cached_tokens":70}),
        ),
        ("duplicate-usage-wire", json!({"prompt_tokens":80,"completion_tokens":20,"total_tokens":100})),
        ("duplicate-cached-wire", json!({"prompt_tokens":80,"completion_tokens":20,"total_tokens":100,"prompt_tokens_details":{"cached_tokens":10}})),
    ] {
        let mut h = fresh_single_production_harness(label, mock_site);
        freeze_fresh_single_production_harness(&mut h);
        let mut reply: JsonValue = serde_json::from_str(&model_round(
            &[(
                "replay_http",
                json!({"url":h.context.target_url,"method":"GET","identity":"anonymous","family":"authorization"}),
            )],
            100,
        ))
        .unwrap();
        reply["usage"] = usage;
        let mut wire = reply.to_string();
        if label == "duplicate-usage-wire" {
            wire = wire.replacen("\"usage\":", "\"usage\":{\"prompt_tokens\":800,\"completion_tokens\":20,\"total_tokens\":820},\"usage\":", 1);
        } else if label == "duplicate-cached-wire" {
            wire = wire.replacen("\"cached_tokens\":10", "\"cached_tokens\":70,\"cached_tokens\":10", 1);
        }
        // Ordinary JSON parsing would keep the valid last member. The original
        // wire reader must retain the billing ambiguity rather than clear debt.
        assert!(serde_json::from_str::<JsonValue>(&wire).is_ok());
        retarget_model(&mut h, vec![wire]);
        let owned = execute_fresh_single_production_harness(&mut h);
        assert_eq!(
            owned.outcome.terminal_code(),
            terminal_code::REQUEST_RECONCILIATION_REQUIRED,
            "{label}: {:?}",
            owned.outcome
        );
        let db = db::open(&h.db_path).unwrap();
        let root = owned.original_terminal.root_run_id.as_ref().unwrap();
        let request = budget::balance(&db, root, None, "model_requests").unwrap();
        assert_eq!(
            (request.consumed, request.reserved, request.indeterminate),
            (1, 0, 0),
            "{label}"
        );
        for dimension in budget::DIMENSIONS[..3].iter().copied() {
            assert!(
                budget::balance(&db, root, None, dimension)
                    .unwrap()
                    .indeterminate
                    > 0,
                "{label}: {dimension}"
            );
        }
        assert_eq!(
            budget::balance(&db, root, None, "target_requests")
                .unwrap()
                .consumed,
            0
        );
        assert_eq!(h.model_seen.lock().unwrap().len(), 1);
        assert!(
            h.site_seen.lock().unwrap().is_empty(),
            "ambiguous paid response cannot dispatch a target request"
        );
        assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
        drop(owned);
        drop(db);
        fs::remove_dir_all(h.root).unwrap();
    }
}

#[test]
fn single_actual_input_style_usage_records_cached_subset_in_original_ledger() {
    use crate::agent_runtime::multi_agent::budget;
    let (fixture, mut context) = single_exit_private_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let mut reply: JsonValue = serde_json::from_str(&model_round(&[], 100)).unwrap();
        reply["usage"] = json!({"input_tokens":80,"output_tokens":20,"total_tokens":100,
            "input_tokens_details":{"cached_tokens":10}});
        (200, "application/json", reply.to_string())
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(
        agent_model_profile(&context.environment, None).unwrap(),
        &[],
    );
    let response = native_model_transport(
        &context,
        &client,
        vec![json!({"role":"user","content":"actual input-style original usage"})],
        &[],
        1,
    )
    .unwrap_or_else(|_| panic!("unambiguous original input-style receipt must settle"));
    assert!(response.usage_reported);
    assert_eq!(
        (
            response.usage.input_tokens,
            response.usage.cached_input_tokens,
            response.usage.output_tokens,
            response.usage.total_tokens
        ),
        (80, 10, 20, 100)
    );
    let db = db::open(&fixture.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    for (dimension, amount) in [
        ("model_input_tokens", 80),
        ("model_cached_tokens", 10),
        ("model_output_tokens", 20),
        ("model_requests", 1),
    ] {
        let balance = budget::balance(&db, root, None, dimension).unwrap();
        assert_eq!(
            (balance.consumed, balance.reserved, balance.indeterminate),
            (amount, 0, 0),
            "{dimension}"
        );
    }
    assert_eq!(
        budget::balance(&db, root, None, "target_requests")
            .unwrap()
            .consumed,
        0
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}
