// Actual new signed creator -> Root/Mapper -> Web SDK -> first WAF GET.
#[test]
fn protection_witness_actual_new_multi_first_waf_records_unsealed_pre_send_hash() {
    let _real = RealSpecialistTransport::enter();
    let mut h = fresh_multi_production_harness("protection-pre-send");
    let target = h.context.target_url.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let response = if let Some(root) = fresh_multi_root_wire_response(&wire) {
            root
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"frozen frontend map only","priorityContracts":[],"risks":[]}"#,
            )
        } else {
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
        matches!(outcome,AgentTargetOutcome::Limited(ref stop) if stop.code == AGENT_STOP_WAF),
        "{}",
        outcome.detail()
    );
    assert_eq!(h.site_seen.lock().unwrap().len(), 1);
    let path = h
        .context
        .target_dir
        .join(AGENT_HTTP_DIRECTORY)
        .join("0001.json");
    let metadata: JsonValue = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let record = &metadata["response"];
    let hash = record["preSendAuthorizationHash"]
        .as_str()
        .expect("actual new Multi send must freeze original authorization");
    assert_eq!(hash.len(), 64);
    assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(record["provenance"], "transport_observed_unsealed");
    assert!(record.get("originalArtifactSealed").is_none());
    let db = db::open(&h.db_path).unwrap();
    let target_cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &session.lease.root_run_id,
        None,
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (
            target_cost.consumed,
            target_cost.reserved,
            target_cost.indeterminate
        ),
        (1, 0, 0)
    );
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
        1
    );
    let receipt: String = db
        .query_row(
            "SELECT receipt_id FROM agent_protection_response_receipts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let proof = private_historical_web_mode_in(
        &tx,
        &session.lease.scan_id,
        session.lease.attempt_number,
        &session.lease.root_run_id,
        None,
    )
    .unwrap();
    let issuer = crate::agent_runtime::web_mode::protection_issuer::ProtectionIssuer::from_verified_birth_on(&tx,&session.lease.root_run_id,proof).unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let raw = issuer.verify_issued(&tx, &receipt).unwrap();
    let signed: JsonValue = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        signed["classifierVersion"],
        "native-directory-block-signal.v1"
    );
    assert_eq!(signed["firstWriter"]["artifactId"], "0001.json");
    assert_eq!(signed["firstWriter"]["bodySha256"], record["bodySha256"]);
    assert_eq!(
        signed["preSendAuthorization"]["rootRunId"],
        session.lease.root_run_id
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), before);
    for sql in ["UPDATE agent_protection_response_receipts SET signature=zeroblob(32)",
        "DELETE FROM agent_protection_response_receipts",
        "INSERT OR REPLACE INTO agent_protection_response_receipts SELECT * FROM agent_protection_response_receipts",
        "INSERT OR IGNORE INTO agent_protection_response_receipts SELECT * FROM agent_protection_response_receipts"] {
        assert!(tx.execute_batch(sql).is_err());
        assert_eq!(super::tests::application_table_snapshot(&tx),before);
    }
    tx.commit().unwrap();
    drop(session);
    let tx = db.unchecked_transaction().unwrap();
    let after_close = super::tests::application_table_snapshot(&tx);
    assert_eq!(issuer.verify_issued(&tx, &receipt).unwrap(), raw);
    assert_eq!(super::tests::application_table_snapshot(&tx), after_close);
    tx.commit().unwrap();
    // Fixture-only cold corruption can recompute public SHA, never the private
    // first-response domain signature. No read-time repair is permitted.
    let tx = db.unchecked_transaction().unwrap();
    let mut cold = signed.clone();
    cold["firstWriter"]["bodySha256"] = "a".repeat(64).into();
    let cold_raw = cold.to_string();
    assert_ne!(cold_raw, raw);
    tx.execute_batch("DROP TRIGGER protection_response_receipt_no_update")
        .unwrap();
    tx.execute("UPDATE agent_protection_response_receipts SET fact_json=?1,fact_hash=?2 WHERE receipt_id=?3",
        params![cold_raw,crate::agent_runtime::store::stable_hash(&cold_raw),receipt]).unwrap();
    tx.execute_batch(include_str!(
        "../agent_runtime/web_mode/protection_receipt.sql"
    ))
    .unwrap();
    let damaged = super::tests::application_table_snapshot(&tx);
    assert_eq!(
        issuer.verify_issued(&tx, &receipt).unwrap_err(),
        "protection_receipt_original_signature_invalid"
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    drop(tx);

    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = fs::remove_dir_all(&h.root);
}

