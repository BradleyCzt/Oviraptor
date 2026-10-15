// Only the parent process below supplies these temporary fixture arguments.
#[test]
fn assignment_attempt_reassignment_process_probe() {
    use crate::agent_runtime::multi_agent::{
        lease::CoordinatorLease, scheduler::ScheduledChild, specialist,
    };
    use std::io::{Read, Write};
    let Some(path) = std::env::var_os("OVIRAPTOR_WORKER_KILL_DB") else {
        return;
    };
    let phase = std::env::var("OVIRAPTOR_WORKER_KILL_PHASE").unwrap();
    let run = std::env::var("OVIRAPTOR_WORKER_KILL_RUN").unwrap();
    let path = Path::new(&path);
    let db = db::open(path).unwrap();
    let lease=db.query_row("SELECT c.scan_id,c.attempt_number,c.target_key,c.root_run_id,c.lease_epoch,c.fencing_token,c.lease_expires_at
        FROM agent_coordinator_leases c JOIN agent_runs r ON r.root_run_id=c.root_run_id WHERE r.id=?1",[&run],|r|Ok(CoordinatorLease {
        scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
    })).unwrap();
    let assignment = db
        .query_row(
            "SELECT assignment_id FROM agent_runs WHERE id=?1",
            [&run],
            |r| r.get(0),
        )
        .unwrap();
    let child = ScheduledChild {
        assignment_id: assignment,
        run_id: run,
        role: crate::agent_runtime::contract::AgentRole::SpaApiMapper,
    };
    match phase.as_str() {
        "before_claim" => {}
        "after_claim" => {
            assert!(matches!(
                specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap(),
                specialist::Start::Dispatch(_)
            ));
        }
        "after_receipt" => {
            let mut context =
                test_context(path, &lease.target_key, vec![AgentIdentity::anonymous()]);
            context.scan_id = lease.scan_id.clone();
            context.attempt_number = lease.attempt_number;
            context.run = Some(AgentRunLedger {
                db_path: path.into(),
                run_id: lease.root_run_id.clone(),
            });
            context.target_dir = std::env::var_os("OVIRAPTOR_WORKER_KILL_TARGET_DIR")
                .unwrap()
                .into();
            context.log_path = context.target_dir.join("worker.log");
            context.environment.api_base =
                std::env::var("OVIRAPTOR_WORKER_KILL_MODEL_URL").unwrap();
            multi_agent_child_round_transport(
                &context,
                &lease,
                &child,
                "readonly",
                json!({"evidence":"fixed"}),
            )
            .unwrap();
        }
        _ => panic!("unknown kill probe phase"),
    }
    println!("worker-kill-ready");
    std::io::stdout().flush().unwrap();
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
    panic!("parent must SIGKILL without Rust unwinding");
}

