//! PR-D：机器可检重算与验收环（DESIGN §3.2、§10.2 GOLD-08..12）。

use super::test_fixture::Fixture;
use super::{machine, pool, reason, store, types};
use serde_json::json;

#[test]
fn gold08_green_machine_checks_allow_accept() {
    let fixture = Fixture::new();
    let evidence = fixture.add_endpoint_evidence("/api/v1/users", 1);
    let (assignment_id, report_id, slice_id) =
        fixture.reported_assignment("/api/v1/users", &[&evidence]);
    let checks = machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &store::load_assignment(&fixture.connection, &assignment_id)
            .expect("a")
            .expect("有"),
        &store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有"),
        &store::load_report(&fixture.connection, &report_id)
            .expect("r")
            .expect("有"),
    );
    assert!(checks.all_true(), "{checks:?}");
    assert!(fixture
        .run(&fixture.accept_text(&assignment_id, &slice_id, &report_id, &checks, &[&evidence]))
        .applied());
    assert_eq!(
        store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有")
            .state,
        "accepted"
    );
    assert_eq!(
        store::load_assignment(&fixture.connection, &assignment_id)
            .expect("a")
            .expect("有")
            .state,
        "accepted"
    );
    assert_eq!(
        pool::load_worker(&fixture.connection, &fixture.provision_workers(1)[0])
            .expect("worker")
            .expect("存在")
            .state,
        "idle"
    );
    let verdicts: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM agent_acceptance_verdicts WHERE verdict='accept'",
            [],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(verdicts, 1);
    // verdict 行不可改（append-only）
    let failure = fixture
        .connection
        .execute("UPDATE agent_acceptance_verdicts SET verdict='reject'", []);
    assert!(failure.is_err(), "验收结论必须不可变");
}

#[test]
fn gold09_lying_machine_checks_are_overridden_by_the_rust_recompute() {
    let fixture = Fixture::new();
    // 汇报里没有任何证据引用：Rust 重算 evidence_refs_exist=false。
    let (assignment_id, report_id, slice_id) = fixture.reported_assignment("/api/v1/users", &[]);
    let lying = super::test_fixture::checks_all_true();
    assert_eq!(
        fixture.reason_of(&fixture.accept_text(&assignment_id, &slice_id, &report_id, &lying, &[])),
        reason::MACHINE_CHECK_FAILED
    );
    let accepted: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM agent_acceptance_verdicts WHERE verdict='accept'",
            [],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(accepted, 0, "谎报不得产生 accept 结论");
    assert_eq!(
        store::load_assignment(&fixture.connection, &assignment_id)
            .expect("a")
            .expect("有")
            .state,
        "reported"
    );
}

#[test]
fn gold10_missing_evidence_leads_to_retry_with_evidence_missing() {
    let fixture = Fixture::new();
    let (assignment_id, report_id, slice_id) = fixture.reported_assignment("/api/v1/users", &[]);
    let checks = machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &store::load_assignment(&fixture.connection, &assignment_id)
            .expect("a")
            .expect("有"),
        &store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有"),
        &store::load_report(&fixture.connection, &report_id)
            .expect("r")
            .expect("有"),
    );
    assert!(!checks.evidence_refs_exist);
    assert!(fixture
        .run(&fixture.reject_retry_text(
            &assignment_id,
            &slice_id,
            &report_id,
            &checks,
            None,
            &[reason::EVIDENCE_MISSING]
        ))
        .applied());
    let slice = store::load_slice(&fixture.connection, &slice_id)
        .expect("s")
        .expect("有");
    assert_eq!(slice.state, "rejected");
    assert_eq!(slice.retry_count, 1);
    assert_eq!(
        store::load_assignment(&fixture.connection, &assignment_id)
            .expect("a")
            .expect("有")
            .state,
        "rejected_retry"
    );
    assert_eq!(
        store::active_slot_count(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .expect("占用"),
        0
    );
}

