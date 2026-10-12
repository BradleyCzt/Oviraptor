#[test]
fn directive_closure_real_orchestrator_closes_a_late_message_after_provider_failure() {
    check_directive_closure_provider_failure(false, false);
}

#[test]
fn directive_closure_real_orchestrator_reports_provider_and_finalize_failures() {
    check_directive_closure_provider_failure(true, false);
}

#[test]
fn directive_closure_real_orchestrator_reports_settlement_and_finalize_failures() {
    check_directive_closure_provider_failure(true, true);
}

fn check_directive_closure_provider_failure(fail_finalize: bool, fail_settlement: bool) {
    let _real = RealSpecialistTransport::enter();
    let mut harness = fresh_multi_production_harness("directive-close-orchestrator");
    let path=harness.db_path.clone();
    let created=std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured=created.clone();
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(move |request| {
        if let Some(summary) = fresh_directive_closure_wire_response(&request) {
            return (200, "application/json", summary);
        }
        let connection=db::open(&path).unwrap();
        let (root,target):(String,String)=connection.query_row("SELECT id,target_url FROM agent_runs WHERE role='coordinator' AND root_run_id=id AND status IN ('prepared','running')",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        let lease=connection.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1 AND target_key=?2",params![root,target],|r|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {
            scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
        })).unwrap();
        *captured.lock().unwrap()=confirm_queue_directive(&connection,&lease,"请优先复核已有证据");
        if fail_finalize {
            connection.execute_batch("CREATE TRIGGER fixture_directive_close_failure BEFORE UPDATE OF status ON agent_user_directives WHEN NEW.status='deferred' BEGIN SELECT RAISE(ABORT,'fixture_directive_close_failure'); END;").unwrap();
        }
        if fail_settlement {
            connection.execute_batch("CREATE TRIGGER fixture_settlement_failure BEFORE UPDATE OF spent_tokens ON agent_budget_ledger BEGIN SELECT RAISE(ABORT,'fixture_settlement_failure'); END;
                CREATE TRIGGER fixture_known_bill_revoke AFTER INSERT ON agent_budget_entries
                WHEN NEW.dimension='model_requests' AND NEW.kind='consume' AND EXISTS(
                    SELECT 1 FROM agent_assignments WHERE id=NEW.assignment_id AND role='web_executor')
                BEGIN UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime')
                    WHERE child_run_id=(SELECT child_run_id FROM agent_assignments WHERE id=NEW.assignment_id) AND revoked_at=''; END;").unwrap();
            // Exercise summary failure after the actual Web SDK invoice. An
            // unbilled 401 correctly stops before known-cost settlement now.
            return (200, "application/json", model_round(&[("inspect_evidence", json!({}))], 100));
        }
        (401,"application/json",r#"{"error":{"message":"fixture authentication failure","type":"authentication_error"}}"#.into())
    }));
    harness.context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    freeze_fresh_multi_production_harness(&mut harness);
    let prepared=PreparedFrontendTarget{position:1,route:harness.context.route.clone(),target_dir:harness.context.target_dir.clone(),proxy:None,browser:None};
    let outcome=run_agent_target(&prepared,AgentTargetExecution{
        db_path:&harness.db_path,scan_id:"agent-scan",attempt_number:1,
        settings:&serde_json::json!({"agentBackendPolicy":"native"}),environment:&harness.context.environment,
        adaptive:&AgentBudgetSettings::from_json(&serde_json::json!({})),log_path:&harness.context.log_path,
    }).unwrap().outcome;
    if fail_finalize {assert!(matches!(outcome,AgentTargetOutcome::Failed(_)),"{outcome:?}");}
    else {
        assert_eq!(outcome.terminal_code(),terminal_code::REQUEST_RECONCILIATION_REQUIRED,"{outcome:?}");
        assert_eq!(outcome.terminal_status(),"paused","{outcome:?}");
    }
    let wire = seen.lock().unwrap();
    assert_eq!(wire.len(), 6, "three original paid Root, one Mapper, one ExternalSurface, one actual WebExecutor request: {outcome:?}");
    assert_eq!(wire.iter().filter(|r| fresh_multi_wire_system(r).contains("You are the Root Coordinator")).count(), 3);
    let connection=db::open(&harness.db_path).unwrap();
    let id=created.lock().unwrap().clone();
    assert!(!id.is_empty(),"message must arrive while a real provider request is in flight");
    let row:(String,String,String)=connection.query_row("SELECT r.status,d.status,d.rejection_code FROM agent_user_directives d JOIN agent_runs r ON r.id=d.root_run_id WHERE d.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    if fail_finalize {
        assert_eq!(row,("running".into(),"pending".into(),String::new()), "root and directive must roll back together");
        // The paid revocation trigger is still present: the private Root
        // writer must reject its collateral capability UPDATE even when its
        // WHEN clause would be false for Root. The independent 401 case still
        // exercises the directive-close trigger itself.
        let closure_error = if fail_settlement { "not authorized" } else { "fixture_directive_close_failure" };
        assert!(outcome.detail().contains(closure_error), "{outcome:?}");
        let original_cause = if fail_settlement { "原模型账单已保存" } else { "fixture authentication failure" };
        assert!(outcome.detail().contains(original_cause), "the actual executor cause must survive cleanup failures: {outcome:?}");
        assert_eq!(outcome.terminal_code(), AGENT_STOP_PERSISTENCE);
        if fail_settlement {
            assert!(outcome.detail().contains("fixture_settlement_failure"), "{outcome:?}");
            let unsettled: (String, i64, String) = connection.query_row(
                "SELECT a.state,a.reserved_requests,a.budget_settled_at FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE r.role='web_executor'",
                [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
            ).unwrap();
            assert_eq!(unsettled.0, "paused");
            assert!(unsettled.1 > 0);
            assert!(unsettled.2.is_empty());
            let (assignment, receipt_count, unknown_count): (String, i64, i64) = connection.query_row(
                "SELECT a.id,(SELECT count(*) FROM agent_web_model_journal WHERE assignment_id=a.id AND phase='received'),
                    (SELECT count(*) FROM agent_web_model_journal WHERE assignment_id=a.id AND phase='uncertain')
                 FROM agent_assignments a WHERE role='web_executor'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            ).unwrap();
            assert_eq!((receipt_count, unknown_count), (1, 0));
            let paid = crate::agent_runtime::multi_agent::budget::balance(&connection, &connection.query_row("SELECT coordinator_run_id FROM agent_assignments WHERE id=?1", [&assignment], |r|r.get::<_,String>(0)).unwrap(), Some(&assignment), "model_requests").unwrap();
            assert_eq!((paid.consumed, paid.indeterminate), (1, 0));
        }
    } else {
        assert_eq!(row,("terminal".into(),"deferred".into(),"directive_task_ended_without_action".into()));
    }
}

