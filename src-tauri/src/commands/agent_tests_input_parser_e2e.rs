// U180: InputParser wired into the production multi-agent pipeline, dispatched after the
// Executor settles (same place as ClientSide) only when the deterministic trigger is
// eligible and the paid Root tick selects `dispatch:input_parser`.
// U181 (tests changed, reason: U181 按计划只翻 InputParser): the production flag
// `native_root_role_prepare_implemented(InputParser)` is now true, so these tests drive
// the real production gate; no positive dispatch override exists.
// The honest "flag false => no dispatch" negative is kept: it now simulates the flag
// being false through the test-only `InputParserProductionFlagOff` guard (a compile-time
// `false` outside `cfg(test)`) and unit-asserts the pure gate function.

// Full real chain: fresh creator -> paid Root -> paid Mapper -> SDK Executor Broker GET
// `/api/orders?page=1` -> U175 frozen frame -> production hook -> paid Root tick ->
// validated fixed-grant mint -> readonly SDK surface -> delivery, on the flipped flag.
#[test]
fn input_parser_production_dispatches_readonly_child_after_executor_end_to_end() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::budget};
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_production_fixture("ip-e2e");
    let db = db::open(&f.h.db_path).unwrap();

    // Exactly one readonly InputParser child was minted and completed end-to-end; its
    // reservation settled to zero and it carried the fixed 4000/1 grant with no target grant.
    let (state, lane, rt, rr, slice): (String, String, i64, i64, String) = db.query_row(
        "SELECT state,lane,reserved_tokens,reserved_requests,task_slice_json FROM agent_assignments WHERE role='input_parser'",
        [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).unwrap();
    assert_eq!((state.as_str(), lane.as_str(), rt, rr), ("completed", "read_only_analysis", 0, 0));
    let slice: JsonValue = serde_json::from_str(&slice).unwrap();
    // Fixed grant 4000/1, no target grant, and both retained floors (31000/2) frozen in.
    assert_eq!(slice["grant"], json!({"reservedModelTokens":4000,"reservedModelRequests":1,"targetRequestsGranted":0,
        "retainedFloor":{"modelTokens":31000,"modelRequests":2}}));
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE role='input_parser'", [], |r| r.get::<_, i64>(0)).unwrap(), 1, "exactly one InputParser child");
    let child_run: String = db.query_row("SELECT child_run_id FROM agent_assignments WHERE role='input_parser'", [], |r| r.get(0)).unwrap();

    // Its strict readonly assessment was delivered to the coordinator and acknowledged.
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE kind='input_assessment' AND acknowledged_at<>''", [], |r| r.get::<_, i64>(0)).unwrap(), 1, "one delivered input_assessment");

    // The readonly child made no target request: only the Executor's Broker GET hit the site.
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1, "only the Executor Broker GET touched the target");
    assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)", [&child_run], |r| r.get::<_, bool>(0)).unwrap(), "readonly child claims no target request");
    assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=(SELECT id FROM agent_assignments WHERE role='input_parser') AND lane<>'read_only_analysis')", [], |r| r.get::<_, bool>(0)).unwrap(), "no target-touching lane");

    // Retained floors preserved: Reviewer 15000/1 still free after the paid child, and the
    // Reviewer 15000/1 + Root supervision 16000/1 floors (31000/2) are still exactly held.
    budget::root::require_child_capacity(&db, &f.root, 15_000, 1).unwrap();
    assert_eq!(native_coordinator_executor_retained_floor(&db, f.lease()).unwrap(), (31_000, 2));
    native_input_parser_require_headroom(&db, &f.root, 0, 0).unwrap();

    // U181: the production gate (not a test override) opened this dispatch.
    assert!(!input_parser_test_flag_forced_off());
    assert!(input_parser_dispatch_enabled());
    // Exactly one paid InputParser-frame Root tick and exactly one child SDK call.
    let wires = f.h.model_seen.lock().unwrap().clone();
    assert_eq!(wires.iter().filter(|w| w.contains("You are the Root Coordinator") && w.contains("input-parser-output")).count(), 1, "one paid InputParser Root tick");
    let child_wires: Vec<_> = wires.iter().filter(|w| w.contains("你是独立 InputParser 只读输入面专家")).collect();
    assert_eq!(child_wires.len(), 1, "exactly one readonly child SDK call");
    // The child SDK request carried no tools, a bounded output and no query value.
    let body: JsonValue = serde_json::from_str(child_wires[0].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert!(body.get("tools").is_none_or(|v| v == &json!([])), "readonly child gets no tools");
    assert_eq!(body.get("max_tokens").or_else(|| body.get("max_completion_tokens")).and_then(JsonValue::as_u64), Some(512));
    assert!(!child_wires[0].contains("page=1"), "query values stay redacted");
    // No write capability ever: the child lease is exactly the readonly pair and no
    // capability lease of the child stays live.
    let caps: String = db.query_row("SELECT capability_lease_json FROM agent_assignments WHERE role='input_parser'", [], |r| r.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<JsonValue>(&caps).unwrap(), json!(["evidence.read", "mailbox.write"]));
    assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at='')", [&child_run], |r| r.get::<_, bool>(0)).unwrap(), "no live child capability lease after delivery");
    // The delivered output passes the strict schema bound to this lease + paid frame.
    let payload: String = db.query_row("SELECT payload_json FROM agent_messages WHERE kind='input_assessment'", [], |r| r.get(0)).unwrap();
    let payload: JsonValue = serde_json::from_str(&payload).unwrap();
    let decision_hash = slice["frame"]["decisionHash"].as_str().unwrap();
    let task = crate::agent_runtime::multi_agent::input_parser::Task::from_value(&payload["inputParserTask"], f.lease(), decision_hash).unwrap();
    let text = payload["summary"].as_str().unwrap();
    crate::agent_runtime::multi_agent::input_parser::validate_assessment(text, &task).unwrap();
    assert!(!text.contains("confirmed") && !text.contains("candidates"), "no impact claim");

    // U181 (pin updated, reason: U181 按计划只翻 InputParser): only InputParser is true;
    // its (a) blockers stay empty; BL/AZ/ES keep the full shared inventory.
    for role in [AgentRole::InputParser, AgentRole::BusinessLogic, AgentRole::Authorization, AgentRole::ExternalSurface] {
        assert_eq!(native_root_role_prepare_implemented(role), role == AgentRole::InputParser, "{role:?}");
    }
    assert_eq!(native_root_prepare_a_blockers_remaining(AgentRole::InputParser), Some(&[][..]));
}

