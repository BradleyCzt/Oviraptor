// Actual anonymous response and completed specialist output are tested separately.
// No test claims a paid Root output before that production boundary is connected.
fn external_surface_response_actual_fixture() -> (
    BudgetTriggerFixture,
    AgentRunContext,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) {
    external_surface_response_actual_fixture_stage(true)
}

fn external_surface_response_actual_fixture_stage(
    capture: bool,
) -> (
    BudgetTriggerFixture,
    AgentRunContext,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) {
    let f = external_surface_output_actual_prepared_fixture();
    let mut isolated = f.h.context.clone();
    let child = public_surface_child(&mut isolated, &f.session.as_ref().unwrap().lease);
    if !capture {
        return (f, isolated, child);
    }
    let sdk = f.h.model_seen.lock().unwrap().len();
    let response =
        capture_public_surface(&isolated, &f.session.as_ref().unwrap().lease, &child).unwrap();
    assert_eq!(response["capture"]["status"], 200);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(
        f.h.model_seen.lock().unwrap().len(),
        sdk,
        "response capture is not a Root or child SDK assessment"
    );
    (f, isolated, child)
}

fn external_surface_output_actual_prepared_fixture() -> BudgetTriggerFixture {
    external_surface_output_actual_prepared_fixture_with_failure(false)
}

fn external_surface_output_actual_prepared_fixture_with_failure(
    fail_output: bool,
) -> BudgetTriggerFixture {
    external_surface_output_actual_prepared_fixture_behavior(fail_output, None)
}

fn external_surface_output_actual_prepared_fixture_behavior(
    fail_output: bool,
    output_tool: Option<&'static str>,
) -> BudgetTriggerFixture {
    external_surface_output_actual_prepared_fixture_limits(fail_output, output_tool, 60000)
}

