// U177: actual fresh creator -> paid Root -> paid Mapper -> SDK executor Broker GET
// -> U175 frozen InputParser frame -> U176 paid Root `dispatch:input_parser` ->
// real InputParser child mint with the fixed 4000/1 grant, behind Reviewer 15000/1
// + Root supervision 16000/1. No SDK call (U178); U181 flipped only InputParser's flag.
// Calibrated on this actual flow (settled Executor + paid InputParser tick consume
// 500 tokens / 13 requests): the frozen hard limit at which remaining == floor + grant.
const INPUT_PARSER_MINT_TOKEN_EDGE: i64 = 35_500;
const INPUT_PARSER_MINT_REQUEST_EDGE: i64 = 16;
fn input_parser_mint_remaining(db: &rusqlite::Connection, root: &str) -> (i64, i64) {
    let (tokens, requests) =
        crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(db, root).unwrap();
    (tokens.unwrap(), requests.unwrap())
}
fn input_parser_mint_ledger(db: &rusqlite::Connection, root: &str) -> [i64; 4] {
    db.query_row("SELECT reserved_tokens,reserved_requests,spent_tokens,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [root], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?])).unwrap()
}
// U180 position: the Executor has finished and its reservation is settled exactly
// as `multi_agent_finish_execution` does first (receipt-bound usage, same TX shape).
fn input_parser_mint_settle_executor(f: &InputParserRootFixture) {
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate).unwrap();
    let usage = crate::agent_runtime::multi_agent::budget::web_lifetime::child_usage_original(
        &tx, &session.lease, &session.executor.assignment_id, &session.executor.run_id).unwrap();
    settle_child_usage_in_transaction(&tx, &session.lease, &session.executor, &usage).unwrap();
    tx.commit().unwrap();
}
fn input_parser_mint(f: &InputParserRootFixture, decision: &NativeCoordinatorTickReceipt)
    -> Result<crate::agent_runtime::multi_agent::scheduler::ScheduledChild, String> {
    let root = native_coordinator_root_context(&f.h.context, f.lease());
    native_coordinator_mint_input_parser_child(&root, f.lease(), decision)
}

