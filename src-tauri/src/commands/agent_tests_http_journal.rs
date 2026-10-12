fn http_journal_fixture(target: &str, baseline: i64) -> (PathBuf, AgentRunContext, AgentToolRuntime, AgentHttpRequest) {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::scheduler};
    let (root,mut context,lease) = public_surface_fixture(target);
    let connection = db::open(&context.db_path).unwrap();
    let child = scheduler::schedule_child(&connection,&lease,AgentRole::WebExecutor,AgentLane::TargetTouching,
        "http-journal",&serde_json::json!({}),1,&["replay_http".into()],100,1).unwrap();
    scheduler::mark_child_running(&connection,&lease,&child).unwrap();
    context.run.as_mut().unwrap().run_id=child.run_id;
    accounting_native(&context,baseline);
    let mut runtime = AgentToolRuntime {target_requests:baseline as usize,..Default::default()};
    let invocation = runtime.begin_invocation(1);
    context.run.as_ref().unwrap().begin_tool("replay_http",&serde_json::json!({}),&invocation).unwrap();
    let request = AgentHttpRequest {
        identity:AgentIdentity::anonymous(),method:"GET".into(),url:target.into(),extra_headers:vec![],
        body:None,content_type:None,contract_key:String::new(),family:"authorization".into(),
        source:ScopeSource::IdentityComparison,tool:"replay_http".into(),timeout_seconds:5,
    };
    (root,context,runtime,request)
}

#[test]
fn http_journal_real_fanout_survives_stale_checkpoint_without_double_charge() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","hello".into())));
    let (_root,context,mut runtime,request) = http_journal_fixture(&format!("http://127.0.0.1:{port}/"),3);
    for _ in 0..2 { agent_http_exchange(&context,&mut runtime,&request).unwrap(); }
    let connection = db::open(&context.db_path).unwrap();
    let usage = agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!((usage.executor_recorded,usage.executor_unresolved,usage.budget_committed),(5,0,5));
    assert_eq!(NativeAgentState::read(&context.db_path,&context.scan_id,&context.target_url).unwrap().target_requests,3);
    accounting_native(&context,5);
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().budget_committed,5);
    let rows:i64=connection.query_row("SELECT COUNT(*) FROM agent_http_request_claims WHERE request_index IN (1,2) AND response_status=200",[],|r|r.get(0)).unwrap();
    assert_eq!(rows,2);
    let changes=connection.total_changes();
    for _ in 0..3 { assert_eq!(agent_request_accounting_view(&connection,&context.scan_id,1,&context.target_url)["executorRecordedRequests"],5); }
    assert_eq!(changes,connection.total_changes());
    assert_eq!(seen.lock().unwrap().len(),2);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn http_journal_claim_without_send_stays_charged_and_never_replays() {
    let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","hello".into())));
    let (_root,context,mut runtime,request) = http_journal_fixture(&format!("http://127.0.0.1:{port}/"),0);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    let connection=db::open(&context.db_path).unwrap();
    let usage=agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!((usage.executor_recorded,usage.executor_unresolved,usage.budget_committed),(1,1,1));
    assert_eq!(claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap_err(),"http_journal_checkpoint_requires_reconciliation");
    runtime.target_requests=1;
    assert_eq!(claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap_err(),"http_journal_unknown_request_not_replayable");
    assert!(seen.lock().unwrap().is_empty());
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn http_journal_claim_faults_rollback_and_never_send() {
    for trigger in [
        "CREATE TRIGGER fail_claim BEFORE INSERT ON agent_http_request_claims BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fail_claim BEFORE INSERT ON agent_http_request_claims BEGIN SELECT RAISE(ABORT,'injected'); END;",
        "CREATE TRIGGER fail_claim AFTER INSERT ON agent_http_request_claims BEGIN DELETE FROM agent_http_request_claims WHERE ordinal=NEW.ordinal; END;",
    ] {
        let (port,seen,stop) = spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","hello".into())));
        let (_root,context,mut runtime,request)=http_journal_fixture(&format!("http://127.0.0.1:{port}/"),0);
        let connection=db::open(&context.db_path).unwrap();
        connection.execute_batch(trigger).unwrap();
        let error=agent_http_exchange(&context,&mut runtime,&request).unwrap_err();
        assert_eq!(error["code"],"evidence_write_failed");
        assert_eq!(runtime.target_requests,0);
        for table in ["agent_http_request_claims","agent_http_budget_origins"] {
            let n:i64=connection.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).unwrap();
            assert_eq!(n,0);
        }
        assert!(seen.lock().unwrap().is_empty());
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn http_journal_receipt_fault_keeps_unknown_charge_and_does_not_resend() {
    for trigger in [
        "CREATE TRIGGER fail_receipt BEFORE UPDATE ON agent_http_request_claims BEGIN SELECT RAISE(IGNORE); END;",
        "CREATE TRIGGER fail_receipt BEFORE UPDATE ON agent_http_request_claims BEGIN SELECT RAISE(ABORT,'injected'); END;",
    ] {
        let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_| (200,"text/plain","hello".into())));
        let (_root,context,mut runtime,request)=http_journal_fixture(&format!("http://127.0.0.1:{port}/"),0);
        let connection=db::open(&context.db_path).unwrap();
        connection.execute_batch(trigger).unwrap();
        let result=agent_http_exchange(&context,&mut runtime,&request).unwrap_err();
        assert_eq!(result["code"],"evidence_write_failed");
        assert_eq!(runtime.target_requests,1);
        assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().executor_unresolved,1);
        assert!(agent_http_exchange(&context,&mut runtime,&request).is_err());
        assert_eq!(seen.lock().unwrap().len(),1);
        stop.store(true,std::sync::atomic::Ordering::SeqCst);
    }
}

