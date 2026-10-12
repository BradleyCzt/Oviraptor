// Every fault is in the disposable original producer database.
#[test]
fn source_paid_deletion_original_material_receipt_and_parent_damage_cannot_delete() {
    let (root, db, record, report, guard, _) = source_branch_production_report(false);
    let path = root.join("oviraptor.sqlite3");
    assert!(finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap());
    drop(guard);
    let actor = report["sourceMultiAgent"]["rootRunId"].as_str().unwrap();
    for sql in [
        "UPDATE agent_runs SET terminal_reason='not original' WHERE role='coordinator'",
        "DROP TRIGGER source_runtime_no_update; UPDATE source_runtime_contracts SET contract_json=json_set(contract_json,'$.budget.tokenLimit',1)",
        "DROP TRIGGER analysis_view_no_update; UPDATE source_analysis_views SET manifest_digest=printf('%064d',1)",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET response_json='{}'",
        "UPDATE agent_messages SET payload_json='{}' WHERE kind='source_tool_result'",
        "DROP TRIGGER source_ci_policy_no_update; UPDATE source_ci_policies SET max_high=max_high+1",
        "DELETE FROM agent_events WHERE event_type='terminal_reduced' AND json_extract(payload_json,'$.sourceClosureVersion') IS NOT NULL",
    ] {
        db.execute_batch("SAVEPOINT original_fault").unwrap();db.execute_batch(sql).unwrap();
        let before=source_exit_snapshot(&db);
        // Pure Source audit under the original fault transaction; no grants.
        let c=native_source_fresh_finance::original_for_financial_exit(&db,actor);
        assert!(c.and_then(|c|crate::commands::verify_closed_source_for_deletion(&db,&c)).is_err(),"{sql}");
        assert_eq!(source_exit_snapshot(&db),before,"{sql}");
        db.execute_batch("ROLLBACK TO original_fault; RELEASE original_fault").unwrap();
    }
    for (kind, query) in [
        ("source-model", None),
        (
            "source-round-sdk",
            Some("SELECT child_run_id FROM agent_source_model_rounds ORDER BY rowid LIMIT 1"),
        ),
        (
            "specialist-sdk",
            Some("SELECT child_run_id FROM agent_specialist_calls ORDER BY rowid LIMIT 1"),
        ),
    ] {
        let key = match query {
            Some(q) => db.query_row(q, [], |r| r.get::<_, String>(0)).unwrap(),
            None => "source".into(),
        };
        let owner = crate::agent_runtime::execution_owner::probe_native_invocation(
            &path,
            &record.scan_id,
            1,
            kind,
            &key,
        )
        .unwrap()
        .unwrap();
        let before = source_exit_snapshot(&db);
        assert!(
            delete_sentinel_scan_inner(&path, &record.scan_id).is_err(),
            "{kind}"
        );
        assert_eq!(source_exit_snapshot(&db), before, "{kind}");
        drop(owner);
    }
    delete_sentinel_scan_inner(&path, &record.scan_id).unwrap();
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn source_paid_deletion_original_protected_cascade_and_write_faults_preserve_all_rows() {
    let (root, db, record, report, guard, _) = source_branch_production_report(false);
    let path = root.join("oviraptor.sqlite3");
    assert!(finish_native_source_branch(&path, &record.scan_id, 1, &report).unwrap());
    drop(guard);
    db.execute_batch("CREATE TABLE source_private_business(scan_id TEXT,attempt_number INTEGER,payload TEXT,
        FOREIGN KEY(scan_id,attempt_number) REFERENCES source_scope_contracts(scan_id,attempt_number) ON DELETE CASCADE)").unwrap();
    db.execute(
        "INSERT INTO source_private_business VALUES(?1,1,'must preserve')",
        [&record.scan_id],
    )
    .unwrap();
    let before = source_exit_snapshot(&db);
    assert!(delete_sentinel_scan_inner(&path, &record.scan_id).is_err());
    assert_eq!(source_exit_snapshot(&db), before);
    // Remove only the synthetic protected fixture, never a real asset.
    db.execute_batch("DROP TABLE source_private_business; CREATE TABLE private_business(payload TEXT);
        INSERT INTO private_business VALUES('preserve');INSERT INTO assets(id,asset_key) VALUES(987654321,'source-delete-preserve')").unwrap();
    for sql in [
        "CREATE TRIGGER source_delete_fault BEFORE INSERT ON native_deleted_scan_audits_v2 BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER source_delete_fault BEFORE INSERT ON native_deleted_scan_anchors BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER source_delete_fault BEFORE DELETE ON sentinel_scans BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER source_delete_fault AFTER DELETE ON sentinel_scans BEGIN UPDATE private_business SET payload='changed'; END",
        "CREATE TRIGGER source_delete_fault AFTER INSERT ON native_deleted_scan_audits_v2 BEGIN DELETE FROM assets; END",
    ] {
        db.execute_batch(sql).unwrap();let before=source_exit_snapshot(&db);
        assert!(delete_sentinel_scan_inner(&path,&record.scan_id).is_err(),"{sql}");assert_eq!(source_exit_snapshot(&db),before,"{sql}");
        db.execute_batch("DROP TRIGGER source_delete_fault").unwrap();
    }
    delete_sentinel_scan_inner(&path, &record.scan_id).unwrap();
    assert_eq!(
        db.query_row("SELECT asset_key FROM assets WHERE id=987654321", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "source-delete-preserve"
    );
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