#[test]
fn directive_closure_orchestrator_reports_bootstrap_and_finalize_failures() {
    let mut harness = fresh_multi_production_harness("directive-close-bootstrap-error");
    freeze_fresh_multi_production_harness(&mut harness);
    let connection = db::open(&harness.db_path).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER fixture_bootstrap_failure BEFORE INSERT ON agent_budget_ledger BEGIN SELECT RAISE(ABORT,'fixture_bootstrap_failure'); END;
         CREATE TRIGGER fixture_finalize_failure BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='terminal' AND NEW.role='coordinator' BEGIN SELECT RAISE(ABORT,'fixture_finalize_failure'); END;"
    ).unwrap();
    fs::write(harness.context.target_dir.join("frontend-evidence.json"), harness.context.evidence.to_string()).unwrap();
    let prepared = PreparedFrontendTarget { position: 1, route: harness.context.route.clone(), target_dir: harness.context.target_dir.clone(), proxy: None, browser: None };
    let outcome = run_agent_target(&prepared, AgentTargetExecution {
        db_path: &harness.db_path, scan_id: "agent-scan", attempt_number: 1,
        settings: &serde_json::json!({"agentBackendPolicy":"native"}), environment: &harness.context.environment,
        adaptive: &AgentBudgetSettings::from_json(&serde_json::json!({})), log_path: &harness.context.log_path,
    }).unwrap().outcome;
    assert!(outcome.detail().contains("fixture_bootstrap_failure"), "{outcome:?}");
    assert!(outcome.detail().contains("fixture_finalize_failure"), "{outcome:?}");
    assert_eq!(outcome.terminal_code(), AGENT_STOP_PERSISTENCE);
    let terminal: i64 = connection.query_row("SELECT COUNT(*) FROM agent_runs WHERE role='coordinator' AND status='terminal'", [], |r| r.get(0)).unwrap();
    assert_eq!(terminal, 0, "failed persistence cannot pretend the root ended");
}

