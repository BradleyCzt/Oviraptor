// Actual original Root/artifact and configured model; wire factories only.
// No Mapper SDK, paid loop, grant or completed execution is claimed here.
fn mapper_request_inventory(
    db: &rusqlite::Connection,
    f: &RootTickFixture,
) -> super::mapper_evidence_lookup::MapperEvidenceInventory {
    super::mapper_evidence_lookup::MapperEvidenceInventory::capture_on(
        db,
        &f.actor,
        &f.context.target_dir,
        1,
        super::mapper_evidence_lookup::MapperLookupLimits::new(256 * 1024, 512).unwrap(),
    )
    .unwrap()
}
fn mapper_request_response(
    id: &str,
    arguments: JsonValue,
) -> crate::agent_runtime::model::gateway::ModelResponse {
    crate::agent_runtime::model::gateway::ModelResponse {
        tool_calls: vec![crate::agent_runtime::model::gateway::ToolCall {
            id: id.into(),
            name: "evidence.lookup".into(),
            arguments,
        }],
        ..Default::default()
    }
}
#[test]
fn mapper_loop_request_original_wire_complete_bound_and_actual_second_are_readonly() {
    let f = root_tick_fixture("mapper-request-wire", "http://127.0.0.1:65530/v1");
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let configured =
        agent_model_profile(&f.context.environment, f.context.proxy.as_deref()).unwrap();
    let inventory = mapper_request_inventory(&tx, &f);
    let bounds = super::mapper_loop_request::VerifiedMapperLoopRequestBounds::capture_on(
        &tx,
        &f.actor,
        &f.context.target_dir,
        inventory,
        &f.context.environment,
        f.context.proxy.as_deref(),
    )
    .unwrap();
    assert_eq!(bounds.profile().model, configured.model);
    assert_eq!(bounds.profile().endpoint, configured.endpoint);
    assert_eq!(bounds.profile().max_output_tokens, Some(512));
    assert!(configured.max_output_tokens.is_none_or(|cap| cap >= 512));
    assert_eq!(bounds.first_request().tools.len(), 1);
    assert_eq!(bounds.first_request().tools[0].name, "evidence.lookup");
    let specs: Vec<_> = bounds
        .first_request()
        .tools
        .iter()
        .map(|t| t.as_function_spec())
        .collect();
    let measured = i64::try_from(
        serde_json::to_vec(&json!({"messages":bounds.first_request().messages,"tools":specs}))
            .unwrap()
            .len(),
    )
    .unwrap()
        + 1024;
    assert_eq!(bounds.first_estimate(), measured);
    assert!(bounds.first_estimate() + bounds.second_estimate() <= 8000);
    assert_eq!(
        bounds.fact()["totalEstimatedTokens"],
        bounds.first_estimate() + bounds.second_estimate()
    );
    let observation = bounds
        .inventory()
        .lookup_on(&tx, &f.actor, &json!({"evidenceId":"frontend-evidence"}))
        .unwrap();
    let response =
        mapper_request_response("original-lookup", json!({"evidenceId":"frontend-evidence"}));
    let second = bounds.second_request(&response, &observation).unwrap();
    assert_eq!(second.messages.len(), 4);
    assert_eq!(
        second.messages[3]["content"],
        observation.value().to_string()
    );
    assert_eq!(
        second.messages[2]["tool_calls"][0]["id"],
        response.tool_calls[0].id
    );
    let actual = i64::try_from(
        serde_json::to_vec(&json!({"messages":second.messages,"tools":specs}))
            .unwrap()
            .len(),
    )
    .unwrap()
        + 1024;
    assert!(actual <= bounds.second_estimate());
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
}
#[test]
fn mapper_loop_request_raw_escape_utf8_and_invalid_history_preserve_frozen_bounds() {
    let f = root_tick_fixture("mapper-request-escape", "http://127.0.0.1:65530/v1");
    // Freeze this temporary artifact before inventory capture; do not alter a
    // captured original hash or model budget to manufacture a passing bound.
    fs::write(
        f.context.target_dir.join("frontend-evidence.json"),
        json!({"url":f.actor.target_key,"notes":"集团省市\n\"\\界".repeat(1000)}).to_string(),
    )
    .unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let bounds = super::mapper_loop_request::VerifiedMapperLoopRequestBounds::capture_on(
        &tx,
        &f.actor,
        &f.context.target_dir,
        mapper_request_inventory(&tx, &f),
        &f.context.environment,
        f.context.proxy.as_deref(),
    )
    .unwrap();
    let obs = bounds
        .inventory()
        .lookup_on(&tx, &f.actor, &json!({"evidenceId":"frontend-evidence"}))
        .unwrap();
    assert_eq!(obs.value()["truncated"], true);
    assert!(obs.value().to_string().len() <= 512);
    for id in ["\0".repeat(64), "界".repeat(21)] {
        let response = mapper_request_response(&id, json!({"evidenceId":"frontend-evidence"}));
        let second = bounds.second_request(&response, &obs).unwrap();
        let specs: Vec<_> = second.tools.iter().map(|t| t.as_function_spec()).collect();
        let actual = i64::try_from(
            serde_json::to_vec(&json!({"messages":second.messages,"tools":specs}))
                .unwrap()
                .len(),
        )
        .unwrap()
            + 1024;
        assert!(actual <= bounds.second_estimate());
    }
    let mut invalid = vec![
        mapper_request_response("", json!({})),
        mapper_request_response(&"界".repeat(22), json!({})),
        mapper_request_response("id", json!({"evidenceId":"x".repeat(70)})),
    ];
    let mut text = mapper_request_response("id", json!({}));
    text.text = "not frozen empty tool text".into();
    invalid.push(text);
    for response in invalid {
        assert!(
            matches!(bounds.second_request(&response,&obs),Err(e) if e=="mapper_loop_actual_history_exceeds_frozen_bound")
        );
    }
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
}
#[test]
fn mapper_loop_request_actor_artifact_and_observation_limit_drift_refuse_without_writes() {
    let f = root_tick_fixture("mapper-request-drift", "http://127.0.0.1:65530/v1");
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let mut foreign = f.actor.clone();
    foreign.attempt_number += 1;
    assert!(
        matches!(super::mapper_loop_request::VerifiedMapperLoopRequestBounds::capture_on(&tx,&foreign,&f.context.target_dir,mapper_request_inventory(&tx,&f),&f.context.environment,None),Err(e) if e=="mapper_lookup_actor_changed")
    );
    let inventory = super::mapper_evidence_lookup::MapperEvidenceInventory::capture_on(
        &tx,
        &f.actor,
        &f.context.target_dir,
        1,
        super::mapper_evidence_lookup::MapperLookupLimits::new(256 * 1024, 513).unwrap(),
    )
    .unwrap();
    assert!(
        matches!(super::mapper_loop_request::VerifiedMapperLoopRequestBounds::capture_on(&tx,&f.actor,&f.context.target_dir,inventory,&f.context.environment,None),Err(e) if e=="mapper_loop_inventory_bound_invalid")
    );
    let inventory = mapper_request_inventory(&tx, &f);
    fs::write(
        f.context.target_dir.join("frontend-evidence.json"),
        json!({"url":f.actor.target_key,"changed":true}).to_string(),
    )
    .unwrap();
    assert!(
        matches!(super::mapper_loop_request::VerifiedMapperLoopRequestBounds::capture_on(&tx,&f.actor,&f.context.target_dir,inventory,&f.context.environment,None),Err(e) if e=="mapper_lookup_source_changed")
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
}
