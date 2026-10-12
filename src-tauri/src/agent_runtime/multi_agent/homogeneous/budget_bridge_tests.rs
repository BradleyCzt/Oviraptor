//! Temporary-database regressions for original shared model accounting.
use super::test_fixture::{Fixture, TARGET};
use super::{apply, intent, machine, pool, reason, store, types, worker};
use crate::agent_runtime::model::gateway::ModelResponse;
use crate::agent_runtime::multi_agent::{attempts, budget};
use crate::agent_runtime::store::UsageDelta;
use rusqlite::params;
use serde_json::json;

const PATH: &str = "/api/v1/users";

fn original_fixture() -> Fixture {
    original_fixture_with_limits(1000, 10)
}

fn original_fixture_with_limits(tokens: i64, requests: i64) -> Fixture {
    let fixture = Fixture::with_capacity(tokens, requests, 1);
    fixture.connection.execute(
        "UPDATE agent_runs SET plan_json=?1 WHERE id=?2",
        params![json!({"timeoutSeconds":600,"budgets":{"hardModelRequests":requests},"scheduling_model":"homogeneous_workers"}).to_string(), fixture.root_run_id],
    ).unwrap();
    let tx = fixture.connection.unchecked_transaction().unwrap();
    budget::root::RootOwner::initialize_financial_fixture_for_test(&tx, &fixture.root_run_id)
        .expect("original financial fixture");
    tx.commit().unwrap();
    store::ensure_ledger(&fixture.connection, &fixture.lease).unwrap();
    fixture
}

fn pay_original_root(fixture: &Fixture, input: i64, output: i64, estimate: i64) {
    let tx = fixture.connection.unchecked_transaction().unwrap();
    let call = budget::root::model::RootModelCall::claim(
        &tx,
        &fixture.root_run_id,
        1,
        &"a".repeat(64),
        estimate,
    )
    .unwrap();
    tx.commit().unwrap();
    let response = ModelResponse {
        text: "original financial fixture".into(),
        tool_calls: Vec::new(),
        usage: UsageDelta {
            input_tokens: input,
            cached_input_tokens: 0,
            output_tokens: output,
            total_tokens: input + output,
            model_requests: 1,
        },
        usage_reported: true,
        cached_usage_reported: true,
        finish_reason: "stop".into(),
    };
    let tx = fixture.connection.unchecked_transaction().unwrap();
    assert!(!call.terminal(&tx, "received", Some(&response), "").unwrap());
    tx.commit().unwrap();
}

fn prepare(fixture: &Fixture) -> (String, String, String) {
    fixture.add_endpoint_evidence(PATH, 1);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", PATH)]))
        .applied());
    let worker = fixture.provision_workers(1).remove(0);
    let slice = fixture.slice_id("path_prefix", PATH);
    let assignment = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(&slice, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
    );
    (worker, slice, assignment)
}

fn assign(fixture: &Fixture, worker: &str, slice: &str, attempt: i64) {
    let outcome = fixture.run(&fixture.assign_text(
        slice,
        worker,
        200,
        1,
        &["evidence.read", "mailbox.write"],
        attempt,
    ));
    assert!(outcome.applied(), "{outcome:?}");
}

fn assert_attempt_balance(fixture: &Fixture, assignment: &str, attempt: &str, tokens: i64) {
    for dimension in budget::DIMENSIONS[..4].iter() {
        let expected = if *dimension == "model_requests" && tokens > 0 {
            1
        } else {
            tokens
        };
        let balance = budget::balance_for_attempt(
            &fixture.connection,
            &fixture.root_run_id,
            assignment,
            attempt,
            dimension,
        )
        .unwrap();
        assert_eq!(balance.reserved, expected);
        assert_eq!(balance.consumed, 0);
        assert_eq!(balance.indeterminate, 0);
    }
}

