// Test-only subprocess barrier after all deletion postconditions, before COMMIT.
pub(super) fn paid_deletion_commit_barrier(path: &Path, scan: &str) -> Result<(), String> {
    let Ok(bound) = std::env::var("OVIRAPTOR_PAID_DELETE_FIXTURE_BINDING") else {
        return Ok(());
    };
    let expected = json!([path.to_string_lossy(), scan]).to_string();
    if bound != expected
        || std::env::var("OVIRAPTOR_PAID_DELETE_FIXTURE_MODE").as_deref() != Ok("before_commit")
    {
        return Ok(());
    }
    let marker = path.with_extension("deletion-before-commit");
    fs::write(marker, b"verified original transaction").map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    Err("paid_deletion_fixture_commit_timeout".into())
}
fn paid_scan_deletion_subprocess_entry() -> bool {
    let Ok(bound) = std::env::var("OVIRAPTOR_PAID_DELETE_FIXTURE_BINDING") else {
        return false;
    };
    let args: Vec<String> = serde_json::from_str(&bound).unwrap();
    assert_eq!(args.len(), 2);
    let path = Path::new(&args[0]);
    delete_sentinel_scan_inner(path, &args[1]).unwrap();
    // Deliberately lose the successful IPC response after the durable commit.
    assert_ne!(
        std::env::var("OVIRAPTOR_PAID_DELETE_FIXTURE_MODE").as_deref(),
        Ok("lost_reply"),
        "test-only lost deletion response"
    );
    true
}
fn paid_scan_deletion_child(f: &WebModeFixture, mode: &str) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "scanning::agent_tests::scan_deletion_paid_single_actual_process_death_before_commit_rolls_back_then_retries",
            "--exact",
            "--test-threads=1",
        ])
        .env(
            "OVIRAPTOR_PAID_DELETE_FIXTURE_BINDING",
            json!([f.path.to_string_lossy(), f.scan]).to_string(),
        )
        .env("OVIRAPTOR_PAID_DELETE_FIXTURE_MODE", mode)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap()
}
#[test]
fn scan_deletion_paid_single_actual_process_death_before_commit_rolls_back_then_retries() {
    if paid_scan_deletion_subprocess_entry() {
        return;
    }
    let (f, context, root) = paid_single_deletion_fixture();
    let db = db::open(&f.path).unwrap();
    let before = single_finally_physical(&db);
    let file = fs::read(context.target_dir.join("frontend-evidence.json")).unwrap();
    let mut child = paid_scan_deletion_child(&f, "before_commit");
    let marker = f.path.with_extension("deletion-before-commit");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !marker.exists() && Instant::now() < deadline {
        assert!(
            child.try_wait().unwrap().is_none(),
            "child failed before verified precommit boundary"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    if !marker.exists() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("precommit marker missing");
    }
    // Parent sees only original rows while actual child holds the uncommitted delete.
    assert_eq!(single_finally_physical(&db), before);
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    assert_eq!(single_finally_physical(&db), before);
    assert_eq!(
        fs::read(context.target_dir.join("frontend-evidence.json")).unwrap(),
        file
    );
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, &root).is_ok()
    );
    delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
    assert!(crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &f.scan).unwrap());
    assert_eq!(
        fs::read(context.target_dir.join("frontend-evidence.json")).unwrap(),
        file
    );
}
#[test]
fn scan_deletion_paid_single_actual_lost_commit_reply_retries_without_new_audit_or_cost() {
    if paid_scan_deletion_subprocess_entry() {
        return;
    }
    let (f, context, root) = paid_single_deletion_fixture();
    let db = db::open(&f.path).unwrap();
    let fees = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid",
        [],
    )
    .unwrap();
    let file = fs::read(context.target_dir.join("frontend-evidence.json")).unwrap();
    let mut child = paid_scan_deletion_child(&f, "lost_reply");
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("committed child timeout");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!status.success());
    assert!(crate::agent_runtime::deleted_scan_audit::verify_deleted(&db, &f.scan).unwrap());
    let after = single_finally_physical(&db);
    delete_sentinel_scan_inner(&f.path, &f.scan).unwrap();
    assert_eq!(single_finally_physical(&db), after);
    assert!(
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            "SELECT rowid,* FROM agent_budget_entries ORDER BY rowid",
            []
        )
        .unwrap()
            == fees
    );
    assert_eq!(
        fs::read(context.target_dir.join("frontend-evidence.json")).unwrap(),
        file
    );
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, &root)
            .is_err()
    );
}
