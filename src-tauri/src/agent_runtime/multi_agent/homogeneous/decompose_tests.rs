//! PR-B：分解与确定性降级（DESIGN §10.2 GOLD-01/02/19 + DESIGN §7）。

use super::decompose::{
    deterministic_decompose, drop_overlapping, normalize_path_prefix, scopes_overlap, slice_id_for,
    DecomposeOutcome, Scope,
};
use super::test_fixture::Fixture;
use super::{reason, store, types};

#[test]
fn gold01_three_path_slices_are_accepted_and_persisted() {
    let fixture = Fixture::new();
    let outcome = fixture.run(&fixture.decompose_text(&[
        ("path_prefix", "/api/v1/users"),
        ("path_prefix", "/api/v1/orders"),
        ("path_prefix", "/api/v1/payments"),
    ]));
    assert!(outcome.applied(), "{outcome:?}");
    let slices = store::load_slices(&fixture.connection, &fixture.task().task_id).expect("切片");
    assert_eq!(slices.len(), 3);
    assert!(slices.iter().all(|slice| slice.state == "pending"));
    assert_eq!(fixture.task().state, "decomposed");
}

#[test]
fn gold02_containing_paths_reject_the_whole_proposal_then_replenish_deterministically() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.reason_of(
            &fixture.decompose_text(&[("path_prefix", "/api"), ("path_prefix", "/api/v1/users")])
        ),
        reason::SLICE_OVERLAP
    );
    let count: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM agent_task_slices WHERE decomposition_model='coordinator'",
            [],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(count, 0, "被拒主管 proposal 必须一行都不落库");
    let slices = store::load_slices(&fixture.connection, &fixture.task().task_id).unwrap();
    assert_eq!(slices.len(), 1, "§J 空队列即时补一条合法确定性方向");
    assert_eq!(slices[0].scope_kind, "host");
    assert_eq!(slices[0].decomposition_model, "deterministic");
}

#[test]
fn segment_boundary_overlap_rules() {
    assert!(scopes_overlap(
        &Scope::new("path_prefix", "/api"),
        &Scope::new("path_prefix", "/api/v1")
    ));
    // 收紧：非段边界的裸前缀相似不算包含（比 SPEC 的 starts_with 更严，只会多拒）。
    assert!(!scopes_overlap(
        &Scope::new("path_prefix", "/api/v1"),
        &Scope::new("path_prefix", "/api/v1user")
    ));
    assert!(scopes_overlap(
        &Scope::new("host", "a.example"),
        &Scope::new("host", "a.example")
    ));
    assert!(!scopes_overlap(
        &Scope::new("host", "a.example"),
        &Scope::new("path_prefix", "/a")
    ));
    assert!(scopes_overlap(
        &Scope::new("unknown_kind", "x"),
        &Scope::new("unknown_kind", "y")
    ));
}

#[test]
fn normalization_and_drop_overlapping_are_deterministic() {
    assert_eq!(normalize_path_prefix("API/v1/users?x=1"), "/API/v1/users");
    assert_eq!(normalize_path_prefix("/api/v1/orders/"), "/api/v1/orders");
    assert_eq!(normalize_path_prefix("//api///v1//"), "/api/v1");
    assert_eq!(normalize_path_prefix(""), "/");
    let scopes = vec![
        Scope::new("path_prefix", "/api"),
        Scope::new("path_prefix", "/api/v1/users"),
        Scope::new("path_prefix", "/api/v1/orders"),
    ];
    let kept = drop_overlapping(&scopes);
    assert_eq!(
        kept.iter()
            .map(|scope| scope.value.clone())
            .collect::<Vec<String>>(),
        vec!["/api/v1/orders".to_string(), "/api/v1/users".to_string()]
    );
    // 同输入两次调用结果必须一致（回放要求）。
    assert_eq!(drop_overlapping(&scopes), kept);
}