fn external_surface_output_actual_prepared_fixture_limits(
    fail_output: bool,
    output_tool: Option<&'static str>,
    tokens: i64,
) -> BudgetTriggerFixture {
    let mut h = fresh_multi_production_harness_with_site("external-response-v12", |wire| {
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
            if phase == "external-surface-output" {
                if let Some(tool) = output_tool {
                    return (
                        200,
                        "application/json",
                        model_round(&[(tool, json!({}))], 10),
                    );
                }
            }
            if fail_output && phase == "external-surface-output" {
                return (
                    503,
                    "application/json",
                    json!({"error":{"message":"actual surface output provider unavailable"}})
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
                "new-evidence" => "assess:new_evidence",
                "protection-changed" => "assess:protection_changed",
                "lease-expired" => "assess:lease_expired",
                other => panic!("unexpected actual Root phase: {other}"),
            };
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original signed production fact"],"missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string())
        } else if wire.contains("External Surface 公开面专家") {
            proposal_model_response(
                r#"{"summary":"original anonymous surface assessment","observations":["observed public JSON entry"],"coverageGaps":["other paths not requested"],"confirmedFindings":false}"#,
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
    h.context.execution_plan.hard_total_tokens = tokens;
    h.context.execution_plan.soft_uncached_tokens = 30000.min(tokens);
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
    .with_external_surface_v12_bootstrap_for_test()
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
fn external_surface_response_actual_v12_get_seals_known_fee_and_pure_history() {
    let _real = RealSpecialistTransport::enter();
    let (f, _, child) = external_surface_response_actual_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let actor = &f.session.as_ref().unwrap().lease;
    let fees = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(&child.assignment_id),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (fees.consumed, fees.reserved, fees.indeterminate),
        (1, 0, 0)
    );
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let source =
        crate::scanning::external_surface_response_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .unwrap();
    assert!(!source.receipt_id().is_empty());
    assert_eq!(source.fact()["executionAllowed"], false);
    assert_eq!(source.fact()["retainedBytesOnly"], true);
    assert_eq!(source.fact()["response"]["capture"]["status"], 200);
    assert_eq!(
        db.query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "running",
        "one received response does not complete the worker"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind IN ('evidence_summary','external_surface_result')",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let id = source.receipt_id().to_string();
    assert_eq!(
        crate::scanning::external_surface_response_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .unwrap()
            .receipt_id(),
        id
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn external_surface_response_actual_signed_source_rejects_foreign_scope_signature_and_claim() {
    let _real = RealSpecialistTransport::enter();
    let (f, _, child) = external_surface_response_actual_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let actor = &f.session.as_ref().unwrap().lease;
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let mut foreign = actor.clone();
    foreign.root_run_id = Uuid::new_v4().to_string();
    assert!(
        crate::scanning::external_surface_response_receipt::read_original_on(&tx, &foreign, &child)
            .is_err()
    );
    let mut foreign_child = child.clone();
    foreign_child.run_id = Uuid::new_v4().to_string();
    assert!(
        crate::scanning::external_surface_response_receipt::read_original_on(
            &tx,
            actor,
            &foreign_child
        )
        .is_err()
    );
    tx.rollback().unwrap();
    for damage in ["signature", "claim"] {
        let tx = db.unchecked_transaction().unwrap();
        if damage == "signature" {
            let trigger:String=tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='public_surface_response_no_update'",[],|r|r.get(0)).unwrap();
            tx.execute_batch("DROP TRIGGER public_surface_response_no_update")
                .unwrap();
            assert_eq!(tx.execute("UPDATE agent_public_surface_response_receipts SET signature=zeroblob(32) WHERE assignment_id=?1",[&child.assignment_id]).unwrap(),1);
            tx.execute_batch(&trigger).unwrap();
        } else {
            let protected = super::tests::application_table_snapshot(&tx);
            let error=tx.execute("UPDATE agent_external_surface_captures SET target_url=target_url||'/cold' WHERE assignment_id=?1",[&child.assignment_id]).unwrap_err();
            assert!(error
                .to_string()
                .contains("immutable_external_surface_capture"));
            assert_eq!(super::tests::application_table_snapshot(&tx), protected);
            let trigger:String=tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='immutable_external_surface_capture'",[],|r|r.get(0)).unwrap();
            tx.execute_batch("DROP TRIGGER immutable_external_surface_capture")
                .unwrap();
            assert_eq!(tx.execute("UPDATE agent_external_surface_captures SET target_url=target_url||'/cold' WHERE assignment_id=?1",[&child.assignment_id]).unwrap(),1);
            tx.execute_batch(&trigger).unwrap();
        }
        let damaged = super::tests::application_table_snapshot(&tx);
        let error = crate::scanning::external_surface_response_receipt::read_original_on(
            &tx, actor, &child,
        )
        .err()
        .expect("original source must reject cold physical changes");
        assert_eq!(
            error,
            if damage == "signature" {
                "public_surface_response_signature_invalid"
            } else {
                "public_surface_response_original_claim_missing"
            }
        );
        assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn external_surface_response_actual_retained_record_cold_change_is_not_adopted() {
    let _real = RealSpecialistTransport::enter();
    let (f, context, child) = external_surface_response_actual_fixture();
    let db = db::open(&f.h.db_path).unwrap();
    let actor = &f.session.as_ref().unwrap().lease;
    let tx = db.unchecked_transaction().unwrap();
    let source =
        crate::scanning::external_surface_response_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .unwrap();
    let artifact = source.fact()["artifactId"].as_str().unwrap().to_string();
    tx.rollback().unwrap();
    let path = context.target_dir.join(AGENT_HTTP_DIRECTORY).join(artifact);
    let original = fs::read(&path).unwrap();
    let mut changed = original.clone();
    changed.push(b' ');
    fs::write(&path, &changed).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        crate::scanning::external_surface_response_receipt::read_original_on(&tx, actor, &child)
            .err()
            .expect("record bytes may not be silently rehashed"),
        "public_surface_response_original_artifact_changed"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    fs::write(&path, &original).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::scanning::external_surface_response_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .is_some()
    );
    tx.rollback().unwrap();
}

#[test]
fn external_surface_response_actual_claim_trigger_side_writes_reject_before_get() {
    let _real = RealSpecialistTransport::enter();
    for attack in ["scan", "budget"] {
        let (f, context, child) = external_surface_response_actual_fixture_stage(false);
        let db = db::open(&f.h.db_path).unwrap();
        let actor = &f.session.as_ref().unwrap().lease;
        let body = if attack == "scan" {
            "UPDATE sentinel_scans SET task_name='unauthorized surface trigger';"
        } else {
            "INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at) SELECT entry_id||'-malicious',root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key||'-malicious',source_id||'-malicious',created_at FROM agent_budget_entries LIMIT 1;"
        };
        db.execute_batch(&format!("CREATE TRIGGER malicious_surface_claim AFTER INSERT ON agent_external_surface_captures BEGIN {body} END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let sdk = f.h.model_seen.lock().unwrap().len();
        assert!(
            capture_public_surface(&context, actor, &child).is_err(),
            "an AFTER trigger cannot inherit direct writer authority: {attack}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert!(f.h.site_seen.lock().unwrap().is_empty());
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
        assert_eq!(db.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name='malicious_surface_claim'",[],|r|r.get::<_,i64>(0)).unwrap(),1,"failure must not silently remove the malicious trigger");
    }
}

#[test]
fn external_surface_response_actual_issue_trigger_error_caught_then_commit_keeps_original_claim_only(
) {
    use crate::scanning::external_surface_response_receipt;
    let _real = RealSpecialistTransport::enter();
    let (f, context, child) = external_surface_response_actual_fixture_stage(false);
    let actor = &f.session.as_ref().unwrap().lease;
    let mut writer = Box::new(db::open(&f.h.db_path).unwrap());
    let tx = writer.transaction().unwrap();
    let witness =
        external_surface_response_receipt::claim_on(&tx, &context, actor, &child).unwrap();
    tx.commit().unwrap();
    let fresh =
        external_surface_response_receipt::send_transport(&writer, &context, witness).unwrap();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    let written =
        agent_write_http_record_fresh(&context, 1, &fresh.request(), fresh.view(), fresh.body())
            .unwrap();
    writer.execute_batch("CREATE TRIGGER malicious_surface_issue AFTER INSERT ON agent_public_surface_response_receipts BEGIN UPDATE sentinel_scans SET task_name='unauthorized surface receipt trigger'; END;").unwrap();
    let before = super::tests::application_table_snapshot(&writer);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = writer.transaction().unwrap();
    assert!(
        external_surface_response_receipt::issue_received_on(&tx, &context, fresh, &written)
            .is_err()
    );
    // The caller catches the error and commits: the proof factory's savepoint
    // must still undo receive, fee reconciliation, graph and signature writes.
    tx.commit().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&writer), before);
    assert_eq!(
        writer
            .query_row(
                "SELECT state FROM agent_external_surface_captures WHERE assignment_id=?1",
                [&child.assignment_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "claimed"
    );
    assert_eq!(writer.query_row("SELECT count(*) FROM agent_public_surface_response_receipts WHERE assignment_id=?1",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let fees = crate::agent_runtime::multi_agent::budget::balance(
        &writer,
        &actor.root_run_id,
        Some(&child.assignment_id),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (fees.consumed, fees.reserved, fees.indeterminate),
        (0, 0, 1),
        "the real dispatched GET remains unresolved, never refunded or falsely received"
    );
    assert_eq!(writer.query_row("SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name='malicious_surface_issue'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn external_surface_response_actual_same_name_revision_trigger_cold_ddl_is_rejected() {
    use crate::scanning::external_surface_response_receipt;
    let _real = RealSpecialistTransport::enter();
    for attack in ["extra_write", "changed_value"] {
        let (f, context, child) = external_surface_response_actual_fixture_stage(false);
        let actor = &f.session.as_ref().unwrap().lease;
        let mut writer = Box::new(db::open(&f.h.db_path).unwrap());
        let tx = writer.transaction().unwrap();
        let witness =
            external_surface_response_receipt::claim_on(&tx, &context, actor, &child).unwrap();
        tx.commit().unwrap();
        let fresh =
            external_surface_response_receipt::send_transport(&writer, &context, witness).unwrap();
        let written = agent_write_http_record_fresh(
            &context,
            1,
            &fresh.request(),
            fresh.view(),
            fresh.body(),
        )
        .unwrap();
        let original:String=writer.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='agent_evidence_nodes_revision_insert'",[],|r|r.get(0)).unwrap();
        let evil = if attack == "extra_write" {
            let end = original.rfind("END").unwrap();
            format!(
                "{} UPDATE sentinel_scans SET task_name='evil original-named revision trigger'; {}",
                &original[..end],
                &original[end..]
            )
        } else {
            assert_eq!(original.matches("'node:' || NEW.id").count(), 1);
            original.replacen("'node:' || NEW.id", "'evil-node:' || NEW.id", 1)
        };
        writer
            .execute_batch("DROP TRIGGER agent_evidence_nodes_revision_insert")
            .unwrap();
        writer.execute_batch(&evil).unwrap();
        let actual_evil:String=writer.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='agent_evidence_nodes_revision_insert'",[],|r|r.get(0)).unwrap();
        let before = super::tests::application_table_snapshot(&writer);
        let sdk = f.h.model_seen.lock().unwrap().len();
        let tx = writer.transaction().unwrap();
        let error =
            external_surface_response_receipt::issue_received_on(&tx, &context, fresh, &written)
                .err()
                .expect("an original trigger name cannot authenticate changed DDL");
        assert_eq!(
            error, "public_surface_revision_trigger_original_ddl_invalid",
            "{attack}"
        );
        tx.commit().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&writer), before);
        assert_eq!(writer.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='agent_evidence_nodes_revision_insert'",[],|r|r.get::<_,String>(0)).unwrap(),actual_evil,"the receipt guard must refuse rather than silently repair cold DDL");
        assert_eq!(
            writer
                .query_row(
                    "SELECT state FROM agent_external_surface_captures WHERE assignment_id=?1",
                    [&child.assignment_id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "claimed"
        );
        assert_eq!(writer.query_row("SELECT count(*) FROM agent_public_surface_response_receipts WHERE assignment_id=?1",[&child.assignment_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        let fees = crate::agent_runtime::multi_agent::budget::balance(
            &writer,
            &actor.root_run_id,
            Some(&child.assignment_id),
            "target_requests",
        )
        .unwrap();
        assert_eq!(
            (fees.consumed, fees.reserved, fees.indeterminate),
            (0, 0, 1)
        );
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn external_surface_output_actual_v12_specialist_completion_seals_ack_and_free_history() {
    use crate::scanning::external_surface_output_receipt;
    let _real = RealSpecialistTransport::enter();
    let f = external_surface_output_actual_prepared_fixture();
    let actor = &f.session.as_ref().unwrap().lease;
    let sdk = f.h.model_seen.lock().unwrap().len();
    let result = multi_agent_external_surface(&f.h.context, actor).unwrap();
    assert_eq!(result["state"], "completed");
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    let db = db::open(&f.h.db_path).unwrap();
    let (assignment, run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='external_surface'",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::ExternalSurface,
    };
    for dimension in ["target_requests", "model_requests"] {
        let fee = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )
        .unwrap();
        assert_eq!(
            (fee.consumed, fee.reserved, fee.indeterminate),
            (1, 0, 0),
            "{dimension}"
        );
    }
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let source = external_surface_output_receipt::read_original_on(&tx, actor, &child)
        .unwrap()
        .unwrap();
    assert_eq!(
        source.fact()["purpose"],
        "original_closed_external_surface_output"
    );
    assert_eq!(
        source.fact()["collection"]["payload"]["confirmedFinding"],
        false
    );
    assert_eq!(
        source.fact()["collection"]["payload"]["scope"],
        "anonymous_entry_only"
    );
    let id = source.receipt_id().to_string();
    let mid = source.message_id().to_string();
    let (raw_request,raw_dispatch):(String,String)=tx.query_row("SELECT q.fact_json,j.receipt_json FROM agent_root_tick_receipts q JOIN agent_root_model_journal j ON j.call_id=q.call_id AND j.phase='dispatch' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='external-surface-output'",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let request: JsonValue = serde_json::from_str(&raw_request).unwrap();
    let dispatch: JsonValue = serde_json::from_str(&raw_dispatch).unwrap();
    let remaining = request["request"]["basis"]["budgetSnapshot"]["remainingModelTokens"]
        .as_i64()
        .unwrap();
    let estimate = dispatch["reservedTokens"].as_i64().unwrap();
    assert!(estimate > 0 && remaining >= estimate);
    eprintln!(
        "external_surface_output_original_capacity_probe {}",
        json!({"originalTokenCap":60000,"remainingBefore":remaining,"reservedTokens":estimate,"afterReserve":remaining-estimate,"paidTokensBefore":60000-remaining})
    );

    let report = serde_json::to_value(
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(
            &tx,
            &actor.root_run_id,
        )
        .unwrap()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(report["pendingExternalSurfaceOutputs"], 0);
    assert!(report["freshWorkBlocked"].is_null());
    let rootfee = crate::agent_runtime::multi_agent::budget::balance(
        &tx,
        &actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(rootfee.indeterminate, 0);
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_root_model_journal j JOIN agent_root_tick_receipts q ON q.call_id=j.call_id AND q.phase='request' WHERE j.root_run_id=?1 AND j.phase='received' AND json_extract(q.fact_json,'$.request.basis.phase')='external-surface-output'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_messages WHERE id=?1 AND kind='evidence_summary' AND acknowledged_at<>'' AND correlation_id=?2",params![mid,format!("public-surface:{}",child.assignment_id)],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        external_surface_output_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .unwrap()
            .receipt_id(),
        id
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    for damage in ["signature", "ack"] {
        let tx = db.unchecked_transaction().unwrap();
        let table = if damage == "signature" {
            "agent_external_surface_output_receipts"
        } else {
            "agent_messages"
        };
        let triggers: Vec<(String, String)> = {
            let mut q = tx
                .prepare("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name=?1")
                .unwrap();
            let rows = q
                .query_map([table], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap();
            rows.collect::<rusqlite::Result<_>>().unwrap()
        };
        for (name, _) in &triggers {
            tx.execute_batch(&format!("DROP TRIGGER {name}")).unwrap();
        }
        if damage == "signature" {
            assert_eq!(tx.execute("UPDATE agent_external_surface_output_receipts SET signature=zeroblob(32) WHERE receipt_id=?1",[&id]).unwrap(),1);
        } else {
            assert_eq!(tx.execute("UPDATE agent_messages SET acknowledged_at=acknowledged_at||'-cold' WHERE id=?1",[&mid]).unwrap(),1);
        }
        for (_, sql) in &triggers {
            tx.execute_batch(sql).unwrap();
        }
        let damaged = super::tests::application_table_snapshot(&tx);
        let error = external_surface_output_receipt::read_original_on(&tx, actor, &child)
            .err()
            .expect("signed completed source must reject cold physical changes");
        assert_eq!(
            error,
            if damage == "signature" {
                "external_surface_output_signature_invalid"
            } else {
                "external_surface_output_original_outbox_changed"
            }
        );
        assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    }
    let replay = multi_agent_external_surface(&f.h.context, actor).unwrap();
    assert_eq!(replay, crate::agent_runtime::secrets::redact_json(&result));
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    // Actual specialist and independent Root output assessment each have their own paid source.
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='external-surface-output'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
}

#[test]
fn external_surface_output_actual_root_503_preserves_closed_source_and_pending_obligation() {
    let _real = RealSpecialistTransport::enter();
    let f = external_surface_output_actual_prepared_fixture_with_failure(true);
    let actor = &f.session.as_ref().unwrap().lease;
    let sdk = f.h.model_seen.lock().unwrap().len();
    let error = multi_agent_external_surface(&f.h.context, actor).unwrap_err();
    assert!(!error.is_empty());
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    let db = db::open(&f.h.db_path).unwrap();
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='external_surface' AND state='completed' AND budget_settled_at<>''",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::ExternalSurface,
    };
    for dimension in ["target_requests", "model_requests"] {
        let b = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )
        .unwrap();
        assert_eq!(
            (b.consumed, b.reserved, b.indeterminate),
            (1, 0, 0),
            "original paid worker is not refunded: {dimension}"
        );
    }
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='external-surface-output'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    let rootbill = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        rootbill.indeterminate, 1,
        "actual failed Root dispatch remains unresolved"
    );
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::scanning::external_surface_output_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .is_some()
    );
    let report = serde_json::to_value(
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(
            &tx,
            &actor.root_run_id,
        )
        .unwrap()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(report["pendingExternalSurfaceOutputs"], 1);
    assert_eq!(report["freshWorkBlocked"], "cost_reconciliation_required");
    tx.rollback().unwrap();
    let retry = multi_agent_external_surface(&f.h.context, actor).unwrap_err();
    assert_eq!(retry, "budget_indeterminate_requires_reconciliation");
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn external_surface_response_actual_fresh_claim_does_not_waive_other_unknown_costs() {
    use crate::scanning::external_surface_response_receipt;
    let _real = RealSpecialistTransport::enter();
    for dimension in ["model_requests", "target_requests"] {
        let (f, context, child) = external_surface_response_actual_fixture_stage(false);
        let actor = &f.session.as_ref().unwrap().lease;
        let mut writer = Box::new(db::open(&f.h.db_path).unwrap());
        let tx = writer.transaction().unwrap();
        let mut witness =
            external_surface_response_receipt::claim_on(&tx, &context, actor, &child).unwrap();
        tx.commit().unwrap();
        let tx = writer.transaction().unwrap();
        // Explicit cold ledger attack in this temporary DB, never a positive
        // fixture invoice: another source must not inherit the fresh GET waiver.
        for kind in ["reserve", "forfeit"] {
            assert_eq!(tx.execute("INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at) SELECT entry_id||'-foreign-unknown',root_run_id,assignment_id,lease_attempt_id,?2,kind,amount,idempotency_key||'-foreign-unknown','cold-foreign-unknown',created_at FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?3 AND dimension='target_requests' AND kind=?4 AND source_id='public:'||?3",params![actor.root_run_id,dimension,child.assignment_id,kind]).unwrap(),1);
        }
        tx.commit().unwrap();
        let before = super::tests::application_table_snapshot(&writer);
        let sdk = f.h.model_seen.lock().unwrap().len();
        let error =
            external_surface_response_receipt::before_send_on(&writer, &context, &mut witness)
                .unwrap_err();
        assert_eq!(
            error, "budget_indeterminate_requires_reconciliation",
            "{dimension}"
        );
        assert_eq!(super::tests::application_table_snapshot(&writer), before);
        assert!(f.h.site_seen.lock().unwrap().is_empty());
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
        let original = crate::agent_runtime::multi_agent::budget::balance(
            &writer,
            &actor.root_run_id,
            Some(&child.assignment_id),
            "target_requests",
        )
        .unwrap();
        assert_eq!(original.consumed, 0);
        assert_eq!(original.reserved, 0);
        assert_eq!(
            original.indeterminate,
            if dimension == "target_requests" { 2 } else { 1 },
            "original claim remains unrefunded"
        );
    }
}

#[test]
fn external_surface_output_actual_paid_tool_calls_are_denied_without_local_steps_or_retry() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    for tool in ["snapshot.read", "plan.propose"] {
        let f = external_surface_output_actual_prepared_fixture_behavior(false, Some(tool));
        let actor = &f.session.as_ref().unwrap().lease;
        let db = db::open(&f.h.db_path).unwrap();
        let beforefee =
            budget::balance(&db, &actor.root_run_id, Some(""), "model_requests").unwrap();
        let sdk = f.h.model_seen.lock().unwrap().len();
        let error = multi_agent_external_surface(&f.h.context, actor).unwrap_err();
        assert_eq!(
            error, "root_external_surface_output_local_tool_denied",
            "{tool}"
        );
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
        let wires = f.h.model_seen.lock().unwrap();
        let output_wires: Vec<JsonValue> = wires
            .iter()
            .filter_map(|wire| {
                let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n")?.1).ok()?;
                let input: JsonValue =
                    serde_json::from_str(body["messages"][1]["content"].as_str()?).ok()?;
                (input["basis"]["phase"] == "external-surface-output").then_some(body)
            })
            .collect();
        assert_eq!(output_wires.len(), 1);
        assert!(output_wires[0]["tools"]
            .as_array()
            .is_none_or(|tools| tools.iter().all(|item| !matches!(
                item["function"]["name"].as_str(),
                Some("snapshot.read" | "plan.propose")
            ))));
        drop(wires);
        let afterfee =
            budget::balance(&db, &actor.root_run_id, Some(""), "model_requests").unwrap();
        assert_eq!(afterfee.consumed, beforefee.consumed + 1);
        assert_eq!((afterfee.reserved, afterfee.indeterminate), (0, 0));
        let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='external_surface' AND state='completed' AND budget_settled_at<>''",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id: assignment,
            run_id: run,
            role: crate::agent_runtime::contract::AgentRole::ExternalSurface,
        };
        for dimension in ["model_requests", "target_requests"] {
            let b = budget::balance(
                &db,
                &actor.root_run_id,
                Some(&child.assignment_id),
                dimension,
            )
            .unwrap();
            assert_eq!((b.consumed, b.reserved, b.indeterminate), (1, 0, 0));
        }
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase IN ('decision','publication') WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='external-surface-output'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0);
        let tx = db.unchecked_transaction().unwrap();
        assert!(
            crate::scanning::external_surface_output_receipt::read_original_on(&tx, actor, &child)
                .unwrap()
                .is_some()
        );
        let diagnostic = serde_json::to_value(
            budget::supervision_capacity::diagnostic(&tx, &actor.root_run_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(diagnostic["pendingExternalSurfaceOutputs"], 1);
        tx.rollback().unwrap();
        // The original known invoice remains paid. Missing saved decision
        // blocks dispatch replay; this code does not reclassify the fee unknown.
        let before = super::tests::application_table_snapshot(&db);
        assert_eq!(
            multi_agent_external_surface(&f.h.context, actor).unwrap_err(),
            "root_tick_dispatch_unknown_no_retry"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    }
}

#[test]
fn external_surface_output_actual_original_token_31600_reserve_has_no_generic_local_floor() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let f = external_surface_output_actual_prepared_fixture_limits(false, None, 31600);
    let actor = &f.session.as_ref().unwrap().lease;
    let db = db::open(&f.h.db_path).unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let originalfee = budget::balance(&db, &actor.root_run_id, Some(""), "model_requests").unwrap();
    let result = multi_agent_external_surface(&f.h.context, actor).unwrap();
    assert_eq!(result["state"], "completed");
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
    let paidfee = budget::balance(&db, &actor.root_run_id, Some(""), "model_requests").unwrap();
    assert_eq!(paidfee.consumed, originalfee.consumed + 1);
    assert_eq!((paidfee.reserved, paidfee.indeterminate), (0, 0));
    let tx = db.unchecked_transaction().unwrap();
    let (request,dispatch):(String,String)=tx.query_row("SELECT q.fact_json,j.receipt_json FROM agent_root_tick_receipts q JOIN agent_root_model_journal j ON j.call_id=q.call_id AND j.phase='dispatch' JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='external-surface-output'",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let request: JsonValue = serde_json::from_str(&request).unwrap();
    let dispatch: JsonValue = serde_json::from_str(&dispatch).unwrap();
    let remaining = request["request"]["basis"]["budgetSnapshot"]["remainingModelTokens"]
        .as_i64()
        .unwrap();
    let estimate = dispatch["reservedTokens"].as_i64().unwrap();
    let after_reserve = remaining.checked_sub(estimate).unwrap();
    assert!((0..15000).contains(&after_reserve),"original paid Surface single request must fit below the generic local floor: remaining={remaining}, estimate={estimate}");
    let report = serde_json::to_value(
        budget::supervision_capacity::diagnostic(&tx, &actor.root_run_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["pendingExternalSurfaceOutputs"], 0);
    assert_eq!(tx.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='external-surface-output'",[&actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    let assignment:String=tx.query_row("SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='external_surface' AND state='completed' AND budget_settled_at<>''",[&actor.root_run_id],|r|r.get(0)).unwrap();
    for dimension in ["target_requests", "model_requests"] {
        let b = budget::balance(&tx, &actor.root_run_id, Some(&assignment), dimension).unwrap();
        assert_eq!((b.consumed, b.reserved, b.indeterminate), (1, 0, 0));
    }
    tx.rollback().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        multi_agent_external_surface(&f.h.context, actor).unwrap(),
        crate::agent_runtime::secrets::redact_json(&result)
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 2);
}
