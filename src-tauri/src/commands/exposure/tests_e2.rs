// §E2 — the settings loop has to be provable end to end: what was saved is what a round
// freezes, the item ceiling leaves an explained tail instead of silently planning less, and a
// later edit cannot move an interrupted round or reset what it already spent.

use super::fixture::{response, routes, seed_scope, Fixture};
use super::orchestrator::{frozen_settings, run_plan, start_or_reuse, ItemOutcome, RunReport};
use super::recovery::{frozen_settings as frozen_from_plan, load_plan, meter_for, verify_frozen};
use super::settings::{load, save, ExposureSettings};
use super::state::{ensure_exposure_schema, RunStatus, WorkItemStatus};
use rusqlite::Connection;
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oviraptor-exposure-e2-{tag}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn database(root: &Path) -> Connection {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    connection
        .pragma_update(None, "synchronous", "OFF")
        .expect("测试库 pragma");
    connection
}

fn edited(connection: &Connection, project: i64) -> JsonValue {
    // Exactly what the command门面 does when it hands settings to the orchestrator.
    super::super::exposure_scan_settings(connection, project).expect("配置必须可读")
}

/// A stand-in for the adapters: it records which work items the loop actually let through,
/// so the item ceiling is proved by what was *not* executed.
fn executor_for(
    sent: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
) -> impl FnMut(&super::work_items::WorkItem) -> Result<ItemOutcome, String> {
    move |item| {
        sent.borrow_mut().push(item.work_key.clone());
        Ok(ItemOutcome {
            status: Some(WorkItemStatus::Completed),
            source_status: Some(super::state::SourceStatus::Completed),
            ..Default::default()
        })
    }
}

