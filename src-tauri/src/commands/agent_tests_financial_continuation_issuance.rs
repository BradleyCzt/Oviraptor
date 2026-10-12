// Original caller → real pause → exact fee confirmation → separate approval →
// actual second provider request in a new Root. No status fixtures replace pause.
fn financial_issuance_fixture(tag: &str) -> (RootTickFixture, FinancePreviewSeen, String) {
    financial_issuance_fixture_with_limits(tag, (200_000, 20))
}

fn financial_issuance_fixture_with_limits(
    tag: &str, limits: (i64, i64),
) -> (RootTickFixture, FinancePreviewSeen, String) {
    financial_issuance_fixture_options(tag, limits, None, true)
}

fn financial_issuance_fixture_options(
    tag: &str, limits: (i64, i64), fixture_root: Option<&Path>, confirm_cost: bool,
) -> (RootTickFixture, FinancePreviewSeen, String) {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let path = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let writer = path.clone();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let call = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 {
            let db = db::open(writer.lock().unwrap().as_ref().unwrap()).unwrap();
            db.execute_batch("CREATE TRIGGER issue_cost_fault BEFORE INSERT ON agent_budget_entries
                WHEN NEW.idempotency_key LIKE '%:actual' BEGIN SELECT RAISE(ABORT,'original_fee_fault'); END;").unwrap();
            (200, "application/json", root_finite_cost_wire().to_string())
        } else {
            let mut response: JsonValue = serde_json::from_str(&proposal_model_response(
                &root_tick_valid_text("fresh approved assessment"),
            ))
            .unwrap();
            response["usage"] = json!({"prompt_tokens":31,"completion_tokens":7,"total_tokens":38,"prompt_tokens_details":{"cached_tokens":3}});
            (200, "application/json", response.to_string())
        }
    }));
    let endpoint = format!("http://127.0.0.1:{port}/v1");
    let mut f = match fixture_root {
        Some(root) => root_tick_fixture_from_web_mode(tag, &endpoint, true, limits, Some(900),
            web_mode_fixture_at_root("multi", "https://authorized.example.test", root.to_path_buf())),
        None => root_tick_fixture_protocol_limits(tag, &endpoint, true, limits),
    };
    *path.lock().unwrap() = Some(f.context.db_path.clone());
    let db = db::open(&f.context.db_path).unwrap();
    let settings = json!({"modelProfiles":[{"id":"fixture-model","llm":f.context.environment.llm,
        "apiKey":"test-key","apiBase":f.context.environment.api_base,"deployment":"cloud"}],
        "activeModelProfileId":"fixture-model","agentProxyEnabled":false});
    assert!(
        db.execute(
            "UPDATE config_profiles SET settings_json=?1",
            [settings.to_string()]
        )
        .unwrap()
            > 0
    );
    // The actual caller owns the target scope before its paid provider request.
    // Pause later proves this original inode is idle; it never creates a fake
    // historical exit proof for an already paid call.
    let target = claim_native_invocation(
        &f.context.db_path,
        &f.context.scan_id,
        1,
        "target",
        &f.context.target_url,
    )
    .unwrap();
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(target);
    drop(f.parent.take());
    assert_eq!(
        request_sentinel_pause(&f.context.db_path, &f.context.scan_id).unwrap(),
        1
    );
    assert!(finish_sentinel_pause(&f.context.db_path, &f.context.scan_id, 1).unwrap());
    let invoice = native_root_cost_reconciliations_for_attempt(&db, &f.context.scan_id, 1).unwrap();
    let hash = invoice["items"][0]["confirmationHash"]
        .as_str()
        .unwrap()
        .to_string();
    let call = invoice["items"][0]["callId"].as_str().unwrap();
    db.execute_batch("DROP TRIGGER issue_cost_fault").unwrap();
    if confirm_cost {
        Tick::confirm_cost_reconciliation(
        &db,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        call,
        &hash,
    )
        .unwrap();
    }
    (f, seen, hash)
}

