fn bundle_import_fixture() -> (PathBuf, AppState, rusqlite::Connection) {
    let root = std::env::temp_dir().join(format!("oviraptor-bundle-import-{}", Uuid::new_v4()));
    let app_data_dir = root.join("app");
    let db_path = db::initialize(&app_data_dir).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection.execute_batch("INSERT INTO projects(id,name) VALUES(1,'Keep project');
        INSERT INTO sentinel_scans(id,project_id,status,task_name,attempt_count) VALUES('live',1,'scanning','Keep task',1);
        INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status) VALUES('live',1,'scanning');
        INSERT INTO sentinel_deleted_scans(scan_id) VALUES('deleted');").unwrap();
    let state = AppState {
        db_path,
        app_data_dir,
        legacy_icon_dirs: vec![],
        export_dir: root.join("exports"),
        cancellations: Arc::new(Mutex::new(HashMap::new())),
        active_jobs: Arc::new(AtomicUsize::new(0)),
        worker_service: crate::worker::WorkerServiceControl::default(),
    };
    (root, state, connection)
}

// Every non-import table is part of the preservation contract, including future
// task, permission, budget and model-receipt tables added to this schema.
fn bundle_native_snapshot(connection: &rusqlite::Connection) -> Vec<(String, Vec<String>)> {
    let tables: Vec<String> = connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'import_%' AND name<>'artifact_objects' ORDER BY name")
        .unwrap().query_map([], |r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    tables
        .into_iter()
        .map(|name| {
            let without_rowid: bool = connection
                .query_row(
                    "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                    [&name],
                    |r| r.get(0),
                )
                .unwrap();
            let quoted = name.replace('"', "\"\"");
            let order = if without_rowid {
                let keys: Vec<String> = connection
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
            let mut query = connection.prepare(&sql).unwrap();
            let count = query.column_count();
            let rows = query
                .query_map([], |r| {
                    let values = (0..count)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

#[test]
fn bundle_import_results_cannot_overwrite_live_native_state() {
    let (root, state, connection) = bundle_import_fixture();
    let before = bundle_native_snapshot(&connection);
    let content=json!({"format":"oviraptor-sentinel-v1","scan":{"id":"live","status":"completed","taskName":"Overwrite","attemptCount":99},
        "findings":[{"scanId":"live","recordKey":"f1","title":"Imported claim","severity":"critical","recordJson":"{\"reviewState\":\"confirmed\",\"executionEligible\":true}"}],
        "validations":[{"scanId":"live","findingKey":"f1","verdict":"confirmed"}]}).to_string();
    assert!(
        import_sentinel_results_content(&state.db_path, &state.app_data_dir, &content).unwrap() > 0
    );
    assert_eq!(
        bundle_native_snapshot(&connection),
        before,
        "JSON import must never update native tables"
    );
    let history = historical_import_runs(&connection, None).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].scan_id, "live");
    let previews = historical_import_previews(&connection, "live").unwrap();
    assert!(previews.iter().any(|row| row.title == "Imported claim"));
    assert!(previews
        .iter()
        .all(|row| row.read_only && !row.execution_eligible && row.review_state == "unreviewed"));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_import_project_never_revives_deleted_scan_or_creates_native_rows() {
    let (root, state, connection) = bundle_import_fixture();
    let before = bundle_native_snapshot(&connection);
    let source = root.join("old-project.json");
    let content = json!({"format":"asset-atlas-sentinel-project-v2","project":{"name":"Restored"},
        "scans":[{"id":"deleted","status":"scanning"},{"id":"new-history","status":"completed"}],
        "findings":[{"scanId":"new-history","recordKey":"f1","title":"History","severity":"high"}]})
    .to_string();
    fs::write(&source, &content).unwrap();
    assert!(
        import_sentinel_project_path(
            &state.db_path,
            &state.app_data_dir,
            source.to_str().unwrap()
        )
        .unwrap()
            > 0
    );
    assert_eq!(fs::read_to_string(&source).unwrap(), content);
    assert_eq!(
        bundle_native_snapshot(&connection),
        before,
        "project import must preserve tombstones and all native rows"
    );
    let history = historical_import_runs(&connection, None).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].scan_id, "new-history");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_import_rejects_foreign_rows_and_malformed_documents_before_writing() {
    let (root, state, connection) = bundle_import_fixture();
    for content in [
        json!({"scan":{"id":"live"}}),
        json!({"format":"unrelated","scan":{"id":"live"}}),
        json!({"format":"oviraptor-sentinel-v1","scan":{"id":"live"},"findings":{}}),
        json!({"format":"oviraptor-sentinel-v1","scan":{"id":"live"},"validations":[{"scanId":"foreign"}]}),
        json!({"format":"oviraptor-sentinel-project-v2","project":{},"scans":[{"id":"live"}],"targets":[{"scanId":"foreign","url":"https://example.invalid/"}]}),
        json!({"format":"oviraptor-sentinel-project-v2","project":{},"scans":[{"id":"live"},{"id":"live"}]}),
        json!({"format":"oviraptor-sentinel-v1","scan":{"id":"live","attemptCount":-1}}),
        json!({"format":"oviraptor-sentinel-v1","scan":{"id":"live"},"findings":[{"recordKey":"f"},{"recordKey":"f"}]}),
        json!({"format":"oviraptor-source-review-v1","schemaVersion":2,"review":{"scanId":"live","attemptNumber":1,"findings":[]}}),
    ] {
        let before = deletion_snapshot(&connection);
        assert!(
            import_sentinel_results_content(
                &state.db_path,
                &state.app_data_dir,
                &content.to_string()
            )
            .is_err(),
            "{content}"
        );
        assert_eq!(deletion_snapshot(&connection), before);
        assert!(!state.app_data_dir.join("artifact-cas").exists());
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_import_preserves_raw_bytes_seals_secrets_and_is_idempotent_across_entrypoints() {
    let (root, state, connection) = bundle_import_fixture();
    let content="{\n  \"format\": \"asset-atlas-sentinel-v1\", \"scan\": {\"id\":\"live\",\"taskPath\":\"/never/read/this\"},\n  \"findings\":[{\"recordKey\":\"f1\",\"title\":\"One\",\"recordJson\":\"{\\\"authorization\\\":\\\"Bearer never-expose-bundle-secret\\\"}\"}], \"futureExtension\": {\"keep\":true}\n}";
    let source = root.join("any-name.json");
    fs::write(&source, content).unwrap();
    let before = bundle_native_snapshot(&connection);
    assert_eq!(
        import_sentinel_results_content(&state.db_path, &state.app_data_dir, content).unwrap(),
        2
    );
    let counts: (i64,i64,i64)=connection.query_row("SELECT (SELECT COUNT(*) FROM import_bundles),(SELECT COUNT(*) FROM import_record_revisions),(SELECT COUNT(*) FROM import_projection_memberships)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(
        import_sentinel_project_path(
            &state.db_path,
            &state.app_data_dir,
            source.to_str().unwrap()
        )
        .unwrap(),
        2
    );
    let after: (i64,i64,i64)=connection.query_row("SELECT (SELECT COUNT(*) FROM import_bundles),(SELECT COUNT(*) FROM import_record_revisions),(SELECT COUNT(*) FROM import_projection_memberships)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(counts, after);
    assert_eq!(fs::read(&source).unwrap(), content.as_bytes());
    assert_eq!(bundle_native_snapshot(&connection), before);
    let (stored,secret,display):(String,i64,String)=connection.query_row("SELECT storage_path,secret_material,display_text FROM artifact_objects WHERE content_hash=?1",[crate::artifact_import::canonical::sha256_hex(content.as_bytes())],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(secret, 1);
    assert!(!display.contains("never-expose-bundle-secret"));
    let raw = fs::read(stored).unwrap();
    assert_ne!(raw, content.as_bytes());
    let key = crate::artifact_import::OriginalKey::load_or_create(
        &state.app_data_dir.join("artifact-import.key"),
    )
    .unwrap();
    let restored = crate::artifact_import::open_sealed_original(&key, &raw);
    assert_eq!(restored.unwrap(), content.as_bytes());
    let envelopes: Vec<String> = connection
        .prepare("SELECT envelope_json FROM import_record_revisions")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(envelopes
        .iter()
        .all(|text| !text.contains("never-expose-bundle-secret")));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_import_project_commit_rolls_back_every_scope_on_second_scope_failure() {
    let (root, state, connection) = bundle_import_fixture();
    connection.execute_batch("CREATE TRIGGER bundle_import_fault BEFORE INSERT ON import_projection_memberships WHEN NEW.scope_key='scan=z-history' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let before = deletion_snapshot(&connection);
    let content=json!({"format":"oviraptor-sentinel-project-v2","project":{"name":"History"},"scans":[{"id":"a-history"},{"id":"z-history"}]}).to_string();
    assert!(
        import_sentinel_results_content(&state.db_path, &state.app_data_dir, &content).is_err()
    );
    assert_eq!(
        deletion_snapshot(&connection),
        before,
        "neither first scope nor bundle/CAS ledger may partially commit"
    );
    connection
        .execute_batch("DROP TRIGGER bundle_import_fault")
        .unwrap();
    assert_eq!(
        import_sentinel_results_content(&state.db_path, &state.app_data_dir, &content).unwrap(),
        2
    );
    assert_eq!(historical_import_runs(&connection, None).unwrap().len(), 2);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_import_all_project_collections_are_read_only_and_row_identity_keeps_distinct_locations() {
    let (root, state, connection) = bundle_import_fixture();
    let before = bundle_native_snapshot(&connection);
    let content=json!({"format":"oviraptor-sentinel-project-v2","project":{"name":"All"},"scans":[{"id":"live"}],
        "findings":[{"scanId":"live","recordKey":"same","targetUrl":"https://one.invalid/","title":"One"},{"scanId":"live","recordKey":"same","targetUrl":"https://two.invalid/","title":"Two"}],
        "targets":[{"scanId":"live","url":"https://one.invalid/","status":"queued"}],
        "checkpoints":[{"scanId":"live","stage":"s1","rawJson":"{\"status\":\"complete\"}"}],
        "validations":[{"scanId":"live","findingKey":"same","verdict":"confirmed"}],
        "opportunities":[{"scanId":"live","opportunityKey":"go","recommendedActionJson":"{\"action\":\"execute\"}"}],
        "fuseZone":[{"sourceScanId":"live","url":"https://one.invalid/","archived":true},{"sourceScanId":"","url":"https://project.invalid/"}]}).to_string();
    assert_eq!(
        import_sentinel_results_content(&state.db_path, &state.app_data_dir, &content).unwrap(),
        9
    );
    assert_eq!(bundle_native_snapshot(&connection), before);
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM import_record_revisions WHERE record_kind='finding_candidate'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_import_file_limits_and_nonregular_sources_are_rejected() {
    let (root, state, connection) = bundle_import_fixture();
    let before = deletion_snapshot(&connection);
    let source = root.join("oversized.json");
    fs::File::create(&source)
        .unwrap()
        .set_len(crate::artifact_import::Limits::default().file_bytes + 1)
        .unwrap();
    assert!(import_sentinel_project_path(
        &state.db_path,
        &state.app_data_dir,
        source.to_str().unwrap()
    )
    .unwrap_err()
    .contains("bytes_limit"));
    assert!(import_sentinel_project_path(
        &state.db_path,
        &state.app_data_dir,
        root.to_str().unwrap()
    )
    .is_err());
    #[cfg(unix)]
    {
        let link = root.join("link.json");
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert!(import_sentinel_project_path(
            &state.db_path,
            &state.app_data_dir,
            link.to_str().unwrap()
        )
        .is_err());
    }
    let deep = format!("{}0{}", "[".repeat(25), "]".repeat(25));
    assert!(
        import_sentinel_results_content(&state.db_path, &state.app_data_dir, &deep)
            .unwrap_err()
            .contains("depth_limit")
    );
    assert_eq!(deletion_snapshot(&connection), before);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_import_real_source_report_roundtrip_retains_bytes_without_reviewer_authority() {
    let (source_root, source_db, lease, result) =
        source_reviewer_execution_fixture("all_confirmed", None);
    let execution = result.unwrap();
    let expected = execution["sourceDecisionProjection"]["findings"].clone();
    let report = write_native_source_findings_export(
        &source_db,
        &source_root.join("exports"),
        &lease.scan_id,
        1,
    )
    .unwrap();
    let bytes = fs::read(&report).unwrap();
    let exported: JsonValue = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        exported["review"]["ciGate"]["freeze"],
        execution["gate"]["freeze"]
    );
    assert_eq!(
        exported["review"]["ciGate"]["policy"],
        execution["gate"]["policy"]
    );
    let (root, state, connection) = bundle_import_fixture();
    let before = bundle_native_snapshot(&connection);
    assert_eq!(
        import_sentinel_project_path(&state.db_path, &state.app_data_dir, &report).unwrap(),
        expected.as_array().unwrap().len() as i64 + 1
    );
    assert_eq!(bundle_native_snapshot(&connection), before);
    assert_eq!(
        native_source_findings(&connection, &lease.scan_id, 1, 0, 50).unwrap_err(),
        "attempt_not_found"
    );
    let runs = historical_import_runs(&connection, None).unwrap();
    let history = runs.iter().find(|r| r.scan_id == lease.scan_id).unwrap();
    assert_eq!(
        history.finding_candidates,
        expected.as_array().unwrap().len() as i64
    );
    let header: String = connection
        .query_row(
            "SELECT envelope_json FROM import_record_revisions WHERE record_kind='run_state'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let header: JsonValue = serde_json::from_str(&header).unwrap();
    assert_eq!(header["extensions"]["ciGate"], exported["review"]["ciGate"]);
    let previews =
        historical_bundle_previews(&connection, &history.bundle_id, history.row_id).unwrap();
    assert!(previews
        .iter()
        .all(|p| p.read_only && !p.execution_eligible && p.review_state == "unreviewed"));
    let stored: String = connection
        .query_row(
            "SELECT storage_path FROM artifact_objects WHERE content_hash=?1",
            [crate::artifact_import::canonical::sha256_hex(&bytes)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fs::read(stored).unwrap(), bytes);
    assert_eq!(fs::read(&report).unwrap(), bytes);
    // Claims embedded inside the source format cannot elevate the envelope.
    let mut forged: JsonValue = serde_json::from_slice(&bytes).unwrap();
    forged["executionEligible"] = json!(true);
    forged["coverageReviewCompleted"] = json!(true);
    import_sentinel_results_content(&state.db_path, &state.app_data_dir, &forged.to_string())
        .unwrap();
    assert_eq!(bundle_native_snapshot(&connection), before);
    assert_eq!(
        native_source_findings(&connection, &lease.scan_id, 1, 0, 50).unwrap_err(),
        "attempt_not_found"
    );
    drop(connection);
    drop(source_db);
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(source_root).unwrap();
}

#[test]
fn bundle_import_real_project_export_preserves_all_collections_without_native_writes() {
    let (root, state, connection) = bundle_import_fixture();
    connection.execute_batch("INSERT INTO sentinel_targets(project_id,scan_id,url) VALUES(1,'live','https://bundle.invalid/');
        INSERT INTO sentinel_checkpoints(scan_id,url,stage) VALUES('live','https://bundle.invalid/','recon');
        INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title) VALUES('live','https://bundle.invalid/','recon','candidate','f1','Retained finding');
        INSERT INTO sentinel_validations(scan_id,url,finding_key,verdict) VALUES('live','https://bundle.invalid/','f1','confirmed');
        INSERT INTO sentinel_opportunities(project_id,scan_id,opportunity_key) VALUES(1,'live','o1');
        INSERT INTO sentinel_fuse_zone(project_id,url,normalized_url,source_scan_id) VALUES(1,'https://bundle.invalid/','https://bundle.invalid/','live');").unwrap();
    let before = bundle_native_snapshot(&connection);
    let content =
        serde_json::to_string_pretty(&sentinel_project_bundle(&state, 1).unwrap()).unwrap();
    assert_eq!(
        import_sentinel_results_content(&state.db_path, &state.app_data_dir, &content).unwrap(),
        7
    );
    assert_eq!(bundle_native_snapshot(&connection), before);
    let previews = historical_import_previews(&connection, "live").unwrap();
    assert_eq!(previews.len(), 7);
    assert!(previews
        .iter()
        .all(|p| p.read_only && !p.execution_eligible && p.review_state == "unreviewed"));
    assert!(previews.iter().any(|p| p.title == "Retained finding"));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_import_old_attempt_and_post_deletion_reimports_cannot_restore_projection() {
    let (root, state, connection) = bundle_import_fixture();
    let snapshot = |attempt, title| {
        json!({"format":"oviraptor-sentinel-v1",
        "scan":{"id":"live","attemptCount":attempt},
        "findings":[{"recordKey":"same","title":title}]})
        .to_string()
    };
    let newer = snapshot(2, "Newer");
    let older = snapshot(1, "Older");
    import_sentinel_results_content(&state.db_path, &state.app_data_dir, &newer).unwrap();
    import_sentinel_results_content(&state.db_path, &state.app_data_dir, &older).unwrap();
    let previews = historical_import_previews(&connection, "live").unwrap();
    assert!(previews.iter().any(|p| p.title == "Newer"));
    assert!(!previews.iter().any(|p| p.title == "Older"));
    connection
        .execute_batch(
            "UPDATE sentinel_scan_attempts SET status='completed' WHERE scan_id='live';
        UPDATE sentinel_scans SET status='completed' WHERE id='live';",
        )
        .unwrap();
    delete_sentinel_scan_inner(&state.db_path, "live").unwrap();
    let deleted = bundle_native_snapshot(&connection);
    for content in [&newer, &older, &snapshot(3, "Forged next attempt")] {
        import_sentinel_results_content(&state.db_path, &state.app_data_dir, content).unwrap();
        assert!(historical_import_runs(&connection, None)
            .unwrap()
            .is_empty());
        assert!(historical_import_previews(&connection, "live")
            .unwrap()
            .is_empty());
        assert_eq!(bundle_native_snapshot(&connection), deleted);
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn bundle_import_concurrent_secret_snapshots_share_one_sealed_object_and_projection() {
    let (root, state, connection) = bundle_import_fixture();
    let before = bundle_native_snapshot(&connection);
    let content=json!({"format":"oviraptor-sentinel-v1","scan":{"id":"history"},
        "findings":[{"recordKey":"same","title":"Secret snapshot","authorization":"Bearer concurrent-import-secret"}]}).to_string();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let database = state.db_path.clone();
            let directory = state.app_data_dir.clone();
            let content = content.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                import_sentinel_results_content(&database, &directory, &content)
            })
        })
        .collect();
    for thread in threads {
        assert_eq!(thread.join().unwrap().unwrap(), 2);
    }
    for (table, expected) in [
        ("import_record_revisions", 2),
        ("import_projection_memberships", 2),
        ("artifact_objects", 1),
        ("import_bundles", 1),
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, expected, "{table}");
    }
    assert_eq!(bundle_native_snapshot(&connection), before);
    let stored: String = connection
        .query_row("SELECT storage_path FROM artifact_objects", [], |r| {
            r.get(0)
        })
        .unwrap();
    let sealed = fs::read(stored).unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("concurrent-import-secret"));
    let key = crate::artifact_import::OriginalKey::load_or_create(
        &state.app_data_dir.join("artifact-import.key"),
    )
    .unwrap();
    assert_eq!(
        crate::artifact_import::open_sealed_original(&key, &sealed).unwrap(),
        content.as_bytes()
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_export_task_repetition_does_not_replace_prior_bytes() {
    let (root, state, connection) = bundle_import_fixture();
    let first = export_sentinel_results_inner(&state.db_path, &state.export_dir, "live").unwrap();
    let original = fs::read(&first).unwrap();
    connection
        .execute(
            "UPDATE sentinel_scans SET task_name='changed' WHERE id='live'",
            [],
        )
        .unwrap();
    let before = bundle_native_snapshot(&connection);
    let second = export_sentinel_results_inner(&state.db_path, &state.export_dir, "live").unwrap();
    assert_ne!(first, second);
    assert_eq!(fs::read(first).unwrap(), original);
    assert_eq!(bundle_native_snapshot(&connection), before);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn snapshot_export_existing_legacy_filename_symlink_is_never_followed() {
    let (root, state, connection) = bundle_import_fixture();
    fs::create_dir_all(&state.export_dir).unwrap();
    let victim = root.join("keep.json");
    fs::write(&victim, b"user owned bytes").unwrap();
    std::os::unix::fs::symlink(&victim, state.export_dir.join("sentinel-live.json")).unwrap();
    let exported =
        export_sentinel_results_inner(&state.db_path, &state.export_dir, "live").unwrap();
    assert_eq!(fs::read(&victim).unwrap(), b"user owned bytes");
    assert_ne!(
        PathBuf::from(exported),
        state.export_dir.join("sentinel-live.json")
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_export_opaque_scan_id_and_long_project_name_are_not_filesystem_authority() {
    let (root, state, connection) = bundle_import_fixture();
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,status) VALUES(?1,1,'completed')",
            ["../../retained/identifier"],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE projects SET name=?1 WHERE id=1",
            ["project".repeat(100)],
        )
        .unwrap();
    let task = export_sentinel_results_inner(
        &state.db_path,
        &state.export_dir,
        "../../retained/identifier",
    )
    .unwrap();
    let project = export_sentinel_project_inner(&state.db_path, &state.export_dir, 1).unwrap();
    for path in [task, project] {
        assert_eq!(Path::new(&path).parent(), Some(state.export_dir.as_path()));
        assert!(Path::new(&path).file_name().unwrap().len() < 100);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o077, 0);
        }
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_export_real_task_and_project_files_roundtrip_as_history_only() {
    let (root, state, connection) = bundle_import_fixture();
    connection.execute_batch("INSERT INTO sentinel_findings(scan_id,target_url,stage,kind,record_key,title,severity,record_json)
        VALUES('live','https://history.invalid/','native','finding','retained','Retained finding','high','{}');
        INSERT INTO sentinel_checkpoints(scan_id,url,stage,raw_json)
        VALUES('live','https://history.invalid/','native','{\"futureExtension\":true}');").unwrap();
    let before = bundle_native_snapshot(&connection);
    let task = export_sentinel_results_inner(&state.db_path, &state.export_dir, "live").unwrap();
    let project = export_sentinel_project_inner(&state.db_path, &state.export_dir, 1).unwrap();
    let project_bytes = fs::read(&project).unwrap();
    let repeated = export_sentinel_project_inner(&state.db_path, &state.export_dir, 1).unwrap();
    assert_ne!(
        project, repeated,
        "project exports cannot collide within one second"
    );
    assert_eq!(fs::read(&project).unwrap(), project_bytes);
    for path in [&task, &project] {
        let bytes = fs::read(path).unwrap();
        let report: JsonValue = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report["qualification"], "historical_snapshot");
        assert_eq!(report["executionEligible"], false);
        assert_eq!(report["findings"][0]["title"], "Retained finding");
        assert!(
            import_sentinel_project_path(&state.db_path, &state.app_data_dir, path).unwrap() >= 3
        );
        let (stored, secret): (String, i64) = connection
            .query_row(
                "SELECT storage_path,secret_material FROM artifact_objects WHERE content_hash=?1",
                [crate::artifact_import::canonical::sha256_hex(&bytes)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let retained = fs::read(stored).unwrap();
        let retained = if secret != 0 {
            let key = crate::artifact_import::OriginalKey::load_or_create(
                &state.app_data_dir.join("artifact-import.key"),
            )
            .unwrap();
            crate::artifact_import::open_sealed_original(&key, &retained).unwrap()
        } else {
            retained
        };
        assert_eq!(
            retained, bytes,
            "import retains the actual exported file bytes"
        );
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(bundle_native_snapshot(&connection), before);
    }
    let previews = historical_import_previews(&connection, "live").unwrap();
    assert!(previews.iter().any(|row| row.title == "Retained finding"));
    assert!(previews
        .iter()
        .all(|row| row.read_only && !row.execution_eligible && row.review_state == "unreviewed"));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshot_export_missing_records_and_blocked_directory_leave_user_data_untouched() {
    let (root, state, connection) = bundle_import_fixture();
    let before = bundle_native_snapshot(&connection);
    assert!(export_sentinel_results_inner(&state.db_path, &state.export_dir, "missing").is_err());
    assert!(export_sentinel_project_inner(&state.db_path, &state.export_dir, 999).is_err());
    assert!(
        !state.export_dir.exists(),
        "missing records must fail before filesystem publication"
    );
    fs::write(&state.export_dir, b"keep user file").unwrap();
    assert_eq!(
        export_sentinel_results_inner(&state.db_path, &state.export_dir, "live").unwrap_err(),
        "historical_snapshot_export_directory_failed"
    );
    assert_eq!(
        export_sentinel_project_inner(&state.db_path, &state.export_dir, 1).unwrap_err(),
        "historical_snapshot_export_directory_failed"
    );
    assert_eq!(fs::read(&state.export_dir).unwrap(), b"keep user file");
    assert_eq!(bundle_native_snapshot(&connection), before);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
