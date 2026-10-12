#[test]
fn assignment_attempt_supervisor_new_withdrawal_receipt_is_unique_immutable_and_readonly() {
    use crate::agent_runtime::multi_agent::{attempts, supervisor::WorkerSupervisor};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let service = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let worker = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",
            [&worker.id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    attempts::withdrawal_receipt::verify_original(&tx, &worker.id).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    drop(tx);
    drop(service);
    let restart = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    for sql in ["UPDATE agent_worker_expiry_receipts SET fact_hash=printf('%064d',0)","DELETE FROM agent_worker_expiry_receipts","INSERT OR REPLACE INTO agent_worker_expiry_receipts SELECT * FROM agent_worker_expiry_receipts"] {
        assert!(db.execute_batch(sql).is_err(),"{sql}");
        assert_eq!(super::tests::application_table_snapshot(&db),before);
    }
    drop(restart);
    db.execute_batch("INSERT OR REPLACE INTO agent_worker_expiry_receipts(rowid,receipt_id,worker_attempt_id,root_run_id,assignment_id,child_run_id,fact_json,fact_hash,created_at) SELECT rowid,'alternate-receipt','alternate-worker',root_run_id,assignment_id,child_run_id,fact_json,fact_hash,created_at FROM agent_worker_expiry_receipts").unwrap_err();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    for sql in [
        "UPDATE agent_worker_expiry_receipts SET rowid=rowid+100",
        "UPDATE agent_worker_expiry_receipts SET created_at='2000-01-01 00:00:00'",
    ] {
        let tx = db.unchecked_transaction().unwrap();
        tx.execute_batch("DROP TRIGGER worker_expiry_receipt_no_update")
            .unwrap();
        tx.execute_batch(sql).unwrap();
        tx.execute_batch(include_str!(
            "../../../agent_runtime/multi_agent/attempts/withdrawal_receipt/schema.sql"
        ))
        .unwrap();
        let damaged = super::tests::application_table_snapshot(&tx);
        assert!(
            attempts::withdrawal_receipt::verify_original(&tx, &worker.id).is_err(),
            "{sql}"
        );
        assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
        drop(tx);
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    for sql in [
        "UPDATE agent_runs SET attempt_number=attempt_number+1 WHERE id=?1",
        "UPDATE agent_runs SET role='coordinator' WHERE id=?1",
        "UPDATE agent_runs SET backend='foreign' WHERE id=?1",
        "UPDATE agent_runs SET parent_run_id=NULL WHERE id=?1",
        "UPDATE agent_runs SET lane='foreign' WHERE id=?1",
    ] {
        let tx = db.unchecked_transaction().unwrap();
        tx.execute(sql, [&child.run_id]).unwrap();
        let damaged = super::tests::application_table_snapshot(&tx);
        assert!(
            attempts::withdrawal_receipt::verify_original(&tx, &worker.id).is_err(),
            "{sql}"
        );
        assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
        drop(tx);
        assert_eq!(super::tests::application_table_snapshot(&db), before);
    }
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_receipt_ignore_rolls_back_withdrawal_and_legacy_expired_is_not_backfilled(
) {
    use crate::agent_runtime::multi_agent::{attempts, supervisor::WorkerSupervisor};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    db.execute_batch("CREATE TRIGGER expiry_receipt_ignore BEFORE INSERT ON agent_worker_expiry_receipts BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(WorkerSupervisor::start(&context.db_path, &lease).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    db.execute_batch("DROP TRIGGER expiry_receipt_ignore")
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert!(attempts::try_expire_original_in_transaction(&tx, &lease, &child).unwrap());
    tx.commit().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let service = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_worker_expiry_receipts",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let worker = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    assert_eq!(
        attempts::withdrawal_receipt::verify_original(&tx, &worker.id).unwrap_err(),
        "worker_expiry_original_receipt_missing"
    );
    drop(tx);
    drop(service);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_received_then_new_withdrawal_keeps_receipt_through_saved_delivery()
{
    use crate::agent_runtime::multi_agent::{attempts, supervisor::WorkerSupervisor};
    use std::sync::Arc;
    let (port, seen, stop) = spawn_endpoint(Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response("saved localhost mapper summary"),
        )
    }));
    let (root, mut context, lease, child) = specialist_journal_fixture();
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    context.target_dir = root.join("worker");
    fs::create_dir_all(&context.target_dir).unwrap();
    let db = db::open(&context.db_path).unwrap();
    let service = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    context.supervision = Some(service.ticket());
    let (text, usage) = multi_agent_child_round_transport(
        &context,
        &lease,
        &child,
        "readonly",
        json!({"fixed":true}),
    )
    .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    // Discardable deadline only; the running production sweep performs expiry.
    db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','+1 second') WHERE child_run_id=?1", [&child.run_id]).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        service.check().unwrap();
        if attempts::current(&db, &lease, &child.assignment_id)
            .unwrap()
            .state
            == "expired"
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    let worker = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    attempts::withdrawal_receipt::verify_original(&tx, &worker.id).unwrap();
    drop(tx);
    let payload = json!({"summary":text});
    complete_readonly_assessment(&db, &lease, &child, &usage, &payload).unwrap();
    let saved = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    attempts::withdrawal_receipt::verify_original(&tx, &worker.id).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&tx), saved);
    drop(tx);
    complete_readonly_assessment(&db, &lease, &child, &usage, &payload).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), saved);
    assert_eq!(seen.lock().unwrap().len(), 1);
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    drop(service);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
