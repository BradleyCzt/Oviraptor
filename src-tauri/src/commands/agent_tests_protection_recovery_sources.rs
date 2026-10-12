#[test]
fn protection_recovery_sources_original_paid_receipt_is_readonly_and_not_readiness() {
    let (f, selected, _) = protection_recovery_approval_fixture("recovery-source-discovery");
    let db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let ro = native_protection_recovery_open(&f.h.db_path, false).unwrap();
    assert_eq!(
        native_protection_recovery_sources_on(&ro, &selected.scan_id).unwrap_err(),
        "protection_recovery_snapshot_required"
    );
    let snapshot = ro.unchecked_transaction().unwrap();
    let report = native_protection_recovery_sources_on(&snapshot, &selected.scan_id).unwrap();
    assert_eq!(report["items"].as_array().unwrap().len(), 1);
    assert_eq!(report["items"][0]["selection"], json!(selected));
    assert_eq!(report["items"][0]["sourceVerified"], true);
    assert_eq!(report["items"][0]["targetUrl"], f.h.context.target_url);
    assert_eq!(report["readinessAssessed"], false);
    assert_eq!(report["approval"], false);
    assert_eq!(report["executionAllowed"], false);
    assert_eq!(report["targetRequestsGranted"], 0);
    assert_eq!(
        native_protection_recovery_sources_on(&snapshot, "absent-scan").unwrap_err(),
        "protection_recovery_sources_scan_missing"
    );
    snapshot.commit().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    let shadow = db.unchecked_transaction().unwrap();
    shadow
        .execute_batch("CREATE TEMP TABLE agent_protection_response_receipts(dummy TEXT)")
        .unwrap();
    assert_eq!(
        native_protection_recovery_sources_on(&shadow, &selected.scan_id).unwrap_err(),
        "protection_recovery_sources_schema_shadowed"
    );
    shadow.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let cold = db.unchecked_transaction().unwrap();
    cold.execute_batch("DROP TRIGGER protection_response_receipt_no_update")
        .unwrap();
    cold.execute("UPDATE main.agent_protection_response_receipts SET signature=zeroblob(32) WHERE receipt_id=?1", [&selected.protection_receipt_id]).unwrap();
    cold.execute_batch(include_str!(
        "../agent_runtime/web_mode/protection_receipt.sql"
    ))
    .unwrap();
    let damaged = super::tests::application_table_snapshot(&cold);
    for _ in 0..2 {
        assert!(native_protection_recovery_sources_on(&cold, &selected.scan_id).is_err());
    }
    assert_eq!(super::tests::application_table_snapshot(&cold), damaged);
    cold.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}
