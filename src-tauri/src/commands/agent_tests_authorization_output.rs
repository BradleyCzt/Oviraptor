// Actual creator + two validated identities + controls saved before start.
// Every control pays exactly A->X, B->X and B->Y; no synthesized Auth result.
fn authorization_output_actual_fixture(deny_cross: Vec<bool>) -> BudgetTriggerFixture {
    authorization_output_actual_fixture_phase(deny_cross, false)
}

fn authorization_output_actual_fixture_phase(
    deny_cross: Vec<bool>,
    fail_auth: bool,
) -> BudgetTriggerFixture {
    authorization_output_actual_fixture_response(deny_cross, fail_auth, None)
}

fn authorization_output_actual_fixture_response(
    deny_cross: Vec<bool>,
    fail_auth: bool,
    local_tool: Option<&'static str>,
) -> BudgetTriggerFixture {
    authorization_output_actual_fixture_interruption(deny_cross, fail_auth, local_tool, None)
}

type AuthorizationOutputPause = std::sync::Arc<std::sync::Mutex<Option<(PathBuf, String, String)>>>;
fn authorization_output_actual_fixture_interruption(
    deny_cross: Vec<bool>,
    fail_auth: bool,
    local_tool: Option<&'static str>,
    pause: Option<AuthorizationOutputPause>,
) -> BudgetTriggerFixture {
    authorization_output_actual_fixture_prebirth(deny_cross, fail_auth, local_tool, pause, 80)
}

fn authorization_output_actual_fixture_prebirth(
    deny_cross: Vec<bool>,
    fail_auth: bool,
    local_tool: Option<&'static str>,
    pause: Option<AuthorizationOutputPause>,
    hard_requests: i64,
) -> BudgetTriggerFixture {
    authorization_output_actual_fixture_limits(
        deny_cross,
        fail_auth,
        local_tool,
        pause,
        hard_requests,
        120000,
    )
}

