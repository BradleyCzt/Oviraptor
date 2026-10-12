// Actual paid v15 Mapper closure; no synthetic specialist call or closed labels.
fn mapper_loop_deleted_archive_fixture() -> (
    RootTickFixture,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) {
    use crate::agent_runtime::multi_agent::specialist::mapper_loop;
    let (mut f, child, seen) = mapper_loop_diagnostic_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let output = mapper_loop::verified_original_output_on(&tx, &f.actor, &child).unwrap();
    tx.rollback().unwrap();
    complete_readonly_assessment(
        &db,
        &f.actor,
        &child,
        output.usage(),
        &json!({"summary":output.text()}),
    )
    .unwrap();
    finish_coordinator_run(
        &db,
        &f.actor,
        &AgentTargetOutcome::incomplete(
            "Original paid Mapper assessment has completed; target execution was not dispatched",
        ),
    )
    .unwrap();
    drop(f.parent.take());
    assert_eq!(seen.lock().unwrap().len(), 2);
    (f, seen)
}

#[test]
fn mapper_loop_deleted_archive_actual_closed_sources_restore_without_single_sdk_fabrication() {
    let (f, seen) = mapper_loop_deleted_archive_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_mapper_loop_final_receipts WHERE root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_sdk_log_mapper_owners m JOIN native_sdk_log_owners o ON o.owner_id=m.owner_id WHERE o.root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let archive = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &f.actor.scan_id).expect("actual v15 sources must independently restore and verify their final HMAC, journal and diagnostic ownership");
    archive.verify_scope(&tx, &f.actor.scan_id).unwrap();
    drop(archive);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn mapper_loop_deleted_archive_cold_final_signature_cannot_authorize_task_deletion() {
    let (f, seen) = mapper_loop_deleted_archive_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    let original: Vec<u8> = db
        .query_row(
            "SELECT signature FROM agent_mapper_loop_final_receipts WHERE root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let trigger: String = db.query_row("SELECT sql FROM sqlite_schema WHERE type='trigger' AND name='mapper_loop_final_no_update'", [], |r| r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER mapper_loop_final_no_update")
        .unwrap();
    db.execute(
        "UPDATE agent_mapper_loop_final_receipts SET signature=zeroblob(32) WHERE root_run_id=?1",
        [&f.actor.root_run_id],
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    let corrupted = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::agent_runtime::deleted_scan_audit::prepare(&tx, &f.actor.scan_id).is_err(),
        "closed labels and original fee rows cannot substitute a valid final signature"
    );
    tx.rollback().unwrap();
    assert!(delete_sentinel_scan_inner(&f.context.db_path, &f.actor.scan_id).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), corrupted);
    assert_eq!(seen.lock().unwrap().len(), 2);
    db.execute_batch("DROP TRIGGER mapper_loop_final_no_update")
        .unwrap();
    db.execute(
        "UPDATE agent_mapper_loop_final_receipts SET signature=?1 WHERE root_run_id=?2",
        params![original, f.actor.root_run_id],
    )
    .unwrap();
    db.execute_batch(&trigger).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let archive = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &f.actor.scan_id).unwrap();
    archive.verify_scope(&tx, &f.actor.scan_id).unwrap();
    drop(archive);
    tx.rollback().unwrap();
}
