// U174: deterministic InputParser trigger over verified observed Broker HTTP facts.
fn input_parser_trigger_observe(
    context: &AgentRunContext,
    index: usize,
    request: JsonValue,
    request_id: &str,
) -> String {
    let body = format!("saved response {index}");
    let view = serde_json::json!({"requestId":request_id,"status":200,
        "bodySha256":format!("{:x}",Sha256::digest(body.as_bytes()))});
    let artifact = agent_write_http_record(context, index, &request, &view, body.as_bytes()).unwrap();
    agent_record_http_observation(context, "replay_http", &artifact, &view, request_id).unwrap();
    artifact
}

fn input_parser_trigger_root(context: &AgentRunContext) -> String {
    db::open(&context.db_path)
        .unwrap()
        .query_row(
            "SELECT root_run_id FROM agent_runs WHERE id=?1",
            [&context.run.as_ref().unwrap().run_id],
            |r| r.get(0),
        )
        .unwrap()
}

fn input_parser_trigger_node(db: &rusqlite::Connection, artifact: &str) -> String {
    db.query_row(
        "SELECT n.id FROM agent_evidence_nodes n, json_each(n.artifact_refs_json) a
         WHERE n.kind='request_record' AND a.value=?1",
        [format!("agent-http/{artifact}")],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn input_parser_trigger_requires_observed_controllable_input_from_verified_facts() {
    let (root, context, _, _) = http_journal_fixture("http://127.0.0.1:9/", 0);
    let root_run_id = input_parser_trigger_root(&context);
    let db = db::open(&context.db_path).unwrap();
    let decide = || {
        let before = super::tests::application_table_snapshot(&db);
        let changes = db.total_changes();
        let decision =
            native_input_parser_trigger_decision(&db, &root_run_id, &context.target_dir).unwrap();
        // Zero-write: the predicate never mutates a row or table.
        assert_eq!(db.total_changes(), changes);
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        decision
    };
    // No observed request at all: not eligible, prerequisite named.
    let empty = decide();
    assert!(!empty.eligible);
    assert_eq!(empty.role, "input_parser");
    assert_eq!(empty.missing_prerequisites, vec!["observed_controllable_input".to_string()]);
    assert!(empty.evidence_refs.is_empty() && empty.reason_codes.is_empty());
    // A plain GET of the target (headers only, no query/body) is not a controllable input.
    let plain = input_parser_trigger_observe(&context, 1,
        serde_json::json!({"method":"GET","url":context.target_url,"identity":"anonymous",
            "headers":[{"name":"Accept","value":"*/*"},{"name":"Content-Type","value":"application/json"}]}), "ip-plain");
    let decision = decide();
    assert!(!decision.eligible, "{decision:?}");
    assert_eq!(decision.missing_prerequisites, vec!["observed_controllable_input".to_string()]);
    // An actually requested query parameter makes InputParser eligible, citing only that fact.
    let query = input_parser_trigger_observe(&context, 2,
        serde_json::json!({"method":"GET","url":"http://127.0.0.1:9/items?id=7","identity":"anonymous",
            "contractKey":"items-by-id"}), "ip-query");
    let decision = decide();
    assert!(decision.eligible, "{decision:?}");
    assert_eq!(decision.reason_codes, vec!["observed_query_parameter".to_string()]);
    assert_eq!(decision.evidence_refs, vec![input_parser_trigger_node(&db, &query)]);
    assert_eq!(decision.contract_keys, vec!["items-by-id".to_string()]);
    assert!(decision.missing_prerequisites.is_empty());
    // An observed request body adds its own reason and fact; the plain GET is never cited.
    let body = input_parser_trigger_observe(&context, 3,
        serde_json::json!({"method":"POST","url":"http://127.0.0.1:9/items","identity":"anonymous",
            "body":"{\"name\":\"n\"}"}), "ip-body");
    let decision = decide();
    assert_eq!(decision.reason_codes,
        vec!["observed_query_parameter".to_string(), "observed_request_body".to_string()]);
    let mut expected = vec![input_parser_trigger_node(&db, &query), input_parser_trigger_node(&db, &body)];
    expected.sort();
    assert_eq!(decision.evidence_refs, expected);
    assert!(!decision.evidence_refs.contains(&input_parser_trigger_node(&db, &plain)));
    // Same facts, same decision (deterministic, order independent of reads).
    assert_eq!(decide(), decision);
    // Facts of another Root never make this Root eligible.
    let foreign = native_input_parser_trigger_decision(&db, "foreign-root", &context.target_dir).unwrap();
    assert!(!foreign.eligible && foreign.evidence_refs.is_empty());
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_parser_trigger_fails_closed_on_tampered_fact_and_never_opens_the_root_gate() {
    use crate::agent_runtime::contract::AgentRole;
    let (root, context, _, _) = http_journal_fixture("http://127.0.0.1:9/", 0);
    let root_run_id = input_parser_trigger_root(&context);
    let query = input_parser_trigger_observe(&context, 1,
        serde_json::json!({"method":"GET","url":"http://127.0.0.1:9/items?id=7","identity":"anonymous"}), "ip-query");
    let db = db::open(&context.db_path).unwrap();
    assert!(native_input_parser_trigger_decision(&db, &root_run_id, &context.target_dir).unwrap().eligible);
    // Eligibility is not dispatch: every A04 Root gate stays honest and not_ready.
    for role in [AgentRole::InputParser, AgentRole::BusinessLogic, AgentRole::Authorization, AgentRole::ExternalSurface] {
        // U181 (pin updated, reason: U181 按计划只翻 InputParser): exactly InputParser is true.
        assert_eq!(native_root_role_prepare_implemented(role), role == AgentRole::InputParser, "{role:?}");
        // U175/U177/U178/U179 (pin changed, reason: cleared `zero_write_proof_on_failure`)
        // cleared every InputParser item, so its inventory is empty while the gate below
        // still stays not_ready; the other roles keep the full shared inventory.
        let expected = if role == AgentRole::InputParser {
            NATIVE_INPUT_PARSER_PREPARE_A_BLOCKERS
        } else {
            NATIVE_ROOT_PREPARE_A_BLOCKERS
        };
        assert_eq!(native_root_prepare_a_blockers_remaining(role), Some(expected));
        if role != AgentRole::InputParser {
            assert!(!expected.is_empty());
        }
    }
    assert_eq!(native_root_schedule_decision_role_gate(AgentRole::InputParser),
        Err(NATIVE_INPUT_PARSER_DISPATCH_NOT_READY.to_string()));
    assert!(!native_reserved_role_output_producer_ready("input-parser-output"));
    // U176: input-parser-output is wired (frozen producer only); the display skeleton is refused.
    assert_eq!(native_try_produce_input_parser_output_frame("prepare").err().as_deref(),
        Some("root_trigger_skeleton_kind_invalid"));
    // A record whose bytes no longer match the hashed fact fails closed, writing nothing.
    let path = context.target_dir.join(AGENT_HTTP_DIRECTORY).join(&query);
    let original = std::fs::read(&path).unwrap();
    let mut tampered: JsonValue = serde_json::from_slice(&original).unwrap();
    tampered["request"]["url"] = "http://127.0.0.1:9/items".into();
    std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(native_input_parser_trigger_decision(&db, &root_run_id, &context.target_dir),
        Err("input_parser_trigger_fact_invalid".to_string()));
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    // Restoring the original hashed bytes restores the same verified decision.
    std::fs::write(&path, &original).unwrap();
    assert!(native_input_parser_trigger_decision(&db, &root_run_id, &context.target_dir).unwrap().eligible);
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_parser_trigger_core_ignores_headers_paths_and_unparseable_urls() {
    let record = |request: JsonValue| serde_json::json!({"request": request});
    let decision = native_input_parser_trigger_from_records(&[
        ("a".into(), record(serde_json::json!({"method":"GET","url":"http://h/users/42","headerNames":["Authorization","Content-Type"]}))),
        ("b".into(), record(serde_json::json!({"method":"GET","url":"http://h/search?","bodyBytes":0}))),
        ("c".into(), record(serde_json::json!({"method":"GET","url":"not a url?id=1"}))),
        ("d".into(), record(serde_json::json!({"method":"POST","url":"http://h/x","bodyBytes":null}))),
    ]);
    assert!(!decision.eligible, "{decision:?}");
    assert_eq!(decision.missing_prerequisites, vec!["observed_controllable_input".to_string()]);
    let decision = native_input_parser_trigger_from_records(&[
        ("z".into(), record(serde_json::json!({"url":"http://h/a?q=1","contractKey":"k2"}))),
        ("y".into(), record(serde_json::json!({"url":"http://h/b","bodyBytes":5,"contractKey":"k1"}))),
        ("x".into(), record(serde_json::json!({"url":"http://h/c?q=2","contractKey":"k2"}))),
    ]);
    assert!(decision.eligible);
    assert_eq!(decision.evidence_refs, vec!["x".to_string(), "y".into(), "z".into()]);
    assert_eq!(decision.contract_keys, vec!["k1".to_string(), "k2".into()]);
    assert_eq!(decision.reason_codes,
        vec!["observed_query_parameter".to_string(), "observed_request_body".into()]);
}