fn authorization_output_actual_fixture_limits(
    deny_cross: Vec<bool>,
    fail_auth: bool,
    local_tool: Option<&'static str>,
    pause: Option<AuthorizationOutputPause>,
    hard_requests: i64,
    hard_tokens: i64,
) -> BudgetTriggerFixture {
    let groups = deny_cross.len();
    assert!((1..=4).contains(&groups));
    let (site_port, site_seen, _) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let first = wire.lines().next().unwrap();
        let index = (0..groups)
            .find(|i| first.contains(&format!("/api/orders{i}?")))
            .unwrap();
        assert!(first.starts_with("GET "));
        let tester = first.contains(&format!("id=tester{index}"));
        let denied = deny_cross[index] && !tester && wire.to_ascii_lowercase().contains("cookie-b");
        let object = if tester {
            format!("tester{index}")
        } else {
            format!("owner{index}")
        };
        (
            if denied { 403 } else { 200 },
            "application/json",
            json!({"order":{"id":object,"name":format!("order-{object}")}}).to_string(),
        )
    }));
    let (port, model_seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        if wire.contains("You are the Root Coordinator") {
            let body: JsonValue =
                serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            if input["basis"]["phase"] == "authorization-output" {
                if let Some(pause) = &pause {
                    if let Some((path, scan, root)) = pause.lock().unwrap().take() {
                        let database = db::open(&path).unwrap();
                        assert_eq!(
                            database
                                .query_row(
                                    "SELECT scan_id FROM agent_runs WHERE id=?1",
                                    [&root],
                                    |r| r.get::<_, String>(0)
                                )
                                .unwrap(),
                            scan
                        );
                        assert_eq!(
                            database
                                .execute(
                                    "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
                                    [&scan]
                                )
                                .unwrap(),
                            1
                        );
                    }
                }

                if let Some(tool) = local_tool {
                    return (
                        200,
                        "application/json",
                        model_round(&[(tool, json!({}))], 10),
                    );
                }
            }
            if fail_auth && input["basis"]["phase"] == "authorization-output" {
                return (
                    503,
                    "application/json",
                    json!({"error":{"message":"actual Auth assessment provider unavailable"}})
                        .to_string(),
                );
            }
            let step = match input["basis"]["phase"].as_str().unwrap_or("") {
                "identity-session-output" => Some("assess:identity_session_metadata"),
                "mapper-output" => Some("dispatch:web_executor"),
                "budget-allocation" => Some("assess:budget_allocation"),
                "authorization-output" => Some("assess:authorization_output"),
                "capability-changed" => Some("assess:capability_changed"),
                "budget-consumption" => Some("assess:budget_consumption"),
                _ => None,
            };
            if let Some(step) = step {
                return (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":["original physical source"],"missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string()));
            }
        }
        let response = if let Some(root) = fresh_multi_root_wire_response(&wire) {
            root
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"original paid Mapper","priorityContracts":[],"risks":[]}"#,
            )
        } else if wire.contains("IdentitySession") {
            proposal_model_response(&json!({"summary":"actual paid identity metadata only","observedAuthentication":["two validated scoped handles"],"evidenceGaps":["no target identity comparison"],"authorizationProven":false}).to_string())
        } else {
            model_round(&[], 10)
        };
        (200, "application/json", response)
    }));
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("oviraptor-auth-output-{}", Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let mut connection = db::open(&db_path).unwrap();
    connection
        .execute(
            "INSERT INTO projects(id,name) VALUES(9001,'Auth output fixture')",
            [],
        )
        .unwrap();
    let target = format!("http://127.0.0.1:{site_port}");
    let scope = Uuid::new_v4().to_string();
    for (id, cookie, authorization) in [
        ("session-a", "cookie-a", "Bearer a"),
        ("session-b", "cookie-b", "Bearer b"),
    ] {
        let document = json!({"schemaVersion":1,"id":id,"projectId":9001,"entryUrl":target,"scopeHosts":["127.0.0.1"],"cookies":[{"name":"session","value":cookie}],"headers":{"authorization":authorization},"identityIsolation":{"sharedWithOtherSessions":false}});
        connection.execute("INSERT INTO browser_auth_sessions(id,project_id,draft_scope_id,name,entry_url,status,session_json,expires_at) VALUES(?1,9001,?2,?1,?3,'valid',?4,?5)",params![id,scope,target,document.to_string(),(chrono::Utc::now()+chrono::Duration::hours(8)).to_rfc3339()]).unwrap();
    }
    let scan = create_sentinel_url_scan_with_mode_in(
        &connection,
        9001,
        "Auth output fixture".into(),
        vec![target.clone()],
        Some("standard".into()),
        None,
        None,
        Some(vec!["session-a".into(), "session-b".into()]),
        Some(scope),
        None,
        None,
        None,
        Some("multi".into()),
    )
    .unwrap();
    let attempt = authorization_control_setup_in(&connection, &root, &scan.id, &target)
        .unwrap()
        .attempt_number;
    for i in 0..groups {
        let control = SaveAuthorizationControlInput {
            scan_id: scan.id.clone(),
            attempt_number: attempt,
            target_url: target.clone(),
            contract_key: format!("idor|/api/orders{i}"),
            owner_object_url: format!("{target}/api/orders{i}?id=owner{i}"),
            tester_control_url: format!("{target}/api/orders{i}?id=tester{i}"),
            object_query_key: "id".into(),
            owner_object_value: format!("owner{i}"),
            tester_object_value: format!("tester{i}"),
            response_object_pointer: "/order/id".into(),
            owner_identity: "session-a".into(),
            tester_identity: "session-b".into(),
        };
        save_authorization_control_in(&mut connection, &root, &control).unwrap();
    }
    let work =
        web_mode_test_start(&root, &mut connection, &scan.id, WebStartMode::Confirm).unwrap();
    let mut context = test_context(
        &db_path,
        &target,
        vec![
            AgentIdentity::scoped("session-a"),
            AgentIdentity::scoped("session-b"),
        ],
    );
    context.scan_id = scan.id;
    context.attempt_number = attempt;
    context.execution_plan = test_plan_for("standard", &target).with_attempt(attempt);
    context.execution_plan.hard_total_tokens = hard_tokens;
    context.execution_plan.soft_uncached_tokens = 60000.min(hard_tokens);
    context.execution_plan.hard_model_requests = hard_requests;
    context.execution_plan.soft_model_requests = 40.min(hard_requests);
    context.execution_plan.max_turns = 1;
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    context.target_dir = work.join("url-pipeline/target-00000");
    context.log_path = work.join("runner.log");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(
        context.target_dir.join(".oviraptor-scan-id"),
        &context.scan_id,
    )
    .unwrap();
    fs::write(
        context.target_dir.join("frontend-evidence.json"),
        context.evidence.to_string(),
    )
    .unwrap();
    let proof = private_web_mode_on(&connection, &context.scan_id, attempt).unwrap();
    let mode = crate::agent_runtime::web_mode::root::NewRootModeDeclaration::from_verified(
        &proof,
        &target,
        &context.execution_plan.hash(),
    )
    .unwrap()
    .with_local_deliberation()
    .unwrap()
    .with_authorization_v11_bootstrap_for_test()
    .unwrap()
    .with_new_evidence_supervision()
    .unwrap()
    .with_lease_expiry_supervision()
    .unwrap()
    .with_protection_supervision()
    .unwrap();
    let budget =
        fresh_web_root_budget_declaration(&proof, &target, &context.execution_plan).unwrap();
    crate::agent_runtime::store::record_attempt_plan_with_declarations(
        &connection,
        &context.scan_id,
        attempt,
        &target,
        context.execution_plan.backend,
        &context.execution_plan.hash(),
        &context.execution_plan.as_json(),
        Some(&budget),
        Some(&mode),
    )
    .unwrap();
    context.run = Some(runtime_open_run(&db_path, &context.scan_id, &context.route).unwrap());
    bind_agent_evidence_location(&context).unwrap();
    native_frozen_web_root_mode(&context).unwrap();
    let session = multi_agent_prepare(&mut context).unwrap();
    let mut f = BudgetTriggerFixture {
        h: AgentHarness {
            root,
            db_path,
            context,
            site_seen,
            model_seen,
        },
        session: Some(session),
        boundary: std::sync::Arc::new(std::sync::Mutex::new(None)),
        stop,
    };
    let outcome = run_native_agent(&f.h.context);
    assert!(
        matches!(
            outcome,
            AgentTargetOutcome::Incomplete(_) | AgentTargetOutcome::BoundedCompleted(_)
        ),
        "{}",
        outcome.detail()
    );
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    f
}

