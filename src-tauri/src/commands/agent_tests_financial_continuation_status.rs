// Real original paid call, pause, confirmation and issuance fixtures. Status
// observers cannot create fees, completion, workers or collateral rows.
fn financial_status_fixture(
    tag: &str,
) -> (
    RootTickFixture,
    FinancePreviewSeen,
    String,
    String,
    JsonValue,
) {
    let (f, seen, receipt) = financial_issuance_fixture(tag);
    let db = db::open(&f.context.db_path).unwrap();
    let proposal = finance_preview_read(&f, &db, &receipt).unwrap();
    let hash = proposal["proposalHash"].as_str().unwrap().to_string();
    let issued = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        &hash,
    )
    .unwrap();
    (f, seen, receipt, hash, issued)
}
fn financial_status_read(
    f: &RootTickFixture,
    db: &rusqlite::Connection,
    receipt: &str,
    hash: &str,
) -> Result<JsonValue, String> {
    native_financial_continuation_status_on(
        db,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        receipt,
        hash,
    )
}

#[test]
fn financial_continuation_status_exact_scope_issued_and_real_closed_are_readonly() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt, hash, issued) = financial_status_fixture("status-real-closed");
    let db = db::open(&f.context.db_path).unwrap();
    let before = receipt_database_snapshot(&db);
    let writes = db.total_changes();
    let read = financial_status_read(&f, &db, &receipt, &hash).unwrap();
    assert_eq!(read, issued);
    assert_eq!(read["status"], "issued");
    assert_eq!(read["assessmentCompleted"], false);
    assert_eq!(read["attemptClosed"], false);
    assert_eq!(read["workerStartAllowed"], true);
    assert_eq!(read["reviewReason"], JsonValue::Null);
    assert_eq!(read["automaticResumeAllowed"], false);
    assert_eq!(read["originalSemanticReplayAllowed"], false);
    assert_eq!(read.as_object().unwrap().len(), 19);
    let _: FinancialContinuationStatus = serde_json::from_value(read.clone()).unwrap();
    let mut bad = read.clone();
    bad["inventedCapability"] = true.into();
    assert!(serde_json::from_value::<FinancialContinuationStatus>(bad).is_err());
    for (scan, attempt, source, bill, proposal) in [
        (
            "other-scan",
            1,
            f.actor.root_run_id.as_str(),
            receipt.as_str(),
            hash.as_str(),
        ),
        (
            f.context.scan_id.as_str(),
            2,
            f.actor.root_run_id.as_str(),
            receipt.as_str(),
            hash.as_str(),
        ),
        (
            f.context.scan_id.as_str(),
            1,
            f.actor.root_run_id.as_str(),
            "wrong-receipt",
            hash.as_str(),
        ),
        (
            f.context.scan_id.as_str(),
            1,
            f.actor.root_run_id.as_str(),
            receipt.as_str(),
            "wrong-proposal",
        ),
    ] {
        assert_eq!(
            native_financial_continuation_status_on(&db, scan, attempt, source, bill, proposal)
                .err()
                .as_deref(),
            Some("financial_continuation_status_binding_conflict")
        );
    }
    assert_eq!(
        native_financial_continuation_status_on(
            &db,
            &f.context.scan_id,
            1,
            "wrong-root",
            &receipt,
            &hash
        )
        .err()
        .as_deref(),
        Some("financial_continuation_issuance_missing")
    );
    assert_eq!(db.total_changes(), writes);
    assert_eq!(receipt_database_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let new_root = issued["newRootRunId"].as_str().unwrap();
    native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, new_root).unwrap();
    let before = receipt_database_snapshot(&db);
    let writes = db.total_changes();
    let closed = financial_status_read(&f, &db, &receipt, &hash).unwrap();
    assert_eq!(closed["status"], "assessment_published");
    assert_eq!(closed["assessmentCompleted"], true);
    assert_eq!(closed["attemptClosed"], true);
    assert_eq!(closed["workerStartAllowed"], false);
    assert_eq!(db.total_changes(), writes);
    assert_eq!(receipt_database_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn financial_continuation_status_failure_review_never_requests_worker_restart() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt, hash, issued) = financial_status_fixture("status-review");
    let db = db::open(&f.context.db_path).unwrap();
    native_financial_continuation_stop_attempt(
        &db,
        &f.context.scan_id,
        issued["newRootRunId"].as_str().unwrap(),
    )
    .unwrap();
    let before = receipt_database_snapshot(&db);
    let writes = db.total_changes();
    let read = financial_status_read(&f, &db, &receipt, &hash).unwrap();
    assert_eq!(read["status"], "review_required");
    assert_eq!(read["assessmentCompleted"], false);
    assert_eq!(read["attemptClosed"], false);
    assert_eq!(read["workerStartAllowed"], false);
    assert_eq!(read["reviewReason"], "financial_assessment_requires_review");
    let retry = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        &hash,
    )
    .unwrap();
    assert_eq!(retry, read);
    assert_eq!(db.total_changes(), writes);
    assert_eq!(receipt_database_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn financial_continuation_status_cold_completion_text_cannot_replace_original_proofs() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt, hash, issued) = financial_status_fixture("status-cold-completion");
    let db = db::open(&f.context.db_path).unwrap();
    let root = issued["newRootRunId"].as_str().unwrap();
    for damage in [
        "UPDATE sentinel_scan_attempts SET status='partial',stop_reason='financial_continuation_assessment_only',checkpoint='financial_assessment_published',finished_at=datetime('now','localtime') WHERE attempt_number=2",
        "UPDATE agent_runs SET status='terminal',terminal_code='financial_continuation_assessment_only',finished_at=datetime('now','localtime') WHERE attempt_number=2",
    ] {
        let tx = db.unchecked_transaction().unwrap();tx.execute_batch(damage).unwrap();
        let before = receipt_database_snapshot(&tx);let writes = tx.total_changes();
        assert_eq!(financial_status_read(&f,&tx,&receipt,&hash).err().as_deref(),
            Some("financial_continuation_status_closure_unproven"));
        assert_eq!(tx.total_changes(),writes);assert_eq!(receipt_database_snapshot(&tx),before);
        assert_eq!(tx.query_row("SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1",[root],|r|r.get::<_,i64>(0)).unwrap(),0);
        tx.rollback().unwrap();
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn financial_continuation_status_published_before_failed_finish_is_review_without_fake_closure() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt, hash, issued) = financial_status_fixture("status-published-review");
    let db = db::open(&f.context.db_path).unwrap();
    let root = issued["newRootRunId"].as_str().unwrap();
    db.execute_batch(
        "CREATE TRIGGER status_finish_fault BEFORE INSERT ON agent_multi_exit_receipts
        BEGIN SELECT RAISE(ABORT,'status_finish_fault'); END;",
    )
    .unwrap();
    assert!(
        native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, root).is_err()
    );
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='publication'",[root],|r|r.get::<_,i64>(0)).unwrap(),1);
    let before = receipt_database_snapshot(&db);
    let writes = db.total_changes();
    let read = financial_status_read(&f, &db, &receipt, &hash).unwrap();
    assert_eq!(read["status"], "review_required");
    assert_eq!(read["assessmentCompleted"], true);
    assert_eq!(read["attemptClosed"], false);
    assert_eq!(read["workerStartAllowed"], false);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.total_changes(), writes);
    assert_eq!(receipt_database_snapshot(&db), before);
    db.execute_batch("DROP TRIGGER status_finish_fault")
        .unwrap();
    let retry = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        &hash,
    )
    .unwrap();
    assert_eq!(retry["workerStartAllowed"], false);
    assert_eq!(retry["attemptClosed"], false);
    assert_eq!(seen.lock().unwrap().len(), 2);
}

#[test]
fn financial_continuation_status_expired_original_authority_is_readable_but_never_worker_grant() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt, hash, _) = financial_status_fixture("status-expired-authority");
    let db = db::open(&f.context.db_path).unwrap();
    // Expire only mutable original live C; its historical financial identity,
    // invoice and confirmation receipt remain exactly as issued.
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','localtime','-1 second') WHERE root_run_id=?1",
        [&f.actor.root_run_id]).unwrap();
    let reader = rusqlite::Connection::open_with_flags(
        &f.context.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .unwrap();
    let before = receipt_database_snapshot(&reader);
    let writes = reader.total_changes();
    let read = financial_status_read(&f, &reader, &receipt, &hash).unwrap();
    assert_eq!(read["status"], "review_required");
    assert_eq!(read["reviewReason"], "authority_revalidation_required");
    assert_eq!(read["workerStartAllowed"], false);
    assert_eq!(read["assessmentCompleted"], false);
    assert_eq!(read["attemptClosed"], false);
    assert_eq!(reader.total_changes(), writes);
    assert_eq!(receipt_database_snapshot(&reader), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
