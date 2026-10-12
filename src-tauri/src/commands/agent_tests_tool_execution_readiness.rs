// Real administrator signatures, original startup and Native Root birth. These
// negative admissions do not execute a package or synthesize a runner receipt.
fn homogeneous_execution_test_metadata(scan:&str)->JsonValue {
    json!({"schemaVersion":1,"scanId":scan,"attemptNumber":1,"scheduling_model":"homogeneous_workers",
        "workerExecution":"model_broker_v1","allowedParallel":3,"lane":"read_only_analysis",
        "targetParallel":1,"reviewParallel":1,"credentialVisibility":"plaintext","goal":"在授权材料内解析、验证并独立审核"})
}

#[cfg(all(target_os="macos",target_arch="aarch64"))]
#[test]
fn homogeneous_execution_original_binding_is_sealed_and_late_ui_plan_cannot_enable_it() {
    use crate::agent_runtime::multi_agent::homogeneous::execution_policy;
    use crate::tool_supply::{container_parser,frozen_plan::tests::TestSupplyFixture};
    let runtime=container_parser::Runtime::inspect().unwrap();
    let (f,_supply,_)=frozen_tool_supply_original_birth_with_mode(Some(true),|path|
        TestSupplyFixture::approve_container_on(path,runtime.digest()),true);
    let db=db::open(&f.context.db_path).unwrap();let before=super::tests::application_table_snapshot(&db);
    let original=execution_policy::require_on(&db,&f.actor).unwrap();
    assert_eq!(AgentExecutionPlan::from_json(&f.context.execution_plan.as_json()).unwrap(),f.context.execution_plan);
    assert_eq!(super::tests::application_table_snapshot(&db),before);
    let mut changed=homogeneous_execution_test_metadata(&f.actor.scan_id);changed["goal"]=json!("changed after birth");
    db.execute("UPDATE native_scan_plans SET plan_json=?1,plan_hash=?2 WHERE scan_id=?3 AND attempt_number=1",params![changed.to_string(),crate::agent_runtime::store::stable_hash(&changed.to_string()),f.actor.scan_id]).unwrap();
    let damaged=super::tests::application_table_snapshot(&db);
    assert_eq!(execution_policy::require_on(&db,&f.actor).unwrap_err(),"homogeneous_execution_original_binding_changed");
    assert_eq!(super::tests::application_table_snapshot(&db),damaged);
    assert_eq!(original.as_value()["targetParallel"],1);assert_eq!(original.as_value()["reviewParallel"],1);

    let (f,_supply,_)=frozen_tool_supply_original_birth_fixture(Some(true));
    let db=db::open(&f.context.db_path).unwrap();let plan=homogeneous_execution_test_metadata(&f.actor.scan_id);
    db.execute("INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash) VALUES(?1,1,'web','native',?2,?3)",params![f.actor.scan_id,plan.to_string(),crate::agent_runtime::store::stable_hash(&plan.to_string())]).unwrap();
    let before=super::tests::application_table_snapshot(&db);
    assert_eq!(execution_policy::require_on(&db,&f.actor).unwrap_err(),"homogeneous_execution_not_frozen_at_birth");
    assert_eq!(super::tests::application_table_snapshot(&db),before);
}

#[cfg(all(target_os="macos",target_arch="aarch64"))]
#[test]
fn tool_execution_container_original_signed_birth_binds_actual_runtime_without_granting_worker() {
    use crate::tool_supply::{container_parser, execution_readiness, frozen_plan::tests::TestSupplyFixture, approved_bundle};
    let runtime=container_parser::Runtime::inspect().unwrap();
    let (f,supply,captured)=frozen_tool_supply_original_birth_with(Some(true),|path|
        TestSupplyFixture::approve_container_on(path,runtime.digest()));
    let db=db::open(&f.context.db_path).unwrap();
    let before=super::tests::application_table_snapshot(&db);
    let tx=db.unchecked_transaction().unwrap();
    let ready=execution_readiness::require_current_on(&tx,supply.configuration(),&f.actor.root_run_id,supply.approval_id()).unwrap();
    assert_eq!(ready.original_plan().fact(),&captured.unwrap());
    assert_eq!(ready.original_plan().fact()["executionAllowed"],false);
    assert_eq!(ready.original_approval().runtime_adapter(),container_parser::ADAPTER);
    assert_eq!(ready.container().unwrap().digest(),runtime.digest());
    let bundle=approved_bundle::open_on(&tx,supply.configuration(),supply.approval_id()).unwrap();
    assert_eq!(bundle.approval_id(),supply.approval_id());
    assert_eq!(bundle.metadata().entries()[0].path(),"bin/parser");
    assert_eq!(super::tests::application_table_snapshot(&tx),before);
    tx.commit().unwrap();assert_eq!(super::tests::application_table_snapshot(&db),before);
}