fn authorization_output_actual_assert(deny: Vec<bool>, expected: i64) {
    let _real = RealSpecialistTransport::enter();
    let n = deny.len() as i64;
    let mut f = authorization_output_actual_fixture(deny);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let cap_before = db
        .query_row(
            "SELECT count(*) FROM agent_web_capability_transition_receipts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0),
        )
        .unwrap();
    multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), (3 * n) as usize);
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='authorization'",[&root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment.clone(),
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::Authorization,
    };
    let payload:String=db.query_row("SELECT payload_json FROM agent_messages WHERE assignment_id=?1 AND kind='authorization_result'",[&assignment],|r|r.get(0)).unwrap();
    let payload: JsonValue = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["verified"], expected);
    assert_eq!(payload["requestCount"], 3 * n);
    let fees = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &root,
        Some(&assignment),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (fees.consumed, fees.reserved, fees.indeterminate),
        (3 * n, 0, 0)
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND status='pending'",[&root],|r|r.get::<_,i64>(0)).unwrap(),expected);
    assert_eq!(db.query_row("SELECT count(*) FROM sentinel_findings WHERE scan_id=?1 AND stage<>'native-agent-coverage'",[&f.h.context.scan_id],|r|r.get::<_,i64>(0)).unwrap(),0,"Auth candidates need independent review before finding publication");
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_web_capability_transition_receipts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        cap_before,
        "Auth output cannot become a Web capability seal"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='authorization-output'",[&root],|r|r.get::<_,i64>(0)).unwrap(),1);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let receipt = crate::commands::authorization_output_receipt::read_original_on(
        &tx,
        &f.session.as_ref().unwrap().lease,
        &child,
    )
    .unwrap()
    .expect("actual closed Auth requires signed source");
    assert!(!receipt.receipt_id().is_empty());
    assert!(!receipt.message_id().is_empty());
    assert_eq!(
        receipt.fact()["cause"],
        "normal_completed_authorization_result"
    );
    assert_eq!(receipt.fact()["collection"]["controlCount"], n);
    assert_eq!(receipt.fact()["collection"]["requestCount"], 3 * n);
    assert_eq!(receipt.fact()["collection"]["verified"], expected);
    assert_eq!(receipt.fact()["executionAllowed"], false);
    let report =
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, &root)
            .unwrap()
            .unwrap();
    assert_eq!(
        serde_json::to_value(report).unwrap()["pendingAuthorizationOutputs"],
        0
    );

    assert!(
        native_coordinator_authorization_output_publication_replay_on(
            &tx,
            &f.session.as_ref().unwrap().lease,
            &child
        )
        .unwrap()
        .is_some()
    );
    tx.rollback().unwrap();
    native_coordinator_authorization_output_feedback(
        &f.h.context,
        &f.session.as_ref().unwrap().lease,
        &child,
    )
    .unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), (3 * n) as usize);
    // The complete production entry must discover and replay the original
    // completed Auth source before scheduling another target-touching worker.
    multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), (3 * n) as usize);
}