#[test]
fn input_parser_mint_paid_frame_bound_decision_mints_one_child_with_fixed_grant_and_both_floors_kept() {
    use crate::agent_runtime::contract::AgentRole;
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("ip-mint", &["dispatch:input_parser"]);
    let db = db::open(&f.h.db_path).unwrap();
    input_parser_mint_settle_executor(&f);
    let decision = f.feedback().unwrap();
    let (tokens, requests) = input_parser_mint_remaining(&db, &f.root);
    assert!(tokens >= 35_000 && requests >= 3, "fixture headroom {tokens}/{requests}");
    let ledger = input_parser_mint_ledger(&db, &f.root);
    let (calls, paid) = (f.calls(), f.requests(&db));
    let child = input_parser_mint(&f, &decision).unwrap();
    assert_eq!(child.role, AgentRole::InputParser);
    // Exact accounting: the reservation is the fixed grant, nothing is spent or billed.
    assert_eq!(input_parser_mint_remaining(&db, &f.root), (tokens - 4_000, requests - 1));
    assert_eq!(input_parser_mint_ledger(&db, &f.root),
        [ledger[0] + 4_000, ledger[1] + 1, ledger[2], ledger[3]]);
    let after = f.requests(&db);
    assert_eq!((after.consumed, after.reserved, after.indeterminate), (paid.consumed, paid.reserved, 0));
    assert_eq!(f.calls(), calls, "mint makes no SDK call");
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1, "no target request");
    // Both floors remain intact after the reservation.
    crate::agent_runtime::multi_agent::budget::root::require_child_capacity(&db, &f.root, 31_000, 2).unwrap();
    native_input_parser_require_headroom(&db, &f.root, 0, 0).unwrap();
    // One real, running, readonly child bound to this Root, decision and frame.
    let (role, lane, state, trigger, reserved, slice, caps): (String, String, String, String, (i64, i64), String, String) =
        db.query_row("SELECT role,lane,state,trigger_code,reserved_tokens,reserved_requests,task_slice_json,capability_lease_json
             FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2", params![child.assignment_id, f.root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, (r.get(4)?, r.get(5)?), r.get(6)?, r.get(7)?))).unwrap();
    assert_eq!((role.as_str(), lane.as_str(), state.as_str(), trigger.as_str(), reserved),
        ("input_parser", "read_only_analysis", "running", "input_parser_trigger", (4_000, 1)));
    let caps: Vec<String> = serde_json::from_str(&caps).unwrap();
    assert_eq!(caps, ["evidence.read", "mailbox.write"]);
    let (run_role, run_status, run_root): (String, String, String) = db.query_row(
        "SELECT role,status,root_run_id FROM agent_runs WHERE id=?1 AND assignment_id=?2",
        params![child.run_id, child.assignment_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!((run_role.as_str(), run_status.as_str(), run_root.as_str()), ("input_parser", "running", f.root.as_str()));
    let slice: JsonValue = serde_json::from_str(&slice).unwrap();
    let frame = NativeCoordinatorFrame::input_parser_trigger(&db, f.lease(), &f.h.context.target_dir).unwrap();
    assert_eq!(slice["rootDecision"]["eventSequence"], decision.event_sequence);
    assert_eq!(slice["rootDecision"]["step"], "dispatch:input_parser");
    assert_eq!(slice["frame"]["decisionHash"], frame.semantic["decisionHash"]);
    assert_eq!(slice["grant"], json!({"reservedModelTokens":4000,"reservedModelRequests":1,"targetRequestsGranted":0,
        "retainedFloor":{"modelTokens":31000,"modelRequests":2}}));
    assert_eq!(f.input_parser_rows(&db), 2, "exactly one assignment + one run");
    // Same paid decision: replay returns the same child with zero writes and no bill.
    let physical = web_mode_test_rows(&db);
    assert!(input_parser_mint(&f, &decision).unwrap() == child);
    web_mode_assert_rows(&db, &physical);
    // U181 (pin updated, reason: U181 按计划只翻 InputParser): the flipped gate routes the
    // same paid decision to this mint, so it is an exact zero-write replay of the same
    // child; with the flag simulated false the gate is still honest not_ready, zero-write.
    let root = native_coordinator_root_context(&f.h.context, f.lease());
    assert!(native_coordinator_prepare_input_parser(&root, f.lease(), &decision, &json!({})).unwrap() == child);
    web_mode_assert_rows(&db, &physical);
    {
        let _off = InputParserProductionFlagOff::enter();
        assert_eq!(native_coordinator_prepare_input_parser(&root, f.lease(), &decision, &json!({})).unwrap_err(),
            NATIVE_INPUT_PARSER_DISPATCH_NOT_READY);
    }
    web_mode_assert_rows(&db, &physical);
    for role in [AgentRole::InputParser, AgentRole::BusinessLogic, AgentRole::Authorization, AgentRole::ExternalSurface] {
        // U181 (pin updated, reason: U181 按计划只翻 InputParser): exactly InputParser is true.
        assert_eq!(native_root_role_prepare_implemented(role), role == AgentRole::InputParser, "{role:?}");
    }
    // U179 (pin updated, reason: cleared `zero_write_proof_on_failure`) empties the
    // InputParser (a) blocker inventory; prepare still stays not_ready (no flag flip).
    assert_eq!(native_root_prepare_a_blockers_remaining(AgentRole::InputParser),
        Some(&[][..]));
}

#[test]
fn input_parser_mint_headroom_requires_both_floors_plus_grant_to_the_exact_unit() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    let (port, _seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (200, "application/json", changed_fact_response(&request, true))
    }));
    let endpoint = format!("http://127.0.0.1:{port}/v1");
    // A Root whose whole balance is below one grant: unavailable, and nothing minted.
    let f = root_tick_fixture_protocol_limits("ip-mint-headroom-small", &endpoint, true, (3_000, 20));
    let db = db::open(&f.context.db_path).unwrap();
    let physical = web_mode_test_rows(&db);
    for (tokens, requests) in [(4_000, 1), (0, 0)] {
        assert_eq!(native_input_parser_require_headroom(&db, &f.actor.root_run_id, tokens, requests).unwrap_err(),
            NATIVE_INPUT_PARSER_BUDGET_UNAVAILABLE);
    }
    web_mode_assert_rows(&db, &physical);
    // Exact boundary on a real ledger: floor + grant passes, one unit less fails.
    let g = root_tick_fixture_protocol_limits("ip-mint-headroom-edge", &endpoint, true, (60_000, 20));
    let db = db::open(&g.context.db_path).unwrap();
    let root = g.actor.root_run_id.clone();
    scheduler::prepare_readonly_child(&db, &g.actor, AgentRole::InputParser, "validated_input_surface_ready",
        &json!({"objective":"readonly input inventory, no target access"}), 4_000).unwrap();
    let (tokens, requests) = input_parser_mint_remaining(&db, &root);
    assert_eq!(native_input_parser_retained_floor(), (31_000, 2));
    native_input_parser_require_headroom(&db, &root, tokens - 31_000, requests - 2).unwrap();
    for (t, r) in [(tokens - 31_000 + 1, requests - 2), (tokens - 31_000, requests - 2 + 1)] {
        assert_eq!(native_input_parser_require_headroom(&db, &root, t, r).unwrap_err(),
            NATIVE_INPUT_PARSER_BUDGET_UNAVAILABLE, "{t}/{r} of {tokens}/{requests}");
    }
}

