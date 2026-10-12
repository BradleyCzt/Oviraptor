struct GapReceiptFixture {
    root: std::path::PathBuf,
    context: AgentRunContext,
    session: MultiAgentSession,
    child: crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    input: JsonValue,
    original_web: SpecialistOriginalWebExitSeen,
}

fn gap_receipt_response() -> String {
    serde_json::json!({
        "summary":"needs operator review", "nextStep":"manual_review", "gapCode":"missing_control",
        "supportingFactRefs":[], "missingEvidence":["missing control"], "prerequisites":[],
        "proposedContracts":[], "expectedInformationGain":0.0, "impactCeiling":"low",
        "estimatedCost":{"modelTokens":100,"modelRequests":1,"targetRequests":0},
        "sideEffectClass":"read_only", "overlapKeys":[],
        "falsificationCondition":"control refutes claim", "stopCondition":"operator decision"
    })
    .to_string()
}

fn gap_receipt_fixture() -> GapReceiptFixture {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, context, session) = specialist_gap_fixture();
    let connection = db::open(&context.db_path).unwrap();
    let missing = serde_json::json!(["missing control"]);
    let (candidate, gap, refs) = sealed_gap_review_candidate(
        &connection,
        &session.lease.root_run_id,
        "candidate-gap",
        2,
        &missing,
        &context.target_dir,
    )
    .unwrap();
    let input = serde_json::json!({"candidateId":"candidate-gap","evidenceRevision":2,
        "reviewerMissingEvidence":gap,"trustedFactRefs":refs,"frozenCandidate":candidate});
    let child = scheduler::schedule_child(
        &connection,
        &session.lease,
        AgentRole::DeepInvestigator,
        AgentLane::ReadOnlyAnalysis,
        "reviewer_insufficient_evidence",
        &serde_json::json!({"candidateId":"candidate-gap","revision":2,"missingEvidence":missing}),
        2,
        &["evidence.read".into(), "mailbox.write".into()],
        4000,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &session.lease, &child).unwrap();
    let original_web = specialist_original_take_web_seen(&session.lease.root_run_id);
    assert_eq!(original_web.count(), 1, "one actual original Web SDK before recovery");
    GapReceiptFixture {
        original_web,
        root,
        context,
        session,
        child,
        input,
    }
}

fn recover_gap_fixture(
    f: &GapReceiptFixture,
    connection: &rusqlite::Connection,
) -> Result<(), String> {
    let web_before = f.original_web.count();
    assert_eq!(web_before, 1);
    let result = complete_gap_delivery(
        connection,
        &f.context,
        &f.session,
        &f.child,
        "candidate-gap",
        2,
        &serde_json::json!(["missing control"]),
        &f.input,
        "",
        true,
    );
    assert_eq!(f.original_web.count(), web_before, "local original-receipt recovery cannot resend Web SDK");
    result
}

fn assert_gap_receipt_completed(connection: &rusqlite::Connection) {
    let state:(i64,i64,i64,i64,i64,i64) = connection.query_row(
        "SELECT used_tokens,used_requests, \
         (SELECT COUNT(*) FROM agent_messages WHERE kind IN ('gap_proposed','proposal_assessed') AND acknowledged_at<>'' AND delivery_attempts=1), \
         (SELECT COUNT(*) FROM agent_assignments WHERE role='deep_investigator' AND state='completed' AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0), \
         (SELECT COUNT(*) FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at=''), \
         (SELECT COUNT(*) FROM agent_lane_leases l JOIN agent_assignments a ON a.id=l.assignment_id WHERE a.child_run_id=r.id) \
         FROM agent_runs r WHERE role='deep_investigator' AND terminal_code='gap_receipt_reconciled'",
        [],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
    ).unwrap();
    assert_eq!(state, (20, 1, 2, 1, 0, 0));
    let grants:i64 = connection.query_row("SELECT COUNT(*) FROM agent_messages WHERE kind='proposal_assessed' AND json_extract(payload_json,'$.targetRequestsGranted')=0",[],|row|row.get(0)).unwrap();
    assert_eq!(grants, 1);
}

