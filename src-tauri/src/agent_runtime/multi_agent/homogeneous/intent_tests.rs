//! 深度分析 §J 的意图状态机回归；所有 Fixture 仅使用临时库。

use super::apply::TurnOutcome;
use super::test_fixture::{Fixture, TARGET};
use super::{decision, directive_gate, gate, intent, machine, pool, reason, store, worker};
use rusqlite::params;
use serde_json::{json, Value};

fn assigned(fixture: &Fixture, paths: &[&str], retries: i64) -> Vec<String> {
    let workers = fixture.provision_workers(paths.len() as i64);
    let scopes: Vec<(&str, &str)> = paths.iter().map(|path| ("path_prefix", *path)).collect();
    assert!(fixture
        .run(&fixture.decompose_text_with_retries(&scopes, retries))
        .applied());
    paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let slice_id = fixture.slice_id("path_prefix", path);
            assert!(fixture
                .run(&fixture.assign_text(
                    &slice_id,
                    &workers[index],
                    1000,
                    1,
                    &["evidence.read"],
                    1
                ))
                .applied());
            store::assignment_id_for(
                &fixture.root_run_id,
                &store::dedup_key_for(&slice_id, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
            )
        })
        .collect()
}

fn directive(fixture: &Fixture) -> String {
    let id = "dir-0123456789abcdef";
    directive_gate::screen(&fixture.connection, &fixture.task(), id, "", "多关注鉴权").unwrap();
    id.to_string()
}

fn zero_turn(fixture: &Fixture, reason: &str, ids: &[String]) -> Value {
    let id = directive(fixture);
    let mut value: Value =
        serde_json::from_str(&fixture.relay_text(&id, "relay", None, None, Some("只分析现有证据")))
            .unwrap();
    value["zero_intent"] =
        json!({"reason":reason,"evidence_assignment_ids":ids,"note_zh":"下一步依赖现有切片的产出"});
    value["allowlist_refs"]["assignment_ids"] = json!(ids);
    value
}

fn pending(fixture: &Fixture) -> Vec<String> {
    intent::snapshot(&fixture.connection, &fixture.task())
        .unwrap()
        .pending_slice_ids
}

fn audit_count(fixture: &Fixture) -> i64 {
    fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM agent_homogeneous_decision_audit",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn first_zero_intent_is_refused_and_immediately_replenishes_one_slice() {
    let fixture = Fixture::new();
    assert!(matches!(fixture.run("{}"), TurnOutcome::Degraded(_)));
    assert_eq!(pending(&fixture).len(), 1);
    assert!(fixture.task().degraded);
    assert_eq!(
        fixture.task().degrade_reason_code,
        reason::DEGRADED_PATH_USED
    );
    let trigger: String = fixture.connection.query_row(
        "SELECT trigger_reason_code FROM agent_homogeneous_decision_audit ORDER BY audit_id DESC LIMIT 1", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        trigger,
        reason::ZERO_INTENT_ILLEGAL,
        "立即降级必须留存受控触发原因"
    );
    let before = pending(&fixture);
    let workers = fixture.provision_workers(1);
    assert!(
        fixture
            .run(&fixture.assign_text(&before[0], &workers[0], 1000, 1, &["evidence.read"], 1))
            .applied(),
        "补出的意图必须可派发"
    );
}

#[test]
fn first_non_goal_stop_cannot_bypass_progress() {
    for stop_reason in ["degraded_path_used", "budget_exhausted"] {
        let fixture = Fixture::new();
        let outcome = fixture.run(&fixture.stop_text(stop_reason, &[], &[], false));
        assert!(matches!(outcome, TurnOutcome::Degraded(_)), "{outcome:?}");
        assert_eq!(pending(&fixture).len(), 1);
        assert_ne!(fixture.task().state, "stopped");
    }
}

