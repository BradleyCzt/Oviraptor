fn public_surface_fixture(target: &str) -> (
    std::path::PathBuf, AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let (root,path,run_id,lease) = multi_agent_test_root_for_target("public-surface",60_000,20,target);
    let mut context = test_context(&path,target,vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.external_surface = true;
    context.run = Some(AgentRunLedger {db_path:path.clone(),run_id});
    freeze_authorization_test_plan(&context);
    db::open(&path).unwrap().execute(
        "INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,lease_epoch,fencing_token) VALUES(?1,60000,20,?2,?3)",
        params![lease.root_run_id,lease.lease_epoch,lease.fencing_token],
    ).unwrap();
    (root,context,lease)
}

include!("agent_tests_bootstrap_original.rs");

fn public_surface_child(context: &mut AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::scheduler};
    let connection = db::open(&context.db_path).unwrap();
    let child = scheduler::schedule_child(&connection,lease,AgentRole::ExternalSurface,AgentLane::TargetTouching,
        "anonymous_entry_contract",&public_surface_contract(context).unwrap(),1,
        &["public_surface_get".into(),"evidence.read".into(),"mailbox.write".into()],2000,1).unwrap();
    scheduler::start_child_or_release(&connection,lease,&child).unwrap();
    context.run = Some(AgentRunLedger {db_path:context.db_path.clone(),run_id:child.run_id.clone()});
    child
}

fn public_surface_model_text() -> &'static str {
    r#"{"summary":"Observed anonymous entry only","observations":["entry responded"],"coverageGaps":["authenticated behavior untested"],"confirmedFindings":false}"#
}

