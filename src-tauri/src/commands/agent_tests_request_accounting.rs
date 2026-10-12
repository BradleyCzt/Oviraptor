fn accounting_native(context: &AgentRunContext, count: i64) {
    let mut state = NativeAgentState::fresh(context.attempt_number, AgentBackendKind::Native, "evidence", "plan", vec![]);
    state.target_requests = count;
    state.persist(&context.db_path,&context.scan_id,&context.target_url).unwrap();
}

#[test]
fn request_accounting_received_capture_is_additive_and_reads_do_not_replay() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","entry".into())));
    let (_root,mut context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    accounting_native(&context,3);
    let child = public_surface_child(&mut context,&lease);
    capture_public_surface(&context,&lease,&child).unwrap();
    let connection = db::open(&context.db_path).unwrap();
    let before = connection.total_changes();
    for _ in 0..3 {
        let usage = agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap();
        assert_eq!((usage.executor_recorded,usage.external.received,usage.external.unresolved,usage.recorded,usage.budget_committed),(3,1,0,4,4));
        assert_eq!(crate::agent_runtime::target_requests::child_received(&connection,&child.run_id).unwrap(),1);
        let api = read_agent_target_execution(&context.db_path,&context.scan_id,&context.target_url).unwrap();
        assert_eq!(api["runtime"]["requestAccounting"],usage.as_json());
        assert_eq!(NativeAgentState::read(&context.db_path,&context.scan_id,&context.target_url).unwrap().target_requests,3);
    }
    assert_eq!(connection.total_changes(),before);
    assert_eq!(seen.lock().unwrap().len(),1);
    // Lost or tampered evidence invalidates review, not the spent request.
    std::fs::write(context.target_dir.join("agent-http/0001.body"),b"tampered").unwrap();
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().recorded,4);
    connection.execute("DROP TRIGGER immutable_external_surface_capture",[]).unwrap();
    connection.execute("UPDATE agent_external_surface_captures SET response_hash='invalid'",[]).unwrap();
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap_err(),"request_accounting_receipt_invalid");
    assert_eq!(agent_target_request_ceiling(&context),0);
    // Other targets/scans do not inherit this spend.
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,"http://localhost/other").unwrap().recorded,0);
    assert_eq!(agent_request_accounting(&connection,"different-scan",1,&context.target_url).unwrap().recorded,0);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn request_accounting_claim_is_uncertain_and_consumes_shared_cap_without_sending() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","entry".into())));
    let (_root,mut context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    accounting_native(&context,3);
    let ceiling = agent_target_request_ceiling(&context);
    let child = public_surface_child(&mut context,&lease);
    claim_public_surface_capture(&context,&lease,&child).unwrap();
    let connection = db::open(&context.db_path).unwrap();
    let usage = agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!((usage.recorded,usage.budget_committed,usage.external.unresolved),(3,4,1));
    assert_eq!(agent_target_request_ceiling(&context),ceiling-1);
    assert!(capture_public_surface(&context,&lease,&child).is_err());
    assert!(seen.lock().unwrap().is_empty());
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn request_accounting_resume_inherits_native_once_and_fresh_starts_new_budget() {
    let (_root,mut context,lease) = public_surface_fixture("http://127.0.0.1:9/");
    accounting_native(&context,7);
    let child = public_surface_child(&mut context,&lease);
    claim_public_surface_capture(&context,&lease,&child).unwrap();
    let connection = db::open(&context.db_path).unwrap();
    for (attempt,mode) in [(1,"initial"),(2,"resume"),(3,"resume"),(4,"fresh")] {
        connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode) VALUES(?1,?2,?3)",
            params![context.scan_id,attempt,mode]).unwrap();
    }
    let usage = agent_request_accounting(&connection,&context.scan_id,3,&context.target_url).unwrap();
    assert_eq!(usage.attempts,vec![3,2,1]);
    assert_eq!((usage.recorded,usage.budget_committed),(7,8));
    // A second independently leased attempt has its own capture; the native
    // checkpoint is cumulative, so adding both native snapshots is incorrect.
    use crate::agent_runtime::{contract::{AgentRole,AgentLane,AgentRunStatus,MultiAgentPolicy},store::AgentRunRow,multi_agent::lease as leases};
    context.attempt_number=2;
    context.execution_plan=context.execution_plan.with_attempt(2);
    connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",[&context.scan_id]).unwrap();
    let mut root2=AgentRunRow::new("root-accounting-resume",&context.scan_id,2,&context.target_url,
        AgentBackendKind::Native,AgentRole::Coordinator,context.execution_plan.hash(),"evidence")
        .with_budget(30_000,60_000,10,20);
    root2.root_run_id=root2.id.clone();
    root2.status=AgentRunStatus::Running;
    root2.orchestration_policy=MultiAgentPolicy::Multi;
    root2.lane=Some(AgentLane::ReadOnlyAnalysis);
    crate::agent_runtime::store::create_run(&connection,&root2).unwrap();
    context.run=Some(AgentRunLedger {db_path:context.db_path.clone(),run_id:root2.id.clone()});
    freeze_authorization_test_plan(&context);
    let lease2=leases::acquire_coordinator_lease(&connection,&context.scan_id,2,&context.target_url,&root2.id,600).unwrap();
    connection.execute("INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,lease_epoch,fencing_token) VALUES(?1,60000,20,?2,?3)",
        params![lease2.root_run_id,lease2.lease_epoch,lease2.fencing_token]).unwrap();
    accounting_native(&context,9);
    let child2=public_surface_child(&mut context,&lease2);
    claim_public_surface_capture(&context,&lease2,&child2).unwrap();
    let cumulative=agent_request_accounting(&connection,&context.scan_id,3,&context.target_url).unwrap();
    assert_eq!((cumulative.executor_recorded,cumulative.external.unresolved,cumulative.budget_committed),(9,2,11));
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap_err(),"request_accounting_checkpoint_newer_attempt");
    let fresh = agent_request_accounting(&connection,&context.scan_id,4,&context.target_url).unwrap();
    assert_eq!(fresh.attempts,vec![4]);
    assert_eq!((fresh.recorded,fresh.budget_committed),(0,0));
    connection.execute("DELETE FROM sentinel_scan_attempts WHERE attempt_number<3",[]).unwrap();
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,3,&context.target_url).unwrap_err(),"request_accounting_parent_missing");
}

