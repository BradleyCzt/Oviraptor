fn paid_single_deletion_fixture() -> (WebModeFixture, AgentRunContext, String) {
    let (f, context, owned, calls) = original_terminal_dispatch(true);
    assert!(calls > 0);
    let root = owned
        .original_terminal
        .root_run_id
        .as_ref()
        .unwrap()
        .clone();
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &f.path,
        &f.scan,
        &context.route,
        &owned,
        &mut tally
    ));
    let report = json!({"targets":tally.counted(),"originalRoot":root,"stopCode":owned.outcome.terminal_code(),"detail":owned.outcome.detail()});
    drop(owned);
    assert!(finish_native_branch(
        &f.path,
        &f.scan,
        1,
        "web",
        "partial",
        "actual closed target",
        &report
    )
    .unwrap());
    (f, context, root)
}

// Actual ordinary Single target, original consumer and native branch finalizer.
#[test]
fn scan_deletion_paid_single_closed_branch_deletes_task_without_losing_financial_sources() {
    let (f, context, root) = paid_single_deletion_fixture();
    let db = db::open(&f.path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&f.scan],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "partial"
    );
    for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
        let b = crate::agent_runtime::multi_agent::budget::balance(&db, &root, None, dimension)
            .unwrap();
        assert_eq!((b.reserved, b.indeterminate), (0, 0));
    }
    let tables = [
        "agent_budget_entries",
        "agent_root_budget_attempts",
        "agent_root_model_journal",
        "agent_budget_limits",
        "agent_budget_clock_origins",
        "agent_single_exit_receipts",
        "agent_single_projection_receipts",
        "agent_root_budget_definitions",
        "agent_root_mode_definitions",
    ];
    let before = tables.map(|t| {
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            &format!("SELECT rowid,* FROM {t} ORDER BY rowid"),
            [],
        )
        .unwrap()
    });
    let native = fs::read(context.target_dir.join("frontend-evidence.json")).unwrap();
    db.execute_batch("CREATE TABLE private_business(key TEXT PRIMARY KEY,value BLOB) WITHOUT ROWID;INSERT INTO private_business VALUES('asset',X'00FF41');").unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let proof = crate::agent_runtime::deleted_scan_audit::prepare(&tx, &f.scan).unwrap();
    drop(proof);
    tx.rollback().unwrap();
    delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM sentinel_scans WHERE id=?1",
            [&f.scan],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_runs WHERE scan_id=?1",
            [&f.scan],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    for (table, original) in tables.into_iter().zip(before) {
        assert!(
            crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
                &db,
                &format!("SELECT rowid,* FROM {table} ORDER BY rowid"),
                []
            )
            .unwrap()
                == original,
            "{table}"
        );
    }
    assert_eq!(
        fs::read(context.target_dir.join("frontend-evidence.json")).unwrap(),
        native
    );
    assert_eq!(
        db.query_row(
            "SELECT value FROM private_business WHERE key='asset'",
            [],
            |r| r.get::<_, Vec<u8>>(0)
        )
        .unwrap(),
        vec![0, 255, 65]
    );
    let audit: String = db
        .query_row(
            "SELECT audit_json FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
            [&f.scan],
            |r| r.get(0),
        )
        .unwrap();
    assert!(audit.contains(&root));
    delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT audit_json FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
            [&f.scan],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        audit
    );
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, &root)
            .is_err()
    );
}

#[test]
fn scan_deletion_paid_single_audit_trigger_cannot_write_business_or_foreign_task() {
    for mutation in [
        "UPDATE private_business SET value='changed'",
        "UPDATE sentinel_scans SET task_name='changed' WHERE id='other-scan'",
    ] {
        let (f, context, _root) = paid_single_deletion_fixture();
        let db = db::open(&f.path).unwrap();
        db.execute_batch("CREATE TABLE private_business(value TEXT);INSERT INTO private_business VALUES('original');INSERT INTO sentinel_scans(id,project_id,project_name,status,task_name) VALUES('other-scan',9001,'Mode test','draft','original');").unwrap();
        db.execute_batch(&format!("CREATE TRIGGER audit_fault AFTER INSERT ON native_deleted_scan_audits_v2 BEGIN {mutation}; END;")).unwrap();
        let before = single_finally_physical(&db);
        let source = fs::read(context.target_dir.join("frontend-evidence.json")).unwrap();
        assert!(
            delete_sentinel_scan_inner(&f.path, &f.scan).is_err(),
            "{mutation}"
        );
        assert_eq!(single_finally_physical(&db), before, "{mutation}");
        assert_eq!(
            fs::read(context.target_dir.join("frontend-evidence.json")).unwrap(),
            source
        );
    }
}
#[test]
fn scan_deletion_paid_single_archive_includes_original_sdk_log_rows_and_rowids() {
    let (f, _context, _root) = paid_single_deletion_fixture();
    let db = db::open(&f.path).unwrap();
    let rows: i64 = db
        .query_row("SELECT count(*) FROM native_sdk_log_rows", [], |r| r.get(0))
        .unwrap();
    assert!(rows > 0);
    delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
    let text: String = db
        .query_row(
            "SELECT audit_json FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
            [&f.scan],
            |r| r.get(0),
        )
        .unwrap();
    let value: JsonValue = serde_json::from_str(&text).unwrap();
    let logs = value["tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "native_sdk_log_rows")
        .unwrap();
    assert_eq!(
        i64::try_from(logs["rows"].as_array().unwrap().len()).unwrap(),
        rows
    );
    assert_eq!(logs["columns"][0], "rowid");
    assert!(crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &f.scan).unwrap());
}
