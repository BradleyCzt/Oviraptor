// Actual first-response issuer faults: no replay or old-artifact signing path.
#[derive(Clone, Copy)]
enum ProtectionIssuanceFault {
    Abort,
    Ignore,
}

fn protection_actual_issuance_fault(fault: ProtectionIssuanceFault) {
    let _real = RealSpecialistTransport::enter();
    let tag = match fault {
        ProtectionIssuanceFault::Abort => "protection-issuance-abort",
        ProtectionIssuanceFault::Ignore => "protection-issuance-ignore",
    };
    let mut h = fresh_multi_production_harness(tag);
    let target = h.context.target_url.clone();
    let database = h.db_path.clone();
    let installed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let install = installed.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let response = if let Some(root) = fresh_multi_root_wire_response(&wire) {
            root
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"frozen frontend map only","priorityContracts":[],"risks":[]}"#,
            )
        } else {
            // Inject only after the actual Web SDK arrives, before it returns
            // the tool call that performs the first real WAF GET. No claim,
            // invoice, receipt, worker closure or signing proof is fabricated.
            assert!(!install.swap(true, std::sync::atomic::Ordering::SeqCst));
            let db = db::open(&database).unwrap();
            let quoted: String = db.query_row(
                "SELECT quote(id) FROM agent_runs WHERE scan_id='agent-scan' AND backend='native'
                 AND role='coordinator' AND parent_run_id IS NULL AND assignment_id=''",
                [],|r|r.get(0),
            ).unwrap();
            let action = match fault {
                ProtectionIssuanceFault::Abort => {
                    "SELECT RAISE(ABORT,'fixture_protection_issuance_abort');"
                }
                ProtectionIssuanceFault::Ignore => "SELECT RAISE(IGNORE);",
            };
            db.execute_batch(&format!(
                "CREATE TRIGGER fixture_protection_issuance_fault BEFORE INSERT ON agent_protection_response_receipts
                 WHEN NEW.root_run_id={quoted} BEGIN {action} END;",
            )).unwrap();
            model_round(
                &[(
                    "replay_http",
                    json!({"identity":"anonymous","method":"GET","url":format!("{target}/api/gated")}),
                )],
                10,
            )
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = 20;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = 2;
    freeze_fresh_multi_production_harness(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let outcome = run_native_agent(&h.context);
    assert!(
        matches!(outcome,AgentTargetOutcome::Limited(ref stop) if stop.code==AGENT_STOP_WAF),
        "{}",
        outcome.detail()
    );
    assert!(installed.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(h.site_seen.lock().unwrap().len(), 1);
    let db = db::open(&h.db_path).unwrap();
    let root = &session.lease.root_run_id;
    assert_eq!(
        db.query_row("SELECT count(*) FROM agent_protection_stops", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_protection_response_receipts",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let cost =
        crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "target_requests")
            .unwrap();
    assert_eq!(
        (cost.consumed, cost.reserved, cost.indeterminate),
        (1, 0, 0)
    );
    let accounting = agent_request_accounting(
        &db,
        &h.context.scan_id,
        h.context.attempt_number,
        &h.context.target_url,
    )
    .unwrap();
    assert_eq!(
        (accounting.executor_recorded, accounting.executor_unresolved),
        (1, 0)
    );
    let invocation:String=db.query_row("SELECT invocation_id FROM agent_http_request_claims WHERE run_id=?1 AND response_status=403 AND received_at<>''",
        [&session.executor.run_id],|r|r.get(0)).unwrap();
    let directory =
        open_agent_artifact_directory(&h.context.target_dir.join(AGENT_HTTP_DIRECTORY)).unwrap();
    let (metadata, _) = read_agent_artifact_file(&directory, "0001.json", 1_048_576).unwrap();
    let record: JsonValue = serde_json::from_slice(&metadata).unwrap();
    let (body, _) =
        read_agent_artifact_file(&directory, "0001.body", AGENT_MAX_RESPONSE_BYTES as u64).unwrap();
    assert_eq!(
        body,
        mock_site("GET /api/gated HTTP/1.1\r\n\r\n".into())
            .2
            .as_bytes()
    );
    assert_eq!(record["response"]["status"], 403);
    assert_eq!(record["response"]["protectionCode"], AGENT_STOP_WAF);
    assert_eq!(
        record["response"]["provenance"],
        "transport_observed_unsealed"
    );
    assert_eq!(
        record["response"]["preSendAuthorizationHash"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert!(record["response"].get("originalArtifactSealed").is_none());
    let calls = h.model_seen.lock().unwrap().len();
    let target_seen = h.site_seen.lock().unwrap().clone();
    let before = super::tests::application_table_snapshot(&db);
    drop(db);
    let reopened = db::open(&h.db_path).unwrap();
    assert!(super::tests::application_table_snapshot(&reopened) == before);
    let request = AgentHttpRequest {
        identity: AgentIdentity::anonymous(),
        method: "GET".into(),
        url: format!("{}/api/gated", h.context.target_url),
        extra_headers: vec![],
        body: None,
        content_type: None,
        contract_key: String::new(),
        family: "authorization".into(),
        source: ScopeSource::HttpReplay,
        tool: "replay_http".into(),
        timeout_seconds: 5,
    };
    for remove_fault in [false, true] {
        if remove_fault {
            // Removing the fixture fault cannot recreate the transport's spent
            // private Fresh response or sign the existing audit artifact.
            reopened
                .execute_batch("DROP TRIGGER fixture_protection_issuance_fault")
                .unwrap();
        }
        let tx = reopened.unchecked_transaction().unwrap();
        let durable = agent_protection_stop_on(&tx, &h.context).unwrap().unwrap();
        assert_eq!(durable.code, AGENT_STOP_WAF);
        assert!(super::tests::application_table_snapshot(&tx) == before);
        tx.commit().unwrap();
        for _ in 0..2 {
            let mut runtime = AgentToolRuntime {
                protected_stop: Some(durable.clone()),
                current_invocation: invocation.clone(),
                target_requests: 1,
                ..Default::default()
            };
            runtime.protected_stop = None;
            assert!(runtime.target_request_stop.is_none());
            let tx = reopened.unchecked_transaction().unwrap();
            assert_eq!(claim_agent_http_request_on(
                &tx, &h.context, &runtime, &request, &request.url, &[],
            ).unwrap_err(), "http_protection_stop_active");
            assert!(super::tests::application_table_snapshot(&tx) == before);
            tx.commit().unwrap();
            let error = agent_http_exchange(&h.context, &mut runtime, &request).unwrap_err();
            assert_eq!(error["code"], "protected");
            assert_eq!(error["stop"]["code"], AGENT_STOP_WAF);
            assert_eq!(
                runtime.protected_stop.as_ref().unwrap().code,
                AGENT_STOP_WAF
            );
            assert!(super::tests::application_table_snapshot(&reopened) == before);
            assert_eq!(h.model_seen.lock().unwrap().len(), calls);
            assert_eq!(*h.site_seen.lock().unwrap(), target_seen);
            assert_eq!(
                reopened
                    .query_row(
                        "SELECT count(*) FROM agent_protection_response_receipts",
                        [],
                        |r| r.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
            let cost = crate::agent_runtime::multi_agent::budget::balance(
                &reopened,
                root,
                None,
                "target_requests",
            )
            .unwrap();
            assert_eq!(
                (cost.consumed, cost.reserved, cost.indeterminate),
                (1, 0, 0)
            );
            assert_eq!(
                read_agent_artifact_file(&directory, "0001.json", 1_048_576)
                    .unwrap()
                    .0,
                metadata
            );
            assert_eq!(
                read_agent_artifact_file(&directory, "0001.body", AGENT_MAX_RESPONSE_BYTES as u64)
                    .unwrap()
                    .0,
                body
            );
        }
    }
    drop(session);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = fs::remove_dir_all(&h.root);
}

#[test]
fn protection_actual_first_waf_issuance_abort_preserves_stop_fee_and_refuses_replay() {
    protection_actual_issuance_fault(ProtectionIssuanceFault::Abort);
}
#[test]
fn protection_actual_first_waf_issuance_ignore_preserves_stop_fee_and_refuses_replay() {
    protection_actual_issuance_fault(ProtectionIssuanceFault::Ignore);
}