#[cfg(all(target_os="macos",target_arch="aarch64"))]
#[test]
fn tool_execution_container_signed_wrong_runtime_is_refused_without_writes_or_execution() {
    use crate::tool_supply::{execution_readiness, frozen_plan::tests::TestSupplyFixture};
    let digest=format!("sha256:{}","0".repeat(64));
    let (f,supply,_)=frozen_tool_supply_original_birth_with(Some(true),|path|
        TestSupplyFixture::approve_container_on(path,&digest));
    let db=db::open(&f.context.db_path).unwrap();let before=super::tests::application_table_snapshot(&db);
    let tx=db.unchecked_transaction().unwrap();
    let error=execution_readiness::require_current_on(&tx,supply.configuration(),&f.actor.root_run_id,supply.approval_id()).err().unwrap();
    assert_eq!(error,"tool_execution_runtime_digest_mismatch");
    tx.commit().unwrap();assert_eq!(super::tests::application_table_snapshot(&db),before);
}

#[cfg(unix)]
#[test]
fn tool_execution_readiness_original_birth_unknown_approval_has_zero_writes() {
    use crate::tool_supply::execution_readiness;
    let (f, supply, captured) = frozen_tool_supply_original_birth_fixture(Some(true));
    assert!(captured.is_some());
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let unknown = Uuid::new_v4().to_string();
    assert_ne!(unknown, supply.approval_id());
    let error = execution_readiness::require_current_on(
        &tx,
        supply.configuration(),
        &f.actor.root_run_id,
        &unknown,
    )
    .err()
    .expect("an approval absent from this original plan cannot be admitted");
    assert_eq!(error, "tool_execution_approval_not_frozen");
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    // Even a caller committing an unsuccessful admission cannot save anything.
    tx.commit().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn tool_execution_readiness_original_valid_supply_reaches_actual_unsupported_memory() {
    use crate::tool_supply::{execution_readiness, frozen_plan};
    let (f, supply, captured) = frozen_tool_supply_original_birth_fixture(Some(true));
    let captured = captured.unwrap();
    let item = &captured["tools"][0];
    // Establish every preceding fixed-parser policy from the actual signed
    // approval; a platform/runtime mismatch must not masquerade as OS evidence.
    assert_eq!(item["approvalId"], supply.approval_id());
    assert_eq!(item["runtimeAdapter"], "offline-parser-v1");
    assert_eq!(item["manifest"]["platform"], "macos-aarch64");
    assert_eq!(item["manifest"]["networkPolicy"], "none");
    assert_eq!(
        item["manifest"]["filesystemPolicy"],
        "read-only-input-and-private-temp-output"
    );
    assert_eq!(item["manifest"]["processLimit"], 1);
    assert_eq!(item["manifest"]["cpuLimit"], 1);
    assert_eq!(item["manifest"]["inputSchemaVersion"], 1);
    assert_eq!(item["manifest"]["outputSchemaVersion"], 1);
    for field in ["memoryMib", "timeoutSeconds", "outputBytes"] {
        assert!(item["manifest"][field].as_u64().unwrap() > 0);
    }
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let current =
        frozen_plan::require_current_on(&tx, supply.configuration(), &f.actor.root_run_id).unwrap();
    assert_eq!(current.fact(), &captured);
    let error = execution_readiness::require_current_on(
        &tx,
        supply.configuration(),
        &f.actor.root_run_id,
        supply.approval_id(),
    )
    .err()
    .expect("signed supply is not hard-memory-contained execution");
    assert_eq!(
        error,
        "unsupported_sandbox:macos_hard_memory_containment_unverified"
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    tx.commit().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[cfg(unix)]
#[test]
fn tool_execution_readiness_original_absent_and_explicit_empty_supply_has_no_capability() {
    use crate::tool_supply::{execution_readiness, frozen_plan};
    for selection in [None, Some(false)] {
        let (f, supply, captured) = frozen_tool_supply_original_birth_fixture(selection);
        assert!(captured.is_none());
        let db = db::open(&f.context.db_path).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        assert!(
            frozen_plan::read_historical_on(&tx, supply.configuration(), &f.actor.root_run_id)
                .unwrap()
                .is_none()
        );
        let error = execution_readiness::require_current_on(
            &tx,
            supply.configuration(),
            &f.actor.root_run_id,
            supply.approval_id(),
        )
        .err()
        .expect("no original supply cannot advertise execution capability");
        assert_eq!(error, "tool_supply_no_frozen_tools");
        assert_eq!(super::tests::application_table_snapshot(&tx), before);
        tx.commit().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
}
