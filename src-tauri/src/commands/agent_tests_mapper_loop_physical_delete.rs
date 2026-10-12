#[test]
fn mapper_loop_physical_delete_actual_pipeline_exit_retains_paid_sources_and_replays_free() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let mut h = fresh_multi_production_harness("mapper-original-physical-delete");
    let target = h.context.target_url.clone();
    let executors = AtomicUsize::new(0);
    let (port, seen, stop) = spawn_endpoint(Arc::new(move |wire| {
        let response = if wire.contains("You are the Root Coordinator") {
            let body: JsonValue =
                serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let step = match input["basis"]["phase"].as_str() {
                Some("external-surface-output") => Some("assess:external_surface_output"),
                Some("capability-changed") => Some("assess:capability_changed"),
                _ => None,
            };
            if let Some(step) = step {
                proposal_model_response(&json!({"schemaVersion":1,"observed":["original closed source"],"missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string())
            } else if wire.contains("client-side-output") {
                proposal_model_response(&json!({"schemaVersion":1,"observed":["original paid configuration"],"missing":["missing_browser_validation"],"suggestions":["assess:client_side_configuration"],"costNotes":[],"risks":[]}).to_string())
            } else {
                fresh_multi_root_wire_response(&wire).unwrap()
            }
        } else if wire.contains("Analyze frozen frontend evidence only.") {
            sdk_exact_model_body(
                r#"{"summary":"original frozen frontend map","priorityContracts":[],"risks":[]}"#,
            )
        } else if wire.contains("External Surface") {
            proposal_model_response(public_surface_model_text())
        } else if wire.contains("独立ClientSide配置事实专家") {
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
        } else if executors.fetch_add(1, Ordering::SeqCst) == 0 {
            model_round(
                &[(
                    "replay_http",
                    json!({"identity":"anonymous","method":"GET","url":format!("{target}/"),"family":"authorization"}),
                )],
                10,
            )
        } else {
            model_round(&[], 10)
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    let birth = MapperPhysicalNewBirthGuard::install(&h);
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: h.context.route.clone(),
        target_dir: h.context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    let settings = json!({"agentBackendPolicy":"native","agentNoToolTurnLimit":2});
    let adaptive = AgentBudgetSettings::from_json(&settings);
    let _real = RealSpecialistTransport::enter();
    let owned = run_agent_target(
        &prepared,
        AgentTargetExecution {
            db_path: &h.db_path,
            scan_id: &h.context.scan_id,
            attempt_number: 1,
            settings: &settings,
            environment: &h.context.environment,
            adaptive: &adaptive,
            log_path: &h.context.log_path,
        },
    )
    .unwrap();
    drop(birth);
    let root = owned
        .original_terminal
        .root_run_id
        .as_ref()
        .unwrap()
        .clone();
    let db = db::open(&h.db_path).unwrap();
    assert_eq!(
        crate::agent_runtime::web_mode::root::read(&db, &root)
            .unwrap()
            .unwrap()
            .bootstrap_dispatch()
            .unwrap()["schemaVersion"],
        15
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_mapper_loop_final_receipts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_mapper_loop_journal WHERE root_run_id=?1 AND phase='received'",[&root],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls c JOIN agent_assignments a ON a.id=c.assignment_id WHERE c.root_run_id=?1 AND a.role='spa_api_mapper'",[&root],|r|r.get::<_,i64>(0)).unwrap(),0);
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    drop(owned);
    assert!(finish_native_branch(
        &h.db_path,
        &h.context.scan_id,
        1,
        "web",
        "partial",
        "Original v15 owned target exit",
        &json!({"targets":tally.counted(),"originalRoot":root})
    )
    .unwrap());
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&h.context.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "partial"
    );
    let calls = h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let prepared_audit = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &h.context.scan_id)
        .expect("actual pipeline must prove original worker closures and paid sources before physical deletion");
    prepared_audit
        .verify_scope(&tx, &h.context.scan_id)
        .unwrap();
    drop(prepared_audit);
    tx.rollback().unwrap();
    let evidence = fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap();
    let retained = [
        "agent_mapper_loop_journal",
        "agent_mapper_loop_final_receipts",
        "agent_budget_entries",
        "agent_model_cost_facts",
        "native_sdk_log_owners",
        "native_sdk_log_rows",
    ];
    let rows = retained.map(|table| {
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            &format!("SELECT rowid,* FROM {table} ORDER BY rowid"),
            [],
        )
        .unwrap()
    });
    let sidecar = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT * FROM native_sdk_log_mapper_owners ORDER BY owner_id",
        [],
    )
    .unwrap();
    delete_sentinel_scan_inner(&h.context.db_path, &h.context.scan_id).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_scans WHERE id=?1",
            [&h.context.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &h.context.scan_id).unwrap()
    );
    for (table, original) in retained.into_iter().zip(rows) {
        assert!(
            crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
                &db,
                &format!("SELECT rowid,* FROM {table} ORDER BY rowid"),
                []
            )
            .unwrap()
                == original,
            "{table}"
        );
    }
    assert!(
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            "SELECT * FROM native_sdk_log_mapper_owners ORDER BY owner_id",
            []
        )
        .unwrap()
            == sidecar
    );
    let before = super::tests::application_table_snapshot(&db);
    delete_sentinel_scan_inner(&h.context.db_path, &h.context.scan_id).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(
        fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap(),
        evidence
    );
    stop.store(true, Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