#[test]
fn request_accounting_invalid_checkpoint_binding_and_overflow_are_not_zero() {
    let (_root,mut context,lease) = public_surface_fixture("http://127.0.0.1:9/");
    accounting_native(&context,i64::MAX);
    let child = public_surface_child(&mut context,&lease);
    // Full native cap prevents even the dedicated broker from claiming.
    assert_eq!(claim_public_surface_capture(&context,&lease,&child).unwrap_err(),"public_surface_request_budget_exhausted");
    accounting_native(&context,0);
    claim_public_surface_capture(&context,&lease,&child).unwrap();
    accounting_native(&context,i64::MAX);
    let connection = db::open(&context.db_path).unwrap();
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap_err(),"request_accounting_overflow");
    connection.execute("UPDATE sentinel_checkpoints SET raw_json='not-json' WHERE stage=?1",[NATIVE_AGENT_STATE_STAGE]).unwrap();
    let view = agent_request_accounting_view(&connection,&context.scan_id,1,&context.target_url);
    assert_eq!(view["available"],false);
    assert!(view["recordedRequests"].is_null());
    assert_eq!(view["automaticReplayAllowed"],false);
    accounting_native(&context,0);
    let original = NativeAgentState::read(&context.db_path,&context.scan_id,&context.target_url).unwrap().as_json();
    for value in [JsonValue::Null,serde_json::json!(-1),serde_json::json!(0.5),serde_json::json!("0")] {
        let mut invalid=original.clone();
        invalid["targetRequests"]=value;
        connection.execute("UPDATE sentinel_checkpoints SET raw_json=?1 WHERE stage=?2",
            params![invalid.to_string(),NATIVE_AGENT_STATE_STAGE]).unwrap();
        assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap_err(),"request_accounting_checkpoint_counter_unknown");
    }
    let mut missing=original.clone();
    missing.as_object_mut().unwrap().remove("targetRequests");
    connection.execute("UPDATE sentinel_checkpoints SET raw_json=?1 WHERE stage=?2",
        params![missing.to_string(),NATIVE_AGENT_STATE_STAGE]).unwrap();
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap_err(),"request_accounting_checkpoint_counter_unknown");
    missing=original;
    missing.as_object_mut().unwrap().remove("attemptNumber");
    connection.execute("UPDATE sentinel_checkpoints SET raw_json=?1 WHERE stage=?2",
        params![missing.to_string(),NATIVE_AGENT_STATE_STAGE]).unwrap();
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap_err(),"request_accounting_checkpoint_attempt_unknown");
    accounting_native(&context,0);
    connection.execute("UPDATE agent_runs SET role='identity_session' WHERE id=?1",[&child.run_id]).unwrap();
    assert!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).is_err());
    assert_eq!(agent_target_request_ceiling(&context),0);
}

