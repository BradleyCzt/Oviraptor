fn confirmed_directive_for_lease(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> String {
    use crate::agent_runtime::multi_agent::directive;
    let draft = directive::create_draft(
        connection,
        &lease.scan_id,
        lease.attempt_number,
        &lease.root_run_id,
        &lease.target_key,
        "coordinator",
        "请优先复核已有证据",
        lease.lease_epoch,
        &lease.fencing_token,
    )
    .unwrap();
    directive::confirm_draft(
        connection,
        &lease.scan_id,
        lease.attempt_number,
        &lease.root_run_id,
        &lease.target_key,
        &draft.id,
        draft.revision,
        &draft.draft_hash,
    )
    .unwrap()
    .id
}

#[test]
fn directive_claim_does_not_reject_another_targets_confirmed_message() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentRole, AgentRunStatus},
        multi_agent::{directive, lease},
        store::{self, AgentRunRow},
    };
    let (root, db_path, _, first) = multi_agent_test_root("directive-two-targets", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let second_target = "https://second.authorized.example.test";
    let second_root = "root-directive-second-target";
    let mut run = AgentRunRow::new(
        second_root,
        &first.scan_id,
        first.attempt_number,
        second_target,
        AgentBackendKind::Native,
        AgentRole::Coordinator,
        "plan",
        "evidence",
    );
    run.root_run_id = second_root.into();
    run.status = AgentRunStatus::Running;
    store::create_run(&connection, &run).unwrap();
    let second = lease::acquire_coordinator_lease(
        &connection,
        &first.scan_id,
        first.attempt_number,
        second_target,
        second_root,
        600,
    )
    .unwrap();
    let first_message = confirmed_directive_for_lease(&connection, &first);
    let second_message = confirmed_directive_for_lease(&connection, &second);
    let claimed = directive::claim_pending_directives(&connection, &first, 20).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, first_message);
    let other_status: String = connection
        .query_row(
            "SELECT status FROM agent_user_directives WHERE id=?1",
            [&second_message],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        other_status, "pending",
        "another target must retain its confirmed directive"
    );
    let claimed = directive::claim_pending_directives(&connection, &second, 20).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, second_message);
    assert!(directive::claim_pending_directives(&connection, &first, 20)
        .unwrap()
        .is_empty());
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_writes_revalidate_fencing_after_waiting_for_the_database_writer() {
    use crate::agent_runtime::multi_agent::directive;
    use std::sync::atomic::{AtomicBool, Ordering};
    // The directive transition writes through the dedicated worker connection,
    // so its busy callback proves it reached the real SQLite write lock.
    static WRITE_WAITING: AtomicBool = AtomicBool::new(false);
    static WORKER_STARTED: AtomicBool = AtomicBool::new(false);
    let mut observations = Vec::new();
    for finish in [false, true] {
        WRITE_WAITING.store(false, Ordering::SeqCst);
        let (root, db_path, _, lease) = multi_agent_test_root("directive-write-race", 500, 4);
        let connection = db::open(&db_path).unwrap();
        let id = confirmed_directive_for_lease(&connection, &lease);
        assert_eq!(
            directive::claim_pending_directives(&connection, &lease, 20)
                .unwrap()
                .len(),
            1
        );
        if finish {
            directive::transition_directive(&connection, &lease, &id, "claimed", "accepted")
                .unwrap();
            directive::transition_directive(&connection, &lease, &id, "accepted", "applied")
                .unwrap();
        }
        let worker = db::open(&db_path).unwrap();
        worker
            .busy_handler(Some(|attempt| {
                WRITE_WAITING.store(true, Ordering::SeqCst);
                std::thread::sleep(std::time::Duration::from_millis(1));
                attempt < 5_000
            }))
            .unwrap();
        let transaction = rusqlite::Transaction::new_unchecked(
            &connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .unwrap();
        transaction.execute(
            "UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='replacement-token' \
             WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3",
            rusqlite::params![lease.scan_id, lease.attempt_number, lease.target_key],
        ).unwrap();
        let worker_lease = lease.clone();
        let worker_id = id.clone();
        WORKER_STARTED.store(false, Ordering::SeqCst);
        let handle = std::thread::spawn(move || {
            WORKER_STARTED.store(true, Ordering::SeqCst);
            if finish {
                finish_coordinator_run(&worker, &worker_lease, &AgentTargetOutcome::Cancelled)
            } else {
                directive::transition_directive(
                    &worker,
                    &worker_lease,
                    &worker_id,
                    "claimed",
                    "accepted",
                )
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let reached_writer = if finish {
            // Finalization never writes through the caller's connection: the
            // elapsed-fact and closure writers each open their own private,
            // authorized connection (budget/clock/elapsed_fact/writer.rs,
            // budget/clock/closure_writer.rs) with their own busy timeout, so
            // the worker connection's busy handler cannot observe them. Their
            // only pre-lock work is reads; a started worker that is still
            // unfinished after holding the real write lock for a full window
            // is parked on that lock (BEGIN IMMEDIATE), and its result below
            // can only be stale if it validated after the commit.
            while !WORKER_STARTED.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            WORKER_STARTED.load(Ordering::SeqCst) && !handle.is_finished()
        } else {
            while !WRITE_WAITING.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            WRITE_WAITING.load(Ordering::SeqCst)
        };
        transaction.commit().unwrap();
        let result = handle.join().unwrap();
        let actual: String = connection
            .query_row(
                "SELECT status FROM agent_user_directives WHERE id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        observations.push((finish, reached_writer, result, actual));
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
    assert!(
        observations.iter().all(|(_, reached, _, _)| *reached),
        "workers must contend on the held SQLite write lock"
    );
    assert!(
        observations.iter().all(|(_, _, result, _)| result
            .as_ref()
            .is_err_and(|error| error == "stale_coordinator_fencing_token")),
        "both state transition and finalization must reject a revoked lease: {observations:?}"
    );
    for (finish, _, _, actual) in observations {
        assert_eq!(actual, if finish { "applied" } else { "claimed" });
    }
}

#[test]
fn directive_claim_and_application_stop_when_the_attempt_is_not_active() {
    use crate::agent_runtime::multi_agent::directive;
    for status in ["paused", "cancelled", "completed"] {
        let (root, db_path, _, lease) = multi_agent_test_root("directive-inactive", 500, 4);
        let connection = db::open(&db_path).unwrap();
        let pending = confirmed_directive_for_lease(&connection, &lease);
        let claimed = directive::claim_pending_directives(&connection, &lease, 20).unwrap();
        assert_eq!(claimed.len(), 1);
        let waiting = confirmed_directive_for_lease(&connection, &lease);
        connection
            .execute(
                "UPDATE sentinel_scans SET status=?1 WHERE id=?2",
                rusqlite::params![status, lease.scan_id],
            )
            .unwrap();
        let claim_result = directive::claim_pending_directives(&connection, &lease, 20);
        let transition_result =
            directive::transition_directive(&connection, &lease, &pending, "claimed", "accepted");
        assert_eq!(claim_result.unwrap_err(), "agent_attempt_not_active");
        assert_eq!(transition_result.unwrap_err(), "agent_attempt_not_active");
        let state: String = connection
            .query_row(
                "SELECT status FROM agent_user_directives WHERE id=?1",
                [&waiting],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "pending");
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn directive_finalization_after_pause_is_bookkeeping_and_cannot_advance_waiting_work() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db_path, _, lease) = multi_agent_test_root("directive-pause-finalize", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let applied = confirmed_directive_for_lease(&connection, &lease);
    directive::claim_pending_directives(&connection, &lease, 20).unwrap();
    directive::transition_directive(&connection, &lease, &applied, "claimed", "accepted").unwrap();
    directive::transition_directive(&connection, &lease, &applied, "accepted", "applied").unwrap();
    let pending = confirmed_directive_for_lease(&connection, &lease);
    connection
        .execute(
            "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
            [&lease.scan_id],
        )
        .unwrap();
    finish_coordinator_run(&connection, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    let before: i64 = connection.query_row("SELECT COUNT(*) FROM agent_collaboration_events", [], |r| r.get(0)).unwrap();
    finish_coordinator_run(&connection, &lease, &AgentTargetOutcome::Cancelled).unwrap();
    let after: i64 = connection.query_row("SELECT COUNT(*) FROM agent_collaboration_events", [], |r| r.get(0)).unwrap();
    assert_eq!(before, after, "replaying closure cannot duplicate receipts");
    for (id, expected) in [(applied, "deferred"), (pending, "deferred")] {
        let state: String = connection
            .query_row(
                "SELECT status FROM agent_user_directives WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, expected);
    }
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn directive_progression_rejects_replaced_and_deleted_attempts_with_live_leases() {
    use crate::agent_runtime::multi_agent::directive;
    for deleted in [false, true] {
        let (root, db_path, _, lease) = multi_agent_test_root("directive-obsolete", 500, 4);
        let connection = db::open(&db_path).unwrap();
        let id = confirmed_directive_for_lease(&connection, &lease);
        directive::claim_pending_directives(&connection, &lease, 20).unwrap();
        if deleted {
            connection
                .execute(
                    "INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)",
                    [&lease.scan_id],
                )
                .unwrap();
        } else {
            connection
                .execute(
                    "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
                    [&lease.scan_id],
                )
                .unwrap();
        }
        assert_eq!(
            directive::transition_directive(&connection, &lease, &id, "claimed", "accepted")
                .unwrap_err(),
            "agent_attempt_not_active"
        );
        assert_eq!(
            directive::claim_pending_directives(&connection, &lease, 20).unwrap_err(),
            "agent_attempt_not_active"
        );
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn directive_state_and_collaboration_event_roll_back_together_on_write_failure() {
    use crate::agent_runtime::multi_agent::directive;
    let (root, db_path, _, lease) = multi_agent_test_root("directive-write-rollback", 500, 4);
    let connection = db::open(&db_path).unwrap();
    let id = confirmed_directive_for_lease(&connection, &lease);
    directive::claim_pending_directives(&connection, &lease, 20).unwrap();
    let event_count = || {
        connection
            .query_row(
                "SELECT count(*) FROM agent_collaboration_events WHERE scan_id=?1",
                [&lease.scan_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    let before = event_count();
    // Fail the transaction's timeline write, not the request validation. No
    // status update is allowed to escape without its committed event.
    connection.execute_batch(
        "CREATE TEMP TRIGGER directive_event_failure BEFORE INSERT ON agent_collaboration_events \
         BEGIN SELECT RAISE(ABORT, 'injected_collaboration_failure'); END;",
    ).unwrap();
    let result = directive::transition_directive(&connection, &lease, &id, "claimed", "accepted");
    assert!(result
        .unwrap_err()
        .contains("injected_collaboration_failure"));
    assert_eq!(event_count(), before);
    let state: String = connection
        .query_row(
            "SELECT status FROM agent_user_directives WHERE id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "claimed");
    connection
        .execute_batch("DROP TRIGGER directive_event_failure;")
        .unwrap();
    directive::transition_directive(&connection, &lease, &id, "claimed", "accepted").unwrap();
    assert_eq!(event_count(), before + 1);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}
