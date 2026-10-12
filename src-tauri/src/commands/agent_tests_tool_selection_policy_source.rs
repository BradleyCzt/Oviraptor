struct OriginalSelectionPolicyFixture {
    root: PathBuf,
    path: PathBuf,
    scan: String,
    request: String,
    work: PathBuf,
    _supply: crate::tool_supply::frozen_plan::tests::TestSupplyFixture,
}
impl Drop for OriginalSelectionPolicyFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn original_selection_policy_fixture() -> OriginalSelectionPolicyFixture {
    use crate::agent_runtime::web_mode::{draft, WebMode};
    use crate::tool_supply::{frozen_plan::tests::TestSupplyFixture, selection};
    let root =
        std::env::temp_dir().join(format!("oviraptor-original-selection-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let supply = TestSupplyFixture::approve_on(&path).unwrap();
    let mut db = db::open(&path).unwrap();
    db.execute(
        "INSERT INTO projects(id,name,status) VALUES(9001,'original selection policy','active')",
        [],
    )
    .unwrap();
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let window = draft::DraftCreationWindow::begin(&tx).unwrap();
    let scan = Uuid::new_v4().to_string();
    let target = "https://authorized.example.test/";
    // The ordinary draft inputs exist inside the actual creation window; the
    // private register_new factory and stage producer own their real receipts.
    tx.execute("INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name) VALUES(?1,9001,'original selection policy','draft','draft','','web','original selection policy')",[&scan]).unwrap();
    tx.execute("INSERT INTO sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal','{}')",[&scan]).unwrap();
    tx.execute("INSERT INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(9001,?1,'original selection policy',?2,'queued')",params![scan,target]).unwrap();
    let fresh = draft::register_new(window, &scan, WebMode::Multi).unwrap();
    let request = Uuid::new_v4().to_string();
    let declared = selection::DeclaredSelectionRequest::parse(&json!({"schemaVersion":1,"requestId":request,"scope":{"scanId":scan,"attemptNumber":1,"targetUrls":[target]},"selection":supply.tool_supply()})).unwrap();
    let pending = selection::stage_for_new_draft(&fresh, supply.configuration(), Some(&declared))
        .unwrap()
        .unwrap();
    assert_eq!(pending.fact()["executionAllowed"], false);
    assert_eq!(
        pending.fact()["signatureStage"],
        "pending_original_mode_policy_signature"
    );
    drop(fresh);
    tx.commit().unwrap();
    let work = web_mode_test_start(&root, &mut db, &scan, WebStartMode::Confirm).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_runs", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_root_model_journal", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    OriginalSelectionPolicyFixture {
        root,
        path,
        scan,
        request,
        work,
        _supply: supply,
    }
}
#[test]
fn tool_selection_policy_source_actual_private_startup_verifies_original_pending_request() {
    use crate::tool_supply::selection;
    let f = original_selection_policy_fixture();
    let db = db::open(&f.path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let source =
        selection::OriginalSignedToolSelectionPolicy::capture_original_on(&db, &f.scan, 1).unwrap();
    let verified = selection::verify_signed_request_on(&db, &source, &f.request).unwrap();
    assert_eq!(verified.request().as_value()["scope"]["scanId"], f.scan);
    assert_eq!(verified.request().as_value()["scope"]["attemptNumber"], 1);
    assert_eq!(
        verified.request().selection().as_plan_value(),
        f._supply.tool_supply()
    );
    assert_eq!(verified.fact()["executionAllowed"], false);
    assert_eq!(
        verified.fact()["purpose"],
        "original_signed_new_draft_tool_selection"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}
#[test]
fn tool_selection_policy_source_task_policy_and_pending_scope_drift_refuse_without_writes() {
    use crate::tool_supply::selection;
    let f = original_selection_policy_fixture();
    let db = db::open(&f.path).unwrap();
    let source =
        selection::OriginalSignedToolSelectionPolicy::capture_original_on(&db, &f.scan, 1).unwrap();
    let original_task = fs::read(f.work.join("task.json")).unwrap();
    let mut task: JsonValue = serde_json::from_slice(&original_task).unwrap();
    task["effectiveWebPolicy"]["unexpectedUnapprovedSelection"] = json!(true);
    fs::write(f.work.join("task.json"), serde_json::to_vec(&task).unwrap()).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(
        selection::OriginalSignedToolSelectionPolicy::capture_original_on(&db, &f.scan, 1).is_err()
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    fs::write(f.work.join("task.json"), &original_task).unwrap();
    let mut task: JsonValue = serde_json::from_slice(&original_task).unwrap();
    task["scanId"] = json!("foreign-scan");
    fs::write(f.work.join("task.json"), serde_json::to_vec(&task).unwrap()).unwrap();
    assert!(
        selection::OriginalSignedToolSelectionPolicy::capture_original_on(&db, &f.scan, 1).is_err()
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    fs::write(f.work.join("task.json"), &original_task).unwrap();
    // Original source was actually signed, not constructed from caller JSON.
    let trigger: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='tool_selection_no_update'",[],|r|r.get(0)).unwrap();
    let scope: String = db
        .query_row(
            "SELECT scope_json FROM native_tool_selection_requests WHERE request_id=?1",
            [&f.request],
            |r| r.get(0),
        )
        .unwrap();
    let mut damaged: JsonValue = serde_json::from_str(&scope).unwrap();
    damaged["scanId"] = json!("foreign-scan");
    db.execute_batch("DROP TRIGGER tool_selection_no_update")
        .unwrap();
    db.execute(
        "UPDATE native_tool_selection_requests SET scope_json=?1 WHERE request_id=?2",
        params![damaged.to_string(), f.request],
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    let damaged_rows = super::tests::application_table_snapshot(&db);
    assert_eq!(
        selection::verify_signed_request_on(&db, &source, &f.request)
            .err()
            .unwrap(),
        "tool_selection_original_pending_changed"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), damaged_rows);
    db.execute_batch("DROP TRIGGER tool_selection_no_update")
        .unwrap();
    db.execute(
        "UPDATE native_tool_selection_requests SET scope_json=?1 WHERE request_id=?2",
        params![scope, f.request],
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    let restored =
        selection::OriginalSignedToolSelectionPolicy::capture_original_on(&db, &f.scan, 1).unwrap();
    assert!(selection::verify_signed_request_on(&db, &restored, &f.request).is_ok());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[test]
fn tool_selection_policy_source_actual_production_fresh_root_freezes_without_test_birth_hook() {
    use crate::tool_supply::frozen_plan;
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
    let adaptive = AgentBudgetSettings::from_json(&json!({"agentBackendPolicy":"native"}));
    let candidate = build_agent_execution_plan(
        &adaptive,
        &route,
        &environment,
        AgentBackendKind::Native,
        &f.path,
        &f.scan,
    )
    .with_attempt(1);
    assert!(candidate.tool_supply.is_none());
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_tool_selection_for_original_startup(&f.path, &f.scan, 1, &target, &candidate, None)
            .err()
            .unwrap(),
        "tool_selection_app_configuration_missing"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let selected = native_tool_selection_for_original_startup(
        &f.path,
        &f.scan,
        1,
        &target,
        &candidate,
        Some(f._supply.configuration()),
    )
    .unwrap();
    assert_eq!(
        selected.tool_supply.as_ref().unwrap().as_plan_value(),
        f._supply.tool_supply()
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    // No scoped BirthGuard/decorator is installed. The production writer must
    // reject a selected plan without its application-owned trust configuration.
    assert!(persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &target, &selected).is_err());
    assert_eq!(
        super::tests::application_table_snapshot(&db),
        before,
        "failed original fresh birth must roll back every sidecar and invoice"
    );
    persist_frozen_web_execution_plan_with_tool_configuration(
        &f.path,
        &f.scan,
        1,
        &target,
        &selected,
        Some(f._supply.configuration()),
    )
    .unwrap();
    let (root,hash,text):(String,String,String) = db.query_row("SELECT id,plan_hash,plan_json FROM agent_runs WHERE scan_id=?1 AND attempt_number=1 AND target_url=?2 AND role='coordinator'",params![f.scan,target],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(hash, selected.hash());
    assert_eq!(text, selected.as_json().to_string());
    let snapshot = db.unchecked_transaction().unwrap();
    let private_mode = private_web_mode_on(&snapshot, &f.scan, 1).unwrap();
    crate::agent_runtime::web_mode::root::verify_birth(&snapshot, &root, &private_mode).unwrap();
    let budget = crate::agent_runtime::multi_agent::budget::root_definition::read(&snapshot, &root)
        .unwrap()
        .unwrap();
    assert_eq!(budget.plan_hash, selected.hash());
    budget
        .validate(&f.scan, 1, &target, &selected.hash(), &selected.as_json())
        .unwrap();
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&snapshot, &root)
        .unwrap();
    let sealed = frozen_plan::read_historical_on(&snapshot, f._supply.configuration(), &root)
        .unwrap()
        .unwrap();
    assert_eq!(sealed.fact()["scope"]["rootRunId"], root);
    assert_eq!(sealed.fact()["scope"]["scanId"], f.scan);
    assert_eq!(sealed.fact()["scope"]["targetUrl"], target);
    assert_eq!(
        sealed.fact()["tools"][0]["approvalId"],
        f._supply.approval_id()
    );
    assert_eq!(
        snapshot
            .query_row(
                "SELECT count(*) FROM agent_frozen_tool_supply_plans WHERE root_run_id=?1",
                [&root],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let current =
        frozen_plan::require_current_on(&snapshot, f._supply.configuration(), &root).unwrap();
    assert_eq!(current.fact(), sealed.fact());
    snapshot.rollback().unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_root_model_journal", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}