// Honesty: with the production flag false, the real pipeline reaches the InputParser hook
// but dispatches nothing and spends nothing — exactly the behavior before U181.
// U181 (changed, not deleted; reason: the real flag is now true, so "flag false" can only
// be exercised by simulation): the finish runs under the test-only
// `InputParserProductionFlagOff` guard, which makes the gate see the flag as false (a
// compile-time `false` outside `cfg(test)`, so production is exactly the flag), and the
// Only the negative hook closes the actual production gate.
#[test]
fn input_parser_production_flag_false_dispatches_nothing_end_to_end() {
    let _real = RealSpecialistTransport::enter();
    let mut f = input_parser_root_fixture("ip-e2e-off", &["dispatch:input_parser"]);
    // The real flag is true (U181); only the simulation makes the gate see false.
    assert!(native_root_role_prepare_implemented(crate::agent_runtime::contract::AgentRole::InputParser));
    assert!(input_parser_dispatch_enabled(), "the flipped production gate is open by default");
    let _off = InputParserProductionFlagOff::enter();
    assert!(!input_parser_dispatch_enabled(), "simulated flag false closes the gate");
    let outcome = AgentTargetOutcome::incomplete("known observed input; no impact");
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1, "one real Executor Broker GET, none from a child");
    // The InputParser hook ran inside finish_execution but gated out at the honest flag
    // check: no child minted, no readonly assessment, and no paid InputParser Root tick.
    assert_eq!(f.input_parser_rows(&db), 0, "flag false: production dispatches no InputParser child");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE kind='input_assessment'", [], |r| r.get::<_, i64>(0)).unwrap(), 0, "no readonly assessment delivered");
    assert!(!f.h.model_seen.lock().unwrap().iter().any(|w| w.contains("input-parser-output")), "no paid InputParser Root tick when the flag is false");
    // And a second, direct call with the flag still false is a full-table zero-write no-op
    // (the honest gate returns before opening the database for any work).
    let before = super::tests::application_table_snapshot(&db);
    multi_agent_input_parser(&f.h.context, f.session.as_ref().unwrap()).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before, "flag-false production hook wrote to the database");
}