#[test]
fn investigator_receipt_recovery_is_atomic_without_reactivation_or_recharge() {
    for settled in [false, true] {
        for fault in [
            "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.role='deep_investigator' AND NEW.state='completed'",
            "BEFORE UPDATE OF status ON agent_runs WHEN NEW.terminal_code='gap_receipt_reconciled'",
            "BEFORE INSERT ON agent_messages WHEN NEW.kind='gap_proposed'",
            "BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='gap_proposed'",
            "BEFORE INSERT ON agent_messages WHEN NEW.kind='proposal_assessed'",
            "BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='proposal_assessed'",
            "BEFORE INSERT ON agent_collaboration_events",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='assignment'",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='agent_run'",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='mailbox_message' AND json_extract(NEW.payload_json,'$.kind')='gap_proposed'",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='mailbox_message' AND json_extract(NEW.payload_json,'$.kind')='proposal_assessed'",
            "BEFORE UPDATE OF spent_tokens ON agent_budget_ledger",
            "BEFORE UPDATE OF used_tokens ON agent_runs",
            "BEFORE UPDATE OF budget_settled_at ON agent_assignments",
            "BEFORE DELETE ON agent_lane_leases",
            "",
        ] {
            if settled && (fault.contains("spent_tokens") || fault.contains("used_tokens")
                || fault.contains("budget_settled_at") || fault.contains("agent_lane_leases")) { continue; }
            for action in ["IGNORE","ABORT,'gap recovery fault'"] {
                let mut f = gap_receipt_fixture();
                let (port,seen,_stop) = spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(&gap_receipt_response()))));
                f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
                let connection = db::open(&f.context.db_path).unwrap();
                let (_,usage) = multi_agent_child_round_transport(&f.context,&f.session.lease,&f.child,"gap fixture",f.input.clone()).unwrap();
                if settled { settle_child_usage(&connection,&f.session.lease,&f.child,&usage).unwrap(); }
                stop_failed_child_preserving_usage(&connection,&f.session.lease,&f.child,"delivery interrupted").unwrap();
                connection.execute_batch("CREATE TRIGGER no_reactivation BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='running' BEGIN SELECT RAISE(ABORT,'reactivation'); END;
                    CREATE TRIGGER no_new_capability BEFORE INSERT ON agent_capability_leases BEGIN SELECT RAISE(ABORT,'new capability'); END;
                    CREATE TRIGGER no_restored_capability BEFORE UPDATE OF revoked_at ON agent_capability_leases WHEN NEW.revoked_at='' BEGIN SELECT RAISE(ABORT,'restored capability'); END;
                    CREATE TRIGGER no_lease_renewal BEFORE UPDATE ON agent_coordinator_leases BEGIN SELECT RAISE(ABORT,'lease renewal'); END;").unwrap();
                let before = review_delivery_snapshot(&connection);
                let cost_before: (i64,i64) = connection.query_row("SELECT spent_tokens,spent_requests FROM agent_budget_ledger",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
                if !fault.is_empty() {
                    connection.execute_batch(&format!("CREATE TRIGGER recovery_fault {fault} BEGIN SELECT RAISE({action}); END;")).unwrap();
                    let result = recover_gap_fixture(&f,&connection);
                    assert!(result.is_err(),"{settled}/{fault}/{action}: {result:?}");
                    assert_eq!(before,review_delivery_snapshot(&connection),"{settled}/{fault}/{action}");
                    connection.execute_batch("DROP TRIGGER recovery_fault").unwrap();
                }
                recover_gap_fixture(&f,&connection).unwrap();
                assert_gap_receipt_completed(&connection);
                let cost_after: (i64,i64) = connection.query_row("SELECT spent_tokens,spent_requests FROM agent_budget_ledger",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
                assert_eq!(cost_after, if settled { cost_before } else { (cost_before.0+20,cost_before.1+1) });
                let before = review_delivery_snapshot(&connection);
                recover_gap_fixture(&f,&connection).unwrap();
                assert_eq!(before,review_delivery_snapshot(&connection));
                assert_eq!(seen.lock().unwrap().len(),1);
                drop(connection);
                assert_eq!(f.original_web.count(), 1, "no Web SDK recharge after original recovery/refusal");
                std::fs::remove_dir_all(f.root).unwrap();
            }
        }
    }
}