#[test]
fn two_inflight_allow_structured_blocked_zero_and_persist_evidence() {
    let fixture = Fixture::new();
    let ids = assigned(&fixture, &["/a", "/b"], 2);
    let text = zero_turn(&fixture, "blocked_on_inflight_output", &ids);
    assert!(fixture.run(&text.to_string()).applied());
    assert_eq!(
        intent::snapshot(&fixture.connection, &fixture.task())
            .unwrap()
            .inflight_assignments
            .len(),
        2
    );
    assert!(pending(&fixture).is_empty());
    let audit: String = fixture.connection.query_row(
        "SELECT zero_intent_json FROM agent_homogeneous_decision_audit WHERE action='relay_directive' ORDER BY audit_id DESC LIMIT 1", [], |row| row.get(0),
    ).unwrap();
    let saved: Value = serde_json::from_str(&audit).unwrap();
    assert_eq!(saved["reason"], "blocked_on_inflight_output");
    assert_eq!(saved["evidence_assignment_ids"], json!(ids));
    assert!(fixture
        .connection
        .execute("DELETE FROM agent_homogeneous_decision_audit", [])
        .is_err());
}

#[test]
fn two_inflight_without_zero_reason_roll_back_the_entire_action() {
    let fixture = Fixture::new();
    let ids = assigned(&fixture, &["/a", "/b"], 2);
    let mut text = zero_turn(&fixture, "covered_by_inflight", &ids);
    text.as_object_mut().unwrap().remove("zero_intent");
    let before = audit_count(&fixture);
    assert_eq!(
        fixture.reason_of(&text.to_string()),
        reason::ZERO_INTENT_ILLEGAL
    );
    assert_eq!(audit_count(&fixture), before);
    assert_eq!(
        fixture.reason_of(&text.to_string()),
        reason::ZERO_INTENT_ILLEGAL
    );
    assert!(matches!(fixture.run("{}"), TurnOutcome::Idle(_)));
    assert_eq!(audit_count(&fixture), before);
    assert_eq!(
        store::quota_limit(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .unwrap(),
        3
    );
    let directive_id = text["payload"]["directive_id"].as_str().unwrap();
    assert_eq!(
        store::load_gated_directive(&fixture.connection, directive_id)
            .unwrap()
            .unwrap()
            .state,
        "awaiting_coordinator"
    );
    for id in ids {
        assert_eq!(
            store::load_assignment(&fixture.connection, &id)
                .unwrap()
                .unwrap()
                .state,
            "assigned"
        );
    }
}

#[test]
fn zero_reason_missing_fields_invented_reason_and_unknown_evidence_are_refused() {
    for invalid in [
        json!({"reason":"blocked_on_inflight_output","note_zh":"缺证据"}),
        json!({"reason":"等一等","evidence_assignment_ids":["asg-0123456789abcdef"],"note_zh":"胡写"}),
    ] {
        let fixture = Fixture::new();
        let ids = assigned(&fixture, &["/a", "/b"], 2);
        let mut value = zero_turn(&fixture, "covered_by_inflight", &ids);
        value["zero_intent"] = invalid;
        assert_eq!(
            fixture.reason_of(&value.to_string()),
            reason::ZERO_INTENT_ILLEGAL
        );
    }
    let fixture = Fixture::new();
    let ids = assigned(&fixture, &["/a", "/b"], 2);
    let mut value = zero_turn(&fixture, "covered_by_inflight", &ids);
    value["zero_intent"]["evidence_assignment_ids"] = json!(["asg-aaaaaaaaaaaaaaaa"]);
    assert!(!fixture.run(&value.to_string()).applied());
    assert_eq!(audit_count(&fixture), 3);
}

#[test]
fn a_queued_slice_without_real_assignment_does_not_justify_zero() {
    let fixture = Fixture::new();
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/a")]))
        .applied());
    let value = zero_turn(&fixture, "covered_by_inflight", &[]);
    assert_eq!(
        fixture.reason_of(&value.to_string()),
        reason::ZERO_INTENT_ILLEGAL
    );
    assert_eq!(pending(&fixture).len(), 1);
}

