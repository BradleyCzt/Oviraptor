//! P0-3：临时库 + 假模型文本的接线集成测试（DESIGN §5.3、审核 P0-1/P0-2）。
//!
//! 覆盖两件事：
//! 1. `assign` 之后确实出现现网执行载体（child run / attempt / mailbox 消息），
//!    汇报与裁决按状态机推进，载体只在裁决落地后关闭；
//! 2. Root 付费 tick 的返回路径调用的就是 `apply::run_coordinator_turn`
//!    （通过 `native_homogeneous_apply_after_paid_tick` 这一生产入口，不复制逻辑）。

use super::test_fixture::Fixture;
use super::{apply, journal, machine, pool, store, types};

fn carrier(db: &rusqlite::Connection, assignment_id: &str) -> (String, String) {
    db.query_row(
        "SELECT r.status, x.state FROM agent_assignment_attempts x
         JOIN agent_assignments a ON a.id=x.assignment_id AND a.child_run_id=x.child_run_id
         JOIN agent_runs r ON r.id=x.child_run_id
         WHERE a.id=?1",
        [assignment_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("载体可读")
}

fn messages(db: &rusqlite::Connection, assignment_id: &str) -> Vec<(String, String)> {
    db.prepare(
        "SELECT kind, acknowledged_at FROM agent_messages WHERE assignment_id=?1 ORDER BY correlation_id",
    )
    .expect("语句")
    .query_map([assignment_id], |row| Ok((row.get(0)?, row.get(1)?)))
    .expect("查询")
    .filter_map(Result::ok)
    .collect()
}

#[test]
fn three_read_only_dispatches_create_live_carriers_and_accept_settles_only_itself() {
    let fixture = Fixture::with_capacity(24000, 12, 3);
    let db = &fixture.connection;
    let workers = fixture.provision_workers(3);
    let paths = ["/api/v1/a", "/api/v1/b", "/api/v1/c"];
    let scopes: Vec<(&str, &str)> = paths.iter().map(|path| ("path_prefix", *path)).collect();
    assert!(fixture.run(&fixture.decompose_text(&scopes)).applied());
    let mut assignments: Vec<(String, String)> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let slice_id = fixture.slice_id("path_prefix", path);
        assert!(fixture
            .run(&fixture.assign_text(&slice_id, &workers[index], 4000, 2, &["evidence.read"], 1,))
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
        assignments.push((assignment_id, slice_id));
    }
    for (assignment_id, _) in &assignments {
        // 三个互不重叠的只读切片各自拿到一行 live 载体与一条未确认的下发消息。
        assert_eq!(
            carrier(db, assignment_id),
            ("prepared".into(), "leased".into())
        );
        assert_eq!(
            messages(db, assignment_id),
            vec![(journal_kind(), String::new())],
            "assign 必须经现有 mailbox 下发切片任务"
        );
    }
    let legacy_lane_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id IN (?1,?2,?3)",
            rusqlite::params![assignments[0].0, assignments[1].0, assignments[2].0],
            |row| row.get(0),
        )
        .expect("旧 lane 表可读");
    assert_eq!(
        legacy_lane_rows, 0,
        "DESIGN §5.1：同质并行只走 agent_lane_slots，不占用旧互斥表"
    );
    for (assignment_id, _) in &assignments {
        apply::mark_started(db, assignment_id).expect("开跑");
        assert_eq!(
            carrier(db, assignment_id),
            ("running".into(), "running".into())
        );
    }
    let (first_assignment, first_slice) = &assignments[0];
    let evidence = fixture.add_endpoint_evidence("/api/v1/a", 1);
    let report = types::WorkerReport {
        report_id: store::report_id_for(first_assignment, 1),
        assignment_id: first_assignment.clone(),
        slice_id: first_slice.clone(),
        worker_id: workers[0].clone(),
        evidence_refs: vec![evidence.clone()],
        output: serde_json::json!({
            "summary_zh": "发现 2 个只读端点",
            "findings": [],
            "coverage_notes": ["GET /api/v1/a"],
        }),
        consumed_tokens: 1200,
        consumed_requests: 1,
        terminal_hint: "completed".to_string(),
    };
    apply::record_worker_report(db, &fixture.lease, &report).expect("汇报落库");
    assert_eq!(
        store::load_assignment(db, first_assignment)
            .expect("a")
            .expect("有")
            .state,
        "reported"
    );
    let after_report = messages(db, first_assignment);
    assert_eq!(after_report.len(), 1);
    assert_eq!(after_report[0].0, journal_kind());
    assert!(!after_report[0].1.is_empty(), "汇报必须确认它的下发消息");
    // 汇报后载体仍存活：reject_retry 之后还要复用同一 attempt 行。
    assert_eq!(
        carrier(db, first_assignment),
        ("running".into(), "running".into())
    );
    let assignment = store::load_assignment(db, first_assignment)
        .expect("a")
        .expect("有");
    let slice = store::load_slice(db, first_slice).expect("s").expect("有");
    let stored = store::load_report(db, &report.report_id)
        .expect("r")
        .expect("有");
    let checks = machine::recompute(db, &fixture.task(), &assignment, &slice, &stored);
    assert!(fixture
        .run(&fixture.accept_text(
            first_assignment,
            first_slice,
            &report.report_id,
            &checks,
            &[&evidence]
        ))
        .applied());
    assert_eq!(
        carrier(db, first_assignment).0,
        "terminal",
        "accept 之后只关闭被裁决的那条载体"
    );
    for (assignment_id, _) in &assignments[1..] {
        assert_eq!(
            carrier(db, assignment_id),
            ("running".into(), "running".into())
        );
    }
    assert_eq!(
        pool::load_worker(db, &workers[0])
            .expect("w")
            .expect("有")
            .state,
        "idle",
        "裁决落地后 Worker 归还池子"
    );
}

fn journal_kind() -> String {
    super::delivery::MESSAGE_KIND.to_string()
}

#[test]
fn an_awaiting_directive_is_relayed_and_rejected_through_the_paid_tick_entry() {
    // P1-2：awaiting_coordinator 的引导只经 Root 付费 tick 的返回路径落库：
    // 合理引导转发给正在执行的 Worker，不合理引导四字段驳回；
    // 两者都不把任务打成 incomplete（DESIGN §8.6 / §5.5）。
    use super::directive_gate;
    let fixture = Fixture::new();
    let db = &fixture.connection;
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice = fixture.slice_id("path_prefix", "/api/v1/users");
    let worker = fixture.provision_workers(1).remove(0);
    assert!(fixture
        .run(&fixture.assign_text(&slice, &worker, 4000, 2, &["evidence.read"], 1,))
        .applied());

    let relayed_id = directive_id_for("wiring-relay");
    directive_gate::screen(
        db,
        &fixture.task(),
        &relayed_id,
        "",
        "优先看 /api/v1/users 的鉴权返回",
    )
    .expect("初筛");
    assert_eq!(
        store::load_gated_directive(db, &relayed_id)
            .expect("可读")
            .expect("有")
            .state,
        "awaiting_coordinator"
    );
    let relayed = crate::scanning::runtime_test_support::native_homogeneous_apply_after_paid_tick(
        db,
        &fixture.lease,
        &fixture.relay_text(&relayed_id, "relay", None, Some(&worker), None),
    )
    .expect("一轮同质决策");
    assert!(
        relayed.starts_with("root_homogeneous_applied:relay_directive:relay"),
        "{relayed}"
    );
    let directive = store::load_gated_directive(db, &relayed_id)
        .expect("可读")
        .expect("有");
    assert_eq!(directive.state, "relayed");
    assert_eq!(directive.target_worker_id, worker);
    assert!(!directive.user_feedback_zh.is_empty());

    let rejected_id = directive_id_for("wiring-reject");
    directive_gate::screen(db, &fixture.task(), &rejected_id, "", "把预算调到无限").expect("初筛");
    let rejected = crate::scanning::runtime_test_support::native_homogeneous_apply_after_paid_tick(
        db,
        &fixture.lease,
        &fixture.reject_directive_text(&rejected_id, super::reason::DIRECTIVE_BUDGET_CHANGE),
    )
    .expect("一轮同质决策");
    assert!(
        rejected.starts_with("root_homogeneous_applied:reject_directive"),
        "{rejected}"
    );
    assert!(directive_gate::rejected_feedback_ready(db, &rejected_id));
    assert_eq!(
        store::load_gated_directive(db, &rejected_id)
            .expect("可读")
            .expect("有")
            .reason_code,
        super::reason::DIRECTIVE_BUDGET_CHANGE
    );
    assert_eq!(
        fixture.task().state,
        "running",
        "驳回后任务继续，不落到任何终态"
    );
    assert_eq!(
        pool::load_worker(db, &worker)
            .expect("w")
            .expect("有")
            .state,
        "busy",
        "引导裁决不动 Worker"
    );
}

fn directive_id_for(tag: &str) -> String {
    format!(
        "dir-{}",
        &crate::agent_runtime::store::stable_hash(tag)[..24]
    )
}

#[test]
fn the_paid_tick_return_path_is_the_only_side_effect_entry() {
    // P0-1：tick 返回路径调用的就是 apply::run_coordinator_turn；这里用现网入口本身，
    // 不复制逻辑。账本摘要只带固定 step，非法文本按现网既有错误码族拒绝。
    let fixture = Fixture::new();
    let text = fixture.decompose_text(&[("path_prefix", "/api/v1/users")]);
    let applied = crate::scanning::runtime_test_support::native_homogeneous_apply_after_paid_tick(
        &fixture.connection,
        &fixture.lease,
        &text,
    )
    .expect("一轮同质决策");
    assert!(
        applied.starts_with("root_homogeneous_applied:decompose"),
        "{applied}"
    );
    assert_eq!(fixture.task().state, "decomposed");
    let summary = journal::ledger_summary("decompose").expect("摘要投影");
    assert_eq!(
        summary.as_json()["suggestions"]
            .as_array()
            .map(|items| items.len()),
        Some(1)
    );
    assert!(journal::parse("今天天气不错")
        .unwrap_err()
        .starts_with("root_homogeneous_"));
    assert!(journal::parse("{\"action\":\"decompose\"}").is_err());
}

/// 同一 slice_id 的 dedupe 键 → assignment 身份（与 `apply_assign` 同一算法）。
fn assignment_for(fixture: &Fixture, slice_id: &str) -> String {
    store::assignment_id_for(
        &fixture.root_run_id,
        &store::dedup_key_for(
            slice_id,
            super::test_fixture::TARGET,
            1,
            store::HOMOGENEOUS_TRIGGER_CODE,
        ),
    )
}

#[test]
fn the_worker_consumer_reports_dispatched_slices_without_any_manual_calls() {
    // Phase 1.1 第 1/5 条：assign 之后不手动 `mark_started`/`record_worker_report`，
    // 只跑消费者，切片就自动到 `reported`，并且能被主管直接 accept。
    let fixture = Fixture::new();
    let db = &fixture.connection;
    let workers = fixture.provision_workers(2);
    let paths = ["/api/v1/users", "/api/v1/orders"];
    let scopes: Vec<(&str, &str)> = paths.iter().map(|path| ("path_prefix", *path)).collect();
    assert!(fixture.run(&fixture.decompose_text(&scopes)).applied());
    let mut ids: Vec<(String, String)> = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let slice_id = fixture.slice_id("path_prefix", path);
        assert!(fixture
            .run(&fixture.assign_text(&slice_id, &workers[index], 4000, 2, &["evidence.read"], 1,))
            .applied());
        // Worker 唯一能读的只读材料：现网已观测证据，落在切片范围内。
        fixture.add_endpoint_evidence(path, 1);
        ids.push((assignment_for(&fixture, &slice_id), slice_id));
    }

    let consumed = super::worker::drain_for_lease(db, &fixture.lease).expect("消费者一轮");
    assert_eq!(consumed.len(), 2);
    for item in &consumed {
        assert_eq!(item.terminal_hint, "completed", "{item:?}");
        assert_eq!(item.reason_code, None, "{item:?}");
    }
    for (assignment_id, slice_id) in &ids {
        assert_eq!(
            store::load_assignment(db, assignment_id)
                .expect("a")
                .expect("有")
                .state,
            "reported"
        );
        assert_eq!(
            store::load_slice(db, slice_id)
                .expect("s")
                .expect("有")
                .state,
            "reported"
        );
        let (attempts, acked): (i64, String) = db
            .query_row(
                "SELECT delivery_attempts, acknowledged_at FROM agent_messages \
                 WHERE to_run_id=?1 AND kind=?2",
                rusqlite::params![format!("run-{assignment_id}"), journal_kind()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("下发消息可读");
        assert_eq!(attempts, 1, "必须走现网收方 API 投递，不是裸 UPDATE");
        assert!(!acked.is_empty(), "领取后必须确认");
        assert_eq!(
            carrier(db, assignment_id),
            ("running".into(), "running".into()),
            "汇报后载体保持存活，等裁决"
        );
    }

    let (assignment_id, slice_id) = (&ids[0].0, &ids[0].1);
    let assignment = store::load_assignment(db, assignment_id)
        .expect("a")
        .expect("有");
    let slice = store::load_slice(db, slice_id).expect("s").expect("有");
    let report = store::load_report(db, &store::report_id_for(assignment_id, assignment.attempt))
        .expect("r")
        .expect("有");
    let checks = machine::recompute(db, &fixture.task(), &assignment, &slice, &report);
    assert!(
        checks.scope_in_bounds
            && checks.evidence_refs_exist
            && checks.evidence_revision_fresh
            && checks.no_cross_slice_write
            && checks.tools_authorized
            && checks.budget_ok
            && checks.output_schema_ok,
        "自动汇报的切片必须机器全绿：{checks:?}"
    );
    let refs: Vec<&str> = report.evidence_refs.iter().map(String::as_str).collect();
    assert!(fixture
        .run(&fixture.accept_text(assignment_id, slice_id, &report.report_id, &checks, &refs))
        .applied());
    assert_eq!(carrier(db, assignment_id).0, "terminal");
    assert_eq!(
        carrier(db, &ids[1].0).0,
        "running",
        "只关闭被裁决的那条载体"
    );
}

#[test]
fn the_consumer_writes_a_failed_report_instead_of_stalling_in_progress() {
    // Phase 1.1 第 2 条：范围内无证据 / 租约里没有读侧工具，都必须写出可复核的汇报，
    // 状态走到 reported（不卡 running），失败原因只用 SPEC §1.1 已有枚举。
    use super::reason;
    let fixture = Fixture::new();
    let db = &fixture.connection;
    let workers = fixture.provision_workers(2);
    let scopes = [
        ("path_prefix", "/api/v1/none"),
        ("path_prefix", "/api/v1/writable"),
    ];
    assert!(fixture.run(&fixture.decompose_text(&scopes)).applied());
    let none_slice = fixture.slice_id("path_prefix", "/api/v1/none");
    let write_slice = fixture.slice_id("path_prefix", "/api/v1/writable");
    assert!(fixture
        .run(&fixture.assign_text(&none_slice, &workers[0], 4000, 2, &["evidence.read"], 1,))
        .applied());
    assert!(fixture
        .run(&fixture.assign_text(&write_slice, &workers[1], 4000, 2, &["mailbox.write"], 1,))
        .applied());

    let consumed = super::worker::drain_for_lease(db, &fixture.lease).expect("消费者一轮");
    assert_eq!(consumed.len(), 2);
    let codes: Vec<&'static str> = consumed
        .iter()
        .map(|item| {
            assert_eq!(item.terminal_hint, "failed", "{item:?}");
            item.reason_code.expect("失败路径必须带 reason_code")
        })
        .collect();
    assert!(codes.contains(&reason::EVIDENCE_MISSING), "{codes:?}");
    assert!(codes.contains(&reason::TOOL_NOT_AUTHORIZED), "{codes:?}");
    for assignment_id in [
        assignment_for(&fixture, &none_slice),
        assignment_for(&fixture, &write_slice),
    ] {
        assert_eq!(
            store::load_assignment(db, &assignment_id)
                .expect("a")
                .expect("有")
                .state,
            "reported",
            "失败也必须落汇报，不能停在 running"
        );
        assert_eq!(carrier(db, &assignment_id).0, "running");
    }

    // 主管按 Rust 复算的机器检查拒绝验收：证据缺失是既有的 evidence_missing，不是新码。
    let assignment_id = assignment_for(&fixture, &none_slice);
    let assignment = store::load_assignment(db, &assignment_id)
        .expect("a")
        .expect("有");
    let slice = store::load_slice(db, &none_slice).expect("s").expect("有");
    let report = store::load_report(
        db,
        &store::report_id_for(&assignment_id, assignment.attempt),
    )
    .expect("r")
    .expect("有");
    assert!(report.evidence_refs.is_empty(), "{report:?}");
    let checks = machine::recompute(db, &fixture.task(), &assignment, &slice, &report);
    assert!(!checks.evidence_refs_exist, "{checks:?}");
    let green = types::MachineChecks {
        scope_in_bounds: true,
        budget_ok: true,
        tools_authorized: true,
        output_schema_ok: true,
        evidence_refs_exist: true,
        evidence_revision_fresh: true,
        no_cross_slice_write: true,
    };
    assert_eq!(
        fixture.reason_of(&fixture.accept_text(
            &assignment_id,
            &none_slice,
            &report.report_id,
            &green,
            &[]
        )),
        reason::MACHINE_CHECK_FAILED,
        "谎报全绿必须被 Rust 复算覆盖（既有的 machine_check_failed）"
    );
}
