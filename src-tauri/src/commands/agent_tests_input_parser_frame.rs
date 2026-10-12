// U175: frozen, hash-bound InputParser trigger frame + fail-closed re-verification.
// Reuses the real Broker HTTP record/observation writers (U174 helpers); clears only
// `freeze_bound_basis` for InputParser and never opens the dispatch gate.
fn input_parser_frame_fixture() -> (
    PathBuf,
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::scheduler};
    let (root, mut context, lease) = public_surface_fixture("http://127.0.0.1:9/");
    let connection = db::open(&context.db_path).unwrap();
    let child = scheduler::schedule_child(&connection, &lease, AgentRole::WebExecutor,
        AgentLane::TargetTouching, "input-parser-frame", &serde_json::json!({}), 1,
        &["replay_http".into()], 100, 1).unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    context.run.as_mut().unwrap().run_id = child.run_id;
    accounting_native(&context, 0);
    let mut runtime = AgentToolRuntime::default();
    let invocation = runtime.begin_invocation(1);
    context.run.as_ref().unwrap().begin_tool("replay_http", &serde_json::json!({}), &invocation).unwrap();
    (root, context, lease)
}

/// Runs `f` and proves it wrote nothing: same full-table snapshot, same total_changes.
fn input_parser_frame_zero_write<T>(db: &rusqlite::Connection, f: impl FnOnce() -> T) -> T {
    let before = super::tests::application_table_snapshot(db);
    let changes = db.total_changes();
    let out = f();
    assert_eq!(db.total_changes(), changes, "must not write");
    assert_eq!(super::tests::application_table_snapshot(db), before, "must not write");
    out
}