#[test]
fn http_journal_concurrent_claim_has_one_winner_and_no_network() {
    let (_root,context,_runtime,_request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles=vec![];
    for _ in 0..2 {
        let context=context.clone();
        let barrier=barrier.clone();
        handles.push(std::thread::spawn(move || {
            let mut runtime=AgentToolRuntime::default();runtime.set_invocation("inv-1-001");
            let request=AgentHttpRequest {identity:AgentIdentity::anonymous(),method:"GET".into(),url:context.target_url.clone(),
                extra_headers:vec![],body:None,content_type:None,contract_key:String::new(),family:"authorization".into(),
                source:ScopeSource::IdentityComparison,tool:"replay_http".into(),timeout_seconds:5};
            barrier.wait();claim_agent_http_request(&context,&runtime,&request,&request.url).is_ok()
        }));
    }
    assert_eq!(handles.into_iter().map(|h|usize::from(h.join().unwrap())).sum::<usize>(),1);
    let connection=db::open(&context.db_path).unwrap();
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().executor_unresolved,1);
}

#[test]
fn http_journal_scope_invocation_budget_and_revocation_are_checked_before_claim() {
    let (_root,context,mut runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let connection=db::open(&context.db_path).unwrap();
    runtime.set_invocation("inv-1-999");
    assert_eq!(claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap_err(),"http_journal_invocation_not_running");
    runtime.set_invocation("inv-1-001");
    let cap=context.execution_plan.hard_model_requests.max(1).saturating_mul(4).min(400);
    accounting_native(&context,cap);runtime.target_requests=cap as usize;
    assert_eq!(claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap_err(),"request_budget_exhausted");
    accounting_native(&context,0);runtime.target_requests=0;
    connection.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime')",[]).unwrap();
    assert_eq!(claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap_err(),"tool_capability_or_fencing_denied");
    let n:i64=connection.query_row("SELECT COUNT(*) FROM agent_http_request_claims",[],|r|r.get(0)).unwrap();assert_eq!(n,0);
}

#[test]
fn http_journal_late_headers_settle_without_reactivation_and_receipts_are_immutable() {
    let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let claim=claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    let connection=db::open(&context.db_path).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1",[&context.scan_id]).unwrap();
    receive_agent_http_headers(&context,&claim,429).unwrap();
    assert!(receive_agent_http_headers(&context,&claim,200).is_err());
    assert!(connection.execute("UPDATE agent_http_budget_origins SET baseline_requests=0",[]).is_err());
    assert!(connection.execute("UPDATE agent_http_request_claims SET response_status=200",[]).is_err());
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().executor_unresolved,0);
    let status:String=connection.query_row("SELECT status FROM sentinel_scans WHERE id=?1",[&context.scan_id],|r|r.get(0)).unwrap();
    assert_eq!(status,"paused");
}

