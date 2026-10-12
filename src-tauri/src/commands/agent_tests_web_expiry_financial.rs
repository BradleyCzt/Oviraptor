// Real 600-second issued worker deadline, parent-only heartbeat and signed sweep.
// This suite is deliberately long-running: no cold TTL or elapsed-stat rewrite.
fn web_expiry_financial_natural_fixture(
    unknown: bool,
    wait_for_expiry: bool,
) -> (BudgetTriggerFixture, AgentTargetOutcome) {
    web_expiry_financial_natural_fixture_version(unknown, wait_for_expiry, false)
}

fn web_expiry_financial_natural_fixture_version(
    unknown: bool,
    wait_for_expiry: bool,
    capability_v10: bool,
) -> (BudgetTriggerFixture, AgentTargetOutcome) {
    let mut h = fresh_multi_production_harness("web-expiry-natural-v9");
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let response = if wire.contains("You are the Root Coordinator")
            && wire.contains("lease-expired")
        {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original expired Web financial delivery"],"missing":[],"suggestions":["assess:lease_expired"],"costNotes":[],"risks":[]}).to_string())
        } else if let Some(root) = fresh_multi_root_wire_response(&wire) {
            root
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"original closed Mapper","priorityContracts":[],"risks":[]}"#,
            )
        } else {
            let mut response: JsonValue = serde_json::from_str(&model_round(&[], 10)).unwrap();
            if unknown {
                response.as_object_mut().unwrap().remove("usage");
            }
            response.to_string()
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = 20;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = 1;
    // A legitimate original twenty-minute task, frozen before its first Root,
    // allows the issued ten-minute worker deadline to expire under a live C.
    h.context.execution_plan.timeout_seconds = 1200;
    if capability_v10 {
        freeze_fresh_multi_production_harness_capability_v10(&mut h);
    } else {
        freeze_fresh_multi_production_harness_web_expiry_v9(&mut h);
    }
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let f = BudgetTriggerFixture {
        h,
        session: Some(session),
        boundary: std::sync::Arc::new(std::sync::Mutex::new(None)),
        stop,
    };
    let outcome = run_native_agent(&f.h.context);
    if unknown {
        assert_eq!(
            outcome.terminal_code(),
            terminal_code::REQUEST_RECONCILIATION_REQUIRED
        );
    } else {
        assert!(
            matches!(
                outcome,
                AgentTargetOutcome::Incomplete(_) | AgentTargetOutcome::BoundedCompleted(_)
            ),
            "{}",
            outcome.detail()
        );
    }
    if !wait_for_expiry {
        return (f, outcome);
    }
    // ExecutorLeaseHeartbeat has now dropped and joined. The parent remains
    // alive, renews only C, and sweeps the unchanged original worker deadline.
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    let worker = crate::agent_runtime::multi_agent::attempts::current(
        &db,
        &session.lease,
        &session.executor.assignment_id,
    )
    .unwrap();
    let original_expiry = worker.expires_at.clone();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let guard_deadline = Instant::now() + Duration::from_secs(630);
    loop {
        let current = crate::agent_runtime::multi_agent::attempts::current(
            &db,
            &session.lease,
            &session.executor.assignment_id,
        )
        .unwrap();
        assert_eq!(
            current.expires_at, original_expiry,
            "the parent must never renew the exited worker"
        );
        if current.state == "expired" {
            break;
        }
        assert!(
            Instant::now() < guard_deadline,
            "the real signed sweep did not close the issued worker deadline"
        );
        session.supervisor.check().unwrap();
        std::thread::sleep(Duration::from_secs(1));
    }
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&db, &session.lease)
        .unwrap();
    crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &session.lease.root_run_id)
        .unwrap();
    let original_snapshot = db.unchecked_transaction().unwrap();
    crate::agent_runtime::multi_agent::attempts::withdrawal_receipt::verify_original(
        &original_snapshot,
        &worker.id,
    )
    .unwrap();
    original_snapshot.rollback().unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    drop(db);
    (f, outcome)
}

