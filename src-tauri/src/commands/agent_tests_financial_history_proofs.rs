#[test]
fn financial_history_original_http_requires_original_allowed_tool_invocation() {
    use crate::agent_runtime::multi_agent::budget::continuation;
    let (f, selected, scope) = protection_recovery_approval_fixture("finance-http-original-tool-policy");
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.actor.root_run_id;
    let models = f.h.model_seen.lock().unwrap().len();
    let targets = f.h.site_seen.lock().unwrap().len();
    assert!(models > 0 && targets > 0);
    let snapshot = db.unchecked_transaction().unwrap();
    continuation::verify_original_targets_on(&snapshot, root).unwrap();
    native_protection_recovery_proposal_in(&snapshot, &f.h.db_path, &selected, &scope).unwrap();
    snapshot.rollback().unwrap();
    let (run, invocation): (String, String) = db.query_row(
        "SELECT c.run_id,c.invocation_id FROM agent_http_request_claims c JOIN agent_runs r ON r.id=c.run_id
         WHERE r.id=?1 OR r.root_run_id=?1 ORDER BY c.rowid LIMIT 1", [root], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    // Cold damage in a disposable actual paid fixture, never the user's data.
    assert_eq!(db.execute("UPDATE tool_invocations SET policy_decision='deny' WHERE run_id=?1 AND invocation_id=?2",
        params![run, invocation]).unwrap(), 1);
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(continuation::verify_original_targets_on(&snapshot, root).unwrap_err(),
        "continuation_original_target_owner_conflict");
    assert!(native_protection_recovery_proposal_in(&snapshot, &f.h.db_path, &selected, &scope).is_err());
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), models);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), targets);
}

// Actual existing SDK/driver fixtures. These tests never fabricate provider
// invoices, paid terminal proofs or child closure receipts.
#[test]
fn financial_history_original_http_rejects_extra_same_source_charge_keys() {
    use crate::agent_runtime::multi_agent::budget::continuation;
    let (f, selected, scope) = protection_recovery_approval_fixture("finance-extra-http-keys");
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.actor.root_run_id;
    let models = f.h.model_seen.lock().unwrap().len();
    let targets = f.h.site_seen.lock().unwrap().len();
    assert!(models > 0 && targets > 0, "must audit original actual SDK and HTTP costs");
    let snapshot = db.unchecked_transaction().unwrap();
    continuation::verify_original_targets_on(&snapshot, root).unwrap();
    native_protection_recovery_proposal_in(&snapshot, &f.h.db_path, &selected, &scope).unwrap();
    snapshot.rollback().unwrap();
    let source: String = db.query_row("SELECT source_id FROM agent_budget_entries WHERE root_run_id=?1
        AND dimension='target_requests' AND source_id LIKE 'http:%' ORDER BY rowid LIMIT 1", [root], |r| r.get(0)).unwrap();
    let originals: Vec<String> = db.prepare("SELECT entry_id FROM agent_budget_entries
        WHERE root_run_id=?1 AND dimension='target_requests' AND source_id=?2 ORDER BY rowid").unwrap()
        .query_map(params![root, source], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(originals.len(), 3);
    // Cold extra-triplet injection, with no second physical request. Existing
    // exact original keys remain intact; the added wrong keys cannot be paid proof.
    for original in originals {
        assert_eq!(db.execute("INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,
            dimension,kind,amount,idempotency_key,source_id,created_at)
            SELECT ?1,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,?2,source_id,created_at
            FROM agent_budget_entries WHERE entry_id=?3",
            params![Uuid::new_v4().to_string(), format!("extra-target-key:{}", Uuid::new_v4()), original]).unwrap(), 1);
    }
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(continuation::verify_original_targets_on(&snapshot, root).unwrap_err(), "continuation_original_target_cost_conflict");
    assert!(native_protection_recovery_proposal_in(&snapshot, &f.h.db_path, &selected, &scope).is_err());
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), models);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), targets);
}