#[test]
fn input_parser_mint_refuses_floor_encroaching_grant_with_zero_writes() {
    // Calibrated from the success fixture: the same actual flow under a smaller
    // frozen hard limit leaves the Root above both floors but short of floor+grant.
    for (tag, limits) in [("ip-mint-token-floor", (INPUT_PARSER_MINT_TOKEN_EDGE - 1, 20)),
                          ("ip-mint-request-floor", (60_000, INPUT_PARSER_MINT_REQUEST_EDGE - 1))] {
        let _real = RealSpecialistTransport::enter();
        let f = input_parser_root_fixture_original_v5_limits(tag, &["dispatch:input_parser"], limits);
        let db = db::open(&f.h.db_path).unwrap();
        input_parser_mint_settle_executor(&f);
        let decision = f.feedback().unwrap();
        assert!(native_root_paid_suggestion_selected(&decision, NATIVE_INPUT_PARSER_DISPATCH_STEP));
        let (tokens, requests) = input_parser_mint_remaining(&db, &f.root);
        assert!(tokens >= 31_000 && requests >= 2, "{tag}: floors still free {tokens}/{requests}");
        assert!(tokens < 35_000 || requests < 3, "{tag}: grant would encroach {tokens}/{requests}");
        let (physical, calls) = (web_mode_test_rows(&db), f.calls());
        assert_eq!(input_parser_mint(&f, &decision).unwrap_err(), NATIVE_INPUT_PARSER_BUDGET_UNAVAILABLE, "{tag}");
        web_mode_assert_rows(&db, &physical);
        assert_eq!((f.calls(), f.input_parser_rows(&db)), (calls, 0));
        assert_eq!(input_parser_mint_remaining(&db, &f.root), (tokens, requests));
    }
    // One more unit of the frozen limit is exactly enough: the same flow mints.
    for (tag, limits) in [("ip-mint-token-edge", (INPUT_PARSER_MINT_TOKEN_EDGE, 20)),
                          ("ip-mint-request-edge", (60_000, INPUT_PARSER_MINT_REQUEST_EDGE))] {
        let _real = RealSpecialistTransport::enter();
        let f = input_parser_root_fixture_original_v5_limits(tag, &["dispatch:input_parser"], limits);
        let db = db::open(&f.h.db_path).unwrap();
        input_parser_mint_settle_executor(&f);
        let decision = f.feedback().unwrap();
        let (tokens, requests) = input_parser_mint_remaining(&db, &f.root);
        input_parser_mint(&f, &decision).unwrap();
        assert_eq!(input_parser_mint_remaining(&db, &f.root), (tokens - 4_000, requests - 1));
        let left = input_parser_mint_remaining(&db, &f.root);
        assert!(left.0 >= 31_000 && left.1 >= 2, "{tag}: floors kept {left:?}");
        assert!(left.0 == 31_000 || left.1 == 2, "{tag}: edge is exact {left:?}");
    }
}