#[test]
fn web_expiry_financial_natural_paid_receipt_closes_fees_without_business_dispatch() {
    use crate::agent_runtime::{
        multi_agent::budget::model::expired_web_receipt,
        web_mode::withdrawal_issuer::WithdrawalIssuer,
    };
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = web_expiry_financial_natural_fixture(false, true);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let assignment = f.session.as_ref().unwrap().executor.assignment_id.clone();
    let sdk_before_finish = f.h.model_seen.lock().unwrap().len();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk_before_finish + 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='lease-expired'", [&root], |r| r.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_expiry'", [&assignment], |r| r.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'", [&assignment], |r| r.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(
        db.query_row(
            "SELECT status FROM agent_runs WHERE id=?1",
            [&f.session.as_ref().unwrap().executor.run_id],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "paused"
    );
    let published = super::tests::application_table_snapshot(&db);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), published);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk_before_finish + 1);
    let session = f.session.as_ref().unwrap();
    let birth_snapshot = db.unchecked_transaction().unwrap();
    let issuer = WithdrawalIssuer::from_verified_birth_on(
        &birth_snapshot,
        &session.lease.root_run_id,
        original_root_capture_mode(&birth_snapshot, &session.lease.root_run_id).unwrap(),
    )
    .unwrap();
    birth_snapshot.rollback().unwrap();
    let calls = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
        &db,
        "SELECT rowid,* FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid",
        [&session.lease.root_run_id],
    )
    .unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let proof = expired_web_receipt::settle_expired_web_receipt_on(
        &tx,
        &session.lease,
        &session.executor,
        &issuer,
    )
    .unwrap();
    assert_eq!(proof.usage().total_tokens, 50);
    assert_eq!(proof.usage().model_requests, 1);
    assert_eq!(proof.fact()["executionAllowed"], false);
    assert_eq!(proof.fact()["recoveryImplemented"], false);
    assert!(!proof.expiry_receipt_id().is_empty());
    assert!(proof.http_receipt_ids().is_empty());
    proof
        .require_on(&tx, &session.lease, &session.executor)
        .unwrap();
    tx.commit().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let snapshot = db.unchecked_transaction().unwrap();
    proof
        .require_on(&snapshot, &session.lease, &session.executor)
        .unwrap();
    snapshot.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert!(
        crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            "SELECT rowid,* FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid",
            [&session.lease.root_run_id]
        )
        .unwrap()
            == calls
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    drop(proof);
    assert_eq!(db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1 WHERE root_run_id=?2",
        params![Uuid::new_v4().to_string(), session.lease.root_run_id]).unwrap(), 1);
    let withdrawn = super::tests::application_table_snapshot(&db);
    assert!(native_coordinator_lease_expiry_feedback(
        &f.h.context,
        &session.lease,
        &session.executor
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), withdrawn);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn web_expiry_financial_natural_unknown_receipt_never_closes_or_dispatches() {
    use crate::agent_runtime::{
        multi_agent::budget::model::expired_web_receipt,
        web_mode::withdrawal_issuer::WithdrawalIssuer,
    };
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = web_expiry_financial_natural_fixture(true, true);
    let db = db::open(&f.h.db_path).unwrap();
    let sdk_before_finish = f.h.model_seen.lock().unwrap().len();
    assert!(
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).is_err()
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk_before_finish);
    let assignment = &f.session.as_ref().unwrap().executor.assignment_id;
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind IN ('execution_expiry','execution_result')", [assignment], |r| r.get::<_,i64>(0)).unwrap(), 0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request' AND json_extract(fact_json,'$.request.basis.phase')='lease-expired'", [&f.session.as_ref().unwrap().lease.root_run_id], |r| r.get::<_,i64>(0)).unwrap(), 0);
    let session = f.session.as_ref().unwrap();
    let birth_snapshot = db.unchecked_transaction().unwrap();
    let issuer = WithdrawalIssuer::from_verified_birth_on(
        &birth_snapshot,
        &session.lease.root_run_id,
        original_root_capture_mode(&birth_snapshot, &session.lease.root_run_id).unwrap(),
    )
    .unwrap();
    birth_snapshot.rollback().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    assert!(expired_web_receipt::settle_expired_web_receipt_on(
        &tx,
        &session.lease,
        &session.executor,
        &issuer
    )
    .is_err());
    // A caught failure cannot leak a partial financial settlement on commit.
    tx.commit().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn web_expiry_financial_actual_worker_clock_is_paid_readonly_and_tamper_closed() {
    use crate::agent_runtime::multi_agent::budget::{self, clock::original_worker_samples};
    let _real = RealSpecialistTransport::enter();
    let (f, _) = web_expiry_financial_natural_fixture(false, false);
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    let worker = crate::agent_runtime::multi_agent::attempts::current(
        &db,
        &session.lease,
        &session.executor.assignment_id,
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let paid = budget::balance(
        &tx,
        &session.lease.root_run_id,
        Some(&session.executor.assignment_id),
        "wall_time_ms",
    )
    .unwrap();
    assert!(
        paid.consumed > 0,
        "actual SDK admission must retain its legitimate original clock charge"
    );
    assert_eq!((paid.reserved, paid.indeterminate), (0, 0));
    assert_eq!(
        original_worker_samples::original_worker_consumed_on(
            &tx,
            &session.lease,
            &session.executor,
            &worker.id
        )
        .unwrap(),
        paid.consumed
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    for column in ["source_id", "amount"] {
        let tx = db.unchecked_transaction().unwrap();
        let rowid:i64 = tx.query_row("SELECT rowid FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='wall_time_ms' AND kind='consume' ORDER BY rowid LIMIT 1", params![session.lease.root_run_id,session.executor.assignment_id,worker.id], |r|r.get(0)).unwrap();
        let trigger:String = tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='budget_entry_no_update'", [], |r|r.get(0)).unwrap();
        tx.execute_batch("DROP TRIGGER budget_entry_no_update")
            .unwrap();
        let statement = if column == "source_id" {
            "UPDATE agent_budget_entries SET source_id=source_id||':cold' WHERE rowid=?1"
        } else {
            "UPDATE agent_budget_entries SET amount=amount+1 WHERE rowid=?1"
        };
        assert_eq!(tx.execute(statement, [rowid]).unwrap(), 1);
        tx.execute_batch(&trigger).unwrap();
        let altered = super::tests::application_table_snapshot(&tx);
        assert_eq!(
            original_worker_samples::original_worker_consumed_on(
                &tx,
                &session.lease,
                &session.executor,
                &worker.id
            )
            .unwrap_err(),
            "budget_original_clock_pair_changed"
        );
        assert_eq!(super::tests::application_table_snapshot(&tx), altered);
        tx.rollback().unwrap();
        assert_eq!(super::tests::application_table_snapshot(&db), before);
        assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    }
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn web_expiry_financial_v10_natural_paid_drains_pending_after_original_assessment() {
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = web_expiry_financial_natural_fixture_version(false, true, true);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let assignment = f.session.as_ref().unwrap().executor.assignment_id.clone();
    capability_transition_assert_pending(&db, &root, 1, false);
    // The original financial/delivery producers can close their own transaction
    // without buying a Root assessment; this alone must retain the obligation.
    let session = f.session.as_ref().unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let issuer=crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer::from_verified_birth_on(&tx,&root,original_root_capture_mode(&tx,&root).unwrap()).unwrap();
    let financial=crate::agent_runtime::multi_agent::budget::model::expired_web_receipt::settle_expired_web_receipt_on(&tx,&session.lease,&session.executor,&issuer).unwrap();
    crate::agent_runtime::web_mode::web_expiry_delivery::issue_on(
        &tx,
        &session.lease,
        &session.executor,
        &issuer,
        &financial,
    )
    .unwrap();
    tx.commit().unwrap();
    drop(financial);
    capability_transition_assert_pending(&db, &root, 1, false);
    let sdk = f.h.model_seen.lock().unwrap().len();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 1);
    capability_transition_assert_pending(&db, &root, 0, false);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_expiry'",[&assignment],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",[&assignment],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_web_capability_transition_receipts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "expiry cause must never become a normal capability transition seal"
    );
    let before = super::tests::application_table_snapshot(&db);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk + 1);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn web_expiry_financial_v10_natural_unknown_keeps_pending_and_blocks_fresh_work() {
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = web_expiry_financial_natural_fixture_version(true, true, true);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let assignment = f.session.as_ref().unwrap().executor.assignment_id.clone();
    capability_transition_assert_pending(&db, &root, 1, true);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).is_err()
    );
    capability_transition_assert_pending(&db, &root, 1, true);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind IN ('execution_expiry','execution_result')",[&assignment],|r|r.get::<_,i64>(0)).unwrap(),0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_web_capability_transition_receipts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}
