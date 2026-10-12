// DESIGN §5.4 / §5.1 / §2.2：同质 Worker 路径的接线盘点——新 frame kind、策略护栏、
// 任务级 scheduling_model 开关，以及 legacy 调度对新行的隔离。
#[test]
fn scheduling_model_comes_from_the_frozen_scan_plan_and_defaults_to_legacy() {
    // DESIGN §2.2：任务创建时的开关取值只来自冻结计划；缺省/非法一律 legacy。
    use crate::agent_runtime::multi_agent::homogeneous::store;
    let (dir, connection, lease) = homogeneous_wiring_fixture("legacy_role_pipeline");
    assert_eq!(
        store::scheduling_model_from_plan(&connection, &lease.scan_id, lease.attempt_number)
            .expect("无计划"),
        "legacy_role_pipeline"
    );
    connection
        .execute(
            "INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash) VALUES(?1,?2,'web','native','{\"scheduling_model\":\"nonsense\"}','h1')",
            rusqlite::params![lease.scan_id.clone(), lease.attempt_number],
        )
        .expect("写入计划");
    assert_eq!(
        store::scheduling_model_from_plan(&connection, &lease.scan_id, lease.attempt_number)
            .expect("非法值"),
        "legacy_role_pipeline",
        "非法值必须回落 legacy"
    );
    assert_eq!(
        store::activate_task_for_lease(&connection, &lease).expect("激活"),
        None
    );
    connection
        .execute(
            "UPDATE native_scan_plans SET plan_json='{\"scheduling_model\":\"homogeneous_workers\"}' WHERE scan_id=?1 AND attempt_number=?2",
            rusqlite::params![lease.scan_id.clone(), lease.attempt_number],
        )
        .expect("改写计划");
    assert_eq!(
        store::scheduling_model_from_plan(&connection, &lease.scan_id, lease.attempt_number)
            .expect("计划声明"),
        "homogeneous_workers"
    );
    // 只读计划不足以改变行为：run 列未被写为 homogeneous 之前，任务仍不建立。
    assert_eq!(
        store::activate_task_for_lease(&connection, &lease).expect("激活"),
        None,
        "必须先按计划写 run 列，再激活任务"
    );
    store::set_run_scheduling_model(
        &connection,
        &lease.root_run_id,
        store::scheduling_model_from_plan(&connection, &lease.scan_id, lease.attempt_number)
            .expect("模式"),
    )
    .expect("写模式");
    assert!(store::activate_task_for_lease(&connection, &lease)
        .expect("激活")
        .is_some());
    drop(connection);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn homogeneous_frame_kind_has_its_own_step_and_stays_out_of_legacy_inventory() {
    let frame = NativeCoordinatorFrame {
        kind: NATIVE_HOMOGENEOUS_FRAME_KIND,
        semantic: json!({"display": NATIVE_HOMOGENEOUS_FRAME_KIND}),
        rows: Vec::new(),
        review_refs: None,
        review_meta: None,
    };
    assert_eq!(frame.step(), NATIVE_HOMOGENEOUS_FRAME_STEP);
    assert_eq!(
        native_coordinator_frame_step_for_kind(NATIVE_HOMOGENEOUS_FRAME_KIND),
        NATIVE_HOMOGENEOUS_FRAME_STEP
    );
    // 新 kind 单独准入，不借用旧清单。
    assert!(native_supervision_frame_admission(NATIVE_HOMOGENEOUS_FRAME_KIND).is_ok());
    assert!(!NATIVE_WIRED_SUPERVISION_FRAME_KINDS.contains(&NATIVE_HOMOGENEOUS_FRAME_KIND));
    assert!(!NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.contains(&NATIVE_HOMOGENEOUS_FRAME_KIND));
    // 旧盘点必须原样：18 条 wired、1 条 not_ready。
    assert_eq!(NATIVE_WIRED_SUPERVISION_FRAME_KINDS.len(), 18);
    assert_eq!(NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS.len(), 1);
    // 未知 kind 仍旧落到 unmapped，没有被顺带放行。
    assert_eq!(
        native_coordinator_frame_step_for_kind("homogeneous-decisionx"),
        "assess:unmapped_frame_kind"
    );
    assert_eq!(
        native_supervision_frame_admission("homogeneous-decisionx").unwrap_err(),
        "root_trigger_not_enabled"
    );
}

fn homogeneous_wiring_fixture(
    model: &str,
) -> (
    std::path::PathBuf,
    rusqlite::Connection,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    use crate::agent_runtime::multi_agent::{homogeneous::store, lease::acquire_coordinator_lease};
    let dir = std::env::temp_dir().join(format!("oviraptor-homog-wiring-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("临时目录");
    let path = crate::db::initialize(&dir).expect("初始化临时库");
    let connection = crate::db::open(&path).expect("打开临时库");
    connection
        .execute(
            "INSERT INTO projects(name) VALUES(?1)",
            [format!("p-{}", uuid::Uuid::new_v4()).as_str()],
        )
        .expect("项目");
    let project_id = connection.last_insert_rowid();
    let scan_id = format!("scan-{}", uuid::Uuid::new_v4());
    let root_run_id = format!("run-{}", uuid::Uuid::new_v4());
    connection
        .execute(
            "INSERT INTO sentinel_scans(id,project_id,status,attempt_count) VALUES(?1,?2,'scanning',1)",
            rusqlite::params![scan_id, project_id],
        )
        .expect("扫描");
    connection
        .execute(
            "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,role,status,root_run_id,orchestration_policy,hard_token_budget,hard_request_budget)\
             VALUES(?1,?2,1,'https://app.example.com','coordinator','running',?1,'multi',24000,12)",
            rusqlite::params![root_run_id, scan_id],
        )
        .expect("root run");
    assert_eq!(
        store::run_scheduling_model(&connection, &root_run_id).expect("默认模式"),
        "legacy_role_pipeline"
    );
    store::set_run_scheduling_model(&connection, &root_run_id, model).expect("写入开关");
    let lease = acquire_coordinator_lease(
        &connection,
        &scan_id,
        1,
        "https://app.example.com",
        &root_run_id,
        600,
    )
    .expect("Coordinator 租约");
    (dir, connection, lease)
}

#[test]
fn scheduling_model_defaults_to_legacy_and_only_activates_on_explicit_opt_in() {
    use crate::agent_runtime::multi_agent::homogeneous::store;
    let (dir, connection, lease) = homogeneous_wiring_fixture("legacy_role_pipeline");
    // 默认 legacy：Coordinator 激活时不得建立同质任务。
    assert_eq!(
        store::activate_task_for_lease(&connection, &lease).expect("激活"),
        None
    );
    assert!(
        store::load_task_by_root_run(&connection, &lease.root_run_id)
            .expect("读取")
            .is_none()
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(&dir);

    let (dir, connection, lease) = homogeneous_wiring_fixture("homogeneous_workers");
    let task_id = store::activate_task_for_lease(&connection, &lease)
        .expect("激活")
        .expect("显式开关必须建立同质任务");
    assert!(task_id.starts_with("tsk-"));
    let task = store::load_task_by_root_run(&connection, &lease.root_run_id)
        .expect("读取")
        .expect("任务存在");
    assert_eq!(task.state, "created");
    assert_eq!(task.scheduling_model, "homogeneous_workers");
    assert_eq!(task.allowed_parallel, 3);
    drop(connection);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn homogeneous_action_gate_only_admits_actions_the_rust_gate_allows() {
    use crate::agent_runtime::multi_agent::homogeneous::{reason, store};
    let (dir, connection, lease) = homogeneous_wiring_fixture("homogeneous_workers");
    store::activate_task_for_lease(&connection, &lease).expect("激活任务");
    // 新任务只允许分解。
    let accepted = native_homogeneous_action_gate(&connection, &lease, "decompose").expect("护栏");
    assert_eq!(accepted["selection"], json!("accepted"));
    assert_eq!(accepted["step"], json!(NATIVE_HOMOGENEOUS_FRAME_STEP));
    assert_eq!(accepted["advisoryOnly"], json!(true));
    assert_eq!(accepted["allowedActions"], json!(["decompose", "stop"]));
    let refused = native_homogeneous_action_gate(&connection, &lease, "accept").expect("护栏");
    assert_eq!(refused["selection"], json!("refused"));
    assert_eq!(refused["reasonCode"], json!(reason::UNKNOWN_ACTION));
    assert_eq!(
        native_homogeneous_action_gate(&connection, &lease, "escalate").unwrap_err(),
        reason::UNKNOWN_ACTION.to_string()
    );
    // 第 8 个动作在任何层都不被承认。
    assert_eq!(
        native_homogeneous_action_gate(&connection, &lease, "delegate").unwrap_err(),
        reason::UNKNOWN_ACTION.to_string()
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_scheduler_refuses_to_dispatch_on_a_homogeneous_coordinator_run() {
    use crate::agent_runtime::contract::{AgentLane, AgentRole};
    use crate::agent_runtime::multi_agent::homogeneous::store;
    use crate::agent_runtime::multi_agent::scheduler::schedule_child;
    let (dir, connection, lease) = homogeneous_wiring_fixture("homogeneous_workers");
    store::activate_task_for_lease(&connection, &lease).expect("激活任务");
    let error = schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "mapper-bootstrap",
        &json!({"slice_id": "slc-1111111111111111"}),
        1,
        &["evidence.read".to_string()],
        1000,
        1,
    )
    .expect_err("同质任务不得走 legacy 互斥 lane");
    assert!(
        error.contains("homogeneous_worker_requires_homogeneous_scheduler"),
        "{error}"
    );
    // 且没有留下任何 assignment 或 lane 占用。
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM agent_assignments", [], |row| row
                .get::<_, i64>(0))
            .expect("计数"),
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM agent_lane_leases", [], |row| row
                .get::<_, i64>(0))
            .expect("计数"),
        0
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn legacy_lane_occupancy_still_counts_legacy_rows_and_ignores_homogeneous_rows() {
    // DESIGN §5.1 / §6：新加的 scheduling_model 过滤只能把 homogeneous 行挡在
    // legacy 互斥盘点之外，legacy 行本身一格都不能少算。
    let (dir, connection, lease) = homogeneous_wiring_fixture("legacy_role_pipeline");
    let insert = |id: &str, state: &str, model: &str, slice: &str| {
        connection
            .execute(
                "INSERT INTO agent_assignments(id,coordinator_run_id,child_run_id,role,lane,target_key,state,dedup_key,trigger_code,evidence_revision,reserved_tokens,reserved_requests,slice_id,scheduling_model)\
                 VALUES(?1,?2,'','spa_api_mapper','read_only_analysis',?3,?4,?5,'mapper-bootstrap',1,1000,1,?6,?7)",
                rusqlite::params![
                    id,
                    lease.root_run_id,
                    lease.target_key,
                    state,
                    format!("dedup-{id}"),
                    slice,
                    model
                ],
            )
            .expect("assignment");
    };
    insert("asg-0000000000000001", "leased", "legacy_role_pipeline", "");
    insert(
        "asg-0000000000000002",
        "assigned",
        "homogeneous_workers",
        "slc-000000000000000a",
    );
    insert(
        "asg-0000000000000003",
        "running",
        "homogeneous_workers",
        "slc-000000000000000b",
    );
    // 生产里那条带过滤的占用检查。
    let filtered: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.coordinator_run_id \
             WHERE r.scan_id=?1 AND r.attempt_number=?2 AND a.target_key=?3 AND a.lane=?4 \
             AND a.id<>?5 AND a.state IN ('prepared','leased','running','waiting_review') \
             AND a.scheduling_model='legacy_role_pipeline')",
            rusqlite::params![lease.scan_id, lease.attempt_number, lease.target_key, "read_only_analysis", "asg-new"],
            |row| row.get(0),
        )
        .expect("占用查询");
    assert!(filtered, "legacy 行仍必须占用 lane，过滤条件不得漏算");
    // 只数 homogeneous 行时应当是 2 行（证明它们确实存在，只是不参与互斥）。
    let homogeneous_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE scheduling_model='homogeneous_workers'",
            [],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(homogeneous_rows, 2);
    // 未带过滤的旧写法会把三行都当成占用者——这正是要修掉的误报来源。
    let unfiltered_rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE lane='read_only_analysis' \
             AND state IN ('prepared','leased','running','waiting_review','assigned')",
            [],
            |row| row.get(0),
        )
        .expect("计数");
    assert_eq!(unfiltered_rows, 3);
    drop(connection);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_projected_rejection_continues_the_round_instead_of_marking_it_incomplete() {
    // DESIGN §5.5 / §8.6：只有主管把驳回四字段投影给用户，`root_human_directive_deferred`
    // 才让本轮继续；未裁决、读取失败或 legacy 任务一律仍按 incomplete 处理。
    use crate::agent_runtime::multi_agent::homogeneous::{directive_gate, store};
    let answered = |path: &std::path::Path, run: &str| {
        crate::commands::human_deferral_answered_by_reject(path, run)
    };
    let (dir, connection, lease) = homogeneous_wiring_fixture("homogeneous_workers");
    store::activate_task_for_lease(&connection, &lease)
        .expect("激活")
        .expect("同质任务必须建立");
    let task = store::load_task_by_root_run(&connection, &lease.root_run_id)
        .expect("任务可读")
        .expect("任务存在");
    let path = dir.join("oviraptor.sqlite3");
    assert!(!answered(&path, ""), "缺 run_id 一律退回 incomplete");
    assert!(
        !answered(&path, &lease.root_run_id),
        "尚未裁决的引导不得放行为继续"
    );
    let directive_id = format!(
        "dir-{}",
        &crate::agent_runtime::store::stable_hash("adapter-reject-projection")[..24]
    );
    directive_gate::screen(&connection, &task, &directive_id, "", "把预算调到无限").expect("初筛");
    assert!(
        !answered(&path, &lease.root_run_id),
        "仅规则初筛不足以放行本轮"
    );
    directive_gate::mark_rejected(
        &connection,
        &directive_id,
        "directive_budget_change",
        "预算调整不属于用户引导的权限范围。",
        "请把要求限定在已授权的路径与目标内。",
        "这条引导已被驳回：预算只能由现有闸口调整。",
    )
    .expect("驳回");
    assert!(
        answered(&path, &lease.root_run_id),
        "四字段已投影给用户 → 本轮继续而不是 incomplete"
    );
    let (legacy_dir, _legacy_connection, legacy_lease) =
        homogeneous_wiring_fixture("legacy_role_pipeline");
    assert!(
        !answered(
            &legacy_dir.join("oviraptor.sqlite3"),
            &legacy_lease.root_run_id
        ),
        "legacy 任务不得拿到这条放行，行为保持原样"
    );
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&legacy_dir);
}

#[test]
fn the_homogeneous_human_hook_waits_until_every_directive_is_decided() {
    // DESIGN §5.5：同质 root 的引导裁决以 gated 表为准；legacy root 仍走原判定。
    use crate::agent_runtime::multi_agent::homogeneous::{directive_gate, store};
    use crate::commands::homogeneous_directives_decided;
    let (legacy_dir, legacy_db, legacy_lease) = homogeneous_wiring_fixture("legacy_role_pipeline");
    assert_eq!(
        homogeneous_directives_decided(&legacy_db, &legacy_lease.root_run_id).expect("判定"),
        None,
        "legacy run 不得被同质分支接管"
    );

    let (dir, connection, lease) = homogeneous_wiring_fixture("homogeneous_workers");
    store::activate_task_for_lease(&connection, &lease)
        .expect("激活")
        .expect("同质任务必须建立");
    assert_eq!(
        homogeneous_directives_decided(&connection, &lease.root_run_id).expect("判定"),
        Some(true),
        "没有待裁决引导时本轮可以继续"
    );
    let task = store::load_task_by_root_run(&connection, &lease.root_run_id)
        .expect("任务可读")
        .expect("任务存在");
    let directive_id = format!(
        "dir-{}",
        &crate::agent_runtime::store::stable_hash("hook-pending-directive")[..24]
    );
    directive_gate::screen(&connection, &task, &directive_id, "", "把预算调到无限").expect("初筛");
    assert_eq!(
        homogeneous_directives_decided(&connection, &lease.root_run_id).expect("判定"),
        Some(false),
        "引导还在等主管裁决 → 调用方必须保持 deferred，而不是当成已完成"
    );
    directive_gate::mark_rejected(
        &connection,
        &directive_id,
        "directive_budget_change",
        "预算调整不属于用户引导的权限范围。",
        "请把要求限定在已授权的路径与目标内。",
        "这条引导已被驳回：预算只能由现有闸口调整。",
    )
    .expect("驳回");
    assert_eq!(
        homogeneous_directives_decided(&connection, &lease.root_run_id).expect("判定"),
        Some(true),
        "驳回四字段投影后本轮才可继续，且不因驳回被打断"
    );
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(legacy_dir);
}

#[test]
fn homogeneous_frame_carries_the_live_candidate_snapshot_and_refuses_drift() {
    // P1-1：帧只带候选哈希；候选或允许动作漂移时，注入 prompt 前必须被拒。
    use crate::agent_runtime::multi_agent::homogeneous::store;
    let (_dir, connection, lease) = homogeneous_wiring_fixture("homogeneous_workers");
    store::activate_task_for_lease(&connection, &lease)
        .expect("激活")
        .expect("同质任务必须建立");
    let frame = native_homogeneous_decision_frame(&connection, &lease).expect("帧");
    assert_eq!(frame.step(), "assess:homogeneous_decision");
    assert_eq!(frame.semantic["advisoryOnly"], serde_json::json!(true));
    assert_eq!(frame.semantic["executionGrant"], serde_json::json!(false));
    let basis =
        native_homogeneous_require_current_frame(&connection, &lease, &frame).expect("basis");
    assert_eq!(basis["phase"], "homogeneous-decision");
    assert_eq!(
        basis["homogeneousTask"]["allowedActions"],
        serde_json::json!(["decompose", "stop"]),
        "§J：有限冻结预算的新任务保留 stop 候选，Rust 再核真实预算耗尽"
    );
    assert_eq!(
        basis["homogeneousTask"]["candidates"]["workerIds"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    // 任务进入 running 后候选/动作集会变，旧帧必须失效而不是照原样付费问模型。
    connection
        .execute(
            "UPDATE agent_homogeneous_tasks SET state='running' WHERE root_run_id=?1",
            [lease.root_run_id.as_str()],
        )
        .expect("推进任务态");
    assert_eq!(
        native_homogeneous_require_current_frame(&connection, &lease, &frame).unwrap_err(),
        "root_homogeneous_candidate_basis_changed"
    );
}

#[test]
fn a_homogeneous_root_tick_mints_the_frame_pays_once_and_applies_the_decision() {
    // P0-1 + P1-1 的端到端证据：真实付费 SDK 轮里，无帧的 Root 主管轮只有在 run 显式
    // homogeneous_workers 时才 mint 同质主管帧；模型原文只经 tick 返回路径进入
    // run_coordinator_turn，一轮一次发票，副作用可在库里核对。
    use crate::agent_runtime::multi_agent::homogeneous::{
        decompose::slice_id_for, journal, store, types,
    };
    use std::sync::{Arc, Mutex};

    let reply: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let shared = reply.clone();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        (
            200,
            "application/json",
            sdk_exact_model_body(&shared.lock().unwrap().clone()),
        )
    }));
    // 出生轮用不带本地审议的原生协议：同质评估轮的 basis 不带 localParents 证明，
    // 若 Root 出生合同开了本地审议，tick 可执行性检查会在发送前取消这一轮。
    let f = root_tick_fixture_protocol(
        "homogeneous-tick",
        &format!("http://127.0.0.1:{port}/v1"),
        false,
    );
    assert!(
        native_homogeneous_frame_for_root(&f.context, &f.actor)
            .expect("legacy 轮不 mint")
            .is_none(),
        "默认 legacy_role_pipeline 的 Root 必须走原帧路径"
    );

    // 带帧评估轮排在 Root 出生轮之后：现网的原始模型绑定不接受第一轮就换 kind。
    *reply.lock().unwrap() = root_tick_valid_text("homogeneous birth round");
    native_coordinator_tick(&f.context, &f.actor).expect("出生轮");
    assert!(
        native_homogeneous_frame_for_root(&f.context, &f.actor)
            .expect("legacy 出生轮之后仍不 mint")
            .is_none(),
        "开关未打开时不得接管任何轮次"
    );

    let db = db::open(&f.context.db_path).unwrap();
    store::set_run_scheduling_model(&db, &f.actor.root_run_id, types::SCHEDULING_HOMOGENEOUS)
        .expect("写入开关");
    store::activate_task_for_lease(&db, &f.actor)
        .expect("激活")
        .expect("显式开关必须建立同质任务");
    let task = store::load_task_by_root_run(&db, &f.actor.root_run_id)
        .expect("读取任务")
        .expect("任务存在");
    let frame = native_homogeneous_frame_for_root(&f.context, &f.actor)
        .expect("mint")
        .expect("homogeneous 轮必须拿到帧");
    assert_eq!(frame.step(), NATIVE_HOMOGENEOUS_FRAME_STEP);

    let paths = ["/api/v1/users", "/api/v1/orders", "/api/v1/payments"];
    let slices: Vec<serde_json::Value> = paths
        .iter()
        .map(|path| {
            json!({
                "slice_id": slice_id_for(&task.task_id, "path_prefix", path, task.evidence_revision_base),
                "scope_kind": "path_prefix",
                "scope_value": path,
                "lane": "read_only_analysis",
                "evidence_revision_base": task.evidence_revision_base,
                "worker_capability_profile_id": types::PROFILE_HOMOGENEOUS_V1,
                "max_retries": 2
            })
        })
        .collect();
    *reply.lock().unwrap() = json!({
        "decision_id": format!(
            "dec-{}",
            &crate::agent_runtime::store::stable_hash(&format!("{}decompose", task.root_run_id))[..24]
        ),
        "action": "decompose",
        "parent_task_id": task.task_id,
        "payload": {
            "slices": slices,
            "decomposition_basis_zh": "按已发现 API 路径前缀三分"
        },
        "allowlist_refs": {
            "slice_ids": [], "worker_ids": [], "tool_ids": [],
            "target_keys": [task.target_key], "directive_ids": [], "assignment_ids": []
        }
    })
    .to_string();

    let receipt = native_coordinator_tick(&f.context, &f.actor).expect("一轮同质付费 tick");
    assert_eq!(seen.lock().unwrap().len(), 2, "同质评估轮只新增一次发票");
    assert_eq!(
        receipt.summary.as_json()["suggestions"],
        json!([journal::LEDGER_STEP]),
        "账本摘要只是固定 step 的投影，不改 DecisionSummary 结构"
    );
    drop(db);
    let db = db::open(&f.context.db_path).unwrap();
    let task = store::load_task_by_root_run(&db, &f.actor.root_run_id)
        .expect("读取任务")
        .expect("任务存在");
    assert_eq!(
        task.state, "decomposed",
        "副作用必须真的落库，不止 advisory"
    );
    let slice_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_task_slices WHERE parent_task_id=?1",
            [task.task_id.as_str()],
            |row| row.get(0),
        )
        .expect("切片可读");
    assert_eq!(slice_count, 3);
    let log = std::fs::read_to_string(&f.context.log_path).unwrap_or_default();
    assert!(
        log.contains("root_homogeneous_applied:decompose"),
        "tick 返回路径必须留下审计摘要：{log}"
    );
    // 账本里第二评估轮的 basis 就是候选快照本身（phase 标记新 kind），
    // 每轮各只有一次发票与一次已发布决策——没有第二次副作用入口。
    let basis_phase: String = db
        .query_row(
            "SELECT json_extract(fact_json,'$.request.basis.phase') FROM agent_root_tick_receipts \
             WHERE root_run_id=?1 AND round=2 AND phase='request'",
            [&f.actor.root_run_id],
            |row| row.get(0),
        )
        .expect("请求事实可读");
    assert_eq!(basis_phase, NATIVE_HOMOGENEOUS_FRAME_KIND);
    let decision_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='decision'",
            [&f.actor.root_run_id],
            |row| row.get(0),
        )
        .expect("可读");
    assert_eq!(decision_rows, 2, "出生轮 + 一轮同质评估轮，各发布一次");
}

