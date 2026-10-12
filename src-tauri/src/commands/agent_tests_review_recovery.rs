struct ReviewReceiptFixture {
    root: std::path::PathBuf,
    context: AgentRunContext,
    lease: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    candidate: JsonValue,
}

fn review_receipt_fixture() -> ReviewReceiptFixture {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, path, run_id, lease) = multi_agent_test_root("review-recovery", 60_000, 20);
    let connection = db::open(&path).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger {
        db_path: path,
        run_id: run_id.clone(),
    });
    stage_agent_finding(
        &context,
        "web",
        "evidence",
        "recovery",
        "frozen title",
        "info",
        &serde_json::json!({"fact":"frozen"}),
    )
    .unwrap();
    let child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::EvidenceReviewer,
        AgentLane::Review,
        "candidate_ready",
        &serde_json::json!({"candidateId":"receipt","candidateRevision":1}),
        1,
        &["evidence.read".into(), "review.write".into()],
        8000,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    let candidate = serde_json::json!({"findingCandidates":pending_agent_finding_candidates(&connection,&run_id).unwrap()});
    open_review_request(
        &connection,
        &lease,
        &child,
        "review-receipt",
        "receipt",
        1,
        &candidate.to_string(),
        &context.target_dir,
    )
    .unwrap();
    ReviewReceiptFixture {
        root,
        context,
        lease,
        child,
        candidate,
    }
}

fn review_receipt_response() -> String {
    serde_json::json!({"verdict":"confirmed","reasonCodes":["fixture_evidence"],"missingEvidence":[],
        "confidence":0.9,"summary":"reviewed"}).to_string()
}

fn assert_review_receipt_completed(connection: &rusqlite::Connection) {
    let state:(i64,i64,i64,i64,i64,i64,i64) = connection.query_row(
        "SELECT spent_tokens,spent_requests, \
         (SELECT COUNT(*) FROM agent_finding_candidates WHERE status='published'), \
         (SELECT COUNT(*) FROM sentinel_findings), \
         (SELECT COUNT(*) FROM agent_messages WHERE kind='review_decision' AND acknowledged_at<>'' AND delivery_attempts=1), \
         (SELECT COUNT(*) FROM agent_runs WHERE terminal_code='review_receipt_reconciled'), \
         (SELECT COUNT(*) FROM agent_assignments WHERE state='completed' AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0) \
         FROM agent_budget_ledger",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)),
    ).unwrap();
    assert_eq!(state, (20, 1, 1, 1, 1, 1, 1));
    let live:i64 = connection.query_row("SELECT (SELECT COUNT(*) FROM agent_capability_leases WHERE revoked_at='') + (SELECT COUNT(*) FROM agent_lane_leases)",[],|r|r.get(0)).unwrap();
    assert_eq!(live, 0);
}