#[test]
fn financial_continuation_real_pause_confirm_approve_new_root_actual_sdk_exact_remaining() {
    use crate::agent_runtime::{multi_agent::budget, store};
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("issue-new-sdk");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let hash = preview["proposalHash"].as_str().unwrap();
    let old_bytes = fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap();
    let issued = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        hash,
    )
    .expect("explicit approval must atomically issue exact remaining budget");
    assert_eq!(issued["newAttemptNumber"], 2);
    assert_eq!(issued["assessmentCompleted"], false);
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "issuance alone never calls the provider"
    );
    let new_root = issued["newRootRunId"].as_str().unwrap();
    assert_ne!(new_root, f.actor.root_run_id);
    let snapshot = db.unchecked_transaction().unwrap();
    let birth = verify_financial_continuation_birth_on(&snapshot, new_root).unwrap();
    let definition = budget::root_definition::read(&snapshot, new_root)
        .unwrap()
        .unwrap();
    assert_eq!(definition.schema_version, 3);
    assert_eq!(
        &definition.limits[..9],
        &[
            Some(150_000),
            Some(190_000),
            Some(199_990),
            Some(19),
            Some(80),
            Some(0),
            Some(0),
            Some(0),
            Some(0)
        ]
    );
    assert!(definition.limits[9].unwrap() > 0);
    let deadline: String = snapshot.query_row("SELECT strftime('%Y-%m-%d %H:%M:%f',o.started_at,printf('+%f seconds',l.hard_limit/1000.0))
        FROM agent_budget_clock_origins o JOIN agent_budget_limits l ON l.root_run_id=o.root_run_id AND l.dimension='wall_time_ms'
        WHERE o.root_run_id=?1",[new_root],|r|r.get(0)).unwrap();
    assert!(deadline.as_str() <= birth["originalDeadline"].as_str().unwrap());
    assert_eq!(
        budget::root::remaining_child_capacity(&snapshot, new_root).unwrap(),
        (Some(149_990), Some(19))
    );
    let old =
        budget::continuation::settled_source_history_on(&snapshot, &f.actor.root_run_id, &receipt)
            .unwrap()
            .as_json();
    assert_eq!(old, preview["financialSource"]);
    assert_eq!(snapshot.query_row("SELECT count(*) FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=2",[&f.context.scan_id],|r|r.get::<_,i64>(0)).unwrap(),0);
    drop(snapshot);
    let result =
        native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, new_root)
            .expect("new worker must publish an actual paid assessment");
    assert_eq!(result["assessmentCompleted"], true);
    assert_eq!(seen.lock().unwrap().len(), 2);
    let payloads = seen.lock().unwrap();
    assert_ne!(payloads[0], payloads[1], "the old request cannot be resent");
    let wire: JsonValue =
        serde_json::from_str(payloads[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    let input: JsonValue =
        serde_json::from_str(wire["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(input["basis"]["phase"], "financial-continuation");
    assert_eq!(input["basis"]["root"], new_root);
    assert_eq!(input["basis"]["attempt"], 2);
    assert_eq!(
        input["basis"]["financialContinuation"]["approvalId"],
        issued["approvalId"]
    );
    assert_eq!(
        input["basis"]["taskDescription"],
        preview["taskDescription"]
    );
    drop(payloads);
    let second =
        native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, new_root)
            .unwrap();
    assert_eq!(second["assessmentCompleted"], true);
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "reentry cannot duplicate paid SDK"
    );
    let repeated = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        hash,
    )
    .unwrap();
    assert_eq!(repeated["approvalId"], issued["approvalId"]);
    assert_eq!(repeated["assessmentCompleted"], true);
    assert_eq!(
        fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap(),
        old_bytes
    );
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(
        budget::continuation::settled_source_history_on(&snapshot, &f.actor.root_run_id, &receipt)
            .unwrap()
            .as_json(),
        preview["financialSource"]
    );
    assert_eq!(
        store::attempt_plan(&snapshot, &f.context.scan_id, 1, &f.context.target_url).unwrap(),
        f.context.execution_plan.as_json()
    );
    assert_eq!(
        snapshot
            .query_row(
                "SELECT status FROM sentinel_scans WHERE id=?1",
                [&f.context.scan_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "partial"
    );
    assert_eq!(
        snapshot
            .query_row(
                "SELECT count(*) FROM agent_runs WHERE parent_run_id=?1",
                [new_root],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn financial_continuation_bad_proposal_and_late_trigger_rollback_preserve_originals() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("issue-rollback");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let before = web_mode_test_rows(&db);
    assert_eq!(
        native_financial_continuation_approve_on(
            &f.context.db_path,
            &f.f.root,
            &f.context.scan_id,
            1,
            &f.actor.root_run_id,
            &receipt,
            &"a".repeat(64)
        )
        .unwrap_err(),
        "financial_continuation_proposal_changed"
    );
    web_mode_assert_rows(&db, &before);
    db.execute_batch(
        "CREATE TRIGGER issue_late_mutation AFTER INSERT ON agent_runs WHEN NEW.attempt_number=2
        BEGIN UPDATE agent_runs SET plan_json='{}' WHERE id<>NEW.id; END;",
    )
    .unwrap();
    assert!(native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        preview["proposalHash"].as_str().unwrap()
    )
    .is_err());
    db.execute_batch("DROP TRIGGER issue_late_mutation")
        .unwrap();
    web_mode_assert_rows(&db, &before);
    assert!(!f.f.work.parent().unwrap().join("attempt-0002").exists());
    assert_eq!(seen.lock().unwrap().len(), 1);
}
