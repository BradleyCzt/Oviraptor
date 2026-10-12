// U178: the U177-minted InputParser child runs the readonly SDK input surface.
// Full real chain: fresh creator -> paid Root -> paid Mapper -> SDK Executor Broker
// GET `/api/orders?page=1` -> U175 frozen frame -> U177 fixed-grant mint -> readonly
// SDK. The SDK input is only redacted metadata of the authorized frozen record
// (method/path/observed query parameter names/body presence), no tools, no target.
// The output is a strict parameter inventory (name/location/typeAssumption/one
// controlled Broker variant template id), never an attack string, invented
// parameter, live target request or `confirmed` impact. Any out-of-bounds attempt
// (tampered Broker record, fact re-scoped to another run, a write capability, a
// target-request capability) is refused before any provider byte with zero writes.
const IP_SDK_VALID_ASSESSMENT: &str = r#"{"summary":"page 为分页整数的只读类型假设，未实测；仍需实时探测","parameters":[{"name":"page","location":"query","typeAssumption":"integer","brokerVariantTemplateId":"controlled_boundary_overlong"}],"gaps":["missing_live_parameter_probe"]}"#;

// Point the already-finished fixture at a fresh provider that answers the readonly
// InputParser child dispatch, then run the readonly driver against the Root context.
fn input_parser_sdk_run(
    f: &mut InputParserRootFixture,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    response: std::sync::Arc<dyn Fn(&str) -> String + Send + Sync>,
) -> (Result<(), String>, Seen, std::sync::Arc<std::sync::atomic::AtomicBool>) {
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire: String| {
        (200, "application/json", response(&wire))
    }));
    f.h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let lease = f.lease().clone();
    let result = multi_agent_input_parser_readonly(&f.h.context, &lease, child);
    (result, seen, stop)
}

fn input_parser_sdk_mint(f: &InputParserRootFixture)
    -> crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
    input_parser_mint_settle_executor(f);
    let decision = f.feedback().unwrap();
    input_parser_mint(f, &decision).unwrap()
}