#[test]
fn reviewer_receipt_recovery_is_atomic_and_never_reactivates_or_recharges() {
    for settled in [false, true] {
        for fault in [
            "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='completed'",
            "BEFORE UPDATE OF status ON agent_runs WHEN NEW.terminal_code='review_receipt_reconciled'",
            "BEFORE INSERT ON agent_review_decisions",
            "BEFORE UPDATE OF decision_id ON agent_review_requests",
            "BEFORE INSERT ON agent_messages",
            "BEFORE UPDATE OF acknowledged_at ON agent_messages",
            "BEFORE INSERT ON sentinel_findings",
            "BEFORE UPDATE OF status ON agent_finding_candidates WHEN NEW.status='published'",
            "BEFORE INSERT ON agent_collaboration_events",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='agent_run'",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='assignment'",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='mailbox_message'",
            "BEFORE INSERT ON agent_collaboration_events WHEN NEW.event_type='review_gate'",
            "BEFORE UPDATE OF spent_tokens ON agent_budget_ledger",
            "BEFORE UPDATE OF used_tokens ON agent_runs",
            "BEFORE UPDATE OF budget_settled_at ON agent_assignments",
            "BEFORE DELETE ON agent_lane_leases",
            "", // Successful recovery without a fault.
        ] {
            if settled && (fault.contains("spent_tokens") || fault.contains("used_tokens")
                || fault.contains("budget_settled_at") || fault.contains("agent_lane_leases")) { continue; }
            for action in ["IGNORE","ABORT,'receipt recovery fault'"] {
                let mut f = review_receipt_fixture();
                let (port,seen,_stop) = spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response(&review_receipt_response()))));
                f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
                let connection = db::open(&f.context.db_path).unwrap();
                let (_,usage) = multi_agent_child_round_transport(&f.context,&f.lease,&f.child,"review fixture",f.candidate.clone()).unwrap();
                if settled { settle_child_usage(&connection,&f.lease,&f.child,&usage).unwrap(); }
                finish_failed_review(&connection,&f.lease,&f.child,"review-receipt","delivery interrupted").unwrap();
                connection.execute_batch("CREATE TRIGGER no_reactivation BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='running' BEGIN SELECT RAISE(ABORT,'reactivation'); END;
                    CREATE TRIGGER no_new_capability BEFORE INSERT ON agent_capability_leases BEGIN SELECT RAISE(ABORT,'new capability'); END;
                    CREATE TRIGGER no_restored_capability BEFORE UPDATE OF revoked_at ON agent_capability_leases WHEN NEW.revoked_at='' BEGIN SELECT RAISE(ABORT,'restored capability'); END;
                    CREATE TRIGGER no_lease_renewal BEFORE UPDATE ON agent_coordinator_leases BEGIN SELECT RAISE(ABORT,'lease renewal'); END;").unwrap();
                let before = review_delivery_snapshot(&connection);
                if !fault.is_empty() {
                    connection.execute_batch(&format!("CREATE TRIGGER recovery_fault {fault} BEGIN SELECT RAISE({action}); END;")).unwrap();
                    let result = recover_received_review(&connection,&f.lease,"receipt",1,&f.candidate.to_string(),&f.context.target_dir);
                    assert!(result.is_err(),"{settled}/{fault}/{action}: {result:?}");
                    assert_eq!(before,review_delivery_snapshot(&connection),"{settled}/{fault}/{action}");
                    connection.execute_batch("DROP TRIGGER recovery_fault").unwrap();
                }
                recover_received_review(&connection,&f.lease,"receipt",1,&f.candidate.to_string(),&f.context.target_dir).unwrap();
                assert_review_receipt_completed(&connection);
                let before = review_delivery_snapshot(&connection);
                assert!(recover_received_review(&connection,&f.lease,"receipt",1,&f.candidate.to_string(),&f.context.target_dir).is_err());
                assert_eq!(before,review_delivery_snapshot(&connection));
                assert_eq!(seen.lock().unwrap().len(),1);
                drop(connection);
                std::fs::remove_dir_all(f.root).unwrap();
            }
        }
    }
}