// Negative: the deterministic trigger is ineligible (no observed controllable input).
// The production entry must dispatch nothing, pay no Root tick, and write nothing —
// proven by full-table snapshot equality across the whole hook.
#[test]
fn input_parser_production_ineligible_trigger_dispatches_nothing_zero_write() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("ip-u180-ineligible", &["dispatch:input_parser"]);
    let db = db::open(&f.h.db_path).unwrap();
    // Remove every observed request fact: the clean "not triggered" case (not a tamper).
    db.execute("DELETE FROM agent_evidence_nodes WHERE root_run_id=?1 AND kind='request_record'", [&f.root]).unwrap();
    assert!(!native_input_parser_trigger_decision(&db, &f.root, &f.h.context.target_dir).unwrap().eligible, "predicate must be ineligible");
    let before = super::tests::application_table_snapshot(&db);
    // U181: the production entry on the flipped flag, no test override.
    assert!(input_parser_dispatch_enabled());
    multi_agent_input_parser(&f.h.context, f.session.as_ref().unwrap()).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before, "ineligible trigger wrote to the database");
    assert_eq!(f.input_parser_rows(&db), 0, "ineligible trigger minted no child");
    assert!(!f.h.model_seen.lock().unwrap().iter().any(|w| w.contains("input-parser-output")), "ineligible trigger paid no Root tick");
}

// Negative: the cited Broker record is tampered after it was frozen. The production
// entry fails closed at the verified-fact predicate — before any paid tick or mint —
// and leaves every application table byte-identical (full-table zero write).
#[test]
fn input_parser_production_tampered_fact_fails_closed_zero_write() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("ip-u180-tamper", &["dispatch:input_parser"]);
    let db = db::open(&f.h.db_path).unwrap();
    let (_node, path) = f.record(&db);
    let original = std::fs::read(&path).unwrap();
    let mut tampered: JsonValue = serde_json::from_slice(&original).unwrap();
    tampered["request"]["url"] = format!("{}/api/orders?page=2", f.h.context.target_url).into();
    std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    // U181: the production entry on the flipped flag, no test override.
    assert!(input_parser_dispatch_enabled());
    let err = multi_agent_input_parser(&f.h.context, f.session.as_ref().unwrap()).unwrap_err();
    assert_eq!(err, "input_parser_trigger_fact_invalid");
    assert!(super::tests::application_table_snapshot(&db) == before, "tampered fact wrote to the database");
    assert_eq!(f.input_parser_rows(&db), 0, "tamper minted no child");
    assert!(!f.h.model_seen.lock().unwrap().iter().any(|w| w.contains("input-parser-output")), "tamper paid no Root tick");
}