#[test]
fn authorization_output_actual_one_verified_control_paid_once() {
    authorization_output_actual_assert(vec![false], 1);
}
#[test]
fn authorization_output_actual_zero_verified_control_still_paid_once() {
    authorization_output_actual_assert(vec![true], 0);
}
#[test]
fn authorization_output_actual_mixed_two_controls_one_paid_assessment() {
    authorization_output_actual_assert(vec![false, true], 1);
}

#[test]
fn authorization_output_actual_signed_source_rejects_foreign_scope_worker_and_cold_signature() {
    use crate::commands::authorization_output_receipt;
    let _real = RealSpecialistTransport::enter();
    let mut f = authorization_output_actual_fixture(vec![false]);
    let db = db::open(&f.h.db_path).unwrap();
    multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).unwrap();
    let session = f.session.as_ref().unwrap();
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='authorization'",[&session.lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::Authorization,
    };
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let sites = f.h.site_seen.lock().unwrap().len();
    assert_eq!(sites, 3);
    let mut foreign = session.lease.clone();
    foreign.root_run_id = Uuid::new_v4().to_string();
    let tx = db.unchecked_transaction().unwrap();
    assert!(authorization_output_receipt::read_original_on(&tx, &foreign, &child).is_err());
    let mut foreign_worker = child.clone();
    foreign_worker.run_id = Uuid::new_v4().to_string();
    assert!(
        authorization_output_receipt::read_original_on(&tx, &session.lease, &foreign_worker)
            .is_err()
    );
    tx.rollback().unwrap();
    assert!(
        native_coordinator_authorization_output_feedback(&f.h.context, &foreign, &child).is_err()
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let tx = db.unchecked_transaction().unwrap();
    let trigger:String=tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='authorization_output_no_update'",[],|r|r.get(0)).unwrap();
    tx.execute_batch("DROP TRIGGER authorization_output_no_update")
        .unwrap();
    assert_eq!(tx.execute("UPDATE agent_authorization_output_receipts SET signature=zeroblob(32) WHERE root_run_id=?1",[&session.lease.root_run_id]).unwrap(),1);
    tx.execute_batch(&trigger).unwrap();
    let damaged = super::tests::application_table_snapshot(&tx);
    assert_eq!(
        authorization_output_receipt::read_original_on(&tx, &session.lease, &child)
            .err()
            .expect("cold original tag must reject"),
        "authorization_output_signature_invalid"
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), sites);
}

#[test]
fn authorization_output_actual_root_503_preserves_paid_source_and_blocks_fresh_replay() {
    let _real = RealSpecialistTransport::enter();
    let mut f = authorization_output_actual_fixture_phase(vec![false], true);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    assert!(multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).is_err());
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 3);
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='authorization'",[&root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment.clone(),
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::Authorization,
    };
    let fees = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &root,
        Some(&assignment),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (fees.consumed, fees.reserved, fees.indeterminate),
        (3, 0, 0)
    );
    let root_fees =
        crate::agent_runtime::multi_agent::budget::balance(&db, &root, Some(""), "model_requests")
            .unwrap();
    assert!(
        root_fees.indeterminate > 0,
        "the actual unavailable provider bill must not be refunded or reported as zero"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='authorization_result' AND delivered_at<>'' AND acknowledged_at<>'' AND delivery_attempts=1",[&assignment],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        db.query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&assignment],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "completed"
    );
    let tx = db.unchecked_transaction().unwrap();
    let receipt = crate::commands::authorization_output_receipt::read_original_on(
        &tx,
        &f.session.as_ref().unwrap().lease,
        &child,
    )
    .unwrap()
    .expect("the failed independent Root cannot erase original completed Auth source");
    assert_eq!(receipt.fact()["collection"]["requestCount"], 3);
    assert_eq!(receipt.fact()["collection"]["verified"], 1);
    assert!(
        native_coordinator_authorization_output_publication_replay_on(
            &tx,
            &f.session.as_ref().unwrap().lease,
            &child
        )
        .unwrap()
        .is_none()
    );
    let report =
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, &root)
            .unwrap()
            .unwrap();
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["pendingAuthorizationOutputs"], 1);
    assert_eq!(value["freshWorkBlocked"], "cost_reconciliation_required");
    tx.rollback().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 3);
}