#[test]
fn original_budget_reclaim_and_retry_release_only_their_own_attempt() {
    let fixture = original_fixture();
    let (first_worker, slice, assignment) = prepare(&fixture);
    assign(&fixture, &first_worker, &slice, 1);
    let first = attempts::current(&fixture.connection, &fixture.lease, &assignment).unwrap();
    assert_attempt_balance(&fixture, &assignment, &first.id, 200);
    assert_eq!(
        budget::root::remaining_child_capacity(&fixture.connection, &fixture.root_run_id).unwrap(),
        (Some(800), Some(9))
    );
    fixture.connection.execute(
        "UPDATE agent_assignment_attempts SET expires_at=datetime('now','-1 second','localtime') WHERE id=?1",
        [&first.id],
    ).unwrap();
    pool::reclaim_expired_for_task(&fixture.connection, &fixture.task()).unwrap();
    assert_attempt_balance(&fixture, &assignment, &first.id, 0);
    assert_eq!(
        budget::root::remaining_child_capacity(&fixture.connection, &fixture.root_run_id).unwrap(),
        (Some(1000), Some(10))
    );
    let second_worker = pool::provision_workers(
        &fixture.connection,
        &fixture.task(),
        fixture.lease.lease_epoch,
        1,
    )
    .unwrap()
    .into_iter()
    .find(|worker| {
        pool::load_worker(&fixture.connection, worker)
            .unwrap()
            .unwrap()
            .state
            == "idle"
    })
    .unwrap();
    assert_ne!(first_worker, second_worker);
    assign(&fixture, &second_worker, &slice, 2);
    let second = attempts::current(&fixture.connection, &fixture.lease, &assignment).unwrap();
    assert_ne!(first.id, second.id);
    assert_attempt_balance(&fixture, &assignment, &first.id, 0);
    assert_attempt_balance(&fixture, &assignment, &second.id, 200);
    let namespaced: bool = fixture.connection.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM agent_budget_entries WHERE lease_attempt_id=?1 AND substr(idempotency_key,1,length(?2))<>?2)",
        params![second.id, format!("worker:{}:", second.id)], |row| row.get(0),
    ).unwrap();
    assert!(namespaced);
    worker::drain_for_lease(&fixture.connection, &fixture.lease).unwrap();
    let report_id = store::report_id_for(&assignment, 2);
    let report = store::load_report(&fixture.connection, &report_id)
        .unwrap()
        .unwrap();
    let stored = store::load_assignment(&fixture.connection, &assignment)
        .unwrap()
        .unwrap();
    let stored_slice = store::load_slice(&fixture.connection, &slice)
        .unwrap()
        .unwrap();
    let checks = machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &stored,
        &stored_slice,
        &report,
    );
    assert!(fixture
        .run(&fixture.reject_retry_text(
            &assignment,
            &slice,
            &report_id,
            &checks,
            Some(json!({"adequate":false,"question_zh":"证据够吗？","answer_zh":"需补证。"})),
            &[reason::MODEL_JUDGEMENT_FAIL],
        ))
        .applied());
    assert_attempt_balance(&fixture, &assignment, &second.id, 0);
    assert_eq!(
        budget::root::remaining_child_capacity(&fixture.connection, &fixture.root_run_id).unwrap(),
        (Some(1000), Some(10))
    );
}

