//! PR-G：黄金集全量映射（DESIGN §10.2、SPEC §8）。
//!
//! 每个 GOLD 都是一个「模型输出夹具 → 真实 pipeline → 断言 reason_code / 落库」的测试，
//! 不依赖网络与真模型；关键 18 条必须 100%。

use super::allowlist::AllowlistSnapshot;
use super::apply::TurnOutcome;
use super::test_fixture::{Fixture, TARGET};
use super::{gate, reason, store, types};
use serde_json::json;

#[test]
fn gold03_invented_role_is_refused_through_the_pipeline() {
    let fixture = Fixture::new();
    let task = fixture.task();
    assert_eq!(fixture.reason_of(&fixture.envelope(
        "decompose",
        json!({
            "slices": [{
                "slice_id": super::decompose::slice_id_for(&task.task_id, "path_prefix", "/admin", 1),
                "scope_kind": "path_prefix",
                "scope_value": "/admin",
                "lane": "read_only_analysis",
                "evidence_revision_base": 1,
                "worker_capability_profile_id": "AuthorizationExpert",
                "max_retries": 1
            }],
            "decomposition_basis_zh": "越权画像"
        }),
        json!({
            "slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [TARGET], "directive_ids": [], "assignment_ids": []
        }),
    )), reason::INVENTED_ROLE);
    let count: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM agent_task_slices WHERE decomposition_model='coordinator'",
            [],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(
        count, 0,
        "拒绝invented_role原单；独立确定性降级只能使用homogeneous_v1"
    );
    let slices = store::load_slices(&fixture.connection, &task.task_id).unwrap();
    assert_eq!(slices.len(), 1);
    assert_eq!(
        slices[0].worker_capability_profile_id,
        types::PROFILE_HOMOGENEOUS_V1
    );
}

#[test]
fn gold07_invented_target_key_is_refused() {
    let fixture = Fixture::new();
    let text = fixture.envelope(
        "decompose",
        json!({
            "slices": [{
                "slice_id": super::decompose::slice_id_for(&fixture.task().task_id, "path_prefix", "/api/v1/users", 1),
                "scope_kind": "path_prefix",
                "scope_value": "/api/v1/users",
                "lane": "read_only_analysis",
                "evidence_revision_base": 1,
                "worker_capability_profile_id": types::PROFILE_HOMOGENEOUS_V1,
                "max_retries": 2
            }],
            "decomposition_basis_zh": "编造目标"
        }),
        json!({
            "slice_ids": [], "worker_ids": [], "tool_ids": [],
            "target_keys": ["https://evil.example.com"], "directive_ids": [], "assignment_ids": []
        }),
    );
    assert_eq!(fixture.reason_of(&text), reason::URL_INVENTED);
}

#[test]
fn gold18_fence_is_stripped_once_and_bad_content_is_still_refused() {
    let fixture = Fixture::new();
    let good = fixture.decompose_text(&[("path_prefix", "/api/v1/users")]);
    let fenced = format!("```json\n{good}\n```");
    assert!(fixture.run(&fenced).applied(), "剥一层围栏后必须合法");

    let other = Fixture::new();
    let garbage =
        "```json\n{\"decision_id\":\"dec-1111111111111111\",\"action\":\"decompose\"}\n```";
    match apply_at(&other, garbage) {
        TurnOutcome::Refused { reason_code, .. } => {
            assert_eq!(reason_code, reason::MISSING_REQUIRED_FIELD)
        }
        other => panic!("围栏里的坏 schema 必须被拒，实际 {other:?}"),
    }
}

#[test]
fn gold19_free_text_twice_degrades_instead_of_burning_more_model_calls() {
    let fixture = Fixture::new();
    fixture.add_endpoint_evidence("/api/v1/users", 1);
    assert!(matches!(
        fixture.run("先看用户接口吧，预算我觉得够用。"),
        TurnOutcome::Refused { reason_code, .. } if reason_code == reason::FREE_TEXT_INSTEAD_OF_JSON
    ));
    assert!(matches!(
        fixture.run("还是先并行三路比较稳妥。"),
        TurnOutcome::Idle(_)
    ));
    assert_eq!(
        fixture.allowed_actions(),
        Vec::<String>::new(),
        "streak 达阈后不得再给模型任何动作（DESIGN §7.3）"
    );
}