/// 同质决策信封（与 SPEC §2.1 同一形状）；refs 必须给满六个键。
fn homogeneous_decision(
    seed: &str,
    task_id: &str,
    action: &str,
    payload: serde_json::Value,
    refs: serde_json::Value,
) -> String {
    json!({
        "decision_id": format!(
            "dec-{}",
            &crate::agent_runtime::store::stable_hash(&format!("{seed}{action}"))[..24]
        ),
        "action": action,
        "parent_task_id": task_id,
        "payload": payload,
        "allowlist_refs": refs
    })
    .to_string()
}

#[test]
fn the_heartbeat_sweep_reclaims_expired_homogeneous_carriers() {
    // Phase 1.1 第 3 条：到期回收挂在现网 WorkerSupervisor 心跳扫描上。
    // 心跳第一步是 `validate_parent`，所以这里用带完整预算账本的现网 Root 夹具；
    // 加了 legacy 过滤之前，这条扫描会把同质载体送进旧提款写者并报
    // `assignment_expiry_original_state_conflict`，监护线程直接 fail。
    use crate::agent_runtime::multi_agent::homogeneous::{
        decompose::slice_id_for, pool, store, types,
    };
    let mut f = root_tick_fixture("homogeneous-sweep", "http://127.0.0.1:9/v1");
    let db = db::open(&f.context.db_path).unwrap();
    store::set_run_scheduling_model(&db, &f.actor.root_run_id, types::SCHEDULING_HOMOGENEOUS)
        .expect("写入开关");
    store::activate_task_for_lease(&db, &f.actor)
        .expect("激活")
        .expect("同质任务必须建立");
    let task = store::load_task_by_root_run(&db, &f.actor.root_run_id)
        .expect("任务可读")
        .expect("任务存在");
    let workers = pool::provision_workers(&db, &task, f.actor.lease_epoch, 1).expect("预置 Worker");
    let path = "/api/v1/slow";
    let slice_id = slice_id_for(
        &task.task_id,
        "path_prefix",
        path,
        task.evidence_revision_base,
    );
    crate::commands::native_homogeneous_apply_after_paid_tick(
        &db,
        &f.actor,
        &homogeneous_decision(
            "sweep-",
            &task.task_id,
            "decompose",
            json!({
                "slices": [json!({
                    "slice_id": slice_id,
                    "scope_kind": "path_prefix",
                    "scope_value": path,
                    "lane": "read_only_analysis",
                    "evidence_revision_base": task.evidence_revision_base,
                    "worker_capability_profile_id": types::PROFILE_HOMOGENEOUS_V1,
                    "max_retries": 2
                })],
                "decomposition_basis_zh": "单切片只读范围"
            }),
            json!({"slice_ids": [], "worker_ids": [], "tool_ids": [],
                   "target_keys": [task.target_key], "directive_ids": [], "assignment_ids": []}),
        ),
    )
    .expect("分解一轮");
    let task = store::load_task_by_root_run(&db, &f.actor.root_run_id)
        .expect("任务可读")
        .expect("任务存在");
    let task_id = task.task_id.clone();
    drop(db);
    let db = db::open(&f.context.db_path).unwrap();
    crate::commands::native_homogeneous_apply_after_paid_tick(
        &db,
        &f.actor,
        &homogeneous_decision(
            "sweep-assign-",
            &task_id,
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
            json!({"slice_ids": [slice_id], "worker_ids": [workers[0]],
                   "tool_ids": ["evidence.read"], "target_keys": [task.target_key],
                   "directive_ids": [], "assignment_ids": []}),
        ),
    )
    .expect("派发一轮");
    let assignment_id = store::assignment_id_for(
        &f.actor.root_run_id,
        &store::dedup_key_for(
            &slice_id,
            &task.target_key,
            task.evidence_revision_base,
            store::HOMOGENEOUS_TRIGGER_CODE,
        ),
    );
    let run_id: String = db
        .query_row(
            "SELECT child_run_id FROM agent_assignments WHERE id=?1",
            [&assignment_id],
            |row| row.get(0),
        )
        .expect("载体可读");
    assert_eq!(
        db.query_row(
            "SELECT state FROM agent_assignment_attempts WHERE child_run_id=?1",
            [&run_id],
            |row| row.get::<_, String>(0)
        )
        .expect("attempt 可读"),
        "leased"
    );
    // 只让载体到期：主管自己的租约必须仍然有效，否则心跳第一步就失败。
    db.execute(
        "UPDATE agent_assignment_attempts SET expires_at=datetime('now','localtime','-1 hour') WHERE child_run_id=?1",
        [&run_id],
    )
    .expect("置过期");
    drop(db);
    drop(f.parent.take());
    let supervisor = start_original_fixture_supervisor(&f.context.db_path, &f.actor)
        .expect("心跳必须容得下过期同质载体");
    drop(supervisor);

    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        store::load_assignment(&db, &assignment_id)
            .expect("a")
            .expect("有")
            .state,
        "cancelled"
    );
    assert_eq!(
        store::load_slice(&db, &slice_id)
            .expect("s")
            .expect("有")
            .state,
        "rejected"
    );
    assert_eq!(
        pool::load_worker(&db, &workers[0])
            .expect("w")
            .expect("有")
            .state,
        "dead"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM agent_runs WHERE id=?1",
            [&run_id],
            |row| row.get::<_, String>(0)
        )
        .expect("run 可读"),
        "terminal"
    );
    let open: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_assignment_attempts WHERE state IN ('leased','running','paused')",
            [],
            |row| row.get(0),
        )
        .expect("可读");
    assert_eq!(open, 0, "回收后不能留下 open attempt 虚占只读配额");
    let legacy_lane_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&assignment_id],
            |row| row.get(0),
        )
        .expect("可读");
    assert_eq!(legacy_lane_rows, 0, "同质载体全程不占旧互斥 lane");
    let released: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_lane_slots WHERE assignment_id=?1 AND released_at=''",
            [&assignment_id],
            |row| row.get(0),
        )
        .expect("可读");
    assert_eq!(released, 0, "slot 必须随回收释放");
}

