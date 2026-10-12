//! PR-C/PR-F：分配、配额、去重键与收尾（DESIGN §5.1/§5.2、§10.2 GOLD-21/22）。

use super::test_fixture::{Fixture, TARGET};
use super::{pool, reason, store, types};
use rusqlite::params;

#[test]
fn gold04_legal_assign_takes_slot_and_marks_worker_busy() {
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(1);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert!(fixture
        .run(&fixture.assign_text(
            &slice_id,
            &workers[0],
            8000,
            4,
            &["evidence.read", "snapshot.read", "mailbox.write"],
            1
        ))
        .applied());
    let assignment_id = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(&slice_id, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
    );
    let assignment = store::load_assignment(&fixture.connection, &assignment_id)
        .expect("读取")
        .expect("必须存在");
    assert_eq!(assignment.state, "assigned");
    assert_eq!(assignment.slice_id, slice_id);
    assert_eq!(assignment.worker_id, workers[0]);
    assert_eq!(
        store::load_slice(&fixture.connection, &slice_id)
            .expect("切片")
            .expect("有")
            .state,
        "assigned"
    );
    assert_eq!(
        pool::load_worker(&fixture.connection, &workers[0])
            .expect("w")
            .expect("有")
            .state,
        "busy"
    );
    assert_eq!(fixture.task().state, "running");
    assert_eq!(
        store::active_slot_count(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .expect("占用"),
        1
    );
}

#[test]
fn dedup_key_contains_slice_id_so_three_slices_become_three_assignments() {
    // X-01：现状 role:trigger:target:rev 会把三个切片压成一行，目标态必须分开。
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(3);
    assert!(fixture
        .run(&fixture.decompose_text(&[
            ("path_prefix", "/api/v1/users"),
            ("path_prefix", "/api/v1/orders"),
            ("path_prefix", "/api/v1/payments"),
        ]))
        .applied());
    for (index, path) in ["/api/v1/users", "/api/v1/orders", "/api/v1/payments"]
        .iter()
        .enumerate()
    {
        let slice_id = fixture.slice_id("path_prefix", path);
        assert!(fixture
            .run(&fixture.assign_text(&slice_id, &workers[index], 4000, 2, &["evidence.read"], 1))
            .applied());
    }
    let count: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(DISTINCT id) FROM agent_assignments WHERE scheduling_model='homogeneous_workers'",
            [],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(
        count, 3,
        "同 profile/trigger/target/revision 的不同切片必须是三个实例"
    );
    let legacy_occupied: i64 = fixture
        .connection
        .query_row("SELECT COUNT(*) FROM agent_lane_leases", [], |row| {
            row.get(0)
        })
        .expect("计数");
    assert_eq!(
        legacy_occupied, 0,
        "DESIGN §5.1：homogeneous 不写旧 lane 互斥表"
    );
}

#[test]
fn gold05_unauthorized_tool_is_refused() {
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(1);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert_eq!(
        fixture.reason_of(&fixture.assign_text(
            &slice_id,
            &workers[0],
            4000,
            2,
            &["http.get", "shell.exec"],
            1
        )),
        reason::TOOL_NOT_AUTHORIZED
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
fn gold06_reservation_above_remaining_is_refused() {
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(1);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert_eq!(
        fixture.reason_of(&fixture.assign_text(
            &slice_id,
            &workers[0],
            999_999_999,
            4,
            &["evidence.read"],
            1
        )),
        reason::BUDGET_EXCEEDED
    );
    let reserved: i64 = fixture
        .connection
        .query_row(
            "SELECT reserved_tokens FROM agent_budget_ledger WHERE root_run_id=?1",
            [fixture.root_run_id.clone()],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(reserved, 0, "被拒的分配不得预留预算");
}

#[test]
fn x04_lane_quota_blocks_the_fourth_parallel_slice() {
    // X-03/X-04：只读 lane 允许 3 并行；超出配额必须 lane_quota_exceeded。
    let fixture = Fixture::with_capacity(24000, 12, 3);
    let workers = fixture.provision_workers(4);
    let paths = ["/api/v1/a", "/api/v1/b", "/api/v1/c", "/api/v1/d"];
    let scopes: Vec<(&str, &str)> = paths.iter().map(|path| ("path_prefix", *path)).collect();
    assert!(fixture.run(&fixture.decompose_text(&scopes)).applied());
    for index in 0..3 {
        let slice_id = fixture.slice_id("path_prefix", paths[index]);
        assert!(fixture
            .run(&fixture.assign_text(&slice_id, &workers[index], 1000, 1, &["evidence.read"], 1))
            .applied());
    }
    let fourth = fixture.slice_id("path_prefix", paths[3]);
    assert_eq!(
        fixture.reason_of(&fixture.assign_text(
            &fourth,
            &workers[3],
            1000,
            1,
            &["evidence.read"],
            1
        )),
        reason::LANE_QUOTA_EXCEEDED
    );
    assert_eq!(
        store::quota_limit(&fixture.connection, &fixture.root_run_id, "target_touching")
            .expect("配额"),
        1,
        "Phase 1 不得给 target_touching 扩量"
    );
}

#[test]
fn x05_replaying_the_same_slice_is_refused_and_never_duplicates_the_row() {
    // SPEC §2.2 校验 1：只有 pending/rejected 的切片可派；已 assigned 的再派必须被拒，
    // 且 agent_assignments 里只能有一行（DESIGN §5.2 幂等）。
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(2);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    let text = fixture.assign_text(&slice_id, &workers[0], 4000, 2, &["evidence.read"], 1);
    assert!(fixture.run(&text).applied());
    assert_eq!(
        fixture.reason_of(&fixture.assign_text(
            &slice_id,
            &workers[1],
            4000,
            2,
            &["evidence.read"],
            1
        )),
        reason::ASSIGNMENT_NOT_FOUND
    );
    let rows: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE slice_id=?1",
            [slice_id.clone()],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(rows, 1, "重复分配不得产生第二行 assignment");
}

#[test]
fn x05b_same_dedup_key_with_a_different_plan_is_duplicate_slice() {
    // SPEC §2.2 校验 7：同一 dedup key 撞到不同计划 → duplicate_slice，绝不静默覆盖。
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(2);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert!(fixture
        .run(&fixture.assign_text(&slice_id, &workers[0], 4000, 2, &["evidence.read"], 1))
        .applied());
    // 模拟丢更新的异常态：切片被外力改回 pending，但原 assignment 仍然活跃。
    fixture
        .connection
        .execute(
            "UPDATE agent_task_slices SET state='pending' WHERE slice_id=?1",
            [slice_id.clone()],
        )
        .expect("回退切片状态");
    assert_eq!(
        fixture.reason_of(&fixture.assign_text(
            &slice_id,
            &workers[1],
            4000,
            2,
            &["evidence.read"],
            1
        )),
        reason::DUPLICATE_SLICE
    );
    let rows: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE slice_id=?1",
            [slice_id],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(rows, 1);
}

#[test]
fn a_retry_attempt_replanning_beyond_the_remaining_budget_is_budget_exceeded() {
    // SPEC §1.4：reserved_* 必须 ≤ 主管剩余。上一 attempt 终结后同一行可复用，
    // 但超预算的新计划必须被拒，且不覆盖原计划（DESIGN §6 幂等）。
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(2);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert!(fixture
        .run(&fixture.assign_text(&slice_id, &workers[0], 8000, 4, &["evidence.read"], 1))
        .applied());
    fixture
        .connection
        .execute(
            "UPDATE agent_task_slices SET state='rejected',retry_count=1 WHERE slice_id=?1",
            [slice_id.clone()],
        )
        .expect("模拟上一 attempt 已终结");
    fixture
        .connection
        .execute(
            "UPDATE agent_assignments SET state='rejected_retry' WHERE slice_id=?1 AND scheduling_model='homogeneous_workers'",
            [slice_id.clone()],
        )
        .expect("模拟上一 attempt 已终结");
    assert_eq!(
        fixture.reason_of(&fixture.assign_text(
            &slice_id,
            &workers[1],
            12000,
            9,
            &["evidence.read"],
            2
        )),
        reason::BUDGET_EXCEEDED
    );
    let (attempt, reserved, state): (i64, i64, String) = fixture
        .connection
        .query_row(
            "SELECT attempt,reserved_tokens,state FROM agent_assignments WHERE slice_id=?1",
            [slice_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("原计划");
    assert_eq!(
        (attempt, reserved, state.as_str()),
        (1, 8000, "rejected_retry"),
        "被拒的重规划不得改写原 attempt 或额度"
    );
}

#[test]
fn rejected_slice_can_be_reassigned_with_a_new_attempt_until_the_retry_budget_is_spent() {
    // DESIGN §3.2 汇报—验收—再调度闭环：单切片任务一次不通过不得被判死。
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(2);
    let evidence = fixture.add_endpoint_evidence("/api/v1/users", 1);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert!(fixture
        .run(&fixture.assign_text(&slice_id, &workers[0], 8000, 4, &["evidence.read"], 1))
        .applied());
    let first = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(&slice_id, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
    );
    super::apply::mark_started(&fixture.connection, &first).expect("开跑");
    let empty_refs_report = types::WorkerReport {
        report_id: store::report_id_for(&first, 1),
        assignment_id: first.clone(),
        slice_id: slice_id.clone(),
        worker_id: workers[0].clone(),
        evidence_refs: vec![],
        output: serde_json::json!({"summary_zh":"有结论但没证据","findings":[],"coverage_notes":["GET /api/v1/users"]}),
        consumed_tokens: 1000,
        consumed_requests: 1,
        terminal_hint: "completed".to_string(),
    };
    super::apply::record_worker_report(&fixture.connection, &fixture.lease, &empty_refs_report)
        .expect("汇报");
    let checks = super::machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &store::load_assignment(&fixture.connection, &first)
            .expect("a")
            .expect("有"),
        &store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有"),
        &empty_refs_report,
    );
    assert!(fixture
        .run(&fixture.reject_retry_text(
            &first,
            &slice_id,
            &empty_refs_report.report_id,
            &checks,
            None,
            &[reason::EVIDENCE_MISSING]
        ))
        .applied());
    // 任务必须还活着，并且还能再派 attempt 2。
    assert_eq!(fixture.task().state, "running");
    assert!(fixture.allowed_actions().contains(&"assign".to_string()));
    // 同 dedup key 的重放必须携带同一计划（SPEC §2.2 校验 7：只有 attempt 变，预留不变），
    // 不同预留额量的二次分配会被判成 duplicate_slice 而不是静默覆盖。
    assert!(fixture
        .run(&fixture.assign_text(&slice_id, &workers[1], 8000, 4, &["evidence.read"], 2))
        .applied());
    let second = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(&slice_id, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
    );
    assert_eq!(second, first, "同 dedup key 必须复用同一行，只推进 attempt");
    let attempt: i64 = fixture
        .connection
        .query_row(
            "SELECT attempt FROM agent_assignments WHERE id=?1",
            [first.clone()],
            |row| row.get(0),
        )
        .expect("attempt");
    assert_eq!(attempt, 2);
    assert_eq!(
        store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有")
            .state,
        "assigned"
    );
    // 第二次带证据汇报 → accept → 再 stop 收尾。
    super::apply::mark_started(&fixture.connection, &first).expect("再开跑");
    let good_report = types::WorkerReport {
        report_id: store::report_id_for(&first, 2),
        assignment_id: first.clone(),
        slice_id: slice_id.clone(),
        worker_id: workers[1].clone(),
        evidence_refs: vec![evidence.clone()],
        output: serde_json::json!({"summary_zh":"补齐证据","findings":[],"coverage_notes":["GET /api/v1/users"]}),
        consumed_tokens: 1200,
        consumed_requests: 1,
        terminal_hint: "completed".to_string(),
    };
    super::apply::record_worker_report(&fixture.connection, &fixture.lease, &good_report)
        .expect("汇报");
    let good_checks = super::machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &store::load_assignment(&fixture.connection, &first)
            .expect("a")
            .expect("有"),
        &store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有"),
        &good_report,
    );
    assert!(good_checks.all_true(), "{good_checks:?}");
    assert!(fixture
        .run(&fixture.accept_text(
            &first,
            &slice_id,
            &good_report.report_id,
            &good_checks,
            &[&evidence]
        ))
        .applied());
    assert!(fixture
        .run(&fixture.stop_text("goal_reached", &[&slice_id], &[], false))
        .applied());
    assert_eq!(fixture.task().state, "stopped");
}

#[test]
fn gold21_stop_with_running_assignment_is_refused() {
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(1);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert!(fixture
        .run(&fixture.assign_text(&slice_id, &workers[0], 4000, 2, &["evidence.read"], 1))
        .applied());
    let assignment_id = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(&slice_id, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
    );
    super::apply::mark_started(&fixture.connection, &assignment_id).expect("开跑");
    // DESIGN 附录 C：Phase 1 有执行中的 assignment 时禁止 stop。
    assert_eq!(
        fixture.reason_of(&fixture.stop_text("goal_reached", &[], &[&slice_id], false)),
        reason::ALLOWLIST_VIOLATION
    );
    assert_eq!(fixture.task().state, "running");
    // gate 仍会给出 stop（SPEC §3.1 running→stop 合法），真正的拦截在 apply 层（DESIGN 附录 C）。
    assert!(fixture.allowed_actions().contains(&"stop".to_string()));
}

#[test]
fn gold22_partial_stop_publishes_only_accepted_slices() {
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(2);
    assert!(fixture
        .run(&fixture.decompose_text(&[
            ("path_prefix", "/api/v1/users"),
            ("path_prefix", "/api/v1/orders"),
        ]))
        .applied());
    let good = fixture.slice_id("path_prefix", "/api/v1/users");
    let bad = fixture.slice_id("path_prefix", "/api/v1/orders");
    let evidence = fixture.add_endpoint_evidence("/api/v1/users", 1);
    // 第一路：分配 → 开跑 → 带证据汇报 → accept
    assert!(fixture
        .run(&fixture.assign_text(
            &good,
            &workers[0],
            8000,
            4,
            &["evidence.read", "snapshot.read", "mailbox.write"],
            1
        ))
        .applied());
    let good_assignment = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(&good, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
    );
    super::apply::mark_started(&fixture.connection, &good_assignment).expect("开跑");
    let report = types::WorkerReport {
        report_id: store::report_id_for(&good_assignment, 1),
        assignment_id: good_assignment.clone(),
        slice_id: good.clone(),
        worker_id: workers[0].clone(),
        evidence_refs: vec![evidence.clone()],
        output: serde_json::json!({"summary_zh":"发现 1 个端点","findings":[],"coverage_notes":["GET /api/v1/users"]}),
        consumed_tokens: 3200,
        consumed_requests: 2,
        terminal_hint: "completed".to_string(),
    };
    super::apply::record_worker_report(&fixture.connection, &fixture.lease, &report).expect("汇报");
    let checks = super::machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &store::load_assignment(&fixture.connection, &good_assignment)
            .expect("a")
            .expect("有"),
        &store::load_slice(&fixture.connection, &good)
            .expect("s")
            .expect("有"),
        &report,
    );
    assert!(checks.all_true(), "{checks:?}");
    assert!(fixture
        .run(&fixture.accept_text(
            &good_assignment,
            &good,
            &report.report_id,
            &checks,
            &[&evidence]
        ))
        .applied());
    // 第二路：分配 → 开跑 → 汇报无证据 → reject_retry
    assert!(fixture
        .run(&fixture.assign_text(&bad, &workers[1], 8000, 4, &["evidence.read"], 1))
        .applied());
    let bad_assignment = store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(&bad, TARGET, 1, store::HOMOGENEOUS_TRIGGER_CODE),
    );
    super::apply::mark_started(&fixture.connection, &bad_assignment).expect("开跑");
    let bad_report = types::WorkerReport {
        report_id: store::report_id_for(&bad_assignment, 1),
        assignment_id: bad_assignment.clone(),
        slice_id: bad.clone(),
        worker_id: workers[1].clone(),
        evidence_refs: vec![],
        output: serde_json::json!({"summary_zh":"发现 2 个端点","findings":[],"coverage_notes":["GET /api/v1/orders"]}),
        consumed_tokens: 1000,
        consumed_requests: 1,
        terminal_hint: "completed".to_string(),
    };
    super::apply::record_worker_report(&fixture.connection, &fixture.lease, &bad_report)
        .expect("汇报");
    let bad_checks = super::machine::recompute(
        &fixture.connection,
        &fixture.task(),
        &store::load_assignment(&fixture.connection, &bad_assignment)
            .expect("a")
            .expect("有"),
        &store::load_slice(&fixture.connection, &bad)
            .expect("s")
            .expect("有"),
        &bad_report,
    );
    assert!(!bad_checks.evidence_refs_exist);
    assert!(fixture
        .run(&fixture.reject_retry_text(
            &bad_assignment,
            &bad,
            &bad_report.report_id,
            &bad_checks,
            None,
            &[reason::EVIDENCE_MISSING]
        ))
        .applied());
    // stop partial：只有 accepted 的证据进入结论
    assert!(fixture
        .run(&fixture.stop_text("goal_reached", &[&good], &[&bad], true))
        .applied());
    assert_eq!(fixture.task().state, "stopped");
    let verdicts: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM agent_acceptance_verdicts WHERE verdict='accept'",
            [],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(verdicts, 1);
    let accepted_state: String = fixture
        .connection
        .query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            params![good_assignment],
            |row| row.get(0),
        )
        .expect("状态");
    assert_eq!(accepted_state, "accepted");
    assert_eq!(
        store::active_slot_count(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .expect("占用"),
        0,
        "stop 后不得留占用"
    );
}

#[test]
fn a_degraded_task_dispatches_only_the_first_pending_slice() {
    // SPEC §4 / DESIGN §7 第 4 条：确定性降级后第一期只串行派第一个 pending 切片，
    // 其余切片不进候选，被模型引用时以 allowlist_violation 拒绝（不是静默跳过）。
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(2);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/a"), ("path_prefix", "/api/b"),]))
        .applied());
    fixture
        .connection
        .execute(
            "UPDATE agent_homogeneous_tasks SET degraded=1,degrade_reason_code='degraded_path_used' WHERE root_run_id=?1",
            [fixture.root_run_id.as_str()],
        )
        .expect("标记降级");
    let task = fixture.task();
    assert!(task.degraded, "任务必须带降级标记");
    let candidates = super::gate::snapshot_for(&fixture.connection, &task)
        .expect("候选")
        .slice_ids;
    assert_eq!(
        candidates.len(),
        2,
        "候选仍列全部切片，串行限制是 apply 的硬护栏"
    );
    let first = fixture.slice_id("path_prefix", "/api/a");
    let second = fixture.slice_id("path_prefix", "/api/b");
    assert_eq!(candidates[0], first);
    assert_eq!(
        fixture.reason_of(&fixture.assign_text(
            &second,
            &workers[1],
            4000,
            2,
            &["evidence.read"],
            1
        )),
        reason::ALLOWLIST_VIOLATION
    );
    assert!(fixture
        .run(&fixture.assign_text(&first, &workers[0], 4000, 2, &["evidence.read"], 1))
        .applied());
    assert_eq!(
        store::load_slice(&fixture.connection, &second)
            .expect("s")
            .expect("有")
            .state,
        "pending",
        "未派发的切片必须留在 pending，不得被静默终结"
    );
}

#[test]
fn an_expired_worker_carrier_is_reclaimed_before_the_next_decision() {
    // P1-3 / DESIGN §6：心跳到期后，主管轮次先把 slot 与 Worker 收回来，
    // 再把切片留在可 retry 的 rejected，而不是让配额虚占。
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(2);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    assert!(fixture
        .run(&fixture.assign_text(&slice_id, &workers[0], 8000, 4, &["evidence.read"], 1,))
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
    assert_eq!(
        store::active_slot_count(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .expect("占用"),
        1
    );
    fixture
        .connection
        .execute(
            "UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','-1 hours')",
            [],
        )
        .expect("模拟心跳到期");
    // 下一轮（这里是一句非法输出，只为了触发轮首回收）之前，载体必须已被收回。
    let reclaimed =
        pool::reclaim_expired_for_task(&fixture.connection, &fixture.task()).expect("回收");
    assert_eq!(reclaimed, vec![assignment_id.clone()]);
    assert_eq!(
        store::active_slot_count(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .expect("占用"),
        0,
        "到期回收必须释放只读槽位"
    );
    assert_eq!(
        store::load_assignment(&fixture.connection, &assignment_id)
            .expect("a")
            .expect("有")
            .state,
        "cancelled"
    );
    assert_eq!(
        store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有")
            .state,
        "rejected",
        "基础设施到期不是质量失败：切片保持可 retry"
    );
    assert_eq!(
        pool::load_worker(&fixture.connection, &workers[0])
            .expect("w")
            .expect("有")
            .state,
        "dead"
    );
    assert_eq!(
        fixture
            .connection
            .query_row::<String, _, _>(
                "SELECT status FROM agent_runs WHERE id=(SELECT child_run_id FROM agent_assignments WHERE id=?1)",
                [assignment_id.as_str()],
                |row| row.get(0),
            )
            .expect("载体状态"),
        "terminal",
        "到期回收同时关闭执行载体"
    );
    // 释放后同一切片仍可以新 attempt 再派给另一个 Worker。
    let retry =
        fixture.run(&fixture.assign_text(&slice_id, &workers[1], 8000, 4, &["evidence.read"], 2));
    assert!(retry.applied(), "到期回收后必须能再派：{retry:?}");
}
