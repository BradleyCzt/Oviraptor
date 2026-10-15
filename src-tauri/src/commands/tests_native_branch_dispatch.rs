fn dispatch_fixture() -> (PathBuf, PathBuf, rusqlite::Connection) {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["web", "source"]).unwrap();
    (root, path, connection)
}

fn dispatch_row(connection: &rusqlite::Connection) -> (String, String, String, String) {
    connection.query_row(
        "SELECT s.status,b.status,d.claim_id,d.claimed_at FROM sentinel_scans s
         JOIN native_scan_branches b ON b.scan_id=s.id
         JOIN native_branch_dispatches d ON d.scan_id=b.scan_id AND d.attempt_number=b.attempt_number AND d.branch=b.branch
         WHERE b.branch='web' AND b.attempt_number=1", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
    ).unwrap()
}

#[test]
fn native_dispatch_receipt_survives_owner_release_and_refuses_replay() {
    let (root, path, connection) = dispatch_fixture();
    assert_eq!(dispatch_row(&connection), ("scanning".into(),"pending".into(),String::new(),String::new()));
    let mut owner = NativeBranchGuard::claim(&path,"source-regression",1,"web").unwrap();
    let claimed = dispatch_row(&connection);
    assert!(!claimed.2.is_empty());
    assert!(!claimed.3.is_empty());
    owner.disarm();
    drop(owner);
    let lock = claim_native_invocation(&path,"source-regression",1,"branch","web").unwrap();
    drop(lock);
    assert!(NativeBranchGuard::claim(&path,"source-regression",1,"web").err().unwrap().contains("already_claimed"));
    assert_eq!(dispatch_row(&connection), claimed, "denied replay must not construct a failure guard");
    let before = connection.total_changes();
    let status = native_scan_status(&connection,"source-regression").unwrap();
    assert_eq!(connection.total_changes(),before);
    let web = status["branches"].as_array().unwrap().iter().find(|b| b["branch"]=="web").unwrap();
    assert_eq!(web["dispatch"]["state"],"claimed");
    assert_eq!(web["dispatch"]["automaticReplayAllowed"],false);
    assert!(!status.to_string().contains(&claimed.2),"internal ownership identifiers are not UI capabilities");
    let mut source = NativeBranchGuard::claim(&path,"source-regression",1,"source").unwrap();
    source.disarm();
    drop(source);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_dispatch_legacy_rows_are_not_backfilled_as_safe_to_replay() {
    let (root,path,connection) = dispatch_fixture();
    connection.execute("DELETE FROM native_branch_dispatches",[]).unwrap();
    db::initialize(&root.join("app")).unwrap();
    assert_eq!(connection.query_row("SELECT count(*) FROM native_branch_dispatches",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert!(NativeBranchGuard::claim(&path,"source-regression",1,"web").is_err());
    let status = native_scan_status(&connection,"source-regression").unwrap();
    assert!(status["branches"].as_array().unwrap().iter().all(|b|b["dispatch"]["state"]=="legacy_unknown"));
    assert_eq!(status["status"],"scanning");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_dispatch_registration_faults_roll_back_branch_set() {
    for sql in [
        "CREATE TRIGGER fault BEFORE INSERT ON native_branch_dispatches BEGIN SELECT RAISE(ABORT,'fault'); END;",
        "CREATE TRIGGER fault BEFORE INSERT ON native_branch_dispatches BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fault AFTER INSERT ON native_branch_dispatches BEGIN DELETE FROM native_branch_dispatches; END;",
        "CREATE TRIGGER fault AFTER INSERT ON native_branch_dispatches BEGIN UPDATE native_branch_dispatches SET claim_id='unexpected',claimed_at='unexpected'; END;",
        "CREATE TRIGGER fault AFTER INSERT ON native_branch_dispatches BEGIN UPDATE native_scan_branches SET status='completed'; END;",
    ] {
        let (root,_,path) = source_regression_fixture();
        let connection = db::open(&path).unwrap();
        connection.execute_batch(sql).unwrap();
        assert!(register_native_branches(&connection,"source-regression",1,&["web","source"]).is_err(),"{sql}");
        for table in ["native_scan_branches","native_branch_dispatches"] {
            assert_eq!(connection.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get::<_,i64>(0)).unwrap(),0);
        }
        connection.execute_batch("DROP TRIGGER fault").unwrap();
        register_native_branches(&connection,"source-regression",1,&["web","source"]).unwrap();
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn native_dispatch_web_startup_commits_receipt_with_attempt_or_rolls_back_all() {
    for fault in [
        "BEFORE INSERT ON native_branch_dispatches BEGIN SELECT RAISE(ABORT,'fault'); END;",
        "BEFORE INSERT ON native_branch_dispatches BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER INSERT ON native_branch_dispatches BEGIN DELETE FROM native_branch_dispatches; END;",
        "AFTER INSERT ON native_branch_dispatches BEGIN UPDATE native_branch_dispatches SET claim_id='unexpected',claimed_at='unexpected'; END;",
    ] {
        let (root,_,mut connection) = web_start_fixture();
        let before = web_start_snapshot(&connection);
        connection.execute_batch(&format!("CREATE TRIGGER fault {fault}")).unwrap();
        assert!(run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).is_err(),"{fault}");
        assert_eq!(web_start_snapshot(&connection),before);
        assert!(!root.join("agent-jobs/start-test/attempt-0001").exists());
        assert_eq!(connection.query_row("SELECT count(*) FROM native_branch_dispatches",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        connection.execute_batch("DROP TRIGGER fault").unwrap();
        run_web_start_fixture(&mut connection,&root,WebStartMode::Confirm).unwrap();
        let row: (i64,String,String) = connection.query_row(
            "SELECT d.attempt_number,d.claim_id,d.claimed_at FROM native_branch_dispatches d
             JOIN sentinel_scan_attempts a ON a.scan_id=d.scan_id AND a.attempt_number=d.attempt_number
             WHERE d.scan_id='start-test' AND d.branch='web'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).unwrap();
        assert_eq!(row,(1,String::new(),String::new()));
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn native_dispatch_claim_faults_do_not_release_effects_or_fail_branch() {
    for sql in [
        "CREATE TRIGGER fault BEFORE UPDATE ON native_branch_dispatches BEGIN SELECT RAISE(ABORT,'fault'); END;",
        "CREATE TRIGGER fault BEFORE UPDATE ON native_branch_dispatches BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fault AFTER UPDATE ON native_branch_dispatches BEGIN DELETE FROM native_branch_dispatches; END;",
        "CREATE TRIGGER fault AFTER UPDATE ON native_branch_dispatches BEGIN UPDATE native_branch_dispatches SET claim_id='tampered'; END;",
        "CREATE TRIGGER fault AFTER UPDATE ON native_branch_dispatches BEGIN UPDATE native_branch_dispatches SET claimed_at='tampered'; END;",
        "CREATE TRIGGER fault AFTER UPDATE ON native_branch_dispatches BEGIN UPDATE sentinel_scans SET attempt_count=2; END;",
        "CREATE TRIGGER fault AFTER UPDATE ON native_branch_dispatches BEGIN UPDATE native_scan_branches SET status='completed'; END;",
        "CREATE TRIGGER fault AFTER UPDATE ON native_branch_dispatches BEGIN INSERT INTO environment_preparation_lease(singleton,owner) VALUES(1,'fixture'); END;",
        "CREATE TABLE dispatch_parent(id INTEGER PRIMARY KEY); CREATE TABLE dispatch_child(id INTEGER REFERENCES dispatch_parent(id) DEFERRABLE INITIALLY DEFERRED);
         CREATE TRIGGER fault AFTER UPDATE ON native_branch_dispatches BEGIN INSERT INTO dispatch_child VALUES(9); END;",
    ] {
        let (root,path,connection) = dispatch_fixture();
        let before = dispatch_row(&connection);
        connection.execute_batch(sql).unwrap();
        assert!(NativeBranchGuard::claim(&path,"source-regression",1,"web").is_err(),"{sql}");
        assert_eq!(dispatch_row(&connection),before,"{sql}");
        connection.execute_batch("DROP TRIGGER fault").unwrap();
        let mut owner = NativeBranchGuard::claim(&path,"source-regression",1,"web").unwrap();
        owner.disarm();
        drop(owner);
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn native_dispatch_rechecks_attempt_project_and_environment_before_claim() {
    for sql in [
        "UPDATE sentinel_scans SET status='paused';",
        "UPDATE sentinel_scans SET attempt_count=2;",
        "UPDATE sentinel_scans SET scan_type='web';",
        "INSERT INTO projects(id,name) VALUES(99,'deleted'); UPDATE sentinel_scans SET scan_type='web',project_id=99; DELETE FROM projects WHERE id=99;",
        "INSERT INTO projects(id,name,status) VALUES(99,'archived','archived'); UPDATE sentinel_scans SET project_id=99;",
        "INSERT INTO environment_preparation_lease(singleton,owner) VALUES(1,'fixture');",
        "INSERT INTO sentinel_deleted_scans(scan_id) VALUES('source-regression');",
    ] {
        let (root,path,connection) = dispatch_fixture();
        connection.execute_batch(sql).unwrap();
        assert!(NativeBranchGuard::claim(&path,"source-regression",1,"web").is_err(),"{sql}");
        assert!(dispatch_row(&connection).2.is_empty());
        assert_eq!(dispatch_row(&connection).1,"pending");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

// Invoked only by the parent test below. No target/model/browser traffic.
#[test]
fn native_dispatch_process_crash_probe() {
    use std::io::{Read, Write};
    let Some(path) = std::env::var_os("OVIRAPTOR_DISPATCH_CRASH_DB") else { return; };
    let phase = std::env::var("OVIRAPTOR_DISPATCH_CRASH_PHASE").unwrap();
    let _owner = if phase == "before_claim" { None } else {
        Some(NativeBranchGuard::claim(Path::new(&path),"source-regression",1,"web").unwrap())
    };
    if phase == "after_local_effect" {
        let marker = std::env::var_os("OVIRAPTOR_DISPATCH_CRASH_MARKER").unwrap();
        fs::write(marker,b"one local effect").unwrap();
    }
    println!("dispatch-crash-ready");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
    panic!("parent should kill this process without running guard Drop");
}

#[test]
fn native_dispatch_real_process_kill_preserves_before_and_after_claim_distinction() {
    use std::io::BufRead;
    for phase in ["before_claim","after_claim","after_local_effect"] {
        let (root,path,connection) = dispatch_fixture();
        let marker = root.join("local-effect");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact","scanning::tests::native_dispatch_process_crash_probe","--nocapture"])
            .env("OVIRAPTOR_DISPATCH_CRASH_DB",&path)
            .env("OVIRAPTOR_DISPATCH_CRASH_PHASE",phase)
            .env("OVIRAPTOR_DISPATCH_CRASH_MARKER",&marker)
            .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped())
            .spawn().unwrap();
        // Child::wait closes child.stdin before waiting. Keep a separate writer
        // alive until termination so EOF/panic cannot race SIGKILL and count as
        // a successful no-Drop crash probe.
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (ready_tx,ready_rx) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
                if line.unwrap().contains("dispatch-crash-ready") {
                    let _ = ready_tx.send(());
                    return;
                }
            }
        });
        let ready = ready_rx.recv_timeout(Duration::from_secs(20));
        let killed = child.kill();
        let exit = child.wait().unwrap();
        drop(input);
        reader.join().unwrap();
        assert!(ready.is_ok(),"child did not reach {phase}: {ready:?}");
        assert!(killed.is_ok(),"child kill failed during {phase}: {killed:?}");
        assert!(!exit.success());
        #[cfg(unix)]
        assert_eq!(exit.signal(),Some(libc::SIGKILL),"{phase} must terminate without Rust unwinding");
        let row = dispatch_row(&connection);
        assert_eq!((&row.0,&row.1),(&"scanning".to_string(),&"pending".to_string()));
        assert_eq!(marker.exists(),phase=="after_local_effect");
        if phase=="before_claim" {
            assert!(row.2.is_empty());
            let mut owner = NativeBranchGuard::claim(&path,"source-regression",1,"web").unwrap();
            owner.disarm();
            drop(owner);
        } else {
            assert!(!row.2.is_empty());
            assert!(NativeBranchGuard::claim(&path,"source-regression",1,"web").is_err());
            assert_eq!(dispatch_row(&connection),row);
        }
        if marker.exists() { assert_eq!(fs::read(&marker).unwrap(),b"one local effect"); }
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}