#[test]
fn http_journal_resume_inherits_once_fresh_excludes_and_unknowns_are_not_zero() {
    let (_root,context,runtime,request)=http_journal_fixture("http://127.0.0.1:9/",3);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    let connection=db::open(&context.db_path).unwrap();
    for (attempt,mode) in [(1,"initial"),(2,"resume"),(3,"resume"),(4,"fresh")] {
        connection.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode) VALUES(?1,?2,?3)",
            params![context.scan_id,attempt,mode]).unwrap();
    }
    let usage=agent_request_accounting(&connection,&context.scan_id,3,&context.target_url).unwrap();
    assert_eq!(usage.attempts,vec![3,2,1]);
    assert_eq!((usage.executor_recorded,usage.executor_unresolved,usage.budget_committed),(4,1,4));
    let fresh=agent_request_accounting(&connection,&context.scan_id,4,&context.target_url).unwrap();
    assert_eq!((fresh.executor_recorded,fresh.executor_unresolved,fresh.budget_committed),(0,0,0));
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,3,"http://localhost/other").unwrap().executor_recorded,0);
    accounting_native(&context,5);
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,3,&context.target_url).unwrap_err(),"http_journal_checkpoint_exceeds_ledger");
    accounting_native(&context,3);
    connection.execute("DROP TRIGGER immutable_http_request_claim",[]).unwrap();
    connection.execute("UPDATE agent_http_request_claims SET attempt_number=999",[]).unwrap();
    assert_eq!(agent_request_accounting_view(&connection,&context.scan_id,3,&context.target_url)["available"],false);
}

#[test]
fn http_journal_fragment_and_empty_body_cannot_disguise_an_unknown_request() {
    let (_root,context,mut runtime,mut request)=http_journal_fixture("http://127.0.0.1:9/",0);
    let claim=claim_agent_http_request(&context,&runtime,&request,&format!("{}#first",request.url)).unwrap().unwrap();
    runtime.target_requests=1;
    request.body=Some(String::new());
    assert_eq!(claim_agent_http_request(&context,&runtime,&request,&format!("{}#second",request.url)).unwrap_err(),"http_journal_unknown_request_not_replayable");
    let mut wrong=context.clone();wrong.attempt_number=2;
    assert!(receive_agent_http_headers(&wrong,&claim,200).is_err());
    request.identity.key="unbound-identity".into();
    assert_eq!(claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap_err(),"http_journal_identity_invalid");
    let connection=db::open(&context.db_path).unwrap();
    assert_eq!(agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap().executor_unresolved,1);
    connection.execute("DELETE FROM sentinel_scans WHERE id=?1",[&context.scan_id]).unwrap();
    for table in ["agent_http_request_claims","agent_http_budget_origins"] {
        let n:i64=connection.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).unwrap();assert_eq!(n,0);
    }
}

