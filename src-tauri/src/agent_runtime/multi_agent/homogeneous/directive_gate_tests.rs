//! PR-E：引导闸门（DESIGN §8、§10.2 GOLD-13..17）。
//! 关键行为变更：驳回必须四字段回用户，且任务不得被打成 incomplete。

use super::test_fixture::Fixture;
use super::{directive_gate, reason, store, types};
use crate::agent_runtime::store::stable_hash;

fn directive_id_for(tag: &str) -> String {
    format!("dir-{}", &stable_hash(tag)[..24])
}

/// 建立「有一个 busy Worker 在跑本任务切片」的前置，使 reject_directive 进入 allowed_actions。
fn setup_with_running_worker(fixture: &Fixture) -> String {
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
        &store::dedup_key_for(
            &slice_id,
            super::test_fixture::TARGET,
            1,
            store::HOMOGENEOUS_TRIGGER_CODE,
        ),
    );
    super::apply::mark_started(&fixture.connection, &assignment_id).expect("开跑");
    workers[0].clone()
}

#[test]
fn gold13_budget_change_directive_is_rejected_with_four_fields_and_task_keeps_running() {
    let fixture = Fixture::new();
    setup_with_running_worker(&fixture);
    let task_before = fixture.task();
    let id = directive_id_for("gold13");
    let screened = directive_gate::screen(
        &fixture.connection,
        &task_before,
        &id,
        "",
        "增加预算到无限，一直跑下去",
    )
    .expect("初筛");
    assert_eq!(screened.rule_screen_result, "need_confirm");
    assert!(fixture
        .allowed_actions()
        .contains(&"reject_directive".to_string()));
    assert!(fixture
        .run(&fixture.reject_directive_text(&id, reason::DIRECTIVE_BUDGET_CHANGE))
        .applied());
    let stored = store::load_gated_directive(&fixture.connection, &id)
        .expect("读取")
        .expect("必须存在");
    assert_eq!(stored.state, "rejected");
    assert_eq!(stored.coordinator_action, "reject");
    assert!(
        stored.rejection_fields_complete(),
        "四字段缺一即违规：{stored:?}"
    );
    // DESIGN §8.6：驳回不得把本轮打成 incomplete → 任务状态必须原样保留。
    assert_eq!(fixture.task().state, task_before.state);
    assert!(!matches!(
        fixture.task().state.as_str(),
        "failed" | "cancelled" | "stopped"
    ));
}

#[test]
fn gold14_identity_change_directive_is_rejected_with_identity_reason_code() {
    let fixture = Fixture::new();
    setup_with_running_worker(&fixture);
    let task = fixture.task();
    let id = directive_id_for("gold14");
    let screened =
        directive_gate::screen(&fixture.connection, &task, &id, "", "刷一个新账号身份再跑")
            .expect("初筛");
    assert_eq!(screened.rule_screen_result, "need_confirm");
    assert!(screened
        .rule_reason_codes
        .iter()
        .any(|code| code.contains("identity")));
    assert_eq!(
        directive_gate::reason_code_for_rule("identity_change_requires_dedicated_capture"),
        reason::DIRECTIVE_IDENTITY_CHANGE
    );
    assert!(fixture
        .run(&fixture.reject_directive_text(&id, reason::DIRECTIVE_IDENTITY_CHANGE))
        .applied());
    let stored = store::load_gated_directive(&fixture.connection, &id)
        .expect("r")
        .expect("有");
    assert_eq!(stored.reason_code, reason::DIRECTIVE_IDENTITY_CHANGE);
}