// These fixtures enter the actual SDK for Root and read-only specialists.
// The provider only fails once the actual target executor reaches its request.
fn fresh_directive_closure_wire_response(request: &str) -> Option<String> {
    if let Some(summary) = fresh_multi_root_wire_response(request) { return Some(summary); }
    let system = fresh_multi_wire_system(request);
    let body = if system.contains("SPA/API Mapper") {
        r#"{"summary":"actual local mapper","priorityContracts":[],"risks":[]}"#
    } else if system.contains("External Surface") {
        r#"{"summary":"actual local public entry assessment","observations":[],"coverageGaps":["single anonymous entry only"],"confirmedFindings":false}"#
    } else if system.contains("Evidence Reviewer") {
        r#"{"verdict":"confirmed","reasonCodes":["fixture_confirmed"],"missingEvidence":[],"confidence":1.0,"summary":"actual local reviewer"}"#
    } else { return None; };
    Some(proposal_model_response(body))
}

fn fresh_directive_closure_session(tag: &str) -> (AgentHarness, MultiAgentSession) {
    let mut h = fresh_multi_production_harness(tag);
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        if specialist_original_is_web_sdk(&request) {
            return (200, "application/json", proposal_model_response("actual bounded Web exit; no target tool"));
        }
        (200, "application/json", fresh_directive_closure_wire_response(&request)
            .expect("local closure fixture expects actual Root/Mapper/Reviewer requests"))
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen.clone();
    h.context.execution_plan.max_turns = 1;
    freeze_fresh_multi_production_harness(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 3, "two Root and one actual Mapper");
    (h, session)
}