#[test]
fn reclaim_with_no_retry_queue_replenishes_a_new_direction() {
    let fixture = Fixture::new();
    let ids = assigned(&fixture, &["/a"], 0);
    let value = zero_turn(&fixture, "blocked_on_inflight_output", &ids);
    fixture.connection.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','-1 minute') WHERE assignment_id=?1", [&ids[0]]).unwrap();
    assert!(!fixture.run(&value.to_string()).applied());
    assert_eq!(
        store::load_assignment(&fixture.connection, &ids[0])
            .unwrap()
            .unwrap()
            .state,
        "cancelled"
    );
    assert_eq!(pending(&fixture).len(), 1);
    let fresh = store::load_slice(&fixture.connection, &pending(&fixture)[0])
        .unwrap()
        .unwrap();
    assert_eq!(fresh.scope_kind, "host");
    assert_eq!(fresh.decomposition_model, "deterministic");
    fixture.provision_workers(1);
    let idle =
        pool::worker_ids_in_state(&fixture.connection, &fixture.root_run_id, "idle").unwrap();
    assert_eq!(idle.len(), 1);
    assert!(fixture
        .run(&fixture.assign_text(&fresh.slice_id, &idle[0], 1000, 1, &["evidence.read"], 1))
        .applied());
}

#[test]
fn reclaim_retryable_slice_can_be_assigned_to_a_capacity_replacement() {
    let fixture = Fixture::with_capacity(24000, 12, 1);
    let ids = assigned(&fixture, &["/a"], 2);
    fixture.connection.execute("UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','-1 minute') WHERE assignment_id=?1", [&ids[0]]).unwrap();
    pool::reclaim_expired_for_task(&fixture.connection, &fixture.task()).unwrap();
    assert_eq!(
        store::remaining_budget(&fixture.connection, &fixture.root_run_id).unwrap(),
        (24000, 12),
        "回收必须释放未消费预留"
    );
    let cancelled = store::load_assignment(&fixture.connection, &ids[0])
        .unwrap()
        .unwrap();
    pool::reclaim_expired_worker(
        &fixture.connection,
        &fixture.task(),
        &cancelled.worker_id,
        &ids[0],
    )
    .unwrap();
    assert_eq!(
        store::remaining_budget(&fixture.connection, &fixture.root_run_id).unwrap(),
        (24000, 12),
        "重复回收不能重复释放"
    );
    let before = intent::snapshot(&fixture.connection, &fixture.task()).unwrap();
    assert_eq!(before.retryable_slice_ids.len(), 1);
    assert!(before.inflight_assignments.is_empty());
    fixture.provision_workers(1);
    let idle =
        pool::worker_ids_in_state(&fixture.connection, &fixture.root_run_id, "idle").unwrap();
    assert_eq!(idle.len(), 1);
    assert!(fixture
        .run(&fixture.assign_text(
            &before.retryable_slice_ids[0],
            &idle[0],
            1000,
            1,
            &["evidence.read"],
            2
        ))
        .applied());
    assert_eq!(
        store::load_assignment(&fixture.connection, &ids[0])
            .unwrap()
            .unwrap()
            .attempt,
        2
    );
}

#[test]
fn automatic_reports_accept_accumulate_then_goal_stop_is_terminal() {
    let fixture = Fixture::new();
    for path in ["/a", "/b"] {
        fixture.add_endpoint_evidence(path, 1);
    }
    let ids = assigned(&fixture, &["/a", "/b"], 2);
    let reports = worker::drain_for_lease(&fixture.connection, &fixture.lease).unwrap();
    assert_eq!(reports.len(), 2);
    let mut slices = Vec::new();
    for id in ids {
        let assignment = store::load_assignment(&fixture.connection, &id)
            .unwrap()
            .unwrap();
        let slice = store::load_slice(&fixture.connection, &assignment.slice_id)
            .unwrap()
            .unwrap();
        let report = store::load_report(&fixture.connection, &store::report_id_for(&id, 1))
            .unwrap()
            .unwrap();
        let checks = machine::recompute(
            &fixture.connection,
            &fixture.task(),
            &assignment,
            &slice,
            &report,
        );
        assert!(checks.all_true());
        let refs: Vec<&str> = report.evidence_refs.iter().map(String::as_str).collect();
        assert!(fixture
            .run(&fixture.accept_text(&id, &slice.slice_id, &report.report_id, &checks, &refs))
            .applied());
        slices.push(slice.slice_id);
    }
    assert!(intent::snapshot(&fixture.connection, &fixture.task())
        .unwrap()
        .is_empty());
    let refs: Vec<&str> = slices.iter().map(String::as_str).collect();
    assert!(fixture
        .run(&fixture.stop_text("goal_reached", &refs, &[], false))
        .applied());
    assert_eq!(fixture.task().state, "stopped");
    assert_eq!(
        intent::latest_stop_reason(&fixture.connection, &fixture.task().task_id)
            .unwrap()
            .as_deref(),
        Some("goal_reached")
    );
    assert!(matches!(fixture.run("{}"), TurnOutcome::Idle(_)));
}

