// Disposable actual creator/SDK/finally/consumer/branch; never a relabelled Root.
#[test]
fn scan_deletion_paid_single_atomic_failures_preserve_original_rows_and_files() {
    for fault in [
        "CREATE TRIGGER audit_fault BEFORE INSERT ON native_deleted_scan_audits_v2 BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER audit_fault BEFORE INSERT ON sentinel_deleted_scans BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER audit_fault BEFORE DELETE ON sentinel_scans BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER audit_fault AFTER DELETE ON sentinel_scans BEGIN UPDATE private_business SET value='changed'; END;",
        "CREATE TRIGGER audit_fault AFTER INSERT ON native_deleted_scan_audits_v2 BEGIN DELETE FROM sentinel_targets WHERE scan_id='other-scan'; END;",
    ] {
        let (f,context,_root)=paid_single_deletion_fixture();let db=db::open(&f.path).unwrap();
        db.execute_batch("CREATE TABLE private_business(value TEXT);INSERT INTO private_business VALUES('original');INSERT INTO sentinel_scans(id,project_id,project_name,status,task_name) VALUES('other-scan',9001,'Mode test','draft','original');INSERT INTO sentinel_targets(project_id,scan_id,company,url) VALUES(9001,'other-scan','other','https://other.example.test/');").unwrap();
        db.execute_batch(fault).unwrap();let before=single_finally_physical(&db);let file=fs::read(context.target_dir.join("frontend-evidence.json")).unwrap();
        assert!(delete_sentinel_scan_inner(&f.path,&f.scan).is_err(),"{fault}");assert_eq!(single_finally_physical(&db),before,"{fault}");assert_eq!(fs::read(context.target_dir.join("frontend-evidence.json")).unwrap(),file);
    }
}
#[test]
fn scan_deletion_paid_single_existing_sdk_owner_and_missing_original_inode_refuse() {
    use sha2::Digest;
    for missing in [false, true] {
        let (f, _context, root) = paid_single_deletion_fixture();
        let db = db::open(&f.path).unwrap();
        let held = if missing {
            let canonical = fs::canonicalize(&f.path).unwrap();
            let mut name = canonical.file_name().unwrap().to_os_string();
            name.push(".invocations");
            let key = serde_json::to_vec(&(&f.scan, 1, "single-root-sdk", &root)).unwrap();
            let path = canonical
                .with_file_name(name)
                .join(format!("{:x}.lock", sha2::Sha256::digest(key)));
            assert!(path.is_file());
            fs::remove_file(&path).unwrap();
            Some(path)
        } else {
            None
        };
        let owner = if missing {
            None
        } else {
            crate::agent_runtime::execution_owner::probe_native_invocation(
                &f.path,
                &f.scan,
                1,
                "single-root-sdk",
                &root,
            )
            .unwrap()
        };
        assert!(missing || owner.is_some());
        let before = single_finally_physical(&db);
        assert!(delete_sentinel_scan_inner(&f.path, &f.scan)
            .unwrap_err()
            .contains("native_paid_audit_retention_required"));
        assert_eq!(single_finally_physical(&db), before);
        if let Some(path) = held {
            assert!(!path.exists());
        } else {
            drop(owner);
            delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
        }
    }
}
#[test]
fn scan_deletion_paid_single_scope_closure_rejects_foreign_native_alias_and_protected_records() {
    for foreign in [false, true] {
        let (f, _context, root) = paid_single_deletion_fixture();
        let db = db::open(&f.path).unwrap();
        if foreign {
            db.execute_batch("CREATE TABLE native_foreign_alias(scan_id TEXT,run_id TEXT REFERENCES agent_runs(id) ON DELETE CASCADE);").unwrap();
            db.execute(
                "INSERT INTO native_foreign_alias VALUES('other-scan',?1)",
                [&root],
            )
            .unwrap();
        } else {
            db.execute_batch("CREATE TABLE protected_business(scan_id TEXT REFERENCES sentinel_scans(id) ON DELETE CASCADE,value BLOB);").unwrap();
            db.execute(
                "INSERT INTO protected_business VALUES(?1,X'00FF41')",
                [&f.scan],
            )
            .unwrap();
        }
        let before = single_finally_physical(&db);
        assert!(delete_sentinel_scan_inner(&f.path, &f.scan).is_err());
        assert_eq!(single_finally_physical(&db), before);
    }
}
#[test]
fn scan_deletion_paid_single_deleted_audit_is_immutable_and_rehashed_source_damage_cannot_grant() {
    let (f, _context, root) = paid_single_deletion_fixture();
    let db = db::open(&f.path).unwrap();
    delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
    let after = single_finally_physical(&db);
    for sql in ["UPDATE native_deleted_scan_audits_v2 SET audit_hash='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'","DELETE FROM native_deleted_scan_audits_v2","INSERT OR REPLACE INTO native_deleted_scan_audits_v2 SELECT * FROM native_deleted_scan_audits_v2"] {assert!(db.execute_batch(sql).is_err());assert_eq!(single_finally_physical(&db),after);}
    let text: String = db
        .query_row(
            "SELECT audit_json FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
            [&f.scan],
            |r| r.get(0),
        )
        .unwrap();
    let mut value: JsonValue = serde_json::from_str(&text).unwrap();
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
        .position(|c| c == "status")
        .unwrap();
    run["rows"][0][column]["value"] = json!("running");
    let corrupt = value.to_string();
    // Deliberate corruption in disposable DB only; no production trigger removal.
    db.execute_batch("DROP TRIGGER deleted_audit_v2_no_update")
        .unwrap();
    db.execute(
        "UPDATE native_deleted_scan_audits_v2 SET audit_json=?1,audit_hash=?2 WHERE scan_id=?3",
        params![
            corrupt,
            crate::agent_runtime::store::stable_hash(&corrupt),
            f.scan
        ],
    )
    .unwrap();
    let before = single_finally_physical(&db);
    assert!(crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &f.scan).is_err());
    assert!(delete_sentinel_scan_inner(&f.path, &f.scan).is_err());
    assert_eq!(single_finally_physical(&db), before);
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, &root)
            .is_err()
    );
}
#[test]
fn scan_deletion_paid_single_missing_audit_cannot_acknowledge_ambiguous_retry() {
    let (f, _context, _root) = paid_single_deletion_fixture();
    let db = db::open(&f.path).unwrap();
    delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
    db.execute_batch(
        "DROP TRIGGER deleted_audit_v2_no_delete;DELETE FROM native_deleted_scan_audits_v2;",
    )
    .unwrap();
    let before = single_finally_physical(&db);
    assert!(delete_sentinel_scan_inner(&f.path, &f.scan)
        .unwrap_err()
        .contains("deleted_audit_original_sources_missing"));
    assert_eq!(single_finally_physical(&db), before);
}

