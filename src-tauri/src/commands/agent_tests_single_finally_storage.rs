// Actual backend returns and original financial APIs; no provider/target I/O.
#[test]
fn single_finally_exit_all_unique_replace_and_saved_replay_preserve_physical_rows() {
    let harness = single_finally_fixture("single-exit-unique");
    assert!(matches!(
        NativeAgentBackend.execute(&harness.context),
        AgentTargetOutcome::Failed(_)
    ));
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let (receipt, control): (String, String) = db
        .query_row(
            "SELECT receipt_id,control_id FROM agent_single_exit_receipts WHERE root_run_id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    db.execute_batch("PRAGMA recursive_triggers=OFF").unwrap();
    let before = single_finally_physical(&db);
    for collision in ["primary", "root", "control"] {
        let id = if collision == "primary" {
            receipt.clone()
        } else {
            Uuid::new_v4().to_string()
        };
        let scope = if collision == "root" {
            root.clone()
        } else {
            "another-single-root".into()
        };
        let owner = if collision == "control" {
            control.clone()
        } else {
            Uuid::new_v4().to_string()
        };
        assert!(db.execute("INSERT OR REPLACE INTO agent_single_exit_receipts
            (receipt_id,root_run_id,control_id,fact_json,created_at)
            SELECT ?2,?3,?4,fact_json,created_at FROM agent_single_exit_receipts WHERE root_run_id=?1",
            params![root,id,scope,owner]).is_err(), "{collision}");
        assert_eq!(single_finally_physical(&db), before, "{collision}");
    }
    for sql in [
        "UPDATE agent_single_exit_receipts SET rowid=rowid+100",
        "DELETE FROM agent_single_exit_receipts",
    ] {
        assert!(db.execute_batch(sql).is_err());
        assert_eq!(single_finally_physical(&db), before);
    }
    drop(db);
    assert!(matches!(
        NativeAgentBackend.execute(&harness.context),
        AgentTargetOutcome::Failed(_)
    ));
    let db = db::open(&harness.db_path).unwrap();
    assert_eq!(
        single_finally_physical(&db),
        before,
        "saved financial exit cannot charge a second interval"
    );
    assert!(
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&db, root)
            .unwrap()
            .require_live(&db)
            .is_err(),
        "exit proof is not continuation authorization"
    );
    drop(db);
    single_target_cleanup(harness);
}

#[test]
fn single_finally_exit_receipt_faults_roll_back_wall_fees_and_all_original_rows() {
    for fault in ["IGNORE", "ABORT", "FAIL", "business", "other-root"] {
        let harness = single_finally_fixture("single-exit-receipt-fault");
        let db = db::open(&harness.db_path).unwrap();
        let sibling = crate::agent_runtime::store::AgentRunRow::new(
            Uuid::new_v4().to_string(),
            harness.context.scan_id.clone(),
            1,
            "https://single-sibling.invalid",
            AgentBackendKind::Native,
            crate::agent_runtime::contract::AgentRole::Coordinator,
            "plan",
            "evidence",
        );
        crate::agent_runtime::store::create_run(&db, &sibling).unwrap();
        let effect = match fault {
            "business" => "INSERT INTO projects(name) VALUES('unapproved-single-exit')".to_string(),
            "other-root" => format!(
                "UPDATE agent_runs SET terminal_reason='unapproved-single-exit' WHERE id='{}'",
                sibling.id
            ),
            "IGNORE" => "SELECT RAISE(IGNORE)".into(),
            "ABORT" => "SELECT RAISE(ABORT,'single-exit-fault')".into(),
            _ => "SELECT RAISE(FAIL,'single-exit-fault')".into(),
        };
        db.execute_batch(&format!(
            "CREATE TRIGGER single_exit_fault BEFORE INSERT
            ON agent_single_exit_receipts BEGIN {effect}; END;"
        ))
        .unwrap();
        let before = single_finally_physical(&db);
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert_eq!(
            outcome.terminal_code(),
            AGENT_STOP_PERSISTENCE,
            "{fault}: {outcome:?}"
        );
        assert!(
            outcome.detail().contains("缺少模型名"),
            "original reason survives: {outcome:?}"
        );
        assert_eq!(single_finally_physical(&db), before, "{fault}");
        assert!(harness.model_seen.lock().unwrap().is_empty());
        assert!(harness.site_seen.lock().unwrap().is_empty());
        drop(db);
        single_target_cleanup(harness);
    }
}

#[test]
fn single_finally_exit_overhard_retains_full_elapsed_without_clipping_original_failure() {
    let mut harness = single_target_harness("single-exit-overhard", 1);
    harness.context.environment.llm.clear();
    let db = db::open(&harness.db_path).unwrap();
    let root = &harness.context.run.as_ref().unwrap().run_id;
    let sources = single_finally_original_sources(&db, root);
    std::thread::sleep(Duration::from_millis(1100));
    let outcome = NativeAgentBackend.execute(&harness.context);
    assert!(
        matches!(outcome, AgentTargetOutcome::Failed(_)),
        "{outcome:?}"
    );
    assert!(outcome.detail().contains("缺少模型名"));
    let text: String = db
        .query_row(
            "SELECT fact_json FROM agent_single_exit_receipts WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    let fact: JsonValue = serde_json::from_str(&text).unwrap();
    assert_eq!(fact["hardMs"], 1000);
    assert_eq!(fact["exhausted"], true);
    assert!(fact["elapsedMs"].as_i64().unwrap() >= 1050);
    assert_eq!(fact["journaledMs"], 0);
    assert_eq!(fact["unsettledMs"], fact["elapsedMs"]);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "wall_time_ms")
            .unwrap(),
        Default::default()
    );
    assert_eq!(single_finally_original_sources(&db, root), sources);
    let before = single_finally_physical(&db);
    std::thread::sleep(Duration::from_millis(35));
    NativeAgentBackend.execute(&harness.context);
    assert_eq!(single_finally_physical(&db), before);
    assert!(harness.model_seen.lock().unwrap().is_empty());
    assert!(harness.site_seen.lock().unwrap().is_empty());
    drop(db);
    single_target_cleanup(harness);
}

