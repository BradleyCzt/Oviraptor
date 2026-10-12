include!("tests_web_resume_finance.rs");

fn web_start_fixture() -> (PathBuf,PathBuf,rusqlite::Connection) {
    let root = std::env::temp_dir().join(format!("oviraptor-web-start-{}",Uuid::new_v4()));
    let db_path = db::initialize(&root).unwrap();
    let connection = db::open(&db_path).unwrap();
    connection.execute("INSERT INTO projects(id,name) VALUES(1,'Startup')",[]).unwrap();
    with_web_creator_test_id("start-test",||create_sentinel_url_scan_with_mode_in(&connection,1,"Startup".into(),
        vec!["https://start.example.test/one".into(),"https://start.example.test/two".into()],Some("standard".into()),
        None,None,None,None,None,None,None,Some("multi".into()))).unwrap();
    // Display labels are fixture inputs fixed before the first startup HMAC.
    connection.execute("UPDATE sentinel_targets SET company=CASE WHEN url LIKE '%/one' THEN 'one' ELSE 'two' END WHERE scan_id='start-test'",[]).unwrap();
    (root,db_path,connection)
}

fn web_start_snapshot(connection: &rusqlite::Connection) -> Vec<Vec<String>> {
    ["sentinel_scans","sentinel_targets","sentinel_scan_contexts","sentinel_scan_attempts",
        "native_scan_branches","sentinel_checkpoints","sentinel_findings","sentinel_opportunities",
        "app_settings","agent_runs","browser_auth_sessions"].iter().map(|table| {
        let mut statement = connection.prepare(&format!("SELECT * FROM {table} ORDER BY 1,2")).unwrap();
        let count = statement.column_count();
        statement.query_map([], |row| {
            let fields = (0..count).map(|i| row.get::<_,rusqlite::types::Value>(i).map(|v| format!("{v:?}"))).collect::<Result<Vec<_>,_>>()?;
            Ok(format!("{fields:?}"))
        }).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
    }).collect()
}