#[test]
fn authorization_output_actual_local_tool_response_is_paid_but_never_executed() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    for tool in ["snapshot.read", "plan.propose"] {
        let mut f = authorization_output_actual_fixture_response(vec![false], false, Some(tool));
        let db = db::open(&f.h.db_path).unwrap();
        let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
        let before_fee = budget::balance(&db, &root, Some(""), "model_requests").unwrap();
        let error =
            multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).unwrap_err();
        assert!(
            error.contains("root_authorization_output_local_tool_denied"),
            "{tool}: {error}"
        );
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 3);
        let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='authorization'",[&root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id: assignment.clone(),
            run_id: run,
            role: crate::agent_runtime::contract::AgentRole::Authorization,
        };
        let after_fee = budget::balance(&db, &root, Some(""), "model_requests").unwrap();
        assert_eq!(
            after_fee.consumed,
            before_fee.consumed + 1,
            "the actual known received invoice must remain paid"
        );
        assert_eq!((after_fee.reserved, after_fee.indeterminate), (0, 0));
        let fees = budget::balance(&db, &root, Some(&assignment), "target_requests").unwrap();
        assert_eq!(
            (fees.consumed, fees.reserved, fees.indeterminate),
            (3, 0, 0)
        );
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts d ON d.call_id=q.call_id AND d.phase IN ('decision','publication') WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='authorization-output'",[&root],|r|r.get::<_,i64>(0)).unwrap(),0,"no saved local step or final publication can follow the denied response");
        let wires = f.h.model_seen.lock().unwrap();
        let auth_wires: Vec<JsonValue> = wires
            .iter()
            .filter_map(|wire| {
                let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n")?.1).ok()?;
                let input: JsonValue =
                    serde_json::from_str(body["messages"][1]["content"].as_str()?).ok()?;
                (input["basis"]["phase"] == "authorization-output").then_some(body)
            })
            .collect();
        assert_eq!(
            auth_wires.len(),
            1,
            "no second model request after a forbidden local tool"
        );
        assert!(auth_wires[0]["tools"]
            .as_array()
            .is_none_or(|tools| tools.iter().all(|item| !matches!(
                item["function"]["name"].as_str(),
                Some("snapshot.read" | "plan.propose")
            ))));
        drop(wires);
        let before = super::tests::application_table_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        assert!(
            crate::commands::authorization_output_receipt::read_original_on(
                &tx,
                &f.session.as_ref().unwrap().lease,
                &child
            )
            .unwrap()
            .is_some()
        );
        let report = budget::supervision_capacity::diagnostic(&tx, &root)
            .unwrap()
            .unwrap();
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["pendingAuthorizationOutputs"], 1);
        assert!(
            value["freshWorkBlocked"].is_null(),
            "known paid semantic refusal is not unknown provider debt"
        );
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 3);
    }
}

#[test]
fn authorization_output_actual_scope_paused_during_paid_sdk_retains_bill_without_publication() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let pause: AuthorizationOutputPause = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mut f = authorization_output_actual_fixture_interruption(
        vec![false],
        false,
        None,
        Some(pause.clone()),
    );
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    *pause.lock().unwrap() = Some((
        f.h.db_path.clone(),
        f.h.context.scan_id.clone(),
        root.clone(),
    ));
    let before_fee = budget::balance(&db, &root, Some(""), "model_requests").unwrap();
    assert!(multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).is_err());
    assert!(
        pause.lock().unwrap().is_none(),
        "the pause must occur inside the actual Auth Root request"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&f.h.context.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 3);
    let after_fee = budget::balance(&db, &root, Some(""), "model_requests").unwrap();
    assert_eq!(
        after_fee.consumed,
        before_fee.consumed + 1,
        "scope loss cannot refund the real known Root invoice"
    );
    assert_eq!((after_fee.reserved, after_fee.indeterminate), (0, 0));
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='authorization'",[&root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment.clone(),
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::Authorization,
    };
    let (call,invoice):(String,String)=db.query_row("SELECT q.call_id,j.receipt_json FROM agent_root_tick_receipts q JOIN agent_root_model_journal j ON j.call_id=q.call_id AND j.phase='received' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='authorization-output'",[&root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let invoice: JsonValue = serde_json::from_str(&invoice).unwrap();
    assert_eq!(invoice["usageReported"], true);
    assert!(invoice["usage"]["totalTokens"].as_i64().unwrap() > 0);
    assert!(!invoice["responseHash"].as_str().unwrap().is_empty());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE call_id=?1 AND phase='publication'",[&call],|r|r.get::<_,i64>(0)).unwrap(),0);
    let fees = budget::balance(&db, &root, Some(&assignment), "target_requests").unwrap();
    assert_eq!(
        (fees.consumed, fees.reserved, fees.indeterminate),
        (3, 0, 0)
    );
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::commands::authorization_output_receipt::read_original_on(
            &tx,
            &f.session.as_ref().unwrap().lease,
            &child
        )
        .unwrap()
        .is_some(),
        "original signed Auth history remains readable after scan pause"
    );
    tx.rollback().unwrap();
    assert!(multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 3);
}

