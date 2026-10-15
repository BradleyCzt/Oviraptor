// Fresh captured draft and signed Multi task, actual original role SDKs,
// Web broker effects, original finance and owned result publication.
fn fresh_multi_anonymous_control_dispatch(
    revoke: bool,
) -> (AgentHarness, OwnedAgentTargetOutcome, usize) {
    fresh_multi_anonymous_control_dispatch_with_extra(revoke, 1)
}

// Revoking target identity stops execution, while the still-owned Root must
// retain one bounded assessment of the HTTP charge already received.
fn fresh_multi_anonymous_control_paid_calls_after_revocation(
    h: &AgentHarness,
    owned: &OwnedAgentTargetOutcome,
    before: usize,
) -> usize {
    assert!(before > 0, "the authenticated HTTP must reach the revocation boundary");
    let models = h.model_seen.lock().unwrap();
    let after = models.get(before..).unwrap();
    assert_eq!(after.len(), 1, "only the original HTTP-cost advisory may follow revocation");
    let wire = &after[0];
    assert!(fresh_multi_wire_system(wire).contains("You are the Root Coordinator"));
    assert!(!wire.contains("cookie-alpha") && !wire.contains("Bearer alpha"));
    let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
    let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
    let basis = &input["basis"];
    assert_eq!(basis["phase"], "http-budget-consumption");
    let fact = &basis["changedFact"]["semantic"];
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    assert_eq!(fact["rootRunId"], root.as_str());
    assert_eq!(fact["scanId"], h.context.scan_id.as_str());
    assert_eq!(fact["targetKey"], h.context.target_url.as_str());
    assert_eq!(fact["protocol"], "original_received_http_target_cost_advisory_v1");
    assert_eq!(fact["advisoryOnly"], true);
    assert_eq!(fact["executionGrant"], false);
    assert_eq!(fact["additionalScopeGranted"], false);
    assert_eq!(fact["targetRequestsGranted"], 0);
    assert_eq!(fact["targetRequests"], 1);
    assert_eq!(fact["tool"], "compare_identities");
    assert_eq!(fact["identityHandle"], "session-a");
    let db = db::open(&h.db_path).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    let receipt: i64 = snapshot.query_row(
        "SELECT count(*) FROM agent_http_budget_receipts WHERE root_run_id=?1 AND receipt_id=?2 AND source_id=?3 AND run_id=?4 AND scan_id=?5 AND attempt_number=1 AND target_url=?6 AND response_status=200",
        params![root, fact["receiptId"].as_str().unwrap(), fact["sourceId"].as_str().unwrap(),
            fact["childRunId"].as_str().unwrap(), h.context.scan_id, h.context.target_url],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(receipt, 1, "advisory must cite the original received HTTP bill");
    let target_fee: Vec<(String, i64)> = snapshot.prepare(
        "SELECT kind,amount FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 AND dimension='target_requests' ORDER BY rowid"
    ).unwrap().query_map(
        params![root, fact["assignmentId"].as_str().unwrap(), fact["workerAttemptId"].as_str().unwrap(), fact["sourceId"].as_str().unwrap()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(target_fee, vec![("reserve".into(), 1), ("forfeit".into(), 1), ("reconcile".into(), 1)],
        "the original target charge remains reserved, received and reconciled exactly once");
    let requests: Vec<(String, String)> = snapshot.prepare(
        "SELECT call_id,fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='http-budget-consumption'"
    ).unwrap().query_map([root], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
        .collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(requests.len(), 1, "one received HTTP bill has one original advisory request");
    let (call, raw) = &requests[0];
    let saved: JsonValue = serde_json::from_str(raw).unwrap();
    assert_eq!(saved["request"]["basis"]["phase"], basis["phase"]);
    assert_eq!(saved["request"]["basis"]["changedFact"], basis["changedFact"]);
    assert_eq!(saved["request"]["request"]["messages"], body["messages"]);
    let root_fee: (i64, i64) = snapshot.query_row(
        "SELECT count(*),coalesce(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id='' AND source_id=?2 AND dimension='model_requests' AND kind='consume'",
        params![root, call], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(root_fee, (1, 1), "the advisory adds exactly one paid Root request");
    let event: i64 = snapshot.query_row(
        "SELECT json_extract(fact_json,'$.eventSequence') FROM agent_root_tick_receipts WHERE root_run_id=?1 AND call_id=?2 AND phase='publication'",
        params![root, call], |row| row.get(0),
    ).unwrap();
    let publication = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(&snapshot, root, event).unwrap();
    assert_eq!(publication.record["advisoryOnly"], true);
    assert_eq!(publication.record["summary"]["suggestions"], json!(["assess:http_budget_consumption"]));
    assert!(publication.record.get("localTools").is_none());
    let web: (i64, i64) = snapshot.query_row(
        "SELECT r.used_requests,(SELECT count(*) FROM agent_web_model_journal j WHERE j.root_run_id=?1 AND j.child_run_id=r.id AND j.phase='received') FROM agent_runs r WHERE r.id=?2 AND r.role='web_executor'",
        params![root, fact["childRunId"].as_str().unwrap()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(web, (1, 1), "revocation cannot dispatch another Web SDK request");
    let total = before + 1;
    for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
        let balance = crate::agent_runtime::multi_agent::budget::balance(&snapshot, root, None, dimension).unwrap();
        assert_eq!((balance.reserved, balance.indeterminate), (0, 0));
        if dimension == "model_requests" { assert_eq!(balance.consumed, total as i64); }
        if dimension == "target_requests" { assert_eq!(balance.consumed, 2); }
    }
    snapshot.rollback().unwrap();
    total
}

fn fresh_multi_anonymous_control_dispatch_with_extra(
    revoke: bool,
    extra: usize,
) -> (AgentHarness, OwnedAgentTargetOutcome, usize) {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let binding = Arc::new(std::sync::Mutex::new(None::<(PathBuf, String, Seen)>));
    let site_binding = binding.clone();
    let calls_at_revocation = Arc::new(AtomicUsize::new(0));
    let site_calls = calls_at_revocation.clone();
    let mut h = fresh_web_production_harness_with_sessions(
        "multi-authenticated-anonymous-control",
        move |request| {
            if request.starts_with("GET / HTTP/") {
                assert!(!request.to_ascii_lowercase().contains("cookie:"));
                assert!(!request.to_ascii_lowercase().contains("authorization:"));
                return (
                    200,
                    "text/html",
                    "<html><body>public entry</body></html>".into(),
                );
            }
            assert!(request.starts_with("GET /api/profile HTTP/"));
            if request.contains("cookie-alpha") && request.contains("Bearer alpha") {
                if revoke {
                    let (path, scan, models) = site_binding.lock().unwrap().clone().unwrap();
                    site_calls.store(models.lock().unwrap().len(), Ordering::SeqCst);
                    let db = db::open(&path).unwrap();
                    assert_eq!(db.execute("UPDATE browser_auth_sessions SET expires_at='2000-01-01T00:00:00Z' WHERE id='session-a' AND owner_scan_id=?1", [scan]).unwrap(), 1);
                }
                (
                    200,
                    "application/json",
                    r#"{"userId":"self","role":"member"}"#.into(),
                )
            } else {
                (
                    401,
                    "application/json",
                    r#"{"error":"login_required"}"#.into(),
                )
            }
        },
        &[("session-a", "cookie-alpha", "Bearer alpha")],
        "multi",
    );
    let config_target = h.context.target_url.clone();
    let executor_round = AtomicUsize::new(0);
    let (port, seen, stop) = spawn_endpoint(Arc::new(move |wire| {
        let system = fresh_multi_wire_system(&wire);
        let response = if system.contains("You are the Root Coordinator") {
            let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue = serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            if input["basis"]["phase"] == "budget-consumption" {
                proposal_model_response(&json!({"schemaVersion":1,"observed":["original settled Web SDK invoice"],"missing":[],"suggestions":["assess:budget_consumption"],"costNotes":[],"risks":[]}).to_string())
            } else if input["basis"]["phase"] == "http-budget-consumption" {
                proposal_model_response(&json!({"schemaVersion":1,"observed":["original sealed received HTTP fee"],"missing":[],"suggestions":["assess:http_budget_consumption"],"costNotes":[],"risks":[]}).to_string())
            } else if wire.contains("client-side-output") {
                proposal_model_response(&json!({"schemaVersion":1,"observed":["original configuration only"],"missing":["missing_browser_validation"],"suggestions":["assess:client_side_configuration"],"costNotes":[],"risks":[]}).to_string())
            } else if wire.contains("identity-session-output") {
                proposal_model_response(&json!({"schemaVersion":1,"observed":["one captured account"],"missing":["no target comparison in metadata"],"suggestions":["assess:identity_session_metadata"],"costNotes":[],"risks":[]}).to_string())
            } else {
                fresh_multi_root_wire_response(&wire).unwrap()
            }
        } else if system.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"frozen authenticated frontend map","priorityContracts":[],"risks":[]}"#,
            )
        } else if system.contains("IdentitySession") {
            proposal_model_response(
                r#"{"summary":"one validated captured account","observedAuthentication":["one handle"],"evidenceGaps":["missing actual comparison"],"authorizationProven":false}"#,
            )
        } else if system.contains("External Surface") {
            proposal_model_response(public_surface_model_text())
        } else if system.contains("独立ClientSide配置事实专家") {
            let body: JsonValue =
                serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let refs: Vec<_> = input["observations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|o| o["id"].clone())
                .collect();
            proposal_model_response(&json!({"summary":"readonly configuration, no impact proof","observationRefs":refs,"gaps":["missing_browser_validation"],"candidates":[]}).to_string())
        } else if executor_round.fetch_add(1, Ordering::SeqCst) == 0 {
            let mut calls = vec![(
                "compare_identities",
                json!({"leftIdentity":"session-a", "rightIdentity":"anonymous", "method":"GET", "path":"/api/profile", "family":"authentication_session"}),
            )];
            if !revoke {
                for index in 0..extra {
                    let mut request = json!({"identity":"anonymous", "method":"GET", "url":format!("{}/",config_target.trim_end_matches('/')), "family":"authentication_session"});
                    if extra>1 {
                        request["headers"]=json!({"X-Oviraptor-Fixture-Observation":index.to_string()});
                    }
                    calls.push(("replay_http", request));
                }
            }
            model_round(&calls, 100)
        } else {
            model_round(
                &[(
                    "finish_target",
                    json!({"coverage":closing_ledger(&["authentication_session"]),"stopReason":"anonymous control is denied by the authenticated endpoint"}),
                )],
                100,
            )
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    freeze_fresh_multi_production_harness(&mut h);
    *binding.lock().unwrap() = Some((
        h.db_path.clone(),
        h.context.scan_id.clone(),
        h.model_seen.clone(),
    ));
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: h.context.route.clone(),
        target_dir: h.context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    let settings = json!({});
    let _real = RealSpecialistTransport::enter();
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
    stop.store(true, Ordering::SeqCst);
    let calls = calls_at_revocation.load(Ordering::SeqCst);
    let calls = if revoke {
        fresh_multi_anonymous_control_paid_calls_after_revocation(&h, &owned, calls)
    } else {
        calls
    };
    (h, owned, calls)
}

#[test]
fn fresh_multi_authenticated_anonymous_control_actual_entry_preserves_account_metadata_and_fees() {
    let (h, owned, _) = fresh_multi_anonymous_control_dispatch(false);
    let requests = h.site_seen.lock().unwrap();
    let profiles: Vec<_> = requests
        .iter()
        .filter(|r| r.starts_with("GET /api/profile HTTP/"))
        .collect();
    assert_eq!(
        profiles.len(),
        2,
        "actual Multi must retain its credential-free control; total HTTP={} SDK={} outcome={}",
        requests.len(),
        h.model_seen.lock().unwrap().len(),
        owned.outcome.detail()
    );
    assert_eq!(requests.len(), 4, "the extra anonymous Broker configuration capture is real");
    let authenticated: Vec<_> = profiles
        .iter()
        .filter(|r| r.contains("cookie-alpha"))
        .collect();
    assert_eq!(authenticated.len(), 1);
    assert!(authenticated[0].contains("Bearer alpha"));
    let anonymous: Vec<_> = profiles
        .iter()
        .filter(|r| !r.contains("cookie-alpha"))
        .collect();
    assert_eq!(anonymous.len(), 1);
    assert!(!anonymous[0].to_ascii_lowercase().contains("cookie:"));
    assert!(!anonymous[0].to_ascii_lowercase().contains("authorization:"));
    drop(requests);
    let models = h.model_seen.lock().unwrap();
    for wire in models.iter() {
        assert!(!wire.contains("cookie-alpha") && !wire.contains("Bearer alpha"));
        let system = fresh_multi_wire_system(wire);
        if system.contains("SPA/API Mapper") || system.contains("IdentitySession") {
            let body: JsonValue =
                serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            assert_eq!(input["identityMode"], "SingleIdentity");
            assert_eq!(input["identityCount"], 1);
        }
    }
    let diagnostic_db = db::open(&h.db_path).unwrap();
    let diagnostic_usage = diagnostic_db.prepare("SELECT role,hard_request_budget,used_requests,status FROM agent_runs ORDER BY rowid").unwrap()
        .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?))).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    for role in [
        "Root Coordinator",
        "SPA/API Mapper",
        "IdentitySession",
        "External Surface",
        "独立ClientSide配置事实专家",
    ] {
        assert!(
            models
                .iter()
                .any(|w| fresh_multi_wire_system(w).contains(role)),
            "actual SDK missing {role}; outcome={}; SDK={}; originalUsage={:?}",
            owned.outcome.detail(), models.len(), diagnostic_usage
        );
    }
    let calls = models.len();
    drop(models);
    assert!(
        matches!(
            owned.outcome,
            AgentTargetOutcome::Completed(_) | AgentTargetOutcome::BoundedCompleted(_)
        ),
        "{}",
        owned.outcome.detail()
    );
    let db = db::open(&h.db_path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    let independent: (i64,i64) = db.query_row(
        "SELECT count(*),count(DISTINCT json_extract(q.fact_json,'$.request.basis.changedFact.semantic.binding.evidenceId'))
         FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p
          ON p.call_id=q.call_id AND p.root_run_id=q.root_run_id AND p.phase='publication'
         WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='new-evidence'",
        [root], |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert_eq!(independent,(3,3),"each of the three real Web HTTP effects retains a separate paid Root publication");
    assert_eq!(
        native_frozen_web_root_mode(&h.context).unwrap(),
        crate::agent_runtime::web_mode::WebMode::Multi
    );
    let (mode, ids) = crate::auth_session::validated_scan_identities(
        &db,
        &h.context.scan_id,
        &h.context.target_url,
    )
    .unwrap();
    assert_eq!(mode, crate::auth_session::ScanIdentityMode::SingleIdentity);
    assert_eq!(ids, vec!["session-a"]);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM browser_auth_sessions WHERE owner_scan_id=?1",
            [&h.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
        let b =
            crate::agent_runtime::multi_agent::budget::balance(&db, root, None, dimension).unwrap();
        assert_eq!((b.reserved, b.indeterminate), (0, 0));
        if dimension == "model_requests" {
            assert_eq!(b.consumed, calls as i64);
        }
        if dimension == "target_requests" {
            assert_eq!(b.consumed, 4);
        }
    }
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
    let before = single_finally_physical(&db);
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(single_finally_physical(&db), before);
    assert_eq!(tally.counted(), 1);
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 4);
}

#[test]
fn fresh_multi_anonymous_control_mid_http_revocation_preserves_paid_facts_and_blocks_followups() {
    let (h, owned, paid_calls) = fresh_multi_anonymous_control_dispatch(true);
    assert!(paid_calls > 0);
    assert_eq!(
        h.site_seen.lock().unwrap().len(),
        2,
        "one anonymous public capture plus one authenticated comparison request"
    );
    assert_eq!(
        h.model_seen.lock().unwrap().len(),
        paid_calls,
        "only the original bounded HTTP-cost advisory may follow revocation; no execution followups"
    );
    assert_eq!(
        owned.outcome.terminal_code(),
        terminal_code::EXECUTION_AUTHORIZATION_DENIED,
        "{}",
        owned.outcome.detail()
    );
    let db = db::open(&h.db_path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
        let b =
            crate::agent_runtime::multi_agent::budget::balance(&db, root, None, dimension).unwrap();
        assert_eq!((b.reserved, b.indeterminate), (0, 0));
        if dimension == "model_requests" {
            assert_eq!(b.consumed, paid_calls as i64);
        }
        if dimension == "target_requests" {
            assert_eq!(b.consumed, 2);
        }
    }
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    let status: String = db
        .query_row(
            "SELECT status FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
            params![h.context.scan_id, h.context.target_url],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "paused");
    assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
    let before = single_finally_physical(&db);
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(single_finally_physical(&db), before);
    assert_eq!(tally.counted(), 1);
    assert_eq!(h.model_seen.lock().unwrap().len(), paid_calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
}

#[test]
fn fresh_multi_anonymous_control_original_usage_rejects_imported_receipt_corruption_without_writes()
{
    use crate::agent_runtime::multi_agent::{budget::web_lifetime, lease::CoordinatorLease};
    let (h, owned, calls) = fresh_multi_anonymous_control_dispatch(true);
    assert_eq!(
        owned.outcome.terminal_code(),
        terminal_code::EXECUTION_AUTHORIZATION_DENIED
    );
    let db = db::open(&h.db_path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    // Read the original scope; this test never acquires or renews a lease.
    let lease = db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1", [root], |r| Ok(CoordinatorLease {
        scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
    })).unwrap();
    let (assignment,child,invoice):(String,String,String)=db.query_row("SELECT assignment_id,child_run_id,receipt_json FROM agent_web_model_journal WHERE root_run_id=?1 AND phase='received'",[root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let original = single_finally_physical(&db);
    assert!(web_lifetime::child_usage_original(&db, &lease, &assignment, &child).is_err());
    assert_eq!(single_finally_physical(&db), original);
    {
        let tx = db.unchecked_transaction().unwrap();
        let paid = web_lifetime::child_usage_original(&tx, &lease, &assignment, &child).unwrap();
        let receipt: JsonValue = serde_json::from_str(&invoice).unwrap();
        assert_eq!(paid.as_json(), receipt["usage"]);
        assert!(paid.input_tokens > 0 && paid.output_tokens > 0);
        let stored: (i64, i64, i64) = tx
            .query_row(
                "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
                [&child],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            stored,
            (
                paid.total_tokens,
                paid.cached_input_tokens,
                paid.model_requests
            )
        );
        // A denied tool may leave no new checkpoint; it cannot erase the bill.
        let checkpoint =
            NativeAgentState::read(&h.db_path, &h.context.scan_id, &h.context.target_url)
                .map(|s| s.token_usage)
                .unwrap_or_default();
        assert!(checkpoint.model_requests < paid.model_requests);
        for index in 0..7 {
            let mut foreign = lease.clone();
            match index {
                0 => foreign.scan_id.push_str("-foreign"),
                1 => foreign.attempt_number += 1,
                2 => foreign.target_key.push_str("foreign"),
                3 => foreign.root_run_id.push_str("-foreign"),
                4 => foreign.lease_epoch += 1,
                5 => foreign.fencing_token.push_str("-foreign"),
                _ => {}
            }
            let id = if index == 6 {
                "foreign-assignment"
            } else {
                &assignment
            };
            let before = single_finally_physical(&tx);
            assert!(
                web_lifetime::child_usage_original(&tx, &foreign, id, &child).is_err(),
                "scope {index}"
            );
            assert_eq!(single_finally_physical(&tx), before);
        }
        assert!(
            web_lifetime::child_usage_original(&tx, &lease, &assignment, "foreign-child").is_err()
        );
        tx.rollback().unwrap();
    }
    assert_eq!(single_finally_physical(&db), original);
    for column in [
        "root_run_id",
        "assignment_id",
        "child_run_id",
        "lease_epoch",
        "fencing_token",
        "round",
        "request_hash",
    ] {
        let tx = db.unchecked_transaction().unwrap();
        // Simulate imported corruption only in the isolated test database.
        tx.execute_batch("DROP TRIGGER web_model_journal_no_update")
            .unwrap();
        let replacement = if matches!(column, "lease_epoch" | "round") {
            "999"
        } else {
            "'foreign-original-binding'"
        };
        tx.execute(&format!("UPDATE agent_web_model_journal SET {column}={replacement} WHERE child_run_id=?1 AND phase='received'"),[&child]).unwrap();
        let before = single_finally_physical(&tx);
        assert!(
            web_lifetime::child_usage_original(&tx, &lease, &assignment, &child).is_err(),
            "receipt {column}"
        );
        assert_eq!(single_finally_physical(&tx), before);
        tx.rollback().unwrap();
        assert_eq!(single_finally_physical(&db), original);
    }
    for field in [
        "responseHash",
        "modelRequests",
        "cachedInputTokens",
        "usageReported",
    ] {
        let tx = db.unchecked_transaction().unwrap();
        tx.execute_batch("DROP TRIGGER web_model_journal_no_update")
            .unwrap();
        let mut receipt: JsonValue = serde_json::from_str(&invoice).unwrap();
        match field {
            "responseHash" => receipt[field] = json!("f".repeat(64)),
            "modelRequests" => receipt["usage"][field] = json!(0),
            "cachedInputTokens" => receipt["usage"][field] = json!(i64::MAX),
            _ => receipt[field] = json!(false),
        }
        tx.execute("UPDATE agent_web_model_journal SET receipt_json=?2 WHERE child_run_id=?1 AND phase='received'",params![child,receipt.to_string()]).unwrap();
        let before = single_finally_physical(&tx);
        let error =
            web_lifetime::child_usage_original(&tx, &lease, &assignment, &child).unwrap_err();
        if field == "usageReported" {
            assert_eq!(error, "budget_indeterminate_requires_reconciliation");
        }
        assert_eq!(single_finally_physical(&tx), before);
        tx.rollback().unwrap();
        assert_eq!(single_finally_physical(&db), original);
    }
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
}

#[test]
fn fresh_multi_http_supervision_capacity_refuses_batch_before_io_and_preserves_paid_web() {
    let (h, owned, _) = fresh_multi_anonymous_control_dispatch_with_extra(false, 10);
    assert!(matches!(owned.outcome, AgentTargetOutcome::Limited(_)), "{}", owned.outcome.detail());
    assert!(owned.outcome.detail().contains("后续 Root 监督"), "{}", owned.outcome.detail());
    assert!(!owned.outcome.detail().contains("executor_settlement_failed:"));
    let db = db::open(&h.db_path).unwrap();
    let web: (String,i64,i64,String) = db.query_row(
        "SELECT r.status,r.used_requests,r.used_tokens,a.budget_settled_at FROM agent_runs r JOIN agent_assignments a ON a.child_run_id=r.id WHERE r.role='web_executor'",
        [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).unwrap();
    assert_eq!((web.0.as_str(),web.1,web.2), ("terminal",1,140));
    assert!(!web.3.is_empty(), "post-exit failure cannot un-settle the actual Web invoice");
    let physical = h.site_seen.lock().unwrap().len();
    assert!(physical>3 && physical<13,"a batch proposes 12 Web requests, but only funded requests reach the actual server: {physical}");
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    let (claims,paid): (i64,i64) = db.query_row(
        "SELECT (SELECT count(*) FROM agent_http_request_claims c JOIN agent_runs r ON r.id=c.run_id WHERE r.root_run_id=?1 AND r.role='web_executor'),
         (SELECT count(*) FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q ON q.call_id=p.call_id AND q.root_run_id=p.root_run_id AND q.phase='request'
          WHERE p.root_run_id=?1 AND p.phase='publication' AND json_extract(q.fact_json,'$.request.basis.phase')='new-evidence')",
        [root], |r| Ok((r.get(0)?,r.get(1)?)),
    ).unwrap();
    assert_eq!(claims,physical as i64-1,"the initial frontend capture is not a Web claim");
    assert_eq!(paid,claims,"all admitted physical claims have independently paid supervision");
    assert_eq!(h.model_seen.lock().unwrap().iter().filter(|wire| fresh_multi_wire_system(wire).contains("独立ClientSide配置事实专家")).count(),1);
    for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
        let b=crate::agent_runtime::multi_agent::budget::balance(&db,root,None,dimension).unwrap();
        assert_eq!((b.reserved,b.indeterminate),(0,0));
        if dimension=="model_requests" {assert_eq!(b.consumed,h.model_seen.lock().unwrap().len() as i64);}
        if dimension=="target_requests" {assert_eq!(b.consumed,physical as i64);}
    }
}
