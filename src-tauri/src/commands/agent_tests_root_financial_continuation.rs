// Actual localhost invoice fault → original human confirmation → production
// read-only continuation proposal. This does not test new Root issuance.
type FinancePreviewSeen = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

fn finance_preview_fixture(
    tag: &str,
    confirm: bool,
    session: bool,
) -> (RootTickFixture, FinancePreviewSeen, String) {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let path = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let writer = path.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let db = db::open(writer.lock().unwrap().as_ref().unwrap()).unwrap();
        db.execute_batch("CREATE TRIGGER finance_preview_cost_fault BEFORE INSERT ON agent_budget_entries
            WHEN NEW.idempotency_key LIKE '%:actual' BEGIN SELECT RAISE(ABORT,'original_fee_fault'); END;").unwrap();
        (200, "application/json", root_finite_cost_wire().to_string())
    }));
    let f = root_tick_fixture_protocol_limits(
        tag,
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (200_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    *path.lock().unwrap() = Some(f.context.db_path.clone());
    // Configure the actual app profile before the first request is frozen.
    // Do not rewrite the original request to match a later model profile.
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
    let actual =
        agent_model_profile(&model_runtime_env(&sentinel_settings(&db)).unwrap(), None).unwrap();
    let original = agent_model_profile(&f.context.environment, None).unwrap();
    assert_eq!(actual.model, original.model);
    assert_eq!(actual.endpoint, original.endpoint);
    assert_eq!(actual.bearer(), original.bearer());
    assert_eq!(actual.max_context_tokens, original.max_context_tokens);
    assert_eq!(actual.max_output_tokens, original.max_output_tokens);
    if session {
        let document = json!({"schemaVersion":1,"id":"finance-session","projectId":9001,
            "scopeHosts":["authorized.example.test"],"cookies":[{"name":"session","value":"fixture-secret"}],
            "headers":{},"identityIsolation":{"sharedWithOtherSessions":false}});
        db.execute("INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,session_json,expires_at)
            VALUES('finance-session',9001,?1,'Finance session',?2,'valid',?3,?4)",
            params![f.context.scan_id,f.context.target_url,document.to_string(),
                (chrono::Utc::now()+chrono::Duration::hours(1)).to_rfc3339()]).unwrap();
        db.execute("UPDATE sentinel_scan_contexts SET policy_json=json_set(policy_json,'$.authSessionIds',json('[\"finance-session\"]')) WHERE scan_id=?1",[&f.context.scan_id]).unwrap();
    }
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    let call: String = db.query_row("SELECT call_id FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",
        [&f.actor.root_run_id], |r|r.get(0)).unwrap();
    let inventory =
        native_root_cost_reconciliations_for_attempt(&db, &f.context.scan_id, 1).unwrap();
    let hash = inventory["items"][0]["confirmationHash"]
        .as_str()
        .unwrap()
        .to_string();
    db.execute_batch("DROP TRIGGER finance_preview_cost_fault")
        .unwrap();
    if confirm {
        let receipt = Tick::confirm_cost_reconciliation(
            &db,
            &f.context.scan_id,
            1,
            &f.actor.root_run_id,
            &call,
            &hash,
        )
        .unwrap();
        assert_eq!(receipt["receiptId"], hash);
    }
    // A lower-level paused fixture; the later issuance test must use the
    // production pause/quiescence path and actual new worker, separately.
    db.execute(
        "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
        [&f.context.scan_id],
    )
    .unwrap();
    (f, seen, hash)
}

fn finance_preview_read(
    f: &RootTickFixture,
    db: &rusqlite::Connection,
    receipt: &str,
) -> Result<JsonValue, String> {
    if db.is_autocommit() {
        native_financial_continuation_preview_on(
            db,
            &f.context.scan_id,
            1,
            &f.actor.root_run_id,
            receipt,
        )
    } else {
        native_financial_continuation_preview_in(
            db,
            &f.context.scan_id,
            1,
            &f.actor.root_run_id,
            receipt,
        )
    }
}

#[test]
fn root_financial_continuation_original_invoice_confirmation_exact_readonly_preview() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = finance_preview_fixture("finance-preview-exact", true, false);
    let db = db::open(&f.context.db_path).unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let bytes = fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt)
        .expect("the original confirmed fee must yield a read-only proposal");
    assert_eq!(preview["proposedAttemptNumber"], 2);
    assert_eq!(preview["historicalRootsChecked"], 1);
    assert_eq!(preview["identityMode"], "AnonymousOnly");
    for key in [
        "executionAllowed",
        "automaticResumeAllowed",
        "originalSemanticReplayAllowed",
    ] {
        assert_eq!(preview[key], false, "{key}");
    }
    assert_eq!(preview["issuanceImplemented"], true);
    let source = &preview["financialSource"];
    assert_eq!(source["dimensions"], json!(budget::DIMENSIONS));
    assert_eq!(source["remaining"].as_array().unwrap().len(), 10);
    assert_eq!(source["remaining"][0], 150_000);
    assert_eq!(source["remaining"][1], 190_000);
    assert_eq!(source["remaining"][2], 199_990);
    assert_eq!(source["remaining"][3], 19);
    assert_eq!(source["sharedRemainingTokens"], 149_990);
    assert_eq!(source["sharedRemainingRequests"], 19);
    let deadline: String = db.query_row("SELECT strftime('%Y-%m-%d %H:%M:%f',o.started_at,printf('+%f seconds',l.hard_limit/1000.0))
        FROM agent_budget_clock_origins o JOIN agent_budget_limits l ON l.root_run_id=o.root_run_id AND l.dimension='wall_time_ms'
        WHERE o.root_run_id=?1", [&f.actor.root_run_id], |r|r.get(0)).unwrap();
    assert_eq!(source["originalDeadline"], deadline);
    for (i, dim) in budget::DIMENSIONS.iter().enumerate() {
        let cap: Option<i64> = db
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![f.actor.root_run_id, dim],
                |r| r.get(0),
            )
            .unwrap();
        let used = budget::balance(&db, &f.actor.root_run_id, None, dim).unwrap();
        assert_eq!(
            source["remaining"][i],
            json!(cap.map(|n| n - used.consumed)),
            "{dim}"
        );
    }
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert_eq!(
        finance_preview_read(&f, &db, &receipt).unwrap(),
        preview,
        "absolute deadline, never a renewed TTL"
    );
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(
        fs::read(f.context.target_dir.join("frontend-evidence.json")).unwrap(),
        bytes
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert!(
        native_coordinator_tick(&f.context, &f.actor).is_err(),
        "paid original semantic work is still not replayable"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_financial_continuation_unconfirmed_and_cross_scope_preview_never_writes() {
    let _real = RealSpecialistTransport::enter();
    for confirmed in [false, true] {
        let (f, seen, receipt) = finance_preview_fixture("finance-preview-scope", confirmed, false);
        let db = db::open(&f.context.db_path).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        if !confirmed {
            assert!(finance_preview_read(&f, &db, &receipt).is_err());
        }
        for (scan, attempt, root, hash) in [
            (
                "other-scan",
                1,
                f.actor.root_run_id.as_str(),
                receipt.as_str(),
            ),
            (
                f.context.scan_id.as_str(),
                2,
                f.actor.root_run_id.as_str(),
                receipt.as_str(),
            ),
            (
                f.context.scan_id.as_str(),
                1,
                "other-root",
                receipt.as_str(),
            ),
            (
                f.context.scan_id.as_str(),
                1,
                f.actor.root_run_id.as_str(),
                "bad-receipt",
            ),
        ] {
            assert!(
                native_financial_continuation_preview_on(&db, scan, attempt, root, hash).is_err()
            );
            web_mode_assert_rows(&db, &before.0);
            assert_eq!(single_finally_physical(&db), before.1);
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn root_financial_continuation_authority_model_and_session_revalidated_readonly() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = finance_preview_fixture("finance-preview-revoked", true, true);
    let db = db::open(&f.context.db_path).unwrap();
    let accepted = finance_preview_read(&f, &db, &receipt).unwrap();
    assert_eq!(accepted["identityMode"], "SingleIdentity");
    for sql in [
        "UPDATE sentinel_scans SET status='scanning'",
        "UPDATE sentinel_scans SET attempt_count=2",
        "UPDATE projects SET status='archived'",
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00'",
        "UPDATE agent_coordinator_leases SET fencing_token='revoked-fence'",
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].apiKey','changed-key')",
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelProfiles[0].llm','openai/changed-model')",
        "UPDATE browser_auth_sessions SET status='invalid'",
        "UPDATE browser_auth_sessions SET expires_at='2000-01-01T00:00:00Z'",
        "UPDATE browser_auth_sessions SET owner_scan_id='other-scan'",
        "UPDATE browser_auth_sessions SET session_json=json_set(session_json,'$.scopeHosts',json('[\"other.test\"]'))",
        "UPDATE browser_auth_sessions SET session_json='{}'",
    ] {
        db.execute_batch("SAVEPOINT finance_mutation").unwrap();
        db.execute_batch(sql).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(finance_preview_read(&f, &db, &receipt).is_err(), "{sql}");
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1, "{sql}");
        db.execute_batch("ROLLBACK TO finance_mutation; RELEASE finance_mutation").unwrap();
        assert_eq!(finance_preview_read(&f, &db, &receipt).unwrap(), accepted);
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_financial_continuation_original_financial_proof_rejects_physical_tamper() {
    use crate::agent_runtime::multi_agent::budget::continuation;
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = finance_preview_fixture("finance-preview-tamper", true, false);
    let db = db::open(&f.context.db_path).unwrap();
    assert!(
        continuation::settled_source_on(&db, &f.actor.root_run_id, &receipt).is_err(),
        "snapshot is mandatory"
    );
    let proof = {
        let tx = db.unchecked_transaction().unwrap();
        continuation::settled_source_on(&tx, &f.actor.root_run_id, &receipt).unwrap()
    };
    for sql in [
        "DROP TRIGGER budget_entry_no_update; UPDATE agent_budget_entries SET amount=amount+1 WHERE dimension='model_input_tokens' AND kind='consume'",
        "DROP TRIGGER budget_limit_no_update; UPDATE agent_budget_limits SET hard_limit=hard_limit+1 WHERE dimension='model_input_tokens'",
        "DROP TRIGGER root_tick_terminal_proof_no_update; UPDATE agent_root_tick_terminal_proofs SET fact_json='{}'",
        "DROP TRIGGER root_cost_reconciliation_no_update; UPDATE agent_root_cost_reconciliation_receipts SET invoice_hash=printf('%064d',0)",
    ] {
        db.execute_batch("SAVEPOINT finance_corruption").unwrap();
        db.execute_batch(sql).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(proof.verify_on(&db).is_err(), "{sql}");
        assert!(finance_preview_read(&f, &db, &receipt).is_err(), "{sql}");
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        db.execute_batch("ROLLBACK TO finance_corruption; RELEASE finance_corruption").unwrap();
        let tx = db.unchecked_transaction().unwrap();
        proof.verify_on(&tx).unwrap();
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_financial_continuation_additional_unproven_charge_or_reservation_blocks_preview() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = finance_preview_fixture("finance-preview-obligation", true, false);
    let db = db::open(&f.context.db_path).unwrap();
    let accepted = finance_preview_read(&f, &db, &receipt).unwrap();
    for (dimension, kind) in [
        ("target_requests", "reserve"),
        ("model_input_tokens", "consume"),
        ("model_requests", "forfeit"),
    ] {
        db.execute_batch("SAVEPOINT finance_extra").unwrap();
        // Deliberately inject an extra physical obligation into this isolated
        // fixture. The confirmed original call cannot account for this source.
        db.execute(
            "INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,
            dimension,kind,amount,idempotency_key,source_id)
            SELECT 'extra-finance',root_run_id,'',id,?2,?3,1,'extra-finance','unproven-extra'
            FROM agent_root_budget_attempts WHERE root_run_id=?1",
            params![f.actor.root_run_id, dimension, kind],
        )
        .unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        assert!(
            finance_preview_read(&f, &db, &receipt).is_err(),
            "{dimension}/{kind}"
        );
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        db.execute_batch("ROLLBACK TO finance_extra; RELEASE finance_extra")
            .unwrap();
        assert_eq!(finance_preview_read(&f, &db, &receipt).unwrap(), accepted);
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_financial_continuation_durable_owner_cannot_hide_missing_historical_root() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = finance_preview_fixture("finance-preview-hidden-history", true, false);
    let db = db::open(&f.context.db_path).unwrap();
    finance_preview_read(&f, &db, &receipt).unwrap();
    // Corrupt only this disposable database with an additional unproven owner.
    // It has no live Root row and must still be seen through durable lineage.
    db.execute(
        "INSERT INTO agent_root_budget_attempts(id,root_run_id,contract_json)
        VALUES('hidden-owner','hidden-old-root',?1)",
        [json!({"root":{"scan":f.context.scan_id}}).to_string()],
    )
    .unwrap();
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert_eq!(
        finance_preview_read(&f, &db, &receipt).unwrap_err(),
        "financial_continuation_history_scope_missing"
    );
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