// Negative: the Root is above both retained floors (15000/1 + 16000/1 = 31000/2) but
// cannot cover floor + the fixed 4000/1 grant. Driven through the production entry, the
// mint step refuses with the budget-unavailable code, mints no child, and leaves both
// retained floors intact.
#[test]
fn input_parser_production_insufficient_balance_mints_nothing_floors_kept() {
    let _real = RealSpecialistTransport::enter();
    // Same frozen-limit calibration U177 proves at the mint: after the paid tick the Root
    // is above both floors (31000/2) but short of floor + the 4000/1 grant.
    let f = input_parser_root_fixture_original_v5_limits("ip-u180-balance", &["dispatch:input_parser"], (INPUT_PARSER_MINT_TOKEN_EDGE - 1, 20));
    input_parser_mint_settle_executor(&f);
    let db = db::open(&f.h.db_path).unwrap();
    // U181: the production entry on the flipped flag, no test override.
    assert!(input_parser_dispatch_enabled());
    let err = multi_agent_input_parser(&f.h.context, f.session.as_ref().unwrap()).unwrap_err();
    assert_eq!(err, NATIVE_INPUT_PARSER_BUDGET_UNAVAILABLE);
    assert_eq!(f.input_parser_rows(&db), 0, "insufficient balance minted no child");
    // Both retained floors (31000/2) survive the refusal (15000/1 Reviewer + 16000/1 Root).
    native_input_parser_require_headroom(&db, &f.root, 0, 0).unwrap();
}

// U181 (failure zero-write proof on the FLIPPED production path): with the real flag
// true, an SDK provider failure in the dispatched readonly child still publishes no
// result, keeps no live capability lease, never completes the child and never lowers
// the Reviewer 15000/1 + Root supervision 16000/1 floors (31000/2). The error
// propagates out of the production entry (`?`), exactly as U179 proves on the readonly
// driver. The child is driven through the production entry `multi_agent_input_parser`
// (not the readonly helper directly), on the flipped flag, no test override.
#[test]
fn input_parser_production_sdk_failure_publishes_no_result_floors_kept() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    // The child provider answers 503 for the readonly InputParser child.
    let f = input_parser_root_fixture_reply("ip-u181-sdk-fail", || (503, r#"{"error":{"message":"boom"}}"#.to_string()));
    // U180 position: settle the Executor exactly as the production finish hook does first.
    input_parser_mint_settle_executor(&f);
    let db = db::open(&f.h.db_path).unwrap();
    let evidence_before = db.query_row("SELECT count(*) FROM agent_evidence_nodes", [], |r| r.get::<_, i64>(0)).unwrap();
    budget::root::require_child_capacity(&db, &f.root, 15_000, 1).unwrap();
    assert_eq!(native_coordinator_executor_retained_floor(&db, f.lease()).unwrap(), (31_000, 2));
    // Production entry on the flipped flag — no test override.
    assert!(native_root_role_prepare_implemented(crate::agent_runtime::contract::AgentRole::InputParser));
    assert!(input_parser_dispatch_enabled());
    let err = multi_agent_input_parser(&f.h.context, f.session.as_ref().unwrap()).unwrap_err();
    assert!(!err.is_empty(), "SDK failure surfaces an error out of the production entry");
    // No result published anywhere; the paid Root tick happened but the child failed.
    assert_eq!(db.query_row("SELECT count(*) FROM agent_evidence_nodes", [], |r| r.get::<_, i64>(0)).unwrap(), evidence_before, "no evidence on SDK failure");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE kind='input_assessment'", [], |r| r.get::<_, i64>(0)).unwrap(), 0, "no result message");
    let child_run: Option<String> = db.query_row("SELECT child_run_id FROM agent_assignments WHERE role='input_parser'", [], |r| r.get(0)).optional().unwrap();
    if let Some(child_run) = child_run.as_deref() {
        assert_ne!(db.query_row("SELECT state FROM agent_assignments WHERE role='input_parser'", [], |r| r.get::<_, String>(0)).unwrap(), "completed", "a failed child never completes");
        assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at='')", [&child_run], |r| r.get::<_, bool>(0)).unwrap(), "a failed child keeps no live capability lease");
        assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)", [&child_run], |r| r.get::<_, bool>(0)).unwrap(), "no target claim");
    }
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1, "no new target request on failure");
    // Retained floors survive the failed child.
    budget::root::require_child_capacity(&db, &f.root, 15_000, 1).unwrap();
    assert_eq!(native_coordinator_executor_retained_floor(&db, f.lease()).unwrap(), (31_000, 2));
}
