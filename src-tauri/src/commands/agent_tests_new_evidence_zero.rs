// Real original Web/HTTP/SDK/settlement/delivery; no synthesized completed flag.
#[test]
fn new_evidence_original_finance_rejects_foreign_root_journal_without_zero_write() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_root_fixture("ne-foreign-journal", &[]);
    let db = db::open(&f.h.db_path).unwrap();
    let child = &f.session.as_ref().unwrap().executor;
    let actor = f.lease();
    let calls = f.calls();
    let original = receipt_database_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    // Discardable fixture only: model cold corruption and roll the trigger back.
    tx.execute_batch("DROP TRIGGER web_model_journal_no_update")
        .unwrap();
    assert!(
        tx.execute(
            "UPDATE agent_web_model_journal SET root_run_id=?1 WHERE child_run_id=?1",
            [&child.run_id],
        )
        .unwrap()
            > 0
    );
    let changed = receipt_database_snapshot(&tx);
    assert_eq!(
        settle_original_web_child_usage(&tx, actor, child)
            .err()
            .unwrap(),
        "web_model_lifetime_original_call_conflict"
    );
    assert_eq!(receipt_database_snapshot(&tx), changed);
    drop(tx);
    assert_eq!(receipt_database_snapshot(&db), original);
    assert_eq!(f.calls(), calls);
}

#[test]
fn new_evidence_before_transport_actual_sdk_cancellation_settles_zero_original_bill() {
    use crate::agent_runtime::model::OneShotFailure;
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(0);
    let execution = native_web_original_execution_owner(&f.h.context)
        .unwrap()
        .unwrap();
    native_coordinator_budget_before_executor(&f.h.context).unwrap();
    let specs = agent_tool_specs_for(&f.h.context);
    let (_proxy, client) = native_model_setup(&f.h.context, &specs).unwrap();
    let request = agent_model_request(
        vec![json!({"role":"user","content":"original cancelled request"})],
        &specs,
    );
    let hash = crate::agent_runtime::store::stable_hash(&json!({"messages":request.messages,
        "tools":request.tools.iter().map(|tool|tool.as_function_spec()).collect::<Vec<_>>(),
        "temperature":request.temperature,"model":client.profile().model,"endpoint":client.profile().endpoint,
        "maxOutputTokens":client.profile().max_output_tokens,"schemaHash":client.schema_hash()}).to_string());
    let call = native_model_budget_admission(&f.h.context, 1, hash)
        .unwrap()
        .unwrap();
    let calls = f.h.model_seen.lock().unwrap().len();
    let cancel = CancelToken::from_checker(|| true);
    let failure = match client.complete_once_observed(&request, &cancel) {
        Err(OneShotFailure::BeforeTransport(error)) => error,
        _ => panic!("must cancel before actual provider transport"),
    };
    assert_eq!(failure.code(), "user_cancelled");
    native_model_terminal_receipt(&f.h.context, &call, "unsent", None, failure.code()).unwrap();
    drop(call);
    drop(execution);
    let db = db::open(&f.h.db_path).unwrap();
    let child = &f.session.as_ref().unwrap().executor;
    let actor = &f.session.as_ref().unwrap().lease;
    // Missing terminal and a forged unsent code remain unresolved original bills.
    for (damage, expected) in [
        ("DROP TRIGGER web_model_journal_no_delete; DELETE FROM agent_web_model_journal WHERE phase='unsent'",
         "budget_indeterminate_requires_reconciliation"),
        ("DROP TRIGGER web_model_journal_no_update; UPDATE agent_web_model_journal SET receipt_json='{\"code\":\"other\"}' WHERE phase='unsent'",
         "web_model_original_receipt_invalid"),
    ] {
        let intact = receipt_database_snapshot(&db);
        let tx = db.unchecked_transaction().unwrap();
        tx.execute_batch(damage).unwrap();
        let damaged = receipt_database_snapshot(&tx);
        assert_eq!(settle_original_web_child_usage(&tx, actor, child).err().unwrap(), expected);
        assert_eq!(receipt_database_snapshot(&tx), damaged);
        drop(tx);
        assert_eq!(receipt_database_snapshot(&db), intact);
    }
    // Cold journal loss cannot turn an admitted original SDK into no-dispatch.
    let intact = receipt_database_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    // Discardable fixture only; rollback restores the production no-delete guard.
    tx.execute_batch("DROP TRIGGER web_model_journal_no_delete")
        .unwrap();
    assert!(
        tx.execute(
            "DELETE FROM agent_web_model_journal WHERE child_run_id=?1",
            [&child.run_id]
        )
        .unwrap()
            > 0
    );
    let lost = receipt_database_snapshot(&tx);
    assert_eq!(
        settle_original_web_child_usage(&tx, actor, child)
            .err()
            .unwrap(),
        "web_model_settlement_original_receipt_required"
    );
    assert_eq!(receipt_database_snapshot(&tx), lost);
    drop(tx);
    assert_eq!(receipt_database_snapshot(&db), intact);
    db.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01 00:00:00' WHERE root_run_id=?1", [&actor.root_run_id]).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let (usage, idle) = settle_original_web_child_usage(&tx, actor, child).unwrap();
    assert_eq!(usage, AgentTokenUsage::default());
    tx.commit().unwrap();
    drop(idle);
    let paid = receipt_database_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let (usage, idle) = settle_original_web_child_usage(&tx, actor, child).unwrap();
    assert_eq!(usage, AgentTokenUsage::default());
    tx.commit().unwrap();
    drop(idle);
    assert_eq!(receipt_database_snapshot(&db), paid);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
}

#[test]
fn new_evidence_zero_web_sdk_actual_root_deferral_finishes_without_refunding_root_calls() {
    let _real = RealSpecialistTransport::enter();
    let mut f = budget_trigger_fixture(1);
    let db = db::open(&f.h.db_path).unwrap();
    let outcome = run_native_agent(&f.h.context);
    assert!(outcome.detail().contains("root_budget_allocation_deferred"));
    let child = f.session.as_ref().unwrap().executor.clone();
    let actor = f.session.as_ref().unwrap().lease.clone();
    let calls = f.h.model_seen.lock().unwrap().len();
    assert_eq!(calls, 4);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let root_paid = NativeCoordinatorFrozenRows::capture(
        &db,
        "SELECT rowid,* FROM agent_root_tick_receipts WHERE root_run_id=?1 ORDER BY rowid",
        &[&actor.root_run_id],
    )
    .unwrap();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    let usage: (i64,i64,String) = db.query_row("SELECT r.used_tokens,r.used_requests,a.budget_settled_at FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id WHERE r.id=?1", [&child.run_id], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!((usage.0, usage.1), (0, 0));
    assert!(!usage.2.is_empty());
    let tx = db.unchecked_transaction().unwrap();
    let (zero, idle) = settle_original_web_child_usage(&tx, &actor, &child).unwrap();
    assert_eq!(zero, AgentTokenUsage::default());
    tx.commit().unwrap();
    drop(idle);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 0);
    let root_requests = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &actor.root_run_id,
        Some(""),
        "model_requests",
    )
    .unwrap();
    assert_eq!(root_requests.consumed, 3);
    root_paid.verify(&db).unwrap();
}