#[test]
fn http_journal_stop_codes_distinguish_reconciliation_budget_authority_and_storage() {
    for (reason, code, status) in [
        ("http_journal_checkpoint_requires_reconciliation", terminal_code::REQUEST_RECONCILIATION_REQUIRED, "paused"),
        ("http_journal_unknown_request_not_replayable", terminal_code::REQUEST_RECONCILIATION_REQUIRED, "paused"),
        ("request_budget_exhausted", terminal_code::HARD_REQUEST_BUDGET, "protected_stop"),
        ("tool_capability_or_fencing_denied", terminal_code::EXECUTION_AUTHORIZATION_DENIED, "paused"),
        ("tool_identity_binding_denied", terminal_code::EXECUTION_AUTHORIZATION_DENIED, "paused"),
        ("agent_attempt_not_active", terminal_code::EXECUTION_AUTHORIZATION_DENIED, "paused"),
        ("http_journal_claim_invalid", terminal_code::EVIDENCE_INTEGRITY, "failed"),
        ("http_journal_checkpoint_exceeds_ledger", terminal_code::EVIDENCE_INTEGRITY, "failed"),
        ("http_journal_claim_commit_failed", terminal_code::PERSISTENCE_FAILURE, "persistence_failure"),
        ("cancelled", terminal_code::USER_CANCELLED, "cancelled"),
    ] {
        let outcome=agent_tool_terminal_outcome(&agent_http_claim_error(reason.into())).unwrap();
        assert_eq!(outcome.terminal_code(),code,"{reason}");
        assert_eq!(outcome.terminal_status(),status,"{reason}");
        if code!=terminal_code::PERSISTENCE_FAILURE { assert!(!outcome.detail().contains("本地记录失败")); }
    }
    for code in ["scope_denied","invalid_arguments","unknown_method","identity_not_found","identity_session_unavailable"] {
        assert!(agent_tool_terminal_outcome(&serde_json::json!({"code":code})).is_none(),
            "a rejected candidate is not automatically an executor-wide fatal stop: {code}");
    }
}

