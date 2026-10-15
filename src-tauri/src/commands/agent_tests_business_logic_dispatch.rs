// Actual original v14 paid dispatch; General ReAct remains outside this contract.
fn business_logic_dispatch_actual_fixture(defer: bool, eligible: bool) -> BudgetTriggerFixture {
    business_logic_dispatch_actual_fixture_pipeline(defer, eligible, false)
}

fn business_logic_dispatch_actual_fixture_pipeline(
    defer: bool,
    eligible: bool,
    with_auto: bool,
) -> BudgetTriggerFixture {
    let mut h = fresh_multi_production_harness_with_site("business-dispatch-v14", |wire| {
        assert!(wire.lines().next().unwrap().starts_with("GET / HTTP/"));
        assert!(!wire.to_ascii_lowercase().contains("authorization:"));
        assert!(!wire.to_ascii_lowercase().contains("cookie:"));
        (
            200,
            "application/json",
            json!({"public":"original anonymous entry response"}).to_string(),
        )
    });
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let response = if wire.contains("You are the Root Coordinator") {
            let body: JsonValue =
                serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let phase = input["basis"]["phase"].as_str().unwrap();
            let step = match phase {
                "bootstrap" => "dispatch:spa_api_mapper",
                "business-logic-input" => {
                    if defer {
                        ""
                    } else {
                        "dispatch:business_logic"
                    }
                }
                "mapper-output" => "dispatch:web_executor",
                "budget-allocation" => "assess:budget_allocation",
                "capability-changed" => "assess:capability_changed",
                "budget-consumption" => "assess:budget_consumption",
                "http-budget-consumption" => "assess:http_budget_consumption",
                "external-surface-output" => "assess:external_surface_output",
                "business-logic-output" => "assess:business_logic_output",
                "new-evidence" => "assess:new_evidence",
                "protection-changed" => "assess:protection_changed",
                "lease-expired" => "assess:lease_expired",
                other => panic!("unexpected actual Root phase: {other}"),
            };
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original signed production fact"],"missing":[],"suggestions":if step.is_empty(){vec![]}else{vec![step]},"costNotes":[],"risks":[]}).to_string())
        } else if wire.contains("BusinessLogic 只读候选分析专家") {
            proposal_model_response(
                r#"{"schemaVersion":1,"summary":"original frozen business flow candidates","flowCandidates":[{"name":"checkout","evidenceRefs":["frontend-evidence"],"hypothesis":"entry may expose checkout flow"}],"evidenceGaps":["transactions not executed"],"confirmedFindings":false}"#,
            )
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"original paid Mapper","priorityContracts":[],"risks":[]}"#,
            )
        } else {
            model_round(&[], 10)
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = 20;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = 1;
    // Legitimate original evidence is frozen before any Root/plan birth.
    h.context.evidence["forms"] = if eligible {
        json!([{"method":"POST","action":"/checkout","inputs":[{"name":"quantity"}]}])
    } else {
        json!([])
    };
    fs::write(
        h.context.target_dir.join("frontend-evidence.json"),
        h.context.evidence.to_string(),
    )
    .unwrap();
    let db = db::open(&h.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE scan_id=?1",
            [&h.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let c = &mut h.context;
    let proof = private_web_mode_on(&db, &c.scan_id, c.attempt_number).unwrap();
    let mode = crate::agent_runtime::web_mode::root::NewRootModeDeclaration::from_verified(
        &proof,
        &c.target_url,
        &c.execution_plan.hash(),
    )
    .unwrap()
    .with_local_deliberation()
    .unwrap()
    .with_business_logic_v14_bootstrap_for_test()
    .unwrap()
    .with_new_evidence_supervision()
    .unwrap()
    .with_lease_expiry_supervision()
    .unwrap()
    .with_protection_supervision()
    .unwrap();
    let budget =
        fresh_web_root_budget_declaration(&proof, &c.target_url, &c.execution_plan).unwrap();
    crate::agent_runtime::store::record_attempt_plan_with_declarations(
        &db,
        &c.scan_id,
        c.attempt_number,
        &c.target_url,
        c.execution_plan.backend,
        &c.execution_plan.hash(),
        &c.execution_plan.as_json(),
        Some(&budget),
        Some(&mode),
    )
    .unwrap();
    c.run = Some(runtime_open_run(&h.db_path, &c.scan_id, &c.route).unwrap());
    bind_agent_evidence_location(c).unwrap();
    native_frozen_web_root_mode(c).unwrap();
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let mut f = BudgetTriggerFixture {
        h,
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
    if with_auto {
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    } else {
        with_business_logic_followup_disabled_for_test(|| {
            multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome)
        })
        .unwrap();
    }
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    f
}

#[test]
fn business_logic_dispatch_actual_v14_paid_input_mints_one_readonly_worker_and_closes_output() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let f = business_logic_dispatch_actual_fixture(false, true);
    let actor = &f.session.as_ref().unwrap().lease;
    let db = db::open(&f.h.db_path).unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let root = native_coordinator_root_context(&f.h.context, actor);
    let tx = db.unchecked_transaction().unwrap();
    let frame =
        NativeCoordinatorFrame::business_logic_input_on(&tx, actor, &root.target_dir).unwrap();
    tx.rollback().unwrap();
    let decision = native_coordinator_tick_for_frame(&root, actor, &frame).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 1);
    let child =
        native_coordinator_mint_business_logic_child(&f.h.context, actor, &decision).unwrap();
    let (tokens,requests,lane,caps):(i64,i64,String,String)=db.query_row("SELECT reserved_tokens,reserved_requests,lane,capability_lease_json FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        (tokens, requests, lane.as_str()),
        (2000, 1, "read_only_analysis")
    );
    let mut caps: Vec<String> = serde_json::from_str(&caps).unwrap();
    caps.sort();
    assert_eq!(
        caps,
        vec!["evidence.read".to_string(), "mailbox.write".to_string()]
    );
    multi_agent_business_logic_execute_prepared(&f.h.context, actor, &child).unwrap();
    native_coordinator_business_logic_output_feedback(&f.h.context, actor, &child).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 3);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    for (dimension, expected) in [("model_requests", 1), ("target_requests", 0)] {
        let b = budget::balance(
            &db,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )
        .unwrap();
        assert_eq!((b.consumed, b.reserved, b.indeterminate), (expected, 0, 0));
    }
    let before = super::tests::application_table_snapshot(&db);
    native_coordinator_business_logic_output_feedback(&f.h.context, actor, &child).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 3);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn business_logic_dispatch_actual_v14_ineligible_is_readonly_and_paid_defer_mints_nothing() {
    let _real = RealSpecialistTransport::enter();
    let f = business_logic_dispatch_actual_fixture(false, false);
    let actor = &f.session.as_ref().unwrap().lease;
    let db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        NativeCoordinatorFrame::business_logic_input_on(&tx, actor, &f.h.context.target_dir)
            .err()
            .unwrap(),
        "root_business_logic_dispatch_not_eligible"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    drop(f);
    let f = business_logic_dispatch_actual_fixture(true, true);
    let actor = &f.session.as_ref().unwrap().lease;
    let db = db::open(&f.h.db_path).unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let root = native_coordinator_root_context(&f.h.context, actor);
    let tx = db.unchecked_transaction().unwrap();
    let frame =
        NativeCoordinatorFrame::business_logic_input_on(&tx, actor, &root.target_dir).unwrap();
    tx.rollback().unwrap();
    let decision = native_coordinator_tick_for_frame(&root, actor, &frame).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 1);
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_coordinator_mint_business_logic_child(&f.h.context, actor, &decision).unwrap_err(),
        "root_did_not_select_business_logic"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='business_logic'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 1);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn business_logic_dispatch_actual_signed_binding_scope_tamper_and_missing_prevents_execution() {
    use crate::scanning::business_logic_dispatch_receipt;
    let _real = RealSpecialistTransport::enter();
    let f = business_logic_dispatch_actual_fixture(false, true);
    let actor = &f.session.as_ref().unwrap().lease;
    let db = db::open(&f.h.db_path).unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        multi_agent_business_logic_readonly(&f.h.context, actor).unwrap_err(),
        "business_logic_paid_dispatch_required"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    let root = native_coordinator_root_context(&f.h.context, actor);
    let tx = db.unchecked_transaction().unwrap();
    let frame =
        NativeCoordinatorFrame::business_logic_input_on(&tx, actor, &root.target_dir).unwrap();
    tx.rollback().unwrap();
    let mut decision = native_coordinator_tick_for_frame(&root, actor, &frame).unwrap();
    let paid = super::tests::application_table_snapshot(&db);
    let original_event = decision.event_sequence;
    decision.event_sequence += 1;
    assert_eq!(
        native_coordinator_mint_business_logic_child(&f.h.context, actor, &decision).unwrap_err(),
        "root_business_logic_dispatch_publication_changed"
    );
    decision.event_sequence = original_event;
    assert_eq!(super::tests::application_table_snapshot(&db), paid);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 1);
    let child =
        native_coordinator_mint_business_logic_child(&f.h.context, actor, &decision).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        !business_logic_dispatch_receipt::verify_original_on(&tx, actor, &child)
            .unwrap()
            .receipt_id()
            .is_empty()
    );
    let mut foreign = actor.clone();
    foreign.root_run_id = Uuid::new_v4().to_string();
    assert!(business_logic_dispatch_receipt::verify_original_on(&tx, &foreign, &child).is_err());
    let mut other = child.clone();
    other.run_id = Uuid::new_v4().to_string();
    assert!(business_logic_dispatch_receipt::verify_original_on(&tx, actor, &other).is_err());
    let ddl:String=tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='business_logic_dispatch_no_update'",[],|r|r.get(0)).unwrap();
    tx.execute_batch("DROP TRIGGER business_logic_dispatch_no_update")
        .unwrap();
    assert_eq!(tx.execute("UPDATE agent_business_logic_dispatch_receipts SET signature=zeroblob(32) WHERE assignment_id=?1",[&child.assignment_id]).unwrap(),1);
    tx.execute_batch(&ddl).unwrap();
    let damaged = super::tests::application_table_snapshot(&tx);
    assert_eq!(
        business_logic_dispatch_receipt::verify_original_on(&tx, actor, &child)
            .err()
            .unwrap(),
        "business_logic_dispatch_signature_invalid"
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let ddl:String=db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='business_logic_dispatch_no_delete'",[],|r|r.get(0)).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    tx.execute_batch("DROP TRIGGER business_logic_dispatch_no_delete")
        .unwrap();
    assert_eq!(
        tx.execute(
            "DELETE FROM agent_business_logic_dispatch_receipts WHERE assignment_id=?1",
            [&child.assignment_id]
        )
        .unwrap(),
        1
    );
    tx.execute_batch(&ddl).unwrap();
    tx.commit().unwrap();
    let damaged = super::tests::application_table_snapshot(&db);
    assert_eq!(
        multi_agent_business_logic_execute_prepared(&f.h.context, actor, &child).unwrap_err(),
        "business_logic_dispatch_original_binding_missing"
    );
    assert_eq!(
        native_coordinator_mint_business_logic_child(&f.h.context, actor, &decision).unwrap_err(),
        "business_logic_dispatch_original_binding_missing"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), damaged);
    assert_eq!(
        f.h.model_seen.lock().unwrap().len(),
        sdk + 1,
        "missing binding cannot spend the original child SDK"
    );
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn business_logic_dispatch_actual_original_input_changed_or_coordinator_withdrawn_cannot_mint() {
    let _real = RealSpecialistTransport::enter();
    for attack in ["input", "coordinator"] {
        let f = business_logic_dispatch_actual_fixture(false, true);
        let actor = &f.session.as_ref().unwrap().lease;
        let db = db::open(&f.h.db_path).unwrap();
        let root = native_coordinator_root_context(&f.h.context, actor);
        let tx = db.unchecked_transaction().unwrap();
        let frame =
            NativeCoordinatorFrame::business_logic_input_on(&tx, actor, &root.target_dir).unwrap();
        tx.rollback().unwrap();
        let decision = native_coordinator_tick_for_frame(&root, actor, &frame).unwrap();
        let sdk = f.h.model_seen.lock().unwrap().len();
        if attack == "input" {
            let path = f.h.context.target_dir.join("frontend-evidence.json");
            let mut bytes = fs::read(&path).unwrap();
            bytes.push(b' ');
            fs::write(path, bytes).unwrap();
        } else {
            assert_eq!(db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1 WHERE root_run_id=?2",params![Uuid::new_v4().to_string(),actor.root_run_id]).unwrap(),1);
        }
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            native_coordinator_mint_business_logic_child(&f.h.context, actor, &decision).is_err(),
            "{attack}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
        assert!(f.h.site_seen.lock().unwrap().is_empty());
        assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='business_logic'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
}

#[test]
fn business_logic_dispatch_actual_web_finish_pipeline_runs_then_paid_reentry_is_free() {
    let _real = RealSpecialistTransport::enter();
    let f = business_logic_dispatch_actual_fixture_pipeline(false, true, true);
    let session = f.session.as_ref().unwrap();
    let actor = &session.lease;
    let db = db::open(&f.h.db_path).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='business_logic' AND state='completed' AND budget_settled_at<>''",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    for phase in ["business-logic-input", "business-logic-output"] {
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')=?2",params![actor.root_run_id,phase],|r|r.get::<_,i64>(0)).unwrap(),1,"{phase}");
    }
    assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1 AND role='business_logic' AND state='received'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_business_logic_dispatch_receipts WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_business_logic_output_receipts WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let sdk = f.h.model_seen.lock().unwrap().len();
    let before = super::tests::application_table_snapshot(&db);
    multi_agent_business_logic_followup(&f.h.context, session).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn business_logic_dispatch_actual_web_finish_defer_and_ineligible_do_not_mint() {
    let _real = RealSpecialistTransport::enter();
    for eligible in [true, false] {
        let f = business_logic_dispatch_actual_fixture_pipeline(true, eligible, true);
        let actor = &f.session.as_ref().unwrap().lease;
        let db = db::open(&f.h.db_path).unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='business_logic'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='business-logic-input'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),i64::from(eligible));
        assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1 AND role='business_logic'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(f.h.site_seen.lock().unwrap().is_empty());
        if !eligible {
            let sdk = f.h.model_seen.lock().unwrap().len();
            let before = super::tests::application_table_snapshot(&db);
            multi_agent_business_logic_followup(&f.h.context, f.session.as_ref().unwrap()).unwrap();
            assert_eq!(super::tests::application_table_snapshot(&db), before);
            assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
        }
    }
}