// Runs the production preparation/publication functions without Tauri, model,
// browser or worker dispatch. Its transaction ordering matches start_web_scan.
fn run_web_start_fixture(connection: &mut rusqlite::Connection, root: &Path, mode: WebStartMode) -> Result<SentinelScan,String> {
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    let mut startup = prepare_web_startup_in(&transaction,root,"start-test",mode)?;
    let original:String=transaction.query_row("SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id='start-test'",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    let mut policy:JsonValue=serde_json::from_str(&original).map_err(|e|e.to_string())?;
    policy["webModeCeiling"]=json!("deep");
    transaction.execute("UPDATE sentinel_scan_contexts SET policy_json=?1 WHERE scan_id='start-test'", [policy.to_string()]).map_err(|e| e.to_string())?;
    let targets=startup.targets.iter().map(|(company,url)|json!({"company":company,"url":url})).collect::<Vec<_>>();
    fs::write(startup.files.path.join("task.json"),json!({"scanId":"start-test","targets":targets,"effectiveWebPolicy":policy}).to_string()).map_err(|e|e.to_string())?;
    fs::write(startup.files.path.join("targets.json"),json!(targets).to_string()).map_err(|e|e.to_string())?;
    fs::write(startup.files.path.join("targets.txt"),startup.targets.iter().map(|(_,url)|url.as_str()).collect::<Vec<_>>().join("\n")).map_err(|e|e.to_string())?;
    fs::write(startup.files.path.join("agent-instruction.md"),"owned startup instruction").map_err(|e|e.to_string())?;
    if let Some(auth)=web_dispatch_auth_document(&transaction,1,&policy)? {crate::auth_session::write_session_document(&startup.files.path.join("auth-sessions.json"),&auth)?;}
    let result = persist_web_startup_in(&transaction,&startup,"initializing web","test-skill",&policy)?;
    register_web_dispatch_binding_in(&transaction,&startup,&binding_test_runtime())?;
    register_private_web_mode_on(&transaction,&startup,&binding_test_runtime())?;
    startup.files.preserve = true;
    transaction.commit().map_err(|e| format!("web_start_commit_unconfirmed:{e}"))?;
    Ok(result)
}

#[test]
fn web_start_commits_exact_upcoming_attempt_plan_targets_and_branch() {
    let (root,_,mut connection) = web_start_fixture();
    let historical = web_start_root(&root,"start-test").unwrap().join("attempt-0004");
    fs::create_dir(&historical).unwrap();
    fs::write(historical.join("task.json"),"historical plan").unwrap();
    let scan = run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).unwrap();
    assert_eq!(scan.status,"scanning");
    let (attempt,path,backend): (i64,String,String) = connection.query_row("SELECT s.attempt_count,s.task_path,a.backend_plan_json FROM sentinel_scans s JOIN sentinel_scan_attempts a ON a.scan_id=s.id AND a.attempt_number=s.attempt_count WHERE s.id='start-test'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(attempt,5);
    assert!(path.ends_with("attempt-0005/task.json"));
    let plan: JsonValue = serde_json::from_str(&backend).unwrap();
    assert_eq!(plan["attemptNumber"],5);
    assert_eq!(plan["targets"].as_array().unwrap().len(),2);
    assert!(plan["targets"].as_array().unwrap().iter().all(|t| t["backend"] == "native"));
    assert!(web_start_target_snapshot(&connection,"start-test").unwrap().iter().all(|t| t.5 == 5));
    assert_eq!(fs::read_to_string(historical.join("task.json")).unwrap(),"historical plan");
    let before = web_start_snapshot(&connection);
    assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).is_err());
    assert_eq!(web_start_snapshot(&connection),before);
    assert!(!historical.parent().unwrap().join("attempt-0006").exists());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_start_rolls_back_aborted_or_ignored_late_writes_and_owned_files() {
    for (table,event,condition) in [
        ("sentinel_scans","UPDATE","NEW.status='scanning'"),
        ("sentinel_scan_contexts","UPDATE","1"),
        ("sentinel_targets","UPDATE","NEW.last_attempt_number=1 AND NEW.company='two'"),
        ("sentinel_scan_attempts","INSERT","1"),
        ("sentinel_checkpoints","INSERT","NEW.stage='scan_backend_plan'"),
        ("app_settings","INSERT","NEW.key LIKE 'agent-current-attempt:%'"),
        ("native_scan_branches","INSERT","1"),
    ] {
        for fault in ["ABORT,'injected'","IGNORE"] {
            let (root,_,mut connection) = web_start_fixture();
            let before = web_start_snapshot(&connection);
            connection.execute_batch(&format!("CREATE TRIGGER start_fault BEFORE {event} ON {table} WHEN {condition} BEGIN SELECT RAISE({fault}); END;")).unwrap();
            assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).is_err(),"{table}/{fault}");
            assert_eq!(web_start_snapshot(&connection),before,"{table}/{fault}");
            assert!(!root.join("agent-jobs/start-test/attempt-0001").exists(),"{table}/{fault}");
            connection.execute_batch("DROP TRIGGER start_fault").unwrap();
            run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).unwrap();
            drop(connection);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn web_start_detects_post_write_tampering_and_rolls_back_every_surface() {
    for body in [
        "DELETE FROM native_scan_branches;",
        "UPDATE native_scan_branches SET status='completed';",
        "UPDATE native_scan_branches SET report_json='{\"unexpected\":true}';",
        "DELETE FROM sentinel_scan_attempts;",
        "UPDATE sentinel_scan_attempts SET execution_mode='fresh';",
        "UPDATE sentinel_scan_attempts SET backend_plan_json='{}';",
        "UPDATE sentinel_targets SET last_attempt_number=99;",
        "UPDATE sentinel_scans SET status='completed';",
        "UPDATE sentinel_scan_contexts SET policy_json='{}';",
        "DELETE FROM app_settings WHERE key LIKE 'agent-current-attempt:%';",
        "DELETE FROM sentinel_checkpoints WHERE stage='scan_backend_plan';",
        "UPDATE projects SET status='archived';",
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('start-test');",
    ] {
        let (root,_,mut connection) = web_start_fixture();
        let before = web_start_snapshot(&connection);
        connection.execute_batch(&format!("CREATE TRIGGER start_tamper AFTER INSERT ON native_scan_branches BEGIN {body} END;")).unwrap();
        assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).is_err(),"{body}");
        assert_eq!(web_start_snapshot(&connection),before,"{body}");
        assert!(!root.join("agent-jobs/start-test/attempt-0001").exists());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

