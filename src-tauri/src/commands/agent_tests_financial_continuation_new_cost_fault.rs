// A paid new assessment with a failed cost writer must remain reviewable,
// never reported as published/closed or automatically sent again.
#[test]
fn financial_continuation_new_paid_invoice_writer_fault_retains_bill_and_blocks_replay() {
    let _real = RealSpecialistTransport::enter();
    let (f, seen, receipt) = financial_issuance_fixture("issue-new-cost-fault");
    let db = db::open(&f.context.db_path).unwrap();
    let preview = finance_preview_read(&f, &db, &receipt).unwrap();
    let hash = preview["proposalHash"].as_str().unwrap();
    let issued = native_financial_continuation_approve_on(
        &f.context.db_path,
        &f.f.root,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        hash,
    )
    .unwrap();
    let root = issued["newRootRunId"].as_str().unwrap();
    db.execute_batch("CREATE TRIGGER financial_new_cost_fault BEFORE INSERT ON agent_budget_entries
        WHEN NEW.kind='consume' AND NEW.dimension='model_input_tokens'
        AND NEW.idempotency_key LIKE '%:terminal' BEGIN SELECT RAISE(ABORT,'new_paid_cost_writer_fault'); END;").unwrap();
    assert!(
        native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, root).is_err()
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "the new actual paid request occurred exactly once"
    );
    let status = native_financial_continuation_status_on(
        &db,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        hash,
    )
    .unwrap();
    assert_eq!(status["status"], "review_required");
    assert_eq!(status["assessmentCompleted"], false);
    assert_eq!(status["attemptClosed"], false);
    assert_eq!(status["workerStartAllowed"], false);
    let invoices =
        native_root_cost_reconciliations_for_attempt(&db, &f.context.scan_id, 2).unwrap();
    assert_eq!(
        invoices["items"].as_array().unwrap().len(),
        1,
        "received provider usage must survive the failed writer"
    );
    let bill = &invoices["items"][0];
    assert_eq!(bill["rootRunId"], root);
    assert_eq!(bill["usage"]["inputTokens"], 31);
    assert_eq!(bill["usage"]["cachedInputTokens"], 3);
    assert_eq!(bill["usage"]["outputTokens"], 7);
    let before = web_mode_test_rows(&db);
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
    assert_eq!(repeated["status"], "review_required");
    web_mode_assert_rows(&db, &before);
    assert!(
        native_financial_continuation_work(&f.context.db_path, &f.context.scan_id, root).is_err()
    );
    assert_eq!(
        seen.lock().unwrap().len(),
        2,
        "review reentry cannot replay the paid SDK"
    );
    db.execute_batch("DROP TRIGGER financial_new_cost_fault")
        .unwrap();
    let confirmed = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::confirm_cost_reconciliation(
        &db, &f.context.scan_id, 2, root, bill["callId"].as_str().unwrap(),
        bill["confirmationHash"].as_str().unwrap()).unwrap();
    assert_eq!(confirmed["actualCostConfirmed"], true);
    assert_eq!(confirmed["executionAllowed"], false);
    let after_confirmation = native_financial_continuation_status_on(
        &db,
        &f.context.scan_id,
        1,
        &f.actor.root_run_id,
        &receipt,
        hash,
    )
    .unwrap();
    assert_eq!(after_confirmation["status"], "review_required");
    assert_eq!(after_confirmation["workerStartAllowed"], false);
    assert_eq!(seen.lock().unwrap().len(), 2);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::continuation::settled_source_history_on(
            &snapshot,
            &f.actor.root_run_id,
            &receipt
        )
        .unwrap()
        .as_json(),
        preview["financialSource"]
    );
}
