// Real admitted pipeline producer; no SQL positive labels or ownership shim.
fn multi_terminal_owned_dispatch() -> (
    AgentHarness,
    OwnedAgentTargetOutcome,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    multi_terminal_owned_dispatch_with_setup(|_| {})
}
fn multi_terminal_owned_dispatch_with_setup(
    setup: impl FnOnce(&Path),
) -> (
    AgentHarness,
    OwnedAgentTargetOutcome,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let mut h = fresh_multi_production_harness("multi-terminal-owned");
    setup(&h.db_path);
    let target = h.context.target_url.clone();
    let executor_calls = AtomicUsize::new(0);
    let (port, seen, stop) = spawn_endpoint(Arc::new(move |wire| {
        let response = if wire.contains("You are the Root Coordinator") {
            if wire.contains("client-side-output") {
                proposal_model_response(&json!({"schemaVersion":1,"observed":["original paid configuration"],"missing":["missing_browser_validation"],"suggestions":["assess:client_side_configuration"],"costNotes":[],"risks":[]}).to_string())
            } else {
                fresh_multi_root_wire_response(&wire).unwrap()
            }
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"frozen frontend map","priorityContracts":[],"risks":[]}"#,
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
        } else if executor_calls.fetch_add(1, Ordering::SeqCst) == 0 {
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
    let root = owned
        .original_terminal
        .root_run_id
        .as_ref()
        .unwrap()
        .clone();
    h.context.run = Some(AgentRunLedger {
        db_path: h.db_path.clone(),
        run_id: root.clone(),
    });
    let db = db::open(&h.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1,
        "{:?}",
        owned.outcome
    );
    assert!(
        matches!(
            owned.outcome,
            AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)
        ),
        "{:?}",
        owned.outcome
    );
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    assert!(h.model_seen.lock().unwrap().len() >= 5);
    (h, owned, stop)
}

#[test]
fn multi_terminal_original_actual_owned_producer_consumer_and_replay_keep_original_fees() {
    let (h, owned, stop) = multi_terminal_owned_dispatch();
    let calls = h.model_seen.lock().unwrap().len();
    let db = db::open(&h.db_path).unwrap();
    let evidence = fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap();
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    let before = super::tests::application_table_snapshot(&db);
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    assert_eq!(
        fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap(),
        evidence
    );
    let root = owned
        .original_terminal
        .root_run_id
        .as_ref()
        .unwrap()
        .clone();
    let report = json!({"targets":tally.counted(),"originalRoot":root,"stopCode":owned.outcome.terminal_code(),"detail":owned.outcome.detail()});
    drop(owned);
    assert!(finish_native_branch(
        &h.db_path,
        &h.context.scan_id,
        1,
        "web",
        "partial",
        "actual closed Multi target",
        &report
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
    for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
        let b = crate::agent_runtime::multi_agent::budget::balance(&db, &root, None, dimension)
            .unwrap();
        assert_eq!((b.reserved, b.indeterminate), (0, 0));
    }
    // Disposable original audit preflight must succeed without weakening the
    // subsequent production deletion and idempotency requirements.
    let snapshot = db.unchecked_transaction().unwrap();
    let prepared = crate::agent_runtime::deleted_scan_audit::prepare(&snapshot, &h.context.scan_id)
        .unwrap_or_else(|error| panic!("original closed Multi audit preflight failed: {error}"));
    drop(prepared);
    snapshot.rollback().unwrap();
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    assert!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &h.context.scan_id).unwrap()
    );
    let deleted = super::tests::application_table_snapshot(&db);
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    assert!(super::tests::application_table_snapshot(&db) == deleted);
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn multi_terminal_original_actual_owned_corruption_stops_without_target_or_tally_writes() {
    let (h, owned, stop) = multi_terminal_owned_dispatch();
    let calls = h.model_seen.lock().unwrap().len();
    let db = db::open(&h.db_path).unwrap();
    db.execute(
        "UPDATE agent_runs SET terminal_reason='corrupted after actual owned return' WHERE id=?1",
        [owned.original_terminal.root_run_id.as_ref().unwrap()],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let mut tally = AgentPipelineTally::default();
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 0);
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    drop(owned);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
