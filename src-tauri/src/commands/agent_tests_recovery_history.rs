#[test]
fn recovery_history_actual_paid_scan_inventory_is_read_only_and_durable_scope_cannot_hide() {
    let mut f = ProtectionOriginalFixture::new("recovery-history-inventory");
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    drop(f.session.take());
    let mut db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let fact = native_recovery_preview_on(
        &mut db,
        &f.h.db_path,
        &f.actor.scan_id,
        f.actor.attempt_number,
        &f.actor.root_run_id,
        &f.receipt,
    )
    .unwrap();
    assert_eq!(fact["source"], "scan_original_settled_recovery_history");
    assert_eq!(fact["historicalRootIds"], json!([f.actor.root_run_id]));
    assert_eq!(fact["historicalRootsChecked"], 1);
    for key in ["approval", "grant", "executionGrant"] {
        assert_eq!(fact[key], false);
    }
    assert_eq!(fact["targetRequestsGranted"], 0);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    // Move only this disposable run projection. Independent immutable owners
    // still inventory its original ID; it cannot become an empty history.
    let tx = db.unchecked_transaction().unwrap();
    // Disposable empty scan projection: the cold move remains FK-valid and
    // grants no attempt, Root, budget or execution. Roll back both below.
    tx.execute(
        "INSERT INTO sentinel_scans(id,status) VALUES('damaged-runtime-scope','draft')",
        [],
    )
    .unwrap();
    assert_eq!(
        tx.execute(
            "UPDATE agent_runs SET scan_id='damaged-runtime-scope' WHERE id=?1",
            [&f.actor.root_run_id]
        )
        .unwrap(),
        1
    );
    let damaged = super::tests::application_table_snapshot(&tx);
    assert_eq!(
        native_recovery_history_roots_on(&tx, &f.actor.scan_id).unwrap(),
        vec![f.actor.root_run_id.clone()]
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn recovery_history_orphan_durable_budget_owner_blocks_ready_without_repair() {
    let mut f = ProtectionOriginalFixture::new("recovery-history-orphan");
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    drop(f.session.take());
    let db = db::open(&f.h.db_path).unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    // Deliberately orphaned durable import in a disposable fixture; this is
    // missing-owner refusal evidence, not a second actual paid Root.
    assert_eq!(tx.execute("INSERT INTO agent_root_budget_definitions(root_run_id,declaration_id,definition_json,native_plan_text_hash)
        SELECT 'orphan-root',?1,definition_json,native_plan_text_hash FROM agent_root_budget_definitions WHERE root_run_id=?2",
        params![uuid::Uuid::new_v4().to_string(),f.actor.root_run_id]).unwrap(),1);
    let roots = native_recovery_history_roots_on(&tx, &f.actor.scan_id).unwrap();
    assert!(roots.contains(&"orphan-root".to_string()));
    let before = super::tests::application_table_snapshot(&tx);
    for _ in 0..2 {
        let error = native_recovery_history_on(&tx, &f.h.db_path, &f.actor, &f.receipt)
            .err()
            .unwrap();
        assert_eq!(error, "recovery_history_original_scope_missing");
        assert_eq!(super::tests::application_table_snapshot(&tx), before);
    }
    tx.rollback().unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        native_recovery_history_on(&tx, &f.h.db_path, &f.actor, &f.receipt)
            .unwrap()
            .bounded_fact()["historicalRootsChecked"],
        1
    );
    tx.commit().unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn recovery_history_live_original_parent_cannot_prepare_a_second_orchestration() {
    let mut f = ProtectionOriginalFixture::new("recovery-history-parent");
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    let mut db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    for _ in 0..2 {
        let error = native_recovery_preview_on(
            &mut db,
            &f.h.db_path,
            &f.actor.scan_id,
            f.actor.attempt_number,
            &f.actor.root_run_id,
            &f.receipt,
        )
        .unwrap_err();
        assert!(
            error.starts_with("recovery_history_parent_not_idle:"),
            "{error}"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    drop(f.session.take());
    assert_eq!(
        native_recovery_preview_on(
            &mut db,
            &f.h.db_path,
            &f.actor.scan_id,
            f.actor.attempt_number,
            &f.actor.root_run_id,
            &f.receipt
        )
        .unwrap()["readiness"],
        "ready"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
