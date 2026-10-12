// U179: prove the whole InputParser readonly path is zero-write on every failure
// branch (clears `zero_write_proof_on_failure`). The readonly child was minted in
// U177 and runs the U178 input surface; here we inject a failure at each point and
// prove the database never keeps a half-written result.
//
//   * Pre-send failures (lost fence, tampered Broker record, a fact re-scoped to a
//     foreign run, a write capability, a target-request capability) are refused
//     before a single provider byte and leave EVERY application table byte-identical
//     (full-table snapshot equality), spending nothing.
//   * An SDK provider failure (HTTP 5xx / unparseable body) reaches the shared
//     `failed_specialist_error` cleanup: it publishes NO result (no evidence, no
//     `input_assessment` message, no received call, no target request/claim), leaves
//     NO live capability lease, never completes the child, never silently discards
//     the reservation, and never lowers the Reviewer 15000/1 or Root 16000/1 floor.
//   * A schema-invalid output is proven un-delivered (rolled back) by U178's
//     `input_parser_sdk_invalid_output_is_not_delivered`.
//
// The full-table snapshot is `super::tests::application_table_snapshot` (every
// non-internal table, all rows); a mutation that leaves a stray write on any failure
// branch makes the first test go red.

#[test]
fn input_parser_zero_write_preflight_failures_roll_back_every_application_table() {
    use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
    let _real = RealSpecialistTransport::enter();
    let mut f = input_parser_root_fixture("ip-zw-preflight", &["dispatch:input_parser"]);
    let child = input_parser_sdk_mint(&f);
    let db = db::open(&f.h.db_path).unwrap();
    // A failed pre-send branch must spend zero provider bytes and keep every
    // application table byte-identical to the state captured right before the run.
    let run_fail = |f: &mut InputParserRootFixture, lease: &CoordinatorLease, expect: &str| {
        let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|_: String| {
            (200, "application/json", proposal_model_response(IP_SDK_VALID_ASSESSMENT))
        }));
        f.h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let before = super::tests::application_table_snapshot(&db);
        let err = multi_agent_input_parser_readonly(&f.h.context, lease, &child).unwrap_err();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(err, expect);
        assert_eq!(seen.lock().unwrap().len(), 0, "{expect}: zero SDK byte");
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{expect}: a failed branch wrote to an application table"
        );
    };

    // (1) Lost / advanced coordinator fence: the child is no longer this lease's.
    let mut stale = f.lease().clone();
    stale.lease_epoch += 1;
    run_fail(&mut f, &stale, "input_parser_assignment_invalid");

    let current = f.lease().clone();

    // (2) The cited Broker record bytes were tampered after the mint.
    let (node, path) = f.record(&db);
    let original = std::fs::read(&path).unwrap();
    let mut tampered: JsonValue = serde_json::from_slice(&original).unwrap();
    tampered["request"]["url"] = format!("{}/api/orders?page=2", f.h.context.target_url).into();
    std::fs::write(&path, serde_json::to_vec(&tampered).unwrap()).unwrap();
    run_fail(&mut f, &current, "input_parser_surface_fact_invalid");
    std::fs::write(&path, &original).unwrap();

    // (3) The cited fact re-scoped to another live run: never this Root's to read.
    let mapper_run = f.session.as_ref().unwrap().mapper.run_id.clone();
    db.execute("UPDATE agent_evidence_nodes SET root_run_id=?2 WHERE id=?1", params![node, mapper_run]).unwrap();
    run_fail(&mut f, &current, "input_parser_surface_fact_invalid");
    db.execute("UPDATE agent_evidence_nodes SET root_run_id=?2 WHERE id=?1", params![node, f.root]).unwrap();

    // (4) A write capability on the readonly child (attempt to write).
    let caps: String = db.query_row("SELECT capability_lease_json FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| r.get(0)).unwrap();
    db.execute("UPDATE agent_assignments SET capability_lease_json=?2 WHERE id=?1",
        params![child.assignment_id, json!(["evidence.read", "evidence.write", "mailbox.write"]).to_string()]).unwrap();
    run_fail(&mut f, &current, "input_parser_capability_invalid");

    // (5) A target-request capability (attempt to touch the target).
    db.execute("UPDATE agent_assignments SET capability_lease_json=?2 WHERE id=?1",
        params![child.assignment_id, json!(["evidence.read", "mailbox.write", "target.request"]).to_string()]).unwrap();
    run_fail(&mut f, &current, "input_parser_capability_invalid");
    db.execute("UPDATE agent_assignments SET capability_lease_json=?2 WHERE id=?1", params![child.assignment_id, caps]).unwrap();

    // Restored: the authorized readonly surface still delivers exactly once, proving
    // the zero-write refusals above did not corrupt the minted child.
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|_: String| {
        (200, "application/json", proposal_model_response(IP_SDK_VALID_ASSESSMENT))
    }));
    f.h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    multi_agent_input_parser_readonly(&f.h.context, &current, &child).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(db.query_row("SELECT state FROM agent_assignments WHERE id=?1", [&child.assignment_id], |r| r.get::<_, String>(0)).unwrap(), "completed");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
}

