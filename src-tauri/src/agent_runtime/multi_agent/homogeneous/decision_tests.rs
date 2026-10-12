//! PR-A：严格解析层单测（DESIGN §11 PR-A 验收「严格解析绿」）。
//! 覆盖 GOLD-18/19/20/23/24/25 与 SPEC §1.2 的非法例。

use super::allowlist::{collect_payload_ids, verify, AllowlistSnapshot};
use super::decision::{parse_decision_envelope, parse_payload};
use super::reason;
use serde_json::json;

fn envelope(payload: serde_json::Value, action: &str) -> String {
    json!({
        "decision_id": "dec-1111111111111111",
        "action": action,
        "parent_task_id": "tsk-1122334455667788",
        "payload": payload,
        "allowlist_refs": {
            "slice_ids": [], "worker_ids": [], "tool_ids": [],
            "target_keys": [], "directive_ids": [], "assignment_ids": []
        }
    })
    .to_string()
}

fn decompose_payload(profile: &str, first: &str, second: &str) -> serde_json::Value {
    json!({
        "slices": [
            {"slice_id":"slc-a1b2c3d4e5f67890","scope_kind":"path_prefix","scope_value":first,
             "lane":"read_only_analysis","evidence_revision_base":3,
             "worker_capability_profile_id":profile,"max_retries":2},
            {"slice_id":"slc-b2c3d4e5f6789012","scope_kind":"path_prefix","scope_value":second,
             "lane":"read_only_analysis","evidence_revision_base":3,
             "worker_capability_profile_id":profile,"max_retries":2}
        ],
        "decomposition_basis_zh": "按已发现 API 路径前缀二分"
    })
}

#[test]
fn gold18_markdown_fence_is_stripped_once_then_strict() {
    let valid = envelope(
        decompose_payload("homogeneous_v1", "/api/v1/users", "/api/v1/orders"),
        "decompose",
    );
    let fenced = format!("```json\n{valid}\n```");
    let decision = parse_decision_envelope(&fenced).expect("剥一层围栏后必须合法");
    assert_eq!(decision.action, "decompose");
    // 围栏里装的仍是坏 schema：剥完必须严格拒绝，不得宽容。
    let bad_fence =
        "```json\n{\"decision_id\":\"dec-1111111111111111\",\"action\":\"decompose\"}\n```";
    let error = parse_decision_envelope(bad_fence).unwrap_err();
    assert_eq!(error.reason_code, reason::MISSING_REQUIRED_FIELD);
    // 围栏里根本没有 JSON。
    let no_json = "```json\n这里只有一个建议，没有对象\n```";
    assert_eq!(
        parse_decision_envelope(no_json).unwrap_err().reason_code,
        reason::MARKDOWN_WRAPPED_JSON
    );
}

#[test]
fn gold19_free_text_instead_of_json() {
    let error =
        parse_decision_envelope("我觉得应该先并行扫这三个路径，注意预算够不够。").unwrap_err();
    assert_eq!(error.reason_code, reason::FREE_TEXT_INSTEAD_OF_JSON);
    assert_eq!(
        parse_decision_envelope("   ").unwrap_err().reason_code,
        reason::FREE_TEXT_INSTEAD_OF_JSON
    );
}

#[test]
fn gold20_two_actions_in_one_output_is_refused() {
    // 同一个对象里出现两次 action（一次一事被破坏）。
    let duplicated = "{\"decision_id\":\"dec-1111111111111111\",\"action\":\"assign\",\"parent_task_id\":\"tsk-1122334455667788\",\"action\":\"accept\",\"payload\":{},\"allowlist_refs\":{\"slice_ids\":[],\"worker_ids\":[],\"tool_ids\":[],\"target_keys\":[],\"directive_ids\":[],\"assignment_ids\":[]}}";
    assert_eq!(
        parse_decision_envelope(duplicated).unwrap_err().reason_code,
        reason::SCHEMA_INVALID
    );
    // 两个 JSON 对象也一律拒绝。
    let one = envelope(json!({"slices":[]}), "decompose");
    let two_objects = format!("{one} {one}");
    let error = parse_decision_envelope(&two_objects).unwrap_err();
    assert_eq!(error.reason_code, reason::SCHEMA_INVALID);
    assert_eq!(error.detail, "一次只输出一个 JSON 对象");
}