#[test]
fn authorization_output_actual_historical_reader_rejects_cold_mailbox_and_outbox_physical_changes()
{
    let _real = RealSpecialistTransport::enter();
    let mut f = authorization_output_actual_fixture(vec![false]);
    let db = db::open(&f.h.db_path).unwrap();
    multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).unwrap();
    let session = f.session.as_ref().unwrap();
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='authorization'",[&session.lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::Authorization,
    };
    let snapshot = db.unchecked_transaction().unwrap();
    let source = crate::commands::authorization_output_receipt::read_original_on(
        &snapshot,
        &session.lease,
        &child,
    )
    .unwrap()
    .unwrap();
    let message = source.message_id().to_string();
    snapshot.rollback().unwrap();
    let original_signed = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT * FROM agent_authorization_output_receipts WHERE message_id=?1",
        [&message],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let sites = f.h.site_seen.lock().unwrap().len();
    for damage in ["ack", "from", "to", "payload", "outbox_ack"] {
        let tx = db.unchecked_transaction().unwrap();
        // Suppress only the ordinary ACK event writer for a raw ACK cold edit;
        // restore its exact SQL before verification so this is not schema damage.
        if damage == "ack" {
            let trigger:String=tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='agent_collaboration_message_update'",[],|r|r.get(0)).unwrap();
            tx.execute_batch("DROP TRIGGER agent_collaboration_message_update")
                .unwrap();
            assert_eq!(tx.execute("UPDATE agent_messages SET acknowledged_at=acknowledged_at||'-cold' WHERE id=?1",[&message]).unwrap(),1);
            tx.execute_batch(&trigger).unwrap();
        } else {
            let sql=match damage {
                "from"=>"UPDATE agent_messages SET from_run_id='foreign-original-source' WHERE id=?1",
                "to"=>"UPDATE agent_messages SET to_run_id='foreign-original-destination' WHERE id=?1",
                "payload"=>"UPDATE agent_messages SET payload_json=json_set(payload_json,'$.verified',0) WHERE id=?1",
                "outbox_ack"=>"UPDATE agent_collaboration_events SET payload_json=json_set(payload_json,'$.acknowledgedAt','cold-ack') WHERE entity_id=?1 AND sequence=(SELECT max(sequence) FROM agent_collaboration_events WHERE entity_id=?1)",
                _=>unreachable!(),
            };
            assert_eq!(tx.execute(sql, [&message]).unwrap(), 1);
        }
        assert!(
            crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
                &tx,
                "SELECT * FROM agent_authorization_output_receipts WHERE message_id=?1",
                [&message]
            )
            .unwrap()
                == original_signed,
            "the original signed body/tag must remain untouched for {damage}"
        );
        let damaged = super::tests::application_table_snapshot(&tx);
        let error = crate::commands::authorization_output_receipt::read_original_on(
            &tx,
            &session.lease,
            &child,
        )
        .err()
        .expect("physical source damage must reject despite intact signature");
        assert_eq!(
            error,
            if matches!(damage, "ack" | "outbox_ack") {
                "authorization_output_original_outbox_changed"
            } else {
                "authorization_output_original_delivery_changed"
            },
            "{damage}"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&tx),
            damaged,
            "reader must remain pure after {damage}"
        );
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), sites);
    }
}

