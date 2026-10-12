#[test]
fn web_closure_preserves_old_attempt_and_requires_explicit_new_start() {
    let (root,path,work_dir,mut connection) = recovery_test_fixture();
    let old_task = fs::read(work_dir.join("task.json")).unwrap();
    let old_binding: Vec<u8> = connection.query_row("SELECT binding_tag FROM native_web_dispatch_bindings",[],|r|r.get(0)).unwrap();
    // Configuration drift must not be worked around by replaying the old plan.
    // Recovery reconstructs from today's settings, so the drift is the settings
    // change itself; the old attempt's frozen task.json stays untouched (its
    // tampering is covered by the continuation guard test below).
    connection.execute("INSERT INTO app_settings(key,value) VALUES('agent_packet_budget','73')",[]).unwrap();
    let mut drifted_runtime = binding_test_runtime();
    drifted_runtime["runtime"]["packet_budget"] = json!(73);
    let drifted_reconstruct = |c: &rusqlite::Connection| -> Result<PathBuf,String> {
        let (path,targets) = web_recovery_plan_in(c,&root,"start-test",1)?;
        assert_eq!(targets.len(),2);
        verify_web_dispatch_binding_in(c,"start-test",1,&WebBindingDirectory::open(&path)?,&drifted_runtime)?;
        Ok(path)
    };
    assert!(claim_web_recovery(&path,"start-test",1,true,drifted_reconstruct).is_err());
    let closed = close_unclaimed_web_attempt(&path,"start-test",1,true).unwrap();
    assert_eq!(closed["executionState"],"closed_without_dispatch");
    assert_eq!(closed["automaticReplayAllowed"],false);
    assert_eq!(close_unclaimed_web_attempt(&path,"start-test",1,true).unwrap(),closed);
    assert!(claim_web_recovery(&path,"start-test",1,true,drifted_reconstruct).is_err());
    assert!(claim_web_recovery(&path,"start-test",1,true,|c|recovery_test_reconstruct(c,&root)).is_err(),
        "a closed attempt cannot be replayed even with its original configuration");
    assert!(!work_dir.parent().unwrap().join("attempt-0002").exists());
    assert_eq!(connection.query_row("SELECT status FROM sentinel_scans",[],|r|r.get::<_,String>(0)).unwrap(),"cancelled");
    assert_eq!(connection.query_row("SELECT claim_id FROM native_branch_dispatches",[],|r|r.get::<_,String>(0)).unwrap(),"");
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM agent_runs",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    // Only a separate user start action may allocate a new attempt.
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let mut next = prepare_web_startup_in(&transaction,&root,"start-test",WebStartMode::Retry).unwrap();
    let policy: String = transaction.query_row("SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id='start-test'",[],|r|r.get(0)).unwrap();
    let policy: JsonValue = serde_json::from_str(&policy).unwrap();
    persist_web_startup_in(&transaction,&next,"initializing web","test-skill",&policy).unwrap();
    // The new attempt writes its own frozen task, as every real startup does.
    fs::write(next.files.path.join("task.json"),json!({"scanId":"start-test","effectiveWebPolicy":policy,
        "targets":next.targets.iter().map(|(company,url)|json!({"company":company,"url":url})).collect::<Vec<_>>()}).to_string()).unwrap();
    prepare_recovery_test_files(&next);
    for name in ["targets.txt","agent-instruction.md"] {
        fs::write(next.files.path.join(name),"test-owned startup artifact").unwrap();
    }
    let new_runtime = drifted_runtime.clone();
    register_web_dispatch_binding_in(&transaction,&next,&new_runtime).unwrap();
    register_private_web_mode_on(&transaction,&next,&new_runtime).unwrap();
    let next_dir = next.files.path.clone();
    next.files.preserve = true;
    transaction.commit().unwrap();
    let verification = connection.unchecked_transaction().unwrap();
    verify_web_dispatch_binding_in(&verification,"start-test",2,&WebBindingDirectory::open(&next_dir).unwrap(),&new_runtime).unwrap();
    private_web_mode_on(&verification,"start-test",2).unwrap();
    verification.commit().unwrap();
    assert_eq!(connection.query_row("SELECT attempt_count FROM sentinel_scans",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    assert_eq!(connection.query_row("SELECT status FROM sentinel_scan_attempts WHERE attempt_number=1",[],|r|r.get::<_,String>(0)).unwrap(),"cancelled");
    assert_eq!(connection.query_row("SELECT binding_tag FROM native_web_dispatch_bindings WHERE attempt_number=1",[],|r|r.get::<_,Vec<u8>>(0)).unwrap(),old_binding);
    assert_ne!(connection.query_row("SELECT binding_tag FROM native_web_dispatch_bindings WHERE attempt_number=2",[],|r|r.get::<_,Vec<u8>>(0)).unwrap(),old_binding);
    assert_eq!(fs::read(work_dir.join("task.json")).unwrap(),old_task,"the old attempt's frozen task is preserved byte-for-byte");
    assert!(!old_task.is_empty());
    assert!(close_unclaimed_web_attempt(&path,"start-test",1,true).is_err(),"stale UI cannot close the newer attempt");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

// The explicit new start after a closure continues the closed attempt
// (`cancelled` retries only incomplete targets), so it must first re-verify the
// parent's private Web mode. A tampered parent task.json is refused before any
// attempt is allocated, and nothing is changed.
#[test]
fn web_closure_continuation_refuses_a_tampered_parent_task() {
    let (root,path,work_dir,mut connection) = recovery_test_fixture();
    close_unclaimed_web_attempt(&path,"start-test",1,true).unwrap();
    fs::write(work_dir.join("task.json"),"changed configuration").unwrap();
    let before = web_closure_test_snapshot(&connection);
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
    let error = prepare_web_startup_in(&transaction,&root,"start-test",WebStartMode::Retry).err().unwrap();
    assert_eq!(error,"web_mode_task_invalid");
    drop(transaction);
    assert_eq!(web_closure_test_snapshot(&connection),before);
    assert!(!work_dir.parent().unwrap().join("attempt-0002").exists());
    assert_eq!(connection.query_row("SELECT attempt_count FROM sentinel_scans",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_closure_requires_confirmation_exact_attempt_and_both_ownership_locks() {
    let (root,path,_,connection) = recovery_test_fixture();
    let before = web_start_snapshot(&connection);
    for (attempt,confirmed) in [(1,false),(0,true),(-1,true),(2,true),(i64::MAX,true)] {
        assert!(close_unclaimed_web_attempt(&path,"start-test",attempt,confirmed).is_err());
    }
    let lifecycle = claim_scan_control(&path,"start-test").unwrap();
    assert!(close_unclaimed_web_attempt(&path,"start-test",1,true).is_err());
    drop(lifecycle);
    let branch = claim_native_invocation(&path,"start-test",1,"branch","web").unwrap();
    assert!(close_unclaimed_web_attempt(&path,"start-test",1,true).is_err());
    drop(branch);
    assert_eq!(web_start_snapshot(&connection),before);
    close_unclaimed_web_attempt(&path,"start-test",1,true).unwrap();
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

fn web_closure_test_snapshot(connection: &rusqlite::Connection) -> Vec<Vec<String>> {
    let mut snapshot = web_start_snapshot(connection);
    for table in ["native_branch_dispatches","native_web_dispatch_bindings","native_web_attempt_closures"] {
        let mut statement = connection.prepare(&format!("SELECT * FROM {table} ORDER BY 1,2")).unwrap();
        let count = statement.column_count();
        snapshot.push(statement.query_map([],|r| (0..count).map(|i|r.get::<_,rusqlite::types::Value>(i).map(|v|format!("{v:?}")))
            .collect::<Result<Vec<_>,_>>().map(|row|format!("{row:?}"))).unwrap().collect::<Result<Vec<_>,_>>().unwrap());
    }
    snapshot
}

#[test]
fn web_closure_receipt_survives_legacy_progress_sync_and_cannot_be_reactivated() {
    let (root,path,_,connection) = recovery_test_fixture();
    let receipt = close_unclaimed_web_attempt(&path,"start-test",1,true).unwrap();
    let closed = web_closure_test_snapshot(&connection);
    sync_sentinel_attempt(&connection,"start-test");
    assert_eq!(web_closure_test_snapshot(&connection),closed,"history synchronization cannot rewrite explicit closure");
    sentinel_scan_update(&path,"start-test","scanning","late generic progress");
    assert_eq!(web_closure_test_snapshot(&connection),closed,"late generic progress cannot revive a closed attempt");
    assert_eq!(close_unclaimed_web_attempt(&path,"start-test",1,true).unwrap(),receipt);
    connection.execute_batch("UPDATE sentinel_scans SET status='scanning'; UPDATE native_scan_branches SET status='pending';").unwrap();
    assert!(!native_dispatch_attempt_eligible(&connection,"start-test",1,"web").unwrap(),"closure receipt independently denies reactivation");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_closure_rejects_claimed_unknown_unsupported_or_progressed_attempts() {
    for sql in [
        "UPDATE native_branch_dispatches SET claim_id='owned',claimed_at='2026-09-26'",
        "DELETE FROM native_branch_dispatches", "DELETE FROM native_web_dispatch_bindings",
        "UPDATE sentinel_scans SET scan_type='greybox'", "UPDATE sentinel_scans SET source_path='/source'",
        "UPDATE sentinel_scans SET llm_requests=1", "UPDATE sentinel_scans SET total_tokens=1",
        "UPDATE sentinel_scan_attempts SET llm_requests_delta=1", "UPDATE sentinel_scan_attempts SET status='failed'",
        "UPDATE sentinel_scan_attempts SET stop_reason='already stopped'", "UPDATE sentinel_targets SET status='completed'",
        "UPDATE sentinel_scans SET status='paused'", "UPDATE projects SET status='archived'",
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('start-test')",
        "INSERT INTO environment_preparation_lease(singleton,owner) VALUES(1,'test')",
        "INSERT INTO native_scan_branches(scan_id,attempt_number,branch) VALUES('start-test',1,'source')",
        "INSERT INTO sentinel_processes(scan_id,process_id) VALUES('start-test',999999)",
        "INSERT INTO agent_runs(id,scan_id,attempt_number) VALUES('existing-run','start-test',1)",
        "INSERT INTO analyzer_container_receipts(receipt_id,invocation_key,scan_id,attempt_number,purpose,container_name,owner_token,create_args_json) VALUES('r','k','start-test',1,'test','fixture-only-container','owner','[]')",
        "INSERT INTO agent_gap_followups(scan_id,request_id,request_hash,source_scan_id,assessment_message_id,source_hash,source_preview_json) VALUES('start-test','r','h','s','a','h','{}')",
        "INSERT INTO agent_authorization_controls(scan_id,attempt_number,target_url,contract_key,method,owner_object_url,tester_control_url,object_query_key,owner_object_value,tester_object_value,response_object_pointer,owner_identity,tester_identity) VALUES('start-test',1,'https://start.example.test/one','x','GET','x','y','id','x','y','/id','a','b')",
    ] {
        let (root,path,_,connection) = recovery_test_fixture();
        connection.execute_batch(sql).unwrap();
        let before = web_closure_test_snapshot(&connection);
        assert!(close_unclaimed_web_attempt(&path,"start-test",1,true).is_err(),"{sql}");
        assert_eq!(web_closure_test_snapshot(&connection),before,"{sql}");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_closure_preserves_cumulative_cost_and_exposes_truthful_read_only_status() {
    let (root,path,_,connection) = recovery_test_fixture_with(|c| {
        c.execute_batch("UPDATE sentinel_scans SET llm_requests=9,total_tokens=70,input_tokens=40,output_tokens=30,cached_tokens=20;").unwrap();
    });
    let status = native_scan_status(&connection,"start-test").unwrap();
    assert_eq!(status["branches"][0]["dispatch"]["manualClosureAvailable"],true);
    let receipt = close_unclaimed_web_attempt(&path,"start-test",1,true).unwrap();
    let before = web_closure_test_snapshot(&connection);
    for _ in 0..3 {
        let status = native_scan_status(&connection,"start-test").unwrap();
        assert_eq!(status["status"],"cancelled");
        assert_eq!(status["stopDiagnostic"]["code"],WEB_CLOSURE_REASON);
        assert_eq!(status["branches"][0]["dispatch"]["state"],"never_claimed");
        assert_eq!(status["branches"][0]["dispatch"]["manualClosureAvailable"],false);
        assert_eq!(status["branches"][0]["dispatch"]["manualRecoveryAvailable"],false);
        assert_eq!(status["branches"][0]["report"]["closureId"],receipt["closureId"]);
    }
    assert_eq!(web_closure_test_snapshot(&connection),before);
    let budget: (i64,i64,i64,i64,i64) = connection.query_row("SELECT llm_requests,total_tokens,input_tokens,output_tokens,cached_tokens FROM sentinel_scans",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(budget,(9,70,40,30,20));
    let stamp: String = connection.query_row("SELECT updated_at FROM sentinel_scans",[],|r|r.get(0)).unwrap();
    assert_eq!(stamp.len(),19);
    assert_eq!(&stamp[10..11]," ","task sorting retains the existing SQLite local time format");
    assert!(chrono::DateTime::parse_from_rfc3339(receipt["closedAt"].as_str().unwrap()).is_ok());
    assert!(connection.execute("UPDATE native_web_attempt_closures SET id='rewritten'",[]).is_err());
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn web_closure_rolls_back_ignored_aborted_tampered_and_commit_failed_writes() {
    let mut faults = Vec::new();
    for (table,event) in [("native_web_attempt_closures","INSERT"),("native_scan_branches","UPDATE"),("sentinel_scan_attempts","UPDATE"),("sentinel_scans","UPDATE")] {
        for fault in ["IGNORE","ABORT,'injected'"] {
            faults.push(format!("CREATE TRIGGER closure_fault BEFORE {event} ON {table} BEGIN SELECT RAISE({fault}); END;"));
        }
    }
    for body in [
        "DELETE FROM native_web_attempt_closures;",
        "UPDATE native_web_attempt_closures SET id='replaced';",
        "UPDATE native_web_attempt_closures SET preservation_hash=printf('%064d',0);",
        "UPDATE native_branch_dispatches SET claim_id='unexpected',claimed_at='2026-09-26';",
        "UPDATE native_web_dispatch_bindings SET binding_tag=zeroblob(32);",
        "UPDATE sentinel_scans SET total_tokens=1;",
        "UPDATE sentinel_scan_attempts SET total_tokens_start=5;",
        "UPDATE sentinel_scan_attempts SET backend_plan_json='{}';",
        "UPDATE sentinel_scan_attempts SET stage='wrong';",
        "UPDATE native_scan_branches SET report_json='{}';",
        "UPDATE sentinel_targets SET url='https://outside.example.test';",
        "INSERT INTO sentinel_processes(scan_id,process_id) VALUES('start-test',999999);",
        "UPDATE projects SET status='archived';",
    ] {
        faults.push(format!("CREATE TRIGGER closure_fault AFTER UPDATE OF status ON sentinel_scans WHEN NEW.status='cancelled' BEGIN {body} END;"));
    }
    faults.push("CREATE TABLE closure_parent(id INTEGER PRIMARY KEY); CREATE TABLE closure_child(id INTEGER REFERENCES closure_parent(id) DEFERRABLE INITIALLY DEFERRED); CREATE TRIGGER closure_fault AFTER UPDATE OF status ON sentinel_scans BEGIN INSERT INTO closure_child VALUES(1); END;".into());
    for fault in faults {
        let (root,path,_,connection) = recovery_test_fixture();
        connection.execute_batch(&fault).unwrap();
        let before = web_closure_test_snapshot(&connection);
        assert!(close_unclaimed_web_attempt(&path,"start-test",1,true).is_err(),"{fault}");
        assert_eq!(web_closure_test_snapshot(&connection),before,"{fault}");
        assert!(claim_scan_control(&path,"start-test").is_ok());
        assert!(claim_native_invocation(&path,"start-test",1,"branch","web").is_ok());
        connection.execute_batch("DROP TRIGGER closure_fault").unwrap();
        close_unclaimed_web_attempt(&path,"start-test",1,true).unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn web_closure_and_original_dispatch_or_manual_recovery_have_one_winner() {
    for manual in [false,true] {
        let (root,path,work_dir,connection) = recovery_test_fixture();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (close_path,start) = (path.clone(),barrier.clone());
        let close = thread::spawn(move || {start.wait(); close_unclaimed_web_attempt(&close_path,"start-test",1,true)});
        let (dispatch_path,dispatch_root) = (path.clone(),root.clone());
        let dispatch = thread::spawn(move || {
            barrier.wait();
            if manual {
                claim_web_recovery(&dispatch_path,"start-test",1,true,|c|recovery_test_reconstruct(c,&dispatch_root))
                    .map(|owned|owned.guard)
            } else {
                NativeBranchGuard::claim_with_preflight(&dispatch_path,"start-test",1,"web",|c|
                    verify_web_dispatch_binding_in(c,"start-test",1,&WebBindingDirectory::open(&work_dir)?,&binding_test_runtime()))
            }
        });
        let closed = close.join().unwrap();
        let mut dispatched = dispatch.join().unwrap();
        assert_ne!(closed.is_ok(),dispatched.is_ok(),"one winner: manual={manual}");
        if let Ok(guard) = &mut dispatched {guard.disarm();}
        drop(dispatched);
        if closed.is_ok() {
            assert!(claim_web_recovery(&path,"start-test",1,true,|c|recovery_test_reconstruct(c,&root)).is_err());
        } else {
            assert!(close_unclaimed_web_attempt(&path,"start-test",1,true).is_err());
        }
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