#[test]
fn a_homogeneous_plan_steps_aside_from_the_legacy_bootstrap_dispatch() {
    // Phase 1.1 第 4 条：显式声明 homogeneous_workers 的扫描计划不得进入
    // legacy bootstrap dispatch——不建旧角色 child、不占旧 lane、不付 bootstrap 轮。
    use crate::agent_runtime::multi_agent::homogeneous::{apply, store};
    let mut f = root_tick_fixture("homogeneous-prepare-step-aside", "http://127.0.0.1:9/v1");
    let db = db::open(&f.context.db_path).unwrap();
    db.execute(
        "INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash) \
         VALUES(?1,?2,'web','native','{\"scheduling_model\":\"homogeneous_workers\"}','homog-plan') \
         ON CONFLICT(scan_id,attempt_number) DO UPDATE SET plan_json=excluded.plan_json",
        params![f.actor.scan_id, f.actor.attempt_number],
    )
    .expect("写入冻结计划");
    let receipts_before: i64 = db
        .query_row("SELECT COUNT(*) FROM agent_root_tick_receipts", [], |row| {
            row.get(0)
        })
        .expect("可读");
    drop(db);
    // prepare 自己认领 Root 调用所有权，夹具那份监护必须先释放（与其它 prepare 测例同形）。
    drop(f.parent.take());
    let error = match multi_agent_prepare(&mut f.context) {
        Ok(_) => panic!("同质计划必须让路，不能进入 legacy bootstrap dispatch"),
        Err(error) => error,
    };
    assert_eq!(error, "homogeneous_scan_legacy_bootstrap_stepped_aside");
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        store::run_scheduling_model(&db, &f.actor.root_run_id).expect("模式"),
        "homogeneous_workers"
    );
    assert!(
        apply::task_exists_for_root(&db, &f.actor.root_run_id).expect("任务存在"),
        "让路不等于丢掉同质任务：主管循环仍要能看到它"
    );
    for table in [
        "agent_assignments",
        "agent_lane_leases",
        "agent_capability_leases",
    ] {
        let rows: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("计数可读");
        assert_eq!(rows, 0, "{table} 不该被 legacy bootstrap 写入");
    }
    let receipts_after: i64 = db
        .query_row("SELECT COUNT(*) FROM agent_root_tick_receipts", [], |row| {
            row.get(0)
        })
        .expect("可读");
    assert_eq!(
        receipts_after, receipts_before,
        "让路必须发生在付费 bootstrap 轮之前"
    );
}

