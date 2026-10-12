// Actual frozen phase-one BusinessLogic SDK and normal signed output, no target I/O.
fn business_logic_output_actual_fixture() -> BudgetTriggerFixture {
    business_logic_output_actual_fixture_failure(false)
}

fn business_logic_output_actual_fixture_failure(fail_output: bool) -> BudgetTriggerFixture {
    let mut h = fresh_multi_production_harness_with_site("business-output-v13", |wire| {
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
            if fail_output && phase == "business-logic-output" {
                return (
                    503,
                    "application/json",
                    json!({"error":{"message":"actual Business Root output unavailable"}})
                        .to_string(),
                );
            }
            let step = match phase {
                "bootstrap" => "dispatch:spa_api_mapper",
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
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original signed production fact"],"missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string())
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
    .with_business_logic_v13_bootstrap_for_test()
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
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    f
}

#[test]
fn business_logic_output_actual_v13_sdk_normal_seal_and_pure_history() {
    let _real = RealSpecialistTransport::enter();
    let f = business_logic_output_actual_fixture();
    let actor = &f.session.as_ref().unwrap().lease;
    let sdk = f.h.model_seen.lock().unwrap().len();
    let output = multi_agent_business_logic_readonly_output_only(&f.h.context, actor).unwrap();
    assert_eq!(output["state"], "completed");
    assert_eq!(output["targetRequests"], 0);
    assert_eq!(output["confirmedFinding"], false);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    assert_eq!(
        f.h.model_seen.lock().unwrap().len(),
        sdk + 1,
        "one actual Business specialist, no Root output claim"
    );
    let db = db::open(&f.h.db_path).unwrap();
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='business_logic' AND state='completed' AND budget_settled_at<>''",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::BusinessLogic,
    };
    for (dimension, expected) in [("model_requests", 1), ("target_requests", 0)] {
        let b = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )
        .unwrap();
        assert_eq!((b.consumed, b.reserved, b.indeterminate), (expected, 0, 0));
    }
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let source =
        crate::commands::business_logic_output_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .unwrap();
    assert_eq!(source.fact()["collection"]["payload"], output);
    assert_eq!(source.fact()["executionAllowed"], false);
    let id = source.receipt_id().to_string();
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_messages WHERE id=?1 AND kind='evidence_summary' AND correlation_id=?2 AND acknowledged_at<>''",params![source.message_id(),format!("business-logic:{}",child.assignment_id)],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        crate::commands::business_logic_output_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .unwrap()
            .receipt_id(),
        id
    );
    let mut foreign = actor.clone();
    foreign.root_run_id = Uuid::new_v4().to_string();
    assert!(
        crate::commands::business_logic_output_receipt::read_original_on(&tx, &foreign, &child)
            .is_err()
    );
    let mut other = child.clone();
    other.run_id = Uuid::new_v4().to_string();
    assert!(
        crate::commands::business_logic_output_receipt::read_original_on(&tx, actor, &other)
            .is_err()
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    for damage in ["signature", "missing", "paused"] {
        let tx = db.unchecked_transaction().unwrap();
        let trigger = if damage == "missing" {
            "business_logic_output_no_delete"
        } else {
            "business_logic_output_no_update"
        };
        let ddl: String = tx
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name=?1 AND type='trigger'",
                [trigger],
                |r| r.get(0),
            )
            .unwrap();
        if damage != "paused" {
            tx.execute_batch(&format!("DROP TRIGGER {trigger}"))
                .unwrap();
        }
        if damage == "signature" {
            tx.execute("UPDATE agent_business_logic_output_receipts SET signature=zeroblob(32) WHERE receipt_id=?1",[&id]).unwrap();
        } else if damage == "missing" {
            tx.execute(
                "DELETE FROM agent_business_logic_output_receipts WHERE receipt_id=?1",
                [&id],
            )
            .unwrap();
        } else {
            tx.execute(
                "UPDATE agent_assignments SET state='paused' WHERE id=?1",
                [&child.assignment_id],
            )
            .unwrap();
        }
        if damage != "paused" {
            tx.execute_batch(&ddl).unwrap();
        }
        let damaged = super::tests::application_table_snapshot(&tx);
        let read =
            crate::commands::business_logic_output_receipt::read_original_on(&tx, actor, &child);
        if damage == "missing" {
            assert!(read.unwrap().is_none());
            let mode = original_root_capture_mode(&tx, &actor.root_run_id).unwrap();
            assert_eq!(
                crate::commands::business_logic_output_receipt::capture_before_on(
                    &tx,
                    actor,
                    &child,
                    &f.h.context.target_dir,
                    &mode
                )
                .err()
                .unwrap(),
                "assignment_attempt_expired_or_not_executable"
            );
        } else {
            assert_eq!(
                read.err().unwrap(),
                if damage == "signature" {
                    "business_logic_output_signature_invalid"
                } else {
                    "business_logic_output_normal_completed_required"
                }
            );
        }
        assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 1);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn business_logic_output_actual_paid_root_assessment_and_free_publication_replay() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let f = business_logic_output_actual_fixture();
    let actor = &f.session.as_ref().unwrap().lease;
    let sdk = f.h.model_seen.lock().unwrap().len();
    let output = multi_agent_business_logic_readonly_output_only(&f.h.context, actor).unwrap();
    assert_eq!(output["targetRequests"], 0);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 1);
    let db = db::open(&f.h.db_path).unwrap();
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='business_logic' AND state='completed' AND budget_settled_at<>''",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::BusinessLogic,
    };
    let before_fee = budget::balance(&db, &actor.root_run_id, Some(""), "model_requests").unwrap();
    native_coordinator_business_logic_output_feedback(&f.h.context, actor, &child).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    let fee = budget::balance(&db, &actor.root_run_id, Some(""), "model_requests").unwrap();
    assert_eq!(fee.consumed, before_fee.consumed + 1);
    assert_eq!((fee.reserved, fee.indeterminate), (0, 0));
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='business-logic-output'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        native_coordinator_business_logic_output_publication_replay_on(&tx, actor, &child)
            .unwrap()
            .is_some()
    );
    let report = serde_json::to_value(
        budget::supervision_capacity::diagnostic(&tx, &actor.root_run_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["pendingBusinessLogicOutputs"], 0);
    tx.rollback().unwrap();
    native_coordinator_business_logic_output_feedback(&f.h.context, actor, &child).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn business_logic_output_actual_full_entry_paid_then_history_reentry_is_free() {
    let _real = RealSpecialistTransport::enter();
    let f = business_logic_output_actual_fixture();
    let actor = &f.session.as_ref().unwrap().lease;
    let sdk = f.h.model_seen.lock().unwrap().len();
    let output = multi_agent_business_logic_readonly(&f.h.context, actor).unwrap();
    assert_eq!(output["state"], "completed");
    assert_eq!(output["targetRequests"], 0);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    let db = db::open(&f.h.db_path).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='business-logic-output'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        multi_agent_business_logic_readonly(&f.h.context, actor).unwrap(),
        crate::agent_runtime::secrets::redact_json(&output)
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn business_logic_output_actual_full_entry_root_503_keeps_source_and_unknown_fee_no_retry() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let f = business_logic_output_actual_fixture_failure(true);
    let actor = &f.session.as_ref().unwrap().lease;
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(multi_agent_business_logic_readonly(&f.h.context, actor).is_err());
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    let db = db::open(&f.h.db_path).unwrap();
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='business_logic' AND state='completed' AND budget_settled_at<>''",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::BusinessLogic,
    };
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
    let rootfee = budget::balance(&db, &actor.root_run_id, Some(""), "model_requests").unwrap();
    assert_eq!(rootfee.indeterminate, 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='business-logic-output'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::commands::business_logic_output_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .is_some()
    );
    let report = serde_json::to_value(
        budget::supervision_capacity::diagnostic(&tx, &actor.root_run_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["pendingBusinessLogicOutputs"], 1);
    assert_eq!(report["freshWorkBlocked"], "cost_reconciliation_required");
    tx.rollback().unwrap();
    assert_eq!(
        multi_agent_business_logic_readonly(&f.h.context, actor).unwrap_err(),
        "budget_indeterminate_requires_reconciliation"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}
