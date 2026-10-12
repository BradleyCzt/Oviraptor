#[test]
fn assignment_attempt_saved_message_requires_exact_review_request_and_candidate() {
    use crate::agent_runtime::multi_agent::mailbox;
    for wrong in ["correlation", "candidate"] {
        let f = review_receipt_fixture();
        let db = db::open(&f.context.db_path).unwrap();
        attempt_saved_call(
            &db,
            &f.lease,
            &f.child,
            &f.candidate,
            &review_receipt_response(),
        );
        db.execute(
            "UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",
            [],
        )
        .unwrap();
        let payload = serde_json::json!({"verdict":"confirmed","summary":"reviewed",
            "candidateId":if wrong=="candidate" {"foreign-candidate"} else {"receipt"},"candidateRevision":1});
        let before = super::tests::application_table_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        let result = mailbox::send_saved_specialist(
            &tx,
            &f.lease,
            &f.child.run_id,
            &f.lease.root_run_id,
            "evidence_reviewer",
            "coordinator",
            "review_decision",
            if wrong == "correlation" {
                "foreign-request"
            } else {
                "review-receipt"
            },
            &f.child.assignment_id,
            1,
            &payload,
        );
        assert!(
            result.is_err(),
            "saved provider text cannot confer authority for {wrong}"
        );
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        drop(db);
        std::fs::remove_dir_all(f.root).unwrap();
    }
}

#[test]
fn assignment_attempt_saved_coordinator_message_never_reopens_a_worker_grant() {
    use crate::agent_runtime::multi_agent::{attempts, mailbox};
    let f = review_receipt_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    attempt_saved_call(
        &db,
        &f.lease,
        &f.child,
        &f.candidate,
        &review_receipt_response(),
    );
    db.execute(
        "UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",
        [],
    )
    .unwrap();
    let original = attempts::current(&db, &f.lease, &f.child.assignment_id).unwrap();
    let payload = serde_json::json!({"verdict":"confirmed","summary":"reviewed","candidateId":"receipt","candidateRevision":1});
    let tx = db.unchecked_transaction().unwrap();
    let id = mailbox::send_saved_specialist(
        &tx,
        &f.lease,
        &f.child.run_id,
        &f.lease.root_run_id,
        "evidence_reviewer",
        "coordinator",
        "review_decision",
        "review-receipt",
        &f.child.assignment_id,
        1,
        &payload,
    )
    .unwrap();
    tx.commit().unwrap();
    mailbox::deliver_expected(&db, &f.lease, &f.lease.root_run_id, &id).unwrap();
    mailbox::acknowledge(&db, &f.lease, &f.lease.root_run_id, &id).unwrap();
    assert_eq!(
        attempts::current(&db, &f.lease, &f.child.assignment_id).unwrap(),
        original
    );
    assert!(attempts::require_live_for_run(&db, &f.child.run_id).is_err());
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(mailbox::send_saved_specialist(
        &tx,
        &f.lease,
        &f.lease.root_run_id,
        &f.child.run_id,
        "coordinator",
        "evidence_reviewer",
        "assignment",
        "illegal-grant",
        &f.child.assignment_id,
        1,
        &payload
    )
    .is_err());
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    drop(db);
    std::fs::remove_dir_all(f.root).unwrap();
}

#[test]
fn assignment_attempt_saved_gap_messages_require_original_candidate_and_thread() {
    use crate::agent_runtime::multi_agent::mailbox;
    for kind in ["gap_proposed", "proposal_assessed"] {
        for wrong in ["candidate", "correlation"] {
            let f = gap_receipt_fixture();
            let lease = &f.session.lease;
            let db = db::open(&f.context.db_path).unwrap();
            attempt_saved_call(&db, lease, &f.child, &f.input, &gap_receipt_response());
            db.execute(
                "UPDATE agent_assignment_attempts SET expires_at='2000-01-01'",
                [],
            )
            .unwrap();
            // This fixture keeps the Root's live WorkerSupervisor (100 ms sweep),
            // which withdraws the attempts just backdated. Let that production
            // withdrawal commit first so it cannot land between the snapshots.
            settle_supervisor_withdrawal_of_backdated_attempts(&db, &lease.root_run_id);
            let payload = serde_json::json!({"summary":"needs operator review",
                "candidateId":if wrong=="candidate" {"foreign"} else {"candidate-gap"},"evidenceRevision":2,
                "targetRequestsGranted":0,"decision":"deferred_requires_new_evidence_revision","schemaVersion":3});
            let (from, to, from_role, to_role) = if kind == "gap_proposed" {
                (
                    &f.child.run_id,
                    &lease.root_run_id,
                    "deep_investigator",
                    "coordinator",
                )
            } else {
                (
                    &lease.root_run_id,
                    &f.child.run_id,
                    "coordinator",
                    "deep_investigator",
                )
            };
            let before = super::tests::application_table_snapshot(&db);
            let tx = db.unchecked_transaction().unwrap();
            assert!(
                mailbox::send_saved_specialist(
                    &tx,
                    lease,
                    from,
                    to,
                    from_role,
                    to_role,
                    kind,
                    if wrong == "correlation" {
                        "foreign"
                    } else {
                        "review-gap:candidate-gap:2"
                    },
                    &f.child.assignment_id,
                    2,
                    &payload
                )
                .is_err(),
                "{kind}: {wrong}"
            );
            tx.rollback().unwrap();
            assert_eq!(super::tests::application_table_snapshot(&db), before);
            drop(db);
            std::fs::remove_dir_all(f.root).unwrap();
        }
    }
}

/// Waits (bounded) until the live WorkerSupervisor has no due attempt left for
/// this Root, using the supervisor's own deadline predicate.
fn settle_supervisor_withdrawal_of_backdated_attempts(db: &rusqlite::Connection, root_run_id: &str) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let due: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_assignments a
                 JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id
                 WHERE a.coordinator_run_id=?1 AND x.state IN ('leased','running','paused')
                   AND COALESCE(datetime(x.expires_at)<=datetime('now','localtime'),1))",
                [root_run_id],
                |row| row.get(0),
            )
            .unwrap();
        if !due {
            return;
        }
        assert!(
            std::time::Instant::now() < until,
            "live WorkerSupervisor never withdrew the backdated attempts"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
