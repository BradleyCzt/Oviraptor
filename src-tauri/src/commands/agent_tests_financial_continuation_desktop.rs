// Opt-in exact-test driver. All authority is born inside a freshly validated
// private desktop profile; no paid database or signed work tree is copied.
fn desktop_financial_seed_write(path: &Path, value: &JsonValue, replace: bool) {
    use std::io::Write;
    let output = if replace {
        path.with_extension(format!("{}.tmp", Uuid::new_v4()))
    } else {
        path.to_path_buf()
    };
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .unwrap();
    }
    file.write_all(&serde_json::to_vec_pretty(value).unwrap())
        .unwrap();
    file.sync_all().unwrap();
    if replace {
        fs::rename(output, path).unwrap();
    }
}

#[test]
fn desktop_financial_continuation_acceptance_seed() {
    use sha2::{Digest, Sha256};
    use std::time::{Duration, Instant};
    const DRIVER_ENV: &str = "OVIRAPTOR_DESKTOP_FINANCIAL_SEED";
    // Full normal suites execute this test as a no-op. The explicit driver
    // requires both opt-ins and an exact filter; no ignore/skipped test exists.
    let Some(enabled) = std::env::var_os(DRIVER_ENV) else {
        return;
    };
    assert_eq!(
        enabled,
        std::ffi::OsString::from("1"),
        "desktop seed enable flag must be exactly 1"
    );
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.iter().any(|arg| arg == "--exact")
            && args.iter().any(|arg| arg
                == "scanning::agent_tests::desktop_financial_continuation_acceptance_seed"),
        "desktop seed must be selected as one exact test"
    );
    let acceptance = crate::startup_profile::acceptance_root()
        .unwrap()
        .expect("OVIRAPTOR_ACCEPTANCE_ROOT is also required");
    let (data, _) = crate::startup_profile::isolated_directories(&acceptance).unwrap();
    assert!(
        fs::read_dir(&data).unwrap().next().is_none(),
        "desktop acceptance data must be empty; existing DBs must never be adopted"
    );
    let manifest = acceptance.join("a02-desktop-manifest.json");
    let seen_file = acceptance.join("a02-desktop-seen.json");
    let stop_file = acceptance.join("a02-desktop-stop");
    for path in [&manifest, &seen_file, &stop_file] {
        assert_eq!(
            fs::symlink_metadata(path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound,
            "desktop seed control path must not already exist"
        );
    }
    let _real = RealSpecialistTransport::enter();
    let (mut f, seen, confirmation_hash) =
        financial_issuance_fixture_options("desktop-financial", (200_000, 20), Some(&data), false);
    // Only this validated driver retains the original private DB/work tree.
    // The source is already genuinely paused and its supervisor has exited.
    f.f.preserve = true;
    assert!(f.parent.is_none());
    assert_eq!(f.f.root, data);
    assert_eq!(f.context.db_path, data.join("oviraptor.sqlite3"));
    assert!(f
        .context
        .target_dir
        .canonicalize()
        .unwrap()
        .starts_with(&data));
    let db = db::open(&f.context.db_path).unwrap();
    let report = native_root_cost_reconciliations_for_attempt(&db, &f.context.scan_id, 1).unwrap();
    assert_eq!(report["items"].as_array().unwrap().len(), 1);
    assert!(report["receipts"].as_array().unwrap().is_empty());
    assert_eq!(report["items"][0]["confirmationHash"], confirmation_hash);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let state: String = db
        .query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&f.context.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "paused");
    let deadline: String = db.query_row(
        "SELECT strftime('%Y-%m-%d %H:%M:%f',o.started_at,printf('+%f seconds',l.hard_limit/1000.0))
        FROM agent_budget_clock_origins o JOIN agent_budget_limits l ON l.root_run_id=o.root_run_id
        AND l.dimension='wall_time_ms' WHERE o.root_run_id=?1", [&f.actor.root_run_id], |r| r.get(0)).unwrap();
    let digest = |path: &Path| format!("{:x}", Sha256::digest(fs::read(path).unwrap()));
    let wal = f.context.db_path.with_extension("sqlite3-wal");
    let wal_hash = match fs::read(wal) {
        Ok(bytes) => Some(format!("{:x}", Sha256::digest(bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("private acceptance WAL unavailable: {error}"),
    };
    let snapshot = json!({
        "schema":1, "acceptanceRoot":acceptance, "dataRoot":data,
        "dbPath":f.context.db_path, "dbSha256":digest(&f.context.db_path), "dbWalSha256":wal_hash,
        "databaseHashScope":"main_file_and_optional_wal_at_manifest",
        "scanId":f.context.scan_id, "attemptNumber":1, "rootRunId":f.actor.root_run_id,
        "targetKey":f.context.target_url, "targetDir":f.context.target_dir,
        "frontendEvidenceHash":digest(&f.context.target_dir.join("frontend-evidence.json")),
        "taskJsonHash":digest(&f.f.work.join("task.json")),
        "confirmationHash":confirmation_hash, "invoiceHash":report["items"][0]["invoiceHash"],
        "callId":report["items"][0]["callId"], "actualUsage":report["items"][0]["usage"],
        "modelEndpoint":f.context.environment.api_base, "model":f.context.environment.llm,
        "sourceStatus":"paused", "costConfirmationPending":true,
        "newAttemptIssued":false, "automaticResumeAllowed":false,
        "originalDeadline":deadline, "originalCoordinatorExpiry":f.actor.lease_expires_at,
        "stopFile":stop_file, "seenFile":seen_file, "maximumProviderLifetimeSeconds":900
    });
    desktop_financial_seed_write(&manifest, &snapshot, false);
    drop(db);
    println!("desktop_financial_seed_manifest={}", manifest.display());
    let started = Instant::now();
    let maximum = Duration::from_secs(900);
    loop {
        let stopped = fs::symlink_metadata(&stop_file).is_ok();
        let timeout = started.elapsed() >= maximum;
        desktop_financial_seed_write(
            &seen_file,
            &json!({
                "schema":1, "scanId":f.context.scan_id, "sdkRequestCount":seen.lock().unwrap().len(),
                "providerActive":!stopped && !timeout, "stoppedByFile":stopped,
                "maximumLifetimeElapsed":timeout, "elapsedSeconds":started.elapsed().as_secs()
            }),
            true,
        );
        if stopped || timeout {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // Seen is the existing HTTP fixture's lifetime owner. Releasing it stops
    // listening; f.preserve leaves the original private evidence intact.
    drop(seen);
}
