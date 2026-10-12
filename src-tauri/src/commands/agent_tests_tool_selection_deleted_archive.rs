// A real selected startup + production fresh Root, terminated through the
// actual target entry's missing-evidence stop. This is zero-SDK closure evidence,
// not an assessment, sandbox execution or App-invoke acceptance test.
fn tool_selection_original_zero_sdk_closed_archive_fixture() -> (
    OriginalSelectionPolicyFixture,
    String,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let f = original_selection_policy_fixture();
    let db = db::open(&f.path).unwrap();
    let target: String = db
        .query_row(
            "SELECT url FROM sentinel_targets WHERE scan_id=?1",
            [&f.scan],
            |r| r.get(0),
        )
        .unwrap();
    let mut route = test_route("standard", "framework_application");
    route.url = target.clone();
    let environment = test_environment("cloud");
    let settings = json!({"agentBackendPolicy":"native"});
    let adaptive = AgentBudgetSettings::from_json(&settings);
    let candidate = build_agent_execution_plan(
        &adaptive,
        &route,
        &environment,
        AgentBackendKind::Native,
        &f.path,
        &f.scan,
    )
    .with_attempt(1);
    let selected = native_tool_selection_for_original_startup(
        &f.path,
        &f.scan,
        1,
        &target,
        &candidate,
        Some(f._supply.configuration()),
    )
    .unwrap();
    persist_frozen_web_execution_plan_with_tool_configuration(
        &f.path,
        &f.scan,
        1,
        &target,
        &selected,
        Some(f._supply.configuration()),
    )
    .unwrap();
    let root: String = db
        .query_row(
            "SELECT id FROM agent_runs WHERE scan_id=?1 AND role='coordinator'",
            [&f.scan],
            |r| r.get(0),
        )
        .unwrap();
    let actor = db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[&root],|r|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {
        scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
    })).unwrap();
    let missing = f.work.join("actual-missing-local-evidence");
    fs::create_dir_all(&missing).unwrap();
    let prepared = PreparedFrontendTarget {
        position: 1,
        route,
        target_dir: missing,
        proxy: None,
        browser: None,
    };
    let owned = run_agent_target_with_tool_configuration(
        &prepared,
        AgentTargetExecution {
            db_path: &f.path,
            scan_id: &f.scan,
            attempt_number: 1,
            settings: &settings,
            environment: &environment,
            adaptive: &adaptive,
            log_path: &f.root.join("selected-zero-sdk-exit.log"),
        },
        Some(f._supply.configuration()),
    )
    .unwrap();
    assert!(matches!(owned.outcome, AgentTargetOutcome::Failed(_)));
    assert!(
        owned.outcome.detail().contains("缺少本地证据包"),
        "{}",
        owned.outcome.detail()
    );
    finish_coordinator_run(&db, &actor, &owned.outcome).unwrap();
    drop(owned);
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_root_model_journal", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    assert!(db
        .query_row(
            "SELECT status='terminal' FROM agent_runs WHERE id=?1",
            [&root],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    let attempt = request_sentinel_pause(&f.path, &f.scan).unwrap();
    assert_eq!(attempt, 1);
    assert!(finish_sentinel_pause(&f.path, &f.scan, attempt).unwrap());
    (f, root, actor)
}
#[test]
fn tool_selection_deleted_archive_zero_sdk_exit_cannot_replace_original_paid_root_decision() {
    let (f, _root, _actor) = tool_selection_original_zero_sdk_closed_archive_fixture();
    let db = db::open(&f.path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let error = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &f.scan)
        .err()
        .unwrap();
    assert_eq!(error, "root_decision_original_paid_request_required");
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

fn tool_selection_original_paid_closed_archive_fixture() -> RootTickFixture {
    tool_selection_original_paid_closed_archive_fixture_with_branch_finish(false)
}
fn tool_selection_original_paid_closed_archive_fixture_with_branch_finish(
    finish_branch: bool,
) -> RootTickFixture {
    let (mut f, supply, _) = frozen_tool_supply_original_birth_fixture(Some(true));
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200,"application/json",proposal_model_response(&json!({"schemaVersion":1,"observed":["original bounded startup"],"missing":[],"suggestions":[],"costNotes":[],"risks":[]}).to_string()))
    }));
    f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let _real = RealSpecialistTransport::enter();
    native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(f.parent.take());
    let missing = f.context.target_dir.join("actual-missing-local-evidence");
    fs::create_dir_all(&missing).unwrap();
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: f.context.route.clone(),
        target_dir: missing,
        proxy: None,
        browser: None,
    };
    let settings = json!({"agentBackendPolicy":"native"});
    let adaptive = AgentBudgetSettings::from_json(&settings);
    let owned = run_agent_target_with_tool_configuration(
        &prepared,
        AgentTargetExecution {
            db_path: &f.context.db_path,
            scan_id: &f.actor.scan_id,
            attempt_number: f.actor.attempt_number,
            settings: &settings,
            environment: &f.context.environment,
            adaptive: &adaptive,
            log_path: &f.context.log_path,
        },
        Some(supply.configuration()),
    )
    .unwrap();
    assert!(matches!(owned.outcome, AgentTargetOutcome::Failed(_)));
    assert!(
        owned.outcome.detail().contains("缺少本地证据包"),
        "{}",
        owned.outcome.detail()
    );
    let db = db::open(&f.context.db_path).unwrap();
    finish_coordinator_run(&db, &f.actor, &owned.outcome).unwrap();
    let mut tally = AgentPipelineTally::default();
    if finish_branch {
        // Consume the genuine original target outcome while its owner is held.
        // A Failed outcome may stop the pipeline; counted() proves publication.
        let _continued = record_owned_agent_target_outcome(
            &f.context.db_path,
            &f.actor.scan_id,
            &f.context.route,
            &owned,
            &mut tally,
        );
        assert_eq!(tally.counted(), 1);
    }
    drop(owned);
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_model_journal WHERE phase='received'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    if finish_branch {
        assert!(finish_native_branch(
            &f.context.db_path,&f.actor.scan_id,f.actor.attempt_number,"web","failed",
            "Actual selected target stopped before model execution because local evidence was absent",
            &json!({"targets":tally.counted(),"originalRoot":f.actor.root_run_id}),
        ).unwrap());
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM native_scan_branches WHERE scan_id=?1 AND status='pending'",
                [&f.actor.scan_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT status FROM sentinel_scans WHERE id=?1",
                [&f.actor.scan_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "failed"
        );
    } else {
        let attempt = request_sentinel_pause(&f.context.db_path, &f.actor.scan_id).unwrap();
        assert!(finish_sentinel_pause(&f.context.db_path, &f.actor.scan_id, attempt).unwrap());
    }
    supply.revoke_on(&f.context.db_path).unwrap();
    assert_eq!(
        crate::tool_supply::frozen_plan::read_historical_on(
            &db,
            supply.configuration(),
            &f.actor.root_run_id
        )
        .err()
        .unwrap(),
        "tool_supply_historical_trust_unavailable:tool_administrator_approval_revoked"
    );
    f
}