#[test]
fn reviewer_receipt_recovery_rejects_changed_authority_or_receipt() {
    for change in [
        "UPDATE sentinel_scans SET status='paused'",
        "UPDATE sentinel_scans SET attempt_count=2",
        "UPDATE agent_runs SET status='terminal' WHERE role='coordinator'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE agent_coordinator_leases SET fencing_token='other-owner'",
        "UPDATE agent_specialist_calls SET state='uncertain'",
        "UPDATE agent_specialist_calls SET response_json='{}'",
        "UPDATE agent_specialist_calls SET request_json='{}'",
        "UPDATE agent_assignments SET task_slice_json='{}'",
        "UPDATE agent_lane_leases SET lane='target_touching'",
        "DELETE FROM agent_lane_leases",
        "UPDATE agent_specialist_calls SET event_sequence=0",
        "UPDATE agent_specialist_calls SET response_hash='damaged'",
        "UPDATE agent_capability_leases SET revoked_at=''",
        "UPDATE agent_finding_candidates SET title='changed'",
        "UPDATE agent_review_requests SET candidate_json='{}'",
    ] {
        let mut f = review_receipt_fixture();
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (
                200,
                "application/json",
                proposal_model_response(&review_receipt_response()),
            )
        }));
        f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let connection = db::open(&f.context.db_path).unwrap();
        multi_agent_child_round_transport(
            &f.context,
            &f.lease,
            &f.child,
            "review fixture",
            f.candidate.clone(),
        )
        .unwrap();
        finish_failed_review(
            &connection,
            &f.lease,
            &f.child,
            "review-receipt",
            "interrupted",
        )
        .unwrap();
        if change.contains("agent_specialist_calls") {
            // Normal writes cannot alter receipts. Simulate damaged storage
            // separately to exercise the recovery reader's integrity checks.
            assert!(connection.execute_batch(change).is_err());
            connection
                .execute_batch("DROP TRIGGER agent_specialist_call_immutable")
                .unwrap();
        }
        connection.execute_batch(change).unwrap();
        let before = review_delivery_snapshot(&connection);
        assert!(
            recover_received_review(
                &connection,
                &f.lease,
                "receipt",
                1,
                &f.candidate.to_string(),
                &f.context.target_dir
            )
            .is_err(),
            "{change}"
        );
        assert_eq!(before, review_delivery_snapshot(&connection), "{change}");
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(connection);
        std::fs::remove_dir_all(f.root).unwrap();
    }
}

#[test]
fn reviewer_receipt_recovery_requires_the_exact_frozen_model_input_and_valid_decision() {
    for invalid_input in [false, true] {
        let mut f = review_receipt_fixture();
        let response = if invalid_input {
            review_receipt_response()
        } else {
            "{}".into()
        };
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
            (200, "application/json", proposal_model_response(&response))
        }));
        f.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let connection = db::open(&f.context.db_path).unwrap();
        let input = if invalid_input {
            serde_json::json!({"findingCandidates":[]})
        } else {
            f.candidate.clone()
        };
        multi_agent_child_round_transport(&f.context, &f.lease, &f.child, "review fixture", input)
            .unwrap();
        finish_failed_review(
            &connection,
            &f.lease,
            &f.child,
            "review-receipt",
            "interrupted",
        )
        .unwrap();
        let before = review_delivery_snapshot(&connection);
        let error = recover_received_review(
            &connection,
            &f.lease,
            "receipt",
            1,
            &f.candidate.to_string(),
            &f.context.target_dir,
        )
        .unwrap_err();
        assert!(
            error.contains(if invalid_input {
                "candidate_input_mismatch"
            } else {
                "review_decision_invalid_shape"
            }),
            "{error}"
        );
        assert_eq!(before, review_delivery_snapshot(&connection));
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(connection);
        std::fs::remove_dir_all(f.root).unwrap();
    }
}

