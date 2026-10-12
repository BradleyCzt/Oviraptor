// Actual ordinary creator/startup/HMAC and captured user-entry outcome.
// No ad-hoc Root financial issuer, inherited mode, or mutable-state terminal.
pub(super) fn fresh_single_production_harness(
    tag: &str,
    site: impl Fn(String) -> (u16, &'static str, String) + Send + Sync + 'static,
) -> AgentHarness {
    fresh_single_production_harness_with_sessions(tag, site, &[])
}
fn fresh_single_production_harness_with_sessions(
    tag: &str,
    site: impl Fn(String) -> (u16, &'static str, String) + Send + Sync + 'static,
    sessions: &[(&str, &str, &str)],
) -> AgentHarness {
    fresh_web_production_harness_with_sessions(tag, site, sessions, "single")
}
fn fresh_web_production_harness_with_sessions(
    tag: &str,
    site: impl Fn(String) -> (u16, &'static str, String) + Send + Sync + 'static,
    sessions: &[(&str, &str, &str)],
    mode: &str,
) -> AgentHarness {
    let (site_port, site_seen, _) = spawn_endpoint(std::sync::Arc::new(site));
    let (model_port, _, model_seen) = spawn_model(vec![]);
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("oviraptor-fresh-single-{tag}-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let mut db = db::open(&db_path).unwrap();
    db.execute(
        "INSERT INTO projects(id,name) VALUES(9001,'Fresh Single E2E')",
        [],
    )
    .unwrap();
    let target = format!("http://127.0.0.1:{site_port}");
    // Captured credentials are synthetic fixture material. Ownership transfer
    // and identity policy are created by the real draft transaction, not UPDATE.
    let draft_scope = Uuid::new_v4().to_string();
    let host = reqwest::Url::parse(&target)
        .unwrap()
        .host_str()
        .unwrap()
        .to_string();
    for (id, cookie, authorization) in sessions {
        let document = json!({"schemaVersion":1,"id":id,"projectId":9001,"entryUrl":target,
            "scopeHosts":[host],"cookies":[{"name":"session","value":cookie}],
            "headers":{"authorization":authorization},"identityIsolation":{"sharedWithOtherSessions":false}});
        db.execute("INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,draft_scope_id,name,entry_url,status,session_json,expires_at)
            VALUES(?1,9001,'',?2,?1,?3,'valid',?4,?5)",
            params![id,draft_scope,target,document.to_string(),(chrono::Utc::now()+chrono::Duration::hours(8)).to_rfc3339()]).unwrap();
    }
    let ids: Vec<String> = sessions.iter().map(|(id, _, _)| (*id).into()).collect();
    // Only the new creator's ID generator is overridden for old assertions.
    let scan = with_web_creator_test_id("agent-scan", || {
        create_sentinel_url_scan_with_mode_in(
            &db,
            9001,
            "Fresh Single E2E".into(),
            vec![target.clone()],
            Some("standard".into()),
            None,
            None,
            (!ids.is_empty()).then(|| ids.clone()),
            (!ids.is_empty()).then(|| draft_scope.clone()),
            None,
            None,
            None,
            Some(mode.into()),
        )
        .unwrap()
    });
    for id in &ids {
        let binding: (String, String) = db
            .query_row(
                "SELECT owner_scan_id,draft_scope_id FROM browser_auth_sessions WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(binding, (scan.id.clone(), String::new()));
    }
    let work = web_mode_test_start(&root, &mut db, &scan.id, WebStartMode::Confirm).unwrap();
    let (_, validated) =
        crate::auth_session::validated_scan_identities(&db, &scan.id, &target).unwrap();
    assert_eq!(validated, ids);
    let identities = if ids.is_empty() {
        vec![AgentIdentity::anonymous()]
    } else {
        ids.iter().map(AgentIdentity::scoped).collect()
    };
    let mut context = test_context(&db_path, &target, identities);
    context.scan_id = scan.id.clone();
    context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
    context.environment.api_key = "mock-key".into();
    context.target_dir = work.join("url-pipeline/target-00001");
    context.log_path = work.join("runner.log");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"), &scan.id).unwrap();
    fs::write(
        context.target_dir.join("frontend-evidence.json"),
        context.evidence.to_string(),
    )
    .unwrap();
    context.execution_plan = build_agent_execution_plan(
        &AgentBudgetSettings::from_json(&json!({})),
        &context.route,
        &context.environment,
        AgentBackendKind::Native,
        &db_path,
        &scan.id,
    )
    .with_attempt(1);
    AgentHarness {
        root,
        db_path,
        context,
        site_seen,
        model_seen,
    }
}
pub(super) fn freeze_fresh_single_production_harness(h: &mut AgentHarness) {
    let c = &mut h.context;
    persist_frozen_web_execution_plan(&h.db_path, &c.scan_id, 1, &c.target_url, &c.execution_plan)
        .unwrap();
    c.run = Some(runtime_open_run(&h.db_path, &c.scan_id, &c.route).unwrap());
    bind_agent_evidence_location(c).unwrap();
    assert_eq!(
        native_frozen_web_root_mode(c).unwrap(),
        crate::agent_runtime::web_mode::WebMode::Single
    );
    let db = db::open(&h.db_path).unwrap();
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(
        &db,
        &c.run.as_ref().unwrap().run_id,
    )
    .unwrap()
    .require_live(&db)
    .unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [&c.run.as_ref().unwrap().run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        10
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_coordinator_leases", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}
pub(super) fn execute_fresh_single_production_harness(h: &mut AgentHarness) -> OwnedAgentTargetOutcome {
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: h.context.route.clone(),
        target_dir: h.context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    let settings = json!({});
    let owned = run_agent_target(
        &prepared,
        AgentTargetExecution {
            db_path: &h.db_path,
            scan_id: &h.context.scan_id,
            attempt_number: 1,
            settings: &settings,
            environment: &h.context.environment,
            adaptive: &AgentBudgetSettings::from_json(&settings),
            log_path: &h.context.log_path,
        },
    )
    .unwrap();
    assert_eq!(
        owned.original_terminal.root_run_id.as_deref(),
        Some(h.context.run.as_ref().unwrap().run_id.as_str())
    );
    let db = db::open(&h.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_single_exit_receipts WHERE root_run_id=?1",
            [&h.context.run.as_ref().unwrap().run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        h.model_seen.lock().unwrap().len() as i64,
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &h.context.run.as_ref().unwrap().run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed
    );
    owned
}

pub(super) fn assert_fresh_single_surface(
    h: &AgentHarness,
    url: &str,
    status: &str,
    requests: i64,
    findings: usize,
) {
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let owner =
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, root).unwrap();
    owner.read_single_exit(&db).unwrap();
    let paid = |dimension| {
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), dimension).unwrap()
    };
    let model = paid("model_requests");
    let input = paid("model_input_tokens");
    let output = paid("model_output_tokens");
    for dimension in [model.clone(), input.clone(), output.clone()] {
        assert_eq!((dimension.reserved, dimension.indeterminate), (0, 0));
    }
    assert_eq!(model.consumed, h.model_seen.lock().unwrap().len() as i64);
    assert_eq!(paid("target_requests").consumed, requests);
    assert_agent_surface_with_original_usage(
        h,
        url,
        status,
        requests,
        findings,
        Some((model.consumed, input.consumed + output.consumed)),
    );
}

fn fresh_single_paid_callback_fixture(tag: &str) -> (AgentHarness, OwnedAgentTargetOutcome) {
    let mut h = fresh_single_production_harness(tag, mock_site);
    freeze_fresh_single_production_harness(&mut h);
    let url = format!("{}/api/orders", h.context.target_url);
    retarget_model(
        &mut h,
        vec![model_round(
            &[(
                "replay_http",
                json!({"identity":"anonymous", "method":"GET", "url":url, "family":"authorization"}),
            )],
            100,
        )],
    );
    let owned = execute_fresh_single_production_harness(&mut h);
    assert_eq!(h.site_seen.lock().unwrap().len(), 1);
    assert!(h.model_seen.lock().unwrap().len() > 1);
    (h, owned)
}
#[test]
fn fresh_single_paid_callback_foreign_target_preserves_all_original_rows_and_fees() {
    let (h, owned) = fresh_single_paid_callback_fixture("foreign-target");
    let db = db::open(&h.db_path).unwrap();
    let before = single_finally_physical(&db);
    let calls = h.model_seen.lock().unwrap().len();
    let mut foreign = h.context.route.clone();
    foreign.url.push_str("/foreign");
    let mut tally = AgentPipelineTally::default();
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &foreign,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 0);
    assert_eq!(single_finally_physical(&db), before);
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 1);
}
#[test]
fn fresh_single_paid_callback_replaced_attempt_preserves_all_original_rows_and_fees() {
    let (h, owned) = fresh_single_paid_callback_fixture("replaced-attempt");
    let db = db::open(&h.db_path).unwrap();
    // Isolated current-attempt rotation, not a full restart/recovery claim.
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
        [&h.context.scan_id],
    )
    .unwrap();
    let before = single_finally_physical(&db);
    let calls = h.model_seen.lock().unwrap().len();
    let mut tally = AgentPipelineTally::default();
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 0);
    assert_eq!(single_finally_physical(&db), before);
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 1);
}

