#[test]
fn directive_closure_pending_and_accepted_are_not_reported_as_executed() {
    let (root,context,accepted, _original_parent)=proposal_fixture("directive-close-inbox","请优先复核已有证据");
    let inbox=take_human_directives(&context).unwrap();
    let lease=inbox.lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let pending=confirm_queue_directive(&connection,&lease,"请检查已有证据中的缺口");
    finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::incomplete("fixture ended")).unwrap();
    for id in [&accepted,&pending] {
        let (status,reason):(String,String)=connection.query_row("SELECT status,rejection_code FROM agent_user_directives WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(status,"deferred");
        assert_eq!(reason,"directive_task_ended_without_action");
    }
    let before:i64=connection.query_row("SELECT MAX(sequence) FROM agent_collaboration_events",[],|r|r.get(0)).unwrap();
    finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::incomplete("fixture ended")).unwrap();
    let after:i64=connection.query_row("SELECT MAX(sequence) FROM agent_collaboration_events",[],|r|r.get(0)).unwrap();
    assert_eq!(before,after,"terminal replay cannot manufacture another receipt or event");
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_closure_terminal_root_cannot_claim_start_or_confirm_new_work() {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::{directive,scheduler}};
    let (root,context,id, _original_parent)=proposal_fixture("directive-close-no-resurrection","@mapper 请分析已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=directive::proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    consume_proposal_mailbox(&connection,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap();
    let draft=directive::create_draft(&connection,&lease.scan_id,1,&lease.root_run_id,&lease.target_key,"coordinator","请分析已有证据",lease.lease_epoch,&lease.fencing_token).unwrap();
    finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::incomplete("fixture ended")).unwrap();
    assert!(directive::claim_pending_directives(&connection,&lease,20).is_err());
    assert!(directive::prepare_model_context(&connection,&lease).is_err());
    assert!(directive::proposals::start(&connection,&lease,&id).is_err());
    assert!(directive::confirm_draft(&connection,&lease.scan_id,1,&lease.root_run_id,&lease.target_key,&draft.id,draft.revision,&draft.draft_hash).is_err());
    assert!(scheduler::schedule_child(&connection,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,"after-end",&serde_json::json!({}),1,&["evidence.read".into()],10,1).is_err());
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_closure_unstarted_proposal_releases_only_its_unused_budget() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("directive-close-prepared","@mapper 请分析已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::Cancelled).unwrap();
    let row:(String,String,i64,i64)=connection.query_row("SELECT p.state,a.state,a.reserved_requests,(SELECT spent_requests FROM agent_budget_ledger) FROM agent_directive_proposals p JOIN agent_assignments a ON a.id=p.assignment_id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(row,("failed".into(),"cancelled".into(),0,0));
    let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1",[&job.child.assignment_id],|r|r.get(0)).unwrap();
    assert_eq!(count,0);
    let status=native_scan_status(&connection,&context.scan_id).unwrap();
    let item=status["timeline"].as_array().unwrap().iter().find(|item|item["id"]==id).unwrap();
    assert_eq!(item["taskClosure"]["disposition"],"not_started");
    assert_eq!(item["taskClosure"]["automaticRetry"],false);
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_closure_unknown_call_retains_reservation_without_claiming_completion() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("directive-close-executing","@mapper 请分析已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    consume_proposal_mailbox(&connection,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap();
    proposals::start(&connection,&lease,&id).unwrap();
    finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::Cancelled).unwrap();
    let row:(String,String,String,i64,i64)=connection.query_row("SELECT p.state,a.state,d.status,a.reserved_requests,(SELECT spent_requests FROM agent_budget_ledger) FROM agent_directive_proposals p JOIN agent_assignments a ON a.id=p.assignment_id JOIN agent_user_directives d ON d.id=p.directive_id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(row,("uncertain".into(),"paused".into(),"deferred".into(),1,0));
    let reason:String=connection.query_row("SELECT rejection_code FROM agent_user_directives WHERE id=?1",[&id],|r|r.get(0)).unwrap();
    assert_eq!(reason,"directive_task_ended_outcome_unknown");
    let capabilities:i64=connection.query_row("SELECT COUNT(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''",[&job.child.run_id],|r|r.get(0)).unwrap();
    assert_eq!(capabilities,0);
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_reentry_reconciles_abandoned_proposal_without_model_retry() {
    use crate::agent_runtime::multi_agent::{directive::proposals, lease};
    for replaced_fence in [false, true] {
        let (root, context, id, _original_parent) = proposal_fixture(
            "directive-reentry-unknown", "@mapper 请分析已有证据",
        );
        let original = take_human_directives(&context).unwrap().lease.unwrap();
        let connection = db::open(&context.db_path).unwrap();
        let job = proposals::prepare_next(&connection, &original, &context.evidence)
            .unwrap().unwrap();
        consume_proposal_mailbox(&connection, &original, &job.child.run_id,
            &job.request_message_id, "human_assessment_request", &job.input).unwrap();
        assert!(proposals::start(&connection, &original, &id).unwrap());

        let prior_owner = claim_native_invocation(&context.db_path, &original.scan_id,
            original.attempt_number, "target", &original.target_key).unwrap();
        assert!(claim_native_invocation(&context.db_path, &original.scan_id,
            original.attempt_number, "target", &original.target_key).is_err(),
            "a live target owner must not be replaced");
        drop(prior_owner); // Simulate process exit before recording the response.
        let new_owner = claim_native_invocation(&context.db_path, &original.scan_id,
            original.attempt_number, "target", &original.target_key).unwrap();
        if replaced_fence {
            connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 seconds') \
                WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3",
                params![original.scan_id, original.attempt_number, original.target_key]).unwrap();
        }
        let current = lease::acquire_coordinator_lease(&connection, &original.scan_id,
            original.attempt_number, &original.target_key, &original.root_run_id, 600).unwrap();
        if replaced_fence { assert_ne!(current.fencing_token, original.fencing_token); }
        assert_eq!(reconcile_abandoned_human_proposals(&connection, &current, &new_owner).unwrap(), 1);
        assert_eq!(reconcile_abandoned_human_proposals(&connection, &current, &new_owner).unwrap(), 0);
        let row: (String,String,String,String,i64,i64,String) = connection.query_row(
            "SELECT p.state,a.state,r.status,d.status,a.reserved_requests, \
             (SELECT spent_requests FROM agent_budget_ledger WHERE root_run_id=?2),d.rejection_code \
             FROM agent_directive_proposals p JOIN agent_assignments a ON a.id=p.assignment_id \
             JOIN agent_runs r ON r.id=p.child_run_id JOIN agent_user_directives d ON d.id=p.directive_id \
             WHERE p.directive_id=?1", params![id,current.root_run_id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)),
        ).unwrap();
        assert_eq!(row, ("uncertain".into(),"paused".into(),"paused".into(),"deferred".into(),
            1,0,"proposal_reentry_outcome_unknown".into()));
        let active: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''",
            [&job.child.run_id], |row| row.get(0)).unwrap();
        assert_eq!(active, 0);
        let status = native_scan_status(&connection, &context.scan_id).unwrap();
        let item = status["timeline"].as_array().unwrap().iter()
            .find(|item| item["id"] == id).unwrap();
        assert_eq!(item["proposalAction"]["state"], "uncertain");
        assert_eq!(item["status"], "deferred");
        assert_eq!(item["proposalAction"]["errorCode"], "proposal_reentry_outcome_unknown");
        assert!(proposals::prepare_next(&connection, &current, &context.evidence).unwrap().is_none(),
            "recovered provider call must not be reissued");
        drop(new_owner);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn directive_reentry_silent_state_loss_rolls_back_without_refund() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root, context, id, _original_parent) = proposal_fixture(
        "directive-reentry-rollback", "@mapper 请分析已有证据",
    );
    let lease = take_human_directives(&context).unwrap().lease.unwrap();
    let connection = db::open(&context.db_path).unwrap();
    let job = proposals::prepare_next(&connection, &lease, &context.evidence).unwrap().unwrap();
    consume_proposal_mailbox(&connection, &lease, &job.child.run_id,
        &job.request_message_id, "human_assessment_request", &job.input).unwrap();
    proposals::start(&connection, &lease, &id).unwrap();
    let owner = claim_native_invocation(&context.db_path, &lease.scan_id,
        lease.attempt_number, "target", &lease.target_key).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER ignore_reentry_directive BEFORE UPDATE OF status ON agent_user_directives \
         WHEN NEW.status='deferred' BEGIN SELECT RAISE(IGNORE); END;"
    ).unwrap();
    let before = receipt_database_snapshot(&connection);
    assert!(reconcile_abandoned_human_proposals(&connection, &lease, &owner).is_err());
    assert_eq!(before, receipt_database_snapshot(&connection));
    drop(owner);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn directive_reentry_ignored_lane_release_rolls_back_without_refund() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root, context, id, _original_parent) = proposal_fixture(
        "directive-reentry-lane-rollback", "@mapper 请分析已有证据",
    );
    let lease = take_human_directives(&context).unwrap().lease.unwrap();
    let connection = db::open(&context.db_path).unwrap();
    let job = proposals::prepare_next(&connection, &lease, &context.evidence)
        .unwrap().unwrap();
    consume_proposal_mailbox(
        &connection, &lease, &job.child.run_id, &job.request_message_id,
        "human_assessment_request", &job.input,
    ).unwrap();
    assert!(proposals::start(&connection, &lease, &id).unwrap());
    let owner = claim_native_invocation(
        &context.db_path, &lease.scan_id, lease.attempt_number,
        "target", &lease.target_key,
    ).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER ignore_reentry_lane BEFORE DELETE ON agent_lane_leases \
         WHEN OLD.assignment_id IS NOT NULL BEGIN SELECT RAISE(IGNORE); END;",
    ).unwrap();
    let before = receipt_database_snapshot(&connection);
    assert_eq!(
        reconcile_abandoned_human_proposals(&connection, &lease, &owner).unwrap_err(),
        "directive_reentry_postcondition_conflict",
    );
    assert_eq!(before, receipt_database_snapshot(&connection),
        "a silently ignored lane deletion must roll back every proposal, child and budget change");
    drop(owner);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn directive_reentry_real_target_does_not_retry_unknown_proposal_or_block_mapper() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    for replaced_fence in [false, true] {
        let _real = RealSpecialistTransport::enter();
        let mut h = fresh_multi_production_harness("directive-reentry-real-target");
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
            if let Some(summary) = fresh_multi_root_wire_response(&request) {
                (200, "application/json", summary)
            } else {
                (401, "application/json", r#"{"error":{"message":"fixture mapper failure","type":"authentication_error"}}"#.into())
            }
        }));
        h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
        let adaptive = AgentBudgetSettings::from_json(&serde_json::json!({}));
        freeze_fresh_multi_production_harness(&mut h);
        let original = bootstrap_original_actor(&h);
        let parent = start_original_fixture_supervisor(&h.db_path, &original).unwrap();
        h.context.supervision = Some(parent.ticket());
        let db = db::open(&h.db_path).unwrap();
        let tx =
            rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                .unwrap();
        crate::agent_runtime::multi_agent::budget::root::RootOwner::initialize_control(
            &tx,
            &original.root_run_id,
        )
        .unwrap();
        tx.commit().unwrap();
        let id = confirm_queue_directive(&db, &original, "@mapper 请分析已有证据");
        take_human_directives(&h.context).unwrap();
        let job = proposals::prepare_next(&db, &original, &h.context.evidence)
            .unwrap()
            .unwrap();
        consume_proposal_mailbox(
            &db,
            &original,
            &job.child.run_id,
            &job.request_message_id,
            "human_assessment_request",
            &job.input,
        )
        .unwrap();
        assert!(proposals::start(&db, &original, &id).unwrap());
        let rebuilt = build_agent_execution_plan(
            &adaptive,
            &h.context.route,
            &h.context.environment,
            AgentBackendKind::Native,
            &h.db_path,
            "agent-scan",
        )
        .with_attempt(1);
        assert_eq!(h.context.execution_plan.as_json(), rebuilt.as_json());
        let before = fresh_multi_model_financial_rows(&db);
        drop(parent);
        h.context.supervision = None;
        let old_owner =
            claim_native_invocation(&h.db_path, "agent-scan", 1, "target", &original.target_key)
                .unwrap();
        drop(old_owner); // Simulated death after possible provider effect, never a receipt.
        if replaced_fence {
            db.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 seconds') WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3",
                params![original.scan_id, original.attempt_number, original.target_key]).unwrap();
        }
        let prepared = PreparedFrontendTarget {
            position: 1,
            route: h.context.route.clone(),
            target_dir: h.context.target_dir.clone(),
            proxy: None,
            browser: None,
        };
        let outcome = run_agent_target(
            &prepared,
            AgentTargetExecution {
                db_path: &h.db_path,
                scan_id: "agent-scan",
                attempt_number: 1,
                settings: &serde_json::json!({"agentBackendPolicy":"native"}),
                environment: &h.context.environment,
                adaptive: &adaptive,
                log_path: &h.context.log_path,
            },
        )
        .unwrap()
        .outcome;
        assert_eq!(seen.lock().unwrap().len(), if replaced_fence { 0 } else { 2 },
            "expired original control cannot dispatch; same original C reaches Root then unrelated Mapper, never old proposal retry: {outcome:?}");
        let row: (String, String, String, i64, i64) = db.query_row(
            "SELECT p.state,d.status,d.rejection_code,a.reserved_requests,(SELECT spent_requests FROM agent_budget_ledger WHERE root_run_id=?2) FROM agent_directive_proposals p JOIN agent_user_directives d ON d.id=p.directive_id JOIN agent_assignments a ON a.id=p.assignment_id WHERE d.id=?1",
            params![id, original.root_run_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        if replaced_fence {
            assert_eq!(
                row,
                ("executing".into(), "assigned".into(), String::new(), 1, 0)
            );
            assert_eq!(fresh_multi_model_financial_rows(&db), before,
                "reentry cannot adopt original Root fees, child reservations, limits or coarse projection under a replacement C");
        } else {
            assert_eq!(
                row,
                (
                    "uncertain".into(),
                    "deferred".into(),
                    "proposal_reentry_outcome_unknown".into(),
                    1,
                    0
                )
            );
            let root_calls: i64 = db.query_row("SELECT COUNT(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'", [&original.root_run_id], |r| r.get(0)).unwrap();
            assert_eq!(root_calls, 1);
            assert_eq!(
                crate::agent_runtime::multi_agent::budget::balance(
                    &db,
                    &original.root_run_id,
                    Some(""),
                    "model_requests"
                )
                .unwrap()
                .consumed,
                1
            );
        }
        let lanes: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1",
                [&job.child.assignment_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(lanes, if replaced_fence { 1 } else { 0 });
        drop(db);
        let _ = fs::remove_dir_all(h.root);
    }
}

#[test]
fn directive_closure_silent_proposal_state_loss_rolls_back_entire_root() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    for started in [false, true] {
        let (root, context, id, _original_parent) = proposal_fixture(
            "directive-close-silent-proposal",
            "@mapper 请分析已有证据",
        );
        let lease = take_human_directives(&context).unwrap().lease.unwrap();
        let connection = db::open(&context.db_path).unwrap();
        let job = proposals::prepare_next(&connection, &lease, &context.evidence)
            .unwrap()
            .unwrap();
        if started {
            consume_proposal_mailbox(
                &connection,
                &lease,
                &job.child.run_id,
                &job.request_message_id,
                "human_assessment_request",
                &job.input,
            )
            .unwrap();
            proposals::start(&connection, &lease, &id).unwrap();
        }
        connection.execute_batch(&format!(
            "CREATE TRIGGER ignore_closing_proposal BEFORE UPDATE OF state ON agent_directive_proposals \
             WHEN NEW.state='{}' BEGIN SELECT RAISE(IGNORE); END;",
            if started { "uncertain" } else { "failed" }
        )).unwrap();
        let before = receipt_database_snapshot(&connection);
        assert!(finish_coordinator_run(&connection, &lease, &AgentTargetOutcome::Cancelled).is_err());
        assert_eq!(before, receipt_database_snapshot(&connection));
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn directive_closure_silent_child_finalization_loss_rolls_back_entire_root() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    for (table, column, value) in [
        ("agent_assignments", "state", "cancelled"),
        ("agent_runs", "terminal_state", "cancelled"),
    ] {
        let (root, context, _id, _original_parent) = proposal_fixture(
            "directive-close-silent-child", "@mapper 请分析已有证据",
        );
        let lease = take_human_directives(&context).unwrap().lease.unwrap();
        let connection = db::open(&context.db_path).unwrap();
        proposals::prepare_next(&connection, &lease, &context.evidence).unwrap().unwrap();
        connection.execute_batch(&format!(
            "CREATE TRIGGER ignore_closing_child BEFORE UPDATE OF {column} ON {table} \
             WHEN NEW.{column}='{value}' BEGIN SELECT RAISE(IGNORE); END;"
        )).unwrap();
        let before = receipt_database_snapshot(&connection);
        assert!(finish_coordinator_run(&connection, &lease, &AgentTargetOutcome::Cancelled).is_err(), "{table}");
        assert_eq!(before, receipt_database_snapshot(&connection), "{table}");
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn directive_closure_silent_capability_revocation_rolls_back_entire_root() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root, context, id, _original_parent) = proposal_fixture(
        "directive-close-silent-capability", "@mapper 请分析已有证据",
    );
    let lease = take_human_directives(&context).unwrap().lease.unwrap();
    let connection = db::open(&context.db_path).unwrap();
    let job = proposals::prepare_next(&connection, &lease, &context.evidence).unwrap().unwrap();
    consume_proposal_mailbox(&connection, &lease, &job.child.run_id, &job.request_message_id,
        "human_assessment_request", &job.input).unwrap();
    proposals::start(&connection, &lease, &id).unwrap();
    let active: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at=''",
        [&job.child.run_id], |r| r.get(0),
    ).unwrap();
    assert!(active > 0, "fixture must hold a capability that closure is obliged to revoke");
    connection.execute_batch(&format!(
        "CREATE TRIGGER ignore_closing_capability BEFORE UPDATE OF revoked_at ON agent_capability_leases \
         WHEN NEW.child_run_id='{}' AND NEW.revoked_at<>'' BEGIN SELECT RAISE(IGNORE); END;",
        job.child.run_id,
    )).unwrap();
    let before = receipt_database_snapshot(&connection);
    assert!(finish_coordinator_run(&connection, &lease, &AgentTargetOutcome::Cancelled).is_err());
    assert_eq!(before, receipt_database_snapshot(&connection));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn directive_closure_failure_rolls_back_root_child_budget_and_inbox() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("directive-close-atomic","@mapper 请分析已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    connection.execute_batch("CREATE TRIGGER fail_directive_closure BEFORE UPDATE OF status ON agent_user_directives WHEN NEW.status='deferred' BEGIN SELECT RAISE(ABORT,'injected_closure_failure'); END;").unwrap();
    assert!(finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::Cancelled).unwrap_err().contains("injected_closure_failure"));
    let row:(String,String,String,i64)=connection.query_row("SELECT r.status,p.state,d.status,a.reserved_requests FROM agent_directive_proposals p JOIN agent_assignments a ON a.id=p.assignment_id JOIN agent_runs r ON r.id=a.coordinator_run_id JOIN agent_user_directives d ON d.id=p.directive_id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(row,("running".into(),"prepared".into(),"assigned".into(),1));
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_closure_received_response_is_retained_for_local_reconciliation() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("directive-close-received","@mapper 请分析已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    consume_proposal_mailbox(&connection,&lease,&job.child.run_id,&job.request_message_id,"human_assessment_request",&job.input).unwrap();
    proposals::start(&connection,&lease,&id).unwrap();
    let response=validated_human_proposal(valid_proposal_text(),false);
    let usage=serde_json::json!({"inputTokens":10,"cachedInputTokens":0,"outputTokens":10,"totalTokens":20,"modelRequests":1});
    proposals::record_response(&connection,&lease,&id,&response,&usage).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[&lease.scan_id]).unwrap();
    finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::Cancelled).unwrap();
    let row:(String,String,String,String,i64)=connection.query_row(
        "SELECT p.state,p.response_json,p.usage_json,d.rejection_code,a.reserved_requests FROM agent_directive_proposals p JOIN agent_user_directives d ON d.id=p.directive_id JOIN agent_assignments a ON a.id=p.assignment_id WHERE d.id=?1",[&id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
    assert_eq!(row,("received".into(),response.to_string(),usage.to_string(),"directive_task_ended_receipt_pending".into(),1));
    let status=native_scan_status(&connection,&context.scan_id).unwrap();
    let item=status["timeline"].as_array().unwrap().iter().find(|item|item["id"]==id).unwrap();
    assert_eq!(item["taskClosure"]["disposition"],"receipt_pending");
    assert_eq!(item["taskClosure"]["requiresReconciliation"],true);
    assert!(connection.execute("UPDATE agent_user_directives SET payload_json=json_remove(payload_json,'$.taskClosure') WHERE id=?1",[&id]).unwrap_err().to_string().contains("directive_task_closure_immutable"));
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_closure_old_fence_never_refunds_or_adopts_prior_assignment() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("directive-close-old-fence","@mapper 请分析已有证据");
    let mut lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let job=proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    let before:(i64,i64)=connection.query_row("SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    connection.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='replacement' WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
    lease.lease_epoch+=1;
    lease.fencing_token="replacement".into();
    let all_before=super::tests::application_table_snapshot(&connection);
    let physical_before=final_elapsed_physical_rows(&connection);
    // A live successor cannot close the original worker's Root, adopt its
    // financial identity or rewrite the original directive into a receipt.
    assert_eq!(finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::Cancelled).unwrap_err(),"budget_root_original_owner_conflict");
    assert!(super::tests::application_table_snapshot(&connection)==all_before);
    assert_eq!(final_elapsed_physical_rows(&connection),physical_before);
    let after:(i64,i64)=connection.query_row("SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(before,after);
    let row:(String,String,i64)=connection.query_row("SELECT p.state,a.state,a.lease_epoch FROM agent_directive_proposals p JOIN agent_assignments a ON a.id=p.assignment_id WHERE p.directive_id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(row,("prepared".into(),"leased".into(),lease.lease_epoch-1));
    let reason:String=connection.query_row("SELECT rejection_code FROM agent_user_directives WHERE id=?1",[&id],|r|r.get(0)).unwrap();
    assert_eq!(reason,"");
    assert!(proposals::start(&connection,&lease,&id).is_err());
    assert!(crate::agent_runtime::multi_agent::scheduler::mark_child_running(&connection,&lease,&job.child).is_err());
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_closure_is_exactly_scoped_and_preserves_existing_receipts() {
    let (root,context,id, _original_parent)=proposal_fixture("directive-close-scope","请优先复核已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    // Deliberately minimal rows test the closure selector, not draft execution.
    for (suffix,attempt,target,owner,status) in [
        ("target",1,"https://other.example.test",lease.root_run_id.as_str(),"pending"),
        ("attempt",2,lease.target_key.as_str(),lease.root_run_id.as_str(),"pending"),
        ("root",1,lease.target_key.as_str(),"other-root","pending"),
        ("completed",1,lease.target_key.as_str(),lease.root_run_id.as_str(),"completed"),
        ("deferred",1,lease.target_key.as_str(),lease.root_run_id.as_str(),"deferred"),
    ] {
        connection.execute("INSERT INTO agent_user_directives(id,scan_id,attempt_number,target_key,root_run_id,text_redacted,status,payload_json) VALUES(?1,?2,?3,?4,?5,'scope fixture',?6,'{\"original\":true}')",
            params![suffix,lease.scan_id,attempt,target,owner,status]).unwrap();
    }
    finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::Cancelled).unwrap();
    let state:String=connection.query_row("SELECT status FROM agent_user_directives WHERE id=?1",[&id],|r|r.get(0)).unwrap();
    assert_eq!(state,"deferred");
    for (id,expected) in [("target","pending"),("attempt","pending"),("root","pending"),("completed","completed"),("deferred","deferred")] {
        let row:(String,String)=connection.query_row("SELECT status,payload_json FROM agent_user_directives WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(row,(expected.into(),"{\"original\":true}".into()));
    }
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_closure_terminal_root_denies_existing_executor_tools_and_renewal() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::scheduler};
    let (root,mut context,_, _original_parent)=proposal_fixture("directive-close-tools","请优先复核已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let child=scheduler::schedule_child(&connection,&lease,AgentRole::WebExecutor,AgentLane::TargetTouching,"fixture",&serde_json::json!({}),1,&["inspect_evidence".into()],10,1).unwrap();
    scheduler::mark_child_running(&connection,&lease,&child).unwrap();
    context.run.as_mut().unwrap().run_id=child.run_id.clone();
    context.identities=vec![AgentIdentity::anonymous()];
    assert_eq!(agent_authorize_tool(&context,"inspect_evidence"),Ok(()));
    finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::Cancelled).unwrap();
    assert_eq!(agent_authorize_tool(&context,"inspect_evidence"),Err("worker_supervision_denied"));
    assert_eq!(scheduler::refresh_running_executor_leases(&connection,&child.run_id).unwrap_err(),"coordinator_not_executable");
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_closure_upgrade_installs_cancellation_guard_without_weakening_dispatch() {
    use crate::agent_runtime::multi_agent::directive::proposals;
    let (root,context,id, _original_parent)=proposal_fixture("directive-close-upgrade","@mapper 请分析已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    proposals::prepare_next(&connection,&lease,&context.evidence).unwrap().unwrap();
    connection.execute_batch("DROP TRIGGER agent_directive_proposal_state_guard_v2; CREATE TRIGGER agent_directive_proposal_state_guard BEFORE UPDATE OF state ON agent_directive_proposals BEGIN SELECT RAISE(ABORT,'old_guard'); END;").unwrap();
    crate::collaboration_events::ensure_schema(&connection).unwrap();
    crate::collaboration_events::ensure_schema(&connection).unwrap();
    assert!(connection.execute("UPDATE agent_directive_proposals SET state='failed',error_code='proposal_task_ended_before_dispatch' WHERE directive_id=?1",[&id]).unwrap_err().to_string().contains("directive_proposal_state_conflict"));
    finish_coordinator_run(&connection,&lease,&AgentTargetOutcome::Cancelled).unwrap();
    let _=fs::remove_dir_all(root);
}

#[test]
fn directive_closure_rechecks_fencing_after_waiting_for_the_writer() {
    let (root,context,id, _original_parent)=proposal_fixture("directive-close-writer","请优先复核已有证据");
    let lease=take_human_directives(&context).unwrap().lease.unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let worker=db::open(&context.db_path).unwrap();
    let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).unwrap();
    tx.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='replacement' WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
    let worker_lease=lease.clone();
    let (ready_tx,ready)=std::sync::mpsc::channel();
    let (done_tx,done)=std::sync::mpsc::channel();
    let handle=std::thread::spawn(move || {
        ready_tx.send(()).unwrap();
        done_tx.send(finish_coordinator_run(&worker,&worker_lease,&AgentTargetOutcome::Cancelled)).unwrap();
    });
    ready.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    // The actual shared entry must wait on the held SQLite writer, then read
    // replacement C after acquisition. A caller busy callback is deliberately
    // not inherited by the private connection and cannot be its probe.
    let early=done.recv_timeout(std::time::Duration::from_millis(350));
    tx.commit().unwrap();
    let was_waiting=matches!(early,Err(std::sync::mpsc::RecvTimeoutError::Timeout));
    let result=match early {Ok(result)=>result,Err(_)=>done.recv_timeout(std::time::Duration::from_secs(5)).unwrap()};
    handle.join().unwrap();
    assert!(was_waiting,"must actually wait for the held writer before replacement commits");
    assert_eq!(result.unwrap_err(),"stale_coordinator_fencing_token");
    let row:(String,String)=connection.query_row("SELECT r.status,d.status FROM agent_runs r JOIN agent_user_directives d ON d.root_run_id=r.id WHERE d.id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(row,("running".into(),"accepted".into()));
    let _=fs::remove_dir_all(root);
}