#[test]
fn reviewer_receipt_recovery_runs_in_production_and_replay_checks_actual_publication() {
    for persistent in [false, true] {
        // Actual frozen Web Root/C/worker (original 60,000 tokens / 20
        // requests); the born Root freezes this one model endpoint.
        let (port, seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|request| {
            if specialist_original_is_web_sdk(&request) {
                return (200, "application/json", proposal_model_response("actual bounded Web exit; no target tool"));
            }
            (200, "application/json", proposal_model_response(&review_receipt_response()))
        }));
        let mut born = root_tick_fixture_from_web_mode_turns("review-recovery-production",
            &format!("http://127.0.0.1:{port}/v1"), true, (60_000,20), None,
            web_mode_fixture("multi", "https://authorized.example.test"), Some(1));
        drop(born.parent.take());
        born.f.preserve = true;
        let root = born.f.root.clone();
        let mut context = born.context;
        let connection = db::open(&context.db_path).unwrap();
        stage_agent_finding(
            &context,
            "web",
            "evidence",
            "production",
            "frozen title",
            "info",
            &serde_json::json!({"fact":"frozen"}),
        )
        .unwrap();
        let manual = pending_agent_finding_candidates(&connection, &agent_root_run_id(&context).unwrap()).unwrap();
        assert_eq!(manual.len(), 1, "only the exact staged production slot requires review: {manual:?}");
        assert_eq!(manual[0]["stage"], "web");
        assert_eq!(manual[0]["kind"], "evidence");
        assert_eq!(manual[0]["recordKey"], "production");
        assert_eq!(manual[0]["title"], "frozen title");
        assert_eq!(manual[0]["record"], json!({"fact":"frozen"}));
        let manual_id = manual[0]["id"].as_str().unwrap().to_string();
        let coverage_scan=context.scan_id.clone();
        let coverage_target=context.target_url.clone();
        let coverage_rows = || {
            connection.prepare("SELECT id,scan_id,target_url,stage,kind,record_key,title,severity,record_json,updated_at
                FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage=?3 AND kind='coverage' AND record_key='coverage' ORDER BY id")
                .unwrap().query_map(params![coverage_scan,coverage_target,AGENT_COVERAGE_STAGE], |r|Ok(json!({
                    "id":r.get::<_,i64>(0)?,"scan":r.get::<_,String>(1)?,"target":r.get::<_,String>(2)?,
                    "stage":r.get::<_,String>(3)?,"kind":r.get::<_,String>(4)?,"key":r.get::<_,String>(5)?,
                    "title":r.get::<_,String>(6)?,"severity":r.get::<_,String>(7)?,
                    "record":r.get::<_,String>(8)?,"updatedAt":r.get::<_,String>(9)?})))
                .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
        };
        assert!(coverage_rows().is_empty());
        let mut session = multi_agent_original_prepare(&mut context);
        let actual_web = run_native_agent(&context);
        assert!(matches!(actual_web, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{}", actual_web.detail());
        multi_agent_finish_execution(&context, &mut session, &actual_web).unwrap();
        // Production coverage is an observation allowlist row, never a
        // fabricated Reviewer candidate/confirmed vulnerability.
        let coverage = coverage_rows();
        assert_eq!(coverage.len(), 1, "actual Web coverage baseline: {coverage:?}");
        let coverage_fact: JsonValue = serde_json::from_str(coverage[0]["record"].as_str().unwrap()).unwrap();
        assert_eq!(coverage_fact["source"], "native-agent");
        assert_eq!(coverage_fact["modelRequests"], 1);
        assert_eq!(coverage_fact["totalTokens"], 20);
        assert_eq!(coverage_fact["targetRequests"], 0);
        assert_eq!(pending_agent_finding_candidates(&connection, &session.lease.root_run_id).unwrap(), manual);
        let baseline_findings: i64 = connection.query_row("SELECT count(*) FROM sentinel_findings", [], |r|r.get(0)).unwrap();
        assert_eq!(baseline_findings, 1, "the Web coverage observation is the only pre-review publication");
        let work_calls_before_review = specialist_handler_call_count(&seen);
        assert_eq!(work_calls_before_review, 1, "one actual original Web SDK exit");
        let outcome = || AgentTargetOutcome::incomplete("fixture executor concluded");
        let condition = if persistent {
            "1"
        } else {
            "OLD.state='running'"
        };
        connection
            .execute_batch(&format!(
                "CREATE TRIGGER original_delivery_fault BEFORE UPDATE OF state ON agent_assignments
            WHEN NEW.role='evidence_reviewer' AND NEW.state='completed' AND {condition}
            BEGIN SELECT RAISE(ABORT,'original delivery fault'); END;"
            ))
            .unwrap();
        let _real = RealSpecialistTransport::enter();
        let result = multi_agent_review(&context, &mut session, outcome());
        if persistent {
            assert!(
                matches!(result, AgentTargetOutcome::Failed(_)),
                "{result:?}"
            );
            connection
                .execute_batch("DROP TRIGGER original_delivery_fault")
                .unwrap();
            let result = multi_agent_review(&context, &mut session, outcome());
            assert!(
                matches!(result,AgentTargetOutcome::Incomplete(ref reason) if reason.reason=="fixture executor concluded"),
                "{result:?}"
            );
        } else {
            assert!(
                matches!(result,AgentTargetOutcome::Incomplete(ref reason) if reason.reason=="fixture executor concluded"),
                "{result:?}"
            );
        }
        let reviews: Vec<(String,String,String,i64,String,i64)> = connection.prepare(
            "SELECT q.assignment_id,q.reviewer_run_id,q.candidate_id,q.candidate_revision,q.candidate_json,q.decision_id
             FROM agent_review_requests q JOIN agent_review_decisions d ON d.id=q.decision_id AND d.root_run_id=q.root_run_id
              AND d.candidate_id=q.candidate_id AND d.candidate_revision=q.candidate_revision AND d.reviewer_run_id=q.reviewer_run_id
             WHERE q.root_run_id=?1 AND q.status='confirmed' AND d.verdict='confirmed'")
            .unwrap().query_map([&session.lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
            .unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(reviews.len(), 1, "one original paid Reviewer decision: {reviews:?}");
        let (assignment, reviewer_id, candidate_id, revision, frozen, decision_id) = &reviews[0];
        let frozen:JsonValue = serde_json::from_str(frozen).unwrap();
        assert_eq!(frozen["findingCandidates"], json!(manual), "original SDK input reviews only the staged production candidate");
        let reviewer = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id:assignment.clone(),run_id:reviewer_id.clone(),role:crate::agent_runtime::contract::AgentRole::EvidenceReviewer,
        };
        let tx = connection.unchecked_transaction().unwrap();
        let received = crate::agent_runtime::multi_agent::specialist::received_for_reconciliation(&tx,&session.lease,&reviewer).unwrap();
        assert_eq!(received.text, review_receipt_response());
        assert_eq!((received.usage.total_tokens,received.usage.cached_input_tokens,received.usage.model_requests),(20,0,1));
        crate::agent_runtime::multi_agent::specialist::verify_received_original(&tx,&session.lease,assignment).unwrap();
        for (dimension,consumed) in [("model_input_tokens",10),("model_cached_tokens",0),("model_output_tokens",10),("model_requests",1)] {
            let cost=crate::agent_runtime::multi_agent::budget::balance(&tx,&session.lease.root_run_id,Some(assignment),dimension).unwrap();
            assert_eq!((cost.consumed,cost.reserved,cost.indeterminate),(consumed,0,0));
        }
        let ack:Vec<String> = tx.prepare("SELECT payload_json FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2
            AND from_run_id=?3 AND to_run_id=?1 AND kind='review_decision' AND evidence_revision=?4
            AND delivered_at<>'' AND acknowledged_at<>'' AND delivery_attempts=1").unwrap()
            .query_map(params![session.lease.root_run_id,assignment,reviewer_id,revision],|r|r.get(0)).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(ack.len(),1);
        assert_eq!(serde_json::from_str::<JsonValue>(&ack[0]).unwrap(),json!({"verdict":"confirmed","summary":"reviewed",
            "candidateId":candidate_id,"candidateRevision":revision}));
        let published:Vec<JsonValue> = tx.prepare("SELECT c.id,c.stage,c.kind,c.record_key,c.title,c.severity,c.record_json
            FROM agent_finding_candidates c JOIN sentinel_findings f ON f.scan_id=c.scan_id AND f.target_url=c.target_url
             AND f.stage=c.stage AND f.kind=c.kind AND f.record_key=c.record_key AND f.title=c.title
             AND f.severity=c.severity AND f.record_json=c.record_json WHERE c.id=?1 AND c.root_run_id=?2
             AND c.candidate_revision=?3 AND c.reviewer_run_id=?4 AND c.status='published' AND c.published_at<>''")
            .unwrap().query_map(params![manual_id,session.lease.root_run_id,revision,reviewer_id],|r|{
                let record:String=r.get(6)?;
                Ok(json!({"id":r.get::<_,String>(0)?,"stage":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,
                    "recordKey":r.get::<_,String>(3)?,"title":r.get::<_,String>(4)?,"severity":r.get::<_,String>(5)?,
                    "record":serde_json::from_str::<JsonValue>(&record).unwrap()}))
            }).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(published,manual,"exact original staged slot published by decision {decision_id}");
        let row:(i64,i64,i64) = tx.query_row("SELECT used_tokens,used_requests,(SELECT COUNT(*) FROM sentinel_findings)
            FROM agent_runs WHERE id=?1 AND role='evidence_reviewer' AND terminal_code='review_receipt_reconciled'",
            [reviewer_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(row,(20,1,baseline_findings+i64::try_from(published.len()).unwrap()));
        tx.commit().unwrap();
        assert_eq!(coverage_rows(),coverage,"coverage is unchanged historical observation, not Reviewer publication");
        let original_costs=client_delivery_rows(&connection,"agent_budget_entries");
        let before = super::tests::application_table_snapshot(&connection);
        let sdk=seen.lock().unwrap().len();
        multi_agent_review(&context, &mut session, outcome());
        assert_eq!(before, super::tests::application_table_snapshot(&connection));
        assert_eq!(seen.lock().unwrap().len(),sdk);
        assert_eq!(coverage_rows(),coverage);
        // Same status is insufficient when either the published row or the
        // candidate diverges from what the Reviewer actually saw.
        for corrupt in ["published_title","candidate_and_published_title","missing_publication"] {
            if corrupt=="missing_publication" {
                assert_eq!(connection.execute("DELETE FROM sentinel_findings WHERE scan_id=?1 AND target_url=?2 AND stage='web' AND kind='evidence' AND record_key='production'",
                    params![context.scan_id,context.target_url]).unwrap(),1);
            } else {
                assert_eq!(connection.execute("UPDATE sentinel_findings SET title='tampered' WHERE scan_id=?1 AND target_url=?2 AND stage='web' AND kind='evidence' AND record_key='production'",
                    params![context.scan_id,context.target_url]).unwrap(),1);
                if corrupt=="candidate_and_published_title" {
                    assert_eq!(connection.execute("UPDATE agent_finding_candidates SET title='tampered' WHERE id=?1",[&manual_id]).unwrap(),1);
                }
            }
            let before = super::tests::application_table_snapshot(&connection);
            let result = multi_agent_review(&context,&mut session,outcome());
            assert!(matches!(result,AgentTargetOutcome::Incomplete(ref reason) if reason.reason.contains("publication_incomplete")),"{corrupt}: {result:?}");
            assert_eq!(before,super::tests::application_table_snapshot(&connection));
            assert_eq!(seen.lock().unwrap().len(),sdk);
            assert_eq!(client_delivery_rows(&connection,"agent_budget_entries"),original_costs);
            assert_eq!(coverage_rows(),coverage,"tamper is scoped only to the actual reviewed slot");
            // Restore only the reviewed fixture's exact title. Never overwrite
            // the real Web coverage observation with the artificial test title.
            connection.execute("UPDATE agent_finding_candidates SET title='frozen title' WHERE id=?1",[&manual_id]).unwrap();
            connection.execute("UPDATE sentinel_findings SET title='frozen title' WHERE scan_id=?1 AND target_url=?2 AND stage='web' AND kind='evidence' AND record_key='production'",
                params![context.scan_id,context.target_url]).unwrap();
        }
        assert_eq!(specialist_handler_call_count(&seen) - work_calls_before_review, 1, "one original Reviewer SDK and no recovery recharge");
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}