#[test]
fn a_legacy_plan_still_runs_the_bootstrap_dispatch_unchanged() {
    // Phase 1.1 第 6 条 legacy smoke：默认计划（冻结计划里没有 scheduling_model 声明）
    // 照旧走完 legacy prepare——bootstrap 付费轮 + Mapper/Executor 固定角色 child，
    // 旧 lane 互斥表照写，且绝不建立同质任务，让路分支一次都不执行。
    use crate::agent_runtime::contract::AgentRole;
    use crate::agent_runtime::multi_agent::homogeneous::{apply, store};
    // legacy prepare 会真的跑一个只读 specialist child，需要现网 specialist 传输。
    let _real = RealSpecialistTransport::enter();
    let (port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            if request.contains("You are the Root Coordinator")
                || request.contains("SPA/API Mapper")
            {
                changed_fact_response(&request, true)
            } else {
                model_round(&[], 10)
            },
        )
    }));
    let fixture = web_mode_fixture("multi", "https://authorized.example.test");
    let mut f = root_tick_fixture_from_web_mode_turns(
        "legacy-prepare-smoke",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (60_000, 20),
        None,
        fixture,
        Some(1),
    );
    drop(f.parent.take());
    let session = multi_agent_prepare(&mut f.context).expect("legacy prepare 必须照旧成功");
    assert_eq!(session.mapper.role, AgentRole::SpaApiMapper);
    assert_eq!(session.executor.role, AgentRole::WebExecutor);
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        store::run_scheduling_model(&db, &session.lease.root_run_id).expect("模式"),
        "legacy_role_pipeline",
        "默认值仍是 legacy_role_pipeline"
    );
    assert!(
        !apply::task_exists_for_root(&db, &session.lease.root_run_id).expect("无同质任务"),
        "legacy root 不得建立同质任务"
    );
    let assignments: i64 = db
        .query_row("SELECT COUNT(*) FROM agent_assignments", [], |row| {
            row.get(0)
        })
        .expect("可读");
    assert!(assignments >= 2, "legacy child 照旧建立：{assignments}");
    let legacy_model_rows: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE scheduling_model='legacy_role_pipeline'",
            [],
            |row| row.get(0),
        )
        .expect("可读");
    assert_eq!(legacy_model_rows, assignments);
    let lane_leases: i64 = db
        .query_row("SELECT COUNT(*) FROM agent_lane_leases", [], |row| {
            row.get(0)
        })
        .expect("可读");
    assert!(
        lane_leases > 0,
        "legacy 路径必须继续用旧互斥 lane，不被同质分支改动"
    );
}