#[test]
fn http_journal_execution_stop_survives_target_run_and_checkpoint_projection() {
    use crate::agent_runtime::{checkpoint, contract::AgentRole, store};
    use std::sync::{Arc, Mutex};
    for code in [
        terminal_code::REQUEST_RECONCILIATION_REQUIRED,
        terminal_code::EXECUTION_AUTHORIZATION_DENIED,
    ] {
        // A fabricated stop has no original execution or financial exit. It
        // cannot terminalize even a live fixture Root by finding today's row.
        let (_root, context, _runtime, _request) = http_journal_fixture("http://127.0.0.1:9/", 0);
        let connection = db::open(&context.db_path).unwrap();
        connection.execute("INSERT INTO sentinel_targets(project_id,scan_id,url,status) SELECT project_id,id,?2,'scanning' FROM sentinel_scans WHERE id=?1",
            params![context.scan_id,context.target_url]).unwrap();
        let before = single_finally_physical(&connection);
        let mut tally = AgentPipelineTally::default();
        assert!(!record_agent_target_outcome(
            &context.db_path,
            &context.scan_id,
            &context.route,
            AgentTargetOutcome::Incomplete(AgentStop::new(code, "manual resolution required")),
            &mut tally,
        ));
        assert_eq!(single_finally_physical(&connection), before);
        assert_eq!(tally.counted(), 0);

        // Actual new creator and original Single Root. The site sees the paid
        // request; a malformed response leaves its effect unknown, or the
        // captured identity expires before a real response can become evidence.
        let revoked = code == terminal_code::EXECUTION_AUTHORIZATION_DENIED;
        let binding = Arc::new(Mutex::new(None::<(PathBuf, String)>));
        let site_binding = binding.clone();
        let sessions: &[(&str, &str, &str)] = if revoked {
            &[("session-a", "cookie-alpha", "Bearer alpha")]
        } else {
            &[]
        };
        let mut h = fresh_single_production_harness_with_sessions(
            "original-http-terminal",
            move |request| {
                assert!(request.starts_with("GET /api/orders HTTP/"));
                if revoked {
                    assert!(request.contains("cookie-alpha") && request.contains("Bearer alpha"));
                    let (path, scan) = site_binding.lock().unwrap().clone().unwrap();
                    let db = db::open(&path).unwrap();
                    assert_eq!(db.execute("UPDATE browser_auth_sessions SET expires_at='2000-01-01T00:00:00Z' WHERE id='session-a' AND owner_scan_id=?1", [scan]).unwrap(), 1);
                    (200, "application/json", r#"{"items":[]}"#.into())
                } else {
                    assert!(!request.to_ascii_lowercase().contains("cookie:"));
                    assert!(!request.to_ascii_lowercase().contains("authorization:"));
                    (0, "text/plain", "unusable response".into())
                }
            },
            sessions,
        );
        freeze_fresh_single_production_harness(&mut h);
        *binding.lock().unwrap() = Some((h.db_path.clone(), h.context.scan_id.clone()));
        let first_url = format!("{}/api/orders", h.context.target_url);
        let second_url = format!("{}/must-not-send", h.context.target_url);
        retarget_model(
            &mut h,
            vec![model_round(
                &[
                    (
                        "replay_http",
                        json!({"identity":if revoked {"session-a"} else {"anonymous"},
                "method":"GET", "url":first_url, "family":"authorization"}),
                    ),
                    (
                        "replay_http",
                        json!({"identity":"anonymous", "method":"GET",
                "url":second_url, "family":"authorization"}),
                    ),
                ],
                100,
            )],
        );
        let owned = execute_fresh_single_production_harness(&mut h);
        assert_eq!(
            owned.outcome.terminal_code(),
            code,
            "{}",
            owned.outcome.detail()
        );
        assert_eq!(h.model_seen.lock().unwrap().len(), 1);
        assert_eq!(h.site_seen.lock().unwrap().len(), 1);
        let db = db::open(&h.db_path).unwrap();
        let root = &h.context.run.as_ref().unwrap().run_id;
        let original =
            crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, root)
                .unwrap();
        original.read_single_exit(&db).unwrap();
        for dimension in [
            "model_requests",
            "model_input_tokens",
            "model_output_tokens",
        ] {
            let b =
                crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), dimension)
                    .unwrap();
            assert_eq!((b.reserved, b.indeterminate), (0, 0));
            assert!(b.consumed > 0);
            if dimension == "model_requests" {
                assert_eq!(b.consumed, 1);
            }
        }
        let target = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            root,
            Some(""),
            "target_requests",
        )
        .unwrap();
        assert_eq!(
            (target.reserved, target.consumed, target.indeterminate),
            if revoked { (0, 1, 0) } else { (0, 0, 1) }
        );
        let mut tally = AgentPipelineTally::default();
        assert!(record_owned_agent_target_outcome(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            &owned,
            &mut tally,
        ));
        assert_eq!(tally.counted(), 1);
        let run = store::find_run(
            &db,
            &h.context.scan_id,
            1,
            &h.context.target_url,
            AgentRole::Coordinator,
        )
        .unwrap()
        .unwrap();
        assert_eq!(run.id, *root);
        assert!(run.is_terminal());
        assert_eq!(run.terminal_code, code);
        let snapshot = checkpoint::recover(&db, root).unwrap();
        assert_eq!(snapshot.state.terminal_code, code);
        let terminal = read_agent_checkpoint(
            &h.db_path,
            &h.context.scan_id,
            &h.context.target_url,
            "agent_terminal",
        );
        assert_eq!(terminal["code"], code);
        assert_eq!(terminal["status"], "paused");
        let status: String = db
            .query_row(
                "SELECT status FROM sentinel_targets WHERE scan_id=?1 AND url=?2",
                params![h.context.scan_id, h.context.target_url],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "paused");
        assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
        let before = single_finally_physical(&db);
        assert!(record_owned_agent_target_outcome(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            &owned,
            &mut tally,
        ));
        assert_eq!(single_finally_physical(&db), before);
        assert_eq!(tally.counted(), 1);
        assert_eq!(h.model_seen.lock().unwrap().len(), 1);
        assert_eq!(h.site_seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn http_journal_stale_checkpoint_stop_is_sticky_across_urls_and_invocations() {
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"text/plain","hello".into())));
    let (_root,context,mut runtime,mut request)=http_journal_fixture(&format!("http://127.0.0.1:{port}/"),0);
    claim_agent_http_request(&context,&runtime,&request,&request.url).unwrap().unwrap();
    let error=agent_http_exchange(&context,&mut runtime,&request).unwrap_err();
    assert_eq!(error["code"],terminal_code::REQUEST_RECONCILIATION_REQUIRED);
    // Neither refreshing the volatile counter nor changing the request or
    // invocation clears an unknown effect within this executor.
    runtime.target_requests=1;
    runtime.set_invocation("inv-1-002");
    request.url.push_str("other");
    assert_eq!(agent_http_exchange(&context,&mut runtime,&request).unwrap_err(),error);
    let result=agent_execute_tool_at(&context,&mut runtime,"replay_http",
        &serde_json::json!({"identity":"anonymous","method":"GET","url":request.url,"family":"authorization"}),"inv-1-003");
    assert_eq!(result.model_view["code"],terminal_code::REQUEST_RECONCILIATION_REQUIRED);
    assert_eq!(runtime.target_requests,1);
    assert!(seen.lock().unwrap().is_empty());
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn http_journal_transport_and_partial_body_failures_preserve_charge_without_retry() {
    for partial_body in [false,true] {
        let listener=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port=listener.local_addr().unwrap().port();
        let server=thread::spawn(move || {
            let (mut stream,_)=listener.accept().unwrap();
            let request=read_http_request(&mut stream).unwrap();
            if partial_body {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 80\r\nConnection: close\r\n\r\npartial").unwrap();
            }
            (listener,request)
        });
        let (_root,context,mut runtime,mut request)=http_journal_fixture(&format!("http://127.0.0.1:{port}/?token=journal-secret"),0);
        let error=agent_http_exchange(&context,&mut runtime,&request).unwrap_err();
        let (listener,received)=server.join().unwrap();
        assert!(received.contains("journal-secret"));
        assert_eq!(error["code"],terminal_code::REQUEST_RECONCILIATION_REQUIRED);
        assert_eq!(error["phase"],if partial_body {"reading_body"} else {"awaiting_headers"});
        assert!(!error.to_string().contains("journal-secret"));
        let connection=db::open(&context.db_path).unwrap();
        let usage=agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap();
        assert_eq!((usage.executor_recorded,usage.executor_unresolved,usage.budget_committed),(1,if partial_body {0} else {1},1));
        assert!(runtime.requests.is_empty(),"headers alone must not become a verified request trace");
        request.url=format!("http://127.0.0.1:{port}/different");
        assert_eq!(agent_http_exchange(&context,&mut runtime,&request).unwrap_err(),error);
        assert_eq!(runtime.target_requests,1);
        listener.set_nonblocking(true).unwrap();
        assert_eq!(listener.accept().unwrap_err().kind(),std::io::ErrorKind::WouldBlock);
    }
}

