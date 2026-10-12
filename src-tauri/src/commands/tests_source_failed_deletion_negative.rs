// Damage is introduced only in disposable production-generated originals.
#[test]
fn source_failed_deletion_original_damage_unknown_and_busy_preserve_every_row() {
    use crate::agent_runtime::deleted_scan_audit;
    let (root, db, actor) = source_failed_deletion_original_fixture(
        "analyst_exhausted",
        "first_tool_finish_received",
        7,
    );
    let path = root.join("oviraptor.sqlite3");
    for sql in [
        "UPDATE agent_runs SET terminal_reason='not original' WHERE role='coordinator'",
        "UPDATE agent_runs SET terminal_state='cancelled' WHERE role='coordinator'",
        "UPDATE agent_coordinator_leases SET target_key='source:other'",
        "UPDATE agent_assignment_attempts SET failure_class='not original' WHERE state='failed'",
        "UPDATE agent_budget_ledger SET spent_requests=spent_requests-1",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET state='uncertain' WHERE round_number=3",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET response_json='{}' WHERE round_number=3",
        "UPDATE agent_messages SET payload_json='{}' WHERE kind='evidence_summary'",
        "DROP TRIGGER source_runtime_no_update; UPDATE source_runtime_contracts SET contract_json=json_set(contract_json,'$.budget.tokenLimit',1)",
        "DROP TRIGGER analysis_view_no_update; UPDATE source_analysis_views SET manifest_digest=printf('%064d',1)",
        "DROP TRIGGER source_ci_policy_no_update; UPDATE source_ci_policies SET max_high=max_high+1",
        "DROP TRIGGER source_ci_policy_no_delete; DELETE FROM source_ci_policies",
        "UPDATE native_scan_branches SET report_json=json_set(report_json,'$.error','not original') WHERE branch='source'",
        "UPDATE native_scan_branches SET report_json=json_set(report_json,'$.independentReviewCompleted',json('true')) WHERE branch='source'",
        "INSERT INTO agent_events(run_id,sequence,event_type,payload_json) SELECT id,(SELECT coalesce(max(sequence),0)+1 FROM agent_events WHERE run_id=r.id),'terminal_reduced','{\"sourceClosureVersion\":4}' FROM agent_runs r WHERE role='coordinator'",
    ] {
        let tx=db.unchecked_transaction().unwrap();tx.execute_batch(sql).unwrap();
        assert!(tx.changes()>0,"fault must affect an actual original row: {sql}");
        let before=source_exit_snapshot(&db);
        let proof=deleted_scan_audit::prepare(&tx,&actor.scan_id);
        assert!(proof.is_err(),"{sql}");assert_eq!(source_exit_snapshot(&db),before,"{sql}");
        drop(proof);tx.rollback().unwrap();
    }
    for (kind, key) in [
        ("source-model", "source".to_string()),
        (
            "source-round-sdk",
            db.query_row(
                "SELECT child_run_id FROM agent_source_model_rounds ORDER BY rowid LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
        ),
        (
            "specialist-sdk",
            db.query_row(
                "SELECT child_run_id FROM agent_specialist_calls ORDER BY rowid LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
        ),
    ] {
        let owner = crate::agent_runtime::execution_owner::probe_native_invocation(
            &path,
            &actor.scan_id,
            1,
            kind,
            &key,
        )
        .unwrap()
        .unwrap();
        let before = source_exit_snapshot(&db);
        assert!(
            delete_sentinel_scan_inner(&path, &actor.scan_id).is_err(),
            "{kind}"
        );
        assert_eq!(source_exit_snapshot(&db), before, "{kind}");
        drop(owner);
    }
    let files = source_failed_deletion_cas_files(&root);
    delete_sentinel_scan_inner(&path, &actor.scan_id).unwrap();
    assert!(deleted_scan_audit::verify_deleted(&db, &actor.scan_id).unwrap());
    assert_eq!(
        db.query_row(
            "SELECT version FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
            [&actor.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM native_deleted_scan_audits WHERE scan_id=?1",
            [&actor.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let cold = source_exit_snapshot(&db);
    for sql in [
        "DROP TRIGGER budget_entry_no_update; UPDATE agent_budget_entries SET amount=amount+1 WHERE dimension='model_requests' AND kind='consume'",
        "DROP TRIGGER deleted_audit_v2_no_update; UPDATE native_deleted_scan_audits_v2 SET audit_json='{}'",
    ] {
        db.execute_batch("SAVEPOINT failed_cold_fault").unwrap();db.execute_batch(sql).unwrap();assert!(db.changes()>0,"{sql}");let before=source_exit_snapshot(&db);
        assert!(deleted_scan_audit::verify_deleted(&db,&actor.scan_id).is_err(),"{sql}");assert_eq!(source_exit_snapshot(&db),before);
        db.execute_batch("ROLLBACK TO failed_cold_fault; RELEASE failed_cold_fault").unwrap();
    }
    assert_eq!(source_exit_snapshot(&db), cold);
    assert!(deleted_scan_audit::verify_deleted(&db, &actor.scan_id).unwrap());
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_failed_deletion_original_protected_business_and_write_faults_preserve_assets_and_cas() {
    let (root, db, actor) =
        source_failed_deletion_original_fixture("tool_exhausted", "first_tool_received", 5);
    let path = root.join("oviraptor.sqlite3");
    let files = source_failed_deletion_cas_files(&root);
    db.execute_batch("CREATE TABLE failed_source_business(scan_id TEXT,attempt_number INTEGER,payload TEXT,
        FOREIGN KEY(scan_id,attempt_number) REFERENCES source_scope_contracts(scan_id,attempt_number) ON DELETE CASCADE)").unwrap();
    db.execute(
        "INSERT INTO failed_source_business VALUES(?1,1,'preserve')",
        [&actor.scan_id],
    )
    .unwrap();
    let before = source_exit_snapshot(&db);
    assert!(delete_sentinel_scan_inner(&path, &actor.scan_id).is_err());
    assert_eq!(source_exit_snapshot(&db), before);
    // Only this synthetic protected relationship is removed, never real data.
    db.execute_batch("DROP TABLE failed_source_business; CREATE TABLE failed_private_business(payload TEXT);INSERT INTO failed_private_business VALUES('preserve');
        INSERT INTO assets(id,asset_key) VALUES(987654322,'failed-source-preserve')").unwrap();
    for sql in [
        "CREATE TRIGGER failed_delete_fault BEFORE INSERT ON native_deleted_scan_audits_v2 BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER failed_delete_fault BEFORE INSERT ON native_deleted_scan_anchors BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER failed_delete_fault BEFORE DELETE ON sentinel_scans BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER failed_delete_fault AFTER DELETE ON sentinel_scans BEGIN UPDATE failed_private_business SET payload='changed'; END",
        "CREATE TRIGGER failed_delete_fault AFTER INSERT ON native_deleted_scan_audits_v2 BEGIN DELETE FROM assets; END",
    ] {
        db.execute_batch(sql).unwrap();let before=source_exit_snapshot(&db);assert!(delete_sentinel_scan_inner(&path,&actor.scan_id).is_err(),"{sql}");
        assert_eq!(source_exit_snapshot(&db),before,"{sql}");assert_eq!(source_failed_deletion_cas_files(&root),files);
        db.execute_batch("DROP TRIGGER failed_delete_fault").unwrap();
    }
    delete_sentinel_scan_inner(&path, &actor.scan_id).unwrap();
    assert_eq!(
        db.query_row("SELECT asset_key FROM assets WHERE id=987654322", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "failed-source-preserve"
    );
    assert_eq!(
        db.query_row("SELECT payload FROM failed_private_business", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        "preserve"
    );
    assert_eq!(source_failed_deletion_cas_files(&root), files);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
