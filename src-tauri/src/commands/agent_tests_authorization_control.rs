fn authorization_fixture() -> (PathBuf, PathBuf, PathBuf, SaveAuthorizationControlInput) {
    let (root, db_path) = temp_database("authorization-control");
    seed_scan(&db_path, "agent-scan", "draft");
    seed_target(&db_path, "https://app.example.invalid");
    seed_session(&db_path, "session-a", "cookie-a", "Bearer a");
    seed_session(&db_path, "session-b", "cookie-b", "Bearer b");
    db::open(&db_path).unwrap().execute(
        "INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES('agent-scan',?1)",
        [serde_json::json!({"authSessionIds":["session-a","session-b"]}).to_string()],
    ).unwrap();
    let app_data_dir = root.join("app-data");
    let input = SaveAuthorizationControlInput {
        scan_id: "agent-scan".into(),
        attempt_number: 2,
        target_url: "https://app.example.invalid".into(),
        contract_key: "idor|/api/orders".into(),
        owner_object_url: "https://app.example.invalid/api/orders?id=owner".into(),
        tester_control_url: "https://app.example.invalid/api/orders?id=tester".into(),
        object_query_key: "id".into(),
        owner_object_value: "owner".into(),
        tester_object_value: "tester".into(),
        response_object_pointer: "/order/id".into(),
        owner_identity: "session-a".into(),
        tester_identity: "session-b".into(),
    };
    (root, db_path, app_data_dir, input)
}

fn freeze_authorization_test_plan(context: &AgentRunContext) {
    persist_agent_execution_plan(
        &context.db_path,
        &context.scan_id,
        context.attempt_number,
        &context.target_url,
        &context.execution_plan,
    ).unwrap();
}

#[test]
fn authorization_control_registers_only_for_next_draft_attempt_and_is_immutable() {
    let (_root, db_path, app_data_dir, mut input) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Ok(()));
    let stored: (i64, String) = connection.query_row(
        "SELECT attempt_number,owner_object_url FROM agent_authorization_controls WHERE scan_id='agent-scan'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(stored, (2, input.owner_object_url.clone()));
    input.owner_object_value = "different".into();
    input.owner_object_url = "https://app.example.invalid/api/orders?id=different".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Err("authorization_control_immutable_conflict".into()));
    let unchanged: String = connection.query_row(
        "SELECT owner_object_url FROM agent_authorization_controls WHERE scan_id='agent-scan'", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(unchanged, stored.1);
}

#[test]
fn authorization_control_refuses_a_fifth_group_before_the_scan_starts() {
    let (_root, db_path, app_data_dir, mut input) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    for index in 0..4 {
        input.contract_key = format!("orders-{index}");
        save_authorization_control_in(&mut connection, &app_data_dir, &input).unwrap();
    }
    input.contract_key = "orders-4".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input),
        Err("authorization_control_group_limit_exceeded".into()));
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_authorization_controls WHERE scan_id=?1", [&input.scan_id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 4);
}

#[test]
fn authorization_control_rejects_scope_identity_selector_and_started_attempt() {
    let (_root, db_path, app_data_dir, input) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    let mut candidate = input.clone();
    candidate.owner_object_url = "https://other.example.invalid/api/orders?id=owner".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_control_scope_invalid".into()));
    candidate = input.clone();
    candidate.owner_object_url = "https://app.example.invalid/api/orders?id=owner&id=owner".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_object_selector_missing_or_ambiguous".into()));
    candidate = input.clone();
    candidate.owner_identity = "unbound".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_control_identity_unbound".into()));
    candidate = input.clone();
    candidate.response_object_pointer = "/order/~2bad".into();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_control_invalid".into()));
    candidate = input.clone();
    candidate.attempt_number = 1;
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &candidate), Err("authorization_control_task_binding_invalid".into()));
    connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url) VALUES('started','agent-scan',2,'https://app.example.invalid')", [],
    ).unwrap();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Err("authorization_control_attempt_already_started".into()));
    connection.execute("UPDATE sentinel_scans SET status='scanning' WHERE id='agent-scan'", []).unwrap();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Err("authorization_control_task_binding_invalid".into()));
}

#[test]
fn authorization_control_follows_directory_planner_without_creating_an_attempt() {
    let (_root, db_path, app_data_dir, mut input) = authorization_fixture();
    let scan_dir = neutral_scan_work_root(&app_data_dir).join("agent-scan");
    fs::create_dir_all(scan_dir.join("attempt-0005")).unwrap();
    let mut connection = db::open(&db_path).unwrap();
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Err("authorization_control_task_binding_invalid".into()));
    input.attempt_number = 6;
    assert_eq!(save_authorization_control_in(&mut connection, &app_data_dir, &input), Ok(()));
    assert!(!scan_dir.join("attempt-0006").exists());
}

#[test]
fn authorization_control_table_is_created_for_an_existing_database() {
    let (root, db_path, _app_data_dir, _input) = authorization_fixture();
    let connection = db::open(&db_path).unwrap();
    connection.execute("DROP TABLE agent_authorization_controls", []).unwrap();
    drop(connection);
    let reopened = db::initialize(&root).unwrap();
    assert_eq!(reopened, db_path);
    let connection = db::open(&db_path).unwrap();
    let exists: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_authorization_controls'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(exists, 1);
    let claims: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_authorization_probe_claims'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(claims, 1);
}

#[test]
fn authorization_setup_exposes_only_bound_draft_target_and_valid_identity_handles() {
    let (_root, db_path, app_data_dir, input) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    let setup = authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).unwrap();
    assert_eq!(setup.attempt_number, 2);
    assert_eq!(setup.identity_ids, vec!["session-a", "session-b"]);
    assert!(setup.contract_keys.is_empty());
    let visible = serde_json::to_string(&setup).unwrap();
    assert!(!visible.contains("cookie-a"));
    assert!(!visible.contains("Bearer a"));
    save_authorization_control_in(&mut connection, &app_data_dir, &input).unwrap();
    let setup = authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).unwrap();
    assert_eq!(setup.contract_keys, vec!["idor|/api/orders"]);
    assert!(authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, "https://other.example.invalid",
    ).is_err());
    connection.execute("UPDATE sentinel_scans SET status='scanning' WHERE id=?1", [&input.scan_id]).unwrap();
    assert!(authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).is_err());
}

#[test]
fn authorization_setup_rejects_single_or_invalid_identity_without_leaking_session_material() {
    let (_root, db_path, app_data_dir, input) = authorization_fixture();
    let connection = db::open(&db_path).unwrap();
    connection.execute(
        "UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id=?2",
        params![serde_json::json!({"authSessionIds":["session-a"]}).to_string(), input.scan_id],
    ).unwrap();
    assert_eq!(authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).unwrap_err(), "authorization_control_requires_two_identities");
    connection.execute(
        "UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id=?2",
        params![serde_json::json!({"authSessionIds":["session-a","session-b"]}).to_string(), input.scan_id],
    ).unwrap();
    connection.execute("UPDATE browser_auth_sessions SET status='invalid' WHERE id='session-b'", []).unwrap();
    assert!(authorization_control_setup_in(
        &connection, &app_data_dir, &input.scan_id, &input.target_url,
    ).is_err());
}