#[test]
fn input_parser_frame_freezes_verified_trigger_and_reverifies_with_zero_writes() {
    use crate::agent_runtime::contract::AgentRole;
    let (root, context, lease) = input_parser_frame_fixture();
    let db = db::open(&context.db_path).unwrap();
    // No observed controllable input: no frame, nothing written.
    let refused = input_parser_frame_zero_write(&db, || {
        NativeCoordinatorFrame::input_parser_trigger(&db, &lease, &context.target_dir).err()
    });
    assert_eq!(refused.as_deref(), Some("root_input_parser_trigger_not_eligible"));
    input_parser_trigger_observe(&context, 1, serde_json::json!({"method":"GET","url":context.target_url,
        "identity":"anonymous","headers":[{"name":"Accept","value":"*/*"}]}), "ipf-plain");
    let refused = input_parser_frame_zero_write(&db, || {
        NativeCoordinatorFrame::input_parser_trigger(&db, &lease, &context.target_dir).err()
    });
    assert_eq!(refused.as_deref(), Some("root_input_parser_trigger_not_eligible"));
    let query = input_parser_trigger_observe(&context, 2, serde_json::json!({"method":"GET",
        "url":"http://127.0.0.1:9/items?id=7","identity":"anonymous","contractKey":"items-by-id"}), "ipf-query");
    let node_id = input_parser_trigger_node(&db, &query);
    let frame = input_parser_frame_zero_write(&db, || {
        NativeCoordinatorFrame::input_parser_trigger(&db, &lease, &context.target_dir).unwrap()
    });
    // Reserved kind + reserved step, frozen from exactly the U174 decision.
    assert_eq!(frame.kind, "input-parser-output");
    assert_eq!(frame.step(), "dispatch:input_parser");
    let decision = native_input_parser_trigger_decision(&db, &lease.root_run_id, &context.target_dir).unwrap();
    assert!(decision.eligible);
    assert_eq!(frame.semantic["trigger"], serde_json::to_value(&decision).unwrap());
    assert_eq!(frame.semantic["trigger"]["evidenceRefs"], serde_json::json!([node_id]));
    assert_eq!(frame.semantic["rootRunId"], lease.root_run_id.as_str());
    assert_eq!(frame.semantic["role"], AgentRole::InputParser.as_str());
    assert_eq!((frame.semantic["targetRequestsGranted"].clone(), frame.semantic["dispatchGranted"].clone()),
        (serde_json::json!(0), serde_json::json!(false)));
    // Hash binding: the cited fact's verified Broker record/body hashes are frozen in.
    let (payload, refs): (String, String) = db.query_row(
        "SELECT payload_json,artifact_refs_json FROM agent_evidence_nodes WHERE id=?1", [&node_id],
        |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    let payload: JsonValue = serde_json::from_str(&payload).unwrap();
    let record_sha = payload["recordSha256"].as_str().unwrap().to_string();
    assert_eq!(record_sha.len(), 64);
    let on_disk = std::fs::read(context.target_dir.join(AGENT_HTTP_DIRECTORY).join(&query)).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&on_disk)), record_sha, "binding is the real record hash");
    assert_eq!(frame.semantic["factBindings"], serde_json::json!([{"evidenceRef":node_id,
        "artifactRefs":serde_json::from_str::<JsonValue>(&refs).unwrap(),"recordSha256":record_sha,
        "bodySha256":payload["bodySha256"],"requestId":"ipf-query"}]));
    assert_eq!(frame.semantic["decisionHash"], native_input_parser_frame_decision_hash(
        &frame.semantic["trigger"], &frame.semantic["factBindings"]));
    assert_eq!(frame.rows.len(), 1);
    let fact = frame.fact();
    assert_eq!(fact["kind"], "input-parser-output");
    assert_eq!(fact["physicalFacts"][0]["keys"], serde_json::json!([node_id, lease.root_run_id]));
    // Re-verification of the untouched frame succeeds and writes nothing (twice: stable).
    for _ in 0..2 {
        input_parser_frame_zero_write(&db, || frame.verify(&db, &context, &lease).unwrap());
    }
    // A frozen basis is not a dispatch: gate and flags stay closed. (U176 admitted this
    // frozen kind as an advisory Root input; admission is no longer the closed gate.)
    assert_eq!(native_supervision_frame_admission(frame.kind), Ok(()));
    assert!(!native_reserved_role_output_producer_ready("input-parser-output"));
    assert_eq!(native_root_schedule_decision_role_gate(AgentRole::InputParser),
        Err(NATIVE_INPUT_PARSER_DISPATCH_NOT_READY.to_string()));
    for role in [AgentRole::InputParser, AgentRole::BusinessLogic, AgentRole::Authorization, AgentRole::ExternalSurface] {
        // U181 (pin updated, reason: U181 按计划只翻 InputParser): exactly InputParser is true.
        assert_eq!(native_root_role_prepare_implemented(role), role == AgentRole::InputParser, "{role:?}");
    }
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_parser_frame_reverify_fails_closed_on_tampered_fact_foreign_root_or_modified_frame() {
    let (root, context, lease) = input_parser_frame_fixture();
    let query = input_parser_trigger_observe(&context, 1, serde_json::json!({"method":"POST",
        "url":"http://127.0.0.1:9/items?id=7","identity":"anonymous","body":"{\"n\":1}"}), "ipf-query");
    let db = db::open(&context.db_path).unwrap();
    let node_id = input_parser_trigger_node(&db, &query);
    let frame = NativeCoordinatorFrame::input_parser_trigger(&db, &lease, &context.target_dir).unwrap();
    let check = |frame: &NativeCoordinatorFrame, actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease| {
        input_parser_frame_zero_write(&db, || frame.verify(&db, &context, actor))
    };
    assert_eq!(check(&frame, &lease), Ok(()));

    // (1) Broker record bytes tampered after freezing: fail closed, zero writes; restore heals.
    let path = context.target_dir.join(AGENT_HTTP_DIRECTORY).join(&query);
    let original = std::fs::read(&path).unwrap();
    let mut tampered: JsonValue = serde_json::from_slice(&original).unwrap();
    tampered["request"]["url"] = "http://127.0.0.1:9/items".into();
    std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert_eq!(check(&frame, &lease), Err("input_parser_trigger_fact_invalid".to_string()));
    std::fs::write(&path, &original).unwrap();
    assert_eq!(check(&frame, &lease), Ok(()));

    // (2) The frozen evidence row itself changed: the physical hash no longer matches.
    let payload: String = db.query_row("SELECT payload_json FROM agent_evidence_nodes WHERE id=?1",
        [&node_id], |r| r.get(0)).unwrap();
    db.execute("UPDATE agent_evidence_nodes SET payload_json=json_set(payload_json,'$.status',500) WHERE id=?1",
        [&node_id]).unwrap();
    assert_eq!(check(&frame, &lease), Err("root_frame_original_fact_changed".to_string()));
    db.execute("UPDATE agent_evidence_nodes SET payload_json=?2 WHERE id=?1", params![node_id, payload]).unwrap();
    assert_eq!(check(&frame, &lease), Ok(()));

    // (3) The fact re-scoped to another existing run (FK-valid, not this Root): it is no
    // longer this Root's fact.
    let other_run = context.run.as_ref().unwrap().run_id.clone();
    assert_ne!(other_run, lease.root_run_id);
    db.execute("UPDATE agent_evidence_nodes SET root_run_id=?2 WHERE id=?1", params![node_id, other_run]).unwrap();
    assert_eq!(check(&frame, &lease), Err("root_frame_physical_fact_missing".to_string()));
    db.execute("UPDATE agent_evidence_nodes SET root_run_id=?2 WHERE id=?1",
        params![node_id, lease.root_run_id]).unwrap();
    assert_eq!(check(&frame, &lease), Ok(()));

    // (4) A foreign Root / scan / target actor can never adopt this frame.
    for mutate in [
        (|l: &mut crate::agent_runtime::multi_agent::lease::CoordinatorLease| l.root_run_id = "foreign-root".into())
            as fn(&mut crate::agent_runtime::multi_agent::lease::CoordinatorLease),
        |l| l.scan_id = "foreign-scan".into(),
        |l| l.target_key = "foreign-target".into(),
    ] {
        let mut foreign = lease.clone();
        mutate(&mut foreign);
        assert_eq!(check(&frame, &foreign), Err("root_input_parser_frame_scope_invalid".to_string()));
        // Nor can a foreign Root freeze one from this Root's facts.
        if foreign.root_run_id != lease.root_run_id {
            assert_eq!(input_parser_frame_zero_write(&db, || {
                NativeCoordinatorFrame::input_parser_trigger(&db, &foreign, &context.target_dir).err()
            }).as_deref(), Some("root_input_parser_trigger_not_eligible"));
        }
    }

    // (5) Any modification of the frozen frame itself fails closed.
    let modified = |edit: &dyn Fn(&mut NativeCoordinatorFrame)| {
        let mut copy = frame.clone();
        edit(&mut copy);
        check(&copy, &lease)
    };
    let changed = Err("root_input_parser_trigger_changed".to_string());
    let binding = Err("root_input_parser_frame_binding_changed".to_string());
    let scope = Err("root_input_parser_frame_scope_invalid".to_string());
    assert_eq!(modified(&|f| f.semantic["trigger"]["reasonCodes"] = serde_json::json!(["observed_query_parameter"])), changed);
    assert_eq!(modified(&|f| f.semantic["trigger"]["contractKeys"] = serde_json::json!(["forged"])), changed);
    assert_eq!(modified(&|f| f.semantic["factBindings"][0]["recordSha256"] = "0".repeat(64).into()), binding);
    assert_eq!(modified(&|f| f.semantic["factBindings"][0]["requestId"] = "forged".into()), binding);
    assert_eq!(modified(&|f| f.semantic["decisionHash"] = "forged".into()), binding);
    assert_eq!(modified(&|f| f.semantic["targetRequestsGranted"] = 1.into()), scope);
    assert_eq!(modified(&|f| f.semantic["dispatchGranted"] = true.into()), scope);
    assert_eq!(modified(&|f| f.semantic["role"] = "business_logic".into()), scope);
    assert_eq!(modified(&|f| f.semantic["rootRunId"] = "foreign-root".into()), scope);
    assert_eq!(modified(&|f| { f.rows.pop(); }), scope);
    assert_eq!(modified(&|f| f.rows[0].hash = "forged".into()),
        Err("root_frame_original_fact_changed".to_string()));
    assert_eq!(modified(&|f| f.rows[0].keys[1] = "foreign-root".into()),
        Err("root_frame_physical_fact_missing".to_string()));
    assert_eq!(check(&frame, &lease), Ok(()));

    // (6) A new controllable fact after freezing makes the frozen basis stale, not silently wider.
    input_parser_trigger_observe(&context, 2, serde_json::json!({"method":"GET",
        "url":"http://127.0.0.1:9/other?q=1","identity":"anonymous"}), "ipf-later");
    assert_eq!(check(&frame, &lease), changed);
    drop(db);
    std::fs::remove_dir_all(root).unwrap();
}

