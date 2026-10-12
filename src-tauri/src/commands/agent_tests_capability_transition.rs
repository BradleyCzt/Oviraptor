// Actual v10 creator, original Web SDK/checkpoint, and production final delivery.
fn capability_transition_actual_fixture(
    unknown: bool,
    fail_transition: bool,
) -> (BudgetTriggerFixture, AgentTargetOutcome) {
    let mut h = fresh_multi_production_harness("capability-transition-v10");
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        if fail_transition
            && wire.contains("You are the Root Coordinator")
            && wire.contains("capability-changed")
        {
            return (
                503,
                "application/json",
                json!({"error":{"message":"actual transition provider unavailable"}}).to_string(),
            );
        }
        let response = if wire.contains("You are the Root Coordinator")
            && wire.contains("capability-changed")
        {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original closed capability transition"],"missing":[],"suggestions":["assess:capability_changed"],"costNotes":[],"risks":[]}).to_string())
        } else if wire.contains("You are the Root Coordinator")
            && wire.contains("budget-consumption")
        {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original paid Web source"],"missing":[],"suggestions":["assess:budget_consumption"],"costNotes":[],"risks":[]}).to_string())
        } else if let Some(root) = fresh_multi_root_wire_response(&wire) {
            root
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"actual closed original Mapper","priorityContracts":[],"risks":[]}"#,
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
    freeze_fresh_multi_production_harness_capability_v10(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let f = BudgetTriggerFixture {
        h,
        session: Some(session),
        boundary: std::sync::Arc::new(std::sync::Mutex::new(None)),
        stop,
    };
    let diagnostic_db = db::open(&f.h.db_path).unwrap();
    capability_transition_assert_pending(
        &diagnostic_db,
        &f.session.as_ref().unwrap().lease.root_run_id,
        1,
        false,
    );
    drop(diagnostic_db);
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
        let checkpoint =
            NativeAgentState::read(&f.h.db_path, &f.h.context.scan_id, &f.h.context.target_url)
                .unwrap();
        assert_eq!(checkpoint.turns, 1);
        assert_eq!(checkpoint.token_usage.total_tokens, 50);
    }
    (f, outcome)
}

fn capability_transition_assert_pending(
    db: &rusqlite::Connection,
    root: &str,
    expected: i64,
    unknown: bool,
) {
    let before = super::tests::application_table_snapshot(db);
    let tx = db.unchecked_transaction().unwrap();
    let report =
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, root)
            .unwrap()
            .expect("v10 original request diagnostic");
    let value = serde_json::to_value(report).unwrap();
    assert_eq!(value["pendingCapabilityTransitions"], expected);
    assert_eq!(value["executionAllowed"], false);
    assert_eq!(value["futureTokenCapacityGuaranteed"], false);
    if unknown {
        assert_eq!(value["freshWorkBlocked"], "cost_reconciliation_required");
    }
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(db), before);
}

fn capability_transition_published_count(db: &rusqlite::Connection, root: &str) -> i64 {
    db.query_row("SELECT count(*) FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='capability-changed'",[root],|r|r.get(0)).unwrap()
}

fn capability_transition_assert_original_receipt(
    db: &rusqlite::Connection,
    session: &MultiAgentSession,
) {
    let before = super::tests::application_table_snapshot(db);
    let tx = db.unchecked_transaction().unwrap();
    let source = crate::agent_runtime::web_mode::capability_transition_receipt::read_original_on(
        &tx,
        &session.lease,
        &session.executor,
    )
    .unwrap()
    .expect("production final must issue a signed original transition");
    assert!(!source.receipt_id().is_empty());
    assert!(!source.message_id().is_empty());
    let fact = source.fact();
    assert_eq!(fact["cause"], "normal_completed_execution_result");
    assert_eq!(fact["rootRunId"], session.lease.root_run_id);
    assert_eq!(fact["childRunId"], session.executor.run_id);
    assert_eq!(fact["executionAllowed"], false);
    assert_eq!(fact["recoveryImplemented"], false);
    let before_caps = fact["beforeCapabilities"].as_array().unwrap();
    let after_caps = fact["afterCapabilities"].as_array().unwrap();
    assert!(!before_caps.is_empty());
    assert_eq!(before_caps.len(), after_caps.len());
    assert!(before_caps.iter().all(|v| v["revokedAt"] == ""));
    assert!(after_caps
        .iter()
        .all(|v| v["revokedAt"].as_str().is_some_and(|s| !s.is_empty())));
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(db), before);
}