#[test]
fn gold11_retry_beyond_max_retries_is_refused() {
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(1);
    assert!(fixture
        .run(&fixture.decompose_text_with_retries(&[("path_prefix", "/api/v1/users")], 0))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert!(fixture
        .run(&fixture.assign_text(&slice_id, &workers[0], 4000, 2, &["evidence.read"], 1))
        .applied());
    let assignment_id = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(
            &slice_id,
            super::test_fixture::TARGET,
            1,
            store::HOMOGENEOUS_TRIGGER_CODE,
        ),
    );
    super::apply::mark_started(&fixture.connection, &assignment_id).expect("开跑");
    let report = types::WorkerReport {
        report_id: store::report_id_for(&assignment_id, 1),
        assignment_id: assignment_id.clone(),
        slice_id: slice_id.clone(),
        worker_id: workers[0].clone(),
        evidence_refs: vec![],
        output: json!({"summary_zh":"无证据的结论","findings":[],"coverage_notes":["GET /api/v1/users"]}),
        consumed_tokens: 500,
        consumed_requests: 1,
        terminal_hint: "completed".to_string(),
    };
    super::apply::record_worker_report(&fixture.connection, &fixture.lease, &report).expect("汇报");
    let checks = machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &store::load_assignment(&fixture.connection, &assignment_id)
            .expect("a")
            .expect("有"),
        &store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有"),
        &report,
    );
    assert_eq!(
        fixture.reason_of(&fixture.reject_retry_text(
            &assignment_id,
            &slice_id,
            &report.report_id,
            &checks,
            None,
            &[reason::EVIDENCE_MISSING]
        )),
        reason::RETRY_LIMIT_EXCEEDED
    );
}

#[test]
fn gold12_report_for_another_slice_is_cross_slice_write() {
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(1);
    assert!(fixture
        .run(&fixture.decompose_text(&[
            ("path_prefix", "/api/v1/users"),
            ("path_prefix", "/api/v1/orders")
        ]))
        .applied());
    let mine = fixture.slice_id("path_prefix", "/api/v1/users");
    let other = fixture.slice_id("path_prefix", "/api/v1/orders");
    assert!(fixture
        .run(&fixture.assign_text(&mine, &workers[0], 4000, 2, &["evidence.read"], 1))
        .applied());
    let assignment_id = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(
            &mine,
            super::test_fixture::TARGET,
            1,
            store::HOMOGENEOUS_TRIGGER_CODE,
        ),
    );
    super::apply::mark_started(&fixture.connection, &assignment_id).expect("开跑");
    let forged = types::WorkerReport {
        report_id: store::report_id_for(&assignment_id, 1),
        assignment_id: assignment_id.clone(),
        slice_id: other.clone(),
        worker_id: workers[0].clone(),
        evidence_refs: vec![],
        output: json!({"summary_zh":"冒充别的切片","findings":[],"coverage_notes":["GET /api/v1/orders"]}),
        consumed_tokens: 100,
        consumed_requests: 1,
        terminal_hint: "completed".to_string(),
    };
    let error = super::apply::record_worker_report(&fixture.connection, &fixture.lease, &forged)
        .expect_err("跨切片汇报必须被拒");
    assert_eq!(error.reason_code, reason::CROSS_SLICE_WRITE);
    assert_eq!(
        store::load_slice(&fixture.connection, &other)
            .expect("s")
            .expect("有")
            .state,
        "pending"
    );
}

#[test]
fn worker_output_shape_is_closed_to_the_three_spec_keys() {
    assert!(machine::output_shape_valid(&json!({
        "summary_zh":"ok","findings":[],"coverage_notes":["GET /a"]
    })));
    assert!(!machine::output_shape_valid(&json!({
        "summary_zh":"ok","findings":[],"coverage_notes":["GET /a"],"new_tool":"shell.exec"
    })));
    assert!(!machine::output_shape_valid(&json!({"summary_zh":"ok"})));
}

#[test]
fn consumed_above_reserved_is_refused_at_report_time() {
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(1);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert!(fixture
        .run(&fixture.assign_text(&slice_id, &workers[0], 100, 1, &["evidence.read"], 1))
        .applied());
    let assignment_id = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(
            &slice_id,
            super::test_fixture::TARGET,
            1,
            store::HOMOGENEOUS_TRIGGER_CODE,
        ),
    );
    super::apply::mark_started(&fixture.connection, &assignment_id).expect("开跑");
    let report = types::WorkerReport {
        report_id: store::report_id_for(&assignment_id, 1),
        assignment_id,
        slice_id,
        worker_id: workers[0].clone(),
        evidence_refs: vec![],
        output: json!({"summary_zh":"超额消费","findings":[],"coverage_notes":["GET /api/v1/users"]}),
        consumed_tokens: 999_999,
        consumed_requests: 1,
        terminal_hint: "completed".to_string(),
    };
    assert_eq!(
        super::apply::record_worker_report(&fixture.connection, &fixture.lease, &report)
            .expect_err("必须拒绝")
            .reason_code,
        reason::BUDGET_EXCEEDED
    );
}
