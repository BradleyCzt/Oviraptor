// Actual frozen Reviewer HTTP requests and reported invoices, never hand-built
// model receipts. The fault runs at the last finding status write in delivery.
const AUTHORIZATION_REVIEW_PUBLICATION_FIXTURE_TRIGGER: &str =
    "fixture_authorization_review_publication_fault";

#[test]
fn authorization_review_actual_paid_publication_abort_recovers_same_receipt_once() {
    authorization_review_publication_fault_audit(AuthorizationReviewFault::FindingPublicationAbort);
}

#[test]
fn authorization_review_actual_paid_publication_ignore_recovers_same_receipt_once() {
    authorization_review_publication_fault_audit(AuthorizationReviewFault::FindingPublicationIgnore);
}

#[test]
fn authorization_review_actual_paid_publication_source_ack_change_rolls_back_and_recovers_once() {
    authorization_review_publication_fault_audit(AuthorizationReviewFault::FindingPublicationSourceAckChange);
}

fn authorization_review_install_publication_fault(
    db: &rusqlite::Connection,
    finding: &JsonValue,
    scan: &str,
    attempt: i64,
    fault: AuthorizationReviewFault,
) -> Vec<NativeCoordinatorFrozenRows> {
    let transaction = db.unchecked_transaction().unwrap();
    let db = &transaction;
    let id = finding["id"].as_str().unwrap();
    let root: String = db.query_row(
        "SELECT c.root_run_id FROM agent_finding_candidates c JOIN agent_runs root ON root.id=c.root_run_id AND root.role='coordinator' AND root.scan_id=c.scan_id WHERE c.id=?1 AND c.scan_id=?2 AND root.attempt_number=?3 AND c.status='pending' AND c.record_json=?4",
        params![id,scan,attempt,finding["record"].to_string()], |row| row.get(0),
    ).unwrap();
    // SQLite quote() quotes only parameter values. Neither the signed candidate
    // nor the fixture ID is interpolated as arbitrary SQL code.
    let literal: (String, String) = db.query_row("SELECT quote(?1),quote(?2)",
        params![id,root], |row| Ok((row.get(0)?,row.get(1)?))).unwrap();
    let original_source = vec![
        NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,* FROM agent_messages WHERE root_run_id=?1 AND kind='authorization_result' ORDER BY rowid",
            &[&root]).unwrap(),
        NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,* FROM agent_collaboration_events WHERE entity_id IN(SELECT id FROM agent_messages WHERE root_run_id=?1 AND kind='authorization_result') ORDER BY sequence",
            &[&root]).unwrap(),
    ];
    let (timing, statement) = match fault {
        AuthorizationReviewFault::FindingPublicationAbort =>
            ("BEFORE", "SELECT RAISE(ABORT,'original_review_publication_fault');"),
        AuthorizationReviewFault::FindingPublicationIgnore => ("BEFORE", "SELECT RAISE(IGNORE);"),
        // Ordinary UPDATE intentionally emits another original mailbox event.
        // Both this ACK change and that event must roll back with delivery.
        AuthorizationReviewFault::FindingPublicationSourceAckChange => ("AFTER",
            "UPDATE agent_messages SET acknowledged_at='' WHERE root_run_id=NEW.root_run_id AND kind='authorization_result' AND acknowledged_at<>'';"),
        _ => panic!("only explicit publication faults may install this trigger"),
    };
    db.execute_batch(&format!(
        "CREATE TRIGGER {AUTHORIZATION_REVIEW_PUBLICATION_FIXTURE_TRIGGER} {timing} UPDATE OF status ON agent_finding_candidates WHEN NEW.id={} AND NEW.root_run_id={} AND NEW.status='published' BEGIN {statement} END;",
        literal.0,literal.1,
    )).unwrap();
    transaction.commit().unwrap();
    original_source
}