#[test]
fn financial_history_multi_rejects_balanced_unimplemented_action_records() {
    use crate::agent_runtime::multi_agent::budget::continuation;
    let (f, seen, receipt) = financial_issuance_fixture("multi-unsupported-action-proof");
    let db = db::open(&f.context.db_path).unwrap();
    let root = &f.actor.root_run_id;
    let models = seen.lock().unwrap().len();
    let snapshot = db.unchecked_transaction().unwrap();
    continuation::settled_source_history_on(&snapshot, root, &receipt).unwrap();
    snapshot.rollback().unwrap();
    let control: String = db.query_row("SELECT id FROM agent_root_budget_attempts WHERE root_run_id=?1", [root], |r| r.get(0)).unwrap();
    for kind in ["reserve", "release"] {
        db.execute("INSERT INTO agent_budget_entries(entry_id,root_run_id,assignment_id,lease_attempt_id,
            dimension,kind,amount,idempotency_key,source_id) VALUES(?1,?2,'',?3,'concurrency_batches',?4,1,?5,'unsupported-original-batch')",
            params![Uuid::new_v4().to_string(), root, control, kind, Uuid::new_v4().to_string()]).unwrap();
    }
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(continuation::settled_source_history_on(&snapshot, root, &receipt).err().unwrap(),
        "continuation_unsupported_original_action_proof");
    assert_eq!(continuation::verify_settled_history_on(&snapshot, root).unwrap_err(),
        "continuation_unsupported_original_action_proof");
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), models);
}

#[test]
fn financial_history_input_parser_original_closed_receipt_is_readonly() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_production_fixture("finance-ip-original");
    let db = db::open(&f.h.db_path).unwrap();
    let assignment: String = db
        .query_row(
            "SELECT id FROM agent_assignments WHERE role='input_parser'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let providers = f.h.model_seen.lock().unwrap().len();
    let targets = f.h.site_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let before = web_mode_test_rows(&tx);
    let changes = tx.total_changes();
    crate::agent_runtime::multi_agent::specialist::verify_received_original(
        &tx,
        f.lease(),
        &assignment,
    )
    .expect("actual delivered InputParser must retain its full original paid proof");
    assert_eq!(tx.total_changes(), changes);
    web_mode_assert_rows(&tx, &before);
    tx.rollback().unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), providers);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), targets);
}

#[test]
fn financial_history_input_parser_original_cost_mismatch_fails_without_repair() {
    let _real = RealSpecialistTransport::enter();
    let f = input_parser_production_fixture("finance-ip-cost-conflict");
    let db = db::open(&f.h.db_path).unwrap();
    let assignment: String = db
        .query_row(
            "SELECT id FROM agent_assignments WHERE role='input_parser'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let providers = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::specialist::verify_received_original(
        &tx,
        f.lease(),
        &assignment,
    )
    .unwrap();
    // Cold damage simulation in this disposable fixture only. The savepoint's
    // rollback restores both original fees and the production immutable guard.
    tx.execute_batch("DROP TRIGGER budget_entry_no_update")
        .unwrap();
    let changed=tx.execute("UPDATE agent_budget_entries SET amount=amount+1 WHERE entry_id=(
        SELECT entry_id FROM agent_budget_entries WHERE assignment_id=?1 AND dimension='model_input_tokens'
        AND kind='consume' ORDER BY rowid LIMIT 1)",[&assignment]).unwrap();
    assert_eq!(
        changed, 1,
        "must corrupt a real charged original invoice row"
    );
    let before = web_mode_test_rows(&tx);
    let changes = tx.total_changes();
    assert!(
        crate::agent_runtime::multi_agent::specialist::verify_received_original(
            &tx,
            f.lease(),
            &assignment
        )
        .is_err()
    );
    assert_eq!(tx.total_changes(), changes);
    web_mode_assert_rows(&tx, &before);
    tx.rollback().unwrap();
    let tx = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::specialist::verify_received_original(
        &tx,
        f.lease(),
        &assignment,
    )
    .unwrap();
    tx.rollback().unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), providers);
}