#[test]
fn retries_exhausted_keep_the_coordinator_available_for_new_direction_or_stop() {
    let fixture = Fixture::new();
    let ids = assigned(&fixture, &["/a"], 1);
    worker::drain_for_lease(&fixture.connection, &fixture.lease).unwrap();
    let assignment = store::load_assignment(&fixture.connection, &ids[0])
        .unwrap()
        .unwrap();
    let slice = store::load_slice(&fixture.connection, &assignment.slice_id)
        .unwrap()
        .unwrap();
    let report = store::load_report(&fixture.connection, &store::report_id_for(&ids[0], 1))
        .unwrap()
        .unwrap();
    let checks = machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &assignment,
        &slice,
        &report,
    );
    assert!(!checks.all_true());
    assert!(fixture
        .run(&fixture.reject_retry_text(
            &ids[0],
            &slice.slice_id,
            &report.report_id,
            &checks,
            None,
            &["evidence_missing"]
        ))
        .applied());
    assert_eq!(fixture.task().state, "running");
    assert!(intent::snapshot(&fixture.connection, &fixture.task())
        .unwrap()
        .is_empty());
    assert!(fixture.allowed_actions().contains(&"decompose".to_string()));
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/b")]))
        .applied());
    assert_eq!(pending(&fixture).len(), 1);
}

#[test]
fn completely_exhausted_fallback_stops_instead_of_paying_for_empty_candidates() {
    let fixture = Fixture::new();
    assert!(fixture
        .run(&fixture.decompose_text_with_retries(&[("host", "app.example.com")], 0))
        .applied());
    let slice_id = fixture.slice_id("host", "app.example.com");
    // 模拟基础设施回收后无可重试队列；不覆写任何真实库。
    fixture
        .connection
        .execute(
            "UPDATE agent_task_slices SET state='rejected' WHERE slice_id=?1",
            [&slice_id],
        )
        .unwrap();
    store::set_task_state(
        &fixture.connection,
        &fixture.task().task_id,
        "running",
        fixture.lease.lease_epoch,
    )
    .unwrap();
    assert!(matches!(fixture.run("{}"), TurnOutcome::Degraded(_)));
    assert_eq!(fixture.task().state, "stopped");
    assert_eq!(
        intent::latest_stop_reason(&fixture.connection, &fixture.task().task_id)
            .unwrap()
            .as_deref(),
        Some("retry_limit_exceeded")
    );
    assert!(matches!(fixture.run("{}"), TurnOutcome::Idle(_)));
}

#[test]
fn assignment_state_and_retry_changes_invalidate_the_basis() {
    let fixture = Fixture::new();
    let ids = assigned(&fixture, &["/a"], 2);
    let before = gate::snapshot_for(&fixture.connection, &fixture.task()).unwrap();
    assert_eq!(before.assignment_ids, ids);
    super::apply::mark_started(&fixture.connection, &ids[0]).unwrap();
    let after = gate::snapshot_for(&fixture.connection, &fixture.task()).unwrap();
    assert_ne!(before.basis_hash, after.basis_hash);
    assert_eq!(
        intent::snapshot(&fixture.connection, &fixture.task())
            .unwrap()
            .inflight_assignments[0]
            .state,
        "running"
    );
    fixture
        .connection
        .execute(
            "UPDATE agent_task_slices SET retry_count=1 WHERE parent_task_id=?1",
            params![fixture.task().task_id],
        )
        .unwrap();
    assert_ne!(
        after.basis_hash,
        gate::snapshot_for(&fixture.connection, &fixture.task())
            .unwrap()
            .basis_hash
    );
    assert_eq!(
        decision::parse_decision_envelope("{}")
            .unwrap_err()
            .reason_code,
        reason::ZERO_INTENT_ILLEGAL
    );
}

