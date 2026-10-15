// The parent test below supplies only its isolated fixture. Normal suite
// invocation performs zero work; it is not itself SIGKILL acceptance evidence.
#[test]
fn assignment_attempt_parent_ownership_process_probe() {
    use crate::agent_runtime::multi_agent::{
        lease::CoordinatorLease, supervisor::WorkerSupervisor,
    };
    use std::io::{Read, Write};
    let Some(path) = std::env::var_os("OVIRAPTOR_PARENT_OWNER_TEST_DB") else {
        return;
    };
    let run = std::env::var("OVIRAPTOR_PARENT_OWNER_TEST_ROOT").unwrap();
    let db = db::open(Path::new(&path)).unwrap();
    let lease = db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
        FROM agent_coordinator_leases WHERE root_run_id=?1", [&run], |r|Ok(CoordinatorLease {
        scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
    })).unwrap();
    let service = WorkerSupervisor::start(Path::new(&path), &lease);
    if std::env::var("OVIRAPTOR_PARENT_OWNER_TEST_MODE").unwrap() == "reject" {
        assert!(service.is_err());
        return;
    }
    let service = service.unwrap();
    service.check().unwrap();
    println!("parent-owner-ready");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
    panic!("test must SIGKILL without Rust cleanup");
}

#[cfg(unix)]
#[test]
fn assignment_attempt_parent_ownership_real_process_exclusion_and_sigkill_release() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    use std::{
        io::BufRead,
        os::unix::process::ExitStatusExt,
        process::{Command, Stdio},
    };
    let (root, context, lease, _) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let probe = || {
        let mut process = Command::new(std::env::current_exe().unwrap());
        process
            .args([
                "--exact",
                "scanning::agent_tests::assignment_attempt_parent_ownership_process_probe",
                "--nocapture",
            ])
            .env("OVIRAPTOR_PARENT_OWNER_TEST_DB", &context.db_path)
            .env("OVIRAPTOR_PARENT_OWNER_TEST_ROOT", &lease.root_run_id);
        process
    };
    let first = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let output = probe()
        .env("OVIRAPTOR_PARENT_OWNER_TEST_MODE", "reject")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    first.check().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(first);

    let mut process = probe()
        .env("OVIRAPTOR_PARENT_OWNER_TEST_MODE", "own")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = process.stdin.take().unwrap();
    let output = process.stdout.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in std::io::BufReader::new(output).lines() {
            if line.unwrap().contains("parent-owner-ready") {
                let _ = sender.send(());
                return;
            }
        }
    });
    let ready = receiver.recv_timeout(Duration::from_secs(20));
    let duplicate = if ready.is_ok() {
        Some(WorkerSupervisor::start(&context.db_path, &lease))
    } else {
        None
    };
    // Always terminate and join before assertions, including a failed probe.
    let killed = process.kill();
    let exit = process.wait().unwrap();
    drop(input);
    reader.join().unwrap();
    assert!(ready.is_ok());
    assert!(duplicate.unwrap().is_err());
    assert!(killed.is_ok());
    assert_eq!(exit.signal(), Some(libc::SIGKILL));
    assert!(super::tests::application_table_snapshot(&db) == before);
    let recovered = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    recovered.check().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(recovered);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