#[test]
fn original_root_paid_gross_rejects_child_grant_atomically() {
    let fixture = original_fixture();
    let tx = fixture.connection.unchecked_transaction().unwrap();
    let call = budget::root::model::RootModelCall::claim(
        &tx,
        &fixture.root_run_id,
        1,
        &"a".repeat(64),
        700,
    )
    .unwrap();
    tx.commit().unwrap();
    let response = ModelResponse {
        text: "original financial fixture".into(),
        tool_calls: Vec::new(),
        usage: UsageDelta {
            input_tokens: 350,
            cached_input_tokens: 0,
            output_tokens: 350,
            total_tokens: 700,
            model_requests: 1,
        },
        usage_reported: true,
        cached_usage_reported: true,
        finish_reason: "stop".into(),
    };
    let tx = fixture.connection.unchecked_transaction().unwrap();
    assert!(!call.terminal(&tx, "received", Some(&response), "").unwrap());
    tx.commit().unwrap();
    let (worker, slice, assignment) = prepare(&fixture);
    assert_eq!(
        fixture.reason_of(&fixture.assign_text(&slice, &worker, 400, 1, &["evidence.read"], 1,)),
        reason::BUDGET_EXCEEDED
    );
    assert!(store::load_assignment(&fixture.connection, &assignment)
        .unwrap()
        .is_none());
    assert_eq!(
        pool::load_worker(&fixture.connection, &worker)
            .unwrap()
            .unwrap()
            .state,
        "idle"
    );
    assert_eq!(
        store::load_slice(&fixture.connection, &slice)
            .unwrap()
            .unwrap()
            .state,
        "pending"
    );
    assert_eq!(
        store::active_slot_count(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .unwrap(),
        0
    );
    assert_eq!(
        budget::root::remaining_child_capacity(&fixture.connection, &fixture.root_run_id).unwrap(),
        (Some(300), Some(9))
    );
}

#[test]
fn original_budget_refuses_nonzero_custom_report_without_token_classes() {
    let fixture = original_fixture();
    let (worker, slice, assignment) = prepare(&fixture);
    assign(&fixture, &worker, &slice, 1);
    let custom = "rpt-111122223333444455556666";
    // Exercise the reporting API's supported custom identity. No immutable
    // invoice is rewritten; this fake worker claims aggregate usage without
    // the original input/output model receipt that settlement requires.
    apply::mark_started(&fixture.connection, &assignment).unwrap();
    let report = types::WorkerReport {
        report_id: custom.into(),
        assignment_id: assignment.clone(),
        slice_id: slice.clone(),
        worker_id: worker.clone(),
        evidence_refs: vec![fixture.add_endpoint_evidence(PATH, 1)],
        output: json!({"summary_zh":"同范围只读证据","findings":[],"coverage_notes":[PATH]}),
        consumed_tokens: 1,
        consumed_requests: 1,
        terminal_hint: "completed".into(),
    };
    apply::record_worker_report(&fixture.connection, &fixture.lease, &report).unwrap();
    assert!(
        store::load_report(&fixture.connection, &store::report_id_for(&assignment, 1))
            .unwrap()
            .is_none()
    );
    let stored = store::load_assignment(&fixture.connection, &assignment)
        .unwrap()
        .unwrap();
    let stored_slice = store::load_slice(&fixture.connection, &slice)
        .unwrap()
        .unwrap();
    let checks = machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &stored,
        &stored_slice,
        &report,
    );
    let refs: Vec<&str> = report.evidence_refs.iter().map(String::as_str).collect();
    let outcome = fixture.run(&fixture.accept_text(&assignment, &slice, custom, &checks, &refs));
    assert_eq!(
        outcome.refused_reason(),
        Some(reason::BUDGET_EXCEEDED),
        "{outcome:?}"
    );
    let attempt = attempts::current(&fixture.connection, &fixture.lease, &assignment).unwrap();
    assert_attempt_balance(&fixture, &assignment, &attempt.id, 200);
    assert_eq!(
        store::load_assignment(&fixture.connection, &assignment)
            .unwrap()
            .unwrap()
            .state,
        "reported"
    );
    assert_eq!(
        budget::root::remaining_child_capacity(&fixture.connection, &fixture.root_run_id).unwrap(),
        (Some(800), Some(9))
    );
}

#[test]
fn original_budget_refuses_reclaim_when_child_usage_is_unknown() {
    let fixture = original_fixture();
    let (worker, slice, assignment) = prepare(&fixture);
    assign(&fixture, &worker, &slice, 1);
    let attempt = attempts::current(&fixture.connection, &fixture.lease, &assignment).unwrap();
    fixture
        .connection
        .execute(
            "UPDATE agent_runs SET used_tokens=1 WHERE id=?1",
            [&attempt.child_run_id],
        )
        .unwrap();
    let error =
        pool::reclaim_expired_worker(&fixture.connection, &fixture.task(), &worker, &assignment)
            .unwrap_err();
    assert_eq!(error.reason_code, reason::BUDGET_EXCEEDED);
    assert_eq!(
        error.detail,
        "budget_homogeneous_usage_requires_original_receipt"
    );
    assert_attempt_balance(&fixture, &assignment, &attempt.id, 200);
    assert_eq!(
        store::load_assignment(&fixture.connection, &assignment)
            .unwrap()
            .unwrap()
            .state,
        "assigned"
    );
    assert_eq!(
        budget::root::remaining_child_capacity(&fixture.connection, &fixture.root_run_id).unwrap(),
        (Some(800), Some(9))
    );
}