#[test]
fn tool_selection_deleted_archive_actual_paid_root_exit_retains_revoked_original_signed_source() {
    let f = tool_selection_original_paid_closed_archive_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let archive = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &f.actor.scan_id).unwrap();
    archive.verify_scope(&tx, &f.actor.scan_id).unwrap();
    drop(archive);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_frozen_tool_supply_plans WHERE root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_tool_selection_requests WHERE scan_id=?1",
            [&f.actor.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn tool_selection_deleted_archive_paid_original_supply_signature_damage_refuses_retention() {
    let f = tool_selection_original_paid_closed_archive_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let root = &f.actor.root_run_id;
    let original: Vec<u8> = db
        .query_row(
            "SELECT signature FROM agent_frozen_tool_supply_plans WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    let trigger: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='frozen_tool_supply_no_update'",[],|r|r.get(0)).unwrap();
    let mut damaged = original.clone();
    damaged[0] ^= 1;
    db.execute_batch("DROP TRIGGER frozen_tool_supply_no_update")
        .unwrap();
    db.execute(
        "UPDATE agent_frozen_tool_supply_plans SET signature=?1 WHERE root_run_id=?2",
        params![damaged, root],
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(crate::agent_runtime::deleted_scan_audit::prepare(&tx, &f.actor.scan_id).is_err());
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    db.execute_batch("DROP TRIGGER frozen_tool_supply_no_update")
        .unwrap();
    db.execute(
        "UPDATE agent_frozen_tool_supply_plans SET signature=?1 WHERE root_run_id=?2",
        params![original, root],
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    let restored = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let archive = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &f.actor.scan_id).unwrap();
    archive.verify_scope(&tx, &f.actor.scan_id).unwrap();
    drop(archive);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), restored);
}