fn assert_fresh_single_identity_wire(h: &AgentHarness) {
    let requests = h.site_seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for (cookie, authorization, other_cookie, other_authorization) in [
        ("cookie-alpha", "Bearer alpha", "cookie-beta", "Bearer beta"),
        ("cookie-beta", "Bearer beta", "cookie-alpha", "Bearer alpha"),
    ] {
        let wires: Vec<_> = requests.iter().filter(|raw| raw.contains(cookie)).collect();
        assert_eq!(wires.len(), 1, "exactly one request per captured identity");
        let wire = wires[0];
        assert!(wire.starts_with("GET /api/profile HTTP/"));
        assert!(wire.contains(authorization));
        assert!(!wire.contains(other_cookie) && !wire.contains(other_authorization));
    }
    drop(requests);
    let models = h.model_seen.lock().unwrap().join("\n");
    for secret in ["cookie-alpha", "cookie-beta", "Bearer alpha", "Bearer beta"] {
        assert!(
            !models.contains(secret),
            "captured credentials must not be model input"
        );
    }
}
#[test]
fn fresh_single_identity_policy_withdrawal_after_sdk_refuses_before_http_and_preserves_fee() {
    fresh_single_identity_revocation_after_sdk("policy-withdrawal");
}
#[test]
fn fresh_single_identity_expiry_after_sdk_refuses_before_http_and_preserves_fee() {
    fresh_single_identity_revocation_after_sdk("expiry");
}
#[test]
fn fresh_single_identity_foreign_owner_after_sdk_refuses_before_http_and_preserves_fee() {
    fresh_single_identity_revocation_after_sdk("foreign-owner");
}
#[test]
fn fresh_single_identity_host_change_after_sdk_refuses_before_http_and_preserves_fee() {
    fresh_single_identity_revocation_after_sdk("host-change");
}
#[test]
fn fresh_single_identity_reused_material_after_sdk_refuses_before_http_and_preserves_fee() {
    fresh_single_identity_revocation_after_sdk("reused-material");
}
#[test]
fn fresh_single_identity_missing_material_after_sdk_refuses_before_http_and_preserves_fee() {
    fresh_single_identity_revocation_after_sdk("missing-material");
}
#[test]
fn fresh_single_identity_corrupt_document_after_sdk_refuses_before_http_and_preserves_fee() {
    fresh_single_identity_revocation_after_sdk("corrupt-document");
}
#[test]
fn fresh_single_identity_null_document_after_sdk_refuses_before_http_and_preserves_fee() {
    fresh_single_identity_revocation_after_sdk("null-document");
}
fn fresh_single_identity_revocation_after_sdk(kind: &'static str) {
    use std::sync::{
        atomic::Ordering,
        Arc,
    };
    let mut h = fresh_single_production_harness_with_sessions(
        kind,
        equal_role_site,
        &[
            ("session-a", "cookie-alpha", "Bearer alpha"),
            ("session-b", "cookie-beta", "Bearer beta"),
        ],
    );
    freeze_fresh_single_production_harness(&mut h);
    let path = h.db_path.clone();
    let scan = h.context.scan_id.clone();
    let live_context = h.context.clone();
    let url = format!("{}/api/profile", h.context.target_url);
    let revocation: &'static str = match kind {
        "policy-withdrawal" => "UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.authSessionIds',json('[]'),'$.authSessionId','') WHERE scan_id=?1",
        "expiry" => "UPDATE browser_auth_sessions SET expires_at='2000-01-01T00:00:00Z' WHERE id='session-b' AND owner_scan_id=?1",
        "foreign-owner" => "UPDATE browser_auth_sessions SET owner_scan_id='other-task' WHERE id='session-b' AND owner_scan_id=?1",
        "host-change" => "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.scopeHosts',json('[\"example.invalid\"]')) WHERE id='session-b' AND owner_scan_id=?1",
        "reused-material" => "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.cookies',(SELECT json_extract(session_json,'$.cookies') FROM browser_auth_sessions WHERE id='session-a'),'$.headers',(SELECT json_extract(session_json,'$.headers') FROM browser_auth_sessions WHERE id='session-a')) WHERE id='session-b' AND owner_scan_id=?1",
        "missing-material" => "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.cookies',json('[]'),'$.headers',json('{}')) WHERE id='session-b' AND owner_scan_id=?1",
        "corrupt-document" => "UPDATE browser_auth_sessions SET session_json='{' WHERE id='session-b' AND owner_scan_id=?1",
        "null-document" => "UPDATE browser_auth_sessions SET session_json='null' WHERE id='session-b' AND owner_scan_id=?1",
        _ => panic!("unknown revocation fixture"),
    };
    // The original Root transport legitimately cancels an in-flight SDK call as
    // soon as its authority is gone (100ms probe; outcome unknown, reserve held
    // as indeterminate, reconciliation required). A withdrawal *committed* while
    // the response is still on the wire therefore races that probe under load.
    // This fixture exercises the other boundary, "bill received, response
    // refused": the withdrawal is applied on a held write transaction before the
    // response is written (invisible to the probe, which reads committed state)
    // and committed only after the client has consumed the response and closed
    // the connection; the original `received` bill write then waits for it, so
    // the post-receipt authority recheck deterministically sees the withdrawal.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let server = {
        let (seen, stop) = (seen.clone(), stop.clone());
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let respond = |stream: &mut std::net::TcpStream, body: String| {
                stream.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
                stream.flush().unwrap();
            };
            let mut changed = 0;
            while !stop.load(Ordering::SeqCst) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("model fixture: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                let Some(request) = read_http_request(&mut stream) else { continue };
                let first = seen.lock().unwrap().is_empty();
                seen.lock().unwrap().push(request);
                if !first {
                    respond(&mut stream, model_round(&[], 10));
                    continue;
                }
                // Actual SDK is already sent. Change only this temporary task's
                // authority, retaining both original captured session rows.
                let db = db::open(&path).unwrap();
                let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate).unwrap();
                changed = tx.execute(revocation, [&scan]).unwrap();
                assert_eq!(changed, 1);
                assert_eq!(
                    agent_authorize_tool_on(&tx, &live_context, "replay_http"),
                    Err("tool_identity_binding_denied")
                );
                respond(&mut stream, model_round(
                    &[(
                        "replay_http",
                        json!({"identity":"session-a","method":"GET","url":url,"family":"authorization"}),
                    )],
                    100,
                ));
                // The client closes only after it has consumed the whole response.
                stream.set_read_timeout(Some(std::time::Duration::from_secs(20))).unwrap();
                let mut rest = [0u8; 256];
                while matches!(stream.read(&mut rest), Ok(n) if n > 0) {}
                tx.commit().unwrap();
            }
            changed
        })
    };
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    let owned = execute_fresh_single_production_harness(&mut h);
    stop.store(true, Ordering::SeqCst);
    assert_eq!(server.join().unwrap(), 1, "the withdrawal was committed exactly once");
    assert_eq!(
        h.site_seen.lock().unwrap().len(),
        0,
        "withdrawn identity policy must deny before any target HTTP"
    );
    assert_eq!(
        h.model_seen.lock().unwrap().len(),
        1,
        "no further SDK after authority withdrawal"
    );
    assert_eq!(
        owned.outcome.terminal_code(),
        terminal_code::EXECUTION_AUTHORIZATION_DENIED
    );
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let owner =
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, root).unwrap();
    owner.read_single_exit(&db).unwrap();
    let balance = |dimension| {
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), dimension).unwrap()
    };
    for dimension in [
        "model_requests",
        "model_input_tokens",
        "model_output_tokens",
        "target_requests",
    ] {
        let paid = balance(dimension);
        assert_eq!((paid.reserved, paid.indeterminate), (0, 0));
    }
    assert_eq!(balance("model_requests").consumed, 1);
    assert!(balance("model_input_tokens").consumed > 0);
    assert!(balance("model_output_tokens").consumed > 0);
    assert_eq!(balance("target_requests").consumed, 0);
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
            params![h.context.scan_id, h.context.target_url],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    assert_eq!(tally.counted(), 1);
    assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
    // The executor returns directly at an authorization stop before committing
    // the round. Do not fabricate a resumable checkpoint after revocation.
    assert!(
        NativeAgentState::load(&h.db_path, &h.context.scan_id, &h.context.target_url)
            .unwrap()
            .is_none()
    );
    // Original Root transport saves the bill and rechecks authority before
    // publishing the response: the proposed tool never becomes an invocation.
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM tool_invocations WHERE run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        agent_authorize_tool(&h.context, "replay_http"),
        Err("coordinator_not_executable")
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM browser_auth_sessions WHERE project_id=9001",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}
