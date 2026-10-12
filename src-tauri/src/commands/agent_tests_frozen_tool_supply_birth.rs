// The decorator changes only this test's newly published request. Approval
// authority remains the actual administrator signature verified at birth.
struct FrozenSupplyBirthRequest {
    db_path: PathBuf,
    scan: String,
    attempt: i64,
    target: String,
    supply: Option<JsonValue>,
    original_plan: JsonValue,
    decorated: usize,
}
thread_local! {
    static FROZEN_SUPPLY_BIRTH_REQUEST: std::cell::RefCell<Option<FrozenSupplyBirthRequest>> = const { std::cell::RefCell::new(None) };
}
struct FrozenSupplyBirthRequestGuard;
impl FrozenSupplyBirthRequestGuard {
    fn install(
        path: &Path,
        scan: &str,
        attempt: i64,
        target: &str,
        supply: Option<JsonValue>,
        original_plan: JsonValue,
    ) -> Self {
        FROZEN_SUPPLY_BIRTH_REQUEST.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some(FrozenSupplyBirthRequest {
                db_path: path.into(),
                scan: scan.into(),
                attempt,
                target: target.into(),
                supply,
                original_plan,
                decorated: 0,
            });
        });
        Self
    }
}
impl Drop for FrozenSupplyBirthRequestGuard {
    fn drop(&mut self) {
        FROZEN_SUPPLY_BIRTH_REQUEST.with(|slot| {
            let scope = slot.borrow_mut().take().unwrap();
            if !std::thread::panicking() {
                assert_eq!(scope.decorated, 1);
            }
        });
    }
}
// Only the exact new test scope replaces the common fixture's synthetic
// ceilings before private birth. Existing roots and all other fixtures are
// untouched; subsequent full original-plan equality remains mandatory.
fn native_frozen_tool_supply_prebirth_fixture_plan_for_test(
    path: &Path,
    scan: &str,
    attempt: i64,
    target: &str,
    candidate: &AgentExecutionPlan,
) -> Result<AgentExecutionPlan, String> {
    FROZEN_SUPPLY_BIRTH_REQUEST.with(|slot| {
        let borrowed = slot.borrow();
        let Some(scope) = borrowed.as_ref() else {
            return Ok(candidate.clone());
        };
        if scope.db_path != path
            || scope.scan != scan
            || scope.attempt != attempt
            || scope.target != target
        {
            return Ok(candidate.clone());
        }
        if scope.decorated != 0
            || db::open(path)?
                .query_row(
                    "SELECT count(*) FROM main.agent_runs WHERE scan_id=?1",
                    [scan],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())?
                != 0
        {
            return Err("tool_supply_test_prebirth_scope_changed".into());
        }
        let plan = AgentExecutionPlan::from_json(&scope.original_plan)
            .ok_or("tool_supply_test_prebirth_plan_invalid")?;
        if plan.as_json() != scope.original_plan
            || plan.attempt_number != attempt
            || plan.target_url != target
            || plan.backend != AgentBackendKind::Native
            || plan.tool_supply.is_some()
        {
            return Err("tool_supply_test_prebirth_plan_invalid".into());
        }
        Ok(plan)
    })
}

pub(super) fn native_frozen_tool_supply_plan_for_test(
    path: &Path,
    scan: &str,
    attempt: i64,
    target: &str,
    mut original: JsonValue,
) -> Result<JsonValue, String> {
    FROZEN_SUPPLY_BIRTH_REQUEST.with(|slot| {
        let mut scoped = slot.borrow_mut();
        let Some(scope) = scoped.as_mut() else {
            return Ok(original);
        };
        if scope.db_path != path
            || scope.scan != scan
            || scope.attempt != attempt
            || scope.target != target
        {
            return Ok(original);
        }
        let db = db::open(path)?;
        if scope.decorated != 0
            || original.get("toolSupply").is_some()
            || db
                .query_row(
                    "SELECT count(*) FROM main.agent_runs WHERE scan_id=?1",
                    [scan],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())?
                != 0
        {
            return Err("tool_supply_test_fresh_request_conflict".into());
        }
        if original != scope.original_plan {
            return Err("tool_supply_test_prebirth_input_changed".into());
        }
        let object = original
            .as_object_mut()
            .ok_or("tool_supply_test_original_plan_invalid")?;
        if let Some(supply) = &scope.supply {
            object.insert("toolSupply".into(), supply.clone());
        }
        scope.decorated += 1;
        Ok(original)
    })
}