#[test]
fn input_parser_mint_refuses_unselected_tampered_rebound_or_stale_fenced_decisions_with_zero_writes() {
    let _real = RealSpecialistTransport::enter();
    // A paid deferral is not a dispatch.
    let deferred = input_parser_root_fixture("ip-mint-defer", &[]);
    let db = db::open(&deferred.h.db_path).unwrap();
    let decision = deferred.feedback().unwrap();
    let physical = web_mode_test_rows(&db);
    assert_eq!(input_parser_mint(&deferred, &decision).unwrap_err(), "root_did_not_select_input_parser");
    web_mode_assert_rows(&db, &physical);
    drop(db);
    drop(deferred);
    let f = input_parser_root_fixture("ip-mint-tamper", &["dispatch:input_parser"]);
    let db = db::open(&f.h.db_path).unwrap();
    input_parser_mint_settle_executor(&f);
    let decision = f.feedback().unwrap();
    let (calls, physical) = (f.calls(), web_mode_test_rows(&db));
    let refused = |code: &str| {
        assert_eq!(input_parser_mint(&f, &decision).unwrap_err(), code);
        web_mode_assert_rows(&db, &physical);
        assert_eq!((f.calls(), f.input_parser_rows(&db)), (calls, 0));
    };
    // (1) Broker record bytes tampered after the paid decision.
    let (node, path) = f.record(&db);
    let original = std::fs::read(&path).unwrap();
    let mut tampered: JsonValue = serde_json::from_slice(&original).unwrap();
    tampered["request"]["url"] = format!("{}/api/orders", f.h.context.target_url).into();
    std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    refused("input_parser_trigger_fact_invalid");
    std::fs::write(&path, &original).unwrap();
    // (2) The cited fact changed after the paid decision: no longer eligible.
    let payload: String = db.query_row("SELECT payload_json FROM agent_evidence_nodes WHERE id=?1", [&node], |r| r.get(0)).unwrap();
    db.execute("UPDATE agent_evidence_nodes SET payload_json=json_set(payload_json,'$.status',500) WHERE id=?1", [&node]).unwrap();
    let changed = web_mode_test_rows(&db);
    assert!(input_parser_mint(&f, &decision).is_err());
    web_mode_assert_rows(&db, &changed);
    db.execute("UPDATE agent_evidence_nodes SET payload_json=?2 WHERE id=?1", params![node, payload]).unwrap();
    web_mode_assert_rows(&db, &physical);
    // (3) A still-valid but different frame (re-revisioned fact) was never paid for.
    db.execute("UPDATE agent_evidence_nodes SET revision=revision+1 WHERE id=?1", [&node]).unwrap();
    let rebound = web_mode_test_rows(&db);
    NativeCoordinatorFrame::input_parser_trigger(&db, f.lease(), &f.h.context.target_dir).unwrap();
    assert_eq!(input_parser_mint(&f, &decision).unwrap_err(), "root_input_parser_decision_frame_mismatch");
    web_mode_assert_rows(&db, &rebound);
    db.execute("UPDATE agent_evidence_nodes SET revision=revision-1 WHERE id=?1", [&node]).unwrap();
    web_mode_assert_rows(&db, &physical);
    // (4) A stale or forged fence cannot issue under the original Root.
    let root = native_coordinator_root_context(&f.h.context, f.lease());
    let mut stale = f.lease().clone();
    stale.fencing_token = uuid::Uuid::new_v4().to_string();
    assert!(native_coordinator_mint_input_parser_child(&root, &stale, &decision).is_err());
    web_mode_assert_rows(&db, &physical);
    assert_eq!(f.input_parser_rows(&db), 0);
    // Restored facts: the original paid decision mints exactly once.
    input_parser_mint(&f, &decision).unwrap();
    assert_eq!(f.input_parser_rows(&db), 2);
    assert_eq!(f.calls(), calls);
}