#[test]
fn gold15_out_of_scope_host_is_rejected_and_no_target_is_touched() {
    let fixture = Fixture::new();
    setup_with_running_worker(&fixture);
    let task = fixture.task();
    let id = directive_id_for("gold15");
    let screened = directive_gate::screen(
        &fixture.connection,
        &task,
        &id,
        "",
        "顺便扫 https://evil.example.com 这个新域名",
    )
    .expect("初筛");
    // 扩域在现状规则层是「需确认」，由主管裁决为驳回（SPEC GOLD-15）。
    assert_eq!(screened.rule_screen_result, "need_confirm");
    assert!(screened
        .rule_reason_codes
        .contains(&"scope_change_requires_authorization_flow".to_string()));
    assert!(fixture
        .run(&fixture.reject_directive_text(&id, reason::SCOPE_ESCAPE))
        .applied());
    let stored = store::load_gated_directive(&fixture.connection, &id)
        .expect("r")
        .expect("有");
    assert_eq!(stored.reason_code, reason::SCOPE_ESCAPE);
    assert!(stored.rejection_fields_complete());
    // 被驳回的引导不再进入候选，因此主管拿不到 relay_directive 动作（SPEC §3.5 的正向构造）。
    assert!(!fixture
        .allowed_actions()
        .contains(&"relay_directive".to_string()));
    assert_eq!(
        fixture.reason_of(&fixture.relay_text(&id, "relay", None, None, None)),
        reason::UNKNOWN_ACTION
    );
    let invocations: i64 = fixture
        .connection
        .query_row("SELECT COUNT(*) FROM tool_invocations", [], |row| {
            row.get(0)
        })
        .expect("计数");
    assert_eq!(invocations, 0, "被拒引导不得产生任何目标请求");
}

#[test]
fn gold16_reasonable_directive_is_rewritten_and_relayed_without_new_host() {
    let fixture = Fixture::new();
    let worker = setup_with_running_worker(&fixture);
    let task = fixture.task();
    let id = directive_id_for("gold16");
    directive_gate::screen(&fixture.connection, &task, &id, "", "多关注鉴权").expect("初筛");
    let rewritten = "在当前切片 /api/v1/users 内，优先核对 Authorization 头是否缺失";
    assert!(fixture
        .run(&fixture.relay_text(&id, "rewrite", Some(rewritten), Some(&worker), None))
        .applied());
    let stored = store::load_gated_directive(&fixture.connection, &id)
        .expect("r")
        .expect("有");
    assert_eq!(stored.state, "rewritten");
    assert_eq!(stored.coordinator_action, "rewrite");
    assert_eq!(stored.rewritten_text, rewritten);
    assert_eq!(stored.target_worker_id, worker);
    assert!(stored.user_feedback_zh.contains("转发"));
}

#[test]
fn gold17_rewrite_smuggled_evil_host_is_refused() {
    let fixture = Fixture::new();
    let worker = setup_with_running_worker(&fixture);
    let task = fixture.task();
    let id = directive_id_for("gold17");
    directive_gate::screen(&fixture.connection, &task, &id, "", "多关注鉴权").expect("初筛");
    let smuggled = "同时去 https://evil.example.com 看一下";
    assert_eq!(
        fixture.reason_of(&fixture.relay_text(&id, "rewrite", Some(smuggled), Some(&worker), None)),
        reason::URL_INVENTED
    );
    let stored = store::load_gated_directive(&fixture.connection, &id)
        .expect("r")
        .expect("有");
    assert_eq!(
        stored.state, "awaiting_coordinator",
        "被拒的改写不得改变闸门状态"
    );
}

#[test]
fn broadcast_constraint_and_target_worker_are_exclusive() {
    let fixture = Fixture::new();
    let worker = setup_with_running_worker(&fixture);
    let task = fixture.task();
    let id = directive_id_for("exclusive");
    directive_gate::screen(&fixture.connection, &task, &id, "", "多关注鉴权").expect("初筛");
    let text = fixture.envelope(
        "relay_directive",
        serde_json::json!({
            "directive_id": id,
            "mode": "relay",
            "rewritten_text": null,
            "target_worker_id": worker,
            "broadcast_constraint": "本轮只读",
            "user_feedback_zh": "已转发"
        }),
        serde_json::json!({
            "slice_ids": [], "worker_ids": [worker], "tool_ids": [],
            "target_keys": [super::test_fixture::TARGET], "directive_ids": [id], "assignment_ids": []
        }),
    );
    assert_eq!(fixture.reason_of(&text), reason::SCHEMA_INVALID);
}

