#[test]
fn assignment_attempt_supervisor_natural_expiry_without_callback_withdraws_and_keeps_unknown_resources(
) {
    use crate::agent_runtime::multi_agent::{
        attempts, budget, specialist, supervisor::WorkerSupervisor,
    };
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    assert!(matches!(
        specialist::start(&db, &lease, &child, &json!({"messages":[]})).unwrap(),
        specialist::Start::Dispatch(_)
    ));
    db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','+1 seconds') WHERE child_run_id=?1",[&child.run_id]).unwrap();
    let original = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    let model = budget::balance(&db, &lease.root_run_id, None, "model_requests").unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    loop {
        guard.check().unwrap();
        if attempts::current(&db, &lease, &child.assignment_id)
            .unwrap()
            .state
            == "expired"
        {
            break;
        }
        assert!(std::time::Instant::now()<deadline,"background supervision must withdraw on natural time expiry without a callback or explicit cleanup");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let expired = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    assert_eq!(expired.id, original.id);
    assert_eq!(expired.worker_id, original.worker_id);
    assert_eq!(expired.fencing_token, original.fencing_token);
    assert_eq!(expired.expires_at, original.expires_at);
    assert_eq!(
        budget::balance(&db, &lease.root_run_id, None, "model_requests").unwrap(),
        model
    );
    assert_eq!(
        budget::balance(&db, &lease.root_run_id, None, "concurrency_batches")
            .unwrap()
            .reserved,
        1
    );
    assert!(budget::admission::require_determinate(&db, &lease.root_run_id).is_err());
    let stable = super::tests::application_table_snapshot(&db);
    drop(guard);
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(super::tests::application_table_snapshot(&db) == stable);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_rejects_missing_database_and_wrong_parent_without_writes() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, _) = specialist_journal_fixture();
    let missing = root.join("must-not-create.sqlite3");
    assert!(WorkerSupervisor::start(&missing, &lease).is_err());
    assert!(!missing.exists());
    let db = db::open(&context.db_path).unwrap();
    for field in ["root", "target", "scan", "epoch", "fence"] {
        let mut wrong = lease.clone();
        match field {
            "root" => wrong.root_run_id = uuid::Uuid::new_v4().to_string(),
            "target" => wrong.target_key = "https://other.invalid".into(),
            "scan" => wrong.scan_id = "other-scan".into(),
            "epoch" => wrong.lease_epoch += 1,
            _ => wrong.fencing_token = uuid::Uuid::new_v4().to_string(),
        }
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            WorkerSupervisor::start(&context.db_path, &wrong).is_err(),
            "{field}"
        );
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{field}"
        );
    }
    db.execute(
        "UPDATE agent_runs SET orchestration_policy='single' WHERE id=?1",
        [&lease.root_run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(WorkerSupervisor::start(&context.db_path, &lease).is_err());
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_stop_joins_before_later_expiry() {
    use crate::agent_runtime::multi_agent::{attempts, supervisor::WorkerSupervisor};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    let start = std::time::Instant::now();
    drop(guard);
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    expire_worker_deadline(&db, &child.run_id);
    let before = super::tests::application_table_snapshot(&db);
    std::thread::sleep(std::time::Duration::from_millis(350));
    assert_eq!(
        attempts::current(&db, &lease, &child.assignment_id)
            .unwrap()
            .state,
        "running"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_async_write_failure_is_visible_and_atomic() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    db.execute_batch(
        "CREATE TRIGGER supervisor_fault BEFORE UPDATE OF state ON agent_assignment_attempts
        WHEN NEW.state='expired' BEGIN SELECT RAISE(ABORT,'supervisor-fault'); END;",
    )
    .unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let before = super::tests::application_table_snapshot(&db);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let failure = loop {
        if let Err(error) = guard.check() {
            break error;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(failure.contains("supervisor-fault"), "{failure}");
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(guard);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_bad_deadline_and_partial_withdrawal_roll_back() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    for fault in [
        "invalid-deadline",
        "ignore-worker",
        "collateral-budget",
        "revoke-parent",
    ] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        expire_worker_deadline(&db, &child.run_id);
        match fault {
            "invalid-deadline"=>{db.execute("UPDATE agent_assignment_attempts SET expires_at='invalid' WHERE child_run_id=?1",[&child.run_id]).unwrap();},
            "ignore-worker"=>db.execute_batch("CREATE TRIGGER supervisor_fault BEFORE UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='expired' BEGIN SELECT RAISE(IGNORE); END;").unwrap(),
            "collateral-budget"=>db.execute_batch("CREATE TRIGGER supervisor_fault AFTER UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='expired' BEGIN UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens+1; END;").unwrap(),
            _=>db.execute_batch("CREATE TRIGGER supervisor_fault AFTER UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='expired' BEGIN UPDATE agent_runs SET cancel_requested_at='fault' WHERE role='coordinator'; END;").unwrap(),
        }
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            WorkerSupervisor::start(&context.db_path, &lease).is_err(),
            "{fault}"
        );
        assert!(
            super::tests::application_table_snapshot(&db) == before,
            "{fault}"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_supervisor_writer_contention_waits_then_withdraws() {
    use crate::agent_runtime::multi_agent::{attempts, supervisor::WorkerSupervisor};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let guard = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    std::thread::sleep(std::time::Duration::from_millis(650));
    guard
        .check()
        .expect("an active writer is not a failed supervisor");
    assert!(super::tests::application_table_snapshot(&tx) == before);
    tx.commit().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        guard.check().unwrap();
        if attempts::current(&db, &lease, &child.assignment_id)
            .unwrap()
            .state
            == "expired"
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    drop(guard);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_missing_worker_is_not_a_healthy_empty_queue() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    // A damaged restored Native store, not a normal production deletion.
    db.execute_batch("DROP TRIGGER assignment_attempt_no_delete")
        .unwrap();
    db.execute(
        "DELETE FROM agent_assignment_attempts WHERE child_run_id=?1",
        [&child.run_id],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let service = WorkerSupervisor::start(&context.db_path, &lease);
    assert!(
        service.is_err(),
        "a running assignment with no worker must fail supervision"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    drop(service);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_successor_scope_does_not_adopt_other_root_workers() {
    use crate::agent_runtime::multi_agent::{attempts, specialist, supervisor::WorkerSupervisor};
    for different_root in [false, true] {
        let (root, context, old, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        assert!(matches!(
            specialist::start(&db, &old, &child, &json!({"messages":[]})).unwrap(),
            specialist::Start::Dispatch(_)
        ));
        let guard = WorkerSupervisor::start(&context.db_path, &old).unwrap();
        let successor = replace_target_cost_coordinator(&db, &old, different_root);
        if different_root {
            // The billing helper intentionally creates a planless successor.
            // A living execution parent now requires its own frozen clock;
            // initialize only this new fixture Root, never repair the old one.
            db.execute(
                "UPDATE agent_runs SET plan_json=?2 WHERE id=?1",
                params![
                    successor.root_run_id,
                    context.execution_plan.as_json().to_string()
                ],
            )
            .unwrap();
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while guard.check().is_ok() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(guard);
        expire_worker_deadline(&db, &child.run_id);
        let before = super::tests::application_table_snapshot(&db);
        let service = WorkerSupervisor::start(&context.db_path, &successor).unwrap();
        service.check().unwrap();
        let worker = attempts::current(&db, &old, &child.assignment_id).unwrap();
        assert_eq!(
            worker.state,
            if different_root { "running" } else { "expired" }
        );
        assert_eq!(worker.coordinator_epoch, old.lease_epoch);
        assert_eq!(worker.coordinator_fencing_token, old.fencing_token);
        if different_root {
            assert!(super::tests::application_table_snapshot(&db) == before);
        } else {
            assert_expiry_preserved_resources(&db, &before);
        }
        drop(service);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_supervisor_private_withdrawal_denies_business_other_root_and_forged_emitter_writes(
) {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    for change in [
        "INSERT INTO projects(name) VALUES('expiry-trigger-business')",
        "UPDATE agent_runs SET status='paused' WHERE id='expiry-other-root'",
        "INSERT INTO agent_collaboration_events(scan_id,attempt_number,entity_type,entity_id,event_type,payload_json) SELECT scan_id,attempt_number,'run',id,'run','{}' FROM agent_runs WHERE role='coordinator'",
    ] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        db.execute("INSERT INTO agent_runs(id,scan_id,target_url,root_run_id,role,status,orchestration_policy) VALUES('expiry-other-root',?1,'https://other.invalid','expiry-other-root','coordinator','prepared','multi')", [&lease.scan_id]).unwrap();
        expire_worker_deadline(&db, &child.run_id);
        db.execute_batch(&format!("CREATE TRIGGER expiry_business_escape AFTER UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='expired' BEGIN {change}; END;")).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let allocator = client_delivery_rows(&db, "sqlite_sequence");
        assert!(WorkerSupervisor::start(&context.db_path, &lease).is_err(), "{change}");
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(client_delivery_rows(&db, "sqlite_sequence"), allocator);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    db.execute_batch("DROP TRIGGER agent_collaboration_run_update; CREATE TRIGGER agent_collaboration_run_update AFTER UPDATE ON agent_runs BEGIN INSERT INTO projects(name) VALUES('forged-expiry-emitter'); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let allocator = client_delivery_rows(&db, "sqlite_sequence");
    assert!(WorkerSupervisor::start(&context.db_path, &lease).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(client_delivery_rows(&db, "sqlite_sequence"), allocator);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_supervisor_async_private_writer_rolls_back_without_rowid_business_escape() {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    for operation in [
        "INSERT INTO expiry_business_without_rowid VALUES('extra','trigger')",
        "UPDATE expiry_business_without_rowid SET value='trigger'",
        "DELETE FROM expiry_business_without_rowid",
    ] {
        let (root, context, lease, child) = specialist_journal_fixture();
        let db = db::open(&context.db_path).unwrap();
        db.execute_batch("CREATE TABLE expiry_business_without_rowid(id TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID; INSERT INTO expiry_business_without_rowid VALUES('original','keep');").unwrap();
        let service = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER expiry_async_escape AFTER UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='expired' BEGIN {operation}; END;")).unwrap();
        expire_worker_deadline(&db, &child.run_id);
        let before = super::tests::application_table_snapshot(&db);
        let allocator = client_delivery_rows(&db, "sqlite_sequence");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if service.check().is_err() {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{operation}");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{operation}"
        );
        assert_eq!(client_delivery_rows(&db, "sqlite_sequence"), allocator);
        drop(service);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_supervisor_failed_private_withdrawal_keeps_original_worker_for_clean_restart()
{
    use crate::agent_runtime::multi_agent::{attempts, supervisor::WorkerSupervisor};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let original = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    db.execute_batch("CREATE TRIGGER expiry_restart_escape AFTER UPDATE OF state ON agent_assignment_attempts WHEN NEW.state='expired' BEGIN INSERT INTO projects(name) VALUES('escape'); END;").unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(WorkerSupervisor::start(&context.db_path, &lease).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(
        attempts::current(&db, &lease, &child.assignment_id).unwrap(),
        original
    );
    // Remove only this disposable fixture fault; preserve the original worker.
    db.execute_batch("DROP TRIGGER expiry_restart_escape")
        .unwrap();
    let restart = WorkerSupervisor::start(&context.db_path, &lease).unwrap();
    restart.check().unwrap();
    let withdrawn = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    assert_eq!(withdrawn.state, "expired");
    assert_eq!(withdrawn.id, original.id);
    assert_eq!(withdrawn.worker_id, original.worker_id);
    assert_eq!(withdrawn.fencing_token, original.fencing_token);
    assert_expiry_preserved_resources(&db, &before);
    drop(restart);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