#[cfg(unix)]
fn frozen_tool_supply_original_birth_fixture(
    with_tools: Option<bool>,
) -> (
    RootTickFixture,
    crate::tool_supply::frozen_plan::tests::TestSupplyFixture,
    Option<JsonValue>,
) {
    frozen_tool_supply_original_birth_with(with_tools,crate::tool_supply::frozen_plan::tests::TestSupplyFixture::approve_on)
}
#[cfg(unix)]
fn frozen_tool_supply_original_birth_with(
    with_tools: Option<bool>,
    make_supply: impl FnOnce(&Path)->Result<crate::tool_supply::frozen_plan::tests::TestSupplyFixture,String>,
) -> (RootTickFixture,crate::tool_supply::frozen_plan::tests::TestSupplyFixture,Option<JsonValue>) {
    frozen_tool_supply_original_birth_with_mode(with_tools,make_supply,false)
}
#[cfg(unix)]
fn frozen_tool_supply_original_birth_with_mode(
    with_tools: Option<bool>,
    make_supply: impl FnOnce(&Path)->Result<crate::tool_supply::frozen_plan::tests::TestSupplyFixture,String>,
    execution: bool,
) -> (RootTickFixture,crate::tool_supply::frozen_plan::tests::TestSupplyFixture,Option<JsonValue>) {
    frozen_tool_supply_original_birth_with_task(with_tools,make_supply,execution,"https://authorized.example.test",|_,_,_|None)
}
#[cfg(unix)]
fn frozen_tool_supply_original_birth_with_task(
    with_tools:Option<bool>,make_supply:impl FnOnce(&Path)->Result<crate::tool_supply::frozen_plan::tests::TestSupplyFixture,String>,execution:bool,target:&str,
    prepare:impl FnOnce(&rusqlite::Connection,&Path,&str)->Option<(Vec<String>,String)>,
) -> (RootTickFixture,crate::tool_supply::frozen_plan::tests::TestSupplyFixture,Option<JsonValue>){
    frozen_tool_supply_original_birth_with_task_controls(with_tools,make_supply,execution,target,prepare,&[])
}