#[test]
fn legacy_expiry_scan_ignores_homogeneous_carriers() {
    // Phase 1.1 第 3 条的第二半：`supervisor::sweep` 的到期谓词（due 与 children）
    // 只能挑 legacy 行。同质载体没有 agent_lane_leases、也不在旧状态机里，
    // 一旦被 `attempts/expiry.rs` 提款就会报 assignment_expiry_original_state_conflict。
    // 这里按生产 SQL 原文断言过滤效果（与 legacy_lane_occupancy_* 同一约定）。
    let (dir, connection, lease) = homogeneous_wiring_fixture("homogeneous_workers");
    let insert_assignment = |id: &str, model: &str, state: &str| {
        connection
            .execute(
                "INSERT INTO agent_assignments(id,coordinator_run_id,child_run_id,role,lane,target_key,state,dedup_key,trigger_code,evidence_revision,scheduling_model,homogeneous_worker_id)\
                 VALUES(?1,?2,?3,'deep_investigator','read_only_analysis',?4,?5,?6,'homogeneous-dispatch',1,?7,?8)",
                rusqlite::params![
                    id,
                    lease.root_run_id,
                    format!("run-{id}"),
                    lease.target_key,
                    state,
                    format!("dedup-{id}"),
                    model,
                    if model == "homogeneous_workers" { "wrk-homog" } else { "" }
                ],
            )
            .expect("assignment");
    };
    let insert_expired_attempt = |id: &str, model: &str| {
        connection
            .execute(
                "INSERT INTO agent_assignment_attempts(id,root_run_id,assignment_id,child_run_id,\
                 coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,state,expires_at)\
                 VALUES(?1,?2,?3,?4,?5,?6,1,?7,?8,'leased',datetime('now','localtime','-1 hour'))",
                rusqlite::params![
                    format!("att-{id}"),
                    lease.root_run_id,
                    id,
                    format!("run-{id}"),
                    lease.lease_epoch,
                    lease.fencing_token,
                    format!("fence-{model}"),
                    format!("worker-{model}")
                ],
            )
            .expect("attempt");
    };
    insert_assignment("asg-homog", "homogeneous_workers", "assigned");
    insert_expired_attempt("asg-homog", "homogeneous_workers");
    insert_assignment("asg-legacy", "legacy_role_pipeline", "running");
    insert_expired_attempt("asg-legacy", "legacy_role_pipeline");

    // 不加过滤时两条都会被挑出来：证明这条同质行确实是「已过期且 leased」。
    let unfiltered: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments a \
             JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id \
             WHERE a.coordinator_run_id=?1 AND x.state IN ('leased','running','paused') \
             AND COALESCE(datetime(x.expires_at)<=datetime('now','localtime'),1)",
            [&lease.root_run_id],
            |row| row.get(0),
        )
        .expect("可读");
    assert_eq!(unfiltered, 2);

    // 生产谓词（带 scheduling_model 过滤）只认 legacy 那一条。
    let due: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a \
             JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id \
             WHERE a.coordinator_run_id=?1 AND x.state IN ('leased','running','paused') \
             AND a.scheduling_model='legacy_role_pipeline' \
             AND COALESCE(datetime(x.expires_at)<=datetime('now','localtime'),1))",
            [&lease.root_run_id],
            |row| row.get(0),
        )
        .expect("due 谓词");
    assert!(due, "legacy 过期行必须仍然触发提款");
    let selected: Vec<String> = connection
        .prepare(
            "SELECT a.id FROM agent_assignments a \
             JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=a.child_run_id \
             WHERE a.coordinator_run_id=?1 AND x.state IN ('leased','running','paused') \
             AND a.scheduling_model='legacy_role_pipeline' \
             AND COALESCE(datetime(x.expires_at)<=datetime('now','localtime'),1) ORDER BY a.id",
        )
        .expect("语句")
        .query_map([&lease.root_run_id], |row| row.get::<_, String>(0))
        .expect("查询")
        .collect::<Result<Vec<_>, _>>()
        .expect("行");
    assert_eq!(
        selected,
        vec!["asg-legacy".to_string()],
        "legacy 提款扫描绝不能挑走同质载体"
    );
    drop(connection);
    let _ = std::fs::remove_dir_all(dir);
}
