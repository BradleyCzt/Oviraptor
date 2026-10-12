// Actual frozen v12 provider interruptions; no fake C, fees or clock.
type SurfaceInterruptionHook = std::sync::Arc<
    std::sync::Mutex<Option<(PathBuf, String, String, &'static str)>>,
>;
fn external_surface_interruption_prepared_fixture(
    local_tool: Option<&'static str>,
    pause: SurfaceInterruptionHook,
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
                if let Some((path, scan, root, status)) = pause.lock().unwrap().take() {
                    let database = db::open(&path).unwrap();
                    assert_eq!(
                        database
                            .query_row("SELECT scan_id FROM agent_runs WHERE id=?1", [&root], |r| r
                                .get::<_, String>(0))
                            .unwrap(),
                        scan
                    );
                    assert_eq!(
                        database
                            .execute(
                                "UPDATE sentinel_scans SET status=?2 WHERE id=?1",
                                params![scan, status]
                            )
                            .unwrap(),
                        1
                    );
                }
                if let Some(tool) = local_tool {
                    return (
                        200,
                        "application/json",
                        model_round(&[(tool, json!({}))], 10),
                    );
                }
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
fn external_surface_output_actual_scope_pause_retains_known_root_bill_and_pure_source() {
    external_surface_output_interruption_assert(None, Some("paused"));
}
#[test]
fn external_surface_output_actual_local_tools_are_paid_without_execution_or_publication() {
    for tool in ["snapshot.read", "plan.propose"] {
        external_surface_output_interruption_assert(Some(tool), None);
    }
}
fn external_surface_output_interruption_assert(
    tool: Option<&'static str>,
    scope_status: Option<&'static str>,
) {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let pause = std::sync::Arc::new(std::sync::Mutex::new(None));
    let f = external_surface_interruption_prepared_fixture(tool, pause.clone());
    let actor = &f.session.as_ref().unwrap().lease;
    let root = &actor.root_run_id;
    let db = db::open(&f.h.db_path).unwrap();
    if let Some(status) = scope_status {
        *pause.lock().unwrap() = Some((
            f.h.db_path.clone(),
            f.h.context.scan_id.clone(),
            root.clone(),
            status,
        ));
    }
    let before_root = budget::balance(&db, root, Some(""), "model_requests").unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let error = multi_agent_external_surface(&f.h.context, actor).unwrap_err();
    if tool.is_some() {
        assert!(
            error.contains("root_external_surface_output_local_tool_denied"),
            "{error}"
        );
    }
    if let Some(status) = scope_status {
        assert!(
            pause.lock().unwrap().is_none(),
            "pause executes only in the actual frozen Surface Root request"
        );
        assert_eq!(
            db.query_row(
                "SELECT status FROM sentinel_scans WHERE id=?1",
                [&f.h.context.scan_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            status
        );
    }
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    assert_eq!(
        f.h.model_seen.lock().unwrap().len(),
        sdk + 2,
        "one actual specialist plus one actual Root assessment"
    );
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='external_surface'",[root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment.clone(),
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::ExternalSurface,
    };
    let after_root = budget::balance(&db, root, Some(""), "model_requests").unwrap();
    assert_eq!(
        (
            after_root.consumed,
            after_root.reserved,
            after_root.indeterminate
        ),
        (before_root.consumed + 1, 0, 0),
        "the complete known Root reply must stay paid"
    );
    for dimension in ["target_requests", "model_requests"] {
        let fee = budget::balance(&db, root, Some(&assignment), dimension).unwrap();
        assert_eq!(
            (fee.consumed, fee.reserved, fee.indeterminate),
            (1, 0, 0),
            "{dimension}"
        );
    }
    let (call,invoice):(String,String)=db.query_row("SELECT q.call_id,j.receipt_json FROM agent_root_tick_receipts q JOIN agent_root_model_journal j ON j.call_id=q.call_id AND j.phase='received' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='external-surface-output'",[root],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let invoice: JsonValue = serde_json::from_str(&invoice).unwrap();
    assert_eq!(invoice["usageReported"], true);
    assert!(invoice["usage"]["totalTokens"].as_i64().unwrap() > 0);
    assert!(!invoice["responseHash"].as_str().unwrap().is_empty());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE call_id=?1 AND phase='publication'",[&call],|r|r.get::<_,i64>(0)).unwrap(),0);
    if tool.is_some() {
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE call_id=?1 AND phase='decision'",[&call],|r|r.get::<_,i64>(0)).unwrap(),0,"no parsed/executed local step can be retained");
    }
    let wires = f.h.model_seen.lock().unwrap();
    let assessment: Vec<JsonValue> = wires
        .iter()
        .filter_map(|wire| {
            let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n")?.1).ok()?;
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str()?).ok()?;
            (input["basis"]["phase"] == "external-surface-output").then_some(body)
        })
        .collect();
    assert_eq!(assessment.len(), 1);
    assert!(assessment[0]["tools"]
        .as_array()
        .is_none_or(|items| items.is_empty()));
    drop(wires);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::commands::external_surface_output_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .is_some(),
        "closed original source remains readable after scope loss"
    );
    assert!(
        native_coordinator_external_surface_output_publication_replay_on(&tx, actor, &child)
            .unwrap()
            .is_none()
    );
    let report = serde_json::to_value(
        budget::supervision_capacity::diagnostic(&tx, root)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["pendingExternalSurfaceOutputs"], 1);
    assert!(
        report["freshWorkBlocked"].is_null(),
        "known original fees are not unknown debt"
    );
    tx.rollback().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(
        native_coordinator_external_surface_output_feedback(&f.h.context, actor, &child).is_err()
    );
    assert_eq!(
        super::tests::application_table_snapshot(&db),
        before,
        "failed original re-entry neither rewrites source nor issues a new call"
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn external_surface_output_actual_scope_cancel_retains_known_root_bill_and_pure_source() {
    // Same original endpoint returns a complete 200 invoice after observing the
    // real scan cancellation. If the gateway instead loses the response, this
    // known-cost acceptance fails; no fixture manufactures settlement.
    external_surface_output_interruption_assert(None, Some("cancelled"));
}
#[test]
fn external_surface_output_actual_paid_publication_reentry_after_pause_has_zero_effects() {
    let _real = RealSpecialistTransport::enter();
    let f = external_surface_output_actual_prepared_fixture();
    let actor = &f.session.as_ref().unwrap().lease;
    assert_eq!(
        multi_agent_external_surface(&f.h.context, actor).unwrap()["state"],
        "completed"
    );
    let db = db::open(&f.h.db_path).unwrap();
    let (assignment,run):(String,String)=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='external_surface'",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::ExternalSurface,
    };
    assert_eq!(
        db.execute(
            "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
            [&f.h.context.scan_id]
        )
        .unwrap(),
        1
    );
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    let tx = db.unchecked_transaction().unwrap();
    let source =
        crate::commands::external_surface_output_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .unwrap();
    let id = source.receipt_id().to_string();
    let original =
        native_coordinator_external_surface_output_publication_replay_on(&tx, actor, &child)
            .unwrap()
            .expect("real paid publication remains historical after pause");
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
    tx.rollback().unwrap();
    // Invoke the real foreground entry, not just its pure replay helper.
    native_coordinator_external_surface_output_feedback(&f.h.context, actor, &child).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        crate::commands::external_surface_output_receipt::read_original_on(&tx, actor, &child)
            .unwrap()
            .unwrap()
            .receipt_id(),
        id
    );
    assert_eq!(
        native_coordinator_external_surface_output_publication_replay_on(&tx, actor, &child)
            .unwrap(),
        Some(original)
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