/// Birth variant for fixtures that need more than the default control group:
/// every extra control is registered through the same operator input path
/// before the scan starts (controls cannot be added once a task is live).
#[cfg(unix)]
fn frozen_tool_supply_original_birth_with_task_controls(
    with_tools:Option<bool>,make_supply:impl FnOnce(&Path)->Result<crate::tool_supply::frozen_plan::tests::TestSupplyFixture,String>,execution:bool,target:&str,
    prepare:impl FnOnce(&rusqlite::Connection,&Path,&str)->Option<(Vec<String>,String)>,
    extra_controls:&[SaveAuthorizationControlInput],
) -> (RootTickFixture,crate::tool_supply::frozen_plan::tests::TestSupplyFixture,Option<JsonValue>){
    let root = std::env::temp_dir().join(format!("oviraptor-tool-supply-root-{}", Uuid::new_v4()));
    let path = db::initialize(&root).unwrap();
    let supply = make_supply(&path).unwrap();
    let mut db = db::open(&path).unwrap();
    let approvals_before: i64 = db
        .query_row(
            "SELECT count(*) FROM tool_administrator_decisions",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(approvals_before, 1);
    db.execute(
        "INSERT INTO projects(id,name) VALUES(9001,'Original approved tool birth')",
        [],
    )
    .unwrap();
    let identities=prepare(&db,&path,target);
    let scan = if with_tools == Some(true) {
        create_sentinel_url_scan_with_tool_selection_in(
            &db,
            supply.configuration(),
            9001,
            "Original approved tool birth".into(),
            vec![target.into()],
            Some("standard".into()),
            None,
            None,
            identities.as_ref().map(|v|v.0.clone()),
            identities.as_ref().map(|v|v.1.clone()),
            None,
            None,
            None,
            Some("multi".into()),
            vec![supply.approval_id().into()],
        )
        .unwrap()
    } else {
        web_mode_test_draft(&db, Some("multi"), target).unwrap()
    };
    if let Some((ids,_))=&identities {
        let control=SaveAuthorizationControlInput{scan_id:scan.id.clone(),attempt_number:1,target_url:target.into(),contract_key:"idor|/api/orders".into(),owner_object_url:format!("{target}/api/orders?id=owner"),tester_control_url:format!("{target}/api/orders?id=tester"),object_query_key:"id".into(),owner_object_value:"owner".into(),tester_object_value:"tester".into(),response_object_pointer:"/order/id".into(),owner_identity:ids[0].clone(),tester_identity:ids[1].clone()};
        save_authorization_control_in(&mut db,&root,&control).unwrap();
    }
    for control in extra_controls {
        // The operator drafts the control against the scan being born; the
        // birth path fills the scan binding exactly like the default control.
        let bound = SaveAuthorizationControlInput{
            scan_id: scan.id.clone(),
            attempt_number: 1,
            target_url: target.into(),
            contract_key: control.contract_key.clone(),
            owner_object_url: control.owner_object_url.clone(),
            tester_control_url: control.tester_control_url.clone(),
            object_query_key: control.object_query_key.clone(),
            owner_object_value: control.owner_object_value.clone(),
            tester_object_value: control.tester_object_value.clone(),
            response_object_pointer: control.response_object_pointer.clone(),
            owner_identity: control.owner_identity.clone(),
            tester_identity: control.tester_identity.clone(),
        };
        save_authorization_control_in(&mut db,&root,&bound).unwrap();
    }
    if execution {
        // This fixture input exists before user confirmation/private birth.
        // No existing Native plan, signature, fee or capability is rewritten.
        let plan=homogeneous_execution_test_metadata(&scan.id);
        db.execute("INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash) VALUES(?1,1,'web','native',?2,?3)",params![scan.id,plan.to_string(),crate::agent_runtime::store::stable_hash(&plan.to_string())]).unwrap();
        db.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.scheduling_model','homogeneous_workers','$.workerExecution','model_broker_v1') WHERE scan_id=?1",[&scan.id]).unwrap();
    }
    let work = web_mode_test_start(&root, &mut db, &scan.id, WebStartMode::Confirm).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM tool_administrator_decisions",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        approvals_before
    );
    let web = WebModeFixture {
        root,
        path: path.clone(),
        scan: scan.id.clone(),
        work,
        target: target.into(),
        preserve: false,
    };
    let birth = supply.install_birth(&scan.id, 1, target);
    // Freeze the exact production-built plan before the actual private birth.
    // The common Root fixture's small synthetic ceilings are not compatible
    // with a subsequent real target caller's strict same-attempt comparison.
    let candidate_context = test_context(&path, target, vec![AgentIdentity::anonymous()]);
    let settings = json!({"agentBackendPolicy":"native"});
    let adaptive = AgentBudgetSettings::from_json(&settings);
    let original_plan = build_agent_execution_plan(
        &adaptive,
        &candidate_context.route,
        &candidate_context.environment,
        AgentBackendKind::Native,
        &path,
        &scan.id,
    )
    .with_attempt(1)
    .as_json();
    let request = FrozenSupplyBirthRequestGuard::install(
        &path,
        &scan.id,
        1,
        target,
        with_tools.map(|has_tools| {
            if has_tools {
                supply.tool_supply()
            } else {
                json!({"schemaVersion":1,"approvalIds":[]})
            }
        }),
        original_plan,
    );
    let f = root_tick_fixture_from_web_mode_evidence_protocol(
        "original-approved-tool-birth",
        "http://127.0.0.1:1/v1",
        true,
        (60_000, 20),
        None,
        web,
        RootFixtureEvidence::default(),
    );
    assert_eq!(birth.calls(), 1);
    let fact = birth.observed_fact();
    drop(request);
    drop(birth);
    (f, supply, fact)
}

