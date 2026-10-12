#[test]
fn finding_candidate_is_idempotent_and_frozen_during_review() {
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("finding-freeze", 100, 3);
    let connection = db::open(&db_path).unwrap();
    let mut context = test_context(&db_path, &lease.target_key, vec![]);
    context.scan_id = lease.scan_id;
    context.run = Some(AgentRunLedger {
        db_path: db_path.clone(),
        run_id: root_run_id.clone(),
    });
    let original = serde_json::json!({"evidence":"first"});
    let updated = serde_json::json!({"evidence":"new fact"});
    let stage = |value: &serde_json::Value| {
        stage_agent_finding(&context, "web", "authorization", "object-1", "candidate", "medium", value)
    };
    stage(&original).unwrap();
    connection.execute("UPDATE agent_finding_candidates SET status='rejected',candidate_revision=1,reviewer_run_id='review-1' WHERE root_run_id=?1", [&root_run_id]).unwrap();
    stage(&original).unwrap();
    let row: (String, i64, String, String) = connection.query_row(
        "SELECT status,candidate_revision,reviewer_run_id,record_json FROM agent_finding_candidates WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).unwrap();
    assert_eq!((row.0.as_str(), row.1, row.2.as_str()), ("rejected", 1, "review-1"));
    assert_eq!(row.3, original.to_string());

    stage(&updated).unwrap();
    connection.execute("UPDATE agent_finding_candidates SET candidate_revision=2 WHERE root_run_id=?1", [&root_run_id]).unwrap();
    assert_eq!(stage(&original).unwrap_err(), "finding_candidate_review_in_progress");
    let after: (String, i64, String) = connection.query_row(
        "SELECT status,candidate_revision,record_json FROM agent_finding_candidates WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(after, ("pending".into(), 2, updated.to_string()));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn corrupt_finding_candidate_does_not_enter_reviewer_snapshot() {
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("corrupt-finding", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    let mut context = test_context(&db_path, &lease.target_key, vec![]);
    context.scan_id = lease.scan_id;
    context.run = Some(AgentRunLedger { db_path: db_path.clone(), run_id: root_run_id.clone() });
    stage_agent_finding(&context, "web", "evidence", "corrupt", "candidate", "info", &serde_json::json!({"fact":"one"})).unwrap();
    connection.execute("UPDATE agent_finding_candidates SET record_json='{' WHERE root_run_id=?1", [&root_run_id]).unwrap();
    assert!(pending_agent_finding_candidates(&connection, &root_run_id).unwrap_err().contains("无法解码 Agent finding 候选"));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn finding_publication_requires_an_acknowledged_decision_and_exact_snapshot() {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::{mailbox, scheduler}};

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("publish-gate", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    let mut context = test_context(&db_path, &lease.target_key, vec![]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger { db_path: db_path.clone(), run_id: root_run_id.clone() });
    stage_agent_finding(&context, "web", "evidence", "first", "first", "info", &serde_json::json!({"fact":"first"})).unwrap();
    assert_eq!(
        settle_agent_finding_candidates(&connection, &lease, "unassigned", 1, "confirmed", &context.target_dir).unwrap_err(),
        "review_decision_not_acknowledged_or_fenced"
    );
    let frozen = pending_agent_finding_candidates(&connection, &root_run_id).unwrap();
    let reviewer = scheduler::schedule_child(&connection, &lease, AgentRole::EvidenceReviewer, AgentLane::Review,
        "candidate_ready", &serde_json::json!({"candidateId":"gate","candidateRevision":1}), 1,
        &["evidence.read".into(), "review.write".into()], 8_000, 1).unwrap();
    scheduler::mark_child_running(&connection, &lease, &reviewer).unwrap();
    open_review_request(&connection, &lease, &reviewer, "review-gate", "candidate-gate", 1,
        &serde_json::json!({"findingCandidates":frozen}).to_string(), &context.target_dir).unwrap();
    let decision_message = persist_review_decision(&connection, &lease, &reviewer, "review-gate", "candidate-gate", 1,
        "confirmed", &serde_json::json!([]), &serde_json::json!([]), 0.9, "test", &context.target_dir).unwrap();
    assert_eq!(
        settle_agent_finding_candidates(&connection, &lease, &reviewer.run_id, 1, "confirmed", &context.target_dir).unwrap_err(),
        "review_decision_not_acknowledged_or_fenced"
    );
    for message in mailbox::deliver(&connection, &lease, &root_run_id, 20).unwrap() {
        mailbox::acknowledge(&connection, &lease, &root_run_id, &message.id).unwrap();
    }
    assert!(!decision_message.is_empty());
    assert_eq!(
        settle_agent_finding_candidates(&connection, &lease, &reviewer.run_id, 1, "confirmed", &context.target_dir).unwrap_err(),
        "review_decision_not_acknowledged_or_fenced",
        "an acked decision from a still-running child must not publish"
    );
    scheduler::finish_child(&connection, &lease, &reviewer, true, "review completed").unwrap();
    connection.execute_batch(
        "CREATE TRIGGER pause_during_finding_publication BEFORE INSERT ON sentinel_findings \
         BEGIN UPDATE sentinel_scans SET status='pausing' WHERE id=NEW.scan_id; END;",
    ).unwrap();
    assert_eq!(
        settle_agent_finding_candidates(&connection, &lease, &reviewer.run_id, 1, "confirmed", &context.target_dir).unwrap_err(),
        "review_attempt_not_active",
        "a pause inside the publication transaction must roll back the finding",
    );
    let after_rollback: (String, i64) = connection.query_row(
        "SELECT status,(SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1) FROM sentinel_scans WHERE id=?1",
        [&lease.scan_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(after_rollback, ("scanning".into(), 0));
    connection.execute_batch("DROP TRIGGER pause_during_finding_publication;").unwrap();
    stage_agent_finding(&context, "web", "evidence", "late", "late", "info", &serde_json::json!({"fact":"late"})).unwrap();
    assert_eq!(
        settle_agent_finding_candidates(&connection, &lease, &reviewer.run_id, 1, "confirmed", &context.target_dir).unwrap_err(),
        "review_candidate_snapshot_changed"
    );
    let published: i64 = connection.query_row("SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1", [&lease.scan_id], |row| row.get(0)).unwrap();
    assert_eq!(published, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reviewer_manifest_survives_restart_and_refuses_candidate_or_evidence_mutation() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole}, multi_agent::scheduler,
    };
    let (root, db_path, root_run_id, lease) = multi_agent_test_root("review-seal", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    connection.execute(
        "INSERT INTO agent_evidence_revisions(root_run_id,revision,cause_event_id) VALUES(?1,1,'fixture')",
        [&root_run_id],
    ).unwrap();
    let reviewer = scheduler::schedule_child(
        &connection, &lease, AgentRole::EvidenceReviewer, AgentLane::Review,
        "candidate_ready", &serde_json::json!({}), 1,
        &["evidence.read".into(), "review.write".into()], 8_000, 1,
    ).unwrap();
    scheduler::mark_child_running(&connection, &lease, &reviewer).unwrap();
    let frozen = serde_json::json!({"persistedFactRefs":[],"findingCandidates":[]}).to_string();
    open_review_request(
        &connection, &lease, &reviewer, "review-seal", "candidate-seal", 1, &frozen, &root,
    ).unwrap();
    let stored: (i64, String) = connection.query_row(
        "SELECT evidence_revision,manifest_hash FROM agent_review_requests WHERE id='review-seal'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(stored.0, 1);
    assert_eq!(stored.1.len(), 64);
    drop(connection);

    let connection = db::open(&db_path).unwrap();
    verify_review_snapshot(&connection, &root_run_id, "candidate-seal", 1, &root).unwrap();
    connection.execute(
        "UPDATE agent_review_requests SET candidate_json='{}' WHERE id='review-seal'", [],
    ).unwrap();
    assert_eq!(
        verify_review_snapshot(&connection, &root_run_id, "candidate-seal", 1, &root).unwrap_err(),
        "review_manifest_changed_requires_new_revision",
    );
    connection.execute(
        "UPDATE agent_review_requests SET candidate_json=?1 WHERE id='review-seal'", [&frozen],
    ).unwrap();
    connection.execute(
        "INSERT INTO agent_evidence_revisions(root_run_id,revision,parent_revision,cause_event_id) VALUES(?1,2,1,'new-fact')",
        [&root_run_id],
    ).unwrap();
    assert_eq!(
        persist_review_decision(
            &connection, &lease, &reviewer, "review-seal", "candidate-seal", 1,
            "confirmed", &serde_json::json!([]), &serde_json::json!([]), 0.9, "stale", &root,
        ).unwrap_err(),
        "review_manifest_changed_requires_new_revision",
    );
    let decisions: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_review_decisions WHERE candidate_id='candidate-seal'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(decisions, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reviewer_finish_failure_cannot_publish_an_acked_decision() {
    // Actual frozen Web Root/C/worker (original 60,000 tokens / 20 requests).
    let (root, mut context, lease, seen, _stop) =
        multi_agent_original_test_root("review-finish-fault", 60_000, 20);
    let root_run_id = lease.root_run_id.clone();
    let connection = db::open(&context.db_path).unwrap();
    stage_agent_finding(&context, "web", "evidence", "review-finish", "candidate", "info",
        &serde_json::json!({"fact":"frozen"})).unwrap();
    let mut session = multi_agent_original_prepare(&mut context);
    let _real = RealSpecialistTransport::enter();
    let outcome = run_native_agent(&context);
    assert!(matches!(outcome, AgentTargetOutcome::Incomplete(_)), "{outcome:?}");
    let web_calls = seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count();
    assert!(web_calls > 0, "delivery requires an actual original paid Web exit");
    multi_agent_finish_execution(&context, &mut session, &outcome).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER reject_reviewer_completion BEFORE UPDATE OF state ON agent_assignments \
         WHEN NEW.role='evidence_reviewer' AND NEW.state='completed' \
         BEGIN SELECT RAISE(ABORT,'review finish unavailable'); END;",
    ).unwrap();
    let result = multi_agent_review(&context, &mut session, outcome);
    assert!(matches!(result, AgentTargetOutcome::Failed(ref reason)
        if reason.reason.contains("review_gate_finish:") && reason.reason.contains("review finish unavailable")), "{result:?}");
    let state: (String, String, String, String) = connection.query_row(
        "SELECT q.status,a.state,r.terminal_state,fc.status FROM agent_review_requests q \
         JOIN agent_assignments a ON a.id=q.assignment_id \
         JOIN agent_runs r ON r.id=q.reviewer_run_id \
         JOIN agent_finding_candidates fc ON fc.root_run_id=q.root_run_id \
         WHERE q.root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
    ).unwrap();
    assert_eq!(state, ("failed".into(), "failed".into(), "failed".into(), "pending".into()));
    let published: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND kind<>'coverage'", [&lease.scan_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(published, 0, "the failed Reviewer cannot publish a candidate; original Web coverage is not a finding");
    let root_requests = crate::agent_runtime::multi_agent::budget::balance(&connection, &root_run_id, None, "model_requests").unwrap();
    assert_eq!(root_requests.consumed, seen.lock().unwrap().len() as i64, "all real Root, Web and Reviewer costs remain charged");
    let decision_acks: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_messages WHERE root_run_id=?1 AND kind='review_decision' AND acknowledged_at<>''",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(decision_acks, 0, "failed delivery must roll back the decision acknowledgement");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