#[test]
fn gold20_two_actions_in_one_output_is_refused() {
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(1);
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    // SPEC §8 GOLD-20：一次输出两个 action → schema_invalid（重复 action 键 / 两个对象）。
    let duplicated = "{\"decision_id\":\"dec-1111111111111111\",\"action\":\"assign\",\"parent_task_id\":\"tsk-1122334455667788\",\"action\":\"accept\",\"payload\":{},\"allowlist_refs\":{\"slice_ids\":[],\"worker_ids\":[],\"tool_ids\":[],\"target_keys\":[],\"directive_ids\":[],\"assignment_ids\":[]}}";
    assert_eq!(
        super::decision::parse_decision_envelope(duplicated)
            .expect_err("重复 action 必须被拒")
            .reason_code,
        reason::SCHEMA_INVALID
    );
    let valid = fixture.decompose_text(&[("path_prefix", "/api/v1/orders")]);
    let two_objects = format!("{valid} {valid}");
    assert_eq!(
        super::decision::parse_decision_envelope(&two_objects)
            .expect_err("两个对象必须被拒")
            .reason_code,
        reason::SCHEMA_INVALID
    );
    // 信封里夹带第二个动作键（next_action）按 extra_field 拒。
    let smuggled = fixture.envelope(
        "assign",
        json!({
            "slice_id": slice_id,
            "worker_id": workers[0],
            "reserved_tokens": 4000,
            "reserved_requests": 2,
            "capability_lease": ["evidence.read"],
            "lane": "read_only_analysis",
            "attempt": 1,
            "next_action": "accept"
        }),
        json!({"slice_ids":[slice_id], "worker_ids":[workers[0].clone()], "tool_ids":["evidence.read"], "target_keys":[TARGET], "directive_ids":[], "assignment_ids":[]}),
    );
    assert_eq!(fixture.reason_of(&smuggled), reason::EXTRA_FIELD);
}

#[test]
fn gold23_refs_inconsistent_with_payload_are_refused() {
    let fixture = Fixture::new();
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let slice_id = fixture.slice_id("path_prefix", "/api/v1/users");
    let workers = fixture.provision_workers(1);
    // payload 指向真切片，但 refs 声明了另一个 id。
    let text = fixture.envelope(
        "assign",
        json!({
            "slice_id": slice_id,
            "worker_id": workers[0],
            "reserved_tokens": 4000,
            "reserved_requests": 2,
            "capability_lease": ["evidence.read"],
            "lane": "read_only_analysis",
            "attempt": 1
        }),
        json!({
            "slice_ids": ["slc-0000000000000000"], "worker_ids": [workers[0].clone()],
            "tool_ids": ["evidence.read"], "target_keys": [TARGET], "directive_ids": [], "assignment_ids": []
        }),
    );
    assert_eq!(fixture.reason_of(&text), reason::ALLOWLIST_VIOLATION);
    assert_eq!(
        store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有")
            .state,
        "pending"
    );
}