// U175: pin that the InputParser `freeze_bound_basis` clearance stays backed by the real
// re-verification dispatch. U177 (pin updated, reason: the real fixed-grant child mint
// `native_coordinator_mint_input_parser_child` + Reviewer/Root floor wire, proven by
// agent_tests_input_parser_mint.rs) also cleared `scheduled_child_mint` and
// `reviewer_floor_grant_wire`; exactly the other 2 items still remain.
#[test]
fn input_parser_frame_source_pins_reverify_dispatch_and_single_cleared_blocker() {
    let frame_src = include_str!("agent_native/coordinator_frame.rs");
    assert!(frame_src.contains(
        "if self.kind == \"input-parser-output\" {\n            self.verify_input_parser_trigger_source(db, context, actor)?;"
    ), "frame verify must dispatch input-parser-output to its source re-verification");
    let policy = include_str!("agent_native/coordinator_rust_policy.rs");
    let start = policy.find("const NATIVE_INPUT_PARSER_PREPARE_A_BLOCKERS").expect("InputParser blockers const");
    let block = &policy[start..start + policy[start..].find("];").expect("const end")];
    // U178 cleared `readonly_sdk_input_surface`; U179 (pin updated, reason: cleared
    // `zero_write_proof_on_failure` via agent_tests_input_parser_zero_write.rs) cleared
    // the final item, so the InputParser (a) blocker inventory is now empty.
    for cleared in ["freeze_bound_basis", "scheduled_child_mint", "reviewer_floor_grant_wire", "readonly_sdk_input_surface", "zero_write_proof_on_failure"] {
        assert!(!block.contains(cleared), "{cleared} is cleared for InputParser");
    }
    assert_eq!(block.matches('"').count() / 2, 0, "every InputParser (a) blocker is cleared (U175/U177/U178/U179)");
    let mint = include_str!("agent_native/coordinator_input_parser_mint.rs");
    for pin in [
        "const NATIVE_INPUT_PARSER_GRANT_TOKENS: i64 = 4_000;",
        "const NATIVE_INPUT_PARSER_GRANT_REQUESTS: i64 = 1;",
        "const NATIVE_INPUT_PARSER_ROOT_SUPERVISION_FLOOR_TOKENS: i64 = 16_000;",
        "const NATIVE_INPUT_PARSER_ROOT_SUPERVISION_FLOOR_REQUESTS: i64 = 1;",
        "scheduler::prepare_readonly_child_in_transaction(",
    ] {
        assert!(mint.contains(pin), "U177 mint must keep {pin}");
    }
    assert_eq!(NATIVE_ROOT_PREPARE_A_BLOCKERS[0], "freeze_bound_basis");
}