#[test]
fn request_accounting_protection_response_counts_without_a_model_round() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (429,"text/plain","limited".into())));
    let (_root,context,lease) = public_surface_fixture(&format!("http://127.0.0.1:{port}/"));
    assert!(multi_agent_external_surface(&context,&lease).is_err());
    let connection = db::open(&context.db_path).unwrap();
    let usage = agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!((usage.external.received,usage.recorded,usage.budget_committed),(1,1,1));
    assert_eq!(seen.lock().unwrap().len(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn request_accounting_terminal_snapshot_preserves_aggregate_and_child_ownership() {
    // A Multi Root is born from the real creator contract and exits through the
    // original `finalize_agent_target` -> `finish_coordinator_run` exit receipt.
    // The runtime terminal consumer for Multi is a pure original reader (no
    // backfill), so the Root aggregate is the terminal request-accounting view
    // over the immutable lineage, while the child keeps its own snapshot.
    let _real = RealSpecialistTransport::enter();
    let mut h = fresh_multi_production_harness("request-accounting-terminal");
    h.context.external_surface = true;
    let (port, models, model_stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let response = if let Some(root) = fresh_multi_root_wire_response(&wire) { root }
            else if wire.contains("SPA/API Mapper") {
                proposal_model_response(r#"{"summary":"frozen frontend map only","priorityContracts":[],"risks":[]}"#)
            } else if specialist_original_is_web_sdk(&wire) {
                proposal_model_response("actual bounded Web exit; no target tool")
            } else { proposal_model_response(public_surface_model_text()) };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = models;
    h.context.execution_plan.max_turns = 1;
    freeze_fresh_multi_production_harness(&mut h);
    let mut session = multi_agent_prepare(&mut h.context).unwrap();
    let context = &h.context;
    let connection = db::open(&context.db_path).unwrap();
    let child:String = connection.query_row("SELECT child_run_id FROM agent_assignments WHERE role='external_surface'",[],|r|r.get(0)).unwrap();
    let snapshot = crate::agent_runtime::store::read_snapshot(&connection,&child).unwrap().unwrap();
    assert_eq!(snapshot.snapshot["targetRequests"],1);
    let outcome = run_native_agent(context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    assert_eq!(h.model_seen.lock().unwrap().iter().filter(|wire|specialist_original_is_web_sdk(wire)).count(),1);
    // The three executor target requests below remain a synthetic accounting
    // fixture. Only the Web model exit and its financial journal are real.
    let mut original_state=NativeAgentState::read(&context.db_path,&context.scan_id,&context.target_url)
        .expect("actual Web exit checkpoint");
    let before_state=original_state.as_json();
    let original_target_requests=original_state.target_requests;
    original_state.target_requests=3;
    original_state.persist(&context.db_path,&context.scan_id,&context.target_url).unwrap();
    let mut reread=NativeAgentState::read(&context.db_path,&context.scan_id,&context.target_url).unwrap();
    assert_eq!(reread.target_requests,3);
    reread.target_requests=original_target_requests;
    assert_eq!(reread.as_json(),before_state,"synthetic target counts cannot replace the actual Web checkpoint");
    multi_agent_finish_execution(context,&mut session,&outcome).unwrap();
    let closed = finalize_agent_target(context,&session.lease,outcome.clone());
    assert_eq!((closed.terminal_code(),closed.detail()),(outcome.terminal_code(),outcome.detail()),"the real Multi exit closes with the original outcome");
    let receipts:i64 = connection.query_row("SELECT count(*) FROM agent_multi_exit_receipts",[],|r|r.get(0)).unwrap();
    assert_eq!(receipts,1);
    let route = FrontendRoute {url:context.target_url.clone(),score:0,mode:"standard".into(),surface:"unknown".into(),reasons:vec![]};
    let before = single_finally_physical(&connection);
    for _ in 0..2 {
        let reduction = record_runtime_terminal_facts_checked(&context.db_path,&context.scan_id,&route,&closed).unwrap().unwrap();
        assert_eq!((reduction.code.as_str(),reduction.reason.as_str()),(outcome.terminal_code(),outcome.detail().as_str()));
        assert_eq!(single_finally_physical(&connection),before,"Multi terminal facts only verify the original exit");
    }
    assert!(crate::agent_runtime::store::read_snapshot(&connection,&session.lease.root_run_id).unwrap().is_none(),"no Root projection is backfilled after the original exit");
    let root = read_agent_target_execution(&context.db_path,&context.scan_id,&context.target_url).unwrap();
    let accounting = &root["runtime"]["requestAccounting"];
    assert_eq!(accounting["available"],true);
    assert_eq!(accounting["recordedRequests"],4);
    assert_eq!(accounting["executorRecordedRequests"],3);
    assert_eq!(accounting["externalSurfaceReceivedRequests"],1);
    assert_eq!(accounting["budgetCommittedRequests"],4);
    let usage = agent_request_accounting(&connection,&context.scan_id,context.attempt_number,&context.target_url).unwrap();
    assert_eq!((usage.executor_recorded,usage.external.received,usage.recorded,usage.budget_committed),(3,1,4,4));
    let snapshot = crate::agent_runtime::store::read_snapshot(&connection,&child).unwrap().unwrap();
    assert_eq!(snapshot.snapshot["targetRequests"],1,"the child keeps only its own request");
    assert_eq!(h.site_seen.lock().unwrap().len(),1);
    model_stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn request_accounting_replay_counts_explicit_deltas_not_tool_completions() {
    use crate::agent_runtime::{checkpoint,contract::AgentEventKind,store};
    let (_root,context,lease) = public_surface_fixture("http://127.0.0.1:9/");
    let connection = db::open(&context.db_path).unwrap();
    let base = checkpoint::RunState::new(&lease.root_run_id,vec![]);
    for delta in [0,3,1] {
        store::append_event(&connection,&lease.root_run_id,AgentEventKind::ToolInvocationCompleted,
            &serde_json::json!({"targetRequestsDelta":delta}),&[]).unwrap();
    }
    let events = store::read_events_after(&connection,&lease.root_run_id,0).unwrap();
    let state = checkpoint::replay(&base,&events);
    assert_eq!(state.target_requests,4);
    assert_eq!(state.as_json()["targetRequests"],4);
    let mut aggregate_base=base.clone();
    aggregate_base.request_accounting=Some(serde_json::json!({"available":true,"recordedRequests":0}));
    let aggregate=checkpoint::replay(&aggregate_base,&events);
    assert_eq!(aggregate.target_requests,4);
    assert_eq!(aggregate.as_json()["requestAccounting"]["reasonCode"],"request_accounting_requires_refresh");
    assert!(aggregate.as_json()["targetRequests"].is_null(),"stale aggregate and fresh per-run count must not contradict each other");
    store::append_event(&connection,&lease.root_run_id,AgentEventKind::ToolInvocationCompleted,
        &serde_json::json!({"tool":"legacy-no-count"}),&[]).unwrap();
    let events = store::read_events_after(&connection,&lease.root_run_id,0).unwrap();
    let state = checkpoint::replay(&base,&events);
    assert_eq!(state.target_requests,4,"missing metadata must not invent one request");
    assert!(state.as_json()["targetRequests"].is_null());
    assert_eq!(state.as_json()["requestAccounting"]["available"],false);
}