#[test]
fn authorization_three_side_proof_requires_owner_and_tester_object_controls() {
    let (_root, _db_path, _app_data_dir, control) = authorization_fixture();
    let owner = authorization_test_response(&control, "owner", 200, r#"{"order":{"id":"owner","name":"A"}}"#);
    let cross = authorization_test_response(&control, "cross", 200, r#"{"order":{"id":"owner","name":"A"}}"#);
    let own = authorization_test_response(&control, "tester", 200, r#"{"order":{"id":"tester","name":"B"}}"#);
    assert_eq!(verify_authorization_control_group(&control, [&owner, &cross, &own]), Ok(()));

    let mut personalized = cross.clone();
    personalized.body = r#"{"order":{"id":"tester","name":"B"}}"#.into();
    assert_eq!(verify_authorization_control_group(&control, [&owner, &personalized, &own]), Err("authorization_cross_object_not_observed"));
    let mut denied = cross.clone();
    denied.status = 403;
    assert_eq!(verify_authorization_control_group(&control, [&owner, &denied, &own]), Err("authorization_side_not_successful"));
    let mut redacted = cross.clone();
    redacted.body = r#"{"order":{"id":"owner"}}"#.into();
    assert_eq!(verify_authorization_control_group(&control, [&owner, &redacted, &own]), Err("authorization_cross_object_not_comparable"));
    let echoed = redacted.clone();
    let owner_echo = authorization_test_response(&control, "owner", 200, r#"{"order":{"id":"owner"}}"#);
    assert_eq!(verify_authorization_control_group(&control, [&owner_echo, &echoed, &own]),
        Err("authorization_cross_object_not_comparable"));
    let mut error = cross.clone();
    error.body = r#"{"order":{"id":"owner","name":"A"},"error":"forbidden"}"#.into();
    assert_eq!(verify_authorization_control_group(&control, [&owner, &error, &own]), Err("authorization_side_unusable"));
}

#[test]
fn authorization_three_side_proof_rejects_unbound_incomplete_or_artificial_responses() {
    let (_root, _db_path, _app_data_dir, control) = authorization_fixture();
    let owner = authorization_test_response(&control, "owner", 200, r#"{"order":{"id":"owner"}}"#);
    let cross = authorization_test_response(&control, "cross", 200, r#"{"order":{"id":"owner"}}"#);
    let own = authorization_test_response(&control, "tester", 200, r#"{"order":{"id":"tester"}}"#);
    type Mutation = Box<dyn Fn(&mut AuthorizationProbeResponse)>;
    let mut cases: Vec<(usize, &str, Mutation)> = vec![
        (0, "authorization_side_binding_invalid", Box::new(|r| r.identity = "session-b".into())),
        (1, "authorization_side_binding_invalid", Box::new(|r| r.url.push_str("&extra=1"))),
        (2, "authorization_side_binding_invalid", Box::new(|r| r.attempt_number += 1)),
        (1, "authorization_side_binding_invalid", Box::new(|r| r.contract_key = "other".into())),
        (0, "authorization_side_unusable", Box::new(|r| r.truncated = true)),
        (1, "authorization_side_unusable", Box::new(|r| r.cache_state = "hit".into())),
        (1, "authorization_side_unusable", Box::new(|r| r.redirect_code = "in_scope".into())),
        (2, "authorization_side_unusable", Box::new(|r| r.content_type = "text/html".into())),
        (0, "authorization_object_pointer_missing", Box::new(|r| r.body = "{}".into())),
        (1, "authorization_side_unusable", Box::new(|r| r.body = "<html>login</html>".into())),
    ];
    for (index, expected, mutate) in cases.drain(..) {
        let mut responses = [owner.clone(), cross.clone(), own.clone()];
        mutate(&mut responses[index]);
        assert_eq!(verify_authorization_control_group(&control, [&responses[0], &responses[1], &responses[2]]), Err(expected), "case {index}: {expected}");
    }
}

fn authorization_test_response(
    control: &SaveAuthorizationControlInput, side: &str, status: u16, body: &str,
) -> AuthorizationProbeResponse {
    let (identity, url) = match side {
        "owner" => (&control.owner_identity, &control.owner_object_url),
        "cross" => (&control.tester_identity, &control.owner_object_url),
        "tester" => (&control.tester_identity, &control.tester_control_url),
        _ => panic!("invalid side"),
    };
    AuthorizationProbeResponse {
        attempt_number: control.attempt_number,
        contract_key: control.contract_key.clone(),
        identity: identity.clone(), url: url.clone(), status,
        content_type: "application/json".into(), redirect_code: String::new(),
        cache_state: "miss".into(), truncated: false, body: body.into(),
    }
}

#[test]
fn authorization_broker_only_claims_registered_sides_under_a_live_child_lease() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy},
        multi_agent::{lease, scheduler}, store::{self, AgentRunRow},
    };
    let (_root, db_path, app_data_dir, control) = authorization_fixture();
    let mut connection = db::open(&db_path).unwrap();
    let mut context = test_context(&db_path, &control.target_url,
        vec![AgentIdentity::scoped("session-a"), AgentIdentity::scoped("session-b")]);
    context.attempt_number = 2;
    context.execution_plan = context.execution_plan.with_attempt(2);
    let missing_run = context.clone();
    let dummy_child = scheduler::ScheduledChild {
        assignment_id:"absent".into(), run_id:"absent".into(), role:AgentRole::Authorization,
    };
    assert_eq!(claim_authorization_probe(&missing_run, &dummy_child, &control,
        "owner", &control.owner_identity, &control.owner_object_url).unwrap_err(),
        "authorization_side_binding_invalid");
    save_authorization_control_in(&mut connection, &app_data_dir, &control).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=2 WHERE id=?1", [&control.scan_id]).unwrap();
    let mut root = AgentRunRow::new("authorization-root", &control.scan_id, 2,
        &control.target_url, AgentBackendKind::Native, AgentRole::Coordinator,
        context.execution_plan.hash(), "evidence")
        .with_budget(100, 1000, 2, 10);
    root.status = AgentRunStatus::Running;
    root.root_run_id = root.id.clone();
    root.orchestration_policy = MultiAgentPolicy::Multi;
    root.lane = Some(AgentLane::ReadOnlyAnalysis);
    store::create_run(&connection, &root).unwrap();
    freeze_authorization_test_plan(&context);
    let coordinator = lease::acquire_coordinator_lease(&connection, &control.scan_id, 2,
        &control.target_url, &root.id, 600).unwrap();
    assert_eq!(scheduler::schedule_child(&connection, &coordinator, AgentRole::Authorization,
        AgentLane::TargetTouching, "test", &serde_json::json!({}), 1,
        &["replay_http".into()], 0, 3).unwrap_err(), "role_capability_not_allowed");
    assert_eq!(scheduler::schedule_child(&connection, &coordinator, AgentRole::Authorization,
        AgentLane::ReadOnlyAnalysis, "test", &serde_json::json!({}), 1,
        &["authorization_probe".into()], 0, 3).unwrap_err(), "authorization_requires_target_lane");
    let child = scheduler::schedule_child(&connection, &coordinator, AgentRole::Authorization,
        AgentLane::TargetTouching, "test", &serde_json::json!({}), 1,
        &["authorization_probe".into()], 0, 0).unwrap();
    scheduler::mark_child_running(&connection, &coordinator, &child).unwrap();
    context.run = Some(AgentRunLedger { db_path:db_path.clone(), run_id:child.run_id.clone() });
    assert_eq!(claim_authorization_probe(&context, &child, &control,
        "cross", &control.tester_identity, &control.tester_control_url).unwrap_err(),
        "authorization_side_binding_invalid");
    accounting_native(&context,400);
    assert_eq!(claim_authorization_probe(&context, &child, &control,
        "owner", &control.owner_identity, &control.owner_object_url).unwrap_err(),
        "authorization_request_budget_exhausted");
    accounting_native(&context,0);
    claim_authorization_probe(&context, &child, &control, "owner",
        &control.owner_identity, &control.owner_object_url).unwrap();
    let usage = agent_request_accounting(&connection,&context.scan_id,2,&context.target_url).unwrap();
    assert_eq!((usage.authorization.received,usage.authorization.unresolved,usage.recorded,usage.budget_committed),(0,1,0,1));
    assert_eq!(claim_authorization_probe(&context, &child, &control, "owner",
        &control.owner_identity, &control.owner_object_url).unwrap_err(), "authorization_side_already_claimed");
    // PROD-BUGFIX-1 guard: a forged extra account is still denied before any
    // side is reserved, while the fixed credential-free control that prepare
    // appends next to validated accounts passes the binding (reaching the
    // side-claim check).
    let mut forged = context.clone();
    forged.identities.push(AgentIdentity::scoped("session-forged"));
    assert_eq!(claim_authorization_probe(&forged, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap_err(), "authorization_identity_binding_denied");
    let mut with_control = context.clone();
    with_control.identities.push(AgentIdentity::anonymous());
    assert_eq!(claim_authorization_probe(&with_control, &child, &control, "owner",
        &control.owner_identity, &control.owner_object_url).unwrap_err(), "authorization_side_already_claimed");
    let claims: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE scan_id=?1", [&control.scan_id], |r| r.get(0)).unwrap();
    assert_eq!(claims, 1, "a denied identity must not reserve a side");
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&control.scan_id]).unwrap();
    assert_eq!(claim_authorization_probe(&context, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap_err(),
        "authorization_capability_or_fencing_denied");
    connection.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=3 WHERE id=?1", [&control.scan_id]).unwrap();
    assert_eq!(claim_authorization_probe(&context, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap_err(),
        "authorization_capability_or_fencing_denied");
    connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1", [&control.scan_id]).unwrap();
    connection.execute("UPDATE agent_runs SET status='terminal' WHERE id=?1", [&coordinator.root_run_id]).unwrap();
    assert_eq!(claim_authorization_probe(&context, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap_err(),
        "authorization_capability_or_fencing_denied");
    connection.execute("UPDATE agent_runs SET status='running' WHERE id=?1", [&coordinator.root_run_id]).unwrap();
    connection.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1", [&child.run_id]).unwrap();
    assert_eq!(claim_authorization_probe(&context, &child, &control, "cross",
        &control.tester_identity, &control.owner_object_url).unwrap_err(),
        "authorization_capability_or_fencing_denied");
}

#[test]
fn authorization_control_reaches_an_independent_reviewer_before_publication() {
    run_authorization_control_to_review(false, false, 0, 0, false, false, false);
}

#[test]
fn rejected_review_completes_the_child_without_publishing_a_finding() {
    run_authorization_control_to_review(false, false, 0, 0, true, false, false);
}

#[test]
fn denied_cross_object_control_does_not_create_a_candidate_or_finding() {
    run_authorization_control_to_review(true, false, 0, 0, false, false, false);
}

#[test]
fn authorization_mailbox_failure_releases_target_lane_and_reservation() {
    run_authorization_control_to_review(false, true, 0, 0, false, false, false);
}

#[test]
fn pausing_after_the_owner_probe_prevents_the_cross_and_control_requests() {
    run_authorization_control_to_review(false, false, 1, 0, false, false, false);
}

#[test]
fn pausing_after_the_cross_probe_prevents_the_control_request() {
    run_authorization_control_to_review(false, false, 2, 0, false, false, false);
}

#[test]
fn pausing_after_the_control_response_prevents_finding_publication() {
    run_authorization_control_to_review(false, false, 3, 0, false, false, false);
}

#[test]
fn disconnected_owner_control_is_not_replayed_or_reviewed() {
    run_authorization_control_to_review(false, false, 0, 1, false, false, false);
}

#[test]
fn disconnected_cross_control_is_not_replayed_or_reviewed() {
    run_authorization_control_to_review(false, false, 0, 2, false, false, false);
}

#[test]
fn disconnected_tester_control_is_not_replayed_or_reviewed() {
    run_authorization_control_to_review(false, false, 0, 3, false, false, false);
}

#[test]
fn pause_during_candidate_write_cannot_stage_or_publish_authorization() {
    run_authorization_control_to_review(false, false, 0, 0, false, true, false);
}

#[test]
fn pause_after_authorization_before_review_cannot_publish() {
    run_authorization_control_to_review(false, false, 0, 0, false, false, true);
}

fn run_authorization_control_to_review(
    deny_cross: bool,
    fail_mailbox: bool,
    pause_after_probe: usize,
    disconnect_probe: usize,
    reject_review: bool,
    pause_on_candidate: bool,
    pause_before_review: bool,
) {
    run_authorization_control_to_review_audit(
        deny_cross, fail_mailbox, pause_after_probe, disconnect_probe,
        reject_review, pause_on_candidate, pause_before_review, false, false,
        AuthorizationReviewFault::None, |_, _, _, _, _, _| {},
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AuthorizationReviewFault {
    None,
    OriginalAckDuringReviewerSdk,
    OriginalAckColdDuringReviewerSdk,
    FindingPublicationAbort,
    FindingPublicationIgnore,
    FindingPublicationSourceAckChange,
}

// Disposable-fixture offline corruption/restoration, not a production mailbox
// write. Restore exact emitter DDL and never rewrite an immutable original event.
fn authorization_fixture_cold_ack(
    db: &rusqlite::Connection,
    message: &str,
    expected: &str,
    replacement: &str,
) {
    assert!(!db.is_autocommit());
    crate::collaboration_events::client_delivery_schema::verify(db).unwrap();
    let events = NativeCoordinatorFrozenRows::capture(db,
        "SELECT rowid,* FROM agent_collaboration_events WHERE entity_id=?1 ORDER BY sequence",
        &[message]).unwrap();
    let ddl: String = db.query_row(
        "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='agent_collaboration_message_update'",
        [], |row| row.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER agent_collaboration_message_update").unwrap();
    assert_eq!(db.execute(
        "UPDATE agent_messages SET acknowledged_at=?1 WHERE id=?2 AND kind='authorization_result' AND acknowledged_at=?3 AND delivered_at<>''",
        params![replacement,message,expected]).unwrap(), 1);
    db.execute_batch(&ddl).unwrap();
    crate::collaboration_events::client_delivery_schema::verify(db).unwrap();
    let restored: String = db.query_row(
        "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='agent_collaboration_message_update'",
        [], |row| row.get(0)).unwrap();
    assert_eq!(restored, ddl);
    events.verify(db).unwrap();
}

fn authorization_reviewer_saved_ack_recovery_audit(
    context: &AgentRunContext,
    session: &mut MultiAgentSession,
    db: &rusqlite::Connection,
    models: &SpecialistTestSeen,
    targets: &SpecialistTestSeen,
) {
    use crate::agent_runtime::multi_agent::{attempts, budget, specialist};
    let root = &session.lease.root_run_id;
    let sdk_before = models.lock().unwrap().len();
    let target_before = targets.lock().unwrap().clone();
    let (candidate, revision, raw, assignment, run): (String, i64, String, String, String) = db.query_row(
        "SELECT candidate_id,candidate_revision,candidate_json,assignment_id,reviewer_run_id FROM agent_review_requests WHERE root_run_id=?1 AND status='failed'",
        [root], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
    ).unwrap();
    let reviewer = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::EvidenceReviewer,
    };
    let tx = db.unchecked_transaction().unwrap();
    let original_rows = [
        NativeCoordinatorFrozenRows::capture(&tx,
            "SELECT rowid,assignment_id,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,response_json,usage_json,response_hash,event_sequence,finished_at FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",
            &[&reviewer.assignment_id,&reviewer.run_id]).unwrap(),
        NativeCoordinatorFrozenRows::capture(&tx,
            "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND kind='consume' ORDER BY rowid",
            &[root,&reviewer.assignment_id]).unwrap(),
        NativeCoordinatorFrozenRows::capture(&tx,
            "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND kind='reserve' ORDER BY rowid",
            &[root]).unwrap(),
        NativeCoordinatorFrozenRows::capture(&tx,
            "SELECT rowid,* FROM agent_capability_leases WHERE root_run_id=?1 ORDER BY rowid",
            &[root]).unwrap(),
        // Parent heartbeat may advance the deadline; local recovery must keep
        // the original coordinator identity and fence without acquiring a new C.
        NativeCoordinatorFrozenRows::capture(&tx,
            "SELECT rowid,scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,created_at FROM agent_coordinator_leases WHERE root_run_id=?1 ORDER BY rowid",
            &[root]).unwrap(),
    ];
    let original_receipt = specialist::received_for_reconciliation(&tx, &session.lease, &reviewer).unwrap();
    // Recover the exact ACK from its immutable original consumption event,
    // never from now(), a captured mutable scalar or a fabricated receipt.
    let acknowledgments: Vec<(String, String)> = tx.prepare(
        "SELECT m.id,json_extract(e.payload_json,'$.acknowledgedAt') FROM agent_messages m JOIN agent_collaboration_events e ON e.entity_id=m.id WHERE m.root_run_id=?1 AND m.kind='authorization_result' AND m.acknowledged_at='' AND m.delivered_at<>'' AND e.scan_id=?2 AND e.attempt_number=?3 AND e.entity_type='mailbox_message' AND e.event_type='mailbox_message' AND json_extract(e.payload_json,'$.kind')=m.kind AND json_extract(e.payload_json,'$.deliveredAt')=m.delivered_at AND json_extract(e.payload_json,'$.acknowledgedAt')<>'' ORDER BY e.sequence",
    ).unwrap().query_map(params![root,context.scan_id,context.attempt_number],
        |row| Ok((row.get(0)?,row.get(1)?))).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(acknowledgments.len(), 1);
    let (message, ack) = &acknowledgments[0];
    assert!(!ack.is_empty());
    let original_auth_events = NativeCoordinatorFrozenRows::capture(&tx,
        "SELECT rowid,* FROM agent_collaboration_events WHERE entity_id=?1 ORDER BY sequence",
        &[message]).unwrap();
    authorization_fixture_cold_ack(&tx, message, "", ack);
    let frozen: JsonValue = serde_json::from_str(&raw).unwrap();
    native_authorization_original_candidate_on(&tx, &session.lease, &context.target_dir, &frozen).unwrap();
    tx.commit().unwrap();
    let recovered = recover_received_review(db, &session.lease, &candidate, revision, &raw, &context.target_dir).unwrap();
    assert_eq!(recovered.verdict, "confirmed");
    assert_eq!(models.lock().unwrap().len(), sdk_before);
    assert_eq!(*targets.lock().unwrap(), target_before);
    let tx = db.unchecked_transaction().unwrap();
    for row in &original_rows { row.verify(&tx).unwrap(); }
    original_auth_events.verify(&tx).unwrap();
    specialist::verify_received_original(&tx, &session.lease, &reviewer.assignment_id).unwrap();
    let completed_receipt = specialist::received_for_reconciliation(&tx, &session.lease, &reviewer).unwrap();
    assert_eq!(completed_receipt.text, original_receipt.text);
    assert_eq!(completed_receipt.usage, original_receipt.usage);
    assert_eq!(completed_receipt.rejection, original_receipt.rejection);
    assert_eq!(attempts::current(&tx, &session.lease, &reviewer.assignment_id).unwrap().state, "completed");
    let closed: (String, i64, i64, bool, String, String) = tx.query_row(
        "SELECT a.state,a.reserved_tokens,a.reserved_requests,a.budget_settled_at<>'',r.status,r.terminal_state FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2",
        params![reviewer.assignment_id,reviewer.run_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
    ).unwrap();
    assert_eq!(closed, ("completed".into(), 0, 0, true, "terminal".into(), "completed".into()));
    for (dimension, consumed) in [("model_input_tokens", 10), ("model_cached_tokens", 0), ("model_output_tokens", 10), ("model_requests", 1)] {
        let balance = budget::balance(&tx, root, Some(&reviewer.assignment_id), dimension).unwrap();
        assert_eq!((balance.consumed, balance.reserved, balance.indeterminate), (consumed, 0, 0));
    }
    let published: (i64, i64, i64, i64) = tx.query_row(
        "SELECT (SELECT count(*) FROM agent_review_decisions WHERE root_run_id=?1 AND reviewer_run_id=?2 AND verdict='confirmed'),(SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?3 AND kind='review_decision' AND delivered_at<>'' AND acknowledged_at<>''),(SELECT count(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND reviewer_run_id=?2 AND status='published' AND published_at<>''),(SELECT count(*) FROM sentinel_findings WHERE scan_id=?4 AND record_json LIKE '%authorization_cross_object_read%')",
        params![root,reviewer.run_id,reviewer.assignment_id,context.scan_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap();
    assert_eq!(published, (1, 1, 1, 1));
    tx.commit().unwrap();
    let completed = super::tests::application_table_snapshot(db);
    for _ in 0..2 {
        assert_eq!(recover_received_review(db, &session.lease, &candidate, revision, &raw, &context.target_dir).unwrap_err(),
            "review_receipt_request_not_recoverable");
        assert!(super::tests::application_table_snapshot(db) == completed);
        let replay = replay_frozen_review(context, session, &candidate, revision, &raw,
            AgentTargetOutcome::incomplete("fixture executor finished without a finding")).unwrap();
        assert!(matches!(replay, AgentTargetOutcome::Incomplete(_)), "{replay:?}");
        assert!(super::tests::application_table_snapshot(db) == completed,
            "completed same-call replay must not deliver or settle twice");
        assert_eq!(models.lock().unwrap().len(), sdk_before);
        assert_eq!(*targets.lock().unwrap(), target_before);
    }
}

// The original seven-argument entry point keeps its behavior. New lease tests
// audit while the actual session, original C and both localhost endpoints live.
#[allow(clippy::too_many_arguments)]
fn run_authorization_control_to_review_audit(
    deny_cross: bool,
    fail_mailbox: bool,
    pause_after_probe: usize,
    disconnect_probe: usize,
    reject_review: bool,
    pause_on_candidate: bool,
    pause_before_review: bool,
    expire_reviewer: bool,
    audit_after_replay: bool,
    fault: AuthorizationReviewFault,
    audit: impl FnOnce(
        &AgentRunContext, &mut MultiAgentSession, &rusqlite::Connection,
        &SpecialistTestSeen, &SpecialistTestSeen, &AgentTargetOutcome,
    ),
) {
    if expire_reviewer {
        assert!(!deny_cross && !fail_mailbox && pause_after_probe == 0
            && disconnect_probe == 0 && !pause_on_candidate && !pause_before_review);
    }
    if fault != AuthorizationReviewFault::None {
        assert!(expire_reviewer && !audit_after_replay && !reject_review);
    }
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let target = format!("http://{}", listener.local_addr().unwrap());
    // Migrated positive fixture: the scan is created and started through the
    // actual web-mode creator/startup (two validated identities, the operator
    // control group saved before start), and the attempt gets a genuinely born
    // frozen Web Root/C (persist_frozen_web_execution_plan + runtime_open_run +
    // native_frozen_web_root_mode) instead of a hand-rolled partial root.
    // Original Root budget (soft/hard 60,000/120,000 tokens, 40/80 requests)
    // unchanged.
    let app_data_dir = std::env::temp_dir().canonicalize().unwrap()
        .join(format!("oviraptor-authorization-review-{}", Uuid::new_v4()));
    let db_path = db::initialize(&app_data_dir).unwrap();
    let mut connection = db::open(&db_path).unwrap();
    connection.execute("INSERT INTO projects(id,name) VALUES(9001,'Authorization review fixture')", []).unwrap();
    let scope = Uuid::new_v4().to_string();
    for (id, cookie, authorization) in [("session-a", "cookie-a", "Bearer a"), ("session-b", "cookie-b", "Bearer b")] {
        let document = serde_json::json!({
            "schemaVersion": 1, "id": id, "projectId": 9001, "entryUrl": target,
            "scopeHosts": ["127.0.0.1"], "cookies": [{"name": "session", "value": cookie}],
            "headers": {"authorization": authorization},
            "identityIsolation": {"sharedWithOtherSessions": false},
        });
        connection.execute(
            "INSERT INTO browser_auth_sessions(id,project_id,draft_scope_id,name,entry_url,status,session_json,expires_at) VALUES(?1,9001,?2,?1,?3,'valid',?4,?5)",
            params![id, scope, target, document.to_string(), (chrono::Utc::now() + chrono::Duration::hours(8)).to_rfc3339()],
        ).unwrap();
    }
    let scan = create_sentinel_url_scan_with_mode_in(
        &connection, 9001, "Authorization review fixture".into(), vec![target.clone()],
        Some("standard".into()), None, None,
        Some(vec!["session-a".into(), "session-b".into()]), Some(scope.clone()),
        None, None, None, Some("multi".into()),
    ).unwrap();
    let scan_id = scan.id.clone();
    let scan_id: &str = &scan_id;
    let setup = authorization_control_setup_in(&connection, &app_data_dir, scan_id, &target).unwrap();
    let attempt = setup.attempt_number;
    let control = SaveAuthorizationControlInput {
        scan_id: scan_id.into(),
        attempt_number: attempt,
        target_url: target.clone(),
        contract_key: "idor|/api/orders".into(),
        owner_object_url: format!("{target}/api/orders?id=owner"),
        tester_control_url: format!("{target}/api/orders?id=tester"),
        object_query_key: "id".into(),
        owner_object_value: "owner".into(),
        tester_object_value: "tester".into(),
        response_object_pointer: "/order/id".into(),
        owner_identity: "session-a".into(),
        tester_identity: "session-b".into(),
    };
    save_authorization_control_in(&mut connection, &app_data_dir, &control).unwrap();
    web_mode_test_start(&app_data_dir, &mut connection, scan_id, WebStartMode::Confirm).unwrap();
    let mut plan = test_plan_for("standard", &target).with_attempt(attempt);
    plan.hard_total_tokens = 120_000;
    plan.soft_uncached_tokens = 60_000;
    plan.hard_model_requests = 80;
    plan.soft_model_requests = 40;
    // Every branch pays for and exits the actual original Web Executor.
    plan.max_turns = 1;
    persist_frozen_web_execution_plan(&db_path, scan_id, attempt, &target, &plan).unwrap();
    let server_db_path = db_path.clone();
    let server_scan_id = scan_id.to_string();
    // The born Root freezes this model endpoint (Root ticks, Mapper,
    // IdentitySession, the former in-process Reviewer verdicts) for its life.
    let original_ack = std::sync::Arc::new(std::sync::Mutex::new(None::<(String, String)>));
    let fault_ack = original_ack.clone();
    let publication_source = std::sync::Arc::new(std::sync::Mutex::new(None::<Vec<NativeCoordinatorFrozenRows>>));
    let fault_publication_source = publication_source.clone();
    let fault_db_path = db_path.clone();
    let fault_scan_id = scan_id.to_string();
    // Select new assessment replies from the actual Root input, rather than
    // matching strings that may also occur inside retained evidence.
    fn bounded_original_assessment(request: &str) -> Option<String> {
        let body: JsonValue = serde_json::from_str(request.split_once("\r\n\r\n")?.1).ok()?;
        if !body["messages"][0]["content"].as_str()?.contains("You are the Root Coordinator") {
            return None;
        }
        let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str()?).ok()?;
        let step = match input["basis"]["phase"].as_str()? {
            "budget-consumption" => "assess:budget_consumption",
            "http-budget-consumption" => "assess:http_budget_consumption",
            "capability-changed" => "assess:capability_changed",
            "authorization-output" => "assess:authorization_output",
            _ => return None,
        };
        Some(proposal_model_response(&json!({"schemaVersion":1,
            "observed":["original signed source and settled fees"],
            "missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string()))
    }
    let (port, model_seen, _stop) = if expire_reviewer {
        spawn_endpoint(std::sync::Arc::new(move |request| {
            if specialist_original_is_web_sdk(&request) {
                return (200,"application/json",model_round(&[],10));
            }
            if let Some(response) = bounded_original_assessment(&request) {
                return (200, "application/json", response);
            }
            if request.contains("You are the Root Coordinator") {
                let body: JsonValue = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
                let response = if input["basis"]["phase"] == "lease-expired" {
                    proposal_model_response(&json!({"schemaVersion":1,
                        "observed":["original signed expired Reviewer and actual control evidence"],
                        "missing":[],"suggestions":["assess:lease_expired"],"costNotes":[],"risks":[]}).to_string())
                } else {changed_fact_reviewer_response(&request,true)};
                (200,"application/json",response)
            } else if request.contains("SPA/API Mapper") {
                (200,"application/json",proposal_model_response(
                    r#"{"summary":"actual paid Mapper receipt","priorityContracts":[],"risks":[]}"#))
            } else if request.contains("独立 Evidence Reviewer") {
                let body: JsonValue = serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
                let candidate: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
                let findings = candidate["findingCandidates"].as_array().unwrap();
                assert_eq!(findings.len(),1,"the verdict must review a real nonempty candidate");
                assert_eq!(findings[0]["record"]["verification"],"operator_bound_three_side_native_control");
                assert_eq!(findings[0]["record"]["sides"].as_array().unwrap().len(),3);
                if matches!(fault, AuthorizationReviewFault::OriginalAckDuringReviewerSdk
                    | AuthorizationReviewFault::OriginalAckColdDuringReviewerSdk) {
                    // The provider has received this actual frozen Reviewer request.
                    // Damage only the prior Auth ACK before returning a complete known bill.
                    let db = db::open(&fault_db_path).unwrap();
                    let tx = db.unchecked_transaction().unwrap();
                    let (message, ack): (String, String) = tx.query_row(
                        "SELECT m.id,m.acknowledged_at FROM agent_messages m JOIN agent_runs root ON root.id=m.root_run_id AND root.role='coordinator' WHERE root.scan_id=?1 AND root.attempt_number=?2 AND m.kind='authorization_result' AND m.delivered_at<>'' AND m.acknowledged_at<>''",
                        params![fault_scan_id,attempt], |row| Ok((row.get(0)?,row.get(1)?)),
                    ).unwrap();
                    let mut original = fault_ack.lock().unwrap();
                    assert!(original.is_none(), "the paid Reviewer request must occur once");
                    if fault == AuthorizationReviewFault::OriginalAckColdDuringReviewerSdk {
                        // Separate offline-column corruption case: retain the two
                        // immutable original events instead of making a third one.
                        authorization_fixture_cold_ack(&tx, &message, &ack, "");
                    } else {
                        assert_eq!(tx.execute("UPDATE agent_messages SET acknowledged_at='' WHERE id=?1 AND kind='authorization_result'", [&message]).unwrap(), 1);
                    }
                    *original = Some((message, ack));
                    tx.commit().unwrap();
                } else if matches!(fault, AuthorizationReviewFault::FindingPublicationAbort
                    | AuthorizationReviewFault::FindingPublicationIgnore
                    | AuthorizationReviewFault::FindingPublicationSourceAckChange) {
                    let mut original = fault_publication_source.lock().unwrap();
                    assert!(original.is_none(), "the paid Reviewer request must occur once");
                    *original = Some(authorization_review_install_publication_fault(
                        &db::open(&fault_db_path).unwrap(), &findings[0],
                        &fault_scan_id, attempt, fault,
                    ));
                }
                multi_agent_original_fake_response(request)
            } else if request.contains("IdentitySession") || request.contains("Deep Investigator") {
                multi_agent_original_fake_response(request)
            } else {(200,"application/json",model_round(&[],10))}
        }))
    } else {
        // Own the Root routing here: specialist_original_endpoint intercepts
        // Root requests before its handler sees the actual assessment phase.
        spawn_endpoint(std::sync::Arc::new(|request| {
            if specialist_original_is_web_sdk(&request) {
                (200,"application/json",model_round(&[],10))
            } else if let Some(response) = bounded_original_assessment(&request) {
                (200,"application/json",response)
            } else if request.contains("You are the Root Coordinator") {
                (200,"application/json",changed_fact_reviewer_response(&request,true))
            } else if request.contains("SPA/API Mapper") {
                (200,"application/json",proposal_model_response(
                    r#"{"summary":"actual paid Mapper receipt","priorityContracts":[],"risks":[]}"#))
            } else if request.contains("Original fixture Evidence Reviewer") {
                (200,"application/json",proposal_model_response(
                    r#"{"verdict":"insufficient_evidence","reasonCodes":["missing_control"],"missingEvidence":["missing control"],"confidence":0.2,"summary":"reviewer needs control"}"#))
            } else { multi_agent_original_fake_response(request) }
        }))
    };
    let mut context = test_context(&db_path, &target,
        vec![AgentIdentity::scoped("session-a"), AgentIdentity::scoped("session-b")]);
    context.scan_id = scan_id.into();
    context.attempt_number = attempt;
    context.execution_plan = plan;
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    if reject_review {
        // Part of the frozen frontend evidence before the Root's first tick.
        context.evidence["forceReviewerRejectedForTest"] = serde_json::json!(true);
    }
    let run = runtime_open_run(&db_path, scan_id, &context.route).unwrap();
    let root_id_owned = run.run_id.clone();
    let root_id: &str = &root_id_owned;
    let work: String = connection.query_row(
        "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt], |r| r.get(0)).unwrap();
    context.target_dir = PathBuf::from(work).join("url-pipeline/target-00000");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"), &context.scan_id).unwrap();
    fs::write(context.target_dir.join("frontend-evidence.json"), context.evidence.to_string()).unwrap();
    context.run = Some(run);
    bind_agent_evidence_location(&context).unwrap();
    native_frozen_web_root_mode(&context).unwrap();
    let _transport = RealSpecialistTransport::enter();
    let mut session = multi_agent_prepare(&mut context).unwrap();
    let outcome = AgentTargetOutcome::incomplete("fixture executor finished without a finding");
    // The original Web request has a real invoice and owner/SDK exit before
    // Authorization uses its registered controls. No Web target tool is issued.
    let web_outcome = run_native_agent(&context);
    assert!(matches!(web_outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{web_outcome:?}");
    multi_agent_finish_execution(&context, &mut session, &web_outcome).unwrap();
    assert_eq!(model_seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
    let web_usage: (i64,i64,i64) = connection.query_row(
        "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(web_usage,(50,0,1),"only the real original Web invoice is settled");
    let coverage: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND stage='native-agent-coverage'",
        [scan_id], |r| r.get(0)).unwrap();
    assert_eq!(coverage,1,"bounded Web exit writes its own coverage, not an Authorization finding");
    // The target listener is bound before the scan is created (its URL is
    // frozen into the draft); its accept window starts only now, after the
    // born Root's prepare/executor ticks, so load cannot expire it early.
    let target_seen: SpecialistTestSeen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let server_seen = target_seen.clone();
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        let mut seen = Vec::new();
        while seen.len() < if pause_after_probe > 0 { pause_after_probe } else if disconnect_probe > 0 { disconnect_probe } else { 3 }
            && std::time::Instant::now() < deadline {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(10)); continue;
            };
            // Accepted sockets may inherit the listener's nonblocking mode on
            // some platforms; the fixture expects to read a complete request.
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut buffer = [0_u8; 4096];
            let length = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..length]);
            let first_line = request.lines().next().unwrap_or_default().to_string();
            let denied = deny_cross && first_line.contains("id=owner")
                && request.to_ascii_lowercase().contains("cookie-b");
            let object = if first_line.contains("id=tester") { "tester" } else { "owner" };
            let body = format!(r#"{{"order":{{"id":"{object}","name":"order-{object}"}}}}"#);
            let status = if denied { "403 Forbidden" } else { "200 OK" };
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            if pause_after_probe > 0 && seen.len() + 1 == pause_after_probe {
                db::open(&server_db_path).unwrap().execute(
                    "UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&server_scan_id],
                ).unwrap();
            }
            if disconnect_probe > 0 && seen.len() + 1 == disconnect_probe {
                // The request may have reached the target even though its
                // result is unknown; deliberately break a framed response.
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 999\r\nConnection: close\r\n\r\n{").unwrap();
            } else {
                stream.write_all(response.as_bytes()).unwrap();
            }
            server_seen.lock().unwrap().push(first_line.clone());
            seen.push(first_line);
        }
        seen
    });
    let before: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND stage<>'native-agent-coverage'", [scan_id], |row| row.get(0)).unwrap();
    assert_eq!(before, 0);
    if pause_on_candidate {
        connection.execute_batch(&format!("CREATE TRIGGER pause_authorization_candidate BEFORE INSERT ON agent_finding_candidates \
            BEGIN UPDATE sentinel_scans SET status='paused' WHERE id='{scan_id}'; END;")).unwrap();
    }
    if fail_mailbox || pause_after_probe > 0 || disconnect_probe > 0 || pause_on_candidate {
        if fail_mailbox {
            connection.execute_batch(
                "CREATE TRIGGER reject_authorization_result BEFORE INSERT ON agent_messages \
                 WHEN NEW.kind='authorization_result' BEGIN SELECT RAISE(ABORT,'authorization mailbox unavailable'); END;",
            ).unwrap();
        }
        let error = multi_agent_authorization(&context, &mut session).unwrap_err();
        if fail_mailbox {
            assert!(error.contains("authorization mailbox unavailable"), "{error}");
        } else if pause_on_candidate {
            assert!(error.contains("authorization_attempt_cancelled"), "{error}");
        } else if disconnect_probe > 0 {
            assert!(error.contains("authorization_response_read_failed")
                || error.contains("authorization_request_failed"), "{error}");
        } else {
            assert_eq!(error, "agent_attempt_not_active");
        }
        let state: String = connection.query_row(
            "SELECT state FROM agent_assignments WHERE role='authorization' AND coordinator_run_id=?1",
            [root_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(state, "failed");
        let lanes: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_lane_leases WHERE scan_id=?1 AND lane='target_touching'",
            [scan_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(lanes, 0);
        let reserved: i64 = connection.query_row(
            "SELECT reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [root_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(reserved, 0);
        let published: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND stage<>'native-agent-coverage'",
            [scan_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(published, 0);
        if pause_after_probe > 0 {
            let late_results: i64 = connection.query_row(
                "SELECT COUNT(*) FROM agent_messages WHERE root_run_id=?1 AND kind='authorization_result'",
                [root_id], |row| row.get(0),
            ).unwrap();
            assert_eq!(late_results, 0, "pause must not publish a normal Authorization result");
        }
        let claims: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE scan_id=?1",
            [scan_id], |row| row.get(0),
        ).unwrap();
        let expected_claims = if pause_after_probe > 0 { pause_after_probe as i64 }
            else if disconnect_probe > 0 { disconnect_probe as i64 } else { 3 };
        assert_eq!(claims, expected_claims);
        if pause_after_probe > 0 || disconnect_probe > 0 || pause_on_candidate {
            let pending: i64 = connection.query_row(
                "SELECT COUNT(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND status='pending'",
                [root_id], |row| row.get(0),
            ).unwrap();
            assert_eq!(pending, 0);
        }
        if disconnect_probe > 0 {
            let failed_side: (String, String) = connection.query_row(
                "SELECT side,artifact_id FROM agent_authorization_probe_claims \
                 WHERE scan_id=?2 AND side=?1", [["owner", "cross", "tester"][disconnect_probe - 1], scan_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).unwrap();
            assert_eq!(failed_side.0, ["owner", "cross", "tester"][disconnect_probe - 1]);
            assert!(failed_side.1.is_empty(), "an incomplete response must not be usable evidence");
            let _ = multi_agent_authorization(&context, &mut session).unwrap_err();
            let after_retry: i64 = connection.query_row(
                "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE scan_id=?1",
                [scan_id], |row| row.get(0),
            ).unwrap();
        assert_eq!(after_retry, expected_claims, "a claimed network call must not be reissued in the same attempt");
            let usage = agent_request_accounting(&connection,&context.scan_id,attempt,&context.target_url).unwrap();
            assert_eq!(usage.authorization.unresolved,1);
            assert_eq!(usage.authorization.received,expected_claims-1);
            assert_eq!(usage.budget_committed,expected_claims);
        }
        assert_eq!(server.join().unwrap().len(), expected_claims as usize);
        return;
    }
    multi_agent_authorization(&context, &mut session).unwrap();
    let model_usage: (i64,i64) = connection.query_row(
        "SELECT a.reserved_requests,r.used_requests FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.role='authorization' AND a.coordinator_run_id=?1",
        [root_id],|r|Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert_eq!(model_usage,(0,0),"target traffic must not spend model requests");
    let pending: i64 = connection.query_row("SELECT COUNT(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND status='pending'", [root_id], |row| row.get(0)).unwrap();
    assert_eq!(pending, if deny_cross { 0 } else { 1 });
    let before_review: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND stage<>'native-agent-coverage'", [scan_id], |row| row.get(0)).unwrap();
    assert_eq!(before_review, 0);
    if pause_before_review {
        connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [scan_id]).unwrap();
        let original_rows=web_mode_test_rows(&connection);
        let sdk_calls=model_seen.lock().unwrap().len();
        let stopped=multi_agent_review(&context, &mut session, outcome);
        assert!(matches!(stopped,AgentTargetOutcome::Cancelled),"{stopped:?}");
        web_mode_assert_rows(&connection,&original_rows);
        assert_eq!(model_seen.lock().unwrap().len(),sdk_calls,"paused review must not pay any SDK");
        let reviewers: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'",
            [root_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(reviewers, 0, "a paused attempt must not start a new Reviewer");
        let published: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND stage<>'native-agent-coverage'", [scan_id], |row| row.get(0)).unwrap();
        assert_eq!(published, 0, "a paused attempt must not publish after Authorization finished");
        assert_eq!(server.join().unwrap().len(), 3);
        return;
    }
    if expire_reviewer && fault == AuthorizationReviewFault::None {
        EXPIRE_RECEIVED_REVIEWER_ONCE.with(|flag| assert!(!flag.replace(true)));
    }
    let reviewed = multi_agent_review(&context, &mut session, outcome.clone());
    if expire_reviewer && fault == AuthorizationReviewFault::None {
        EXPIRE_RECEIVED_REVIEWER_ONCE.with(|flag| assert!(!flag.replace(false)));
    }
    if fault != AuthorizationReviewFault::None {
        if matches!(fault, AuthorizationReviewFault::OriginalAckDuringReviewerSdk
            | AuthorizationReviewFault::OriginalAckColdDuringReviewerSdk) {
            let (message, ack) = original_ack.lock().unwrap().clone().expect("the actual provider must inject the original ACK drift");
            assert!(!ack.is_empty());
            assert_eq!(connection.query_row("SELECT acknowledged_at FROM agent_messages WHERE id=?1", [&message], |row| row.get::<_,String>(0)).unwrap(), "");
        }
        let source = publication_source.lock().unwrap().clone();
        if let Some(rows) = &source {
            let tx = connection.unchecked_transaction().unwrap();
            for row in rows { row.verify(&tx).unwrap(); }
            tx.commit().unwrap();
        }
        audit(&context, &mut session, &connection, &model_seen, &target_seen, &reviewed);
        if let Some(rows) = &source {
            let tx = connection.unchecked_transaction().unwrap();
            for row in rows { row.verify(&tx).unwrap(); }
            tx.commit().unwrap();
        }
        assert_eq!(server.join().unwrap(), vec![
            "GET /api/orders?id=owner HTTP/1.1", "GET /api/orders?id=owner HTTP/1.1",
            "GET /api/orders?id=tester HTTP/1.1",
        ]);
        return;
    }
    let published: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND record_json LIKE '%authorization_cross_object_read%'", [scan_id], |row| row.get(0)).unwrap();
    assert_eq!(published, if deny_cross || reject_review { 0 } else { 1 });
    let mut audit = Some(audit);
    if !audit_after_replay {
        audit.take().unwrap()(&context, &mut session, &connection, &model_seen, &target_seen, &reviewed);
    }
    if reject_review {
        let decision: (String, String, String, String) = connection.query_row(
            "SELECT q.status,d.verdict,fc.status,a.state FROM agent_review_requests q \
             JOIN agent_review_decisions d ON d.id=q.decision_id \
             JOIN agent_finding_candidates fc ON fc.root_run_id=q.root_run_id AND fc.candidate_revision=q.candidate_revision \
             JOIN agent_assignments a ON a.id=q.assignment_id WHERE q.root_run_id=?1",
            [root_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
        ).unwrap();
        assert_eq!(decision, ("rejected".into(), "rejected".into(), "rejected".into(), "completed".into()));
        let status = native_scan_status(&connection, scan_id).unwrap();
        assert_eq!(status["reviewGateSatisfied"], true);
        assert_eq!(status["reviewStatus"], "rejected");
        // The actual Web SDK exits without a target-tool invocation; the
        // orchestration must remain unready even though review closed.
        assert_eq!(status["multiAgentReady"], false);
        connection.execute("UPDATE agent_messages SET acknowledged_at='' WHERE root_run_id=?1 AND kind='review_decision'", [root_id]).unwrap();
        let unacknowledged = native_scan_status(&connection, scan_id).unwrap();
        assert_eq!(unacknowledged["reviewGateSatisfied"], false);
        assert_eq!(unacknowledged["multiAgentReady"], false);
        connection.execute("UPDATE agent_messages SET acknowledged_at=datetime('now','localtime') WHERE root_run_id=?1 AND kind='review_decision'", [root_id]).unwrap();
        connection.execute("UPDATE agent_finding_candidates SET candidate_revision=candidate_revision+1 WHERE root_run_id=?1", [root_id]).unwrap();
        assert_eq!(native_scan_status(&connection, scan_id).unwrap()["reviewGateSatisfied"], false,
            "a stale decision must not satisfy a newer candidate revision");
        connection.execute("UPDATE agent_finding_candidates SET candidate_revision=candidate_revision-1 WHERE root_run_id=?1", [root_id]).unwrap();
        connection.execute("UPDATE agent_assignments SET state='failed' WHERE coordinator_run_id=?1 AND role='evidence_reviewer'", [root_id]).unwrap();
        assert_eq!(native_scan_status(&connection, scan_id).unwrap()["reviewGateSatisfied"], false,
            "a failed child cannot turn its historical decision into a completed review");
    }
    let replayed = multi_agent_review(&context, &mut session, outcome);
    assert!(matches!(replayed, AgentTargetOutcome::Incomplete(_)), "{replayed:?}");
    let reviews: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'",
        [root_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(reviews, if deny_cross { 0 } else { 1 },
        "denied control has no candidate to review; publication must not trigger a second Reviewer call");
    let roles: (i64, i64) = connection.query_row(
        "SELECT COUNT(CASE WHEN role='authorization' AND terminal_state='completed' THEN 1 END), \
         COUNT(CASE WHEN role='evidence_reviewer' AND terminal_state='completed' THEN 1 END) \
         FROM agent_runs WHERE root_run_id=?1", [root_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(roles, (1, if deny_cross { 0 } else { 1 }));
    let acknowledged: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_messages WHERE root_run_id=?1 AND kind IN ('authorization_result','review_decision') AND acknowledged_at<>''",
        [root_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(acknowledged, if deny_cross { 1 } else { 2 });
    // Authority-revocation audits run after the normal replay assertions, so
    // intentionally expiring C cannot invalidate the preceding happy path.
    if let Some(audit) = audit {
        audit(&context, &mut session, &connection, &model_seen, &target_seen, &reviewed);
    }
    assert_eq!(server.join().unwrap(), vec![
        "GET /api/orders?id=owner HTTP/1.1", "GET /api/orders?id=owner HTTP/1.1",
        "GET /api/orders?id=tester HTTP/1.1",
    ]);
}

#[test]
fn authorization_identity_binding_accepts_validated_accounts_plus_only_the_fixed_anonymous_control() {
    let validated = || vec!["session-b".to_string(), "session-a".to_string()];
    let a = AgentIdentity::scoped("session-a");
    let b = AgentIdentity::scoped("session-b");
    let control = AgentIdentity::anonymous();
    // Production shape after multi_agent_prepare: accounts + one fixed control.
    assert_eq!(authorization_identity_binding(&[a.clone(), b.clone(), control.clone()], validated(), "session-a"), Ok(()));
    assert_eq!(authorization_identity_binding(&[control.clone(), b.clone(), a.clone()], validated(), "session-b"), Ok(()));
    assert_eq!(authorization_identity_binding(&[a.clone(), b.clone()], validated(), "session-b"), Ok(()));
    let denied = Err("authorization_identity_binding_denied".to_string());
    let impostor_control = AgentIdentity { key: "anonymous".into(), session_id: Some("session-a".into()), anonymous: false };
    let flagged_account = AgentIdentity { key: "session-c".into(), session_id: Some("session-c".into()), anonymous: true };
    let mismatched = AgentIdentity { key: "session-a".into(), session_id: Some("session-b".into()), anonymous: false };
    for (bound, identity) in [
        (vec![a.clone(), b.clone(), AgentIdentity::scoped("session-forged")], "session-a"),
        (vec![a.clone(), control.clone()], "session-a"),
        (vec![a.clone(), b.clone(), control.clone()], "anonymous"),
        (vec![a.clone(), b.clone(), control.clone()], "session-forged"),
        (vec![a.clone(), b.clone(), control.clone(), control.clone()], "session-a"),
        (vec![a.clone(), b.clone(), impostor_control], "session-a"),
        (vec![a.clone(), b.clone(), flagged_account], "session-a"),
        (vec![mismatched, b.clone()], "session-a"),
        (vec![control.clone()], "anonymous"),
    ] {
        assert_eq!(authorization_identity_binding(&bound, validated(), identity), denied, "{bound:?} {identity}");
    }
    assert_eq!(authorization_identity_binding(&[control], Vec::new(), "anonymous"), denied);
}

#[test]
fn authorization_side_receipt_schema_is_append_only_without_backfill() {
    let (root, db_path, _app_data_dir, _input) = authorization_fixture();
    let db = db::open(&db_path).unwrap();
    db.execute("DROP TABLE agent_authorization_side_receipts", []).unwrap();
    drop(db);
    assert_eq!(db::initialize(&root).unwrap(), db_path);
    let db = db::open(&db_path).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_authorization_side_receipts", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    // These synthetic bytes test SQL immutability only, never issuer validity.
    let insert = "INSERT INTO agent_authorization_side_receipts(rowid,receipt_id,root_run_id,scan_id,attempt_number,target_url,contract_key,side,claim_rowid,assignment_id,child_run_id,worker_attempt_id,artifact_id,fact_json,fact_hash,signature) VALUES(10,'receipt','root','scan',1,'target','contract','owner',11,'assignment','child','worker','0001.json','{}',printf('%064d',0),zeroblob(32))";
    assert!(db.execute(&insert.replace("VALUES(10", "VALUES(-1"), []).is_err());
    db.execute(insert, []).unwrap();
    let snapshot: String = db.query_row("SELECT json_array(rowid,receipt_id,fact_json,fact_hash,hex(signature),created_at) FROM agent_authorization_side_receipts", [], |r| r.get(0)).unwrap();
    db.pragma_update(None, "recursive_triggers", false).unwrap();
    for mutation in [
        "UPDATE agent_authorization_side_receipts SET fact_json='{\"changed\":true}'",
        "DELETE FROM agent_authorization_side_receipts",
    ] {
        assert!(db.execute(mutation, []).is_err());
    }
    for candidate in [
        insert.replace("VALUES(10", "VALUES(12").replace("'owner'", "'cross'").replace(",11,", ",12,").replace("'0001.json'", "'0002.json'"),
        insert.replace("VALUES(10", "VALUES(12").replace("'receipt'", "'new-receipt'").replace(",11,", ",12,").replace("'0001.json'", "'0002.json'"),
        insert.replace("'receipt'", "'new-receipt'").replace("'owner'", "'cross'").replace(",11,", ",12,").replace("'0001.json'", "'0002.json'"),
        insert.replace("VALUES(10", "VALUES(12").replace("'receipt'", "'new-receipt'").replace("'owner'", "'cross'").replace("'0001.json'", "'0002.json'"),
        insert.replace("VALUES(10", "VALUES(12").replace("'receipt'", "'new-receipt'").replace("'owner'", "'cross'").replace(",11,", ",12,"),
    ] {
        for resolution in ["INSERT OR REPLACE INTO", "INSERT OR IGNORE INTO"] {
            assert!(db.execute(&candidate.replacen("INSERT INTO", resolution, 1), []).is_err(), "{candidate}");
        }
        assert_eq!(db.query_row("SELECT json_array(rowid,receipt_id,fact_json,fact_hash,hex(signature),created_at) FROM agent_authorization_side_receipts", [], |r| r.get::<_,String>(0)).unwrap(), snapshot);
    }
    let automatic = insert.replace("(rowid,receipt_id", "(receipt_id").replace("VALUES(10,", "VALUES(")
        .replace("'receipt'", "'independent-receipt'").replace("'owner'", "'cross'")
        .replace(",11,", ",12,").replace("'0001.json'", "'0002.json'");
    assert_eq!(db.execute(&automatic, []).unwrap(), 1, "automatic positive rowid must admit an independent original receipt");
    assert!(db.query_row("SELECT storage_id FROM agent_authorization_side_receipts WHERE receipt_id='independent-receipt'", [], |r| r.get::<_,i64>(0)).unwrap() > 0);
    drop(db);
    assert_eq!(db::initialize(&root).unwrap(), db_path);
    let db = db::open(&db_path).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_authorization_side_receipts", [], |r| r.get::<_,i64>(0)).unwrap(), 2, "reinitialization cannot backfill or replace original receipts");
}