fn authorization_review_publication_fault_audit(fault: AuthorizationReviewFault) {
    run_authorization_control_to_review_audit(
        false, false, 0, 0, false, false, false, true, false, fault,
        |context, session, db, models, targets, reviewed| {
            use crate::agent_runtime::multi_agent::{attempts, budget, specialist};
            let failure = match fault {
                AuthorizationReviewFault::FindingPublicationAbort => "original_review_publication_fault",
                AuthorizationReviewFault::FindingPublicationIgnore => "review_candidate_settlement_write_conflict",
                AuthorizationReviewFault::FindingPublicationSourceAckChange => "mailbox_output_not_consumed",
                _ => unreachable!(),
            };
            assert!(matches!(reviewed, AgentTargetOutcome::Failed(_)), "{reviewed:?}");
            assert!(reviewed.detail().contains(failure) && reviewed.detail().contains(";recovery:"),
                "both delivery and immediate local recovery must fail: {reviewed:?}");
            let root = &session.lease.root_run_id;
            let (candidate, revision, raw, assignment, run): (String, i64, String, String, String) = db.query_row(
                "SELECT candidate_id,candidate_revision,candidate_json,assignment_id,reviewer_run_id FROM agent_review_requests WHERE root_run_id=?1 AND status='failed' AND decision_id IS NULL",
                [root], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)),
            ).unwrap();
            let reviewer = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
                assignment_id: assignment,
                run_id: run,
                role: crate::agent_runtime::contract::AgentRole::EvidenceReviewer,
            };
            assert_eq!(models.lock().unwrap().iter().filter(|request|
                request.contains("你是独立 Evidence Reviewer。你没有网络")).count(), 1);
            assert_eq!(targets.lock().unwrap().as_slice(), [
                "GET /api/orders?id=owner HTTP/1.1", "GET /api/orders?id=owner HTTP/1.1",
                "GET /api/orders?id=tester HTTP/1.1",
            ]);
            let sdk_before = models.lock().unwrap().len();
            let target_before = targets.lock().unwrap().clone();
            let tx = db.unchecked_transaction().unwrap();
            let receipt = specialist::received_for_reconciliation(&tx, &session.lease, &reviewer).unwrap();
            assert_eq!(receipt.rejection, "");
            assert_eq!(validated_review_decision(&receipt.text).unwrap().verdict, "confirmed");
            assert_eq!((receipt.usage.input_tokens, receipt.usage.cached_input_tokens,
                receipt.usage.output_tokens, receipt.usage.total_tokens,
                receipt.usage.model_requests), (10, 0, 10, 20, 1));
            assert!(tx.query_row(
                "SELECT json_extract(response_json,'$.usageReported') FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",
                params![reviewer.assignment_id,reviewer.run_id], |row| row.get::<_,bool>(0),
            ).unwrap());
            let state: (String, bool, i64, i64, String, String) = tx.query_row(
                "SELECT a.state,a.budget_settled_at<>'',a.reserved_tokens,a.reserved_requests,r.status,r.terminal_state FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2",
                params![reviewer.assignment_id,reviewer.run_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
            ).unwrap();
            assert_eq!(state, ("failed".into(), true, 0, 0, "terminal".into(), "failed".into()));
            let worker = attempts::current(&tx, &session.lease, &reviewer.assignment_id).unwrap();
            assert_eq!(worker.state, "failed");
            assert!(!worker.finished_at.is_empty());
            assert!(tx.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='') AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
                params![reviewer.assignment_id,reviewer.run_id], |row| row.get::<_,bool>(0),
            ).unwrap());
            for (dimension, consumed) in [("model_input_tokens", 10), ("model_cached_tokens", 0), ("model_output_tokens", 10), ("model_requests", 1)] {
                let balance = budget::balance(&tx, root, Some(&reviewer.assignment_id), dimension).unwrap();
                assert_eq!((balance.consumed, balance.reserved, balance.indeterminate), (consumed, 0, 0));
            }
            let unpublished: (i64, i64, i64, i64) = tx.query_row(
                "SELECT (SELECT count(*) FROM agent_review_decisions WHERE root_run_id=?1),(SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind='review_decision'),(SELECT count(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND (status='published' OR published_at<>'')),(SELECT count(*) FROM sentinel_findings WHERE scan_id=?2 AND record_json LIKE '%authorization_cross_object_read%')",
                params![root,context.scan_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).unwrap();
            assert_eq!(unpublished, (0, 0, 0, 0));
            let original_rows = [
                NativeCoordinatorFrozenRows::capture(&tx,
                    "SELECT rowid,* FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",
                    &[&reviewer.assignment_id,&reviewer.run_id]).unwrap(),
                NativeCoordinatorFrozenRows::capture(&tx,
                    "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND kind='consume' ORDER BY rowid",
                    &[root,&reviewer.assignment_id]).unwrap(),
                NativeCoordinatorFrozenRows::capture(&tx,
                    "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND kind='reserve' ORDER BY rowid",
                    &[root]).unwrap(),
                NativeCoordinatorFrozenRows::capture(&tx,
                    "SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2 ORDER BY rowid",
                    &[&reviewer.assignment_id,&reviewer.run_id]).unwrap(),
                NativeCoordinatorFrozenRows::capture(&tx,
                    "SELECT rowid,* FROM agent_assignment_attempts WHERE id=?1",
                    &[&worker.id]).unwrap(),
                NativeCoordinatorFrozenRows::capture(&tx,
                    "SELECT rowid,scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token FROM agent_coordinator_leases WHERE root_run_id=?1",
                    &[root]).unwrap(),
                NativeCoordinatorFrozenRows::capture(&tx,
                    "SELECT rowid,* FROM agent_messages WHERE root_run_id=?1 AND kind='authorization_result' ORDER BY rowid",
                    &[root]).unwrap(),
                NativeCoordinatorFrozenRows::capture(&tx,
                    "SELECT rowid,* FROM agent_collaboration_events WHERE entity_id IN(SELECT id FROM agent_messages WHERE root_run_id=?1 AND kind='authorization_result') ORDER BY sequence",
                    &[root]).unwrap(),
            ];
            let frozen: JsonValue = serde_json::from_str(&raw).unwrap();
            native_authorization_original_candidate_on(&tx, &session.lease, &context.target_dir, &frozen).unwrap();
            tx.commit().unwrap();
            let blocked = super::tests::application_table_snapshot(db);
            for _ in 0..2 {
                let error = recover_received_review(db, &session.lease, &candidate, revision, &raw, &context.target_dir).unwrap_err();
                if fault == AuthorizationReviewFault::FindingPublicationSourceAckChange {
                    assert_eq!(error, failure, "the final original source check must reject the changed ACK");
                } else {
                    assert!(error.contains(failure), "{error}");
                }
                assert!(super::tests::application_table_snapshot(db) == blocked,
                    "the fixture fault must roll back each complete local delivery");
                let tx = db.unchecked_transaction().unwrap();
                for row in &original_rows { row.verify(&tx).unwrap(); }
                tx.commit().unwrap();
                assert_eq!(models.lock().unwrap().len(), sdk_before);
                assert_eq!(*targets.lock().unwrap(), target_before);
            }
            // Only remove our installed fixture fault. Original schema, seals,
            // invoices, grants, C and failed worker identity remain untouched.
            assert_eq!(db.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name=?1",
                [AUTHORIZATION_REVIEW_PUBLICATION_FIXTURE_TRIGGER], |row| row.get::<_,i64>(0),
            ).unwrap(), 1);
            db.execute_batch(&format!("DROP TRIGGER {AUTHORIZATION_REVIEW_PUBLICATION_FIXTURE_TRIGGER}")).unwrap();
            let recovered = recover_received_review(db, &session.lease, &candidate, revision, &raw, &context.target_dir).unwrap();
            assert_eq!(recovered.verdict, "confirmed");
            let tx = db.unchecked_transaction().unwrap();
            for row in &original_rows { row.verify(&tx).unwrap(); }
            assert_eq!(attempts::current(&tx, &session.lease, &reviewer.assignment_id).unwrap(), worker);
            specialist::verify_received_original(&tx, &session.lease, &reviewer.assignment_id).unwrap();
            let after_receipt = specialist::received_for_reconciliation(&tx, &session.lease, &reviewer).unwrap();
            assert_eq!(after_receipt.text, receipt.text);
            assert_eq!(after_receipt.usage, receipt.usage);
            assert_eq!(after_receipt.rejection, receipt.rejection);
            let published: (i64, i64, i64, i64) = tx.query_row(
                "SELECT (SELECT count(*) FROM agent_review_decisions WHERE root_run_id=?1 AND reviewer_run_id=?2 AND verdict='confirmed'),(SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?3 AND kind='review_decision' AND delivered_at<>'' AND acknowledged_at<>''),(SELECT count(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND reviewer_run_id=?2 AND status='published' AND published_at<>''),(SELECT count(*) FROM sentinel_findings WHERE scan_id=?4 AND record_json LIKE '%authorization_cross_object_read%')",
                params![root,reviewer.run_id,reviewer.assignment_id,context.scan_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            ).unwrap();
            assert_eq!(published, (1, 1, 1, 1));
            assert!(tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='terminal' AND r.terminal_state='completed')",
                params![reviewer.assignment_id,reviewer.run_id], |row| row.get::<_,bool>(0),
            ).unwrap());
            tx.commit().unwrap();
            assert_eq!(models.lock().unwrap().len(), sdk_before);
            assert_eq!(*targets.lock().unwrap(), target_before);
            let completed = super::tests::application_table_snapshot(db);
            for _ in 0..2 {
                assert_eq!(recover_received_review(db, &session.lease, &candidate, revision, &raw, &context.target_dir).unwrap_err(),
                    "review_receipt_request_not_recoverable");
                assert!(super::tests::application_table_snapshot(db) == completed);
                let replay = replay_frozen_review(context, session, &candidate, revision, &raw,
                    AgentTargetOutcome::incomplete("fixture executor finished without a finding")).unwrap();
                assert!(matches!(replay, AgentTargetOutcome::Incomplete(_)), "{replay:?}");
                assert!(super::tests::application_table_snapshot(db) == completed);
                assert_eq!(models.lock().unwrap().len(), sdk_before);
                assert_eq!(*targets.lock().unwrap(), target_before);
            }
        },
    );
}
