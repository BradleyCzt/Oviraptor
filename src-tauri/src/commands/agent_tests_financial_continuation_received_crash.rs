// Child runs the real dedicated worker against the parent's original fixture
// path and live localhost provider. No paid database is copied/rebound.
#[test]
fn a02_test_received_invoice_subprocess_driver() {
    let Ok(path) = std::env::var("OVIRAPTOR_TEST_FINANCIAL_RECEIVED_DRIVER_DB") else {
        return;
    };
    let scan = std::env::var("OVIRAPTOR_TEST_FINANCIAL_RECEIVED_DRIVER_SCAN").unwrap();
    let root = std::env::var("OVIRAPTOR_TEST_FINANCIAL_EXIT_AFTER_RECEIVED_INVOICE").unwrap();
    let _real = RealSpecialistTransport::enter();
    native_financial_continuation_work(Path::new(&path), &scan, &root).unwrap();
    panic!("child must exit after the original received invoice commit");
}

// Snapshot the actual financial/request/birth rows separately from mutable
// failure-review metadata. Reentry may stop the attempt, never erase its bill.
fn financial_received_crash_paid_rows(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    web_mode_test_rows(db)
        .into_iter()
        .filter(|(table, _)| {
            matches!(
                table.as_str(),
                "agent_root_model_journal"
                    | "agent_root_tick_receipts"
                    | "agent_root_tick_terminal_proofs"
                    | "agent_budget_entries"
                    | "agent_budget_limits"
                    | "agent_root_budget_attempts"
                    | "agent_root_cost_reconciliation_receipts"
                    | "agent_financial_continuations"
                    | "agent_financial_root_budget_definitions"
                    | "agent_root_mode_definitions"
                    | "native_web_mode_receipts"
                    | "agent_financial_startup_bindings"
            )
        })
        .collect()
}

#[test]
fn financial_continuation_actual_subprocess_received_invoice_before_terminal_is_unsettled_no_sdk_replay(
) {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt, proposal_hash, issued) =
        financial_status_fixture("issue-received-process-exit");
    let db = db::open(&f.context.db_path).unwrap();
    let root = issued["newRootRunId"].as_str().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commands::agent_tests::a02_test_received_invoice_subprocess_driver",
            "--test-threads=1",
        ])
        .env(
            "OVIRAPTOR_TEST_FINANCIAL_RECEIVED_DRIVER_DB",
            &f.context.db_path,
        )
        .env(
            "OVIRAPTOR_TEST_FINANCIAL_RECEIVED_DRIVER_SCAN",
            &f.context.scan_id,
        )
        .env("OVIRAPTOR_TEST_FINANCIAL_EXIT_AFTER_RECEIVED_INVOICE", root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(87),
        "child stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "old paid call plus exactly one actual new assessment SDK"
    );
    let (call, raw): (String, String) = db.query_row(
        "SELECT call_id,receipt_json FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",
        [root], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    let invoice: JsonValue = serde_json::from_str(&raw).unwrap();
    assert_eq!(invoice["usageReported"], true);
    assert_eq!(
        invoice["usage"],
        json!({"inputTokens":31,"cachedInputTokens":3,
        "outputTokens":7,"totalTokens":38,"modelRequests":1})
    );
    assert_eq!(
        raw,
        invoice.to_string(),
        "real invoice is canonical, not a fixture insertion"
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'", [root], |r|r.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'", [root], |r|r.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_tick_terminal_proofs WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase IN ('decision','publication')", [root], |r|r.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1",
            [root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1 AND source_id=?2 AND kind<>'reserve'", params![root,call], |r|r.get::<_,i64>(0)).unwrap(), 0);
    for dimension in &budget::DIMENSIONS[..4] {
        let balance = budget::balance(&db, root, None, dimension).unwrap();
        assert!(
            balance.reserved > 0,
            "original {dimension} hold remains unsettled"
        );
        assert_eq!(balance.consumed, 0);
        assert_eq!(
            balance.indeterminate, 0,
            "missing terminal is its own admission obligation"
        );
    }
    assert_eq!(
        budget::admission::require_determinate(&db, root)
            .err()
            .as_deref(),
        Some("budget_indeterminate_requires_reconciliation")
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1",
            [&f.context.scan_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "scanning",
        "process exit does not execute Drop or fabricate a graceful review/closure"
    );
    // Observation and an explicit repeat approval preserve all database rows.
    let before = web_mode_test_rows(&db);
    let changes = db.total_changes();
    let status = financial_status_read(&f, &db, &receipt, &proposal_hash).unwrap();
    assert_eq!(status["status"], "review_required");
    assert_eq!(
        status["reviewReason"],
        "financial_continuation_admission_requires_review"
    );
    assert_eq!(status["assessmentCompleted"], false);
    assert_eq!(status["attemptClosed"], false);
    assert_eq!(status["workerStartAllowed"], false);
    let found = native_financial_continuation_discovery_on(&db, &f.context.scan_id, 2).unwrap();
    assert_eq!(found["issued"]["status"], status);
    let bills = native_root_cost_reconciliations_for_attempt(&db, &f.context.scan_id, 2).unwrap();
    assert_eq!(bills["items"].as_array().unwrap().len(), 1);
    assert_eq!(bills["blocked"], json!([]));
    assert_eq!(bills["items"][0]["callId"], call);
    assert_eq!(bills["items"][0]["usage"], invoice["usage"]);
    assert_eq!(bills["items"][0]["operation"], "settle_received");
    let repeated = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        &proposal_hash,
    )
    .unwrap();
    assert_eq!(repeated, status);
    web_mode_assert_rows(&db, &before);
    assert_eq!(db.total_changes(), changes);
    assert_eq!(seen.lock().unwrap().len(), 2);
    // A direct worker reentry is also denied before transport. It may publish
    // honest failure-review metadata; immutable paid facts must stay identical.
    let paid = financial_received_crash_paid_rows(&db);
    assert_eq!(
        native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, root)
            .err()
            .as_deref(),
        Some("root_tick_dispatch_unknown_no_retry")
    );
    assert_eq!(financial_received_crash_paid_rows(&db), paid);
    assert_eq!(seen.lock().unwrap().len(), 2);
    let after = financial_status_read(&f, &db, &receipt, &proposal_hash).unwrap();
    assert_eq!(
        after["reviewReason"],
        "financial_assessment_requires_review"
    );
    assert_eq!(after["assessmentCompleted"], false);
    assert_eq!(after["attemptClosed"], false);
    assert_eq!(after["workerStartAllowed"], false);
    let stopped = web_mode_test_rows(&db);
    for _ in 0..2 {
        let repeated = native_financial_continuation_approve_on(
            &f.context.db_path,
            &f.f.root,
            &f.context.scan_id,
            1,
            &f.actor.root_run_id,
            &receipt,
            &proposal_hash,
        )
        .unwrap();
        assert_eq!(repeated, after);
        assert_eq!(
            native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, root)
                .err()
                .as_deref(),
            Some("financial_continuation_current_scope_invalid")
        );
        web_mode_assert_rows(&db, &stopped);
        assert_eq!(financial_received_crash_paid_rows(&db), paid);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }
    assert_eq!(db.query_row("SELECT receipt_json FROM agent_root_model_journal WHERE call_id=?1 AND phase='received'", [&call], |r|r.get::<_,String>(0)).unwrap(), raw);
}