#[test]
fn protection_witness_cold_protocol_deletion_refuses_actual_send_without_refund() {
    let _real = RealSpecialistTransport::enter();
    let mut h = fresh_multi_production_harness("protection-cold-deletion");
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(|wire| {
        let response = fresh_multi_root_wire_response(&wire).unwrap_or_else(|| {
            proposal_model_response(
                r#"{"summary":"frozen frontend map only","priorityContracts":[],"risks":[]}"#,
            )
        });
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
    accounting_native(&h.context, 0);
    let mut runtime = AgentToolRuntime::default();
    let invocation = runtime.begin_invocation(1);
    h.context
        .run
        .as_ref()
        .unwrap()
        .begin_tool("replay_http", &json!({}), &invocation)
        .unwrap();
    let request = AgentHttpRequest {
        identity: AgentIdentity::anonymous(),
        method: "GET".into(),
        url: format!("{}/api/gated", h.context.target_url),
        extra_headers: vec![],
        body: None,
        content_type: None,
        contract_key: String::new(),
        family: "authorization".into(),
        source: ScopeSource::IdentityComparison,
        tool: "replay_http".into(),
        timeout_seconds: 5,
    };
    let db = db::open(&h.db_path).unwrap();
    let original: String = db
        .query_row(
            "SELECT definition_json FROM agent_root_mode_definitions WHERE root_run_id=?1",
            [&session.lease.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut changed: JsonValue = serde_json::from_str(&original).unwrap();
    assert!(changed
        .as_object_mut()
        .unwrap()
        .remove("protectionSupervision")
        .is_some());
    let canonical = serde_json::to_string(
        &serde_json::from_value::<crate::agent_runtime::web_mode::root::NewRootModeDeclaration>(
            changed,
        )
        .unwrap(),
    )
    .unwrap();
    let guard: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='root_mode_immutable_update'", [], |r|r.get(0)).unwrap();
    let sdk_before = h.model_seen.lock().unwrap().len();
    db.execute_batch("DROP TRIGGER root_mode_immutable_update").unwrap();
    db.execute(
        "UPDATE agent_root_mode_definitions SET definition_json=?1 WHERE root_run_id=?2",
        params![canonical, session.lease.root_run_id],
    )
    .unwrap();
    db.execute_batch(&guard).unwrap();
    let restored: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='root_mode_immutable_update'", [], |r|r.get(0)).unwrap();
    assert_eq!(restored, guard, "restore exact original guard before cold verification");
    let error = agent_http_exchange(&h.context, &mut runtime, &request).unwrap_err();
    // Both real races stop before network: either the background original-birth
    // check withdraws entry before a claim, or the send witness rejects after
    // the one original claim has become indeterminate. Never accept an arbitrary
    // error or erase the distinction between these financial outcomes.
    let before_claim = error == json!({"code":"evidence_write_failed","error":"worker_supervision_denied"});
    if !before_claim {
        assert_eq!(error["phase"], "before_send", "actual cold protocol refusal: {error}");
        assert_eq!(error["code"], terminal_code::REQUEST_RECONCILIATION_REQUIRED);
    }
    assert_eq!(h.model_seen.lock().unwrap().len(), sdk_before);
    assert!(h.site_seen.lock().unwrap().is_empty());
    let cost = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &session.lease.root_run_id,
        None,
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (cost.consumed, cost.reserved, cost.indeterminate),
        (0, 0, if before_claim { 0 } else { 1 })
    );
    let claimed: i64 = db.query_row("SELECT count(*) FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2", params![h.context.scan_id,h.context.target_url], |r|r.get(0)).unwrap();
    assert_eq!(claimed, if before_claim { 0 } else { 1 });
    let signed: i64 = db.query_row("SELECT count(*) FROM agent_protection_response_receipts WHERE root_run_id=?1", [&session.lease.root_run_id], |r|r.get(0)).unwrap();
    assert_eq!(signed, 0);
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        agent_http_exchange(&h.context, &mut runtime, &request).unwrap_err(),
        error
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert!(h.site_seen.lock().unwrap().is_empty());
    drop(session);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let _ = fs::remove_dir_all(&h.root);
}
