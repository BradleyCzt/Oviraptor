#[test]
fn input_parser_root_feedback_actual_paid_root_tick_selects_dispatch_from_frozen_frame_without_mint() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::budget};
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture_original_v5_limits("ip-root-feedback", &["dispatch:input_parser"], (60000, 20));
    let db = db::open(&f.h.db_path).unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1, "one real executor Broker GET");
    let (calls, publications, before) = (f.calls(), root_tick_count(&db, &f.root, "publication"), f.requests(&db));
    assert_eq!(before.indeterminate, 0);
    let decision = f.feedback().unwrap();
    // Exactly one new paid original-Root SDK call, published once, settled exactly.
    assert!(!decision.replayed);
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(root_tick_count(&db, &f.root, "publication"), publications + 1);
    let after = f.requests(&db);
    assert_eq!((after.consumed, after.reserved, after.indeterminate), (before.consumed + 1, before.reserved, 0));
    assert!(native_root_paid_suggestion_selected(&decision, NATIVE_INPUT_PARSER_DISPATCH_STEP));
    // The Root saw exactly the U175 frozen frame: verified trigger, zero grants.
    let input = input_parser_root_wire_input(&f);
    assert_eq!(input["basis"]["phase"], "input-parser-output");
    assert_eq!(input["permittedSuggestion"], "dispatch:input_parser");
    let semantic = &input["basis"]["changedFact"]["semantic"];
    assert_eq!(semantic["role"], "input_parser");
    assert_eq!(semantic["rootRunId"], f.root.as_str());
    assert_eq!((semantic["targetRequestsGranted"].clone(), semantic["dispatchGranted"].clone()), (json!(0), json!(false)));
    assert_eq!(semantic["trigger"]["eligible"], true);
    assert_eq!(semantic["trigger"]["reasonCodes"], json!(["observed_query_parameter"]));
    let (node, _) = f.record(&db);
    assert_eq!(semantic["trigger"]["evidenceRefs"], json!([node]));
    let frame = NativeCoordinatorFrame::input_parser_trigger(&db, f.lease(), &f.h.context.target_dir).unwrap();
    assert_eq!(input["basis"]["changedFact"], frame.fact());
    assert_eq!(NativeCoordinatorTrigger::classify(&frame).unwrap().name(), "input_parser_trigger");
    // Financial floors: Reviewer 15000/1 still free, Root supervision floor unchanged.
    budget::root::require_child_capacity(&db, &f.root, 15_000, 1).unwrap();
    assert_eq!(native_coordinator_executor_retained_floor(&db, f.lease()).unwrap(), (31_000, 2));
    // A paid suggestion is not a dispatch: no child, no policy acceptance.
    assert_eq!(f.input_parser_rows(&db), 0);
    let root = native_coordinator_root_context(&f.h.context, f.lease());
    for role in AgentRole::ALL {
        assert_eq!(native_coordinator_rust_dispatch_policy(&root, &frame, &decision, *role, 512, 1,
            [Some(60000), Some(20), Some(32)]).unwrap_err(), "root_trigger_role_conflict", "{role:?}");
    }
    let physical = web_mode_test_rows(&db);
    // U181 (pin updated, reason: U181 按计划只翻 InputParser): the flipped gate now reaches
    // the validated mint, but under the original v5 contract the Executor still holds its running
    // reservation (not yet settled by `multi_agent_finish_execution`), so the mint
    // refuses with the budget code and writes nothing. With the flag simulated false
    // the gate is still honest not_ready. The generic schedule-decision gate is not the
    // InputParser dispatch path (only the frozen-frame hook is) and stays not_ready.
    assert_eq!(native_coordinator_prepare_input_parser(&root, f.lease(), &decision, &json!({})).unwrap_err(),
        NATIVE_INPUT_PARSER_BUDGET_UNAVAILABLE);
    web_mode_assert_rows(&db, &physical);
    {
        let _off = InputParserProductionFlagOff::enter();
        assert_eq!(native_coordinator_prepare_input_parser(&root, f.lease(), &decision, &json!({})).unwrap_err(),
            NATIVE_INPUT_PARSER_DISPATCH_NOT_READY);
    }
    web_mode_assert_rows(&db, &physical);
    assert_eq!(native_root_schedule_decision_role_gate(AgentRole::InputParser).unwrap_err(),
        NATIVE_INPUT_PARSER_DISPATCH_NOT_READY);
    for role in [AgentRole::InputParser, AgentRole::BusinessLogic, AgentRole::Authorization, AgentRole::ExternalSurface] {
        // U181 (pin updated, reason: U181 按计划只翻 InputParser): exactly InputParser is true.
        assert_eq!(native_root_role_prepare_implemented(role), role == AgentRole::InputParser, "{role:?}");
    }
    // Same frozen fact: replay, zero SDK, zero writes.
    let again = f.feedback().unwrap();
    assert!(again.replayed);
    assert_eq!(again.event_sequence, decision.event_sequence);
    web_mode_assert_rows(&db, &physical);
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(f.input_parser_rows(&db), 0);
}

