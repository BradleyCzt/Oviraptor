// Original paid production chain; all deliberate damage is disposable fixture data.
#[test]
fn scan_deletion_paid_multi_original_owners_in_use_refuse_without_writes() {
    let (h, root, stop) = paid_multi_archive_fixture();
    let db = db::open(&h.db_path).unwrap();
    let specialist:String=db.query_row("SELECT child_run_id FROM agent_specialist_calls WHERE root_run_id=?1 ORDER BY rowid LIMIT 1",[&root],|r|r.get(0)).unwrap();
    let web:String=db.query_row("SELECT child_run_id FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid LIMIT 1",[&root],|r|r.get(0)).unwrap();
    let calls = h.model_seen.lock().unwrap().len();
    for (kind, key) in [
        ("target", h.context.target_url.as_str()),
        ("parent-supervisor", root.as_str()),
        ("root-decision-sdk", root.as_str()),
        ("specialist-sdk", specialist.as_str()),
        ("web-executor-sdk", web.as_str()),
    ] {
        let owner = crate::agent_runtime::execution_owner::probe_native_invocation(
            &h.db_path,
            &h.context.scan_id,
            1,
            kind,
            key,
        )
        .unwrap()
        .expect("real producer original inode");
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).is_err(),
            "{kind}"
        );
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{kind}"
        );
        drop(owner);
    }
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
#[test]
fn scan_deletion_paid_multi_missing_original_target_is_not_created_by_delete() {
    use sha2::Digest;
    let (h, _root, stop) = paid_multi_archive_fixture();
    let db = db::open(&h.db_path).unwrap();
    let canonical = fs::canonicalize(&h.db_path).unwrap();
    let mut name = canonical.file_name().unwrap().to_os_string();
    name.push(".invocations");
    let key =
        serde_json::to_vec(&(&h.context.scan_id, 1, "target", &h.context.target_url)).unwrap();
    let path = canonical
        .with_file_name(name)
        .join(format!("{:x}.lock", sha2::Sha256::digest(key)));
    assert!(path.is_file());
    fs::remove_file(&path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id)
        .unwrap_err()
        .contains("scan_quiescence_original_target_exit_missing"));
    assert!(!path.exists());
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
#[test]
fn scan_deletion_paid_multi_atomic_write_faults_preserve_all_original_rows() {
    let (h, _root, stop) = paid_multi_archive_fixture();
    let db = db::open(&h.db_path).unwrap();
    db.execute_batch("CREATE TABLE private_business(value TEXT);INSERT INTO private_business VALUES('original');INSERT INTO sentinel_scans(id,project_id,project_name,status,task_name) VALUES('other-scan',9001,'original','draft','original');INSERT INTO sentinel_targets(project_id,scan_id,company,url) VALUES(9001,'other-scan','other','https://other.example.test/');").unwrap();
    let file = fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap();
    for fault in [
        "CREATE TRIGGER audit_fault BEFORE INSERT ON native_deleted_scan_audits_v2 BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER audit_fault BEFORE INSERT ON native_deleted_scan_anchors BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER audit_fault BEFORE INSERT ON sentinel_deleted_scans BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER audit_fault BEFORE DELETE ON sentinel_scans BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER audit_fault AFTER DELETE ON sentinel_scans BEGIN UPDATE private_business SET value='changed'; END;",
        "CREATE TRIGGER audit_fault AFTER INSERT ON native_deleted_scan_anchors BEGIN DELETE FROM sentinel_targets WHERE scan_id='other-scan'; END;",
    ] {
        db.execute_batch(fault).unwrap();let before=super::tests::application_table_snapshot(&db);
        assert!(delete_sentinel_scan_inner(&h.db_path,&h.context.scan_id).is_err(),"{fault}");
        assert!(super::tests::application_table_snapshot(&db)==before,"{fault}");
        assert_eq!(fs::read(h.context.target_dir.join("frontend-evidence.json")).unwrap(),file);
        db.execute_batch("DROP TRIGGER audit_fault").unwrap();
    }
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
#[test]
fn scan_deletion_paid_multi_independent_anchor_rejects_rehashed_worker_physical_damage() {
    let (h, root, stop) = paid_multi_archive_fixture();
    let db = db::open(&h.db_path).unwrap();
    delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).unwrap();
    let original = super::tests::application_table_snapshot(&db);
    for sql in ["UPDATE native_deleted_scan_anchors SET audit_hash='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'","DELETE FROM native_deleted_scan_anchors","INSERT OR REPLACE INTO native_deleted_scan_anchors SELECT * FROM native_deleted_scan_anchors"] {
        assert!(db.execute_batch(sql).is_err());assert!(super::tests::application_table_snapshot(&db)==original);
    }
    let raw: String = db
        .query_row(
            "SELECT audit_json FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
            [&h.context.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut value: JsonValue = serde_json::from_str(&raw).unwrap();
    let run = value["tables"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["name"] == "agent_runs")
        .unwrap();
    let column = run["columns"]
        .as_array()
        .unwrap()
        .iter()
        .position(|c| c == "role")
        .unwrap();
    let worker = run["rows"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r[column]["value"] != "coordinator")
        .unwrap();
    let old_worker = worker.to_string();
    worker[0]["value"] = json!(worker[0]["value"].as_i64().unwrap() + 100000);
    assert_eq!(raw.matches(&old_worker).count(), 1);
    let corrupt = raw.replacen(&old_worker, &worker.to_string(), 1);
    // Only the temp audit trigger is removed; independent anchors stay immutable.
    db.execute_batch("DROP TRIGGER deleted_audit_v2_no_update")
        .unwrap();
    db.execute(
        "UPDATE native_deleted_scan_audits_v2 SET audit_json=?1,audit_hash=?2 WHERE scan_id=?3",
        params![
            corrupt,
            crate::agent_runtime::store::stable_hash(&corrupt),
            h.context.scan_id
        ],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &h.context.scan_id)
            .unwrap_err(),
        "deleted_audit_original_anchor_conflict"
    );
    assert!(delete_sentinel_scan_inner(&h.db_path, &h.context.scan_id).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&db, &root)
            .is_err()
    );
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn scan_deletion_paid_multi_original_result_ack_damage_cannot_be_archived_as_closed() {
    let (h, _root, stop) = paid_multi_archive_fixture();
    let db = db::open(&h.db_path).unwrap();
    assert_eq!(db.execute("UPDATE agent_messages SET acknowledged_at='' WHERE from_agent='external_surface' AND to_agent='coordinator' AND kind='evidence_summary'",[]).unwrap(),1);
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        crate::agent_runtime::deleted_scan_audit::prepare(&tx, &h.context.scan_id).is_err(),
        "an original worker ACK is a closure obligation, not a mutable label"
    );
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
