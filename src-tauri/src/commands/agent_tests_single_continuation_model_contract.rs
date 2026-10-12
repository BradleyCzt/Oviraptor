// Current model configuration diagnostics never authenticate an old contract.
#[test]
fn single_continuation_model_contract_caller_custom_cap_reaches_real_single_wire() {
    use crate::agent_runtime::multi_agent::{attempts::audit_rows::Rows, budget};
    let (f, mut context) = single_exit_private_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        let mut response: JsonValue = serde_json::from_str(&model_round(&[], 40)).unwrap();
        response["usage"]["completion_tokens"] = json!(20);
        response["usage"]["total_tokens"] = json!(60);
        (200, "application/json", response.to_string())
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let mut profile = agent_model_profile(&context.environment, None).unwrap();
    // Caller-supplied GatewayProfile coverage, not a claim that the product
    // settings UI or stored model profile can configure an output cap of 37.
    profile.max_output_tokens = Some(37);
    let client = AgentModelClient::new(profile, &[]);
    let reply = native_model_transport(&context, &client,
        vec![json!({"role":"user","content":"actual Single caller custom output bound"})], &[], 1)
        .unwrap_or_else(|_| panic!("actual bounded Single SDK must succeed"));
    assert_eq!(reply.usage.total_tokens, 60);
    context.run.as_ref().unwrap().model_round(1, &reply.usage, &[]).unwrap();
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let wire: JsonValue = serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(wire["max_tokens"], 37);
    drop(requests);
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    for phase in ["dispatch", "received"] {
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase=?2",
            params![root, phase], |row| row.get::<_, i64>(0)).unwrap(), 1, "{phase}");
    }
    for (dimension, consumed) in [("model_input_tokens", 40), ("model_output_tokens", 20),
        ("model_cached_tokens", 0), ("model_requests", 1)] {
        let amount = budget::balance(&db, root, None, dimension).unwrap();
        assert_eq!((amount.consumed, amount.reserved, amount.indeterminate),
            (consumed, 0, 0), "{dimension}");
    }
    let fees = Rows::read(&db, "SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid", []).unwrap();
    budget::root::RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
    assert!(Rows::read(&db, "SELECT rowid,* FROM agent_budget_entries WHERE dimension<>'wall_time_ms' ORDER BY rowid", []).unwrap() == fees);
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(budget::continuation::single::original_source_on(&snapshot, root).unwrap().history()["executionAllowed"], false);
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn single_continuation_model_contract_configuration_changes_hash_but_grants_nothing() {
    let (f, context, seen) = single_model_inflight_paid();
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, root)
        .unwrap().close_single_finance(&db).unwrap();
    assert_eq!(db.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&f.scan]).unwrap(), 1);
    let mut settings = json!({"modelProfiles":[{"id":"single-model-test",
        "llm":context.environment.llm,"apiKey":context.environment.api_key,
        "apiBase":context.environment.api_base,"deployment":"cloud"}],
        "activeModelProfileId":"single-model-test","agentProxyEnabled":false});
    assert!(db.execute("UPDATE config_profiles SET settings_json=?1", [settings.to_string()]).unwrap() > 0);
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(native_single_continuation_model_contract_on(&db, &f.scan, 1, root).unwrap_err(),
        "single_continuation_model_requires_snapshot");
    let snapshot = db.unchecked_transaction().unwrap();
    let original = native_single_continuation_model_contract_on(&snapshot, &f.scan, 1, root).unwrap();
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let wire: JsonValue = serde_json::from_str(requests[0].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(original["modelCandidates"][0]["maxOutputTokens"], wire["max_tokens"]);
    assert_eq!(wire["max_tokens"], 8192);
    drop(requests);
    for (field, value) in [("apiKey", "changed-test-key"),
        ("apiBase", "http://127.0.0.1:9/v1"), ("llm", "changed-test-model")] {
        let saved = settings["modelProfiles"][0][field].clone();
        settings["modelProfiles"][0][field] = json!(value);
        assert!(db.execute("UPDATE config_profiles SET settings_json=?1", [settings.to_string()]).unwrap() > 0);
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        let changed = native_single_continuation_model_contract_on(&snapshot, &f.scan, 1, root).unwrap();
        assert_ne!(changed["modelCandidates"][0]["modelBindingHash"],
            original["modelCandidates"][0]["modelBindingHash"], "{field}");
        for proposal in [&original, &changed] {
            assert_eq!(proposal["executionAllowed"], false);
            assert_eq!(proposal["approvalBlocked"], true);
            assert_eq!(proposal["originalOutputCapAuthenticated"], false);
            assert_eq!(proposal["completeInputUpperBoundAuthenticated"], false);
        }
        snapshot.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(seen.lock().unwrap().len(), 1);
        settings["modelProfiles"][0][field] = saved;
    }
}

#[test]
fn single_continuation_model_contract_non_single_and_foreign_scope_are_refused() {
    let (f, context, seen) = single_model_inflight_paid();
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, root)
        .unwrap().close_single_finance(&db).unwrap();
    assert_eq!(db.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&f.scan]).unwrap(), 1);
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    for (scan, attempt, selected) in [("foreign-scan", 1, root.as_str()),
        (f.scan.as_str(), 2, root.as_str()), (f.scan.as_str(), 1, "foreign-root")] {
        assert_eq!(native_single_continuation_model_contract_on(&snapshot, scan, attempt, selected).unwrap_err(),
            "single_continuation_model_scope_invalid");
    }
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let (multi, multi_seen, _) = financial_issuance_fixture("single-model-contract-multi-denied");
    let db = db::open(&multi.context.db_path).unwrap();
    let calls = multi_seen.lock().unwrap().len();
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(native_single_continuation_model_contract_on(&snapshot,
        &multi.context.scan_id, 1, &multi.actor.root_run_id).unwrap_err(),
        "single_continuation_model_scope_invalid");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(multi_seen.lock().unwrap().len(), calls);
}
