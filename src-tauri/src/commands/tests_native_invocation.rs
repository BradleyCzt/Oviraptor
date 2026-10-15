#[test]
fn native_invocation_duplicate_branch_has_no_failure_guard() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["web", "source"]).unwrap();
    let owner = NativeBranchGuard::claim(&path, "source-regression", 1, "web").unwrap();
    assert!(NativeBranchGuard::claim(&path, "source-regression", 1, "web").is_err());
    let state: (String, String) = connection.query_row(
        "SELECT s.status,b.status FROM sentinel_scans s JOIN native_scan_branches b ON b.scan_id=s.id WHERE b.branch='web'",
        [], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!(state, ("scanning".into(), "pending".into()));
    let source = NativeBranchGuard::claim(&path, "source-regression", 1, "source").unwrap();
    finish_native_branch(
        &path,
        "source-regression",
        1,
        "web",
        "completed",
        "done",
        &json!({}),
    )
    .unwrap();
    drop(owner);
    // Source is still active, but a completed Web branch must not restart.
    assert!(NativeBranchGuard::claim(&path, "source-regression", 1, "web").is_err());
    finish_native_branch(
        &path,
        "source-regression",
        1,
        "source",
        "completed",
        "done",
        &json!({}),
    )
    .unwrap();
    drop(source);
    let status: String = connection
        .query_row("SELECT status FROM sentinel_scans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "completed");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_invocation_racing_branch_admission_has_one_owner() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["web"]).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                let mut result = NativeBranchGuard::claim(&path, "source-regression", 1, "web");
                barrier.wait(); // Keep the winner alive until all contenders tried.
                if let Ok(owner) = &mut result {
                    owner.disarm();
                }
                result.is_ok()
            })
        })
        .collect();
    assert_eq!(
        threads
            .into_iter()
            .map(|t| usize::from(t.join().unwrap()))
            .sum::<usize>(),
        1
    );
    let status: String = connection
        .query_row("SELECT status FROM native_scan_branches", [], |r| r.get(0))
        .unwrap();
    assert_eq!(status, "pending", "losers must not run failure cleanup");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn native_invocation_stale_attempt_guard_cannot_fail_new_owner() {
    let (root, _, path) = source_regression_fixture();
    let connection = db::open(&path).unwrap();
    register_native_branches(&connection, "source-regression", 1, &["web"]).unwrap();
    let old = NativeBranchGuard::claim(&path, "source-regression", 1, "web").unwrap();
    connection
        .execute("UPDATE sentinel_scans SET attempt_count=2", [])
        .unwrap();
    register_native_branches(&connection, "source-regression", 2, &["web"]).unwrap();
    let current = NativeBranchGuard::claim(&path, "source-regression", 2, "web").unwrap();
    drop(old);
    let status: String = connection
        .query_row(
            "SELECT status FROM native_scan_branches WHERE attempt_number=2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "pending");
    assert!(NativeBranchGuard::claim(&path, "source-regression", 1, "web").is_err());
    finish_native_branch(
        &path,
        "source-regression",
        2,
        "web",
        "completed",
        "done",
        &json!({}),
    )
    .unwrap();
    drop(current);
    fs::remove_dir_all(root).unwrap();
}

// Run by a real second test process; ordinary suite invocation is a no-op.
#[test]
fn native_invocation_cross_process_probe() {
    let Some(path) = std::env::var_os("OVIRAPTOR_INVOCATION_TEST_DB") else {
        return;
    };
    assert!(claim_native_invocation(
        Path::new(&path),
        "source-regression",
        1,
        "target",
        "https://example.invalid"
    )
    .is_err());
}

#[test]
fn native_invocation_fences_processes_and_releases_on_drop() {
    let (root, _, path) = source_regression_fixture();
    let owner = claim_native_invocation(
        &path,
        "source-regression",
        1,
        "target",
        "https://example.invalid",
    )
    .unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "scanning::tests::native_invocation_cross_process_probe",
            "--nocapture",
        ])
        .env("OVIRAPTOR_INVOCATION_TEST_DB", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    drop(owner);
    let new_owner = claim_native_invocation(
        &path,
        "source-regression",
        1,
        "target",
        "https://example.invalid",
    )
    .unwrap();
    drop(new_owner);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn native_invocation_canonical_database_alias_cannot_bypass_owner() {
    let (root, _, path) = source_regression_fixture();
    let alias = root.join("alias.db");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    let owner = claim_native_invocation(&path, "s", 1, "target", "url").unwrap();
    assert!(claim_native_invocation(&alias, "s", 1, "target", "url").is_err());
    // Separate kinds, attempts, targets and scans must remain independent.
    for (scan, attempt, kind, target) in [
        ("other", 1, "target", "url"),
        ("s", 2, "target", "url"),
        ("s", 1, "branch", "url"),
        ("s", 1, "target", "other"),
    ] {
        assert!(claim_native_invocation(&path, scan, attempt, kind, target).is_ok());
    }
    drop(owner);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn native_invocation_rejects_symlink_directory_without_touching_database() {
    let (root, _, path) = source_regression_fixture();
    let other = root.join("other");
    fs::create_dir(&other).unwrap();
    let mut directory_name = path.file_name().unwrap().to_os_string();
    directory_name.push(".invocations");
    std::os::unix::fs::symlink(&other, path.with_file_name(directory_name)).unwrap();
    let error = claim_native_invocation(&path, "s", 1, "target", "url").unwrap_err();
    assert_eq!(error, "native_invocation_directory_not_regular");
    assert_eq!(fs::read_dir(other).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}