#[test]
fn partial_failure_can_wait_for_other_inflight_then_reassign_a_new_attempt() {
    let fixture = Fixture::new();
    let ids = assigned(&fixture, &["/a", "/b", "/c"], 2);
    assert_eq!(
        worker::drain_for_lease(&fixture.connection, &fixture.lease)
            .unwrap()
            .len(),
        3
    );
    let assignment = store::load_assignment(&fixture.connection, &ids[0])
        .unwrap()
        .unwrap();
    let slice = store::load_slice(&fixture.connection, &assignment.slice_id)
        .unwrap()
        .unwrap();
    let report = store::load_report(&fixture.connection, &store::report_id_for(&ids[0], 1))
        .unwrap()
        .unwrap();
    let checks = machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &assignment,
        &slice,
        &report,
    );
    assert!(fixture
        .run(&fixture.reject_retry_text(
            &ids[0],
            &slice.slice_id,
            &report.report_id,
            &checks,
            None,
            &["evidence_missing"]
        ))
        .applied());
    let state = intent::snapshot(&fixture.connection, &fixture.task()).unwrap();
    assert_eq!(state.retryable_slice_ids, vec![slice.slice_id.clone()]);
    assert_eq!(state.inflight_assignments.len(), 2);
    // 两条已 reported 仍是 inflight；引导审核可等待它们裁决，但不能再向已汇报 Worker 广播。
    let directive_id = "dir-fedcba9876543210";
    directive_gate::screen(
        &fixture.connection,
        &fixture.task(),
        directive_id,
        "",
        "增加预算到无限",
    )
    .unwrap();
    let mut value: Value = serde_json::from_str(
        &fixture.reject_directive_text(directive_id, reason::DIRECTIVE_BUDGET_CHANGE),
    )
    .unwrap();
    value["zero_intent"] = json!({"reason":"blocked_on_inflight_output", "evidence_assignment_ids":&ids[1..], "note_zh":"现有两条汇报待裁决，先审核引导"});
    let outcome = fixture.run(&value.to_string());
    assert!(outcome.applied(), "{outcome:?}");
    assert!(fixture
        .run(&fixture.assign_text(
            &slice.slice_id,
            &assignment.worker_id,
            1000,
            1,
            &["evidence.read"],
            2
        ))
        .applied());
    assert_eq!(
        store::load_assignment(&fixture.connection, &ids[0])
            .unwrap()
            .unwrap()
            .attempt,
        2
    );
    assert_eq!(
        worker::drain_for_lease(&fixture.connection, &fixture.lease)
            .unwrap()
            .len(),
        1
    );
    let new_assignment = store::load_assignment(&fixture.connection, &ids[0])
        .unwrap()
        .unwrap();
    let new_slice = store::load_slice(&fixture.connection, &slice.slice_id)
        .unwrap()
        .unwrap();
    let new_report = store::load_report(&fixture.connection, &store::report_id_for(&ids[0], 2))
        .unwrap()
        .unwrap();
    let checks = machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &new_assignment,
        &new_slice,
        &new_report,
    );
    assert!(fixture
        .run(&fixture.reject_retry_text(
            &ids[0],
            &slice.slice_id,
            &new_report.report_id,
            &checks,
            None,
            &["evidence_missing"]
        ))
        .applied());
    assert!(
        intent::snapshot(&fixture.connection, &fixture.task())
            .unwrap()
            .retryable_slice_ids
            .is_empty(),
        "retry_count==max_retries 不再入可重派队列"
    );
    assert!(!fixture
        .run(&fixture.assign_text(
            &slice.slice_id,
            &assignment.worker_id,
            1000,
            1,
            &["evidence.read"],
            3
        ))
        .applied());
}