#[test]
fn gold19_free_text_empty_first_turn_immediately_replenishes_deterministically() {
    let fixture = Fixture::new();
    for path in ["/api/v1/users", "/api/v1/orders", "/api/v1/payments"] {
        fixture.add_endpoint_evidence(path, 1);
    }
    let first = fixture.run("我觉得应该先把三个路径并行扫一遍，预算应该够用。");
    assert!(
        matches!(first, super::apply::TurnOutcome::Refused { reason_code, .. } if reason_code == reason::FREE_TEXT_INSTEAD_OF_JSON),
        "{first:?}"
    );
    let second = fixture.run("再考虑一下，也许可以先看用户接口。");
    assert!(
        matches!(second, super::apply::TurnOutcome::Idle(_)),
        "{second:?}"
    );
    let task = fixture.task();
    assert!(task.degraded, "降级必须显式标注，不得假装主管分解成功");
    assert_eq!(task.degrade_reason_code, reason::DEGRADED_PATH_USED);
    assert_eq!(task.state, "decomposed");
    // Phase 1 降级只串行：只读 lane 配额被压回 1。
    assert_eq!(
        store::quota_limit(
            &fixture.connection,
            &fixture.root_run_id,
            "read_only_analysis"
        )
        .expect("配额"),
        1
    );
    let slices = store::load_slices(&fixture.connection, &task.task_id).expect("切片");
    assert_eq!(
        slices.len(),
        1,
        "§J 首轮非法0意图仅补第一条，不能继续付费等第二次失败"
    );
    assert!(slices
        .iter()
        .all(|slice| slice.decomposition_model == "deterministic"));
}

#[test]
fn degradation_falls_through_to_pause_when_there_is_nothing_to_split() {
    let fixture = Fixture::new();
    let task = fixture.task();
    match deterministic_decompose(&fixture.connection, &task).expect("降级") {
        DecomposeOutcome::PauseForUser(message) => {
            assert!(message.contains("path"));
        }
        other => panic!("无候选时必须 PauseForUser，实际 {other:?}"),
    }
    assert_eq!(fixture.task().state, "created", "暂停不是新状态");
    assert!(!fixture.task().degraded);
}

#[test]
fn slice_ids_are_derived_from_scope_not_from_the_model() {
    let fixture = Fixture::new();
    let task = fixture.task();
    let expected = slice_id_for(&task.task_id, "path_prefix", "/api/v1/users", 1);
    // 模型给一个不相干的 slice_id：必须整单拒绝，且落库 id 只能由 scope 派生。
    let text = fixture.decompose_text(&[("path_prefix", "/api/v1/users")]);
    let tampered = text.replace(
        &slice_id_for(&task.task_id, "path_prefix", "/api/v1/users", 1),
        "slc-0000000000000000",
    );
    assert_ne!(tampered, text);
    assert_eq!(fixture.reason_of(&tampered), reason::SCHEMA_INVALID);
    assert_eq!(expected, fixture.slice_id("path_prefix", "/api/v1/users"));
}

#[test]
fn target_touching_slices_cannot_be_assigned_in_phase_one() {
    let fixture = Fixture::new();
    let task = fixture.task();
    let draft = types::SliceDraft {
        slice_id: slice_id_for(&task.task_id, "path_prefix", "/admin", 1),
        scope_kind: "path_prefix".into(),
        scope_value: "/admin".into(),
        lane: "target_touching".into(),
        evidence_revision_base: 1,
        worker_capability_profile_id: types::PROFILE_HOMOGENEOUS_V1.into(),
        max_retries: 2,
    };
    store::insert_slices(&fixture.connection, &task.task_id, &[draft], "coordinator")
        .expect("写入切片");
    store::set_task_state(&fixture.connection, &task.task_id, "decomposed", 1).expect("推进任务");
    let workers = fixture.provision_workers(1);
    let text = fixture.assign_text(
        &fixture.slice_id("path_prefix", "/admin"),
        &workers[0],
        100,
        1,
        &["evidence.read"],
        1,
    );
    assert_eq!(fixture.reason_of(&text), reason::LANE_QUOTA_EXCEEDED);
}
