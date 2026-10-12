#[test]
fn specialist_recovery_pipeline_reuses_completed_and_paused_analysis() {
    for identity in [false, true] {
        let (port,seen,_stop) = specialist_original_mapper_failure_endpoint(std::sync::Arc::new(|request| (200,"application/json",changed_fact_response(&request,true))));
        let (root, mut context, lease) = specialist_original_context("specialist-recovery",&format!("http://127.0.0.1:{port}/v1"));
        let path=context.db_path.clone();
        let connection = db::open(&path).unwrap();
        if identity { configure_specialist_identity(&connection, &lease); }
        let kind = if identity { "identity_assessment" } else { "evidence_summary" };
        connection.execute_batch(&format!("CREATE TRIGGER break_delivery BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='{kind}' BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
        let _real = RealSpecialistTransport::enter();
        assert!(multi_agent_prepare(&mut context).is_err());
        let expected = if identity {2} else {1};
        assert_eq!(specialist_child_call_count(&seen),expected);
        connection.execute_batch("DROP TRIGGER break_delivery;
            CREATE TRIGGER forbid_analysis_reactivation BEFORE UPDATE OF state ON agent_assignments
              WHEN OLD.state IN ('paused','completed') AND NEW.state='running' BEGIN SELECT RAISE(ABORT,'reactivation forbidden'); END;
            CREATE TRIGGER forbid_analysis_regrant BEFORE INSERT ON agent_capability_leases
              WHEN NEW.child_run_id IN (SELECT child_run_id FROM agent_assignments WHERE role IN ('spa_api_mapper','identity_session'))
              BEGIN SELECT RAISE(ABORT,'regrant forbidden'); END;").unwrap();
        let session = multi_agent_prepare(&mut context).unwrap();
        assert_eq!(specialist_child_call_count(&seen),expected,"saved roles must not call the model again");
        let root_calls=if identity {3}else {2};
        assert_eq!(root_tick_count(&connection,&lease.root_run_id,"publication"),root_calls);
        assert_eq!(seen.lock().unwrap().len(),expected+root_calls as usize,"original Root independently assesses each actual closed role output");
        let result:(i64,i64,i64,i64,i64) = connection.query_row(
            "SELECT b.spent_tokens,b.spent_requests,
             (SELECT COUNT(*) FROM agent_assignments WHERE state='completed'),
             (SELECT COUNT(*) FROM agent_messages WHERE kind IN ('evidence_summary','identity_assessment') AND acknowledged_at<>'' AND delivery_attempts=1),
             (SELECT COUNT(*) FROM agent_capability_leases p JOIN agent_assignments a ON a.id=p.assignment_id WHERE a.role IN ('spa_api_mapper','identity_session') AND p.revoked_at='')
             FROM agent_budget_ledger b",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).unwrap();
        assert_eq!(result,(20*expected as i64,expected as i64,expected as i64,expected as i64,0));
        let recovered:i64=connection.query_row("SELECT COUNT(*) FROM agent_runs WHERE terminal_code='readonly_receipt_reconciled'",[],|r|r.get(0)).unwrap();
        assert_eq!(recovered,1);
        assert!(!session.executor.run_id.is_empty());
        let _=fs::remove_dir_all(root);
    }
}

// Guard still bites: only the exact saved-receipt pause (original grant held,
// unsettled, reconciliation failure class, received call only) is adoptable.
#[test]
fn specialist_recovery_paused_mapper_grant_rejects_any_other_paused_projection() {
    // Uncertain/unknown Mapper debt is covered by
    // coordinator_mapper_allocation_actual_unknown_sdk_retains_debt_and_cannot_reallocate
    // (specialist calls are immutable, so it cannot be injected here).
    for damage in ["failure_class", "held_tokens", "settled"] {
        let (port,seen,_stop) = specialist_original_mapper_failure_endpoint(std::sync::Arc::new(|request| (200,"application/json",changed_fact_response(&request,true))));
        let (root, mut context, _lease) = specialist_original_context("specialist-recovery-guard",&format!("http://127.0.0.1:{port}/v1"));
        let connection = db::open(&context.db_path).unwrap();
        connection.execute_batch("CREATE TRIGGER break_delivery BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='evidence_summary' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        let _real = RealSpecialistTransport::enter();
        assert!(multi_agent_prepare(&mut context).is_err());
        connection.execute_batch("DROP TRIGGER break_delivery;").unwrap();
        let state:(String,i64,String,String) = connection.query_row("SELECT state,reserved_tokens,budget_settled_at,failure_class FROM agent_assignments WHERE role='spa_api_mapper'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
        assert_eq!((state.0.as_str(),state.1,state.2.as_str()),("paused",8000,""),"fixture must reach the saved-receipt pause");
        let sql = match damage {
            "failure_class" => "UPDATE agent_assignments SET failure_class='model_error' WHERE role='spa_api_mapper'",
            "held_tokens" => "UPDATE agent_assignments SET reserved_tokens=7999 WHERE role='spa_api_mapper'",
            "settled" => "UPDATE agent_assignments SET budget_settled_at=datetime('now','localtime') WHERE role='spa_api_mapper'",
            _ => unreachable!(),
        };
        connection.execute(sql,[]).unwrap();
        let calls = seen.lock().unwrap().len();
        let error = multi_agent_prepare(&mut context).err().unwrap();
        assert!(error.contains("mapper_allocation_original_grant_conflict"),"{damage}: {error}");
        assert_eq!(seen.lock().unwrap().len(),calls,"{damage}: no model call after a rejected grant");
        let completed:i64 = connection.query_row("SELECT COUNT(*) FROM agent_assignments WHERE role='spa_api_mapper' AND state='completed'",[],|r|r.get(0)).unwrap();
        assert_eq!(completed,0,"{damage}");
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn specialist_recovery_rotated_fence_requires_new_attempt_before_budget_rebinding() {
    for completed_mapper in [false, true] {
        let (port,seen,_stop) = specialist_original_mapper_failure_endpoint(std::sync::Arc::new(move |_| {
            if completed_mapper { (200, "application/json", changed_fact_response("",true)) }
            else { (500, "application/json", "upstream error".to_string()) }
        }));
        let (root, mut context, lease) = specialist_original_context(
            if completed_mapper { "specialist-fence-completed" } else { "specialist-fence-unknown" },
            &format!("http://127.0.0.1:{port}/v1"),
        );
        let path=context.db_path.clone();
        let connection = db::open(&path).unwrap();
        if completed_mapper {
            configure_specialist_identity(&connection, &lease);
            connection.execute_batch("CREATE TRIGGER stop_identity_delivery BEFORE INSERT ON agent_messages
                WHEN NEW.kind='identity_assessment' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        }
        let _real = RealSpecialistTransport::enter();
        assert!(multi_agent_prepare(&mut context).is_err());
        let seen_before = specialist_child_call_count(&seen);
        let all_seen_before=seen.lock().unwrap().len();
        assert_eq!(seen_before, if completed_mapper { 2 } else { 1 });
        if completed_mapper {
            let completed: i64 = connection.query_row(
                "SELECT COUNT(*) FROM agent_assignments WHERE role='spa_api_mapper' AND state='completed'",
                [], |r| r.get(0),
            ).unwrap();
            assert_eq!(completed, 1);
        } else {
            let uncertain: i64 = connection.query_row(
                "SELECT COUNT(*) FROM agent_specialist_calls WHERE state IN ('executing','uncertain')",
                [], |r| r.get(0),
            ).unwrap();
            assert_eq!(uncertain, 1);
        }
        let original: (i64, i64, i64, String, i64) = connection.query_row(
            "SELECT reserved_tokens,reserved_requests,spent_requests,fencing_token,lease_epoch
             FROM agent_budget_ledger", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).unwrap();
        let assignments: i64 = connection.query_row("SELECT COUNT(*) FROM agent_assignments", [], |r| r.get(0)).unwrap();
        connection.execute(
            "UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 second') WHERE root_run_id=?1",
            [&lease.root_run_id],
        ).unwrap();
        let error = multi_agent_prepare(&mut context).err().unwrap();
        assert_eq!(error, "stale_coordinator_fencing_token");
        assert_eq!(specialist_child_call_count(&seen), seen_before, "old model call must not be retried");
        assert_eq!(seen.lock().unwrap().len(),all_seen_before,"no new Root or child SDK after old fencing authority is lost");
        let current: (i64, i64, i64, String, i64) = connection.query_row(
            "SELECT reserved_tokens,reserved_requests,spent_requests,fencing_token,lease_epoch
             FROM agent_budget_ledger", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        ).unwrap();
        assert_eq!(current, original, "failed recovery cannot rebind old budget reservations");
        let after: i64 = connection.query_row("SELECT COUNT(*) FROM agent_assignments", [], |r| r.get(0)).unwrap();
        assert_eq!(after, assignments);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn specialist_recovery_replacement_root_cannot_redo_completed_mapper_in_same_attempt() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy},
        store::{self, AgentRunRow},
    };
    let (port,seen,_stop)=specialist_original_mapper_failure_endpoint(std::sync::Arc::new(|_|(200,"application/json",changed_fact_response("",true))));
    let (root, mut context, lease) = specialist_original_context("specialist-replacement-root",&format!("http://127.0.0.1:{port}/v1"));
    let path=context.db_path.clone();
    let connection=db::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER stop_executor BEFORE INSERT ON agent_assignments
        WHEN NEW.role='web_executor' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let _real=RealSpecialistTransport::enter();
    assert!(multi_agent_prepare(&mut context).is_err());
    assert_eq!(specialist_child_call_count(&seen),1);
    let completed:i64=connection.query_row("SELECT COUNT(*) FROM agent_assignments WHERE role='spa_api_mapper' AND state='completed'",[],|r|r.get(0)).unwrap();
    assert_eq!(completed,1);
    let replacement_id="root-specialist-replacement-new";
    let plan=test_plan_for("standard",&lease.target_key).with_attempt(1);
    let mut replacement=AgentRunRow::new(replacement_id,&lease.scan_id,1,&lease.target_key,
        AgentBackendKind::Native,AgentRole::Coordinator,plan.hash(),"evidence")
        .with_budget(30_000,60_000,10,20);
    replacement.status=AgentRunStatus::Running;
    replacement.root_run_id=replacement_id.into();
    replacement.orchestration_policy=MultiAgentPolicy::Multi;
    replacement.lane=Some(AgentLane::ReadOnlyAnalysis);
    store::create_run(&connection,&replacement).unwrap();
    connection.execute("UPDATE agent_runs SET plan_json=?1 WHERE id=?2",
        rusqlite::params![plan.as_json().to_string(),replacement_id]).unwrap();
    connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at=datetime('now','-1 second') WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
    context.run=Some(AgentRunLedger{db_path:path.clone(),run_id:replacement_id.into()});
    let before=web_mode_test_rows(&connection);
    assert_eq!(multi_agent_prepare(&mut context).err().unwrap(),"web_mode_root_declaration_missing");
    web_mode_assert_rows(&connection,&before);
    assert_eq!(specialist_child_call_count(&seen),1,"replacement root cannot replay the old model request");
    let new_ledger:i64=connection.query_row("SELECT COUNT(*) FROM agent_budget_ledger WHERE root_run_id=?1",[replacement_id],|r|r.get(0)).unwrap();
    assert_eq!(new_ledger,0,"no new budget ledger before history check");
    let _=fs::remove_dir_all(root);
}

#[test]
fn specialist_recovery_paused_delivery_rolls_back_every_partial_write() {
    for action in ["IGNORE","ABORT,'fault'"] {
        for target in [
            "BEFORE UPDATE OF spent_tokens ON agent_budget_ledger",
            "BEFORE UPDATE OF used_tokens ON agent_runs",
            "BEFORE UPDATE OF budget_settled_at ON agent_assignments",
            "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='completed'",
            "BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='terminal'",
            "BEFORE DELETE ON agent_lane_leases",
            "BEFORE INSERT ON agent_messages",
            "BEFORE UPDATE OF acknowledged_at ON agent_messages",
        ] {
            let (root,mut context,lease,child)=specialist_journal_fixture();
            let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response("saved"))));
            context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
            let (text,usage)=multi_agent_child_round_transport(&context,&lease,&child,"readonly",serde_json::json!({})).unwrap();
            let connection=db::open(&context.db_path).unwrap();
            stop_failed_child_preserving_usage(&connection,&lease,&child,"delivery interrupted").unwrap();
            let payload=serde_json::json!({"summary":text});
            connection.execute_batch(&format!("CREATE TRIGGER fault {target} BEGIN SELECT RAISE({action}); END;")).unwrap();
            assert!(complete_readonly_assessment(&connection,&lease,&child,&usage,&payload).is_err(),"{target}/{action}");
            assert_specialist_usage_pending(&connection,"spa_api_mapper",8000);
            let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_messages",[],|r|r.get(0)).unwrap();
            assert_eq!(count,0);
            connection.execute_batch("DROP TRIGGER fault").unwrap();
            for _ in 0..2 { assert_eq!(complete_readonly_assessment(&connection,&lease,&child,&usage,&payload).unwrap(),payload); }
            let result:(i64,i64,i64,i64)=connection.query_row(
                "SELECT spent_tokens,spent_requests,(SELECT COUNT(*) FROM agent_messages WHERE acknowledged_at<>'' AND delivery_attempts=1),(SELECT COUNT(*) FROM agent_lane_leases) FROM agent_budget_ledger",
                [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
            assert_eq!(result,(20,1,1,0));
            assert_eq!(seen.lock().unwrap().len(),1);
            let _=fs::remove_dir_all(root);
        }
    }
}

#[test]
fn specialist_recovery_rejects_mismatched_or_unproven_receipts() {
    for mutation in [
        "DELETE FROM agent_specialist_calls",
        "DELETE FROM agent_events WHERE event_type='model_round_completed'",
        "DELETE FROM agent_snapshots",
        "UPDATE agent_assignments SET failure_class='another_pause_reason'",
        "UPDATE agent_capability_leases SET revoked_at=''",
        "DELETE FROM agent_lane_leases",
        "UPDATE agent_coordinator_leases SET fencing_token='replacement'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01'",
        "UPDATE sentinel_scans SET status='paused'",
        "UPDATE sentinel_scans SET attempt_count=attempt_count+1",
        "UPDATE agent_runs SET status='terminal' WHERE role='coordinator'",
        "UPDATE agent_runs SET target_url='http://other.invalid' WHERE role='spa_api_mapper'",
    ] {
        let (root,mut context,lease,child)=specialist_journal_fixture();
        let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response("saved"))));
        context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
        let (text,usage)=multi_agent_child_round_transport(&context,&lease,&child,"readonly",serde_json::json!({})).unwrap();
        let connection=db::open(&context.db_path).unwrap();
        stop_failed_child_preserving_usage(&connection,&lease,&child,"interrupted").unwrap();
        connection.execute_batch(mutation).unwrap();
        assert!(complete_readonly_assessment(&connection,&lease,&child,&usage,&serde_json::json!({"summary":text})).is_err(),"{mutation}");
        let row:(i64,i64,String)=connection.query_row("SELECT b.spent_tokens,(SELECT COUNT(*) FROM agent_messages),a.state FROM agent_budget_ledger b JOIN agent_assignments a ON a.coordinator_run_id=b.root_run_id",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(row,(0,0,"paused".into()),"{mutation}");
        assert_eq!(seen.lock().unwrap().len(),1);
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn specialist_recovery_payload_and_usage_must_match_saved_result() {
    let (root,mut context,lease,child)=specialist_journal_fixture();
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response("saved"))));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let (text,usage)=multi_agent_child_round_transport(&context,&lease,&child,"readonly",serde_json::json!({})).unwrap();
    let connection=db::open(&context.db_path).unwrap();
    stop_failed_child_preserving_usage(&connection,&lease,&child,"interrupted").unwrap();
    assert!(complete_readonly_assessment(&connection,&lease,&child,&usage,&serde_json::json!({"summary":"forged"})).unwrap_err().contains("receipt_mismatch"));
    let mut wrong=usage;
    wrong.total_tokens+=1;
    assert!(complete_readonly_assessment(&connection,&lease,&child,&wrong,&serde_json::json!({"summary":text})).unwrap_err().contains("receipt_mismatch"));
    assert_specialist_usage_pending(&connection,"spa_api_mapper",8000);
    assert_eq!(seen.lock().unwrap().len(),1);
    let _=fs::remove_dir_all(root);
}

#[test]
fn specialist_recovery_observer_cannot_pause_dispatch_owner() {
    let (arrived_tx,arrived_rx)=std::sync::mpsc::channel();
    let (release_tx,release_rx)=std::sync::mpsc::channel();
    let release_rx=std::sync::Mutex::new(release_rx);
    let (port,seen,_stop)=specialist_original_mapper_failure_endpoint(std::sync::Arc::new(move |_|{
        arrived_tx.send(()).unwrap();
        release_rx.lock().unwrap().recv_timeout(std::time::Duration::from_secs(20)).unwrap();
        (200,"application/json",changed_fact_response("",true))
    }));
    let (root, mut context, lease) = specialist_original_context("specialist-observer",&format!("http://127.0.0.1:{port}/v1"));
    let path=context.db_path.clone();
    let mut owner=context.clone();
    let worker=std::thread::spawn(move || {let _real=RealSpecialistTransport::enter();multi_agent_prepare(&mut owner)});
    if let Err(error)=arrived_rx.recv_timeout(std::time::Duration::from_secs(20)) {
        let _=release_tx.send(());let result=worker.join().unwrap();
        panic!("actual Mapper did not arrive: {error}; {:?}",result.err());
    }
    let _real=RealSpecialistTransport::enter();
    let connection=db::open(&path).unwrap();
    let before=web_mode_test_rows(&connection);
    let error=multi_agent_prepare(&mut context).err().unwrap();
    let row:(String,String,i64)=connection.query_row("SELECT a.state,r.status,(SELECT COUNT(*) FROM agent_capability_leases WHERE revoked_at='') FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let after=web_mode_test_rows(&connection);
    release_tx.send(()).unwrap();
    let result=worker.join().unwrap();
    assert!(error.starts_with("native_invocation_not_owned:"), "{error}");
    assert_eq!(before,after,"observer cannot change any application row");
    assert_eq!(row,("running".into(),"running".into(),2));
    assert!(result.is_ok(),"{:?}",result.err());
    assert_eq!(specialist_child_call_count(&seen),1);
    assert_eq!(root_tick_count(&connection,&lease.root_run_id,"publication"),2);
    let _=fs::remove_dir_all(root);
}

#[test]
fn specialist_recovery_production_delivery_recovers_locally_once() {
    let (port,seen,_stop)=specialist_original_mapper_failure_endpoint(std::sync::Arc::new(|_|(200,"application/json",changed_fact_response("",true))));
    let (root, mut context, lease) = specialist_original_context("specialist-local-retry",&format!("http://127.0.0.1:{port}/v1"));
    let path=context.db_path.clone();
    let connection=db::open(&path).unwrap();
    // Fail the original delivery, not the explicitly reconciled transition.
    // This drives the production prepare -> delivery -> cleanup -> recovery path.
    connection.execute_batch("CREATE TRIGGER fail_original_delivery BEFORE INSERT ON agent_messages
        WHEN NEW.kind='evidence_summary' AND EXISTS(SELECT 1 FROM agent_runs WHERE id=NEW.from_run_id AND terminal_code='child_completed')
        BEGIN SELECT RAISE(IGNORE); END;
        CREATE TRIGGER never_reactivate BEFORE UPDATE OF status ON agent_runs
        WHEN OLD.status='paused' AND NEW.status='running' BEGIN SELECT RAISE(ABORT,'reactivation'); END;").unwrap();
    let _real=RealSpecialistTransport::enter();
    let session=multi_agent_prepare(&mut context).unwrap();
    let code:String=connection.query_row("SELECT terminal_code FROM agent_runs WHERE id=?1",[&session.mapper.run_id],|r|r.get(0)).unwrap();
    assert_eq!(code,"readonly_receipt_reconciled");
    let result:(i64,i64,i64)=connection.query_row("SELECT spent_tokens,spent_requests,(SELECT COUNT(*) FROM agent_messages WHERE kind='evidence_summary' AND acknowledged_at<>'' AND delivery_attempts=1) FROM agent_budget_ledger",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(result,(20,1,1));
    assert_eq!(specialist_child_call_count(&seen),1);
    assert_eq!(root_tick_count(&connection,&lease.root_run_id,"publication"),2);
    assert_eq!(seen.lock().unwrap().len(),3,"actual paid Root/Mapper/changed-fact Root");
    let _=fs::remove_dir_all(root);
}

#[test]
fn specialist_recovery_concurrent_local_completion_settles_once() {
    let (root,mut context,lease,child)=specialist_journal_fixture();
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",proposal_model_response("saved"))));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let (text,usage)=multi_agent_child_round_transport(&context,&lease,&child,"readonly",serde_json::json!({})).unwrap();
    let connection=db::open(&context.db_path).unwrap();
    stop_failed_child_preserving_usage(&connection,&lease,&child,"interrupted").unwrap();
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(3));
    let workers:Vec<_>=(0..2).map(|_| {
        let path=context.db_path.clone();let lease=lease.clone();let child=child.clone();let text=text.clone();let barrier=barrier.clone();
        std::thread::spawn(move || {let connection=db::open(&path).unwrap();barrier.wait();complete_readonly_assessment(&connection,&lease,&child,&usage,&serde_json::json!({"summary":text}))})
    }).collect();
    barrier.wait();
    for worker in workers {worker.join().unwrap().unwrap();}
    let result:(i64,i64,i64)=connection.query_row("SELECT spent_tokens,spent_requests,(SELECT COUNT(*) FROM agent_messages WHERE acknowledged_at<>'' AND delivery_attempts=1) FROM agent_budget_ledger",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(result,(20,1,1));
    assert_eq!(seen.lock().unwrap().len(),1);
    let _=fs::remove_dir_all(root);
}

#[test]
fn specialist_recovery_bootstrap_creation_and_start_are_atomic() {
    use crate::agent_runtime::{contract::AgentRole,multi_agent::scheduler};
    for target in [
        "BEFORE UPDATE OF state ON agent_assignments WHEN NEW.state='running'",
        "BEFORE UPDATE OF status ON agent_runs WHEN NEW.role='spa_api_mapper' AND NEW.status='running'",
    ] {
        let (root,path,_,lease)=multi_agent_test_root("specialist-start-atomic",60_000,20);
        let connection=db::open(&path).unwrap();
        connection.execute_batch(&format!("CREATE TRIGGER fail_start {target} BEGIN SELECT RAISE(IGNORE); END;")).unwrap();
        assert!(scheduler::prepare_readonly_child(&connection,&lease,AgentRole::SpaApiMapper,"test",&serde_json::json!({}),8000).is_err());
        for table in ["agent_assignments","agent_capability_leases","agent_lane_leases","agent_budget_ledger"] {
            let count:i64=connection.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).unwrap();
            assert_eq!(count,0,"{table}/{target}");
        }
        let count:i64=connection.query_row("SELECT COUNT(*) FROM agent_runs WHERE role='spa_api_mapper'",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
        let _=fs::remove_dir_all(root);
    }
}

#[test]
fn specialist_recovery_changed_source_evidence_does_not_resume_or_recall() {
    let (port,seen,_stop)=specialist_original_mapper_failure_endpoint(std::sync::Arc::new(|_|(200,"application/json",changed_fact_response("",true))));
    let (root, mut context, lease) = specialist_original_context("specialist-changed-input",&format!("http://127.0.0.1:{port}/v1"));
    let path=context.db_path.clone();
    let connection=db::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_delivery BEFORE INSERT ON agent_messages WHEN NEW.kind='evidence_summary' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let _real=RealSpecialistTransport::enter();
    assert!(multi_agent_prepare(&mut context).is_err());
    connection.execute_batch("DROP TRIGGER fail_delivery").unwrap();
    context.evidence["newSourceFact"]=serde_json::json!("different input");
    connection.execute_batch("CREATE TRIGGER changed_input_escape AFTER UPDATE ON agent_runs
        WHEN NEW.role='coordinator' BEGIN INSERT INTO projects(id,name) VALUES(9002,'must never be reached'); END;").unwrap();
    let before=web_mode_test_rows(&connection);
    let error=multi_agent_prepare(&mut context).err().unwrap();
    assert_eq!(error,"root_tick_original_evidence_conflict");
    web_mode_assert_rows(&connection,&before);
    assert_specialist_usage_pending(&connection,"spa_api_mapper",8000);
    assert_eq!(specialist_child_call_count(&seen),1);
    assert_eq!(root_tick_count(&connection,&lease.root_run_id,"publication"),1);
    let _=fs::remove_dir_all(root);
}