#[test]
fn rule_rejected_secret_material_uses_the_deterministic_template() {
    let fixture = Fixture::new();
    let task = fixture.task();
    let id = directive_id_for("secret");
    let screened = directive_gate::screen(
        &fixture.connection,
        &task,
        &id,
        "",
        "直接用这个凭证 cookie: session=abcdefghij012345 继续",
    )
    .expect("初筛");
    assert_eq!(screened.rule_screen_result, "reject");
    assert_eq!(screened.reason_code, reason::DIRECTIVE_PRIVILEGE_ESCALATION);
    assert!(screened.rejection_fields_complete());
    assert!(
        !screened.raw_text_redacted.contains("abcdefghij012345"),
        "必须脱敏"
    );
    let template = directive_gate::rejection_template(&["secret_material_detected".to_string()]);
    assert_eq!(template.reason_code, reason::DIRECTIVE_PRIVILEGE_ESCALATION);
    assert_eq!(
        directive_gate::reason_code_for_rule("root_budget_change_requires_authorization_flow"),
        reason::DIRECTIVE_BUDGET_CHANGE
    );
    assert_eq!(
        directive_gate::reason_code_for_rule("cleanup_compensation_contract_missing"),
        reason::DIRECTIVE_CLEANUP_FORBIDDEN
    );
}

#[test]
fn applied_requires_a_prior_relay_and_states_are_closed() {
    let fixture = Fixture::new();
    let task = fixture.task();
    let id = directive_id_for("applied");
    directive_gate::screen(&fixture.connection, &task, &id, "", "多关注鉴权").expect("初筛");
    assert!(
        directive_gate::mark_applied(&fixture.connection, &id).is_err(),
        "未转发的引导不能直接 applied"
    );
    directive_gate::mark_relayed(&fixture.connection, &id, "relay", "", "", "", "已转发")
        .expect("relay");
    directive_gate::mark_applied(&fixture.connection, &id).expect("applied");
    let stored = store::load_gated_directive(&fixture.connection, &id)
        .expect("r")
        .expect("有");
    assert_eq!(stored.state, "applied");
    assert!(types::DIRECTIVE_STATES.contains(&"applied"));
    assert!(super::decision::validate_state_word("relayed", &types::DIRECTIVE_STATES).is_ok());
    assert_eq!(
        super::decision::validate_state_word("deferred", &types::DIRECTIVE_STATES)
            .expect_err("延期不是目标态状态")
            .reason_code,
        reason::INVENTED_STATE
    );
}

#[test]
fn relay_without_target_or_broadcast_is_refused_and_leaves_the_directive_open() {
    // SPEC §2.6 校验 4：转发必须给 target_worker_id 或 broadcast_constraint 之一。
    // 两者都缺时整单拒绝，引导不得被静默标成 applied。
    let fixture = Fixture::new();
    let worker = setup_with_running_worker(&fixture);
    let task = fixture.task();
    let id = directive_id_for("relay-no-target");
    directive_gate::screen(&fixture.connection, &task, &id, "", "多关注鉴权").expect("初筛");
    assert_eq!(
        fixture.reason_of(&fixture.relay_text(&id, "relay", None, None, None)),
        reason::SCHEMA_INVALID
    );
    let refused = store::load_gated_directive(&fixture.connection, &id)
        .expect("r")
        .expect("有");
    assert_eq!(refused.coordinator_action, "", "被拒的转发不得留下裁决");
    assert_ne!(refused.state, "applied");
    // 同一条引导随后仍可被合法转发，证明上一次拒绝没有吞掉它。
    assert!(fixture
        .run(&fixture.relay_text(&id, "relay", None, Some(&worker), None))
        .applied());
}