#[test]
fn input_parser_root_feedback_rejects_unbounded_paid_suggestion_and_keeps_exact_fee() {
    for suggestions in [&["dispatch:web_executor"][..], &["dispatch:input_parser", "dispatch:business_logic"][..]] {
        let _real = RealSpecialistTransport::enter();
        let f = input_parser_root_fixture("ip-root-forbidden", Box::leak(Box::new(suggestions)));
        let db = db::open(&f.h.db_path).unwrap();
        let (calls, publications, before) = (f.calls(), root_tick_count(&db, &f.root, "publication"), f.requests(&db));
        let executors: i64 = db.query_row("SELECT count(*) FROM agent_assignments WHERE role='web_executor'", [], |r| r.get(0)).unwrap();
        assert_eq!(f.feedback().err().unwrap(), "root_input_parser_step_not_bounded", "{suggestions:?}");
        // The provider bill is real and kept exactly; nothing else follows.
        assert_eq!(f.calls(), calls + 1);
        let after = f.requests(&db);
        assert_eq!((after.consumed, after.reserved, after.indeterminate), (before.consumed + 1, before.reserved, 0));
        // The paid advisory is an auditable Root publication; Rust still refuses to act on it.
        assert_eq!(root_tick_count(&db, &f.root, "publication"), publications + 1);
        assert_eq!(f.input_parser_rows(&db), 0);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE role='web_executor'", [], |r| r.get::<_, i64>(0)).unwrap(), executors);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
        crate::agent_runtime::multi_agent::budget::root::require_child_capacity(&db, &f.root, 15_000, 1).unwrap();
        // Re-presenting the same paid advisory stays refused without another bill.
        let physical = web_mode_test_rows(&db);
        assert_eq!(f.feedback().err().unwrap(), "root_input_parser_step_not_bounded");
        web_mode_assert_rows(&db, &physical);
        assert_eq!(f.calls(), calls + 1);
    }
}