#[test]
fn input_parser_sdk_readonly_surface_delivers_strict_assessment() {
    use crate::agent_runtime::contract::AgentRole;
    let _real = RealSpecialistTransport::enter();
    let mut f = input_parser_root_fixture("ip-sdk-deliver", &["dispatch:input_parser"]);
    let child = input_parser_sdk_mint(&f);
    let db = db::open(&f.h.db_path).unwrap();
    // The readonly surface exposes only redacted frozen metadata, bound to the
    // paid frame: one observed query parameter, no values, no grant, no tools.
    let task = input_parser_build_surface(&db, f.lease(), &child, &f.h.context.target_dir).unwrap();
    assert_eq!(task.phase, "input_parser_readonly");
    assert_eq!((task.target_requests_granted, task.browser_actions_granted), (0, 0));
    assert!(task.tools_granted.is_empty());
    assert_eq!(task.surfaces.len(), 1);
    let surface = &task.surfaces[0];
    assert_eq!((surface.method.as_str(), surface.path.as_str(), surface.body_present),
        ("GET", "/api/orders", false));
    assert_eq!(surface.parameters.len(), 1);
    assert_eq!((surface.parameters[0].name.as_str(), surface.parameters[0].location.as_str()),
        ("page", "query"));
    let slice: JsonValue = serde_json::from_str(&db.query_row(
        "SELECT task_slice_json FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| r.get::<_, String>(0)).unwrap()).unwrap();
    assert_eq!(task.decision_hash, slice["frame"]["decisionHash"].as_str().unwrap());

    let before = f.requests(&db);
    let (result, seen, stop) = input_parser_sdk_run(&mut f, &child, std::sync::Arc::new(|_wire: &str| {
        proposal_model_response(IP_SDK_VALID_ASSESSMENT)
    }));
    result.unwrap();
    // The actual SDK request carried the redacted surface: no values, no tools,
    // bounded output; the raw `?page=1` value never left the box.
    let wire = { let w = seen.lock().unwrap(); assert_eq!(w.len(), 1); w[0].clone() };
    let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert!(body.get("tools").is_none_or(|v| v == &json!([])));
    assert_eq!(body.get("max_tokens").or_else(|| body.get("max_completion_tokens")).and_then(JsonValue::as_u64), Some(512));
    let content = body["messages"][1]["content"].as_str().unwrap();
    assert!(content.contains("input_parser_readonly") && content.contains("page") && content.contains("/api/orders"));
    assert!(!content.contains("page=1"), "query values must stay redacted out of the surface");
    assert!(!wire.contains("page=1"));
    // Delivered: child completed, one acknowledged message, receipt consumed,
    // reservation settled, and — crucially — no new target request and no claim.
    assert_eq!(db.query_row("SELECT state FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| r.get::<_, String>(0)).unwrap(), "completed");
    assert_eq!(db.query_row("SELECT status,terminal_state FROM agent_runs WHERE id=?1", [&child.run_id], |r| Ok((r.get::<_,String>(0)?, r.get::<_,String>(1)?))).unwrap(), ("terminal".into(), "completed".into()));
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='input_assessment' AND acknowledged_at<>''", [&child.assignment_id], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1 AND role='input_parser' AND state='received'", [&child.assignment_id], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| Ok((r.get::<_,i64>(0)?, r.get::<_,i64>(1)?))).unwrap(), (0, 0));
    let after = f.requests(&db);
    assert_eq!((after.consumed, after.indeterminate), (before.consumed, before.indeterminate),
        "readonly child pays its own fixed grant, not the root model-request ledger");
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1, "no target request from the readonly child");
    assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)", [&child.run_id], |r| r.get::<_, bool>(0)).unwrap(), "readonly child claims no target request");
    // No target-touching lane was ever leased to the readonly child.
    assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1 AND lane<>'read_only_analysis')", [&child.assignment_id], |r| r.get::<_, bool>(0)).unwrap());
    // No flag flip: gate stays not_ready, prepare stays false. U179 (pin updated,
    // reason: cleared `zero_write_proof_on_failure`) leaves zero InputParser blockers.
    for role in [AgentRole::InputParser, AgentRole::BusinessLogic, AgentRole::Authorization, AgentRole::ExternalSurface] {
        // U181 (pin updated, reason: U181 按计划只翻 InputParser): exactly InputParser is true.
        assert_eq!(native_root_role_prepare_implemented(role), role == AgentRole::InputParser, "{role:?}");
    }
    assert_eq!(native_root_prepare_a_blockers_remaining(AgentRole::InputParser),
        Some(&[][..]));
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn input_parser_sdk_refuses_tampered_foreign_write_or_target_request_before_sdk_with_zero_writes() {
    let _real = RealSpecialistTransport::enter();
    let mut f = input_parser_root_fixture("ip-sdk-negative", &["dispatch:input_parser"]);
    let child = input_parser_sdk_mint(&f);
    let db = db::open(&f.h.db_path).unwrap();
    let calls = f.calls();
    let answer = std::sync::Arc::new(|_: &str| proposal_model_response(IP_SDK_VALID_ASSESSMENT));
    // A refusal here must spend no provider byte and leave every table untouched.
    let refuse = |f: &mut InputParserRootFixture, physical: &_, expect: &str| {
        let (result, seen, stop) = input_parser_sdk_run(f, &child, answer.clone());
        assert_eq!(result.unwrap_err(), expect);
        assert_eq!(seen.lock().unwrap().len(), 0, "{expect}: zero SDK");
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        web_mode_assert_rows(&db, physical);
        assert_eq!(f.calls(), calls);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1 AND state='received'", [&child.assignment_id], |r| r.get::<_, i64>(0)).unwrap(), 0);
    };
    // (1) Broker record bytes tampered after the mint.
    let (node, path) = f.record(&db);
    let original = std::fs::read(&path).unwrap();
    let mut tampered: JsonValue = serde_json::from_slice(&original).unwrap();
    // Flip only the query VALUE (page=1 -> page=2): the redacted surface is byte-identical
    // (same "page" param name), so nothing but the record hash re-verification can catch it.
    tampered["request"]["url"] = format!("{}/api/orders?page=2", f.h.context.target_url).into();
    std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    let physical = web_mode_test_rows(&db);
    refuse(&mut f, &physical, "input_parser_surface_fact_invalid");
    std::fs::write(&path, &original).unwrap();
    // (2) The cited fact re-scoped to another live run: never this Root's to read.
    let mapper_run = f.session.as_ref().unwrap().mapper.run_id.clone();
    db.execute("UPDATE agent_evidence_nodes SET root_run_id=?2 WHERE id=?1", params![node, mapper_run]).unwrap();
    let physical_moved = web_mode_test_rows(&db);
    refuse(&mut f, &physical_moved, "input_parser_surface_fact_invalid");
    db.execute("UPDATE agent_evidence_nodes SET root_run_id=?2 WHERE id=?1", params![node, f.root]).unwrap();
    // (3) A write capability on the readonly child is refused (attempt to write).
    let caps: String = db.query_row("SELECT capability_lease_json FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| r.get(0)).unwrap();
    db.execute("UPDATE agent_assignments SET capability_lease_json=?2 WHERE id=?1",
        params![child.assignment_id, json!(["evidence.read", "evidence.write", "mailbox.write"]).to_string()]).unwrap();
    let physical_write = web_mode_test_rows(&db);
    refuse(&mut f, &physical_write, "input_parser_capability_invalid");
    // (4) A target-request capability is refused too (attempt to touch the target).
    db.execute("UPDATE agent_assignments SET capability_lease_json=?2 WHERE id=?1",
        params![child.assignment_id, json!(["evidence.read", "mailbox.write", "target.request"]).to_string()]).unwrap();
    let physical_target = web_mode_test_rows(&db);
    refuse(&mut f, &physical_target, "input_parser_capability_invalid");
    db.execute("UPDATE agent_assignments SET capability_lease_json=?2 WHERE id=?1", params![child.assignment_id, caps]).unwrap();
    // Restored: the authorized readonly surface delivers exactly once.
    let (result, seen, stop) = input_parser_sdk_run(&mut f, &child, answer.clone());
    result.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(db.query_row("SELECT state FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| r.get::<_, String>(0)).unwrap(), "completed");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn input_parser_sdk_output_schema_is_strict() {
    use crate::agent_runtime::multi_agent::input_parser::{self, Parameter, Surface, Task};
    let task = Task {
        schema_version: 1,
        phase: input_parser::PHASE.into(),
        root_run_id: "root".into(),
        scan_id: "scan".into(),
        attempt_number: 1,
        target: "https://t.example/".into(),
        coordinator_epoch: 1,
        coordinator_fence: "fence".into(),
        decision_hash: "ab".repeat(32),
        target_requests_granted: 0,
        browser_actions_granted: 0,
        tools_granted: Vec::new(),
        surfaces: vec![Surface {
            evidence_ref: "ev-1".into(),
            request_id: "req-1".into(),
            record_sha256: "cd".repeat(32),
            body_sha256: None,
            method: "GET".into(),
            path: "/api/orders".into(),
            parameters: vec![Parameter { name: "page".into(), location: "query".into() }],
            body_present: false,
        }],
    };
    // The authorized surface + a controlled template passes.
    input_parser::validate_assessment(IP_SDK_VALID_ASSESSMENT, &task).unwrap();
    let good = r#"{"summary":"s","parameters":[{"name":"page","location":"query","typeAssumption":"integer","brokerVariantTemplateId":"controlled_boundary_empty"}],"gaps":["missing_live_parameter_probe"]}"#;
    input_parser::validate_assessment(good, &task).unwrap();
    for (bad, why) in [
        // A confirmed impact / finding write is never a readonly parser output.
        (r#"{"summary":"s","confirmed":true,"parameters":[{"name":"page","location":"query","typeAssumption":"integer","brokerVariantTemplateId":"controlled_boundary_empty"}],"gaps":["missing_live_parameter_probe"]}"#, "confirmed"),
        (r#"{"summary":"s","candidates":[{"x":1}],"parameters":[{"name":"page","location":"query","typeAssumption":"integer","brokerVariantTemplateId":"controlled_boundary_empty"}],"gaps":["missing_live_parameter_probe"]}"#, "candidates"),
        // An invented / attack-string parameter name was never observed.
        (r#"{"summary":"s","parameters":[{"name":"page' OR '1'='1","location":"query","typeAssumption":"string","brokerVariantTemplateId":"controlled_boundary_empty"}],"gaps":["missing_live_parameter_probe"]}"#, "invented_name"),
        // A live target-request template is not a controlled variant.
        (r#"{"summary":"s","parameters":[{"name":"page","location":"query","typeAssumption":"integer","brokerVariantTemplateId":"live_target_probe"}],"gaps":["missing_live_parameter_probe"]}"#, "live_template"),
        // The live-probe gap must stay; an impact can never be claimed from here.
        (r#"{"summary":"s","parameters":[{"name":"page","location":"query","typeAssumption":"integer","brokerVariantTemplateId":"controlled_boundary_empty"}],"gaps":["other"]}"#, "missing_required_gap"),
        // Body parameter location is not exposed by the redacted surface.
        (r#"{"summary":"s","parameters":[{"name":"page","location":"body","typeAssumption":"integer","brokerVariantTemplateId":"controlled_boundary_empty"}],"gaps":["missing_live_parameter_probe"]}"#, "body_location"),
        // An out-of-allowlist type assumption.
        (r#"{"summary":"s","parameters":[{"name":"page","location":"query","typeAssumption":"sql","brokerVariantTemplateId":"controlled_boundary_empty"}],"gaps":["missing_live_parameter_probe"]}"#, "bad_type"),
        // Empty parameter inventory.
        (r#"{"summary":"s","parameters":[],"gaps":["missing_live_parameter_probe"]}"#, "empty_params"),
    ] {
        assert!(input_parser::validate_assessment(bad, &task).is_err(), "{why} must be refused");
    }
}

#[test]
fn input_parser_sdk_invalid_output_is_not_delivered() {
    let _real = RealSpecialistTransport::enter();
    let mut f = input_parser_root_fixture("ip-sdk-badout", &["dispatch:input_parser"]);
    let child = input_parser_sdk_mint(&f);
    let db = db::open(&f.h.db_path).unwrap();
    // The provider returns an out-of-surface (invented parameter) assessment.
    let bad = r#"{"summary":"s","parameters":[{"name":"admin","location":"query","typeAssumption":"string","brokerVariantTemplateId":"controlled_type_confusion"}],"gaps":["missing_live_parameter_probe"]}"#;
    let (result, seen, stop) = input_parser_sdk_run(&mut f, &child, std::sync::Arc::new(move |_: &str| proposal_model_response(bad)));
    assert_eq!(result.unwrap_err(), "input_parser_readonly_assessment_insufficient");
    assert_eq!(seen.lock().unwrap().len(), 1, "the SDK was called");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    // Delivery was refused and rolled back: child never completed, nothing acked,
    // reservation intact. No finding or message was published.
    assert_eq!(db.query_row("SELECT state FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| r.get::<_, String>(0)).unwrap(), "running");
    assert_eq!(db.query_row("SELECT status FROM agent_runs WHERE id=?1", [&child.run_id], |r| r.get::<_, String>(0)).unwrap(), "running");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='input_assessment'", [&child.assignment_id], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(db.query_row("SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| Ok((r.get::<_,i64>(0)?, r.get::<_,i64>(1)?))).unwrap(), (4_000, 1));
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1, "no target request");
}