#[test]
fn tool_selection_deleted_archive_actual_physical_delete_preserves_selected_paid_and_global_sources(
) {
    use crate::agent_runtime::multi_agent::attempts::audit_rows::Rows;
    let f = tool_selection_original_paid_closed_archive_fixture_with_branch_finish(true);
    let db = db::open(&f.context.db_path).unwrap();
    let retained = [
        "agent_frozen_tool_supply_plans",
        "native_tool_selection_requests",
        "agent_root_model_journal",
        "agent_budget_entries",
        "agent_model_cost_facts",
    ];
    let original = retained.map(|table| {
        Rows::read(
            &db,
            &format!("SELECT rowid,* FROM {table} ORDER BY rowid"),
            [],
        )
        .unwrap()
    });
    let administrators = Rows::read(
        &db,
        "SELECT rowid,* FROM tool_administrator_decisions ORDER BY rowid",
        [],
    )
    .unwrap();
    assert_eq!(
        administrators.values.len(),
        2,
        "one original approval plus its actual revocation"
    );
    let task_bytes = fs::read(f.f.work.join("task.json")).unwrap();
    let evidence_bytes = fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap();
    let predelete = db.unchecked_transaction().unwrap();
    let audit =
        crate::agent_runtime::deleted_scan_audit::prepare(&predelete, &f.actor.scan_id).unwrap();
    require_scan_deletion_with_audit_in(&predelete, &f.actor.scan_id, Some(&audit))
        .expect("original deletion obligations must close before the actual delete writer");
    drop(audit);
    predelete.rollback().unwrap();
    delete_sentinel_scan_inner(&f.context.db_path, &f.actor.scan_id).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_scans WHERE id=?1",
            [&f.actor.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_targets WHERE scan_id=?1",
            [&f.actor.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &f.actor.scan_id).unwrap()
    );
    for (table, before) in retained.into_iter().zip(original) {
        assert!(
            Rows::read(
                &db,
                &format!("SELECT rowid,* FROM {table} ORDER BY rowid"),
                []
            )
            .unwrap()
                == before,
            "retained original {table}"
        );
    }
    assert!(
        Rows::read(
            &db,
            "SELECT rowid,* FROM tool_administrator_decisions ORDER BY rowid",
            []
        )
        .unwrap()
            == administrators,
        "task-external administrator inventory is not a scan deletion target"
    );
    let raw: String = db
        .query_row(
            "SELECT audit_json FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
            [&f.actor.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    let bundle: JsonValue = serde_json::from_str(&raw).unwrap();
    let tables = bundle["tables"].as_array().unwrap();
    assert!(
        !tables
            .iter()
            .any(|table| table["name"] == "tool_administrator_decisions"),
        "global inventory must not be copied into the task archive"
    );
    for name in [
        "agent_frozen_tool_supply_plans",
        "native_tool_selection_requests",
    ] {
        let table = tables.iter().find(|table| table["name"] == name).unwrap();
        assert_eq!(table["rows"].as_array().unwrap().len(), 1);
    }
    assert_eq!(fs::read(f.f.work.join("task.json")).unwrap(), task_bytes);
    assert_eq!(
        fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap(),
        evidence_bytes
    );
    let after = super::tests::application_table_snapshot(&db);
    delete_sentinel_scan_inner(&f.context.db_path, &f.actor.scan_id).unwrap();
    assert!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &f.actor.scan_id).unwrap()
    );
    assert_eq!(
        super::tests::application_table_snapshot(&db),
        after,
        "physical-delete retry only verifies durable original history"
    );
}