#[test]
fn investigator_receipt_recovery_rejects_changed_authority_or_saved_input() {
    for change in [
        "UPDATE sentinel_scans SET status='paused'",
        "UPDATE sentinel_scans SET attempt_count=2",
        "UPDATE agent_runs SET status='terminal' WHERE role='coordinator'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE agent_coordinator_leases SET fencing_token='other-owner'",
        "UPDATE agent_specialist_calls SET state='uncertain' WHERE role='deep_investigator'",
        "UPDATE agent_specialist_calls SET response_json='{}' WHERE role='deep_investigator'",
        "UPDATE agent_specialist_calls SET request_json='{}' WHERE role='deep_investigator'",
        "UPDATE agent_specialist_calls SET response_hash='damaged' WHERE role='deep_investigator'",
        "UPDATE agent_specialist_calls SET event_sequence=0 WHERE role='deep_investigator'",
        "UPDATE agent_assignments SET task_slice_json='{}' WHERE role='deep_investigator'",
        "UPDATE agent_lane_leases SET lane='target_touching'",
        "DELETE FROM agent_lane_leases",
        "UPDATE agent_capability_leases SET revoked_at='' WHERE child_run_id IN (SELECT id FROM agent_runs WHERE role='deep_investigator')",
        "UPDATE agent_review_requests SET candidate_json='{}' WHERE candidate_id='candidate-gap'",
        "UPDATE agent_messages SET acknowledged_at='' WHERE kind='review_decision'",
    ] {
        let mut f = gap_receipt_fixture();
        let (port,seen,_stop) = spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(&gap_receipt_response()))));
        f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let connection = db::open(&f.context.db_path).unwrap();
        multi_agent_child_round_transport(&f.context,&f.session.lease,&f.child,"gap fixture",f.input.clone()).unwrap();
        stop_failed_child_preserving_usage(&connection,&f.session.lease,&f.child,"interrupted").unwrap();
        if change.contains("agent_specialist_calls") {
            assert!(connection.execute_batch(change).is_err());
            connection.execute_batch("DROP TRIGGER agent_specialist_call_immutable").unwrap();
        }
        connection.execute_batch(change).unwrap();
        let before = review_delivery_snapshot(&connection);
        assert!(recover_gap_fixture(&f,&connection).is_err(),"{change}");
        assert_eq!(before,review_delivery_snapshot(&connection),"{change}");
        assert_eq!(seen.lock().unwrap().len(),1);
        drop(connection);
        assert_eq!(f.original_web.count(), 1, "no Web SDK recharge after original recovery/refusal");
                std::fs::remove_dir_all(f.root).unwrap();
    }
}

