fn deletion_fixture() -> (PathBuf, PathBuf, rusqlite::Connection) {
    let (root, path, connection) = web_start_fixture();
    connection
        .execute_batch(
            "UPDATE sentinel_scans SET status='completed' WHERE id='start-test';
        INSERT INTO sentinel_scans(id,project_id,project_name,status,previous_scan_id,task_name)
        VALUES('retry-child',1,'Startup','paused','start-test','Keep child');
        INSERT INTO sentinel_targets(project_id,scan_id,company,url,status)
        VALUES(1,'retry-child','child','https://child.example.test/','completed');",
        )
        .unwrap();
    (root, path, connection)
}

fn deletion_snapshot(connection: &rusqlite::Connection) -> Vec<Vec<String>> {
    let tables: Vec<String> = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    ).unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    tables
        .iter()
        .map(|name| {
            let without_rowid: bool = connection
                .query_row(
                    "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                    [name],
                    |r| r.get(0),
                )
                .unwrap();
            let quoted = name.replace('"', "\"\"");
            let order = if without_rowid {
                let keys: Vec<String> = connection
                    .prepare("SELECT name FROM pragma_table_info(?1,'main') WHERE pk>0 ORDER BY pk")
                    .unwrap()
                    .query_map([name], |r| r.get(0))
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
            let mut statement = connection.prepare(&sql).unwrap();
            let count = statement.column_count();
            statement
                .query_map([], |r| {
                    let values = (0..count)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        })
        .collect()
}

#[test]
fn scan_deletion_preserves_files_and_retry_children_and_is_idempotent() {
    let (root, path, connection) = deletion_fixture();
    let historical = root.join("historical-source").join("run.json");
    let native = root.join("agent-jobs/start-test/attempt-0001/task.json");
    let descriptor = root.join("sentinel-tasks/start-test.json");
    for file in [&historical, &native, &descriptor] {
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"immutable evidence").unwrap();
    }
    connection
        .execute(
            "UPDATE sentinel_scans SET task_path=?1 WHERE id='start-test'",
            [historical.to_string_lossy().as_ref()],
        )
        .unwrap();
    let child = scan_deletion_children_in(&connection, "start-test").unwrap();
    delete_sentinel_scan_inner(&path, "start-test").unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_scans WHERE id='start-test'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_targets WHERE scan_id='start-test'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT previous_scan_id FROM sentinel_scans WHERE id='retry-child'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        ""
    );
    let actual: Vec<rusqlite::types::Value> = connection
        .query_row(
            "SELECT * FROM sentinel_scans WHERE id='retry-child'",
            [],
            |r| (0..child[0].1.len()).map(|i| r.get(i)).collect(),
        )
        .unwrap();
    assert_eq!(actual, child[0].1);
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_targets WHERE scan_id='retry-child'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let after = deletion_snapshot(&connection);
    delete_sentinel_scan_inner(&path, "start-test").unwrap();
    assert_eq!(deletion_snapshot(&connection), after);
    assert!(delete_sentinel_scan_inner(&path, "missing").is_err());
    for file in [&historical, &native, &descriptor] {
        assert_eq!(fs::read(file).unwrap(), b"immutable evidence");
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_deletion_active_status_and_each_live_owner_preserve_entire_database() {
    for status in ["scanning", "pausing", "unrecognized"] {
        let (root, path, connection) = deletion_fixture();
        connection
            .execute(
                "UPDATE sentinel_scans SET status=?1 WHERE id='start-test'",
                [status],
            )
            .unwrap();
        let before = deletion_snapshot(&connection);
        assert!(delete_sentinel_scan_inner(&path, "start-test").is_err());
        assert_eq!(deletion_snapshot(&connection), before);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
    for (kind, target) in [
        ("branch", "web"),
        ("branch", "source"),
        ("frontend_producer", "web"),
        ("target", "https://start.example.test/one"),
        ("frontend_recon", "https://start.example.test/two"),
    ] {
        let (root, path, connection) = deletion_fixture();
        let attempt: i64 = connection
            .query_row(
                "SELECT attempt_count FROM sentinel_scans WHERE id='start-test'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let owner = claim_native_invocation(&path, "start-test", attempt, kind, target).unwrap();
        let before = deletion_snapshot(&connection);
        assert!(delete_sentinel_scan_inner(&path, "start-test")
            .unwrap_err()
            .contains("scan_quiescence_worker_active"));
        assert_eq!(deletion_snapshot(&connection), before);
        drop(owner);
        delete_sentinel_scan_inner(&path, "start-test").unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn scan_deletion_unresolved_obligations_cannot_be_erased_by_terminal_label() {
    for sql in [
        "INSERT INTO native_scan_branches(scan_id,attempt_number,branch) VALUES('start-test',1,'web')",
        "UPDATE agent_runs SET status='paused' WHERE id='delete-root'",
        "UPDATE agent_runs SET terminal_code='request_reconciliation_required' WHERE id='delete-root'",
        "UPDATE agent_assignments SET state='running' WHERE id='delete-assignment'",
        "UPDATE agent_assignments SET budget_settled_at='' WHERE id='delete-assignment'",
        "INSERT INTO agent_budget_ledger(root_run_id,reserved_tokens) VALUES('delete-root',1)",
        "INSERT INTO agent_lane_leases(scan_id,attempt_number,target_key,lane,assignment_id) VALUES('start-test',1,'target','review','delete-assignment')",
        "INSERT INTO agent_specialist_calls(assignment_id,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,state) VALUES('delete-assignment','delete-root','delete-root','mapper',1,'f','{}','hash','uncertain')",
        "INSERT INTO tool_invocations(run_id,tool_name) VALUES('delete-root','http_request')",
        "INSERT INTO agent_user_directives(id,scan_id,attempt_number,text_redacted) VALUES('pending-directive','start-test',1,'pending')",
        "INSERT INTO agent_external_surface_captures(scan_id,attempt_number,target_url,assignment_id,child_run_id,state) VALUES('start-test',1,'https://start.example.test/one','delete-assignment','delete-root','claimed')",
        "INSERT INTO agent_http_budget_origins(scan_id,target_url,budget_attempt,baseline_requests) VALUES('start-test','https://start.example.test/one',1,0);
         INSERT INTO agent_http_request_claims(scan_id,target_url,budget_attempt,ordinal,attempt_number,run_id,invocation_id,request_index,tool_name,identity_handle,request_hash)
         VALUES('start-test','https://start.example.test/one',1,1,1,'delete-root','unknown-invocation',1,'http_request','','hash')",
    ] {
        let (root,path,connection)=deletion_fixture();
        connection.execute_batch("INSERT INTO agent_runs(id,scan_id,status) VALUES('delete-root','start-test','completed');
            INSERT INTO agent_assignments(id,coordinator_run_id,state,dedup_key,budget_settled_at) VALUES('delete-assignment','delete-root','completed','one','settled');").unwrap();
        connection.execute_batch(sql).unwrap();
        let before=deletion_snapshot(&connection);
        assert!(delete_sentinel_scan_inner(&path,"start-test").unwrap_err().contains("未结算"),"{sql}");
        assert_eq!(deletion_snapshot(&connection),before,"{sql}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn scan_deletion_allows_settled_native_and_current_native_imports() {
    for (scan_status, run_status) in [("completed", "completed"), ("imported", "completed")] {
        let (root, path, connection) = deletion_fixture();
        connection
            .execute(
                "UPDATE sentinel_scans SET status=?1 WHERE id='start-test'",
                [scan_status],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_runs(id,scan_id,status) VALUES('delete-root','start-test',?1)",
                [run_status],
            )
            .unwrap();
        connection.execute_batch("INSERT INTO agent_assignments(id,coordinator_run_id,state,dedup_key,budget_settled_at)
             VALUES('delete-assignment','delete-root','completed','one','settled');
             INSERT INTO agent_budget_ledger(root_run_id,spent_tokens,spent_requests) VALUES('delete-root',7,1);").unwrap();
        delete_sentinel_scan_inner(&path, "start-test").unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM agent_runs WHERE scan_id='start-test'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn scan_deletion_tombstone_children_and_delete_faults_rollback_together() {
    for action in ["RAISE(ABORT,'injected')", "RAISE(IGNORE)"] {
        for (table, event, when) in [
            ("sentinel_deleted_scans", "INSERT", "1"),
            ("sentinel_scans", "UPDATE", "OLD.id='retry-child'"),
            ("sentinel_targets", "DELETE", "OLD.scan_id='start-test'"),
            ("sentinel_scans", "DELETE", "OLD.id='start-test'"),
        ] {
            let (root, path, connection) = deletion_fixture();
            connection.execute_batch(&format!("CREATE TRIGGER deletion_fault BEFORE {event} ON {table} WHEN {when} BEGIN SELECT {action}; END;")).unwrap();
            let before = deletion_snapshot(&connection);
            assert!(
                delete_sentinel_scan_inner(&path, "start-test").is_err(),
                "{table} {event} {action}"
            );
            assert_eq!(deletion_snapshot(&connection), before);
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
    for sql in [
        "CREATE TRIGGER deletion_fault AFTER INSERT ON sentinel_deleted_scans BEGIN DELETE FROM sentinel_deleted_scans WHERE scan_id=NEW.scan_id; END;",
        "CREATE TRIGGER deletion_fault AFTER INSERT ON sentinel_deleted_scans BEGIN UPDATE sentinel_scans SET task_name='corrupt' WHERE id='retry-child'; END;",
        "CREATE TRIGGER deletion_fault AFTER UPDATE ON sentinel_scans WHEN OLD.id='retry-child' BEGIN UPDATE sentinel_scans SET task_name='corrupt' WHERE id=NEW.id; END;",
        "CREATE TRIGGER deletion_fault AFTER DELETE ON sentinel_scans WHEN OLD.id='start-test' BEGIN DELETE FROM sentinel_deleted_scans WHERE scan_id=OLD.id; END;",
    ] {
        let (root,path,connection)=deletion_fixture();
        connection.execute_batch(sql).unwrap();
        let before=deletion_snapshot(&connection);
        assert!(delete_sentinel_scan_inner(&path,"start-test").is_err());
        assert_eq!(deletion_snapshot(&connection),before);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn scan_deletion_cleanup_receipts_and_inconsistent_tombstone_are_preserved() {
    for sql in [
        "INSERT INTO sentinel_processes(scan_id,process_id) VALUES('start-test',2147483647)",
        "INSERT INTO analyzer_container_receipts(receipt_id,invocation_key,scan_id,attempt_number,purpose,container_name,owner_token,create_args_json,cleanup_status)
         VALUES('receipt','key','start-test',1,'source','fixture-container','owner','[]','unconfirmed')",
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('start-test')",
    ] {
        let (root,path,connection)=deletion_fixture();
        connection.execute_batch(sql).unwrap();
        let before=deletion_snapshot(&connection);
        assert!(delete_sentinel_scan_inner(&path,"start-test").is_err());
        assert_eq!(deletion_snapshot(&connection),before);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn scan_deletion_never_signals_a_stored_pid_or_follows_task_symlinks() {
    use std::os::unix::process::CommandExt;
    let (root, path, connection) = deletion_fixture();
    let target = root.join("external.json");
    let link = root.join("task.json");
    fs::write(&target, "keep external bytes").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    connection
        .execute(
            "UPDATE sentinel_scans SET task_path=?1 WHERE id='start-test'",
            [link.to_string_lossy().as_ref()],
        )
        .unwrap();
    // A real process in its own group, blocked on a pipe we own. The only
    // normal exit is dropping that pipe; no sleep or target network is used.
    let mut child = Command::new("sh")
        .args(["-c", "read deletion_fixture_input"])
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    connection
        .execute(
            "INSERT INTO sentinel_processes(scan_id,process_id) VALUES('start-test',?1)",
            [i64::from(child.id())],
        )
        .unwrap();
    let result = delete_sentinel_scan_inner(&path, "start-test");
    let still_live = child.try_wait().unwrap().is_none();
    drop(child.stdin.take());
    child.wait().unwrap();
    assert!(result.is_err());
    assert!(still_live, "deletion must not signal a stored PID");
    // Fixture simulates an independently verified cleanup; production deletion
    // never removes this registration to make its own admission pass.
    connection
        .execute(
            "DELETE FROM sentinel_processes WHERE scan_id='start-test'",
            [],
        )
        .unwrap();
    delete_sentinel_scan_inner(&path, "start-test").unwrap();
    assert!(fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read_to_string(target).unwrap(), "keep external bytes");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn scan_deletion_lifecycle_contention_and_invalid_ids_do_not_mutate() {
    let (root, path, connection) = deletion_fixture();
    let before = deletion_snapshot(&connection);
    let owner = claim_scan_control(&path, "start-test").unwrap();
    assert!(delete_sentinel_scan_inner(&path, "start-test").is_err());
    drop(owner);
    for invalid in ["", " ", "../start-test", "a/b", "a\\b", "start-test\n"] {
        assert!(delete_sentinel_scan_inner(&path, invalid).is_err());
    }
    assert_eq!(deletion_snapshot(&connection), before);
    delete_sentinel_scan_inner(&path, "start-test").unwrap();
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
