// Production pause/confirmation/issuance/provider/closure fixtures. Cold damage
// exists only inside rollback transactions in disposable test databases.
fn financial_discovery_database_snapshot(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let tables = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    tables
        .into_iter()
        .map(|name| {
            let without_rowid: bool = db
                .query_row(
                    "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                    [&name],
                    |r| r.get(0),
                )
                .unwrap();
            let quoted = name.replace('"', "\"\"");
            let order = if without_rowid {
                let keys: Vec<String> = db
                    .prepare("SELECT name FROM pragma_table_info(?1,'main') WHERE pk>0 ORDER BY pk")
                    .unwrap()
                    .query_map([&name], |r| r.get(0))
                    .unwrap()
                    .collect::<rusqlite::Result<_>>()
                    .unwrap();
                assert!(
                    !keys.is_empty(),
                    "WITHOUT ROWID table must have original primary key"
                );
                keys.iter()
                    .map(|key| format!("\"{}\"", key.replace('"', "\"\"")))
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                "rowid".into()
            };
            let sql = format!("SELECT * FROM \"{quoted}\" ORDER BY {order}");
            let mut q = db.prepare(&sql).unwrap();
            let columns = q.column_count();
            let rows = q
                .query_map([], |r| (0..columns).map(|i| r.get(i)).collect())
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

fn financial_discovery_assert_refusal(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    code: &str,
    seen: &FinancePreviewSeen,
    calls: usize,
) {
    let before = financial_discovery_database_snapshot(db);
    let changes = db.total_changes();
    assert_eq!(
        native_financial_continuation_discovery_on(db, scan, attempt)
            .err()
            .as_deref(),
        Some(code)
    );
    assert_eq!(db.total_changes(), changes);
    assert_eq!(financial_discovery_database_snapshot(db), before);
    assert_eq!(seen.lock().unwrap().len(), calls);
}

#[test]
fn financial_continuation_discovery_closed_readonly_reopen_recovers_original_receipt_and_proposal()
{
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt, hash, issued) = financial_status_fixture("discovery-real-closed");
    let db = db::open(&f.context.db_path).unwrap();
    let root = issued["newRootRunId"].as_str().unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let birth = verify_financial_continuation_birth_on(&tx, root).unwrap();
    tx.rollback().unwrap();
    native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, root).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
    drop(db);
    let reader = rusqlite::Connection::open_with_flags(
        &f.context.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .unwrap();
    let before = financial_discovery_database_snapshot(&reader);
    let changes = reader.total_changes();
    let found = native_financial_continuation_discovery_on(&reader, &f.context.scan_id, 2).unwrap();
    assert_eq!(found.as_object().unwrap().len(), 4);
    assert_eq!(found["schema"], 1);
    assert_eq!(found["scanId"], f.context.scan_id);
    assert_eq!(found["attemptNumber"], 2);
    let recovered = &found["issued"];
    assert_eq!(recovered.as_object().unwrap().len(), 4);
    assert_eq!(recovered["historicalProposal"], true);
    assert_eq!(recovered["proposal"], birth["proposal"]);
    assert_eq!(recovered["proposal"]["proposalHash"], hash);
    assert_eq!(recovered["proposal"]["executionAllowed"], false);
    assert_eq!(recovered["originalReceipt"]["receiptId"], receipt);
    assert_eq!(recovered["originalReceipt"]["attemptNumber"], 1);
    assert_eq!(
        recovered["originalReceipt"]["rootRunId"],
        f.actor.root_run_id
    );
    assert_eq!(recovered["originalReceipt"]["actualCostConfirmed"], true);
    assert_eq!(recovered["originalReceipt"]["executionAllowed"], false);
    assert_eq!(recovered["status"]["status"], "assessment_published");
    assert_eq!(recovered["status"]["assessmentCompleted"], true);
    assert_eq!(recovered["status"]["attemptClosed"], true);
    assert_eq!(recovered["status"]["workerStartAllowed"], false);
    assert_eq!(recovered["status"]["newRootRunId"], root);
    assert_eq!(recovered["status"]["sourceReceiptId"], receipt);
    let _: FinancialContinuationDiscovery = serde_json::from_value(found.clone()).unwrap();
    let mut unknown = found.clone();
    unknown["autoApproval"] = true.into();
    assert!(serde_json::from_value::<FinancialContinuationDiscovery>(unknown).is_err());
    let mut unknown = found.clone();
    unknown["issued"]["autoApproval"] = true.into();
    assert!(serde_json::from_value::<FinancialContinuationDiscovery>(unknown).is_err());
    assert_eq!(
        native_financial_continuation_discovery_on(&reader, &f.context.scan_id, 2).unwrap(),
        found
    );
    assert_eq!(reader.total_changes(), changes);
    assert_eq!(financial_discovery_database_snapshot(&reader), before);
    assert_eq!(seen.lock().unwrap().len(), 2);
    // The original confirmation entry point is still restricted to the current
    // attempt; discovery did not relax its scope guard to recover history.
    let tx = reader.unchecked_transaction().unwrap();
    let call = recovered["originalReceipt"]["callId"].as_str().unwrap();
    assert_eq!(crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::
        read_cost_reconciliation(&tx, &f.context.scan_id, 1, &f.actor.root_run_id, call)
        .err().as_deref(), Some("root_reconciliation_attempt_changed"));
    tx.rollback().unwrap();
}

#[test]
fn financial_continuation_discovery_real_paused_unissued_attempt_returns_only_null() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, _) = financial_issuance_fixture("discovery-unissued-null");
    let reader = rusqlite::Connection::open_with_flags(
        &f.context.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .unwrap();
    let before = financial_discovery_database_snapshot(&reader);
    let changes = reader.total_changes();
    assert_eq!(
        native_financial_continuation_discovery_on(&reader, &f.context.scan_id, 1).unwrap(),
        json!({"schema":1,"scanId":f.context.scan_id,"attemptNumber":1,"issued":null})
    );
    assert_eq!(reader.total_changes(), changes);
    assert_eq!(financial_discovery_database_snapshot(&reader), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn financial_continuation_discovery_scope_deleted_and_cold_birth_receipt_columns_signatures_fail_closed(
) {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, _, _, issued) = financial_status_fixture("discovery-cold-corruption");
    let db = db::open(&f.context.db_path).unwrap();
    native_financial_continuation_work(
        &f.context.db_path,
        &f.context.scan_id,
        issued["newRootRunId"].as_str().unwrap(),
    )
    .unwrap();
    financial_discovery_assert_refusal(
        &db,
        &f.context.scan_id,
        1,
        "financial_continuation_discovery_scope_invalid",
        &seen,
        2,
    );
    financial_discovery_assert_refusal(
        &db,
        "another-scan",
        2,
        "financial_continuation_discovery_scope_invalid",
        &seen,
        2,
    );
    financial_discovery_assert_refusal(
        &db,
        &f.context.scan_id,
        3,
        "financial_continuation_discovery_scope_invalid",
        &seen,
        2,
    );
    let tx = db.unchecked_transaction().unwrap();
    tx.execute(
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",
        [&f.context.scan_id],
    )
    .unwrap();
    financial_discovery_assert_refusal(
        &tx,
        &f.context.scan_id,
        2,
        "financial_continuation_discovery_scope_invalid",
        &seen,
        2,
    );
    tx.rollback().unwrap();
    // Damaged columns remain discoverable via original JSON/Root/startup scope,
    // and may never silently turn an existing financial birth into `null`.
    for (damage, code) in [
        ("DROP TRIGGER financial_continuation_no_update; UPDATE agent_financial_continuations SET fact_hash=printf('%064d',0)", "financial_continuation_birth_invalid"),
        ("DROP TRIGGER financial_continuation_no_update; UPDATE agent_financial_continuations SET scan_id='cold-moved-scan'", "financial_continuation_birth_invalid"),
        ("DROP TRIGGER financial_continuation_no_update; UPDATE agent_financial_continuations SET new_attempt_number=99", "financial_continuation_birth_invalid"),
        ("DROP TRIGGER financial_continuation_no_update; UPDATE agent_financial_continuations SET source_root_run_id='cold-moved-source'", "financial_continuation_birth_invalid"),
        ("DROP TRIGGER financial_continuation_no_update; UPDATE agent_financial_continuations SET new_root_run_id='cold-moved-root'", "financial_continuation_birth_invalid"),
        ("DROP TRIGGER root_cost_reconciliation_no_update; UPDATE agent_root_cost_reconciliation_receipts SET invoice_hash=printf('%064d',0)", "root_reconciliation_receipt_changed"),
        ("DROP TRIGGER root_cost_reconciliation_no_delete; DELETE FROM agent_root_cost_reconciliation_receipts", "continuation_original_receipt_required"),
        ("DROP TRIGGER financial_startup_no_update; UPDATE agent_financial_startup_bindings SET binding_tag=zeroblob(32)", "financial_continuation_discovery_startup_changed"),
        ("DROP TRIGGER web_mode_receipt_immutable_update; UPDATE native_web_mode_receipts SET signature=zeroblob(32) WHERE attempt_number=2", "web_mode_private_proof_changed"),
        ("DROP TRIGGER web_mode_receipt_immutable_update; UPDATE native_web_mode_receipts SET signature=zeroblob(32) WHERE attempt_number=1", "web_mode_private_proof_changed"),
        ("DROP TRIGGER financial_continuation_no_delete; DELETE FROM agent_financial_continuations", "financial_continuation_discovery_birth_missing"),
        ("INSERT INTO agent_financial_continuations(approval_id,proposal_hash,scan_id,source_attempt_number,new_attempt_number,source_root_run_id,new_root_run_id,source_receipt_id,fact_json,fact_hash) SELECT '00000000-0000-4000-8000-000000000009',printf('%064d',0),scan_id,source_attempt_number,new_attempt_number,'cold-extra-source','cold-extra-root',source_receipt_id,fact_json,fact_hash FROM agent_financial_continuations", "financial_continuation_discovery_ambiguous"),
    ] {
        let tx = db.unchecked_transaction().unwrap();
        tx.execute_batch(damage).unwrap();
        financial_discovery_assert_refusal(&tx, &f.context.scan_id, 2, code, &seen, 2);
        // Cold simulation never erases actual paid provider costs or invents
        // an extra SDK request, even when a receipt is deliberately damaged.
        assert_eq!(tx.query_row("SELECT count(*) FROM agent_root_model_journal WHERE phase='received'", [], |r|r.get::<_,i64>(0)).unwrap(), 2);
        assert!(tx.query_row("SELECT sum(amount) FROM agent_budget_entries WHERE kind IN ('consume','reconcile') AND dimension='model_requests'", [], |r|r.get::<_,i64>(0)).unwrap() >= 2);
        tx.rollback().unwrap();
    }
    assert_eq!(
        native_financial_continuation_discovery_on(&db, &f.context.scan_id, 2).unwrap()["issued"]
            ["status"]["attemptClosed"],
        true
    );
}

#[test]
fn financial_continuation_discovery_expired_source_authority_is_history_not_worker_permission() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, _, _, _) = financial_status_fixture("discovery-expired-source");
    let db = db::open(&f.context.db_path).unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','localtime','-1 second') WHERE root_run_id=?1",
        [&f.actor.root_run_id]).unwrap();
    drop(db);
    let reader = rusqlite::Connection::open_with_flags(
        &f.context.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .unwrap();
    let before = financial_discovery_database_snapshot(&reader);
    let changes = reader.total_changes();
    let found = native_financial_continuation_discovery_on(&reader, &f.context.scan_id, 2).unwrap();
    assert_eq!(found["issued"]["status"]["status"], "review_required");
    assert_eq!(
        found["issued"]["status"]["reviewReason"],
        "authority_revalidation_required"
    );
    assert_eq!(found["issued"]["status"]["workerStartAllowed"], false);
    assert_eq!(found["issued"]["status"]["assessmentCompleted"], false);
    assert_eq!(found["issued"]["status"]["attemptClosed"], false);
    assert_eq!(reader.total_changes(), changes);
    assert_eq!(financial_discovery_database_snapshot(&reader), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