#[test]
fn gold24_extra_field_and_gold25_missing_field_are_refused() {
    let fixture = Fixture::new();
    assert!(fixture
        .run(&fixture.decompose_text(&[("path_prefix", "/api/v1/users")]))
        .applied());
    let task = fixture.task();
    let with_extra = fixture.envelope(
        "stop",
        json!({
            "stop_reason": "goal_reached", "accepted_slice_ids": [], "rejected_slice_ids": [],
            "summary_zh": "无", "publish_partial": false, "priority": 1
        }),
        json!({"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []}),
    );
    assert_eq!(fixture.reason_of(&with_extra), reason::EXTRA_FIELD);

    let missing = json!({
        "action": "stop",
        "parent_task_id": task.task_id,
        "payload": {},
        "allowlist_refs": {"slice_ids": [], "worker_ids": [], "tool_ids": [], "target_keys": [], "directive_ids": [], "assignment_ids": []}
    })
    .to_string();
    assert_eq!(
        super::decision::parse_decision_envelope(&missing)
            .expect_err("缺 decision_id 必须被拒")
            .reason_code,
        reason::MISSING_REQUIRED_FIELD
    );
}

fn apply_at(fixture: &Fixture, text: &str) -> TurnOutcome {
    fixture.run(text)
}

#[test]
fn closed_sets_are_exactly_the_spec_members() {
    // H1 护栏：任何人加第 8 个动作、第 9 个状态或多一条 reason_code 都会红。
    assert_eq!(
        types::ACTIONS,
        [
            "decompose",
            "assign",
            "accept",
            "reject_retry",
            "stop",
            "relay_directive",
            "reject_directive"
        ]
    );
    assert_eq!(
        types::LANES,
        ["read_only_analysis", "target_touching", "review"]
    );
    assert_eq!(types::TASK_STATES.len(), 8);
    assert_eq!(types::SLICE_STATES.len(), 7);
    assert_eq!(types::ASSIGNMENT_STATES.len(), 8);
    assert_eq!(types::DIRECTIVE_STATES.len(), 7);
    assert_eq!(types::VERDICTS, ["accept", "retry", "reject"]);
    assert_eq!(types::WORKER_STATES, ["idle", "busy", "draining", "dead"]);
    assert_eq!(types::PROFILE_HOMOGENEOUS_V1, "homogeneous_v1");
    assert_eq!(
        reason::ALL.len(),
        38,
        "SPEC 37 + 深度分析 §J 的 zero_intent_illegal"
    );
    let mut sorted = reason::ALL.to_vec();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 38, "reason_code 不得重复");
    for code in reason::ALL {
        assert!(
            code.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'),
            "reason_code 必须 snake_case 英文：{code}"
        );
    }
}

#[test]
fn database_checks_refuse_invented_states_and_roles() {
    let fixture = Fixture::new();
    let task = fixture.task();
    let bad_state = fixture
        .connection
        .execute(
            "INSERT INTO agent_task_slices(slice_id,parent_task_id,scope_kind,scope_value,evidence_revision_base,lane,worker_capability_profile_id,state,max_retries,retry_count)\
             VALUES('slc-1111111111111111',?1,'path_prefix','/a',1,'read_only_analysis','homogeneous_v1','thinking',2,0)",
            params_one(&task.task_id),
        )
        .expect_err("DB CHECK 必须挡住 invented_state");
    assert!(bad_state.to_string().contains("CHECK"));
    let bad_profile = fixture
        .connection
        .execute(
            "INSERT INTO agent_task_slices(slice_id,parent_task_id,scope_kind,scope_value,evidence_revision_base,lane,worker_capability_profile_id,state,max_retries,retry_count)\
             VALUES('slc-2222222222222222',?1,'path_prefix','/b',1,'read_only_analysis','RepoMapper','pending',2,0)",
            params_one(&task.task_id),
        )
        .expect_err("DB CHECK 必须挡住 invented_role");
    assert!(bad_profile.to_string().contains("CHECK"));
    let bad_worker = fixture
        .connection
        .execute(
            "INSERT INTO agent_worker_instances(worker_id,root_run_id,capability_profile_id,assignment_id,state,lease_epoch)\
             VALUES('wrk-3333333333333333',?1,'homogeneous_v1','','sleeping',1)",
            params_one(&fixture.root_run_id),
        )
        .expect_err("Worker 状态必须封闭");
    assert!(bad_worker.to_string().contains("CHECK"));
}

fn params_one(value: &str) -> impl rusqlite::Params {
    [value.to_string()]
}

#[test]
fn legacy_lane_table_and_legacy_states_are_untouched_by_homogeneous_rows() {
    // X-01/X-10：homogeneous 的 assignment 不得被 legacy 查询认作 lane 占用。
    let fixture = Fixture::new();
    let workers = fixture.provision_workers(2);
    assert!(fixture
        .run(&fixture.decompose_text(&[
            ("path_prefix", "/api/v1/users"),
            ("path_prefix", "/api/v1/orders")
        ]))
        .applied());
    for (index, path) in ["/api/v1/users", "/api/v1/orders"].iter().enumerate() {
        let slice_id = fixture.slice_id("path_prefix", path);
        assert!(fixture
            .run(&fixture.assign_text(&slice_id, &workers[index], 4000, 2, &["evidence.read"], 1))
            .applied());
    }
    // 现状 legacy 占用检查看到的 homogeneous 行必须被 scheduling_model 过滤掉。
    let legacy_visible: i64 = fixture
        .connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE lane='read_only_analysis' AND state IN ('prepared','leased','running','waiting_review') AND scheduling_model='legacy_role_pipeline'",
            [],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(legacy_visible, 0);
    let state: String = fixture
        .connection
        .query_row(
            "SELECT state FROM agent_assignments WHERE scheduling_model='homogeneous_workers' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("状态");
    assert_eq!(state, "assigned", "新 vocabulary 只在 homogeneous 行出现");
    assert_eq!(
        fixture
            .connection
            .query_row("SELECT COUNT(*) FROM agent_lane_leases", [], |row| row
                .get::<_, i64>(0))
            .expect("计数"),
        0
    );
}

#[test]
fn tool_candidates_come_from_the_registry_not_the_spec_examples() {
    // DESIGN §4.9：SPEC 示例里的 http.get 在仓库不存在，候选必须是真实串。
    let candidates = gate::tool_allowlist_for_lane("read_only_analysis");
    assert!(candidates.contains(&"evidence.read".to_string()));
    assert!(candidates.contains(&"snapshot.read".to_string()));
    assert!(!candidates.contains(&"http.get".to_string()));
    let snapshot = AllowlistSnapshot {
        tool_ids: candidates,
        ..Default::default()
    };
    assert_eq!(
        super::allowlist::verify_capability_lease(&snapshot, &["http.get".to_string()])
            .expect_err("照抄示例必须被拒")
            .reason_code,
        reason::TOOL_NOT_AUTHORIZED
    );
}

#[test]
fn evidence_alias_round_trip_is_stable_and_out_of_root_refs_are_missing() {
    // X-12：evd- 别名可逆；跨 root 引用按 evidence_missing 拒。
    let fixture = Fixture::new();
    let alias = fixture.add_endpoint_evidence("/api/v1/users", 1);
    assert!(types::valid_evidence_ref(&alias));
    let resolved = store::resolve_evidence_alias(&fixture.connection, &alias)
        .expect("解析")
        .expect("必须命中");
    let again =
        store::register_evidence_alias(&fixture.connection, &resolved.0, &fixture.root_run_id, 1)
            .expect("幂等");
    assert_eq!(again, alias);
    let other = Fixture::new();
    assert!(store::resolve_evidence_alias(&other.connection, &alias)
        .expect("读取")
        .is_none());
}

#[test]
fn stop_after_full_success_publishes_every_accepted_slice() {
    let fixture = Fixture::new();
    let evidence = fixture.add_endpoint_evidence("/api/v1/users", 1);
    let (assignment_id, report_id, slice_id) =
        fixture.reported_assignment("/api/v1/users", &[&evidence]);
    let checks = super::machine::recompute(
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
    assert!(fixture
        .run(&fixture.accept_text(&assignment_id, &slice_id, &report_id, &checks, &[&evidence]))
        .applied());
    // SPEC §3.1：全 accept 后仍等主管 stop 收尾，不由验收动作自行宣告成功。
    assert_eq!(fixture.task().state, "running");
    assert!(fixture
        .run(&fixture.stop_text("goal_reached", &[&slice_id], &[], false))
        .applied());
    assert_eq!(fixture.task().state, "stopped");
}

#[test]
fn illegal_state_transitions_are_refused_without_writing() {
    // DESIGN §6：非法迁移整单拒绝且不写库（SPEC §3.2/§3.1 的禁止列）。
    let fixture = Fixture::new();
    let evidence = fixture.add_endpoint_evidence("/api/v1/users", 1);
    let (assignment_id, report_id, slice_id) =
        fixture.reported_assignment("/api/v1/users", &[&evidence]);
    let checks = super::machine::recompute(
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
    assert!(fixture
        .run(&fixture.accept_text(&assignment_id, &slice_id, &report_id, &checks, &[&evidence]))
        .applied());
    // accepted 是终态：任何回退都必须被拒且不写库。
    let error = store::set_slice_state(&fixture.connection, &slice_id, "assigned")
        .expect_err("accepted → assigned 必须被拒");
    assert_eq!(error.reason_code, reason::ALLOWLIST_VIOLATION);
    assert_eq!(
        store::load_slice(&fixture.connection, &slice_id)
            .expect("s")
            .expect("有")
            .state,
        "accepted"
    );
    // 任务终态同理：stopped 之后不得再回到 decomposed。
    store::set_task_state(&fixture.connection, &fixture.task().task_id, "stopped", 1)
        .expect("stopped");
    let error = store::set_task_state(
        &fixture.connection,
        &fixture.task().task_id,
        "decomposed",
        1,
    )
    .expect_err("终态任务不得回退");
    assert_eq!(error.reason_code, reason::ALLOWLIST_VIOLATION);
    assert_eq!(fixture.task().state, "stopped");
    // 未知状态词仍旧按 invented_state 拒（SPEC §1.3 非法例 2）。
    assert_eq!(
        store::set_slice_state(&fixture.connection, &slice_id, "thinking")
            .expect_err("非法状态词")
            .reason_code,
        reason::INVENTED_STATE
    );
}

#[test]
fn system_prompt_never_widens_the_closed_action_set() {
    // DESIGN §9：提示词可微调，但不得放宽枚举。
    assert!(super::prompt::prompt_actions_are_closed());
    let prompt = super::prompt::system_prompt();
    for forbidden in ["delegate", "escalate", "spawn_worker", "claim_task"] {
        assert!(
            !prompt.contains(forbidden),
            "提示里出现了第 8 个动作：{forbidden}"
        );
    }
    assert!(prompt.contains("homogeneous_v1"));
    assert!(prompt.contains("reason_code"));
    assert!(prompt.contains("suggestion_zh"));
    assert!(super::prompt::FEW_SHOTS.len() >= 7);
    // few-shot 的「正确：」半句里不得出现任何封闭集合之外的动作词或专家画像建议。
    for (_, text) in super::prompt::FEW_SHOTS {
        if let Some(right) = text.split("正确：").nth(1) {
            assert!(
                !right.contains("Expert") && !right.contains("delegate"),
                "few-shot 把越权写法当成正确做法：{right}"
            );
        }
    }
}

#[test]
fn lease_fencing_refuses_a_stale_coordinator() {
    // DESIGN §12：过期主管既不能读账本也不能落状态。
    let fixture = Fixture::new();
    let mut stale = fixture.lease.clone();
    stale.fencing_token = "00000000-0000-0000-0000-000000000000".to_string();
    let text = fixture.decompose_text(&[("path_prefix", "/api/v1/users")]);
    assert_eq!(
        fixture.reason_of_with_lease(&stale, &text),
        reason::LEASE_INVALID,
        "过期租约必须失败"
    );
    assert_eq!(
        store::load_task(&fixture.connection, &fixture.task().task_id)
            .expect("t")
            .expect("有")
            .state,
        "created"
    );
}