// An SDK provider failure writes no result, keeps no live capability lease, never
// completes the child or lowers a retained floor.
fn input_parser_zero_write_sdk_failure(tag: &str, status: u16, body: &'static str) {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let mut f = input_parser_root_fixture(tag, &["dispatch:input_parser"]);
    let child = input_parser_sdk_mint(&f);
    let db = db::open(&f.h.db_path).unwrap();
    let count = |sql: &str| db.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    let evidence_before = count("SELECT count(*) FROM agent_evidence_nodes");
    let (rt0, rr0): (i64, i64) = db.query_row(
        "SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1",
        [&child.assignment_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert!(rt0 > 0 && rr0 > 0, "the minted child holds its fixed-grant reservation");
    // Retained floors before the failed run.
    budget::root::require_child_capacity(&db, &f.root, 15_000, 1).unwrap();
    let floor = native_coordinator_executor_retained_floor(&db, f.lease()).unwrap();
    assert_eq!(floor, (31_000, 2));

    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |_: String| (status, "application/json", body.to_string())));
    f.h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let lease = f.lease().clone();
    let err = multi_agent_input_parser_readonly(&f.h.context, &lease, &child).unwrap_err();
    assert!(!err.is_empty(), "SDK failure surfaces an error");
    assert_eq!(seen.lock().unwrap().len(), 1, "the SDK was actually attempted");
    stop.store(true, std::sync::atomic::Ordering::SeqCst);

    // No result was published anywhere.
    assert_eq!(count("SELECT count(*) FROM agent_evidence_nodes"), evidence_before, "no evidence written on SDK failure");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='input_assessment'", [&child.assignment_id], |r| r.get::<_, i64>(0)).unwrap(), 0, "no result message");
    assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1 AND state='received'", [&child.assignment_id], |r| r.get::<_, i64>(0)).unwrap(), 0, "no received result");
    assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)", [&child.run_id], |r| r.get::<_, bool>(0)).unwrap(), "no target claim");
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1, "no new target request (only the Mapper's earlier Broker GET)");

    // No residual live capability lease; the child never completes.
    assert!(!db.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at='')", [&child.run_id], |r| r.get::<_, bool>(0)).unwrap(), "a failed child keeps no live capability lease");
    let (state, settled, rt, rr): (String, String, i64, i64) = db.query_row(
        "SELECT state,budget_settled_at,reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1",
        [&child.assignment_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).unwrap();
    assert_ne!(state, "completed", "SDK failure never completes the child");
    let run_status: String = db.query_row("SELECT terminal_state FROM agent_runs WHERE id=?1", [&child.run_id], |r| r.get(0)).unwrap();
    assert_ne!(run_status, "completed");
    // The reservation is never silently discarded: either preserved for reconciliation
    // (settle marker empty, reservation intact) or fully settled (reservation zeroed).
    if settled.is_empty() {
        assert_eq!((rt, rr), (rt0, rr0), "preserved reservation stays intact for reconciliation");
    } else {
        assert_eq!((rt, rr), (0, 0), "a settled failed child releases its reservation");
    }

    // A failed child never reduces a retained floor.
    budget::root::require_child_capacity(&db, &f.root, 15_000, 1).unwrap();
    assert_eq!(native_coordinator_executor_retained_floor(&db, &lease).unwrap(), (31_000, 2));
}

#[test]
fn input_parser_zero_write_sdk_http_error_publishes_no_result() {
    input_parser_zero_write_sdk_failure("ip-zw-503", 503, r#"{"error":{"message":"boom"}}"#);
}

#[test]
fn input_parser_zero_write_sdk_unparseable_body_publishes_no_result() {
    input_parser_zero_write_sdk_failure("ip-zw-malformed", 200, "not json at all");
}