#[cfg(unix)]
#[test]
fn frozen_tool_supply_actual_administrator_and_original_native_birth_are_bound() {
    use crate::tool_supply::frozen_plan;
    let (f, supply, captured) = frozen_tool_supply_original_birth_fixture(Some(true));
    let captured = captured.unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let original =
        frozen_plan::read_historical_on(&tx, supply.configuration(), &f.actor.root_run_id)
            .unwrap()
            .unwrap();
    assert_eq!(original.fact(), &captured);
    assert_eq!(original.fact()["scope"]["rootRunId"], f.actor.root_run_id);
    assert_eq!(original.fact()["scope"]["scanId"], f.actor.scan_id);
    assert_eq!(
        original.fact()["scope"]["attemptNumber"],
        f.actor.attempt_number
    );
    assert_eq!(
        original.fact()["tools"][0]["approvalId"],
        supply.approval_id()
    );
    assert_eq!(original.fact()["executionAllowed"], false);
    let current =
        frozen_plan::require_current_on(&tx, supply.configuration(), &f.actor.root_run_id).unwrap();
    assert_eq!(current.fact(), original.fact());
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[cfg(unix)]
#[test]
fn frozen_tool_supply_missing_root_cannot_read_original_signed_scope() {
    use crate::tool_supply::frozen_plan;
    let (f, supply, _) = frozen_tool_supply_original_birth_fixture(Some(true));
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(frozen_plan::read_historical_on(&tx, supply.configuration(), "foreign-root").is_err());
    assert!(frozen_plan::require_current_on(&tx, supply.configuration(), "foreign-root").is_err());
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[cfg(unix)]
#[test]
fn frozen_tool_supply_actual_revocation_denies_history_and_pause_denies_current_use() {
    use crate::tool_supply::frozen_plan;
    let (mut f, supply, captured) = frozen_tool_supply_original_birth_fixture(Some(true));
    drop(f.parent.take());
    // This fixture has a real born Root, but no target caller yet. Exercise the
    // production target entry with genuinely absent local evidence: it owns its
    // original invocation and returns before a paid model request. Dropping the
    // returned owner proves its actual local exit for the pause interlock.
    let absent_evidence = f.context.target_dir.join("actual-missing-evidence");
    fs::create_dir_all(&absent_evidence).unwrap();
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: f.context.route.clone(),
        target_dir: absent_evidence,
        proxy: None,
        browser: None,
    };
    let settings = json!({"agentBackendPolicy":"native"});
    let adaptive = AgentBudgetSettings::from_json(&settings);
    let mut expected_plan = f.context.execution_plan.as_json();
    expected_plan.as_object_mut().unwrap().remove("toolSupply");
    let rebuilt = build_agent_execution_plan(
        &adaptive,
        &prepared.route,
        &f.context.environment,
        AgentBackendKind::Native,
        &f.context.db_path,
        &f.context.scan_id,
    )
    .with_attempt(f.context.attempt_number)
    .as_json();
    assert_eq!(
        rebuilt, expected_plan,
        "all non-supply production fields are original before reentry"
    );
    let paid_before = db::open(&f.context.db_path)
        .unwrap()
        .query_row("SELECT count(*) FROM agent_root_model_journal", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    let owned = run_agent_target_with_tool_configuration(
        &prepared,
        AgentTargetExecution {
            db_path: &f.context.db_path,
            scan_id: &f.context.scan_id,
            attempt_number: f.context.attempt_number,
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
    drop(owned);
    assert_eq!(
        db::open(&f.context.db_path)
            .unwrap()
            .query_row("SELECT count(*) FROM agent_root_model_journal", [], |r| r
                .get::<_, i64>(
                0
            ),)
            .unwrap(),
        paid_before
    );
    let attempt = request_sentinel_pause(&f.context.db_path, &f.actor.scan_id).unwrap();
    assert_eq!(attempt, f.actor.attempt_number);
    assert!(finish_sentinel_pause(&f.context.db_path, &f.actor.scan_id, attempt).unwrap());
    let db = db::open(&f.context.db_path).unwrap();
    assert!(!db
        .query_row(
            "SELECT status IN ('queued','scanning','pausing') FROM sentinel_scans WHERE id=?1",
            [&f.actor.scan_id],
            |r| r.get::<_, bool>(0)
        )
        .unwrap());
    supply.revoke_on(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let archive = frozen_plan::read_original_seal_on(&tx, &f.actor.root_run_id)
        .unwrap().expect("revocation preserves the original sealed historical fact");
    assert_eq!(archive.historical_fact(), captured.as_ref().unwrap());
    assert_eq!(archive.historical_fact()["executionAllowed"], false);
    let error = frozen_plan::read_historical_on(&tx, supply.configuration(), &f.actor.root_run_id)
        .err()
        .expect("original revoked approval cannot verify an available tool plan");
    assert_eq!(
        error,
        "tool_supply_historical_trust_unavailable:tool_administrator_approval_revoked"
    );
    // Current use also refuses: user pause itself revokes live execution.
    // The historical error above independently proves the actual revocation.
    assert!(
        frozen_plan::require_current_on(&tx, supply.configuration(), &f.actor.root_run_id).is_err()
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[cfg(unix)]
#[test]
fn frozen_tool_supply_original_missing_request_stays_empty_without_backfill() {
    use crate::tool_supply::frozen_plan;
    let (f, supply, captured) = frozen_tool_supply_original_birth_fixture(None);
    assert!(captured.is_none());
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        frozen_plan::read_historical_on(&tx, supply.configuration(), &f.actor.root_run_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        frozen_plan::require_current_on(&tx, supply.configuration(), &f.actor.root_run_id)
            .err()
            .unwrap(),
        "tool_supply_no_frozen_tools"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_frozen_tool_supply_plans",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[cfg(unix)]
#[test]
fn frozen_tool_supply_actual_second_root_rejects_first_root_signature() {
    use crate::tool_supply::frozen_plan;
    let (first, first_supply, _) = frozen_tool_supply_original_birth_fixture(Some(true));
    let (second, second_supply, _) = frozen_tool_supply_original_birth_fixture(Some(true));
    assert_ne!(first.actor.root_run_id, second.actor.root_run_id);
    let first_db = db::open(&first.context.db_path).unwrap();
    let signature: Vec<u8> = first_db
        .query_row(
            "SELECT signature FROM agent_frozen_tool_supply_plans WHERE root_run_id=?1",
            [&first.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    // Both scopes were produced through actual independent original births.
    assert!(frozen_plan::read_historical_on(
        &first_db,
        first_supply.configuration(),
        &first.actor.root_run_id
    )
    .unwrap()
    .is_some());
    let second_db = db::open(&second.context.db_path).unwrap();
    assert!(frozen_plan::read_historical_on(
        &second_db,
        second_supply.configuration(),
        &second.actor.root_run_id
    )
    .unwrap()
    .is_some());
    let original: Vec<u8> = second_db
        .query_row(
            "SELECT signature FROM agent_frozen_tool_supply_plans WHERE root_run_id=?1",
            [&second.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let trigger: String = second_db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='frozen_tool_supply_no_update'", [], |r|r.get(0)).unwrap();
    // Disposable fixture cold corruption only; restore the exact original guard
    // before invoking either reader. No fresh signer is available here.
    second_db
        .execute_batch("DROP TRIGGER frozen_tool_supply_no_update")
        .unwrap();
    second_db
        .execute(
            "UPDATE agent_frozen_tool_supply_plans SET signature=?1 WHERE root_run_id=?2",
            params![signature, second.actor.root_run_id],
        )
        .unwrap();
    second_db.execute_batch(&trigger).unwrap();
    let before = super::tests::application_table_snapshot(&second_db);
    let tx = second_db.unchecked_transaction().unwrap();
    assert_eq!(
        frozen_plan::read_historical_on(
            &tx,
            second_supply.configuration(),
            &second.actor.root_run_id
        )
        .err()
        .unwrap(),
        "tool_supply_frozen_signature_invalid"
    );
    assert_eq!(
        frozen_plan::require_current_on(
            &tx,
            second_supply.configuration(),
            &second.actor.root_run_id
        )
        .err()
        .unwrap(),
        "tool_supply_frozen_signature_invalid"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&second_db), before);
    second_db
        .execute_batch("DROP TRIGGER frozen_tool_supply_no_update")
        .unwrap();
    second_db
        .execute(
            "UPDATE agent_frozen_tool_supply_plans SET signature=?1 WHERE root_run_id=?2",
            params![original, second.actor.root_run_id],
        )
        .unwrap();
    second_db.execute_batch(&trigger).unwrap();
    assert!(frozen_plan::read_historical_on(
        &second_db,
        second_supply.configuration(),
        &second.actor.root_run_id
    )
    .unwrap()
    .is_some());
}

#[cfg(unix)]
#[test]
fn frozen_tool_supply_original_explicit_empty_request_is_not_backfilled() {
    use crate::tool_supply::frozen_plan;
    let (f, supply, captured) = frozen_tool_supply_original_birth_fixture(Some(false));
    assert!(captured.is_none());
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        frozen_plan::read_historical_on(&tx, supply.configuration(), &f.actor.root_run_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        frozen_plan::require_current_on(&tx, supply.configuration(), &f.actor.root_run_id)
            .err()
            .unwrap(),
        "tool_supply_no_frozen_tools"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[cfg(unix)]
#[test]
fn frozen_tool_supply_original_same_attempt_selection_reentry_is_pure_and_strict() {
    let (f, _supply, _) = frozen_tool_supply_original_birth_fixture(Some(true));
    let original = f.context.execution_plan.clone();
    let mut candidate = original.clone();
    candidate.tool_supply = None;
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let inherited = native_same_attempt_tool_supply_reentry(
        &f.context.db_path,
        &f.actor.scan_id,
        f.actor.attempt_number,
        &f.context.target_url,
        &candidate,
    )
    .unwrap();
    assert_eq!(inherited, original);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let mut changed_budget = candidate.clone();
    changed_budget.hard_total_tokens += 1;
    let mut changed_identity = candidate.clone();
    changed_identity
        .identities
        .push("not-original-identity".into());
    let mut changed_attempt = candidate.clone();
    changed_attempt.attempt_number += 1;
    let mut changed_target = candidate.clone();
    changed_target.target_url = "https://foreign.example.test".into();
    for changed in [
        changed_budget,
        changed_identity,
        changed_attempt,
        changed_target,
    ] {
        assert_eq!(
            native_same_attempt_tool_supply_reentry(
                &f.context.db_path,
                &f.actor.scan_id,
                f.actor.attempt_number,
                &f.context.target_url,
                &changed,
            )
            .err()
            .unwrap(),
            "agent_attempt_plan_frozen_conflict"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    let signature: Vec<u8> = db
        .query_row(
            "SELECT signature FROM agent_root_mode_birth_seals WHERE root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let trigger: String = db.query_row(
        "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='root_mode_birth_no_update'",
        [], |r|r.get(0),
    ).unwrap();
    let mut damaged = signature.clone();
    damaged[0] ^= 1;
    // Cold source corruption in this disposable fixture only. Restore the
    // original immutable guard before the readonly historical verifier runs.
    db.execute_batch("DROP TRIGGER root_mode_birth_no_update")
        .unwrap();
    db.execute(
        "UPDATE agent_root_mode_birth_seals SET signature=?1 WHERE root_run_id=?2",
        params![damaged, f.actor.root_run_id],
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    let corrupted = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_same_attempt_tool_supply_reentry(
            &f.context.db_path,
            &f.actor.scan_id,
            f.actor.attempt_number,
            &f.context.target_url,
            &candidate,
        )
        .err()
        .unwrap(),
        "root_mode_birth_signature_invalid"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), corrupted);
    db.execute_batch("DROP TRIGGER root_mode_birth_no_update")
        .unwrap();
    db.execute(
        "UPDATE agent_root_mode_birth_seals SET signature=?1 WHERE root_run_id=?2",
        params![signature, f.actor.root_run_id],
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    assert_eq!(
        native_same_attempt_tool_supply_reentry(
            &f.context.db_path,
            &f.actor.scan_id,
            f.actor.attempt_number,
            &f.context.target_url,
            &candidate,
        )
        .unwrap(),
        original
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}
