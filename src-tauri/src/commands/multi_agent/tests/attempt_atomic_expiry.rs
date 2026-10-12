pub(super) fn expire_worker_deadline(db: &rusqlite::Connection, run: &str) {
    // Only the deadline is restored; production cleanup must issue the expired
    // transition itself. This is a deterministic clock fixture, not real kill QA.
    db.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime') WHERE child_run_id=?1", [run]).unwrap();
}

fn assert_expiry_preserved_resources(db: &rusqlite::Connection, before: &[(String, Vec<String>)]) {
    let after = super::tests::application_table_snapshot(db);
    for (table, rows) in before {
        if table == "agent_worker_expiry_receipts" {
            let current = &after.iter().find(|(name, _)| name == table).unwrap().1;
            assert!(
                rows.iter().all(|row| current.contains(row)),
                "original withdrawal receipts changed"
            );
            let tx = if db.is_autocommit() {
                Some(db.unchecked_transaction().unwrap())
            } else {
                None
            };
            let snapshot = tx.as_deref().unwrap_or(db);
            let mut q = snapshot
                .prepare("SELECT worker_attempt_id FROM agent_worker_expiry_receipts")
                .unwrap();
            let workers = q
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            for worker in workers {
                crate::agent_runtime::multi_agent::attempts::withdrawal_receipt::verify_original(
                    snapshot, &worker,
                )
                .unwrap();
            }
            continue;
        }
        if ![
            "agent_assignment_attempts",
            "agent_assignments",
            "agent_runs",
            "agent_capability_leases",
            "agent_collaboration_events",
        ]
        .contains(&table.as_str())
        {
            assert_eq!(
                after.iter().find(|(name, _)| name == table).unwrap().1,
                *rows,
                "{table}"
            );
        }
    }
}

#[test]
fn assignment_attempt_atomic_expiry_cleanup_preserves_unknown_cost_and_original_resources() {
    use crate::agent_runtime::multi_agent::{attempts, budget, specialist};
    let (root, context, lease, child) = specialist_journal_fixture();
    let db = db::open(&context.db_path).unwrap();
    let specialist::Start::Dispatch(call) = specialist::start(&db, &lease, &child,
        &json!({"schemaVersion":1,"messages":[{"role":"user","content":"original call"}],"tools":[]})).unwrap() else { panic!() };
    specialist::record_uncertain(&db, &call, "unknown provider outcome").unwrap();
    expire_worker_deadline(&db, &child.run_id);
    let original = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    let cost = budget::balance(
        &db,
        &lease.root_run_id,
        Some(&child.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!(cost.indeterminate, 1);
    let before = super::tests::application_table_snapshot(&db);
    stop_failed_child_preserving_usage(&db, &lease, &child, "worker timeout").unwrap();
    let expired = attempts::current(&db, &lease, &child.assignment_id).unwrap();
    assert_eq!(expired.state, "expired");
    assert_eq!(expired.failure_class, "worker_lease_expired");
    assert!(!expired.finished_at.is_empty());
    assert_eq!(
        (
            &expired.id,
            &expired.worker_id,
            &expired.fencing_token,
            &expired.expires_at
        ),
        (
            &original.id,
            &original.worker_id,
            &original.fencing_token,
            &original.expires_at
        )
    );
    assert_eq!(
        budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_requests"
        )
        .unwrap(),
        cost
    );
    assert_eq!(
        budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "concurrency_batches"
        )
        .unwrap()
        .reserved,
        1
    );
    assert_expiry_preserved_resources(&db, &before);
    assert!(attempts::require_live_for_run(&db, &child.run_id).is_err());
    let saved = super::tests::application_table_snapshot(&db);
    assert!(complete_readonly_assessment(
        &db,
        &lease,
        &child,
        &AgentTokenUsage::default(),
        &json!({"summary":"unknown result"})
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), saved);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_atomic_expiry_real_review_cleanup_then_local_receipt_keeps_expired_audit() {
    use crate::agent_runtime::multi_agent::{attempts, budget};
    let f = review_receipt_fixture();
    let db = db::open(&f.context.db_path).unwrap();
    attempt_saved_call(
        &db,
        &f.lease,
        &f.child,
        &f.candidate,
        &review_receipt_response(),
    );
    expire_worker_deadline(&db, &f.child.run_id);
    finish_failed_review(&db, &f.lease, &f.child, "review-receipt", "worker timeout").unwrap();
    assert_eq!(
        attempts::current(&db, &f.lease, &f.child.assignment_id)
            .unwrap()
            .state,
        "expired"
    );
    let original = expired_saved_worker_row(&db, &f.child.run_id);
    assert_eq!(
        budget::balance(
            &db,
            &f.lease.root_run_id,
            Some(&f.child.assignment_id),
            "concurrency_batches"
        )
        .unwrap()
        .reserved,
        1
    );
    forbid_expired_saved_execution(&db);
    recover_received_review(
        &db,
        &f.lease,
        "receipt",
        1,
        &f.candidate.to_string(),
        &f.context.target_dir,
    )
    .unwrap();
    assert_review_receipt_completed(&db);
    assert_eq!(expired_saved_worker_row(&db, &f.child.run_id), original);
    assert!(attempts::require_live_for_run(&db, &f.child.run_id).is_err());
    let completed = super::tests::application_table_snapshot(&db);
    stop_failed_child_preserving_usage(&db, &f.lease, &f.child, "completed repeat").unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), completed);
    drop(db);
    fs::remove_dir_all(f.root).unwrap();
}
