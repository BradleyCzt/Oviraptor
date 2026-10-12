// Actual paid normal closure with the pre-additive schema shape. This is not
// evidence of an expired worker: no clock or expiry state is manufactured.
#[test]
fn mapper_loop_legacy_archive_absent_empty_expired_table_keeps_original_normal_sources() {
    let (f, seen) = mapper_loop_deleted_archive_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_mapper_loop_expired_receipts",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    // Only remove an empty additive table in this disposable fixture. All paid
    // journals, HMACs, ACKs, original birth and ledger rows remain untouched.
    db.execute_batch("DROP TABLE agent_mapper_loop_expired_receipts")
        .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let archive = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &f.actor.scan_id)
        .expect("an original normal v15 archive must not require a later empty expiry source");
    archive.verify_scope(&tx, &f.actor.scan_id).unwrap();
    drop(archive);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(db.query_row("SELECT count(*) FROM main.sqlite_master WHERE name='agent_mapper_loop_expired_receipts'", [], |r|r.get::<_,i64>(0)).unwrap(),0,"pure archive verification must not backfill the missing schema");
    assert_eq!(seen.lock().unwrap().len(), 2);
}