#[test]
fn real_accepted_assignment_cannot_be_reused_as_zero_evidence() {
    let fixture = Fixture::new();
    fixture.add_endpoint_evidence("/a", 1);
    let ids = assigned(&fixture, &["/a", "/b"], 2);
    worker::drain_for_lease(&fixture.connection, &fixture.lease).unwrap();
    let assignment = store::load_assignment(&fixture.connection, &ids[0])
        .unwrap()
        .unwrap();
    let slice = store::load_slice(&fixture.connection, &assignment.slice_id)
        .unwrap()
        .unwrap();
    let report = store::load_report(&fixture.connection, &store::report_id_for(&ids[0], 1))
        .unwrap()
        .unwrap();
    let checks = machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &assignment,
        &slice,
        &report,
    );
    let refs: Vec<&str> = report.evidence_refs.iter().map(String::as_str).collect();
    assert!(fixture
        .run(&fixture.accept_text(&ids[0], &slice.slice_id, &report.report_id, &checks, &refs))
        .applied());
    let value = zero_turn(&fixture, "covered_by_inflight", &ids[..1]);
    assert!(!fixture.run(&value.to_string()).applied());
    assert_eq!(
        store::load_assignment(&fixture.connection, &ids[0])
            .unwrap()
            .unwrap()
            .state,
        "accepted"
    );
    assert_eq!(
        intent::snapshot(&fixture.connection, &fixture.task())
            .unwrap()
            .inflight_assignments
            .len(),
        1
    );
}

#[test]
fn another_tasks_real_assignment_cannot_cover_this_tasks_zero_turn() {
    let fixture = Fixture::new();
    let ids = assigned(&fixture, &["/a", "/b"], 2);
    let other_root = "run-other-intent-test";
    fixture.connection.execute(
        "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,role,status,root_run_id,orchestration_policy,hard_token_budget,hard_request_budget) VALUES(?1,?2,1,?3,'coordinator','running',?1,'multi',24000,12)",
        params![other_root, fixture.scan_id, TARGET],
    ).unwrap();
    let other_task_id = match store::create_task(
        &fixture.connection,
        other_root,
        &fixture.scan_id,
        1,
        TARGET,
        super::types::SCHEDULING_HOMOGENEOUS,
        1,
        1,
    )
    .unwrap()
    {
        store::TaskInsert::Created(id) | store::TaskInsert::Existing(id) => id,
    };
    let mut other = store::load_assignment(&fixture.connection, &ids[0])
        .unwrap()
        .unwrap();
    other.assignment_id = "asg-1234567890abcdef".to_string();
    other.coordinator_run_id = other_root.to_string();
    other.parent_task_id = other_task_id;
    other.dedup_key = "foreign-intent-dedup".to_string();
    other.slice_id = "slc-1234567890abcdef".to_string();
    store::insert_assignment(&fixture.connection, &other).unwrap();
    let value = zero_turn(&fixture, "covered_by_inflight", &[other.assignment_id]);
    assert!(!fixture.run(&value.to_string()).applied());
    assert_eq!(
        intent::snapshot(&fixture.connection, &fixture.task())
            .unwrap()
            .inflight_assignments
            .len(),
        2
    );
}

