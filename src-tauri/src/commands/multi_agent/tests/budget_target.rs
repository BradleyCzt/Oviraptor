#[test]
fn budget_target_http_claim_and_headers_are_atomic_nonrefundable_sources() {
    use crate::agent_runtime::multi_agent::budget;
    let (root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let db=db::open(&context.db_path).unwrap();
    let root_id:String=db.query_row("SELECT root_run_id FROM agent_runs WHERE id=?1",[&context.run.as_ref().unwrap().run_id],|r|r.get(0)).unwrap();
    let claim=claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    let balance=budget::balance(&db,&root_id,None,"target_requests").unwrap();
    assert_eq!((balance.reserved,balance.consumed,balance.indeterminate),(0,0,1));
    receive_agent_http_headers(&context,&claim,403).unwrap();
    let balance=budget::balance(&db,&root_id,None,"target_requests").unwrap();
    assert_eq!((balance.reserved,balance.consumed,balance.indeterminate),(0,1,0));
    let before=receipt_database_snapshot(&db);
    assert!(receive_agent_http_headers(&context,&claim,200).is_err());
    assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_journal_failure_prevents_http_send_and_rolls_back_claim() {
    for action in ["ABORT,'budget failed'","FAIL,'budget failed'","IGNORE"] {
        let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","hello".into())));
        let (root,context,mut runtime,request)=real_http_transport_fixture(&format!("http://127.0.0.1:{port}/"),0);
        let db=db::open(&context.db_path).unwrap();
        db.execute_batch(&format!("CREATE TRIGGER reject_target_budget BEFORE INSERT ON agent_budget_entries WHEN NEW.dimension='target_requests' BEGIN SELECT RAISE({action}); END;")).unwrap();
        let before=receipt_database_snapshot(&db);
        let result=agent_http_exchange(&context,&mut runtime,&request);
        assert!(result.is_err(),"{action}");
        let error=result.unwrap_err();
        assert_eq!(error["code"],"evidence_write_failed","{action}: {error}");
        if action=="IGNORE" {
            assert_eq!(error["error"],"budget_entry_persistence_conflict","{action}");
        } else {
            assert!(error["error"].as_str().unwrap().contains("budget failed"),
                "{action}: must reach the injected budget write failure: {error}");
        }
        assert_eq!(receipt_database_snapshot(&db),before,"{action}");
        assert_eq!(db.query_row("SELECT count(*) FROM agent_http_request_claims",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert_eq!(runtime.target_requests,0);assert!(seen.lock().unwrap().is_empty());
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn budget_target_late_headers_settle_same_owner_without_restoring_authority() {
    use crate::agent_runtime::multi_agent::budget;
    let (root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let db=db::open(&context.db_path).unwrap();
    let root_id:String=db.query_row("SELECT root_run_id FROM agent_runs WHERE id=?1",[&context.run.as_ref().unwrap().run_id],|r|r.get(0)).unwrap();
    let claim=claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    db.execute("UPDATE agent_runs SET status='terminal' WHERE id=?1",[&root_id]).unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1",[&root_id]).unwrap();
    receive_agent_http_headers(&context,&claim,200).unwrap();
    assert_eq!(budget::balance(&db,&root_id,None,"target_requests").unwrap().consumed,1);
    assert_eq!(db.query_row("SELECT status FROM agent_runs WHERE id=?1",[&root_id],|r|r.get::<_,String>(0)).unwrap(),"terminal");
    assert_eq!(db.query_row("SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[&root_id],|r|r.get::<_,String>(0)).unwrap(),"2000-01-01 00:00:00");
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_replacement_fence_cannot_rebind_old_headers() {
    let (root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let db=db::open(&context.db_path).unwrap();
    let claim=claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token='replacement'",[]).unwrap();
    let original:String=db.query_row("SELECT lease_attempt_id FROM agent_budget_entries WHERE kind='forfeit' AND dimension='target_requests'",[],|r|r.get(0)).unwrap();
    let coordinator:String=db.query_row("SELECT fencing_token FROM agent_coordinator_leases",[],|r|r.get(0)).unwrap();
    // Receipt bookkeeping now preserves the original cost after takeover;
    // it must never adopt the replacement fence or restore executable work.
    receive_agent_http_headers(&context,&claim,200).unwrap();
    assert!(db.query_row("SELECT count(*)=2 AND count(DISTINCT lease_attempt_id)=1 AND min(lease_attempt_id)=?1
        FROM agent_budget_entries WHERE kind IN ('forfeit','reconcile') AND dimension='target_requests'",[original],|r|r.get::<_,bool>(0)).unwrap());
    assert_eq!(db.query_row("SELECT fencing_token FROM agent_coordinator_leases",[],|r|r.get::<_,String>(0)).unwrap(),coordinator);
    assert!(agent_authorize_tool_on(&db,&context,"replay_http").is_err());
    assert_eq!(db.query_row("SELECT response_status FROM agent_http_request_claims",[],|r|r.get::<_,i64>(0)).unwrap(),200);
    let before=super::tests::application_table_snapshot(&db);
    assert!(receive_agent_http_headers(&context,&claim,201).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db),before);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_public_capture_costs_are_bound_to_real_receipts() {
    use crate::agent_runtime::multi_agent::budget;
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_| (200,"text/html","entry".into())));
    let (root,mut context,lease)=public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    let child=public_surface_child(&mut context,&lease);
    capture_public_surface(&context,&lease,&child).unwrap();
    let db=db::open(&context.db_path).unwrap();
    let b=budget::balance(&db,&lease.root_run_id,None,"target_requests").unwrap();
    assert_eq!((b.reserved,b.consumed,b.indeterminate),(0,1,0));
    assert_eq!(seen.lock().unwrap().len(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);drop(db);fs::remove_dir_all(root).unwrap();
}

fn budget_authorization_fixture() -> (PathBuf,AgentRunContext,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,SaveAuthorizationControlInput) {
    budget_authorization_fixture_for_target(None)
}

fn budget_authorization_fixture_for_target(target:Option<&str>) -> (PathBuf,AgentRunContext,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,SaveAuthorizationControlInput) {
    use crate::agent_runtime::{contract::{AgentBackendKind,AgentLane,AgentRole,AgentRunStatus,MultiAgentPolicy},
        multi_agent::{lease,scheduler},store::{self,AgentRunRow}};
    let (root,path,data,mut control)=authorization_fixture();let mut db=db::open(&path).unwrap();
    if let Some(target)=target {
        db.execute("UPDATE sentinel_targets SET url=?1 WHERE scan_id='agent-scan'",[target]).unwrap();
        db.execute("UPDATE browser_auth_sessions SET entry_url=?1,session_json=json_set(session_json,'$.entryUrl',?1,'$.scopeHosts[0]','127.0.0.1')",[target]).unwrap();
        control.target_url=target.into();control.owner_object_url=format!("{target}/api/orders?id=owner");
        control.tester_control_url=format!("{target}/api/orders?id=tester");
    }
    save_authorization_control_in(&mut db,&data,&control).unwrap();
    db.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=2 WHERE id=?1",[&control.scan_id]).unwrap();
    let mut context=test_context(&path,&control.target_url,vec![AgentIdentity::scoped("session-a"),AgentIdentity::scoped("session-b")]);
    context.attempt_number=2;context.execution_plan=context.execution_plan.with_attempt(2);
    let mut run=AgentRunRow::new("budget-auth-root",&control.scan_id,2,&control.target_url,
        AgentBackendKind::Native,AgentRole::Coordinator,context.execution_plan.hash(),"evidence").with_budget(100,1000,2,10);
    run.status=AgentRunStatus::Running;run.root_run_id=run.id.clone();run.orchestration_policy=MultiAgentPolicy::Multi;
    run.lane=Some(AgentLane::ReadOnlyAnalysis);store::create_run(&db,&run).unwrap();freeze_authorization_test_plan(&context);
    let owner=lease::acquire_coordinator_lease(&db,&control.scan_id,2,&control.target_url,&run.id,600).unwrap();
    let child=scheduler::schedule_child(&db,&owner,AgentRole::Authorization,AgentLane::TargetTouching,"budget-auth",
        &serde_json::json!({}),1,&["authorization_probe".into()],0,0).unwrap();
    scheduler::mark_child_running(&db,&owner,&child).unwrap();
    context.run=Some(AgentRunLedger{db_path:path,run_id:child.run_id.clone()});(root,context,child,control)
}

#[test]
fn budget_target_authorization_claim_never_bypasses_journal_or_claim_failure() {
    use crate::agent_runtime::multi_agent::budget;
    for trigger in [
        "CREATE TRIGGER reject_auth BEFORE INSERT ON agent_authorization_probe_claims BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER reject_auth BEFORE INSERT ON agent_budget_entries WHEN NEW.dimension='target_requests' BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER reject_auth BEFORE INSERT ON agent_budget_entries WHEN NEW.dimension='target_requests' BEGIN SELECT RAISE(ABORT,'failed'); END;",
    ] {
        let (root,context,child,control)=budget_authorization_fixture();let db=db::open(&context.db_path).unwrap();
        db.execute_batch(trigger).unwrap();let before=receipt_database_snapshot(&db);
        assert!(claim_authorization_probe(&context,&child,&control,"owner",&control.owner_identity,&control.owner_object_url).is_err());
        assert_eq!(receipt_database_snapshot(&db),before);
        assert_eq!(db.query_row("SELECT count(*) FROM agent_authorization_probe_claims",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(db);fs::remove_dir_all(root).unwrap();
    }
    let (root,context,child,control)=budget_authorization_fixture();let db=db::open(&context.db_path).unwrap();
    claim_authorization_probe(&context,&child,&control,"owner",&control.owner_identity,&control.owner_object_url).unwrap();
    assert_eq!(budget::balance(&db,"budget-auth-root",None,"target_requests").unwrap().indeterminate,1);
    assert!(claim_authorization_probe(&context,&child,&control,"cross",&control.tester_identity,&control.owner_object_url).is_err(),"unknown owner response must block another side");
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_unresolved_journal_cannot_be_orphaned_by_task_deletion() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::{scheduler,budget::{self,Kind}}};
    let (root,path,root_id,lease)=multi_agent_test_root("journal-delete",100,3);let db=db::open(&path).unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::WebExecutor,AgentLane::TargetTouching,"delete",
        &serde_json::json!({}),1,&["replay_http".into()],10,1).unwrap();
    let tx=db.unchecked_transaction().unwrap();
    budget::append(&tx,&lease,&child.assignment_id,"target_requests",Kind::Reserve,1,"target-reserve","durable-claim").unwrap();
    budget::append(&tx,&lease,&child.assignment_id,"target_requests",Kind::Forfeit,1,"target-unknown","durable-claim").unwrap();
    tx.commit().unwrap();scheduler::finish_child(&db,&lease,&child,false,"no model sent").unwrap();
    db.execute("UPDATE agent_runs SET status='completed' WHERE root_run_id=?1",[&root_id]).unwrap();
    // Model summary reports settled. The separate unknown target charge must
    // still prevent deletion even when old status/summary checks all pass.
    db.execute("UPDATE agent_assignments SET budget_settled_at=datetime('now','localtime') WHERE id=?1",[&child.assignment_id]).unwrap();
    db.execute("UPDATE sentinel_scans SET status='completed' WHERE id=?1",[&lease.scan_id]).unwrap();
    let before=receipt_database_snapshot(&db);
    assert!(delete_sentinel_scan_inner(&path,&lease.scan_id).unwrap_err().contains("未结算"));
    assert_eq!(receipt_database_snapshot(&db),before);
    assert_eq!(db.query_row("SELECT count(*) FROM sentinel_scans WHERE id=?1",[&lease.scan_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_authorization_claim_rechecks_revocation_after_insert() {
    let (root,context,child,control)=budget_authorization_fixture();let db=db::open(&context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER revoke_auth AFTER INSERT ON agent_authorization_probe_claims BEGIN
        UPDATE agent_capability_leases SET revoked_at='revoked' WHERE child_run_id=NEW.child_run_id; END;").unwrap();
    let before=receipt_database_snapshot(&db);
    assert!(claim_authorization_probe(&context,&child,&control,"owner",&control.owner_identity,&control.owner_object_url).is_err());
    assert_eq!(receipt_database_snapshot(&db),before);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_authorization_probe_claims",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_unknown_model_cost_blocks_new_target_dispatch() {
    use crate::agent_runtime::multi_agent::budget::{self,Kind};
    let (root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let db=db::open(&context.db_path).unwrap();let tx=db.unchecked_transaction().unwrap();
    let (lease,assignment)=budget::target::child_owner(&tx,&context.run.as_ref().unwrap().run_id).unwrap().unwrap();
    budget::append(&tx,&lease,&assignment,"model_requests",Kind::Forfeit,1,"model-unknown","provider-call").unwrap();tx.commit().unwrap();
    let before=receipt_database_snapshot(&db);
    assert_eq!(claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap_err(),"budget_indeterminate_requires_reconciliation");
    assert_eq!(receipt_database_snapshot(&db),before);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_http_request_claims",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_unknown_target_cost_blocks_an_already_scheduled_model() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::{scheduler,specialist,budget}};
    let (root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let db=db::open(&context.db_path).unwrap();let tx=db.unchecked_transaction().unwrap();
    let (lease,_)=budget::target::child_owner(&tx,&context.run.as_ref().unwrap().run_id).unwrap().unwrap();tx.rollback().unwrap();
    let child=scheduler::schedule_child(&db,&lease,AgentRole::SpaApiMapper,AgentLane::ReadOnlyAnalysis,"before-unknown",
        &serde_json::json!({}),1,&["evidence.read".into()],100,1).unwrap();scheduler::mark_child_running(&db,&lease,&child).unwrap();
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    let before=receipt_database_snapshot(&db);
    assert_eq!(specialist::start(&db,&lease,&child,&serde_json::json!({"messages":[]})).unwrap_err(),"budget_indeterminate_requires_reconciliation");
    assert_eq!(receipt_database_snapshot(&db),before);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_specialist_calls",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_http_claim_rechecks_revocation_before_transport() {
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","entry".into())));
    let (root,context,mut runtime,request)=real_http_transport_fixture(&format!("http://127.0.0.1:{port}/"),0);
    let db=db::open(&context.db_path).unwrap();
    db.execute_batch("CREATE TRIGGER revoke_http AFTER INSERT ON agent_http_request_claims BEGIN
        UPDATE agent_capability_leases SET revoked_at='revoked' WHERE child_run_id=NEW.run_id; END;").unwrap();
    let before=receipt_database_snapshot(&db);
    assert_eq!(agent_http_exchange(&context,&mut runtime,&request).unwrap_err()["code"],"tool_capability_or_fencing_denied");
    assert_eq!(receipt_database_snapshot(&db),before);assert_eq!(runtime.target_requests,0);
    assert!(seen.lock().unwrap().is_empty());stop.store(true,std::sync::atomic::Ordering::SeqCst);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_admission_failures_preserve_truthful_stop_classes() {
    for (reason,code,terminal,status) in [
        ("budget_indeterminate_requires_reconciliation",terminal_code::REQUEST_RECONCILIATION_REQUIRED,terminal_code::REQUEST_RECONCILIATION_REQUIRED,"paused"),
        ("budget_history_requires_reconciliation",terminal_code::REQUEST_RECONCILIATION_REQUIRED,terminal_code::REQUEST_RECONCILIATION_REQUIRED,"paused"),
        ("budget_hard_limit_exceeded","request_budget_exhausted",terminal_code::HARD_REQUEST_BUDGET,"protected_stop"),
        ("budget_clock_origin_conflict",terminal_code::EVIDENCE_INTEGRITY,terminal_code::EVIDENCE_INTEGRITY,"failed"),
        ("budget_wall_time_exhausted","wall_time_budget_exhausted","hard_wall_time_budget","protected_stop"),
    ] {
        let mapped=agent_http_claim_error(reason.into());assert_eq!(mapped["code"],code,"{reason}");
        let outcome=agent_tool_terminal_outcome(&mapped).expect("admission failure must stop this executor");
        assert_eq!(outcome.terminal_code(),terminal,"{reason}");assert_eq!(outcome.terminal_status(),status,"{reason}");
    }
}

#[test]
fn budget_target_diagnostics_show_actual_unknown_charge_without_writes_or_secrets() {
    let (root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
    let db=db::open(&context.db_path).unwrap();let before=receipt_database_snapshot(&db);let changes=db.total_changes();
    let report=native_budget_diagnostics_for_attempt(&db,&context.scan_id,context.attempt_number).unwrap();
    let journal=report["roots"][0]["gaps"]["appendJournal"].as_array().expect("new costs must be observable");
    assert_eq!(journal.len(),10);
    let target=journal.iter().find(|v|v["dimension"]=="target_requests").unwrap();
    assert_eq!((target["reserved"].as_i64(),target["consumed"].as_i64(),target["indeterminate"].as_i64()),(Some(0),Some(0),Some(1)));
    assert_eq!(target["coverage"],"multi_agent_broker");
    assert!(journal.iter().all(|v|v.get("sourceId").is_none() && v.get("leaseAttemptId").is_none()));
    assert_eq!(db.total_changes(),changes);assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);fs::remove_dir_all(root).unwrap();
}

#[test]
fn budget_target_unknown_cost_cannot_be_reported_as_coordinator_complete() {
    use crate::agent_runtime::multi_agent::budget;
    let (root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap();
    let db=db::open(&context.db_path).unwrap();let tx=db.unchecked_transaction().unwrap();
    let (lease,_)=budget::target::child_owner(&tx,&context.run.as_ref().unwrap().run_id).unwrap().unwrap();tx.rollback().unwrap();
    let before=receipt_database_snapshot(&db);
    let outcome=AgentTargetOutcome::Completed(AgentCompletion::without_ledger("reported complete",terminal_code::LEDGER_COMPLETE));
    assert_eq!(finish_coordinator_run(&db,&lease,&outcome).unwrap_err(),"budget_indeterminate_requires_reconciliation");
    assert_eq!(receipt_database_snapshot(&db),before);
    let finalized=finalize_agent_target(&context,&lease,outcome);
    assert_eq!(finalized.terminal_code(),terminal_code::REQUEST_RECONCILIATION_REQUIRED);
    assert_eq!(finalized.terminal_status(),"paused");
    assert_eq!(receipt_database_snapshot(&db),before);
    drop(db);fs::remove_dir_all(root).unwrap();
}