#[test]
fn public_surface_real_capture_and_model_deliver_independent_result() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/html","<html>public entry</html>".into())));
    let (_root,mut context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/login#ignored"));
    let (model_port,model_seen,model_stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"application/json",proposal_model_response(public_surface_model_text()))));
    context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
    let _real = RealSpecialistTransport::enter();
    let ceiling = agent_target_request_ceiling(&context);
    let result = multi_agent_external_surface(&context,&lease).unwrap();
    assert_eq!(result["state"],"completed");
    assert_eq!(result["observedCapture"]["capture"]["status"],200);
    assert_eq!(agent_target_request_ceiling(&context),ceiling-1);
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(),1);
    let request = requests[0].to_lowercase();
    assert!(request.starts_with("get /login http/1.1"));
    for denied in ["\r\ncookie:","\r\nauthorization:","\r\norigin:","#ignored"] { assert!(!request.contains(denied)); }
    let models = model_seen.lock().unwrap();
    assert_eq!(models.len(),1);
    let body: JsonValue = serde_json::from_str(models[0].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(body.get("tools").is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty)));
    assert!(body["messages"].to_string().contains("External Surface"));
    assert!(body["messages"].to_string().contains("public entry"));
    let connection = db::open(&context.db_path).unwrap();
    let actual: (String,String,i64,i64,i64,i64) = connection.query_row(
        "SELECT a.state,a.lane,b.spent_requests,b.spent_tokens,
         (SELECT COUNT(*) FROM agent_messages WHERE assignment_id=a.id AND acknowledged_at<>''),
         (SELECT COUNT(*) FROM agent_evidence_nodes WHERE created_by_run_id=a.child_run_id AND provenance='observed')
         FROM agent_assignments a JOIN agent_budget_ledger b ON b.root_run_id=a.coordinator_run_id WHERE a.role='external_surface'",
        [],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)),
    ).unwrap();
    assert_eq!(actual,("completed".into(),"target_touching".into(),1,20,2,1));
    let active:i64 = connection.query_row("SELECT COUNT(*) FROM agent_capability_leases WHERE revoked_at=''",[],|r|r.get(0)).unwrap();
    assert_eq!(active,0);
    assert_eq!(review_fact_refs(&connection,&lease.root_run_id,&context.target_dir).unwrap().len(),1);
    assert!(connection.execute("UPDATE agent_external_surface_captures SET response_json='{}'",[]).is_err());
    connection.execute("DELETE FROM agent_external_surface_captures",[]).unwrap();
    assert!(review_fact_refs(&connection,&lease.root_run_id,&context.target_dir).unwrap().is_empty(),
        "a graph row without its bound capture receipt is not a review fact");
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    model_stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn public_surface_prepare_hands_real_observation_to_web_executor_without_replay() {
    let _real = RealSpecialistTransport::enter();
    let mut h = fresh_multi_production_harness("public-surface-handoff");
    h.context.external_surface = true;
    let (port, models, stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let response = if let Some(root) = fresh_multi_root_wire_response(&wire) { root }
            else if wire.contains("SPA/API Mapper") {
                proposal_model_response(r#"{"summary":"frozen frontend map only","priorityContracts":[],"risks":[]}"#)
            } else { proposal_model_response(public_surface_model_text()) };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = models;
    freeze_fresh_multi_production_harness(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    assert_eq!(h.context.evidence["multiAgentExternalSurface"]["state"],"completed");
    let connection = db::open(&h.db_path).unwrap();
    let handoff:String = connection.query_row(
        "SELECT payload_json FROM agent_messages WHERE to_run_id=?1 AND kind='execution_assignment'",
        [&session.executor.run_id],|r|r.get(0),
    ).unwrap();
    assert!(handoff.contains("publicSurface"));
    assert!(handoff.contains("anonymous_entry_only"));
    assert!(agent_evidence_digest(&h.context.evidence).to_string().contains("publicSurface"));
    let calls = h.model_seen.lock().unwrap().len();
    assert!(calls >= 4, "Root, Mapper and ExternalSurface need actual independent SDK calls");
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_specialist_calls WHERE role='external_surface' AND state='received'", [], |r|r.get::<_,i64>(0)).unwrap(), 1);
    let original = single_finally_physical(&connection);
    assert!(multi_agent_prepare(&mut h.context).is_err());
    assert_eq!(single_finally_physical(&connection), original);
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn public_surface_http_outcomes_are_truthful_and_never_follow_redirects() {
    for (status,content_type,body,protected) in [
        (302,"text/html\r\nLocation: /must-not-follow","redirect",false),
        (401,"text/plain","authentication required",false),
        (403,"text/plain","permission denied",false),
        (429,"text/plain","rate limited",true),
    ] {
        let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(move |_| (status,content_type,body.into())));
        let (_root,context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/entry"));
        let result = multi_agent_external_surface(&context,&lease);
        if protected {
            let error = result.unwrap_err();
            assert!(error.contains("protected_target"));
            let outcome = multi_agent_bootstrap_outcome(&error);
            assert!(matches!(outcome,AgentTargetOutcome::Limited(_)));
            assert_eq!(outcome.terminal_code(),AGENT_STOP_RATE_LIMIT);
            assert!(outcome.stop().unwrap().requires_fuse());
        }
        else { assert_eq!(result.unwrap()["observedCapture"]["capture"]["status"],status); }
        let connection = db::open(&context.db_path).unwrap();
        let state:String = connection.query_row("SELECT state FROM agent_external_surface_captures",[],|r|r.get(0)).unwrap();
        assert_eq!(state,"received");
        assert_eq!(seen.lock().unwrap().len(),1);
        if protected {
            let rounds:i64 = connection.query_row("SELECT COUNT(*) FROM agent_events WHERE event_type='model_round_completed'",[],|r|r.get(0)).unwrap();
            assert_eq!(rounds,0);
        }
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn public_surface_claim_failures_never_dispatch_network() {
    for trigger in [
        "CREATE TRIGGER bad_claim BEFORE INSERT ON agent_external_surface_captures BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER bad_claim BEFORE INSERT ON agent_external_surface_captures BEGIN SELECT RAISE(ABORT,'injected'); END;",
        "CREATE TRIGGER bad_claim AFTER INSERT ON agent_external_surface_captures BEGIN UPDATE agent_external_surface_captures SET artifact_id='forged' WHERE assignment_id=NEW.assignment_id; END;",
    ] {
        let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","unexpected".into())));
        let (_root,mut context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
        let child = public_surface_child(&mut context,&lease);
        let connection = db::open(&context.db_path).unwrap();
        connection.execute_batch(trigger).unwrap();
        assert!(capture_public_surface(&context,&lease,&child).is_err());
        assert_eq!(seen.lock().unwrap().len(),0);
        let count:i64 = connection.query_row("SELECT COUNT(*) FROM agent_external_surface_captures",[],|r|r.get(0)).unwrap();
        assert_eq!(count,0);
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn public_surface_revoked_expired_changed_or_cancelled_children_do_not_send() {
    for mutation in ["revoked","expired","fence","plan","cancel","target"] {
        let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","unexpected".into())));
        let (_root,mut context,mut lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
        let child = public_surface_child(&mut context,&lease);
        let connection = db::open(&context.db_path).unwrap();
        match mutation {
            "revoked" => { connection.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime')",[]).unwrap(); }
            "expired" => { connection.execute("UPDATE agent_assignments SET lease_expires_at='2000-01-01 00:00:00'",[]).unwrap(); }
            "fence" => lease.fencing_token.push_str("stale"),
            "plan" => context.execution_plan.hard_model_requests += 1,
            "cancel" => { connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[&context.scan_id]).unwrap(); }
            "target" => context.target_url.push_str("changed"),
            _ => unreachable!(),
        }
        assert!(capture_public_surface(&context,&lease,&child).is_err(),"{mutation}");
        assert_eq!(seen.lock().unwrap().len(),0,"{mutation}");
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn public_surface_receipt_failure_keeps_claim_and_never_retries_target() {
    for trigger in [
        "CREATE TRIGGER bad_receipt BEFORE UPDATE OF state ON agent_external_surface_captures BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER bad_fact BEFORE INSERT ON agent_evidence_nodes BEGIN SELECT RAISE(ABORT,'injected'); END;",
    ] {
        let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","entry".into())));
        let (_root,mut context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
        let child = public_surface_child(&mut context,&lease);
        let connection = db::open(&context.db_path).unwrap();
        connection.execute_batch(trigger).unwrap();
        for _ in 0..2 { assert!(capture_public_surface(&context,&lease,&child).is_err()); }
        assert_eq!(seen.lock().unwrap().len(),1);
        let state:String = connection.query_row("SELECT state FROM agent_external_surface_captures",[],|r|r.get(0)).unwrap();
        assert_eq!(state,"claimed");
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn public_surface_invalid_model_output_cannot_complete_or_confirm_findings() {
    for text in ["not json",r#"{"summary":"confirmed","observations":[],"coverageGaps":["gap"],"confirmedFindings":true}"#,
        r#"{"summary":"entry","observations":[],"coverageGaps":["  "],"confirmedFindings":false}"#] {
        let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","entry".into())));
        let (_root,mut context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
        let (model_port,model_seen,model_stop) = spawn_endpoint(std::sync::Arc::new(move |_| (200,"application/json",proposal_model_response(text))));
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        let _real = RealSpecialistTransport::enter();
        assert!(multi_agent_external_surface(&context,&lease).unwrap_err().contains("assessment_invalid"));
        let connection = db::open(&context.db_path).unwrap();
        let completed:i64 = connection.query_row("SELECT COUNT(*) FROM agent_assignments WHERE state='completed'",[],|r|r.get(0)).unwrap();
        assert_eq!(completed,0);
        assert_eq!(seen.lock().unwrap().len(),1);
        assert_eq!(model_seen.lock().unwrap().len(),1);
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
        model_stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn public_surface_preserves_reviewer_floor_15000_when_tokens_tight() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","unexpected".into())));
    let (_root,context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    let connection = db::open(&context.db_path).unwrap();
    // 16k free < 2k surface grant + 15k Reviewer floor.
    connection.execute("UPDATE agent_budget_ledger SET total_tokens=16000",[]).unwrap();
    let result = multi_agent_external_surface(&context,&lease).unwrap();
    assert_eq!(result["state"],"not_scheduled");
    assert_eq!(result["reason"],"insufficient_budget_for_surface_executor_and_review");
    assert_eq!(seen.lock().unwrap().len(),0);
    let count:i64 = connection.query_row("SELECT COUNT(*) FROM agent_assignments",[],|r|r.get(0)).unwrap();
    assert_eq!(count,0);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn public_surface_insufficient_budget_does_not_claim_or_call_target() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","unexpected".into())));
    let (_root,context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    let connection = db::open(&context.db_path).unwrap();
    connection.execute("UPDATE agent_budget_ledger SET total_requests=2",[]).unwrap();
    let result = multi_agent_external_surface(&context,&lease).unwrap();
    assert_eq!(result["state"],"not_scheduled");
    assert_eq!(seen.lock().unwrap().len(),0);
    let count:i64 = connection.query_row("SELECT COUNT(*) FROM agent_assignments",[],|r|r.get(0)).unwrap();
    assert_eq!(count,0);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn public_surface_artifact_change_during_analysis_blocks_result_delivery() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","entry".into())));
    let (_root,mut context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    let directory = context.target_dir.clone();
    let (model_port,model_seen,model_stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        std::fs::write(directory.join("agent-http/0001.body"),b"tampered").unwrap();
        (200,"application/json",proposal_model_response(public_surface_model_text()))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
    let _real = RealSpecialistTransport::enter();
    assert!(multi_agent_external_surface(&context,&lease).unwrap_err().contains("artifact_integrity"));
    let connection = db::open(&context.db_path).unwrap();
    assert_specialist_usage_pending(&connection,"external_surface",2000);
    let delivered:i64 = connection.query_row("SELECT COUNT(*) FROM agent_messages WHERE kind='evidence_summary'",[],|r|r.get(0)).unwrap();
    assert_eq!(delivered,0);
    assert_eq!(seen.lock().unwrap().len(),1);
    assert_eq!(model_seen.lock().unwrap().len(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    model_stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn public_surface_pause_during_response_keeps_claim_without_model_or_replay() {
    let pause = std::sync::Arc::new(Mutex::new(None::<(std::path::PathBuf,String)>));
    let handler_pause = pause.clone();
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let (path,scan) = handler_pause.lock().unwrap().clone().unwrap();
        db::open(&path).unwrap().execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[scan]).unwrap();
        (200,"text/plain","entry".into())
    }));
    let (_root,context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    *pause.lock().unwrap() = Some((context.db_path.clone(),context.scan_id.clone()));
    assert!(multi_agent_external_surface(&context,&lease).is_err());
    let connection = db::open(&context.db_path).unwrap();
    let state:String = connection.query_row("SELECT state FROM agent_external_surface_captures",[],|r|r.get(0)).unwrap();
    assert_eq!(state,"claimed");
    let count:i64 = connection.query_row("SELECT COUNT(*) FROM agent_specialist_calls",[],|r|r.get(0)).unwrap();
    assert_eq!(count,0);
    assert_eq!(seen.lock().unwrap().len(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn public_surface_cannot_use_generic_tools_or_reset_corrupt_request_accounting() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","unexpected".into())));
    let (_root,mut context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    let child = public_surface_child(&mut context,&lease);
    for tool in ["replay_http","compare_identities","targeted_discovery","browser_action","finish_target"] {
        assert!(agent_authorize_tool(&context,tool).is_err(),"{tool}");
    }
    write_agent_checkpoint(&context.db_path,&context.scan_id,&context.target_url,NATIVE_AGENT_STATE_STAGE,
        &serde_json::json!({"schemaVersion":999,"targetRequests":"broken"})).unwrap();
    assert!(capture_public_surface(&context,&lease,&child).unwrap_err().contains("checkpoint_invalid"));
    assert_eq!(seen.lock().unwrap().len(),0);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn public_surface_result_delivery_failure_retains_model_usage_and_does_not_repeat_http() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","entry".into())));
    let (_root,mut context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    let connection = db::open(&context.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER bad_result BEFORE UPDATE OF acknowledged_at ON agent_messages WHEN NEW.kind='evidence_summary' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let (model_port,model_seen,model_stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"application/json",proposal_model_response(public_surface_model_text()))));
    context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
    let _real = RealSpecialistTransport::enter();
    assert!(multi_agent_external_surface(&context,&lease).is_err());
    assert_specialist_usage_pending(&connection,"external_surface",2000);
    assert_eq!(seen.lock().unwrap().len(),1);
    assert_eq!(model_seen.lock().unwrap().len(),1);
    assert!(review_fact_refs(&connection,&lease.root_run_id,&context.target_dir).unwrap().is_empty(),
        "an uncompleted expert cannot contribute accepted reviewer facts");
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    model_stop.store(true,std::sync::atomic::Ordering::SeqCst);
}
