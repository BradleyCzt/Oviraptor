// Actual original Root bills and private exit inode, never a fabricated grant.
fn recovery_source_guard_error_without_writes(
    f: &ProtectionOriginalFixture,
    snapshot: &rusqlite::Connection,
) -> String {
    let before = super::tests::application_table_snapshot(snapshot);
    let changes = snapshot.total_changes();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let gets = f.h.site_seen.lock().unwrap().len();
    let error = native_recovery_source_at_on(snapshot, &f.h.db_path, &f.actor, &f.receipt)
        .err()
        .expect("original unavailable/corrupted source must reject readiness");
    assert_eq!(super::tests::application_table_snapshot(snapshot), before);
    assert_eq!(snapshot.total_changes(), changes);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), gets);
    error
}

#[test]
fn recovery_source_original_root_sdk_guard_blocks_without_recreating_or_charging() {
    let mut f = ProtectionOriginalFixture::new("recovery-source-sdk-guard");
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    drop(f.session.take());
    let db = db::open(&f.h.db_path).unwrap();
    let fact = recovery_source_assert_read_only(&f, &db, true).unwrap();
    let original = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let guard = crate::agent_runtime::execution_owner::probe_native_invocation(
        &f.h.db_path,
        &f.actor.scan_id,
        f.actor.attempt_number,
        crate::agent_runtime::multi_agent::budget::root::model::tick::INVOCATION_KIND,
        &f.actor.root_run_id,
    )
    .unwrap()
    .expect("real Root SDK inode already exists");
    for _ in 0..2 {
        let snapshot = db.unchecked_transaction().unwrap();
        let error = recovery_source_guard_error_without_writes(&f, &snapshot);
        assert!(
            error.starts_with("root_decision_transport_not_idle:"),
            "{error}"
        );
        snapshot.commit().unwrap();
    }
    drop(guard);
    assert_eq!(
        recovery_source_assert_read_only(&f, &db, true).unwrap(),
        fact
    );
    assert_eq!(super::tests::application_table_snapshot(&db), original);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn recovery_source_original_root_paid_journal_terminal_and_cost_cold_drift_are_not_repaired() {
    let mut f = ProtectionOriginalFixture::new("recovery-source-root-physical");
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &f.outcome).unwrap();
    drop(f.session.take());
    let db = db::open(&f.h.db_path).unwrap();
    let fact = recovery_source_assert_read_only(&f, &db, true).unwrap();
    let original = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let calls: Vec<String> = db
        .prepare(
            "SELECT call_id FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'
         AND json_extract(fact_json,'$.request.basis.phase')='protection-changed'",
        )
        .unwrap()
        .query_map([&f.actor.root_run_id], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(calls.len(), 1, "one actual paid protection assessment call");
    let call = &calls[0];
    let terminal_fee: String = db
        .query_row(
            "SELECT entry_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=''
         AND source_id=?2 AND dimension='model_requests' AND kind='consume'",
            params![f.actor.root_run_id, call],
            |r| r.get(0),
        )
        .unwrap();
    let terminal_count: i64 = db.query_row(
        "SELECT count(*) FROM agent_root_tick_terminal_proofs WHERE root_run_id=?1 AND call_id=?2 AND phase='received'",
        params![f.actor.root_run_id, call], |r|r.get(0)).unwrap();
    assert_eq!(
        terminal_count, 1,
        "original known received bill has an immutable terminal proof"
    );
    for (label, table, sql, key, expected) in [
        ("received journal physical time", "agent_root_model_journal",
         "UPDATE agent_root_model_journal SET created_at='2000-01-01 00:00:00' WHERE call_id=?1 AND phase='received'",
         call.as_str(), "root_tick_terminal_proof_changed"),
        ("terminal canonical bytes", "agent_root_tick_terminal_proofs",
         "UPDATE agent_root_tick_terminal_proofs SET fact_json=fact_json||' ' WHERE call_id=?1 AND phase='received'",
         call.as_str(), "root_tick_terminal_proof_invalid"),
        ("original consumed request row", "agent_budget_entries",
         "UPDATE agent_budget_entries SET created_at='2000-01-01 00:00:00' WHERE entry_id=?1",
         terminal_fee.as_str(), "root_tick_terminal_proof_changed"),
    ] {
        // Disposable cold-storage damage only. Restore exact original guards
        // before the read, so missing DDL is never the negative evidence.
        let tx = db.unchecked_transaction().unwrap();
        let triggers = protection_physical_triggers(&tx, table);
        assert!(!triggers.is_empty(), "{label}: original immutable guards exist");
        for (name, _) in &triggers {
            tx.execute_batch(&format!("DROP TRIGGER \"{}\"", name.replace('"', "\"\""))).unwrap();
        }
        assert_eq!(tx.execute(sql, [key]).unwrap(), 1, "{label}: damage one original row");
        for (_, ddl) in &triggers { tx.execute_batch(ddl).unwrap(); }
        assert_eq!(protection_physical_triggers(&tx, table), triggers);
        for _ in 0..2 {
            assert_eq!(recovery_source_guard_error_without_writes(&f, &tx), expected, "{label}");
        }
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), original);
        assert_eq!(recovery_source_assert_read_only(&f, &db, true).unwrap(), fact);
    }
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}
