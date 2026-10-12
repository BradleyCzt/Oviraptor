// Selection-only fixture: these rows are not original paid bills or valid
// confirmation receipts. Full receipt validation stays in the inventory path.
#[test]
fn root_cost_cache_unsigned_bit_cannot_be_promoted_to_provider_reported() {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
        let mut body = root_finite_cost_wire();
        for key in ["prompt_tokens_details", "input_tokens_details", "cached_tokens", "prompt_cache_hit_tokens"] {
            body["usage"].as_object_mut().unwrap().remove(key);
        }
        (200, "application/json", body.to_string())
    }));
    let f = root_tick_fixture_protocol_limits("cache-reader-authentic-origin", &format!("http://127.0.0.1:{port}/v1"), true, (200_000,20));
    let _ = native_coordinator_tick(&f.context, &f.actor);
    let db = db::open(&f.context.db_path).unwrap();
    let call: String = db.query_row("SELECT call_id FROM agent_root_model_journal WHERE phase='received'", [], |r|r.get(0)).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    assert!(!Tick::read_original_cache_for_test(&snapshot, &f.actor.root_run_id, &call).unwrap());
    snapshot.rollback().unwrap();
    let ddl: String = db.query_row("SELECT sql FROM sqlite_master WHERE name='root_cache_provenance_no_update'",[],|r|r.get(0)).unwrap();
    // Cold fixture restores the exact schema after changing only an unsealed
    // bit. Binding and schema checks alone must not mint provider provenance.
    db.execute_batch("DROP TRIGGER root_cache_provenance_no_update; UPDATE agent_root_model_cache_provenance SET cached_usage_reported=1;").unwrap();
    db.execute_batch(&ddl).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert!(!Tick::read_original_cache_for_test(&snapshot, &f.actor.root_run_id, &call).unwrap_or(false),
        "original capture authentication is required; an unsigned bit cannot turn missing cache into a reported zero");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_cost_cache_reader_rejects_foreign_binding_and_noop_immutable_trigger() {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let _real = RealSpecialistTransport::enter();
    for field in ["root_run_id", "request_hash", "response_hash", "trigger"] {
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", root_finite_cost_wire().to_string())));
        let f = root_tick_fixture_protocol_limits("cache-reader-binding", &format!("http://127.0.0.1:{port}/v1"), true, (200_000,20));
        let _ = native_coordinator_tick(&f.context, &f.actor);
        let db = db::open(&f.context.db_path).unwrap();
        let call: String = db.query_row("SELECT call_id FROM agent_root_model_journal WHERE phase='received'", [], |r|r.get(0)).unwrap();
        let snapshot = db.unchecked_transaction().unwrap();
        assert!(Tick::read_original_cache_for_test(&snapshot, &f.actor.root_run_id, &call).unwrap());
        snapshot.rollback().unwrap();
        let ddl: String = db.query_row("SELECT sql FROM sqlite_master WHERE name='root_cache_provenance_no_update'",[],|r|r.get(0)).unwrap();
        db.execute_batch("DROP TRIGGER root_cache_provenance_no_update").unwrap();
        if field == "trigger" {
            db.execute_batch("CREATE TRIGGER root_cache_provenance_no_update BEFORE UPDATE ON agent_root_model_cache_provenance BEGIN SELECT 1; END;").unwrap();
        } else {
            // Fixed fixture-only column choices, never application SQL input.
            db.execute(&format!("UPDATE agent_root_model_cache_provenance SET {field}=?1 WHERE call_id=?2"), params!["f".repeat(64),call]).unwrap();
            db.execute_batch(&ddl).unwrap();
        }
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        let error = Tick::read_original_cache_for_test(&snapshot, &f.actor.root_run_id, &call).unwrap_err();
        assert_eq!(error, if field == "trigger" { "root_cache_provenance_schema_conflict" } else { "root_cache_provenance_original_binding_conflict" });
        snapshot.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}

#[test]
fn root_cost_cache_reader_legacy_absence_is_unknown_and_never_backfilled() {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", root_finite_cost_wire().to_string())));
    let f = root_tick_fixture_protocol_limits("cache-reader-legacy", &format!("http://127.0.0.1:{port}/v1"), true, (200_000,20));
    let _ = native_coordinator_tick(&f.context, &f.actor);
    let db = db::open(&f.context.db_path).unwrap();
    let call: String = db.query_row("SELECT call_id FROM agent_root_model_journal WHERE phase='received'", [], |r|r.get(0)).unwrap();
    let ddl: String = db.query_row("SELECT sql FROM sqlite_master WHERE name='root_cache_provenance_no_delete'",[],|r|r.get(0)).unwrap();
    // Disposable historical fixture; production immutable schema is untouched.
    db.execute_batch("DROP TRIGGER root_cache_provenance_no_delete; DELETE FROM agent_root_model_cache_provenance;").unwrap();
    db.execute_batch(&ddl).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert!(!Tick::read_original_cache_for_test(&snapshot, &f.actor.root_run_id, &call).unwrap());
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_cost_confirmation_inventory_prioritizes_unconfirmed_beyond_first_hundred() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE agent_runs(id TEXT,backend TEXT,role TEXT,scan_id TEXT,attempt_number INTEGER);
        CREATE TABLE agent_root_model_journal(root_run_id TEXT,call_id TEXT,phase TEXT,round INTEGER);
        CREATE TABLE agent_root_cost_reconciliation_receipts(call_id TEXT);
        CREATE TABLE agent_root_tick_terminal_proofs(call_id TEXT);
        CREATE TABLE agent_budget_entries(source_id TEXT,kind TEXT);
        INSERT INTO agent_runs VALUES('root','native','coordinator','scan',1);").unwrap();
    for round in 1..=101 {
        let call = format!("call-{round:03}");
        db.execute("INSERT INTO agent_root_model_journal VALUES('root',?1,'received',?2)",params![call,round]).unwrap();
        if round <= 100 {
            db.execute("INSERT INTO agent_root_cost_reconciliation_receipts VALUES(?1)",[&call]).unwrap();
        }
    }
    db.execute("INSERT INTO agent_root_model_journal VALUES('root','unknown-call','uncertain',102)", []).unwrap();
    db.execute("INSERT INTO agent_root_model_journal VALUES('root','call-101','uncertain',103)", []).unwrap();
    let changes = db.total_changes();
    let calls = native_root_cost_reconciliation_calls_on(&db,"scan",1).unwrap();
    assert_eq!(calls.len(),101);
    assert_eq!(calls[0].1,"call-101","confirmed first hundred cannot hide the next pending bill");
    assert_eq!(calls[1].1,"unknown-call","unknown bills remain visible ahead of confirmed receipts, after actionable bills");
    assert_eq!(calls.iter().map(|r| &r.1).collect::<std::collections::HashSet<_>>().len(),101);
    assert!(native_root_cost_reconciliation_calls_on(&db,"other-scan",1).unwrap().is_empty());
    assert!(native_root_cost_reconciliation_calls_on(&db,"scan",2).unwrap().is_empty());
    assert_eq!(db.total_changes(),changes,"selection is read-only");
}

#[test]
fn root_cost_confirmation_atomic_writer_fault_binding_and_concurrent_ack() {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let _real = RealSpecialistTransport::enter();
    let path = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let writer = path.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let db = db::open(writer.lock().unwrap().as_ref().unwrap()).unwrap();
        db.execute_batch("CREATE TRIGGER confirm_original_writer_fault BEFORE INSERT ON agent_budget_entries
            WHEN NEW.idempotency_key LIKE '%:actual' BEGIN SELECT RAISE(ABORT,'original_cost_fault'); END;").unwrap();
        (200, "application/json", root_finite_cost_wire().to_string())
    }));
    let f = root_tick_fixture_protocol_limits(
        "confirm-writer",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (200_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    *path.lock().unwrap() = Some(f.context.db_path.clone());
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    let root = f.actor.root_run_id.clone();
    let scan = f.context.scan_id.clone();
    let call: String = db.query_row("SELECT call_id FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&root],|r|r.get(0)).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    assert!(crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::read_original_cache_for_test(&snapshot, &root, &call).unwrap());
    snapshot.rollback().unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_cache_provenance p
        JOIN agent_root_model_journal j ON j.call_id=p.call_id AND j.root_run_id=p.root_run_id
          AND j.lease_attempt_id=p.lease_attempt_id AND j.round=p.round AND j.request_hash=p.request_hash
          AND j.phase='received' AND json_extract(j.receipt_json,'$.responseHash')=p.response_hash
        WHERE p.call_id=?1 AND p.cached_usage_reported=1", [&call], |r|r.get::<_,i64>(0)).unwrap(), 1,
        "original v3 cache provenance survives the later cost writer fault; confirmation cannot backfill it");
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    let inventory = native_root_cost_reconciliations_for_attempt(&db, &scan, 1).unwrap();
    assert_eq!(inventory["items"].as_array().unwrap().len(), 1);
    assert_eq!(inventory["executionAllowed"], false);
    let preview = inventory["items"][0].clone();
    let hash = preview["confirmationHash"].as_str().unwrap().to_string();
    assert_eq!(preview["usage"]["inputTokens"], 50_000);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(
        single_finally_physical(&db),
        before.1,
        "preview is physically read-only"
    );
    for (wrong_scan, wrong_attempt, wrong_root, wrong_call, wrong_hash) in [
        (
            "another-scan".to_string(),
            1,
            root.clone(),
            call.clone(),
            hash.clone(),
        ),
        (scan.clone(), 2, root.clone(), call.clone(), hash.clone()),
        (
            scan.clone(),
            1,
            "another-root".to_string(),
            call.clone(),
            hash.clone(),
        ),
        (
            scan.clone(),
            1,
            root.clone(),
            "another-call".to_string(),
            hash.clone(),
        ),
        (scan.clone(), 1, root.clone(), call.clone(), "0".repeat(64)),
    ] {
        assert!(Tick::confirm_cost_reconciliation(
            &db,
            &wrong_scan,
            wrong_attempt,
            &wrong_root,
            &wrong_call,
            &wrong_hash
        )
        .is_err());
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
    }
    assert!(
        Tick::confirm_cost_reconciliation(&db, &scan, 1, &root, &call, &hash).is_err(),
        "the original cost writer fault also rolls back explicit confirmation"
    );
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    db.execute_batch("DROP TRIGGER confirm_original_writer_fault")
        .unwrap();
    // Fail after settlement and terminal-proof INSERT. The outer transaction
    // must roll them back together with the absent confirmation receipt.
    db.execute_batch("CREATE TRIGGER confirm_receipt_fault BEFORE INSERT ON agent_root_cost_reconciliation_receipts
        BEGIN SELECT RAISE(ABORT,'confirmation_receipt_fault'); END;").unwrap();
    let fault = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert!(Tick::confirm_cost_reconciliation(&db, &scan, 1, &root, &call, &hash).is_err());
    web_mode_assert_rows(&db, &fault.0);
    assert_eq!(single_finally_physical(&db), fault.1);
    db.execute_batch("DROP TRIGGER confirm_receipt_fault")
        .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let (path, scan, root, call, hash, barrier) = (
            f.context.db_path.clone(),
            scan.clone(),
            root.clone(),
            call.clone(),
            hash.clone(),
            barrier.clone(),
        );
        handles.push(std::thread::spawn(move || {
            let db = db::open(&path).unwrap();
            barrier.wait();
            Tick::confirm_cost_reconciliation(&db, &scan, 1, &root, &call, &hash)
        }));
    }
    barrier.wait();
    let first = handles.remove(0).join().unwrap().unwrap();
    assert_eq!(handles.remove(0).join().unwrap().unwrap(), first);
    assert_eq!(first["receiptId"], hash);
    assert_eq!(first["actualCostConfirmed"], true);
    assert_eq!(first["executionAllowed"], false);
    let after = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert_eq!(
        Tick::confirm_cost_reconciliation(&db, &scan, 1, &root, &call, &hash).unwrap(),
        first
    );
    let inventory = native_root_cost_reconciliations_for_attempt(&db, &scan, 1).unwrap();
    assert!(inventory["items"].as_array().unwrap().is_empty());
    assert!(inventory["blocked"].as_array().unwrap().is_empty());
    assert_eq!(inventory["receipts"], json!([first]));
    web_mode_assert_rows(&db, &after.0);
    assert_eq!(single_finally_physical(&db), after.1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_cost_reconciliation_receipts",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    for (dimension, actual) in [
        ("model_input_tokens", 50_000),
        ("model_cached_tokens", 10_000),
        ("model_output_tokens", 10),
        ("model_requests", 1),
    ] {
        let b = crate::agent_runtime::multi_agent::budget::balance(&db, &root, None, dimension)
            .unwrap();
        assert_eq!((b.consumed, b.reserved, b.indeterminate), (actual, 0, 0));
    }
    for sql in [
        "UPDATE agent_root_cost_reconciliation_receipts SET invoice_hash=invoice_hash",
        "DELETE FROM agent_root_cost_reconciliation_receipts",
        "INSERT OR REPLACE INTO agent_root_cost_reconciliation_receipts SELECT * FROM agent_root_cost_reconciliation_receipts",
    ] {
        assert!(db.execute(sql,[]).is_err());
        web_mode_assert_rows(&db,&after.0);
        assert_eq!(single_finally_physical(&db),after.1);
    }
    assert!(
        native_coordinator_tick(&f.context, &f.actor).is_err(),
        "original semantic result is still missing"
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "confirmation cannot retry paid SDK work"
    );
}

#[test]
fn root_cost_confirmation_bad_and_over_cap_bills_stay_blocked() {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let _real = RealSpecialistTransport::enter();
    for (tag,wire,cap) in [
        ("over-cap",root_finite_cost_wire().to_string(),40_000),
        ("missing",json!({"choices":[{"message":{"content":"{}"}}]}).to_string(),200_000),
        ("inconsistent",json!({"choices":[{"message":{"content":"{}"}}],"usage":{"prompt_tokens":2,"completion_tokens":3,"total_tokens":1}}).to_string(),200_000),
    ] {
        let (port,seen,_) = spawn_endpoint(std::sync::Arc::new(move |_| (200,"application/json",wire.clone())));
        let f = root_tick_fixture_protocol_limits(tag,&format!("http://127.0.0.1:{port}/v1"),true,(cap,20));
        let db = db::open(&f.context.db_path).unwrap();
        assert!(native_coordinator_tick(&f.context,&f.actor).is_err());
        let call: String = db.query_row("SELECT call_id FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&f.actor.root_run_id],|r|r.get(0)).unwrap();
        let before = (web_mode_test_rows(&db),single_finally_physical(&db));
        let inventory = native_root_cost_reconciliations_for_attempt(&db,&f.context.scan_id,1).unwrap();
        assert!(inventory["items"].as_array().unwrap().is_empty());
        assert_eq!(inventory["blocked"].as_array().unwrap().len(),1);
        for _ in 0..2 {
            assert!(Tick::confirm_cost_reconciliation(&db,&f.context.scan_id,1,&f.actor.root_run_id,&call,&"0".repeat(64)).is_err());
        }
        web_mode_assert_rows(&db,&before.0);
        assert_eq!(single_finally_physical(&db),before.1);
        assert_eq!(seen.lock().unwrap().len(),1);
    }
}

#[test]
fn root_cost_confirmation_exact_forfeit_receipt_and_partial_cost_mismatch() {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let _real = RealSpecialistTransport::enter();
    for exact in [true, false] {
        let path = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
        let writer = path.clone();
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
            let db = db::open(writer.lock().unwrap().as_ref().unwrap()).unwrap();
            let (root, held): (String, i64) = db
                .query_row(
                    "SELECT root_run_id,json_extract(receipt_json,'$.reservedTokens')
                 FROM agent_root_model_journal WHERE phase='dispatch'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            db.execute("INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,reserved_tokens)
                VALUES(?1,200000,20,1)",[&root]).unwrap();
            let mut response = root_finite_cost_wire();
            response["usage"] = json!({"prompt_tokens":held,"completion_tokens":if exact {held} else {0},
                "total_tokens":if exact {2*held} else {held},
                "prompt_tokens_details":{"cached_tokens":if exact {held} else {0}}});
            (200, "application/json", response.to_string())
        }));
        let f = root_tick_fixture_protocol_limits(
            "confirm-forfeit",
            &format!("http://127.0.0.1:{port}/v1"),
            true,
            (200_000, 20),
        );
        *path.lock().unwrap() = Some(f.context.db_path.clone());
        let db = db::open(&f.context.db_path).unwrap();
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        db.execute(
            "UPDATE agent_budget_ledger SET reserved_tokens=0 WHERE root_run_id=?1",
            [&f.actor.root_run_id],
        )
        .unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        let inventory =
            native_root_cost_reconciliations_for_attempt(&db, &f.context.scan_id, 1).unwrap();
        let call: String = db
            .query_row(
                "SELECT call_id FROM agent_root_model_journal WHERE phase='received'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        if exact {
            let p = &inventory["items"][0];
            assert_eq!(p["operation"], "reconcile_forfeit");
            let hash = p["confirmationHash"].as_str().unwrap();
            let receipt = Tick::confirm_cost_reconciliation(
                &db,
                &f.context.scan_id,
                1,
                &f.actor.root_run_id,
                &call,
                hash,
            )
            .unwrap();
            let after = (web_mode_test_rows(&db), single_finally_physical(&db));
            for (old, new) in before.1.iter().zip(&after.1) {
                if !matches!(
                    old.0.as_str(),
                    "agent_budget_entries" | "agent_root_cost_reconciliation_receipts"
                ) {
                    assert_eq!(
                        old, new,
                        "original invoices, terminal proofs and authority stay physical"
                    );
                }
            }
            assert_eq!(
                Tick::confirm_cost_reconciliation(
                    &db,
                    &f.context.scan_id,
                    1,
                    &f.actor.root_run_id,
                    &call,
                    hash
                )
                .unwrap(),
                receipt
            );
            assert_eq!(
                native_root_cost_reconciliations_for_attempt(&db, &f.context.scan_id, 1).unwrap()
                    ["receipts"],
                json!([receipt])
            );
            web_mode_assert_rows(&db, &after.0);
            assert_eq!(single_finally_physical(&db), after.1);
        } else {
            assert!(inventory["items"].as_array().unwrap().is_empty());
            assert_eq!(
                inventory["blocked"][0]["reason"],
                "root_tick_reconcile_actual_cost_mismatch"
            );
            assert!(Tick::confirm_cost_reconciliation(
                &db,
                &f.context.scan_id,
                1,
                &f.actor.root_run_id,
                &call,
                &"0".repeat(64)
            )
            .is_err());
            web_mode_assert_rows(&db, &before.0);
            assert_eq!(single_finally_physical(&db), before.1);
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}


#[test]
fn root_cost_cache_capture_corruption_and_missing_seal_never_reseal() {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let _real = RealSpecialistTransport::enter();
    for missing in [false, true] {
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| (200, "application/json", root_finite_cost_wire().to_string())));
        let f = root_tick_fixture_protocol_limits("cache-capture-seal", &format!("http://127.0.0.1:{port}/v1"), true, (200_000,20));
        let _ = native_coordinator_tick(&f.context, &f.actor);
        let db = db::open(&f.context.db_path).unwrap();
        let call: String = db.query_row("SELECT call_id FROM agent_root_model_journal WHERE phase='received'", [], |r|r.get(0)).unwrap();
        let snapshot = db.unchecked_transaction().unwrap();
        assert!(Tick::read_original_cache_for_test(&snapshot, &f.actor.root_run_id, &call).unwrap());
        snapshot.rollback().unwrap();
        assert!(db.execute("UPDATE agent_root_model_cache_capture SET signature=zeroblob(32)", []).is_err());
        assert!(db.execute("DELETE FROM agent_root_model_cache_capture", []).is_err());
        assert!(db.execute("INSERT OR REPLACE INTO agent_root_model_cache_capture SELECT * FROM agent_root_model_cache_capture", []).is_err());
        let trigger = if missing { "root_cache_capture_no_delete" } else { "root_cache_capture_no_update" };
        let ddl: String = db.query_row("SELECT sql FROM sqlite_master WHERE name=?1", [trigger], |r|r.get(0)).unwrap();
        db.execute_batch(&format!("DROP TRIGGER {trigger}")).unwrap();
        if missing { db.execute("DELETE FROM agent_root_model_cache_capture", []).unwrap(); }
        else { db.execute("UPDATE agent_root_model_cache_capture SET signature=zeroblob(32)", []).unwrap(); }
        db.execute_batch(&ddl).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let snapshot = db.unchecked_transaction().unwrap();
        let result = Tick::read_original_cache_for_test(&snapshot, &f.actor.root_run_id, &call);
        snapshot.rollback().unwrap();
        if missing { assert!(!result.unwrap()); }
        else { assert_eq!(result.unwrap_err(), "root_cache_capture_signature_invalid"); }
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }
}
