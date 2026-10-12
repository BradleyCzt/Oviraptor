// Actual localhost paid invoices; cold damage is confined to disposable DBs.
#[test]
fn single_source_negative_paid_but_unclosed_interval_is_not_a_source() {
    use crate::agent_runtime::multi_agent::budget::continuation;
    let (f, context, seen) = single_model_inflight_paid();
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(continuation::single::original_source_on(&snapshot, root).err().unwrap(),
        "budget_single_exit_receipt_missing");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn single_source_negative_expired_deadline_keeps_history_but_denies_new_work() {
    use crate::agent_runtime::multi_agent::budget::{clock, continuation, root::RootOwner};
    let f = web_mode_fixture("single", "https://single-source-expiry.example.test/app");
    let mut plan = web_mode_test_plan(&f);
    plan.timeout_seconds = 3; // Original pre-birth cap, never extend or rewrite its clock.
    let mut context = test_context(&f.path, &f.target, vec![AgentIdentity::anonymous()]);
    context.scan_id = f.scan.clone();
    context.execution_plan = plan;
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", model_round(&[], 20))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let client = AgentModelClient::new(agent_model_profile(&context.environment, None).unwrap(), &[]);
    // Finish independent setup before this publisher creates the original
    // Root and clock. The original three-second cap remains unchanged.
    persist_frozen_web_execution_plan(&f.path, &f.scan, 1, &f.target, &context.execution_plan).unwrap();
    context.run = runtime_open_run(&f.path, &f.scan, &context.route);
    let reply = native_model_transport(&context, &client,
        vec![json!({"role":"user","content":"paid source before original deadline"})], &[], 1)
        .unwrap_or_else(|_| panic!("original paid call must finish inside its frozen deadline"));
    context.run.as_ref().unwrap().model_round(1, &reply.usage, &[]).unwrap();
    let db = db::open(&f.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    assert!(!RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap().exhausted);
    let snapshot = db.unchecked_transaction().unwrap();
    let original = continuation::single::original_source_on(&snapshot, root).unwrap().history();
    let remaining = continuation::single_remaining::remaining_on(&snapshot, root).unwrap();
    snapshot.rollback().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    std::thread::sleep(Duration::from_millis(3100));
    assert_eq!(clock::remaining(&db, root).unwrap_err(), "budget_wall_time_exhausted");
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(continuation::single::original_source_on(&snapshot, root).unwrap().history(), original);
    assert_eq!(continuation::single_remaining::remaining_on(&snapshot, root).unwrap(), remaining);
    assert_eq!(continuation::single_remaining::remaining_for_new_work_on(&snapshot, root).unwrap_err(),
        "budget_wall_time_exhausted");
    assert_eq!(original["executionAllowed"], false);
    assert_eq!(remaining["executionAllowed"], false);
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn single_source_negative_multi_owner_cannot_mint_single_capability() {
    use crate::agent_runtime::multi_agent::budget::continuation;
    let (f, seen, _) = financial_issuance_fixture("single-source-cross-policy");
    let db = db::open(&f.context.db_path).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let calls = seen.lock().unwrap().len();
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(continuation::single::original_source_on(&snapshot, &f.actor.root_run_id)
        .err().unwrap(), "budget_root_original_owner_conflict");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), calls);
}

#[test]
fn single_source_negative_cold_exit_damage_is_rejected_without_writes_or_sdk() {
    use crate::agent_runtime::multi_agent::budget::{continuation, root::RootOwner};
    for field in ["rootRunId", "binding", "sources", "wall", "exhausted", "unsettledMs"] {
        let (f, context, seen) = single_model_inflight_paid();
        let db = db::open(&f.path).unwrap();
        let root = &context.run.as_ref().unwrap().run_id;
        RootOwner::load_single(&db, root).unwrap().close_single_finance(&db).unwrap();
        let raw: String = db.query_row("SELECT fact_json FROM agent_single_exit_receipts WHERE root_run_id=?1",
            [root], |row| row.get(0)).unwrap();
        let original: JsonValue = serde_json::from_str(&raw).unwrap();
        let old_value = original.get(field).expect("damage an existing original field");
        let new_value = match field {
            "exhausted" => json!(true),
            "unsettledMs" => json!(1),
            _ => json!("foreign-original-source"),
        };
        let key = serde_json::to_string(field).unwrap();
        let old_fragment = format!("{key}:{}", serde_json::to_string(old_value).unwrap());
        let new_fragment = format!("{key}:{}", serde_json::to_string(&new_value).unwrap());
        assert_eq!(raw.matches(&old_fragment).count(), 1, "{field}");
        let damaged = raw.replacen(&old_fragment, &new_fragment, 1);
        assert_ne!(damaged, raw, "{field}");
        assert_eq!(damaged.matches(&new_fragment).count(), 1, "{field}");
        assert_eq!(damaged.replacen(&new_fragment, &old_fragment, 1), raw,
            "all other original bytes and struct field order must remain intact: {field}");
        let mut expected = original;
        expected[field] = new_value;
        assert_eq!(serde_json::from_str::<JsonValue>(&damaged).unwrap(), expected);
        let trigger: String = db.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='single_exit_no_update'",
            [], |row| row.get(0)).unwrap();
        db.execute_batch("DROP TRIGGER single_exit_no_update").unwrap();
        assert_eq!(db.execute("UPDATE agent_single_exit_receipts SET fact_json=?1 WHERE root_run_id=?2",
            params![damaged, root]).unwrap(), 1);
        db.execute_batch(&trigger).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        assert_eq!(continuation::single::original_source_on(&snapshot, root).err().unwrap(),
            "budget_single_exit_receipt_conflict", "{field}");
        snapshot.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before, "{field}");
        assert_eq!(seen.lock().unwrap().len(), 1, "reading damaged history must never reissue a paid SDK request");
    }
}