#[test]
fn single_finally_exit_late_original_invoice_preserves_cutoff_and_denies_next_work() {
    use crate::agent_runtime::{
        multi_agent::budget::{self, root::model::RootModelCall},
    };
    let (fixture,mut context)=single_exit_private_fixture();
    let (port,seen,_stop)=spawn_endpoint(std::sync::Arc::new(|_|(200,"application/json",
        json!({"choices":[{"message":{"content":"actual late invoice"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}).to_string())));
    context.environment.api_base=format!("http://127.0.0.1:{port}/v1");
    let mut profile=agent_model_profile(&context.environment,None).unwrap();profile.max_output_tokens=Some(256);
    let client=AgentModelClient::new(profile,&[]);
    let mut request=agent_model_request(vec![json!({"role":"user","content":"actual original late Single billing"})],&[]);
    let request_hash=crate::agent_runtime::store::stable_hash(&json!({"messages":request.messages,"tools":[],
        "model":client.profile().model,"endpoint":client.profile().endpoint,"maxOutputTokens":256}).to_string());
    let estimate=i64::try_from(serde_json::to_vec(&request.messages).unwrap().len()).unwrap()+256;
    let db = db::open(&fixture.path).unwrap();
    let root = &context.run.as_ref().unwrap().run_id;
    let (call,guard) = {
        let tx = db.unchecked_transaction().unwrap();
        let call = RootModelCall::claim_single_transport(&tx, root, 1, &request_hash, estimate).unwrap();
        tx.commit().unwrap();
        call
    };
    request.request_timeout=Some(call.remaining);
    let response=client.complete_once_observed(&request,&CancelToken::new())
        .unwrap_or_else(|_|panic!("actual late original Single SDK must return"));
    assert_eq!((response.usage.input_tokens,response.usage.output_tokens,response.usage.total_tokens),(2,1,3));
    assert_eq!(seen.lock().unwrap().len(),1);drop(guard);
    // SDK has returned; hold its actual known response for the late callback.
    context.environment.llm.clear();
    assert!(matches!(NativeAgentBackend.execute(&context),AgentTargetOutcome::Failed(_)));
    assert_eq!(db.query_row("SELECT count(*) FROM agent_single_exit_receipts WHERE root_run_id=?1",[root],|r|r.get::<_,i64>(0)).unwrap(),1);
    let fixed = |db: &rusqlite::Connection| {
        single_finally_physical(db)
            .into_iter()
            .filter(|(name, _)| {
                !matches!(
                    name.as_str(),
                    "agent_budget_entries" | "agent_root_model_journal" | "agent_root_model_cache_provenance" | "agent_root_model_cache_capture"
                )
            })
            .collect::<Vec<_>>()
    };
    let original = fixed(&db);
    let wall = budget::balance(&db, root, None, "wall_time_ms").unwrap();
    // Settle only the captured actual invoice after the original exit cutoff.
    let tx = db.unchecked_transaction().unwrap();
    assert!(!call.terminal(&tx, "received", Some(&response), "").unwrap());
    tx.commit().unwrap();
    assert_eq!(
        budget::balance(&db, root, None, "model_input_tokens")
            .unwrap()
            .consumed,
        2
    );
    assert_eq!(
        budget::balance(&db, root, None, "model_output_tokens")
            .unwrap()
            .consumed,
        1
    );
    assert_eq!(
        budget::balance(&db, root, None, "wall_time_ms").unwrap(),
        wall
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_cache_provenance p JOIN agent_root_model_journal j
 ON p.call_id=j.call_id AND p.root_run_id=j.root_run_id AND p.lease_attempt_id=j.lease_attempt_id
 AND p.round=j.round AND p.request_hash=j.request_hash AND p.response_hash=json_extract(j.receipt_json,'$.responseHash')
 WHERE j.root_run_id=?1 AND j.phase='received' AND p.cached_usage_reported=0", [root], |r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_cache_provenance", [], |r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_cache_capture c JOIN agent_root_model_cache_provenance p ON p.call_id=c.call_id
 WHERE p.root_run_id=?1 AND length(c.signature)=32", [root], |r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_cache_capture", [], |r|r.get::<_,i64>(0)).unwrap(),1);
    assert!(!call.verify_original_cache_for_test(&db,&response).unwrap());
    assert_eq!(fixed(&db), original);
    assert!(call.require_next_work(&db).is_err());
    assert_eq!(seen.lock().unwrap().len(),1);
    drop(db);
}