#[test]
fn http_journal_native_loop_stops_same_turn_before_second_tool_or_model_request() {
    for receipt_failure in [false,true] {
        // The executor comes from the real creator, paid Root+Mapper dispatch and
        // the original Root budget frame (current Web4 executor contract).
        let mut f=web_financial_producer_fixture_with_site(move |_| {
            // Invalid response status models a request observed by the server
            // but with no usable response at the client.
            (if receipt_failure {200} else {0},"text/plain","reply".into())
        });
        let executor_run=f.context().run.as_ref().unwrap().run_id.clone();
        let connection=db::open(&f.context().db_path).unwrap();
        if receipt_failure {
            connection.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE ON agent_http_request_claims BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        }
        let target=f.context().target_url.clone();
        f.set_web_reply(move |_| (200,"application/json",model_round(&[
            ("replay_http",serde_json::json!({"identity":"anonymous","method":"GET","url":target,"family":"authorization"})),
            ("replay_http",serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target}/second"),"family":"authorization"})),
        ],10)));
        let outcome=NativeAgentBackend.execute(f.context());
        assert_eq!(outcome.terminal_code(),if receipt_failure {terminal_code::PERSISTENCE_FAILURE} else {terminal_code::REQUEST_RECONCILIATION_REQUIRED},"{}",outcome.detail());
        assert_eq!(f.web_seen.lock().unwrap().len(),1);
        assert_eq!(f.f.h.site_seen.lock().unwrap().len(),1);
        let invocations:i64=connection.query_row("SELECT COUNT(*) FROM tool_invocations WHERE run_id=?1",[&executor_run],|r|r.get(0)).unwrap();
        assert_eq!(invocations,1,"second tool from the same model round must not start");
        let (status,error):(String,String)=connection.query_row("SELECT status,error_class FROM tool_invocations WHERE run_id=?1",[&executor_run],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(status,"failed");
        assert_eq!(error,if receipt_failure {"evidence_write_failed"} else {terminal_code::REQUEST_RECONCILIATION_REQUIRED});
        let usage=agent_request_accounting(&connection,&f.context().scan_id,1,&f.context().target_url).unwrap();
        assert_eq!((usage.executor_recorded,usage.executor_unresolved),(1,1));
        let decision:String=connection.query_row("SELECT policy_decision FROM tool_invocations WHERE run_id=?1",[&executor_run],|r|r.get(0)).unwrap();
        assert_eq!(decision,"allow","a post-dispatch failure must preserve the original authorization");
        f.f.h.context.resume=true;
        let resumed=NativeAgentBackend.execute(f.context());
        assert_eq!(resumed.terminal_code(),terminal_code::REQUEST_RECONCILIATION_REQUIRED,"{}",resumed.detail());
        assert_eq!(f.web_seen.lock().unwrap().len(),1,"resume must stop before spending another model round");
        assert_eq!(f.f.h.site_seen.lock().unwrap().len(),1);
        drop(connection);
    }
}

#[test]
fn http_journal_nested_probe_failure_cannot_hide_inside_successful_outer_result() {
    let (port,seen,stop)=spawn_endpoint(std::sync::Arc::new(|request| {
        if request.contains("oviraptor_no_such_user") { (0,"text/plain","unusable response".into()) }
        else { (200,"application/json",r#"{"exists":true}"#.into()) }
    }));
    let (_root,context,mut runtime,request)=http_journal_fixture(&format!("http://127.0.0.1:{port}/checkUser?username=alice"),0);
    let arguments=serde_json::json!({"identity":"anonymous","method":"GET","url":request.url,"family":"authorization"});
    let result=agent_execute_tool_at(&context,&mut runtime,"replay_http",&arguments,"inv-1-001");
    assert_eq!(result.model_view["code"],terminal_code::REQUEST_RECONCILIATION_REQUIRED);
    assert!(result.model_view.get("partialResult").is_some(),"retain the first response, do not invent an all-success result");
    let artifact = result.model_view["partialResult"]["rawArtifactId"].as_str().unwrap();
    assert!(!artifact.is_empty());
    assert_eq!(result.model_view["rawArtifactId"], artifact, "partial evidence remains linked at the tool boundary");
    assert_eq!(seen.lock().unwrap().len(),2);
    let connection=db::open(&context.db_path).unwrap();
    let row:i64=connection.query_row("SELECT id FROM tool_invocations",[],|r|r.get(0)).unwrap();
    // Durable claims, not a potentially stale in-memory delta, own the
    // authorization history when a partially executed tool is closed.
    context.run.as_ref().unwrap().finish_tool(Some(row),"replay_http",&arguments,&result.model_view,"inv-1-001",0).unwrap();
    let usage=agent_request_accounting(&connection,&context.scan_id,1,&context.target_url).unwrap();
    assert_eq!((usage.executor_recorded,usage.executor_unresolved,usage.budget_committed),(2,1,2));
    let (status,decision):(String,String)=connection.query_row("SELECT status,policy_decision FROM tool_invocations",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!((status.as_str(),decision.as_str()),("failed","allow"));
    let stored_artifact:String=connection.query_row("SELECT response_artifact_id FROM tool_invocations",[],|r|r.get(0)).unwrap();
    assert_eq!(stored_artifact,artifact);
    let next=agent_execute_tool_at(&context,&mut runtime,"replay_http",&arguments,"inv-1-002");
    assert_eq!(next.model_view["code"],terminal_code::REQUEST_RECONCILIATION_REQUIRED);
    assert_eq!(seen.lock().unwrap().len(),2);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}