#[test]
fn input_parser_root_feedback_tampered_or_unadmitted_frames_are_refused_before_sdk_with_zero_writes() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("ip-root-tamper", &["dispatch:input_parser"]);
    let db = db::open(&f.h.db_path).unwrap();
    let root = native_coordinator_root_context(&f.h.context, f.lease());
    let calls = f.calls();
    let refused = |result: Result<NativeCoordinatorTickReceipt, String>| {
        let error = result.err().expect("must be refused");
        assert_eq!(f.calls(), calls, "no Root SDK for {error}");
        error
    };
    let physical = web_mode_test_rows(&db);
    // (1) Broker record bytes tampered after the executor wrote them.
    let (node, path) = f.record(&db);
    let original = std::fs::read(&path).unwrap();
    let mut tampered: JsonValue = serde_json::from_slice(&original).unwrap();
    tampered["request"]["url"] = format!("{}/api/orders", f.h.context.target_url).into();
    std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    assert_eq!(refused(f.feedback()), "input_parser_trigger_fact_invalid");
    web_mode_assert_rows(&db, &physical);
    std::fs::write(&path, &original).unwrap();
    // (2) The observed fact re-scoped away from this Root (FK-valid run): no frame at all.
    let mapper_run = f.session.as_ref().unwrap().mapper.run_id.clone();
    db.execute("UPDATE agent_evidence_nodes SET root_run_id=?2 WHERE id=?1", params![node, mapper_run]).unwrap();
    let physical_moved = web_mode_test_rows(&db);
    assert_eq!(refused(f.feedback()), "root_input_parser_trigger_not_eligible");
    web_mode_assert_rows(&db, &physical_moved);
    db.execute("UPDATE agent_evidence_nodes SET root_run_id=?2 WHERE id=?1", params![node, f.root]).unwrap();
    web_mode_assert_rows(&db, &physical);
    // (3) A frozen frame modified in memory never reaches the paid Root.
    let frame = NativeCoordinatorFrame::input_parser_trigger(&db, f.lease(), &f.h.context.target_dir).unwrap();
    type InputParserFrameMutation<'a> = (&'a dyn Fn(&mut NativeCoordinatorFrame), &'a str);
    let edits: [InputParserFrameMutation<'_>; 6] = [
        (&|f| f.semantic["decisionHash"] = "forged".into(), "root_input_parser_frame_binding_changed"),
        (&|f| f.semantic["factBindings"][0]["recordSha256"] = "0".repeat(64).into(), "root_input_parser_frame_binding_changed"),
        (&|f| f.semantic["trigger"]["reasonCodes"] = json!(["observed_request_body"]), "root_input_parser_trigger_changed"),
        (&|f| f.semantic["dispatchGranted"] = true.into(), "root_input_parser_frame_scope_invalid"),
        (&|f| f.semantic["targetRequestsGranted"] = 1.into(), "root_input_parser_frame_scope_invalid"),
        (&|f| f.rows[0].hash = "forged".into(), "root_frame_original_fact_changed"),
    ];
    for (edit, code) in edits {
        let mut copy = frame.clone();
        edit(&mut copy);
        assert_eq!(refused(native_coordinator_tick_for_frame(&root, f.lease(), &copy)), code);
        web_mode_assert_rows(&db, &physical);
    }
    // An unbacked InputParser skeleton (admitted kind, no frozen fact) is refused too.
    let bare = NativeCoordinatorFrame { kind: "input-parser-output", semantic: json!({"role":"input_parser"}),
        rows: Vec::new(), review_refs: None, review_meta: None };
    assert_eq!(refused(native_coordinator_tick_for_frame(&root, f.lease(), &bare)), "root_input_parser_frame_scope_invalid");
    // Admitted output kinds require their original signed source; a display
    // skeleton cannot acquire an SDK call, grant or publication.
    for (kind,role,code) in [
        ("business-logic-output","business_logic","root_business_logic_output_frame_invalid"),
        ("authorization-output","authorization","root_authorization_output_frame_invalid"),
        ("external-surface-output","external_surface","root_external_surface_output_frame_invalid"),
    ] {
        let frame=NativeCoordinatorFrame {kind,semantic:json!({"role":role,"recovery":"not_ready"}),rows:Vec::new(),review_refs:None,review_meta:None};
        assert_eq!(refused(native_coordinator_tick_for_frame(&root,f.lease(),&frame)),code);
        web_mode_assert_rows(&db,&physical);
    }
    assert_eq!(f.input_parser_rows(&db), 0);
    // Restored original fact: the real frame is admitted and paid exactly once.
    let decision = f.feedback().unwrap();
    assert!(!decision.replayed);
    assert_eq!(f.calls(), calls + 1);
    assert!(native_root_paid_suggestion_selected(&decision, NATIVE_INPUT_PARSER_DISPATCH_STEP));
}