#[test]
fn capability_transition_actual_finish_assesses_original_closed_transition_once() {
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = capability_transition_actual_fixture(false, false);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let assignment = f.session.as_ref().unwrap().executor.assignment_id.clone();
    let paid = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'web-model:%' ORDER BY rowid",[&root]).unwrap();
    assert_eq!(capability_transition_published_count(&db, &root), 0);
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(capability_transition_published_count(&db, &root), 1);
    capability_transition_assert_original_receipt(&db, f.session.as_ref().unwrap());
    capability_transition_assert_pending(&db, &root, 0, false);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result'",[&assignment],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'web-model:%' ORDER BY rowid",[&root]).unwrap()==paid);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn capability_transition_actual_finish_replays_without_new_sdk_or_writes() {
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = capability_transition_actual_fixture(false, false);
    let db = db::open(&f.h.db_path).unwrap();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    let session = f.session.as_ref().unwrap();
    assert_eq!(
        capability_transition_published_count(&db, &session.lease.root_run_id),
        1
    );
    capability_transition_assert_original_receipt(&db, session);
    capability_transition_assert_pending(&db, &session.lease.root_run_id, 0, false);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn capability_transition_actual_unknown_web_fee_never_issues_or_pays_root() {
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = capability_transition_actual_fixture(true, false);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).is_err()
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_web_capability_transition_receipts WHERE root_run_id=?1",
            [&root],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(capability_transition_published_count(&db, &root), 0);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn capability_transition_actual_root_503_keeps_closed_source_and_blocks_fresh_retry() {
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = capability_transition_actual_fixture(false, true);
    let db = db::open(&f.h.db_path).unwrap();
    let root = f.session.as_ref().unwrap().lease.root_run_id.clone();
    let paid=crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'web-model:%' ORDER BY rowid",[&root]).unwrap();
    assert!(
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).is_err()
    );
    capability_transition_assert_original_receipt(&db, f.session.as_ref().unwrap());
    assert_eq!(capability_transition_published_count(&db, &root), 0);
    capability_transition_assert_pending(&db, &root, 1, true);
    assert!(f
        .h
        .model_seen
        .lock()
        .unwrap()
        .iter()
        .any(|wire| wire.contains("capability-changed")));
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,"SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id LIKE 'web-model:%' ORDER BY rowid",[&root]).unwrap()==paid);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).is_err()
    );
    assert_eq!(
        f.h.model_seen.lock().unwrap().len(),
        sdk,
        "an unresolved original failed Root bill cannot buy a fresh ticket"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn capability_transition_original_signature_scope_and_withdrawn_coordinator_are_closed() {
    use crate::agent_runtime::web_mode::capability_transition_receipt;
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = capability_transition_actual_fixture(false, false);
    let db = db::open(&f.h.db_path).unwrap();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    let session = f.session.as_ref().unwrap();
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let trigger:String=tx.query_row("SELECT sql FROM sqlite_master WHERE type='trigger' AND name='capability_transition_no_update'",[],|r|r.get(0)).unwrap();
    tx.execute_batch("DROP TRIGGER capability_transition_no_update")
        .unwrap();
    assert_eq!(tx.execute("UPDATE agent_web_capability_transition_receipts SET signature=zeroblob(32) WHERE root_run_id=?1",[&session.lease.root_run_id]).unwrap(),1);
    tx.execute_batch(&trigger).unwrap();
    let damaged = super::tests::application_table_snapshot(&tx);
    let error =
        capability_transition_receipt::read_original_on(&tx, &session.lease, &session.executor)
            .err()
            .expect("cold signature must fail");
    assert_eq!(error, "capability_transition_signature_invalid");
    assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    tx.rollback().unwrap();
    let mut foreign = session.lease.clone();
    foreign.root_run_id = Uuid::new_v4().to_string();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        capability_transition_receipt::read_original_on(&tx, &foreign, &session.executor).is_err()
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1 WHERE root_run_id=?2",params![Uuid::new_v4().to_string(),session.lease.root_run_id]).unwrap(),1);
    capability_transition_assert_original_receipt(&db, session);
    let withdrawn = super::tests::application_table_snapshot(&db);
    native_coordinator_capability_transition_feedback(
        &f.h.context,
        &session.lease,
        &session.executor,
    )
    .expect("an already paid publication remains a read-only historical replay after C withdrawal");
    assert_eq!(super::tests::application_table_snapshot(&db), withdrawn);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn capability_transition_withdrawn_before_first_paid_assessment_cannot_dispatch_root() {
    let _real = RealSpecialistTransport::enter();
    let (f, outcome) = capability_transition_actual_fixture(false, false);
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    // The real original SDK outcome is financially settled by the same producer
    // used by finish_execution; no caller supplies fees or a completed outcome.
    let tx = db.unchecked_transaction().unwrap();
    let (usage, idle) =
        settle_original_web_child_usage(&tx, &session.lease, &session.executor).unwrap();
    tx.commit().unwrap();
    drop(idle);
    let payload = json!({"terminalCode":outcome.terminal_code(),"summary":outcome.detail(),"usage":{"totalTokens":usage.total_tokens,"modelRequests":usage.model_requests}});
    let success = matches!(
        outcome,
        AgentTargetOutcome::Completed(_)
            | AgentTargetOutcome::BoundedCompleted(_)
            | AgentTargetOutcome::Incomplete(_)
            | AgentTargetOutcome::Limited(_)
    );
    assert!(
        success,
        "the actual production outcome must meet the original delivery condition"
    );
    let guard = complete_target_child_delivery(
        &db,
        &session.lease,
        &session.executor,
        "execution_result",
        &format!("executor-result:{}", session.executor.assignment_id),
        &payload,
        success,
        &outcome.detail(),
    )
    .unwrap();
    drop(guard);
    capability_transition_assert_pending(&db, &session.lease.root_run_id, 1, false);
    capability_transition_assert_original_receipt(&db, session);
    assert_eq!(
        capability_transition_published_count(&db, &session.lease.root_run_id),
        0
    );
    assert_eq!(db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1 WHERE root_run_id=?2",params![Uuid::new_v4().to_string(),session.lease.root_run_id]).unwrap(),1);
    capability_transition_assert_original_receipt(&db, session);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    assert!(native_coordinator_capability_transition_feedback(
        &f.h.context,
        &session.lease,
        &session.executor
    )
    .is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn capability_transition_actual_issued_worker_retains_request_capacity_boundary() {
    use crate::agent_runtime::multi_agent::budget::supervision_capacity;
    let _real = RealSpecialistTransport::enter();
    let (f, _outcome) = capability_transition_actual_fixture(false, false);
    let db = db::open(&f.h.db_path).unwrap();
    let session = f.session.as_ref().unwrap();
    capability_transition_assert_pending(&db, &session.lease.root_run_id, 1, false);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    // Original v10: six follow-up requests + one allocation + the outstanding
    // worker assessment + the proposed new assessment + two-call minimum.
    assert_eq!(
        supervision_capacity::executor_requests_on(&tx, &session.lease, Some(10)).unwrap_err(),
        "multi_agent_executor_budget_unavailable"
    );
    assert_eq!(
        supervision_capacity::executor_requests_on(&tx, &session.lease, Some(11)).unwrap(),
        Some(1)
    );
    // The historical v9 pure formula keeps its original nine-request boundary.
    assert_eq!(
        supervision_capacity::executor_requests(Some(9)).unwrap(),
        Some(1)
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn capability_transition_actual_paid_source_rejects_foreign_actor_and_worker_readonly() {
    use crate::agent_runtime::{
        multi_agent::budget::supervision_capacity, web_mode::capability_transition_receipt,
    };
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = capability_transition_actual_fixture(false, false);
    let db = db::open(&f.h.db_path).unwrap();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    let session = f.session.as_ref().unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let mut foreign_root = session.lease.clone();
    foreign_root.root_run_id = Uuid::new_v4().to_string();
    assert!(supervision_capacity::executor_requests_on(&tx, &foreign_root, Some(11)).is_err());
    let mut foreign_scope = session.lease.clone();
    foreign_scope.target_key = format!("{}-foreign", foreign_scope.target_key);
    assert!(supervision_capacity::executor_requests_on(&tx, &foreign_scope, Some(11)).is_err());
    let mut foreign_worker = session.executor.clone();
    foreign_worker.run_id = Uuid::new_v4().to_string();
    assert!(
        capability_transition_receipt::read_original_on(&tx, &session.lease, &foreign_worker)
            .is_err()
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn capability_transition_missing_original_paid_publication_never_discharges_pending() {
    let _real = RealSpecialistTransport::enter();
    let (mut f, outcome) = capability_transition_actual_fixture(false, false);
    let db = db::open(&f.h.db_path).unwrap();
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    capability_transition_assert_pending(&db, root, 0, false);
    let before = super::tests::application_table_snapshot(&db);
    let sdk = f.h.model_seen.lock().unwrap().len();
    let tx = db.unchecked_transaction().unwrap();
    let call:String=tx.query_row("SELECT q.call_id FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request' AND json_extract(q.fact_json,'$.request.basis.phase')='capability-changed'",[root],|r|r.get(0)).unwrap();
    let trigger: String = tx
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='root_tick_no_delete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    tx.execute_batch("DROP TRIGGER root_tick_no_delete")
        .unwrap();
    assert_eq!(
        tx.execute(
            "DELETE FROM agent_root_tick_receipts WHERE call_id=?1 AND phase='publication'",
            [&call]
        )
        .unwrap(),
        1
    );
    tx.execute_batch(&trigger).unwrap();
    let damaged = super::tests::application_table_snapshot(&tx);
    let report =
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, root)
            .unwrap()
            .unwrap();
    assert_eq!(
        serde_json::to_value(report).unwrap()["pendingCapabilityTransitions"],
        1
    );
    assert_eq!(super::tests::application_table_snapshot(&tx), damaged);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}
