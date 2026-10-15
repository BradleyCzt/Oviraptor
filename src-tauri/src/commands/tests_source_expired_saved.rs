#[test]
fn source_expired_saved_reviewers_keep_original_worker_and_complete_local_publication() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{budget, scheduler::ScheduledChild},
    };
    for coverage in [true, false] {
        let (root, db, _, result) = source_reviewer_execution_fixture_using_calls(
            if coverage { "no_candidates" } else { "valid" },
            None,
            if coverage { 4 } else { 3 },
            None,
            7,
            |root, db, record, lease| {
                if coverage {
                    source_coverage_checkpoint_fixture(root, db, record, lease, "paused_received");
                } else {
                    source_reviewer_checkpoint_fixture(root, db, record, lease, "paused_received");
                }
                let child=db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND trigger_code=?2",
                    params![lease.root_run_id,if coverage {"source_coverage_ready"} else {"source_candidates_ready"}],
                    |r|Ok(ScheduledChild {assignment_id:r.get(0)?,run_id:r.get(1)?,role:AgentRole::EvidenceReviewer})).unwrap();
                let original =
                    crate::scanning::agent_tests::restore_expired_saved_worker(db, &child);
                crate::scanning::agent_tests::forbid_expired_saved_execution(db);
                let database = root.join("oviraptor.sqlite3");
                let work = root.join("attempt-0001");
                let (model, runtime, _) =
                    verify_source_runtime_contract(db, &record.scan_id, 1, &work).unwrap();
                let context = SpecialistTransportContext { supervision: None,
                    db_path: &database,
                    scan_id: &record.scan_id,
                    attempt_number: 1,
                    target_key: &lease.target_key,
                    run_id: &lease.root_run_id,
                    environment: &model,
                    proxy: source_runtime_proxy(&runtime).unwrap(),
                    usage_dir: &work,
                    deadline: None,
                };
                let complete = || {
                    if coverage {
                        complete_source_coverage_review(db, &context, lease, &child)
                    } else {
                        complete_source_candidate_review(db, &context, lease, &child)
                    }
                };
                let tx = db.unchecked_transaction().unwrap();
                if coverage {
                    assert!(
                        matches!(crate::agent_runtime::multi_agent::source_coverage_reviewer::progress(&tx,lease)?,
                    crate::agent_runtime::multi_agent::source_coverage_reviewer::ReviewProgress::Received(ref actual) if *actual==child)
                    );
                } else {
                    assert!(
                        matches!(crate::agent_runtime::multi_agent::source_reviewer::progress(&tx,lease)?,
                    crate::agent_runtime::multi_agent::source_reviewer::ReviewProgress::Received(ref actual) if *actual==child)
                    );
                }
                tx.commit().unwrap();
                let payload = complete()?;
                assert_eq!(
                    crate::scanning::agent_tests::expired_saved_worker_row(db, &child.run_id),
                    original
                );
                assert_eq!(
                    budget::balance(
                        db,
                        &lease.root_run_id,
                        Some(&child.assignment_id),
                        "concurrency_batches"
                    )
                    .unwrap()
                    .reserved,
                    0
                );
                let saved = application_table_snapshot(db);
                assert_eq!(complete()?, payload);
                assert_eq!(application_table_snapshot(db), saved);
                Ok(payload)
            },
        );
        assert!(result.is_ok(), "{coverage}: {result:?}");
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_expired_saved_final_decision_faults_roll_back_all_local_writes() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler::ScheduledChild};
    for coverage in [true, false] {
        let (root, db, _, result) = source_reviewer_execution_fixture_using_calls(
            if coverage { "no_candidates" } else { "valid" },
            None,
            if coverage { 4 } else { 3 },
            None,
            7,
            |root, db, record, lease| {
                if coverage {
                    source_coverage_checkpoint_fixture(root, db, record, lease, "paused_received");
                } else {
                    source_reviewer_checkpoint_fixture(root, db, record, lease, "paused_received");
                }
                let child = db.query_row("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND trigger_code=?2",
                    params![lease.root_run_id, if coverage {"source_coverage_ready"} else {"source_candidates_ready"}],
                    |r| Ok(ScheduledChild { assignment_id:r.get(0)?, run_id:r.get(1)?, role:AgentRole::EvidenceReviewer })).unwrap();
                crate::scanning::agent_tests::restore_expired_saved_worker(db, &child);
                let database = root.join("oviraptor.sqlite3");
                let work = root.join("attempt-0001");
                let (model, runtime, _) =
                    verify_source_runtime_contract(db, &record.scan_id, 1, &work).unwrap();
                let context = SpecialistTransportContext { supervision: None,
                    db_path: &database,
                    scan_id: &record.scan_id,
                    attempt_number: 1,
                    target_key: &lease.target_key,
                    run_id: &lease.root_run_id,
                    environment: &model,
                    proxy: source_runtime_proxy(&runtime).unwrap(),
                    usage_dir: &work,
                    deadline: None,
                };
                let complete = || {
                    if coverage {
                        complete_source_coverage_review(db, &context, lease, &child)
                    } else {
                        complete_source_candidate_review(db, &context, lease, &child)
                    }
                };
                let table = if coverage {
                    "agent_source_coverage_decisions"
                } else {
                    "agent_source_review_decisions"
                };
                // Temporary corruption fixture: prove the final captured call
                // comparison rather than relying on the receipt UPDATE trigger.
                db.execute_batch("DROP TRIGGER agent_specialist_call_immutable")
                    .unwrap();
                for mutation in [
                    format!("UPDATE agent_assignment_attempts SET heartbeat_at='2000-01-01' WHERE child_run_id='{}'", child.run_id),
                    format!("UPDATE agent_assignment_attempts SET failure_class='changed' WHERE child_run_id='{}'", child.run_id),
                    format!("UPDATE agent_assignments SET deadline_at='2099-01-01' WHERE id='{}'", child.assignment_id),
                    format!("UPDATE agent_specialist_calls SET failure_code='changed' WHERE assignment_id='{}'", child.assignment_id),
                    "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'".to_string(),
                ] {
                    db.execute_batch(&format!("CREATE TRIGGER expired_source_publication_fault AFTER INSERT ON {table} BEGIN {mutation}; END;")).unwrap();
                    let before = application_table_snapshot(db);
                    assert!(complete().is_err(), "{coverage}: {mutation}");
                    assert_eq!(application_table_snapshot(db), before, "{coverage}: {mutation}");
                    db.execute_batch("DROP TRIGGER expired_source_publication_fault").unwrap();
                }
                let payload = complete()?;
                for mutation in [
                    "failure_class='changed'",
                    "finished_at=''",
                    "expires_at='2099-01-01'",
                ] {
                    db.execute_batch("SAVEPOINT expired_source_closed_audit")
                        .unwrap();
                    db.execute_batch(&format!(
                        "UPDATE agent_assignment_attempts SET {mutation} WHERE child_run_id='{}'",
                        child.run_id
                    ))
                    .unwrap();
                    let before = application_table_snapshot(db);
                    if coverage {
                        assert!(
                            crate::agent_runtime::multi_agent::source_coverage_reviewer::progress(
                                db, lease
                            )
                            .is_err(),
                            "coverage completed progress: {mutation}"
                        );
                        assert!(crate::agent_runtime::multi_agent::source_coverage_reviewer::audit_delivery(db, lease).is_err(), "coverage closed audit: {mutation}");
                    } else {
                        assert!(
                            crate::agent_runtime::multi_agent::source_reviewer::progress(db, lease)
                                .is_err(),
                            "candidate completed progress: {mutation}"
                        );
                        assert!(
                            crate::agent_runtime::multi_agent::source_reviewer::audit_delivery(
                                db, lease
                            )
                            .is_err(),
                            "candidate closed audit: {mutation}"
                        );
                    }
                    assert_eq!(application_table_snapshot(db), before);
                    db.execute_batch("ROLLBACK TO expired_source_closed_audit; RELEASE expired_source_closed_audit").unwrap();
                }
                db.execute_batch(
                    "SAVEPOINT expired_source_historical;
                    UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01';",
                )
                .unwrap();
                db.execute("UPDATE agent_runs SET status='terminal',terminal_state='completed',finished_at=datetime('now','localtime') WHERE id=?1", [&lease.root_run_id]).unwrap();
                let historical = application_table_snapshot(db);
                let audit = if coverage {
                    crate::agent_runtime::multi_agent::source_coverage_reviewer::audit_delivery(
                        db, lease,
                    )?
                    .payload
                } else {
                    crate::agent_runtime::multi_agent::source_reviewer::audit_delivery(db, lease)?
                        .payload
                };
                assert_eq!(audit, payload);
                assert_eq!(application_table_snapshot(db), historical);
                db.execute_batch(
                    "ROLLBACK TO expired_source_historical; RELEASE expired_source_historical",
                )
                .unwrap();
                Ok(payload)
            },
        );
        assert!(result.is_ok(), "{coverage}: {result:?}");
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