#[test]
fn directive_closure_review_success_is_not_an_applied_action_receipt() {
    let _real = RealSpecialistTransport::enter();
    use crate::agent_runtime::multi_agent::directive;
    let (h, mut session) = fresh_directive_closure_session("directive-close-review-receipt");
    let connection = db::open(&h.db_path).unwrap();
    let context = &h.context;
    let id = confirmed_directive_for_lease(&connection, &session.lease);
    directive::claim_pending_directives(&connection, &session.lease, 20).unwrap();
    directive::transition_directive(&connection, &session.lease, &id, "claimed", "accepted").unwrap();
    directive::transition_directive(&connection, &session.lease, &id, "accepted", "applied").unwrap();
    let outcome = run_native_agent(context);
    assert!(matches!(outcome, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{outcome:?}");
    assert_eq!(h.model_seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
    multi_agent_finish_execution(context, &mut session, &outcome).unwrap();
    let usage: (i64,i64) = connection.query_row("SELECT used_tokens,used_requests FROM agent_runs WHERE id=?1", [&session.executor.run_id], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(usage,(20,1), "actual Web invoice is settled before checking directive completion");
    let reviewed = multi_agent_review(context, &mut session, outcome.clone());
    assert_eq!(reviewed, outcome);
    finish_coordinator_run(&connection, &session.lease, &reviewed).unwrap();
    let row: (String, String) = connection.query_row("SELECT status,rejection_code FROM agent_user_directives WHERE id=?1", [&id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(row, ("deferred".into(), "directive_task_ended_execution_receipt_missing".into()));
    drop(session);
    drop(connection);
    let _ = fs::remove_dir_all(h.root);
}

#[test]
fn directive_closure_executor_settlement_failure_retains_unsettled_budget() {
    let _real = RealSpecialistTransport::enter();
    let (h, mut session) = fresh_directive_closure_session("directive-close-unsettled");
    let connection = db::open(&h.db_path).unwrap();
    let context = &h.context;
    let root_id = session.lease.root_run_id.clone();
    let before: (i64, i64) = connection.query_row("SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1", [&session.executor.assignment_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert!(before.0 > 0 && before.1 > 0);
    let original_web = run_native_agent(context);
    assert!(matches!(original_web, AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)), "{original_web:?}");
    assert_eq!(h.model_seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
    let usage = {
        let tx = rusqlite::Transaction::new_unchecked(&connection, rusqlite::TransactionBehavior::Immediate).unwrap();
        let usage = crate::agent_runtime::multi_agent::budget::web_lifetime::child_usage_original(
            &tx, &session.lease, &session.executor.assignment_id, &session.executor.run_id,
        ).unwrap();
        tx.commit().unwrap();
        usage
    };
    assert_eq!((usage.total_tokens,usage.model_requests),(20,1), "original receipt usage before coarse settlement: {original_web:?}");
    connection.execute_batch("CREATE TRIGGER fixture_settlement_failure BEFORE UPDATE OF spent_tokens ON agent_budget_ledger BEGIN SELECT RAISE(ABORT,'fixture_settlement_failure'); END;").unwrap();
    let error = multi_agent_finish_execution(context, &mut session, &original_web).unwrap_err();
    assert!(error.contains("fixture_settlement_failure"), "{error}");
    let row: (String, i64, i64, String, String) = connection.query_row("SELECT a.state,a.reserved_tokens,a.reserved_requests,a.budget_settled_at,r.status FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1", [&session.executor.assignment_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).unwrap();
    assert_eq!(row, ("paused".into(), before.0, before.1, String::new(), "paused".into()));
    let caps: i64 = connection.query_row("SELECT COUNT(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''", [&session.executor.run_id], |r| r.get(0)).unwrap();
    assert_eq!(caps, 0);
    let lanes: i64 = connection.query_row("SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1", [&session.executor.assignment_id], |r| r.get(0)).unwrap();
    assert_eq!(lanes, 1, "unsettled execution must not be replaced by another task in the same lane");
    stop_failed_child_preserving_usage(&connection, &session.lease, &session.executor, &error).unwrap();
    finish_coordinator_run(&connection, &session.lease, &AgentTargetOutcome::failed(error)).unwrap();
    let ledger: (i64, i64) = connection.query_row("SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1", [&root_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!(ledger, before);
    drop(session);
    drop(connection);
    let _ = fs::remove_dir_all(h.root);
}

#[test]
fn directive_closure_unsettled_cleanup_is_atomic_and_fenced() {
    let _real = RealSpecialistTransport::enter();
    for stale in [false, true] {
        let (h, session) = fresh_directive_closure_session("directive-close-cleanup-atomic");
        let connection = db::open(&h.db_path).unwrap();
        let before: (i64, i64) = connection.query_row("SELECT reserved_tokens,reserved_requests FROM agent_assignments WHERE id=?1", [&session.executor.assignment_id], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
        if stale {
            connection.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='replacement' WHERE root_run_id=?1", [&session.lease.root_run_id]).unwrap();
        } else {
            connection.execute_batch("CREATE TRIGGER fixture_revoke_failure BEFORE UPDATE OF revoked_at ON agent_capability_leases BEGIN SELECT RAISE(ABORT,'fixture_revoke_failure'); END;").unwrap();
        }
        let error = stop_failed_child_preserving_usage(&connection, &session.lease, &session.executor, "unsettled fixture").unwrap_err();
        assert!(error.contains(if stale { "stale_coordinator_fencing_token" } else { "fixture_revoke_failure" }), "{error}");
        let row: (String, String, i64, i64) = connection.query_row("SELECT a.state,r.status,a.reserved_tokens,a.reserved_requests FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1", [&session.executor.assignment_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!(row, ("running".into(), "running".into(), before.0, before.1));
        let caps: i64 = connection.query_row("SELECT COUNT(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''", [&session.executor.run_id], |r| r.get(0)).unwrap();
        assert!(caps > 0, "partial capability revocation must roll back too");
        drop(session);
        drop(connection);
        let _ = fs::remove_dir_all(h.root);
    }
}