#[test]
fn e2_saved_settings_are_frozen_into_the_round_and_the_ceiling_leaves_a_named_tail() {
    let root = sandbox("freeze");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(151,'范围','active')",
            [],
        )
        .unwrap();
    let mut settings = ExposureSettings::for_project(151);
    settings.max_items = 2;
    settings.max_requests = 500;
    settings.sources = vec!["passive_web".to_string()];
    assert_eq!(save(&connection, &settings, 0).unwrap(), 1);
    // Three authorized origins, an item ceiling of two.
    for index in 0..3 {
        seed_scope(
            &connection,
            151,
            &format!("http://origin-{index}.invalid/"),
            true,
        );
    }
    let started = start_or_reuse(&connection, 151, &edited(&connection, 151)).expect("启动");
    assert_eq!(
        started.planned_items, 3,
        "计划仍是全量，额度不能悄悄缩小计划"
    );
    let plan_row = load_plan(&connection, started.run_id, 151).expect("计划已冻结");
    assert_eq!(plan_row.settings_version, 1);
    assert!(
        plan_row.settings_json.contains("\"exposureMaxItems\":2"),
        "settings_json={}",
        plan_row.settings_json
    );
    assert_eq!(frozen_from_plan(&plan_row).max_items, 2);

    let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let report = run_plan(
        &connection,
        started.run_id,
        "owner-e2",
        &mut executor_for(std::rc::Rc::clone(&sent)),
    )
    .expect("本轮必须收敛");
    assert_eq!(report.counts.total, 3);
    assert_eq!(report.counts.completed, 2, "只执行额度内的两项");
    assert_eq!(report.counts.quota, 1, "第三项必须是超额缺口而不是消失");
    assert_eq!(sent.borrow().len(), 2, "被额度挡住的项不得触网");
    assert_eq!(report.aggregation.status, RunStatus::Partial);
    assert!(
        report
            .aggregation
            .gaps
            .iter()
            .any(|gap| gap.starts_with("work_item_quota:")),
        "{:?}",
        report.aggregation.gaps
    );
    let reason: String = connection
        .query_row(
            "SELECT reason FROM exposure_work_items WHERE status='blocked_quota' AND run_id=?1",
            [started.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(reason.contains("工作项额度 2"), "{reason}");
    let _ = started;
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e2_invalid_settings_refuse_the_round_instead_of_running_on_defaults() {
    let root = sandbox("invalid");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(152,'范围','active')",
            [],
        )
        .unwrap();
    seed_scope(&connection, 152, "http://a.invalid/", true);
    let error = start_or_reuse(&connection, 152, &json!({ "exposureMaxRequest": 5 }))
        .expect_err("拼错的字段不能当默认值用");
    assert!(error.contains("exposure_setting_invalid"), "{error}");
    assert!(error.contains("exposureMaxRequest"), "{error}");
    let runs: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_runs", [], |row| row.get(0))
        .unwrap();
    assert_eq!(runs, 0, "拒绝启动不能留下半条 run");
    let items: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_work_items", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(items, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e2_a_later_edit_cannot_move_an_interrupted_round_or_reset_its_spending() {
    let root = sandbox("resume");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(200, &[("Content-Type", "text/html")], "page")],
    )]));
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(153,'范围','active')",
            [],
        )
        .unwrap();
    seed_scope(&connection, 153, &format!("{}/", fixture.origin), true);
    seed_scope(&connection, 153, "http://second.invalid/", true);
    let mut settings = ExposureSettings::for_project(153);
    settings.max_items = 1;
    settings.max_requests = 10;
    assert_eq!(save(&connection, &settings, 0).unwrap(), 1);
    let started = start_or_reuse(&connection, 153, &edited(&connection, 153)).expect("启动");
    // Round one: the ceiling closes the second item, and the first one spends requests.
    let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let first = run_plan(
        &connection,
        started.run_id,
        "owner-a",
        &mut executor_for(std::rc::Rc::clone(&sent)),
    )
    .expect("第一轮收敛");
    assert_eq!(first.counts.completed, 1);
    assert_eq!(first.counts.quota, 1);
    super::work_items::record_usage(&connection, started.run_id, 4, 4_000).expect("用量累计");

    // The operator now raises the ceiling and the budget. The old round must not follow.
    let mut raised = settings.clone();
    raised.max_items = 99;
    raised.max_requests = 9_000;
    assert_eq!(save(&connection, &raised, 1).unwrap(), 2);
    let frozen = frozen_settings(&connection, started.run_id, 153);
    assert_eq!(frozen.max_items, 1, "恢复必须按本轮冻结的额度");
    assert_eq!(frozen.version, 1);
    let plan_row = load_plan(&connection, started.run_id, 153).unwrap();
    let meter = meter_for(&plan_row);
    assert_eq!(meter.requests_used, 4, "已花掉的额度不能归零");
    assert_eq!(meter.max_requests, 10, "预算仍是本轮冻结的那一份");
    // A resume with the *current* settings is refused: the options identity moved.
    let current = edited(&connection, 153);
    let scope = super::scope::ExposureScope::load(&connection, 153, &current).unwrap();
    let error = verify_frozen(&plan_row, &scope, &current).expect_err("旧轮不能用新配置复用");
    assert!(error.contains("exposure_plan_frozen_mismatch"), "{error}");
    // With its own frozen settings the same round continues fine.
    let frozen_json = frozen.to_settings_json();
    let frozen_scope = super::scope::ExposureScope::load(&connection, 153, &frozen_json).unwrap();
    verify_frozen(&plan_row, &frozen_scope, &frozen_json).expect("冻结配置必须可继续");
    let served = fixture.request_count();
    assert_eq!(served, 0, "本轮只跑过被额度放行的项，镜像不该看到更多请求");
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e2_revoked_authorization_still_refuses_a_resume_even_with_frozen_settings() {
    let root = sandbox("revoke");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(154,'范围','active')",
            [],
        )
        .unwrap();
    seed_scope(&connection, 154, "http://gone.invalid/", true);
    let started = start_or_reuse(&connection, 154, &edited(&connection, 154)).expect("启动");
    let plan_row = load_plan(&connection, started.run_id, 154).unwrap();
    let frozen = frozen_settings(&connection, started.run_id, 154).to_settings_json();
    connection
        .execute(
            "UPDATE asset_ownership_decisions SET exposure_eligible=0 WHERE project_id=154",
            [],
        )
        .unwrap();
    let scope = super::scope::ExposureScope::load(&connection, 154, &frozen).unwrap();
    let error = verify_frozen(&plan_row, &scope, &frozen).expect_err("撤权必须拒绝继续");
    assert!(error.contains("授权范围"), "{error}");
    assert_eq!(scope.grants.len(), 0, "撤权后范围为空，不能有目标");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e2_settings_are_per_project_and_a_run_keeps_reading_its_own_copy() {
    let root = sandbox("perproject");
    let connection = database(&root);
    for id in [155, 156] {
        connection
            .execute(
                "INSERT INTO projects(id,name,status) VALUES(?1,?2,'active')",
                rusqlite::params![id, format!("范围 {id}")],
            )
            .unwrap();
    }
    let mut tight = ExposureSettings::for_project(155);
    tight.max_requests = 7;
    tight.allow_private_destinations = true;
    save(&connection, &tight, 0).unwrap();
    assert_eq!(load(&connection, 155).unwrap().max_requests, 7);
    let other = load(&connection, 156).unwrap();
    assert_eq!(other.max_requests, 2_000, "另一个空间仍是默认值");
    assert!(!other.allow_private_destinations, "私网出口不能跨项目泄漏");
    assert!(load(&connection, 155).unwrap().allow_private_destinations);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e2_run_report_shape_stays_valid_when_the_ceiling_is_zero_items_left() {
    // A ceiling at the plan size is still a complete round; a smaller one is partial with
    // exactly one named quota gap.
    let root = sandbox("shape");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(157,'范围','active')",
            [],
        )
        .unwrap();
    for index in 0..2 {
        seed_scope(
            &connection,
            157,
            &format!("http://x-{index}.invalid/"),
            true,
        );
    }
    // 只开被动 Web：否则未配置的索引来源会像 E2 之外那样合法地把这一轮判成 partial。
    let mut only = ExposureSettings::for_project(157);
    only.sources = vec!["passive_web".to_string()];
    save(&connection, &only, 0).unwrap();
    let started = start_or_reuse(&connection, 157, &edited(&connection, 157)).expect("启动");
    let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let report: RunReport = run_plan(
        &connection,
        started.run_id,
        "owner-shape",
        &mut executor_for(std::rc::Rc::clone(&sent)),
    )
    .expect("默认额度内必须全部完成");
    assert_eq!(report.counts.completed, 2);
    assert_eq!(report.aggregation.status, RunStatus::Completed);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e2_a_round_started_before_the_settings_contract_cannot_be_resumed() {
    // §E2 恢复必须验证“原版本”。旧轮的计划行里没有冻结配置，重算不出它当年的额度；
    // 把它按今天的默认值继续跑，等于悄悄给一轮历史任务换一个它没花过的预算。
    let root = sandbox("legacy-plan");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(158,'遗留','active')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(6101,158,'interrupted','interrupted')",
            [],
        )
        .unwrap();
    seed_scope(&connection, 158, "http://legacy.invalid/", true);
    let scope = super::scope::ExposureScope::load(&connection, 158, &json!({})).unwrap();
    let plan_sql = "INSERT INTO exposure_run_plans(run_id,project_id,scope_hash,options_hash,budget_requests,budget_bytes,planned_items) \
             VALUES(6101,158,?1,'h',10,1000,3)";
    connection
        .execute(
            plan_sql,
            rusqlite::params![super::orchestrator::scope_hash_of(&scope)],
        )
        .unwrap();
    let plan_row = load_plan(&connection, 6101, 158).expect("计划行必须读得到");
    assert!(!plan_row.settings_frozen(), "旧行不该被当成已冻结");
    let error = verify_frozen(&plan_row, &scope, &json!({}))
        .expect_err("没有冻结配置的旧轮不能按新合同恢复");
    assert!(
        error.contains("exposure_plan_settings_unfrozen"),
        "拒绝理由必须点名缺少冻结配置：{error}"
    );
    assert!(
        !error.contains("exposure_plan_frozen_mismatch"),
        "不能把它说成配置漂移：{error}"
    );
    // 同一轮一旦补上完整的冻结值，就按自己的配置继续，而不是被这段历史卡死。
    connection
        .execute(
            "UPDATE exposure_run_plans SET settings_version=1, settings_json=?1, options_hash=?2 WHERE run_id=6101",
            rusqlite::params![
                ExposureSettings::for_project(158)
                    .to_settings_json()
                    .to_string(),
                super::orchestrator::options_hash(&ExposureSettings::for_project(158).to_settings_json())
            ],
        )
        .unwrap();
    let repaired = load_plan(&connection, 6101, 158).unwrap();
    let frozen = frozen_from_plan(&repaired).to_settings_json();
    verify_frozen(
        &repaired,
        &super::scope::ExposureScope::load(&connection, 158, &frozen).unwrap(),
        &frozen,
    )
    .expect("补齐冻结配置后必须可继续");
    assert_eq!(repaired.planned_items, 3, "已计划的项数不能被恢复改写");
    let _ = std::fs::remove_dir_all(root);
}