fn seed_web_start_parent(connection: &mut rusqlite::Connection, root: &Path, status: &str) {
    run_web_start_fixture(connection,root,WebStartMode::Confirm).unwrap();
    connection.execute("UPDATE sentinel_scans SET status=?1,current_checkpoint='previous terminal reason'",[status]).unwrap();
    connection.execute("UPDATE sentinel_targets SET status='failed',routing_reason='previous routing',value_score=17",[]).unwrap();
    connection.execute("UPDATE sentinel_scan_attempts SET status=?1,finished_at='previous finish'",[status]).unwrap();
}

#[test]
fn web_start_resume_and_retry_failures_restore_original_state_without_compensation() {
    for (mode,status) in [(WebStartMode::Resume,"paused"),(WebStartMode::Retry,"partial"),(WebStartMode::Retry,"completed")] {
        let (root,_,mut connection) = web_start_fixture();
        seed_web_start_parent(&mut connection,&root,status);
        if matches!(mode,WebStartMode::Resume) {
            connection.execute("UPDATE sentinel_targets SET status='queued'",[]).unwrap();
        }
        let before = web_start_snapshot(&connection);
        connection.execute_batch("CREATE TRIGGER start_fault BEFORE INSERT ON native_scan_branches BEGIN SELECT RAISE(ABORT,'late failure'); END;").unwrap();
        assert!(run_web_start_fixture(&mut connection,&root,mode).is_err());
        assert_eq!(web_start_snapshot(&connection),before);
        assert!(root.join("agent-jobs/start-test/attempt-0001/task.json").is_file());
        assert!(!root.join("agent-jobs/start-test/attempt-0002").exists());
        connection.execute_batch("DROP TRIGGER start_fault").unwrap();
        run_web_start_fixture(&mut connection,&root,mode).unwrap();
        let execution_mode: String = connection.query_row("SELECT execution_mode FROM sentinel_scan_attempts WHERE attempt_number=2",[],|r| r.get(0)).unwrap();
        assert_eq!(execution_mode,if status == "completed" { "fresh" } else { "resume" });
        let plan: String = connection.query_row("SELECT backend_plan_json FROM sentinel_scan_attempts WHERE attempt_number=2",[],|r| r.get(0)).unwrap();
        assert_eq!(serde_json::from_str::<JsonValue>(&plan).unwrap()["attemptNumber"],2);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_start_refuses_missing_corrupt_or_retired_resume_matrix_without_writes() {
    for invalid in ["", "{}", r#"{"schemaVersion":1,"scanId":"start-test","attemptNumber":1,"targets":[{"url":"https://start.example.test/one","backend":"strix"},{"bad":"row"}]}"#,
        r#"{"schemaVersion":1,"scanId":"wrong-scan","attemptNumber":1,"targets":[]}"#,
        r#"{"schemaVersion":1,"scanId":"start-test","attemptNumber":1,"targets":[]}"#,
        r#"{"schemaVersion":1,"scanId":"start-test","attemptNumber":1,"targets":[{"url":"https://start.example.test/one","backend":"strix"},{"url":"https://start.example.test/two","backend":"strix"}]}"#] {
        let (root,_,mut connection) = web_start_fixture();
        seed_web_start_parent(&mut connection,&root,"partial");
        connection.execute("UPDATE sentinel_scan_attempts SET backend_plan_json=?1",[invalid]).unwrap();
        let before = web_start_snapshot(&connection);
        assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Retry).is_err());
        assert_eq!(web_start_snapshot(&connection),before);
        assert!(!root.join("agent-jobs/start-test/attempt-0002").exists());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_start_ignores_archived_fuses_but_enforces_active_fuses() {
    let (root,_,mut connection) = web_start_fixture();
    connection.execute_batch("INSERT INTO sentinel_fuse_zone(project_id,url,normalized_url,archived) VALUES
        (1,'https://start.example.test/one','https://start.example.test/one',1),
        (1,'https://start.example.test/two','https://start.example.test/two',0);").unwrap();
    run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).unwrap();
    let targets = web_start_target_snapshot(&connection,"start-test").unwrap();
    assert_eq!(targets[0].4,"queued");
    assert_eq!(targets[0].5,1);
    assert_eq!(targets[1].4,"fuse_excluded");
    assert_eq!(targets[1].5,0);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_start_refuses_deleted_archived_wrong_type_and_fuse_write_failure() {
    for sql in [
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('start-test');",
        "UPDATE projects SET status='archived';",
        "UPDATE sentinel_scans SET scan_type='code';",
        "INSERT INTO sentinel_fuse_zone(project_id,url,normalized_url) VALUES(1,'https://start.example.test/one','https://start.example.test/one'); CREATE TRIGGER fuse_ignore BEFORE UPDATE ON sentinel_targets WHEN NEW.status='fuse_excluded' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (root,_,mut connection) = web_start_fixture();
        connection.execute_batch(sql).unwrap();
        let before = web_start_snapshot(&connection);
        assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).is_err());
        assert_eq!(web_start_snapshot(&connection),before);
        assert!(!root.join("agent-jobs/start-test/attempt-0001").exists());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_start_concurrent_confirmations_admit_only_one_owner_and_attempt() {
    let (root,db_path,connection) = web_start_fixture();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let (root,db_path,barrier) = (root.clone(),db_path.clone(),barrier.clone());
        handles.push(thread::spawn(move || {
            barrier.wait();
            let _owner = claim_scan_control(&db_path,"start-test")?;
            let mut connection = db::open(&db_path)?;
            run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm)
        }));
    }
    barrier.wait();
    let successes = handles.into_iter().map(|h| h.join().unwrap()).filter(Result::is_ok).count();
    assert_eq!(successes,1);
    let count: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_scan_attempts",[],|r| r.get(0)).unwrap();
    assert_eq!(count,1);
    assert!(!root.join("agent-jobs/start-test/attempt-0002").exists());
    let owner = claim_scan_control(&db_path,"start-test").unwrap();
    assert!(claim_scan_control(&db_path,"start-test").is_err());
    drop(owner);
    assert!(claim_scan_control(&db_path,"start-test").is_ok());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_start_retry_preparation_ignores_and_tampering_cannot_drop_targets() {
    for trigger in [
        "BEFORE UPDATE ON sentinel_targets WHEN NEW.status='queued' AND NEW.company='two' BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER UPDATE ON sentinel_targets WHEN NEW.status='queued' AND NEW.company='two' BEGIN UPDATE sentinel_targets SET status='completed' WHERE id=NEW.id; END;",
        "BEFORE INSERT ON app_settings WHEN NEW.key LIKE 'sentinel-next-attempt-mode:%' BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE UPDATE ON sentinel_scans WHEN NEW.status='draft' BEGIN SELECT RAISE(IGNORE); END;",
    ] {
        let (root,_,mut connection) = web_start_fixture();
        seed_web_start_parent(&mut connection,&root,"partial");
        let before = web_start_snapshot(&connection);
        connection.execute_batch(&format!("CREATE TRIGGER retry_fault {trigger}")).unwrap();
        assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Retry).is_err(),"{trigger}");
        assert_eq!(web_start_snapshot(&connection),before,"{trigger}");
        assert!(!root.join("agent-jobs/start-test/attempt-0002").exists());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_start_ledger_floor_and_attempt_exhaustion_never_overwrite_history() {
    let (root,_,mut connection) = web_start_fixture();
    connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status,backend_plan_json) VALUES('start-test',7,'completed','historical bytes')",[]).unwrap();
    run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).unwrap();
    let attempt: i64 = connection.query_row("SELECT attempt_count FROM sentinel_scans WHERE id='start-test'",[],|r| r.get(0)).unwrap();
    assert_eq!(attempt,8);
    let history: String = connection.query_row("SELECT backend_plan_json FROM sentinel_scan_attempts WHERE attempt_number=7",[],|r| r.get(0)).unwrap();
    assert_eq!(history,"historical bytes");
    connection.execute("UPDATE sentinel_scans SET status='draft',attempt_count=?1",[u32::MAX]).unwrap();
    let before = web_start_snapshot(&connection);
    assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).is_err());
    assert_eq!(web_start_snapshot(&connection),before);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_start_cancel_draft_is_atomic_and_preserves_historical_paths() {
    for fault in ["ABORT,'cancel failure'", "IGNORE"] {
        let (root,_,mut connection) = web_start_fixture();
        connection.execute("INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,session_json) VALUES('owned',1,'start-test','Owned','https://start.example.test','valid','{}')",[]).unwrap();
        let historical = root.join("unrelated-plan.json");
        fs::write(&historical,"preserve historical plan").unwrap();
        connection.execute("UPDATE sentinel_scans SET task_path=?1",[historical.to_string_lossy()]).unwrap();
        let before = web_start_snapshot(&connection);
        connection.execute_batch(&format!("CREATE TRIGGER cancel_fault BEFORE DELETE ON sentinel_scans BEGIN SELECT RAISE({fault}); END;")).unwrap();
        assert!(cancel_draft_scan_in(&connection,"start-test").is_err());
        assert_eq!(web_start_snapshot(&connection),before);
        assert_eq!(fs::read_to_string(&historical).unwrap(),"preserve historical plan");
        connection.execute_batch("DROP TRIGGER cancel_fault").unwrap();
        cancel_draft_scan_in(&connection,"start-test").unwrap();
        assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).is_err());
        assert!(historical.is_file());
        let remaining: i64 = connection.query_row("SELECT COUNT(*) FROM browser_auth_sessions WHERE owner_scan_id='start-test'",[],|r| r.get(0)).unwrap();
        assert_eq!(remaining,0);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_start_committed_attempt_cannot_be_cancelled_as_a_draft() {
    let (root,_,mut connection) = web_start_fixture();
    run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).unwrap();
    let before = web_start_snapshot(&connection);
    assert!(cancel_draft_scan_in(&connection,"start-test").is_err());
    assert_eq!(web_start_snapshot(&connection),before);
    assert!(root.join("agent-jobs/start-test/attempt-0001/task.json").is_file());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_start_commit_failure_preserves_files_without_publishing_partial_rows() {
    let (root,_,mut connection) = web_start_fixture();
    connection.execute_batch("CREATE TABLE startup_fault_parent(id INTEGER PRIMARY KEY);
        CREATE TABLE startup_fault_child(id INTEGER REFERENCES startup_fault_parent(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER start_commit_fault AFTER INSERT ON native_scan_branches BEGIN INSERT INTO startup_fault_child VALUES(123); END;").unwrap();
    let before = web_start_snapshot(&connection);
    let error = run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).err().unwrap();
    assert!(error.contains("web_start_commit_unconfirmed"));
    assert_eq!(web_start_snapshot(&connection),before);
    assert!(root.join("agent-jobs/start-test/attempt-0001/task.json").is_file());
    // Preserved unconfirmed files are never overwritten by a later explicit start.
    connection.execute_batch("DROP TRIGGER start_commit_fault").unwrap();
    run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).unwrap();
    assert!(root.join("agent-jobs/start-test/attempt-0002/task.json").is_file());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn web_start_rejects_symlink_roots_and_does_not_clean_replaced_directories() {
    let (root,_,mut connection) = web_start_fixture();
    let outside = root.join("unrelated");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("task.json"),"keep").unwrap();
    std::os::unix::fs::symlink(&outside,root.join("agent-jobs")).unwrap();
    assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).is_err());
    assert_eq!(fs::read_to_string(outside.join("task.json")).unwrap(),"keep");
    fs::remove_file(root.join("agent-jobs")).unwrap();
    let scan_root = web_start_root(&root,"start-test").unwrap();
    let files = WebStartFiles::allocate(&scan_root,"start-test",1).unwrap();
    let path = files.path.clone();
    fs::rename(&path,scan_root.join("preserved-owner")).unwrap();
    fs::create_dir(&path).unwrap();
    fs::write(path.join(".oviraptor-scan-id"),"start-test").unwrap();
    fs::write(path.join("task.json"),"replacement must survive").unwrap();
    drop(files);
    assert_eq!(fs::read_to_string(path.join("task.json")).unwrap(),"replacement must survive");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