#[test]
fn investigator_production_delivery_recovers_locally_after_normal_path_failure() {
    // The born Root freezes its model endpoint, so the endpoint exists before
    // the actual Root/C/worker fixture and is never swapped afterwards.
    let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&gap_receipt_response()),
        )
    }));
    let (root, context, session) =
        specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
    let original_web = specialist_original_take_web_seen(&session.lease.root_run_id);
    assert_eq!(original_web.count(), 1);
    let connection = db::open(&context.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER fault BEFORE INSERT ON agent_messages WHEN NEW.kind='gap_proposed' AND EXISTS(SELECT 1 FROM agent_runs WHERE id=NEW.from_run_id AND status='running') BEGIN SELECT RAISE(ABORT,'normal delivery unavailable'); END;").unwrap();
    let _real = RealSpecialistTransport::enter();
    multi_agent_investigate_review_gap(
        &context,
        &session,
        "candidate-gap",
        2,
        &serde_json::json!(["missing control"]),
    )
    .unwrap();
    assert_gap_receipt_completed(&connection);
    assert_eq!(specialist_handler_call_count(&seen), 1);
    assert_eq!(original_web.count(), 1, "normal-path failure recovery did not resend original Web SDK");
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

// PROD-BUGFIX-3 guard: local recovery sets aside only the genuine
// Root-published `rootDecision`; a foreign task key or a forged/foreign
// decision binding is still rejected and nothing is delivered.
#[test]
fn investigator_local_recovery_still_rejects_foreign_or_forged_root_task_keys() {
    for (label, tamper) in [
        ("foreign key", "json_set(task_slice_json,'$.foreign','injected')"),
        ("forged summary", "json_set(task_slice_json,'$.rootDecision.summaryHash','forged')"),
        ("foreign sequence", "json_set(task_slice_json,'$.rootDecision.eventSequence',999999)"),
        ("wrong step", "json_set(task_slice_json,'$.rootDecision.step','dispatch:web_executor')"),
        ("extra decision key", "json_set(task_slice_json,'$.rootDecision.extra',1)"),
        ("forged frame", "json_set(task_slice_json,'$.rootDecision.frameHash','forged')"),
        ("forged policy fact", "json_set(task_slice_json,'$.rootDecision.rustPolicy.factHash','forged')"),
    ] {
        let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|_| {
            (200, "application/json", proposal_model_response(&gap_receipt_response()))
        }));
        let (root, context, session) =
            specialist_original_gap_fixture_with_web_exit(&format!("http://127.0.0.1:{port}/v1"));
        let connection = db::open(&context.db_path).unwrap();
        connection.execute_batch("CREATE TRIGGER fault BEFORE INSERT ON agent_messages WHEN NEW.kind='gap_proposed' AND EXISTS(SELECT 1 FROM agent_runs WHERE id=NEW.from_run_id AND status='running') BEGIN SELECT RAISE(ABORT,'normal delivery unavailable'); END;").unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER tamper_task AFTER INSERT ON agent_specialist_calls \
            WHEN NEW.role='deep_investigator' BEGIN UPDATE agent_assignments SET task_slice_json={tamper} WHERE id=NEW.assignment_id; END;")).unwrap();
        let _real = RealSpecialistTransport::enter();
        let error = multi_agent_investigate_review_gap(
            &context, &session, "candidate-gap", 2, &serde_json::json!(["missing control"]),
        ).unwrap_err();
        assert!(error.contains("gap_receipt_task_mismatch"), "{label}: {error}");
        let delivered: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_messages WHERE kind='gap_proposed'", [], |r| r.get(0)).unwrap();
        assert_eq!(delivered, 0, "{label}");
        assert_eq!(specialist_handler_call_count(&seen), 2, "{label}: one original Web and one Investigator SDK, no model retry");
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn investigator_receipt_recovery_concurrent_callers_deliver_once() {
    let mut f = gap_receipt_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&gap_receipt_response()),
        )
    }));
    f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let connection = db::open(&f.context.db_path).unwrap();
    multi_agent_child_round_transport(
        &f.context,
        &f.session.lease,
        &f.child,
        "gap fixture",
        f.input.clone(),
    )
    .unwrap();
    stop_failed_child_preserving_usage(&connection, &f.session.lease, &f.child, "interrupted")
        .unwrap();
    let barrier = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let run = || {
            let db = db::open(&f.context.db_path).unwrap();
            barrier.wait();
            recover_gap_fixture(&f, &db)
        };
        let first = scope.spawn(run);
        let second = scope.spawn(run);
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    });
    assert_gap_receipt_completed(&connection);
    let events:i64 = connection.query_row("SELECT COUNT(*) FROM agent_collaboration_events WHERE event_type='assignment' AND entity_id=?1 AND json_extract(payload_json,'$.state')='completed'",[&f.child.assignment_id],|row|row.get(0)).unwrap();
    assert_eq!(events, 1);
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(connection);
    assert_eq!(f.original_web.count(), 1, "no Web SDK recharge after original recovery/refusal");
                std::fs::remove_dir_all(f.root).unwrap();
}

