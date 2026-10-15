#[cfg(unix)]
#[test]
fn homogeneous_execution_original_owner_rejects_rehashed_source_replacement() {
    use crate::agent_runtime::multi_agent::homogeneous::{execution_policy, material_policy};
    use crate::tool_supply::frozen_plan::tests::TestSupplyFixture;
    let (source_root, source_db, record, _) = super::tests::source_plan_owner_frozen_fixture();
    let mut source = crate::native_pipeline::NativeSourcePlan::load(&source_db, &record.scan_id, 1)
        .unwrap()
        .unwrap();
    let (web, _supply, _) = frozen_tool_supply_original_birth_with_mode(
        Some(false),
        TestSupplyFixture::approve_on,
        true,
    );
    let db = db::open(&web.context.db_path).unwrap();
    assert!(
        execution_policy::read_original_on(&db, &web.actor.root_run_id)
            .unwrap()
            .is_some()
    );
    assert!(material_policy::for_root(&db, &web.actor.root_run_id).unwrap());
    source.scan_id = web.actor.scan_id.clone();
    // Even a complete Source-shaped replacement with a correct Source hash
    // and matching scope receipt cannot erase the original Web birth binding.
    db.execute(
        "UPDATE sentinel_scans SET scan_type=?1,source_path=?2 WHERE id=?3",
        params![source.scan_type, record.source_path, web.actor.scan_id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO source_scope_contracts(scan_id,attempt_number,source_path,canonical_root,
        scan_type,scope_mode,diff_base,analysis_policy_version) VALUES(?1,1,?2,?2,?3,'full','',1)",
        params![web.actor.scan_id, record.source_path, source.scan_type],
    )
    .unwrap();
    db.execute(
        "UPDATE native_scan_plans SET scan_type=?1,backend='native',plan_json=?2,plan_hash=?3
        WHERE scan_id=?4 AND attempt_number=1",
        params![
            source.scan_type,
            source.as_json().to_string(),
            source.hash(),
            web.actor.scan_id
        ],
    )
    .unwrap();
    let before = db.total_changes();
    assert!(
        crate::native_pipeline::NativeSourcePlan::verifies_source_owner(
            &db,
            &web.actor.scan_id,
            1,
            &source.as_json(),
            &source.hash()
        )
        .unwrap()
    );
    assert!(
        execution_policy::Binding::capture(&db, &web.actor.scan_id, 1)
            .unwrap()
            .is_none()
    );
    assert!(
        material_policy::for_root(&db, &web.actor.root_run_id).is_err(),
        "current Source ownership cannot remove the original homogeneous execution binding"
    );
    assert_eq!(
        db.total_changes(),
        before,
        "rejected protocol substitution must not write"
    );
    db.execute(
        "UPDATE agent_runs SET plan_json=?1 WHERE id=?2",
        params![source.as_json().to_string(), web.actor.root_run_id],
    )
    .unwrap();
    let before = db.total_changes();
    assert!(
        execution_policy::read_original_on(&db, &web.actor.root_run_id)
            .unwrap()
            .is_none(),
        "the mutable discriminator alone cannot prove the original owner"
    );
    assert!(
        material_policy::for_root(&db, &web.actor.root_run_id).is_err(),
        "the immutable Web mode receipt must reject replacing both current and Root plan JSON"
    );
    assert_eq!(db.total_changes(), before);
    drop(db);
    drop(source_db);
    fs::remove_dir_all(source_root).unwrap();
}

#[cfg(unix)]
#[test]
fn homogeneous_readonly_owner_rejects_source_replacement_without_scope_receipt() {
    use crate::agent_runtime::multi_agent::homogeneous::{
        execution_policy, material_policy, model_worker,
    };
    let (source_root, source_db, record, _) = super::tests::source_plan_owner_frozen_fixture();
    let mut source = crate::native_pipeline::NativeSourcePlan::load(&source_db, &record.scan_id, 1)
        .unwrap()
        .unwrap();
    let (web, _supply, _) = frozen_tool_supply_original_birth_fixture(Some(false));
    let db = db::open(&web.context.db_path).unwrap();
    let mut readonly = homogeneous_execution_test_metadata(&web.actor.scan_id);
    readonly["workerExecution"] = json!("model_read_only_v1");
    db.execute("INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash)
        VALUES(?1,1,'web','native',?2,?3)",
        params![web.actor.scan_id,readonly.to_string(),crate::agent_runtime::store::stable_hash(&readonly.to_string())]).unwrap();
    assert!(material_policy::for_root(&db, &web.actor.root_run_id).unwrap());
    source.scan_id = web.actor.scan_id.clone();
    db.execute("UPDATE native_scan_plans SET scan_type=?1,plan_json=?2,plan_hash=?3 WHERE scan_id=?4 AND attempt_number=1",
        params![source.scan_type,source.as_json().to_string(),source.hash(),web.actor.scan_id]).unwrap();
    let before = db.total_changes();
    assert!(execution_policy::Binding::capture(&db, &web.actor.scan_id, 1).is_err());
    assert!(model_worker::enabled(&db, &web.actor.scan_id, 1).is_err());
    assert!(material_policy::for_root(&db, &web.actor.root_run_id).is_err());
    assert_eq!(db.total_changes(), before);
    drop(db);
    drop(source_db);
    fs::remove_dir_all(source_root).unwrap();
}
