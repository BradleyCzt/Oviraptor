#[test]
fn protection_recovery_cancel_paid_source_retains_signed_history_and_revokes_unborn_authority() {
    let (f, selected, _scope, request, _approval) =
        protection_recovery_draft_fixture("recovery-cancel-retained-history");
    let db = db::open(&f.h.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db)
        .into_iter()
        .filter(|(name, _)| name != "native_protection_recovery_cancellations")
        .collect::<Vec<_>>();
    let draft = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    let scan = draft["scanId"].as_str().unwrap();
    let lookup = db.unchecked_transaction().unwrap();
    assert_eq!(
        native_protection_recovery_cancellation_by_request_on(&lookup, &request).unwrap(),
        None
    );
    lookup.commit().unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let _control = claim_scan_control(&f.h.db_path, scan).unwrap();
    cancel_draft_scan_in(&db, scan).unwrap();
    let cancelled = super::tests::application_table_snapshot(&db);
    let ro = native_protection_recovery_open(&f.h.db_path, false).unwrap();
    let snapshot = ro.unchecked_transaction().unwrap();
    assert_eq!(
        native_protection_recovery_draft_saved_on(&snapshot, &request).unwrap(),
        Some(draft.clone())
    );
    assert!(
        crate::agent_runtime::web_mode::action::read_hash(&snapshot, scan)
            .unwrap()
            .is_some()
    );
    let receipt = native_protection_recovery_cancellation_saved_on(&snapshot, scan)
        .unwrap()
        .unwrap();
    assert_eq!(receipt["authorityRevoked"], true);
    assert_eq!(receipt["executionAllowed"], false);
    assert_eq!(
        native_protection_recovery_cancellation_by_request_on(&snapshot, &request).unwrap(),
        Some(receipt.clone())
    );
    assert_eq!(
        native_protection_recovery_cancellation_by_request_on(
            &snapshot,
            &Uuid::new_v4().to_string()
        )
        .unwrap(),
        None
    );
    assert_eq!(
        native_protection_recovery_cancellation_by_request_on(&snapshot, "not-a-request")
            .unwrap_err(),
        "protection_recovery_request_invalid"
    );
    assert_eq!(
        native_protection_recovery_draft_pristine_scope_on(&snapshot, &draft).unwrap_err(),
        "protection_recovery_draft_not_pristine"
    );
    snapshot.commit().unwrap();
    drop(ro);
    assert_eq!(super::tests::application_table_snapshot(&db), cancelled);
    cancel_draft_scan_in(&db, scan).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), cancelled);
    assert_eq!(
        native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap_err(),
        "protection_recovery_draft_cancelled"
    );
    protection_recovery_draft_assert_original_rows(&before, &db);
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [scan],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "cancelled"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&selected.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    // A cold lifecycle label cannot undo a durable cancellation. This test's
    // transaction rolls the synthetic labels back; no regrant is repaired.
    let tx = db.unchecked_transaction().unwrap();
    tx.execute(
        "UPDATE sentinel_scans SET status='draft' WHERE id=?1",
        [scan],
    )
    .unwrap();
    tx.execute(
        "UPDATE sentinel_targets SET status='queued' WHERE scan_id=?1",
        [scan],
    )
    .unwrap();
    assert_eq!(
        native_protection_recovery_draft_pristine_scope_on(&tx, &draft).unwrap_err(),
        "protection_recovery_draft_not_pristine"
    );
    assert_eq!(
        native_protection_recovery_cancellation_saved_on(&tx, scan).unwrap(),
        Some(receipt)
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), cancelled);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
#[test]
fn protection_recovery_cancel_immutable_receipt_and_cold_resigning_refused() {
    let (f, _selected, _scope, request, _approval) =
        protection_recovery_draft_fixture("recovery-cancel-immutable");
    let draft = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    let scan = draft["scanId"].as_str().unwrap();
    let mut db = db::open(&f.h.db_path).unwrap();
    cancel_draft_scan_in(&db, scan).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    for sql in ["UPDATE native_protection_recovery_cancellations SET draft_hash=draft_hash","DELETE FROM native_protection_recovery_cancellations",
        "INSERT OR REPLACE INTO native_protection_recovery_cancellations(rowid,scan_id,cancellation_id,approval_id,request_id,draft_hash,cancellation_json,signature) SELECT rowid,'different-scan','different-cancel',printf('%064d',2),'different-request',draft_hash,cancellation_json,signature FROM native_protection_recovery_cancellations"]{
        db.pragma_update(None,"recursive_triggers","OFF").unwrap();
        assert!(db.execute(sql,[]).unwrap_err().to_string().contains("protection_recovery_cancellation_immutable"));
        assert_eq!(super::tests::application_table_snapshot(&db),before);
    }
    let tx = db.transaction().unwrap();
    let ddl: String = tx
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='protection_recovery_cancellation_no_update'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut receipt = native_protection_recovery_cancellation_saved_on(&tx, scan)
        .unwrap()
        .unwrap();
    tx.execute_batch("DROP TRIGGER protection_recovery_cancellation_no_update")
        .unwrap();
    receipt["cancellationId"] = json!(Uuid::new_v4().to_string());
    tx.execute("UPDATE native_protection_recovery_cancellations SET cancellation_id=?1,cancellation_json=?2 WHERE scan_id=?3",params![receipt["cancellationId"].as_str(),receipt.to_string(),scan]).unwrap();
    tx.execute_batch(&ddl).unwrap();
    let damaged = super::tests::application_table_snapshot(&tx);
    for _ in 0..2 {
        assert_eq!(
            native_protection_recovery_cancellation_saved_on(&tx, scan).unwrap_err(),
            "protection_recovery_cancellation_signature_invalid"
        );
        assert_eq!(
            native_protection_recovery_cancel_draft_in(&tx, scan).unwrap_err(),
            "protection_recovery_cancellation_signature_invalid"
        );
        assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    }
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
#[test]
fn protection_recovery_cancel_late_fault_and_trigger_collateral_roll_back_all_changes() {
    let (f, _selected, _scope, request, _approval) =
        protection_recovery_draft_fixture("recovery-cancel-fault");
    let draft = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    let scan = draft["scanId"].as_str().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    for sql in [
        "CREATE TRIGGER recovery_cancel_fixture_fault BEFORE UPDATE OF status ON sentinel_targets BEGIN SELECT RAISE(ABORT,'fixture-cancel-target-fault'); END;",
        "CREATE TRIGGER recovery_cancel_fixture_fault AFTER UPDATE OF status ON sentinel_targets BEGIN UPDATE sentinel_scans SET current_checkpoint='collateral-cancel-write' WHERE id='agent-scan'; END;",
    ]{
        db.execute_batch(sql).unwrap();let before=super::tests::application_table_snapshot(&db);
        let error=cancel_draft_scan_in(&db,scan).unwrap_err();
        assert!(error.contains("fixture-cancel-target-fault") || error.contains("not authorized"),"{error}");
        assert_eq!(super::tests::application_table_snapshot(&db),before);
        db.execute_batch("DROP TRIGGER recovery_cancel_fixture_fault").unwrap();
    }
    cancel_draft_scan_in(&db, scan).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_protection_recovery_cancellations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn protection_recovery_cancel_lost_draft_relation_cannot_reissue_the_revoked_approval() {
    let (f, _selected, _scope, request, _approval) =
        protection_recovery_draft_fixture("recovery-cancel-lost-draft-relation");
    let draft = native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap();
    let scan = draft["scanId"].as_str().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    cancel_draft_scan_in(&db, scan).unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    // Disposable corruption fixture only: lose the relation, keeping original
    // revocation and scan/targets/action/mode. Restore exact guard DDL, then
    // exercise the real API's separate connection and transaction.
    let ddl: String = db
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='protection_recovery_draft_no_delete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("DROP TRIGGER protection_recovery_draft_no_delete")
        .unwrap();
    assert_eq!(
        db.execute(
            "DELETE FROM native_protection_recovery_drafts WHERE scan_id=?1",
            [scan]
        )
        .unwrap(),
        1
    );
    db.execute_batch(&ddl).unwrap();
    let damaged = super::tests::application_table_snapshot(&db);
    for _ in 0..2 {
        assert_eq!(
            native_protection_recovery_create_draft_on(&f.h.db_path, &request).unwrap_err(),
            "protection_recovery_draft_cancelled"
        );
        assert_eq!(super::tests::application_table_snapshot(&db), damaged);
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_protection_recovery_cancellations",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