#[test]
fn gold24_extra_field_is_refused() {
    let text = json!({
        "decision_id":"dec-1111111111111111",
        "action":"stop",
        "parent_task_id":"tsk-1122334455667788",
        "payload":{},
        "priority":1,
        "allowlist_refs":{"slice_ids":[],"worker_ids":[],"tool_ids":[],"target_keys":[],"directive_ids":[],"assignment_ids":[]}
    })
    .to_string();
    assert_eq!(
        parse_decision_envelope(&text).unwrap_err().reason_code,
        reason::EXTRA_FIELD
    );
}

#[test]
fn gold25_missing_decision_id_is_refused() {
    let text = json!({
        "action":"stop",
        "parent_task_id":"tsk-1122334455667788",
        "payload":{},
        "allowlist_refs":{"slice_ids":[],"worker_ids":[],"tool_ids":[],"target_keys":[],"directive_ids":[],"assignment_ids":[]}
    })
    .to_string();
    assert_eq!(
        parse_decision_envelope(&text).unwrap_err().reason_code,
        reason::MISSING_REQUIRED_FIELD
    );
}

#[test]
fn envelope_rejects_unknown_action_and_bad_id_shape() {
    let text = envelope(json!({}), "escalate");
    assert_eq!(
        parse_decision_envelope(&text).unwrap_err().reason_code,
        reason::UNKNOWN_ACTION
    );
    let bad_id = text.replace("dec-1111111111111111", "dec-ZZZZ");
    assert_eq!(
        parse_decision_envelope(&bad_id).unwrap_err().reason_code,
        reason::SCHEMA_INVALID
    );
}

#[test]
fn gold03_invented_role_is_refused_at_payload_level() {
    let text = envelope(
        decompose_payload("AuthorizationExpert", "/api/v1/users", "/api/v1/orders"),
        "decompose",
    );
    let decision = parse_decision_envelope(&text).expect("信封合法");
    let error = parse_payload(&decision).unwrap_err();
    assert_eq!(error.reason_code, reason::INVENTED_ROLE);
}

#[test]
fn decompose_rejects_out_of_enum_scope_kind_and_empty_slices() {
    let mut body = decompose_payload("homogeneous_v1", "/a", "/b");
    body["slices"][0]["scope_kind"] = json!("regex");
    let decision = parse_decision_envelope(&envelope(body.clone(), "decompose")).unwrap();
    assert_eq!(
        parse_payload(&decision).unwrap_err().reason_code,
        reason::SCHEMA_INVALID
    );
    let empty = envelope(json!({"slices":[]}), "decompose");
    let decision = parse_decision_envelope(&empty).unwrap();
    assert_eq!(
        parse_payload(&decision).unwrap_err().reason_code,
        reason::ZERO_INTENT_ILLEGAL
    );
}