#[test]
fn scan_deletion_paid_single_v2_dual_archive_and_body_version_damage_are_rejected() {
    for dual in [false, true] {
        let (f, _context, _root) = paid_single_deletion_fixture();
        let db = db::open(&f.path).unwrap();
        delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
        assert!(crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &f.scan).unwrap());
        let (version,raw,hash,stamp):(i64,String,String,String)=db.query_row("SELECT version,audit_json,audit_hash,deleted_at FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",[&f.scan],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(version, 2);
        let mut body: JsonValue = serde_json::from_str(&raw).unwrap();
        assert_eq!(body["version"], 2);
        if dual {
            // Disposable cold collision; this is an invalid second archive,
            // not a claim that newly born v2 sources were historically v1.
            db.execute_batch("DROP TRIGGER deleted_audit_v1_v2_conflict")
                .unwrap();
            db.execute("INSERT INTO native_deleted_scan_audits(scan_id,version,audit_json,audit_hash,deleted_at) VALUES(?1,1,?2,?3,?4)",params![f.scan,raw,hash,stamp]).unwrap();
        } else {
            body["version"] = json!(1);
            let corrupt = body.to_string();
            db.execute_batch("DROP TRIGGER deleted_audit_v2_no_update")
                .unwrap();
            db.execute("UPDATE native_deleted_scan_audits_v2 SET audit_json=?1,audit_hash=?2 WHERE scan_id=?3",params![corrupt,crate::agent_runtime::store::stable_hash(&corrupt),f.scan]).unwrap();
        }
        let before = single_finally_physical(&db);
        assert_eq!(
            crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &f.scan).unwrap_err(),
            if dual {
                "deleted_audit_duplicate_archive"
            } else {
                "deleted_audit_deletion_conflict"
            }
        );
        assert!(delete_sentinel_scan_inner(&f.path, &f.scan).is_err());
        assert_eq!(single_finally_physical(&db), before);
    }
}