#[cfg(unix)]
#[test]
fn assignment_attempt_reassignment_real_sigkill_keeps_dispatch_and_receipt_boundaries() {
    use crate::agent_runtime::multi_agent::{attempts, budget, scheduler, specialist};
    use std::{io::BufRead, os::unix::process::ExitStatusExt};
    for phase in ["before_claim", "after_claim", "after_receipt"] {
        let (root, mut context, lease, old) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        context.target_dir = root.join("kill-target");
        context.log_path = root.join("kill-worker.log");
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response("saved kill result"),
            )
        }));
        context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let mut process = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "scanning::agent_tests::assignment_attempt_reassignment_process_probe",
                "--nocapture",
            ])
            .env("OVIRAPTOR_WORKER_KILL_DB", &context.db_path)
            .env("OVIRAPTOR_WORKER_KILL_PHASE", phase)
            .env("OVIRAPTOR_WORKER_KILL_RUN", &old.run_id)
            .env(
                "OVIRAPTOR_WORKER_KILL_MODEL_URL",
                &context.environment.api_base,
            )
            .env("OVIRAPTOR_WORKER_KILL_TARGET_DIR", &context.target_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let input = process.stdin.take().unwrap();
        let output = process.stdout.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(output).lines() {
                if line.unwrap().contains("worker-kill-ready") {
                    let _ = sender.send(());
                    return;
                }
            }
        });
        let ready = receiver.recv_timeout(std::time::Duration::from_secs(20));
        let killed = process.kill();
        let exit = process.wait().unwrap();
        drop(input);
        reader.join().unwrap();
        assert!(ready.is_ok(), "probe not ready: {phase}");
        assert!(killed.is_ok());
        assert_eq!(
            exit.signal(),
            Some(libc::SIGKILL),
            "{phase} must bypass Rust cleanup"
        );
        // The OS death is real. Expiry uses the shared deterministic clock
        // fixture, followed by the actual production withdrawal transaction.
        expire_worker_deadline(&db, &old.run_id);
        stop_failed_child_preserving_usage(&db, &lease, &old, "SIGKILL").unwrap();
        assert_eq!(
            attempts::current(&db, &lease, &old.assignment_id)
                .unwrap()
                .state,
            "expired"
        );
        let audit = expired_saved_worker_row(&db, &old.run_id);
        match phase {
            "before_claim" => {
                let child = scheduler::prepare_supervised_readonly_child(
                    &db,
                    &lease,
                    old.role,
                    "journal-test",
                    &json!({"fixture":true}),
                    8000,
                )
                .unwrap();
                assert_ne!(child.run_id, old.run_id);
                let (text, usage) = multi_agent_child_round_transport(
                    &context,
                    &lease,
                    &child,
                    "readonly",
                    json!({"evidence":"fixed"}),
                )
                .unwrap();
                complete_readonly_assessment(&db, &lease, &child, &usage, &json!({"summary":text}))
                    .unwrap();
                assert_eq!(seen.lock().unwrap().len(), 1);
            }
            "after_claim" => {
                let stable = super::tests::application_table_snapshot(&db);
                assert!(scheduler::prepare_supervised_readonly_child(
                    &db,
                    &lease,
                    old.role,
                    "journal-test",
                    &json!({"fixture":true}),
                    8000
                )
                .is_err());
                assert!(super::tests::application_table_snapshot(&db) == stable);
                assert!(budget::admission::require_determinate(&db, &lease.root_run_id).is_err());
                assert_eq!(
                    budget::balance(&db, &lease.root_run_id, None, "model_requests")
                        .unwrap()
                        .reserved,
                    1
                );
                assert_eq!(
                    budget::balance(&db, &lease.root_run_id, None, "concurrency_batches")
                        .unwrap()
                        .reserved,
                    1
                );
                assert_eq!(seen.lock().unwrap().len(), 0);
            }
            "after_receipt" => {
                let received = {
                    let tx = db.unchecked_transaction().unwrap();
                    specialist::received_for_reconciliation(&tx, &lease, &old).unwrap()
                };
                let recovered = scheduler::prepare_supervised_readonly_child(
                    &db,
                    &lease,
                    old.role,
                    "journal-test",
                    &json!({"fixture":true}),
                    8000,
                )
                .unwrap();
                assert_eq!(recovered, old);
                complete_readonly_assessment(
                    &db,
                    &lease,
                    &old,
                    &received.usage,
                    &json!({"summary":received.text}),
                )
                .unwrap();
                let stable = super::tests::application_table_snapshot(&db);
                complete_readonly_assessment(
                    &db,
                    &lease,
                    &old,
                    &received.usage,
                    &json!({"summary":received.text}),
                )
                .unwrap();
                assert!(super::tests::application_table_snapshot(&db) == stable);
                assert_eq!(
                    budget::balance(&db, &lease.root_run_id, None, "model_requests")
                        .unwrap()
                        .consumed,
                    1
                );
                assert_eq!(
                    budget::balance(&db, &lease.root_run_id, None, "concurrency_batches")
                        .unwrap()
                        .reserved,
                    0
                );
                assert_eq!(seen.lock().unwrap().len(), 1);
            }
            _ => unreachable!(),
        }
        assert_eq!(expired_saved_worker_row(&db, &old.run_id), audit);
        assert!(attempts::require_live_for_run(&db, &old.run_id).is_err());
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