#[test]
fn gold23_allowlist_refs_must_cover_payload_and_be_within_candidates() {
    // SPEC §2.2：assign 引用既有切片与 Worker，refs 必须双向一致。
    let body = json!({
        "slice_id": "slc-a1b2c3d4e5f67890",
        "worker_id": "wrk-0f1e2d3c4b5a6978",
        "reserved_tokens": 8000,
        "reserved_requests": 4,
        "capability_lease": ["evidence.read", "snapshot.read", "mailbox.write"],
        "lane": "read_only_analysis",
        "attempt": 1
    });
    let refs = |slice_ids: &[&str]| {
        json!({
            "slice_ids": slice_ids, "worker_ids": ["wrk-0f1e2d3c4b5a6978"],
            "tool_ids": ["evidence.read", "snapshot.read", "mailbox.write"],
            "target_keys": ["https://app.example.com"], "directive_ids": [], "assignment_ids": []
        })
    };
    let text = json!({
        "decision_id": "dec-2222222222222222",
        "action": "assign",
        "parent_task_id": "tsk-1122334455667788",
        "payload": body,
        "allowlist_refs": refs(&["slc-bbbbbbbbbbbbbbbb"])
    })
    .to_string();
    let decision = parse_decision_envelope(&text).unwrap();
    let declared = collect_payload_ids(&parse_payload(&decision).unwrap());
    let snapshot = AllowlistSnapshot {
        slice_ids: vec!["slc-a1b2c3d4e5f67890".into()],
        worker_ids: vec!["wrk-0f1e2d3c4b5a6978".into()],
        tool_ids: vec![
            "evidence.read".into(),
            "snapshot.read".into(),
            "mailbox.write".into(),
        ],
        target_keys: vec!["https://app.example.com".into()],
        ..Default::default()
    };
    // payload 用的切片没被 refs 声明，且 refs 声明了候选外的 id。
    assert_eq!(
        verify(&snapshot, &decision.allowlist_refs, &declared)
            .unwrap_err()
            .reason_code,
        reason::ALLOWLIST_VIOLATION
    );
    // 完全一致时通过。
    let fixed = json!({
        "decision_id": "dec-2222222222222222",
        "action": "assign",
        "parent_task_id": "tsk-1122334455667788",
        "payload": body,
        "allowlist_refs": refs(&["slc-a1b2c3d4e5f67890"])
    })
    .to_string();
    let decision = parse_decision_envelope(&fixed).unwrap();
    let declared = collect_payload_ids(&parse_payload(&decision).unwrap());
    verify(&snapshot, &decision.allowlist_refs, &declared).expect("一致");
}

#[test]
fn gold07_invented_target_key_is_url_invented() {
    let snapshot = AllowlistSnapshot {
        target_keys: vec!["https://app.example.com".into()],
        ..Default::default()
    };
    let mut decision = parse_decision_envelope(&envelope(json!({}), "decompose")).unwrap();
    decision.allowlist_refs.target_keys = vec!["https://evil.example.com".into()];
    let declared = collect_payload_ids(&parse_payload_with(&decision));
    let error = verify(&snapshot, &decision.allowlist_refs, &declared).unwrap_err();
    assert_eq!(error.reason_code, reason::URL_INVENTED);
}

/// stop 的空 payload 只用于 refs 检查，不参与动作校验。
fn parse_payload_with(
    _decision: &super::types::CoordinatorDecision,
) -> super::decision::DecisionPayload {
    use super::decision::DecisionPayload;
    use super::types::StopPayload;
    DecisionPayload::Stop(StopPayload {
        stop_reason: "goal_reached".into(),
        accepted_slice_ids: vec![],
        rejected_slice_ids: vec![],
        summary_zh: "无".into(),
        publish_partial: false,
    })
}

#[test]
fn machine_checks_and_verdict_pairs_are_strict() {
    let accept_body = json!({
        "assignment_id":"asg-9988776655443322",
        "slice_id":"slc-a1b2c3d4e5f67890",
        "report_id":"rpt-aabbccddeeff0011",
        "verdict":"accept",
        "machine_checks":{
            "scope_in_bounds":false,"budget_ok":true,"tools_authorized":true,
            "output_schema_ok":true,"evidence_refs_exist":true,
            "evidence_revision_fresh":true,"no_cross_slice_write":true
        },
        "model_judgement":{"adequate":true,"question_zh":"?","answer_zh":"?"},
        "reason_codes":["goal_reached"],
        "evidence_refs":["evd-11112222"]
    });
    let decision = parse_decision_envelope(&envelope(accept_body.clone(), "accept")).unwrap();
    // 机器检查有 false 却 accept：SPEC §1.6/§2.3 非法例 1。
    assert_eq!(
        parse_payload(&decision).unwrap_err().reason_code,
        reason::SCHEMA_INVALID
    );
    // reject_retry 用 verdict=accept：SPEC §2.4 非法例 2。
    let mut retry_body = accept_body;
    retry_body["machine_checks"]["scope_in_bounds"] = json!(true);
    retry_body["verdict"] = json!("accept");
    retry_body["model_judgement"] = json!(null);
    retry_body["retry_hint_zh"] = json!("请补证据");
    let decision = parse_decision_envelope(&envelope(retry_body.clone(), "reject_retry")).unwrap();
    assert_eq!(
        parse_payload(&decision).unwrap_err().reason_code,
        reason::SCHEMA_INVALID
    );
    // reject_retry 机器全绿时 adequate 必须为 false（SPEC §2.4 校验 3）。
    retry_body["verdict"] = json!("retry");
    retry_body["model_judgement"] = json!({"adequate":true,"question_zh":"?","answer_zh":"?"});
    let decision = parse_decision_envelope(&envelope(retry_body, "reject_retry")).unwrap();
    assert_eq!(
        parse_payload(&decision).unwrap_err().reason_code,
        reason::SCHEMA_INVALID
    );
}