#[test]
fn investigator_recovery_requires_exact_received_input_and_strict_proposal() {
    for invalid in ["input", "proposal", "task", "caller_input"] {
        let mut f = gap_receipt_fixture();
        let mut response: JsonValue = serde_json::from_str(&gap_receipt_response()).unwrap();
        if invalid == "proposal" {
            response["estimatedCost"]["targetRequests"] = serde_json::json!(3);
        }
        let response = response.to_string();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            (200, "application/json", proposal_model_response(&response))
        }));
        f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let connection = db::open(&f.context.db_path).unwrap();
        let mut sent_input = f.input.clone();
        if invalid == "input" {
            sent_input["reviewerMissingEvidence"] = serde_json::json!(["different gap"]);
        }
        multi_agent_child_round_transport(
            &f.context,
            &f.session.lease,
            &f.child,
            "gap fixture",
            sent_input,
        )
        .unwrap();
        stop_failed_child_preserving_usage(&connection, &f.session.lease, &f.child, "interrupted")
            .unwrap();
        if invalid == "task" {
            connection.execute("UPDATE agent_assignments SET task_slice_json=json_set(task_slice_json,'$.unexpected',1) WHERE id=?1",[&f.child.assignment_id]).unwrap();
        }
        if invalid == "caller_input" {
            f.input["candidateId"] = serde_json::json!("different candidate");
        }
        let before = review_delivery_snapshot(&connection);
        let error = recover_gap_fixture(&f, &connection).unwrap_err();
        assert!(
            error.contains(match invalid {
                "input" => "gap_receipt_frozen_input_mismatch",
                "task" => "gap_receipt_task_mismatch",
                "caller_input" => "gap_review_candidate_changed",
                _ => "gap_proposal_",
            }),
            "{invalid}: {error}"
        );
        assert_eq!(before, review_delivery_snapshot(&connection), "{invalid}");
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(connection);
        assert_eq!(f.original_web.count(), 1, "no Web SDK recharge after original recovery/refusal");
                std::fs::remove_dir_all(f.root).unwrap();
    }
}

#[test]
fn investigator_settled_failure_cleanup_rejects_silently_skipped_updates() {
    for fault in [
        "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='failed'",
        "BEFORE UPDATE OF status ON agent_runs WHEN NEW.terminal_state='failed'",
        "BEFORE UPDATE OF revoked_at ON agent_capability_leases",
        "BEFORE DELETE ON agent_lane_leases",
    ] {
        for action in ["IGNORE", "ABORT,'cleanup fault'"] {
            let mut f = gap_receipt_fixture();
            let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
                (
                    200,
                    "application/json",
                    proposal_model_response(&gap_receipt_response()),
                )
            }));
            f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
            let connection = db::open(&f.context.db_path).unwrap();
            let (_, usage) = multi_agent_child_round_transport(
                &f.context,
                &f.session.lease,
                &f.child,
                "gap fixture",
                f.input.clone(),
            )
            .unwrap();
            settle_child_usage(&connection, &f.session.lease, &f.child, &usage).unwrap();
            let before = review_delivery_snapshot(&connection);
            connection
                .execute_batch(&format!(
                    "CREATE TRIGGER fault {fault} BEGIN SELECT RAISE({action}); END;"
                ))
                .unwrap();
            assert!(
                stop_failed_child_preserving_usage(
                    &connection,
                    &f.session.lease,
                    &f.child,
                    "interrupted"
                )
                .is_err(),
                "{fault}/{action}"
            );
            assert_eq!(
                before,
                review_delivery_snapshot(&connection),
                "{fault}/{action}"
            );
            connection.execute_batch("DROP TRIGGER fault").unwrap();
            stop_failed_child_preserving_usage(
                &connection,
                &f.session.lease,
                &f.child,
                "interrupted",
            )
            .unwrap();
            recover_gap_fixture(&f, &connection).unwrap();
            assert_gap_receipt_completed(&connection);
            assert_eq!(seen.lock().unwrap().len(), 1);
            drop(connection);
            assert_eq!(f.original_web.count(), 1, "no Web SDK recharge after original recovery/refusal");
                std::fs::remove_dir_all(f.root).unwrap();
        }
    }
}