fn financial_history_public_actual_fixture(
    tag: &str,
) -> (
    PathBuf,
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    Seen,
    Seen,
) {
    let (port, targets, stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "text/html",
            "<html>observed public entry</html>".into(),
        )
    }));
    let target = format!("http://127.0.0.1:{port}/login#ignored");
    let (root, path, run, actor) = multi_agent_new_task_root_for_target(tag, 60_000, 20, &target);
    let mut context = test_context(&path, &target, vec![AgentIdentity::anonymous()]);
    context.scan_id = actor.scan_id.clone();
    context.execution_plan =
        web_mode_positive_plan_or(&path, &target, context.execution_plan.clone());
    context.run = Some(AgentRunLedger {
        db_path: path.clone(),
        run_id: run,
    });
    context.external_surface = true;
    let db = db::open(&path).unwrap();
    // The existing low-level public driver fixture also initializes this
    // original scheduler projection before its first real child grant.
    db.execute("INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,lease_epoch,fencing_token)
        VALUES(?1,60000,20,?2,?3)",params![actor.root_run_id,actor.lease_epoch,actor.fencing_token]).unwrap();
    let (model_port, providers, model_stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(public_surface_model_text()),
        )
    }));
    context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
    let result = multi_agent_external_surface(&context, &actor).unwrap();
    assert_eq!(result["state"], "completed");
    assert_eq!(targets.lock().unwrap().len(), 1);
    assert_eq!(providers.lock().unwrap().len(), 1);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    model_stop.store(true, std::sync::atomic::Ordering::SeqCst);
    (root, context, actor, targets, providers)
}

#[test]
fn financial_history_public_actual_capture_and_paid_child_are_verified_readonly() {
    let _real = RealSpecialistTransport::enter();
    let (_root, context, actor, targets, providers) =
        financial_history_public_actual_fixture("finance-public-original");
    let db = db::open(&context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let before = web_mode_test_rows(&tx);
    let changes = tx.total_changes();
    let proof=crate::agent_runtime::multi_agent::budget::continuation::verify_settled_history_on(&tx,&actor.root_run_id)
        .expect("real public received capture and actual closed model fee must prove original financial history");
    assert_eq!(proof["rootRunId"], actor.root_run_id);
    assert_eq!(proof["executionAllowed"], false);
    for field in [
        "rootPhysicalHash",
        "ownerPhysicalHash",
        "financialPhysicalHash",
    ] {
        assert_eq!(proof[field].as_str().unwrap().len(), 64);
    }
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::continuation::verify_settled_history_on(
            &tx,
            &actor.root_run_id
        )
        .unwrap(),
        proof
    );
    assert_eq!(tx.total_changes(), changes);
    web_mode_assert_rows(&tx, &before);
    tx.rollback().unwrap();
    assert_eq!(targets.lock().unwrap().len(), 1);
    assert_eq!(providers.lock().unwrap().len(), 1);
}

#[test]
fn financial_history_public_missing_original_claim_cannot_hide_paid_target_cost() {
    let _real = RealSpecialistTransport::enter();
    let (_root, context, actor, targets, providers) =
        financial_history_public_actual_fixture("finance-public-claim-conflict");
    let db = db::open(&context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::budget::continuation::verify_settled_history_on(
        &tx,
        &actor.root_run_id,
    )
    .unwrap();
    let paid = crate::agent_runtime::multi_agent::budget::balance(
        &tx,
        &actor.root_run_id,
        None,
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (paid.reserved, paid.consumed, paid.indeterminate),
        (0, 1, 0)
    );
    // Remove only the disposable fixture's original capture; the immutable
    // paid entries and observed graph row remain. An aggregate cannot hide it.
    assert_eq!(
        tx.execute(
            "DELETE FROM agent_external_surface_captures WHERE child_run_id IN(
        SELECT id FROM agent_runs WHERE root_run_id=?1 AND role='external_surface')",
            [&actor.root_run_id]
        )
        .unwrap(),
        1
    );
    let before = web_mode_test_rows(&tx);
    let changes = tx.total_changes();
    // Full specialist original-receipt verification reaches child_received
    // first. The real target consume is still 1, while deleting its received
    // capture makes that existing reader reject before the later target audit.
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::continuation::verify_settled_history_on(
            &tx,
            &actor.root_run_id
        )
        .err(),
        Some("request_accounting_received_required".into())
    );
    assert_eq!(tx.total_changes(), changes);
    web_mode_assert_rows(&tx, &before);
    tx.rollback().unwrap();
    assert_eq!(targets.lock().unwrap().len(), 1);
    assert_eq!(providers.lock().unwrap().len(), 1);
}