#[test]
fn malformed_streak_preserves_running_assignments_and_quota() {
    let fixture = Fixture::new();
    let ids = assigned(&fixture, &["/a", "/b"], 2);
    assert!(matches!(
        fixture.run("模型自由长文"),
        TurnOutcome::Refused { .. }
    ));
    assert!(matches!(fixture.run("再次自由长文"), TurnOutcome::Idle(_)));
    assert!(
        fixture.allowed_actions().is_empty(),
        "达到阈值后禁止再付费问模型"
    );
    assert_eq!(fixture.task().state, "running");
    assert_eq!(
        store::quota_limit(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .unwrap(),
        3
    );
    assert_eq!(
        store::active_slot_count(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .unwrap(),
        2
    );
    assert_eq!(
        intent::snapshot(&fixture.connection, &fixture.task())
            .unwrap()
            .inflight_assignments
            .len(),
        2
    );
    assert_eq!(audit_count(&fixture), 3);
    for id in ids {
        assert_eq!(
            store::load_assignment(&fixture.connection, &id)
                .unwrap()
                .unwrap()
                .state,
            "assigned"
        );
    }
}

#[test]
fn malformed_streak_preserves_pending_and_empty_streak_replenishes() {
    let fixture = Fixture::new();
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/a")]))
        .applied());
    let before = pending(&fixture);
    fixture.run("模型自由长文");
    fixture.run("再次自由长文");
    assert_eq!(pending(&fixture), before);
    assert_eq!(
        store::quota_limit(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .unwrap(),
        3
    );
    let fixture = Fixture::new();
    store::bump_schema_fail_streak(&fixture.connection, &fixture.task().task_id).unwrap();
    store::bump_schema_fail_streak(&fixture.connection, &fixture.task().task_id).unwrap();
    assert!(matches!(
        fixture.run("不应再请求模型"),
        TurnOutcome::Degraded(_)
    ));
    assert_eq!(pending(&fixture).len(), 1);
}

#[test]
fn a_directive_on_an_empty_task_waits_until_an_intent_is_dispatched() {
    let fixture = Fixture::new();
    let id = directive(&fixture);
    let text = fixture.relay_text(&id, "relay", None, None, Some("只分析现有证据"));
    assert!(matches!(fixture.run(&text), TurnOutcome::Degraded(_)));
    assert_eq!(
        store::load_gated_directive(&fixture.connection, &id)
            .unwrap()
            .unwrap()
            .state,
        "awaiting_coordinator"
    );
    let workers = fixture.provision_workers(1);
    assert!(fixture
        .run(&fixture.assign_text(
            &pending(&fixture)[0],
            &workers[0],
            1000,
            1,
            &["evidence.read"],
            1
        ))
        .applied());
    assert!(fixture
        .run(&fixture.relay_text(&id, "relay", None, None, Some("只分析现有证据")))
        .applied());
}

#[test]
fn a_real_exhausted_budget_stops_without_creating_an_unusable_intent() {
    let fixture = Fixture::with_capacity(100, 1, 1);
    store::ensure_ledger(&fixture.connection, &fixture.lease).unwrap();
    store::reserve_budget(&fixture.connection, &fixture.lease, 100, 1).unwrap();
    store::settle_budget(&fixture.connection, &fixture.root_run_id, 100, 1, 100, 1).unwrap();
    assert!(fixture
        .run(&fixture.stop_text("budget_exhausted", &[], &[], false))
        .applied());
    assert_eq!(fixture.task().state, "stopped");
    assert!(pending(&fixture).is_empty());
    assert_eq!(
        intent::latest_stop_reason(&fixture.connection, &fixture.task().task_id)
            .unwrap()
            .as_deref(),
        Some("budget_exhausted")
    );
}

#[test]
fn unlimited_zero_caps_cannot_stop_as_budget_exhausted_and_must_replenish() {
    let fixture = Fixture::with_capacity(0, 0, 1);
    assert!(!intent::budget_exhausted(&fixture.connection, &fixture.root_run_id).unwrap());
    assert!(!gate::allowed_actions(&fixture.connection, &fixture.task())
        .unwrap()
        .contains(&"stop".to_string()));
    let outcome = fixture.run(&fixture.stop_text("budget_exhausted", &[], &[], false));
    assert!(!outcome.applied(), "{outcome:?}");
    assert!(pending(&fixture).len() >= 1, "{outcome:?}");
    assert_ne!(fixture.task().state, "stopped");
    assert_eq!(
        intent::latest_stop_reason(&fixture.connection, &fixture.task().task_id).unwrap(),
        None
    );
    let slice = pending(&fixture)[0].clone();
    let worker = fixture.provision_workers(1).remove(0);
    let assign = fixture.run(&fixture.assign_text(&slice, &worker, 10, 1, &["evidence.read"], 1));
    assert!(assign.applied(), "{assign:?}");
    worker::drain_for_lease(&fixture.connection, &fixture.lease).unwrap();
    let assignment = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(&slice, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
    );
    assert_eq!(
        store::load_assignment(&fixture.connection, &assignment)
            .unwrap()
            .unwrap()
            .state,
        "reported"
    );
}
