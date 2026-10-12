#[test]
fn human_recovery_source_actual_priced_approval_is_readonly_and_exact_scope_bound() {
    let (f, selected, scope) = protection_recovery_approval_fixture("human-recovery-source");
    let pricing = priced_recovery_input();
    let mut db = db::open(&f.h.db_path).unwrap();
    let proposal =
        native_priced_recovery_preview_on(&mut db, &f.h.db_path, &selected, &scope, &pricing)
            .unwrap();
    let request = Uuid::new_v4().to_string();
    native_priced_recovery_approve_on(
        &f.h.db_path,
        &selected,
        &scope,
        &pricing,
        &request,
        proposal["proposalHash"].as_str().unwrap(),
        PRICED_RECOVERY_CONSENT,
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let http = f.h.site_seen.lock().unwrap().len();
    assert_eq!(
        native_human_recovery_source_on(&db, &f.h.db_path, &request, &selected)
            .err()
            .unwrap(),
        "root_human_recovery_snapshot_required"
    );
    let tx = db.unchecked_transaction().unwrap();
    let mut foreign = selected.clone();
    foreign.attempt_number += 1;
    assert_eq!(
        native_human_recovery_source_on(&tx, &f.h.db_path, &request, &foreign)
            .err()
            .unwrap(),
        "root_human_recovery_original_scope_conflict"
    );
    assert_eq!(
        native_human_recovery_source_on(&tx, &f.h.db_path, &Uuid::new_v4().to_string(), &selected)
            .err()
            .unwrap(),
        "root_human_recovery_original_approval_missing"
    );
    let proof = native_human_recovery_source_on(&tx, &f.h.db_path, &request, &selected).unwrap();
    assert_eq!(proof.fact()["executionAllowed"], false);
    assert_eq!(proof.fact()["freshBirthImplemented"], false);
    assert_eq!(proof.fact()["completeTenDimensionPricingVerified"], false);
    proof.require_on(&tx, &f.h.db_path).unwrap();
    drop(proof);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), http);
    let approved = {
        let tx = db.unchecked_transaction().unwrap();
        let value = native_priced_recovery_saved_on(&tx, &request)
            .unwrap()
            .unwrap();
        tx.rollback().unwrap();
        value
    };
    native_priced_recovery_cancel_on(
        &f.h.db_path,
        &request,
        approved["approvalId"].as_str().unwrap(),
        PRICED_RECOVERY_CANCEL_CONSENT,
    )
    .unwrap();
    let cancelled = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        native_human_recovery_source_on(&tx, &f.h.db_path, &request, &selected)
            .err()
            .unwrap(),
        "protection_recovery_priced_approval_revoked"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), cancelled);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), http);
}