#[test]
fn original_root_last_paid_request_allows_stop_despite_unused_coarse_child_budget() {
    let fixture = original_fixture_with_limits(1000, 1);
    pay_original_root(&fixture, 10, 10, 100);
    assert_eq!(
        store::remaining_budget(&fixture.connection, &fixture.root_run_id).unwrap(),
        (1000, 1)
    );
    assert_eq!(
        budget::root::remaining_child_capacity(&fixture.connection, &fixture.root_run_id).unwrap(),
        (Some(980), Some(0))
    );
    assert!(intent::budget_exhausted(&fixture.connection, &fixture.root_run_id).unwrap());
    let outcome = fixture.run(&fixture.stop_text("budget_exhausted", &[], &[], false));
    assert!(outcome.applied(), "{outcome:?}");
    assert_eq!(fixture.task().state, "stopped");
    assert_eq!(
        intent::latest_stop_reason(&fixture.connection, &fixture.task().task_id)
            .unwrap()
            .as_deref(),
        Some("budget_exhausted")
    );
    assert_eq!(
        budget::balance(
            &fixture.connection,
            &fixture.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
}

#[test]
fn original_unlimited_and_positive_shared_capacity_refuse_false_budget_stop() {
    for (tokens, requests, expected) in [(0, 0, (None, None)), (1000, 10, (Some(1000), Some(10)))] {
        let fixture = original_fixture_with_limits(tokens, requests);
        let (_, slice, _) = prepare(&fixture);
        assert_eq!(
            budget::root::remaining_child_capacity(&fixture.connection, &fixture.root_run_id)
                .unwrap(),
            expected
        );
        assert!(!intent::budget_exhausted(&fixture.connection, &fixture.root_run_id).unwrap());
        let outcome = fixture.run(&fixture.stop_text("budget_exhausted", &[], &[&slice], false));
        assert_eq!(
            outcome.refused_reason(),
            Some(reason::ZERO_INTENT_ILLEGAL),
            "{outcome:?}"
        );
        assert_ne!(fixture.task().state, "stopped");
        assert!(
            intent::latest_stop_reason(&fixture.connection, &fixture.task().task_id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store::load_slice(&fixture.connection, &slice)
                .unwrap()
                .unwrap()
                .state,
            "pending"
        );
    }
}

#[test]
fn original_shared_projection_conflict_is_not_budget_exhaustion() {
    let fixture = original_fixture_with_limits(1000, 1);
    pay_original_root(&fixture, 10, 10, 100);
    let (_, slice, _) = prepare(&fixture);
    // A temporary-database fault changes the coarse child projection alone;
    // the original Root invoice and its exact financial receipts stay intact.
    fixture
        .connection
        .execute(
            "UPDATE agent_budget_ledger SET reserved_requests=1 WHERE root_run_id=?1",
            [&fixture.root_run_id],
        )
        .unwrap();
    let error = intent::budget_exhausted(&fixture.connection, &fixture.root_run_id).unwrap_err();
    assert_eq!(error.reason_code, reason::BUDGET_EXCEEDED);
    assert_eq!(error.detail, "budget_projection_balance_conflict");
    let outcome = fixture.run(&fixture.stop_text("budget_exhausted", &[], &[&slice], false));
    assert!(
        matches!(outcome, apply::TurnOutcome::Refused {reason_code:reason::BUDGET_EXCEEDED, ref detail, ..} if detail == "budget_projection_balance_conflict"),
        "{outcome:?}"
    );
    assert_ne!(fixture.task().state, "stopped");
    assert!(
        intent::latest_stop_reason(&fixture.connection, &fixture.task().task_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        budget::balance(
            &fixture.connection,
            &fixture.root_run_id,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
}