fn authorization_output_pre_auth_original_capacity(f: &BudgetTriggerFixture) -> (i64, i64) {
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let (tokens, requests) =
        crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(&tx, root)
            .unwrap();
    let spent =
        crate::agent_runtime::multi_agent::budget::balance(&tx, root, None, "model_requests")
            .unwrap();
    let mut statement=tx.prepare("SELECT j.round,json_extract(q.fact_json,'$.request.basis.phase'),json_extract(j.receipt_json,'$.reservedTokens') FROM agent_root_model_journal j JOIN agent_root_tick_receipts q ON q.call_id=j.call_id AND q.phase='request' WHERE j.root_run_id=?1 AND j.phase='dispatch' ORDER BY j.round").unwrap();
    let estimates:Vec<JsonValue>=statement.query_map([root],|r|Ok(json!({"round":r.get::<_,i64>(0)?,"phase":r.get::<_,String>(1)?,"reservedTokens":r.get::<_,i64>(2)?}))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
    drop(statement);
    let sdk = f.h.model_seen.lock().unwrap().len() as i64;
    assert_eq!(
        spent.consumed, sdk,
        "each observed actual provider request must have its original model charge"
    );
    assert_eq!((spent.reserved, spent.indeterminate), (0, 0));
    println!(
        "authorization_output_original_capacity {}",
        json!({"actualPaidSdkCount":sdk,"remainingModelRequests":requests,"remainingModelTokens":tokens,"rootOriginalDispatchEstimates":estimates,"futureTokenCapacityGuaranteed":false})
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    (requests.unwrap(), spent.consumed)
}

#[test]
fn authorization_output_actual_low_original_request_budget_retains_pre_get_floor() {
    let _real = RealSpecialistTransport::enter();
    let mut f = authorization_output_actual_fixture_prebirth(vec![false], false, None, None, 16);
    let (remaining, paid) = authorization_output_pre_auth_original_capacity(&f);
    assert_eq!(remaining + paid, 16);
    if remaining < 7 {
        let db = db::open(&f.h.db_path).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let sdk = f.h.model_seen.lock().unwrap().len();
        assert!(multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).is_err());
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
        assert!(
            f.h.site_seen.lock().unwrap().is_empty(),
            "insufficient original follow-up headroom must reject before first Auth GET"
        );
        drop(db);
        drop(f);
        // Derived from actual paid startup/finalization receipts, with precisely
        // the original six-follow-up plus one new Auth assessment floor.
        f = authorization_output_actual_fixture_prebirth(vec![false], false, None, None, paid + 7);
        let (minimum_remaining, minimum_paid) = authorization_output_pre_auth_original_capacity(&f);
        assert_eq!(minimum_paid, paid);
        assert_eq!(minimum_remaining, 7);
    }
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 3);
    let assignment: String = db
        .query_row(
            "SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='authorization'",
            [&root],
            |r| r.get(0),
        )
        .unwrap();
    let fees = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &root,
        Some(&assignment),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (fees.consumed, fees.reserved, fees.indeterminate),
        (3, 0, 0)
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='authorization-output'",[&root],|r|r.get::<_,i64>(0)).unwrap(),1);
    let tx = db.unchecked_transaction().unwrap();
    let report =
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, &root)
            .unwrap()
            .unwrap();
    assert_eq!(
        serde_json::to_value(report).unwrap()["pendingAuthorizationOutputs"],
        0
    );
    let (round,raw_request,raw_dispatch):(i64,String,String)=tx.query_row("SELECT q.round,q.fact_json,j.receipt_json FROM agent_root_tick_receipts q JOIN agent_root_model_journal j ON j.call_id=q.call_id AND j.phase='dispatch' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='authorization-output'",[&root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let saved_request: JsonValue = serde_json::from_str(&raw_request).unwrap();
    let dispatch: JsonValue = serde_json::from_str(&raw_dispatch).unwrap();
    let captured = &saved_request["request"]["basis"]["budgetSnapshot"];
    let dimensions = &captured["dimensions"];
    let original_paid_tokens = dimensions["model_input_tokens"]["consumed"]
        .as_i64()
        .unwrap()
        + dimensions["model_output_tokens"]["consumed"]
            .as_i64()
            .unwrap();
    println!(
        "authorization_output_original_auth_reserve {}",
        json!({"round":round,"reservedTokens":dispatch["reservedTokens"],"paidModelTokensBeforeAuth":original_paid_tokens,"remainingModelTokensBeforeAuth":captured["remainingModelTokens"],"remainingModelRequestsBeforeAuth":captured["remainingModelRequests"],"inputTokenStateBeforeAuth":dimensions["model_input_tokens"],"outputTokenStateBeforeAuth":dimensions["model_output_tokens"],"cachedTokenStateBeforeAuth":dimensions["model_cached_tokens"],"timing":captured["timing"],"financialOwnerBound":captured["financialOwner"].as_str().is_some_and(|id|!id.is_empty()),"executionGrant":false,"futureTokenCapacityGuaranteed":false})
    );
    tx.rollback().unwrap();
}

#[test]
fn authorization_output_actual_original_token_boundary_uses_only_authenticated_single_call_reserve()
{
    let _real = RealSpecialistTransport::enter();
    let mut f =
        authorization_output_actual_fixture_limits(vec![false], false, None, None, 80, 34000);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let before_fee =
        crate::agent_runtime::multi_agent::budget::balance(&db, &root, Some(""), "model_requests")
            .unwrap();
    multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 3);
    let after_fee =
        crate::agent_runtime::multi_agent::budget::balance(&db, &root, Some(""), "model_requests")
            .unwrap();
    assert_eq!(after_fee.consumed, before_fee.consumed + 1);
    assert_eq!((after_fee.reserved, after_fee.indeterminate), (0, 0));
    let tx = db.unchecked_transaction().unwrap();
    let (raw_request,raw_dispatch):(String,String)=tx.query_row("SELECT q.fact_json,j.receipt_json FROM agent_root_tick_receipts q JOIN agent_root_model_journal j ON j.call_id=q.call_id AND j.phase='dispatch' JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='authorization-output'",[&root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let request: JsonValue = serde_json::from_str(&raw_request).unwrap();
    let dispatch: JsonValue = serde_json::from_str(&raw_dispatch).unwrap();
    let remaining = request["request"]["basis"]["budgetSnapshot"]["remainingModelTokens"]
        .as_i64()
        .unwrap();
    let estimate = dispatch["reservedTokens"].as_i64().unwrap();
    let after_reserve = remaining.checked_sub(estimate).unwrap();
    assert!((0..15000).contains(&after_reserve),"actual original Auth reserve must fit without the generic local 15000-token remainder: remaining={remaining}, estimate={estimate}");
    let report =
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, &root)
            .unwrap()
            .unwrap();
    assert_eq!(
        serde_json::to_value(report).unwrap()["pendingAuthorizationOutputs"],
        0
    );
    tx.rollback().unwrap();
    let assignment: String = db
        .query_row(
            "SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='authorization'",
            [&root],
            |r| r.get(0),
        )
        .unwrap();
    let fees = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &root,
        Some(&assignment),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (fees.consumed, fees.reserved, fees.indeterminate),
        (3, 0, 0)
    );
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    multi_agent_authorization(&f.h.context, f.session.as_mut().unwrap()).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 3);

    // Primitive admission regression only: no non-Auth SDK is dispatched.
    // A generic Root claim still retains the original 15000-token local floor.
    let tx = db.unchecked_transaction().unwrap();
    let available =
        crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(&tx, &root)
            .unwrap()
            .0
            .unwrap();
    let generic_estimate = available.checked_sub(14999).unwrap();
    assert!(generic_estimate > 0);
    let round: i64 = tx
        .query_row(
            "SELECT max(round)+1 FROM agent_root_model_journal WHERE root_run_id=?1",
            [&root],
            |r| r.get(0),
        )
        .unwrap();
    let hash = crate::agent_runtime::store::stable_hash("generic local floor primitive regression");
    // The primitive applies a write authorizer, not a savepoint. Its caller
    // owns atomicity after a late admission refusal.
    tx.execute_batch("SAVEPOINT generic_floor_admission")
        .unwrap();
    let error = crate::agent_runtime::multi_agent::budget::root::model::RootModelCall::claim(
        &tx,
        &root,
        round,
        &hash,
        generic_estimate,
    )
    .err()
    .expect("generic admission must keep its original local floor");
    assert_eq!(error, "child_budget_reservation_exceeded_or_stale");
    tx.execute_batch("ROLLBACK TO generic_floor_admission; RELEASE generic_floor_admission")
        .unwrap();
    assert_eq!(
        super::tests::application_table_snapshot(&tx),
        before,
        "the caller must roll back every temporary primitive reservation on refusal"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
}