#[test]
fn reject_directive_requires_all_four_fields() {
    let base = |code: &str, reason_zh: &str, suggestion: &str, feedback: &str| {
        json!({
            "directive_id":"dir-feedfacecafebeef",
            "reason_code":code,
            "reason_zh":reason_zh,
            "suggestion_zh":suggestion,
            "user_feedback_zh":feedback
        })
    };
    let good = parse_decision_envelope(&envelope(
        base(
            "directive_budget_change",
            "预算不可改",
            "请给范围内的关注点",
            "未采纳",
        ),
        "reject_directive",
    ))
    .unwrap();
    parse_payload(&good).expect("四字段齐备");

    let missing = parse_decision_envelope(&envelope(
        base(
            "directive_budget_change",
            "",
            "请给范围内的关注点",
            "未采纳",
        ),
        "reject_directive",
    ))
    .unwrap();
    assert_eq!(
        parse_payload(&missing).unwrap_err().reason_code,
        reason::MISSING_REQUIRED_FIELD
    );

    let out_of_enum = parse_decision_envelope(&envelope(
        base("随便说说", "预算不可改", "请给范围内的关注点", "未采纳"),
        "reject_directive",
    ))
    .unwrap();
    assert_eq!(
        parse_payload(&out_of_enum).unwrap_err().reason_code,
        reason::SCHEMA_INVALID
    );

    let not_directive_family = parse_decision_envelope(&envelope(
        base(
            "budget_exceeded",
            "预算不可改",
            "请给范围内的关注点",
            "未采纳",
        ),
        "reject_directive",
    ))
    .unwrap();
    assert_eq!(
        parse_payload(&not_directive_family)
            .unwrap_err()
            .reason_code,
        reason::SCHEMA_INVALID
    );
}

#[test]
fn relay_directive_target_and_broadcast_are_exclusive() {
    let body = json!({
        "directive_id":"dir-feedfacecafebeef",
        "mode":"rewrite",
        "rewritten_text":"在当前切片内优先核对鉴权头",
        "target_worker_id":"wrk-0f1e2d3c4b5a6978",
        "broadcast_constraint":"本轮只读",
        "user_feedback_zh":"已转发"
    });
    let decision = parse_decision_envelope(&envelope(body, "relay_directive")).unwrap();
    assert_eq!(
        parse_payload(&decision).unwrap_err().reason_code,
        reason::SCHEMA_INVALID
    );
    // rewrite 缺 rewritten_text → 必填字段缺失。
    let body = json!({
        "directive_id":"dir-feedfacecafebeef","mode":"rewrite","rewritten_text":null,
        "target_worker_id":"wrk-0f1e2d3c4b5a6978","broadcast_constraint":null,
        "user_feedback_zh":"已转发"
    });
    let decision = parse_decision_envelope(&envelope(body, "relay_directive")).unwrap();
    assert_eq!(
        parse_payload(&decision).unwrap_err().reason_code,
        reason::MISSING_REQUIRED_FIELD
    );
}

#[test]
fn state_words_outside_the_closed_set_are_invented_states() {
    use super::decision::validate_state_word;
    use super::types::SLICE_STATES;
    validate_state_word("accepted", &SLICE_STATES).expect("合法状态");
    let error = validate_state_word("thinking", &SLICE_STATES).unwrap_err();
    assert_eq!(error.reason_code, reason::INVENTED_STATE);
}