#[test]
fn input_parser_root_feedback_fact_change_during_paid_tick_withholds_publication_and_replays_without_new_fee() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("ip-root-midflight", &["dispatch:input_parser"]);
    let db = db::open(&f.h.db_path).unwrap();
    let (node, _) = f.record(&db);
    let saved = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let original = saved.clone();
    let changed = node.clone();
    *f.boundary.lock().unwrap() = Some(Box::new(move |db: &rusqlite::Connection| {
        let payload: String = db.query_row("SELECT payload_json FROM agent_evidence_nodes WHERE id=?1", [&changed], |r| r.get(0)).unwrap();
        db.execute("UPDATE agent_evidence_nodes SET payload_json=json_set(payload_json,'$.status',500) WHERE id=?1", [&changed]).unwrap();
        *original.lock().unwrap() = Some(payload);
    }));
    let (calls, publications, before) = (f.calls(), root_tick_count(&db, &f.root, "publication"), f.requests(&db));
    let error = f.feedback().err().unwrap();
    assert_eq!(error, "root_frame_original_fact_changed");
    assert_eq!(f.calls(), calls + 1, "the paid call really happened");
    assert_eq!(root_tick_count(&db, &f.root, "publication"), publications, "publication withheld");
    let after = f.requests(&db);
    assert_eq!((after.consumed, after.reserved, after.indeterminate), (before.consumed + 1, before.reserved, 0));
    assert_eq!(f.input_parser_rows(&db), 0);
    crate::agent_runtime::multi_agent::budget::root::require_child_capacity(&db, &f.root, 15_000, 1).unwrap();
    // Still changed: refused with no SDK and no write.
    let physical = web_mode_test_rows(&db);
    assert!(f.feedback().is_err());
    web_mode_assert_rows(&db, &physical);
    assert_eq!(f.calls(), calls + 1);
    // Restore the original fact: the already-paid decision is recovered locally.
    let payload = saved.lock().unwrap().take().unwrap();
    db.execute("UPDATE agent_evidence_nodes SET payload_json=?2 WHERE id=?1", params![node, payload]).unwrap();
    let costs = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db, "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid", []).unwrap();
    let decision = f.feedback().unwrap();
    assert!(decision.replayed);
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(root_tick_count(&db, &f.root, "publication"), publications + 1);
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db, "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid", []).unwrap() == costs);
    assert!(native_root_paid_suggestion_selected(&decision, NATIVE_INPUT_PARSER_DISPATCH_STEP));
    assert_eq!(f.input_parser_rows(&db), 0);
}

// U176: admission inventory — exactly one role frame (input-parser-output) moved to
// wired with its own trigger name; BL/AZ/ES unchanged and still named not_ready.
#[test]
fn input_parser_root_feedback_admits_only_input_parser_with_distinct_trigger_name() {
    assert!(NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&"input-parser-output"));
    assert!(!NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS.contains(&"input-parser-output"));
    assert_eq!(NATIVE_RESERVED_ROLE_OUTPUT_FRAME_KINDS,
        &["business-logic-output", "authorization-output", "external-surface-output"]);
    assert_eq!(native_supervision_frame_admission("input-parser-output"), Ok(()));
    let bare = NativeCoordinatorFrame { kind: "input-parser-output", semantic: json!({}),
        rows: Vec::new(), review_refs: None, review_meta: None };
    let trigger = NativeCoordinatorTrigger::classify(&bare).unwrap();
    assert_eq!(trigger.name(), "input_parser_trigger");
    assert_ne!(trigger.name(), "child_output_envelope");
    for kind in ["business-logic-output", "authorization-output", "external-surface-output"] {
        let code = format!("root_trigger_{}_not_ready", kind.replace('-', "_"));
        assert_eq!(native_supervision_frame_admission(kind).unwrap_err(), code);
        assert!(!native_reserved_role_output_producer_ready(kind));
    }
    // The display-only skeleton can no longer stand in for the wired kind.
    assert_eq!(native_input_parser_output_frame_skeleton("ip").err().as_deref(),
        Some("root_trigger_skeleton_kind_invalid"));
    let hook = include_str!("agent_native/coordinator_input_parser_hook.rs");
    for pin in [
        "NativeCoordinatorFrame::input_parser_trigger(&tx, actor, &root.target_dir)?",
        "items.iter().all(|item| item.as_str() == Some(frame.step()))",
        "return Err(\"root_input_parser_step_not_bounded\".into());",
        "decision.tick.require_executable(&tx)?;",
        "return Err(\"root_input_parser_publication_changed\".into());",
    ] {
        assert!(hook.contains(pin), "missing {pin}");
    }
    assert_eq!(hook.matches("frame.verify(&tx, &root, actor)?;").count(), 2, "verify before and after the paid tick");
    for forbidden in ["schedule_child", "prepare_readonly_child", "native_coordinator_prepare_input_parser"] {
        assert!(!hook.contains(forbidden), "U176 must not mint: {forbidden}");
    }
}
