// §Q3 — durable work items, atomic start, and crash recovery, proved on a temporary SQLite
// file and a localhost fixture. "Recovered" is measured by what the fixture was asked for
// after the restart, not by a row that merely says running.

use super::adapters::http::{PassiveClient, PassiveHttpLimits, RequestMeter};
use super::adapters::passive_web::{execute as walk_origin, WalkBudget};
use super::fixture::{response, routes, seed_scope, Fixture};
use super::orchestrator::{
    cancel_requested, run_plan, start_or_reuse, ItemOutcome, RunReport, KIND_PASSIVE_WEB,
};
use super::recovery::{
    legacy_projection, load_plan, mark_interrupted_if_dead, meter_for, verify_frozen,
};
use super::scope::{ExposureDenial, ExposureScope};
use super::state::{ensure_exposure_schema, RunStatus, WorkItemStatus};
use super::work_items::{
    claim_next, counts, finish, plan, reclaim_expired, record_usage, usage, WorkItemSeed,
};
use rusqlite::Connection;
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const PROJECT: i64 = 11;

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oviraptor-exposure-q3-{tag}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn database(root: &Path) -> Connection {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("临时库连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    connection
}

fn settings(extra: JsonValue) -> JsonValue {
    let mut base = json!({
        "exposureAllowPrivateDestinations": true,
        "exposureSources": ["passive_web"],
        "exposureMaxRequests": 200,
        "exposureMaxTotalBytes": 10_000_000
    });
    if let (Some(map), Some(extra_map)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra_map {
            map.insert(key.clone(), value.clone());
        }
    }
    base
}

fn scope_for(connection: &Connection, project: i64, config: &JsonValue) -> ExposureScope {
    ExposureScope::load(connection, project, config).expect("范围必须能装载")
}

fn client(cap: usize) -> PassiveClient {
    PassiveClient::new(PassiveHttpLimits {
        max_response_bytes: cap,
        timeout: std::time::Duration::from_secs(5),
        ..PassiveHttpLimits::default()
    })
    .expect("客户端必须可用")
}

fn seeds(keys: &[String]) -> Vec<WorkItemSeed> {
    keys.iter()
        .map(|key| WorkItemSeed {
            work_key: key.clone(),
            kind: KIND_PASSIVE_WEB.to_string(),
            source_key: KIND_PASSIVE_WEB.to_string(),
            asset_id: None,
            origin: format!("http://{key}"),
            entity: key.clone(),
            input_hash: key.clone(),
        })
        .collect()
}

/// One round of the frozen plan. The scope is re-read before every send, so a revocation
/// between two items stops the next request instead of waiting for the next run.
fn run_round(
    connection: &Connection,
    config: &JsonValue,
    run_id: i64,
    owner: &str,
    project: i64,
) -> Result<RunReport, String> {
    let scope = scope_for(connection, project, config);
    let fetcher = client(8_000);
    let mut meter = RequestMeter::for_scope(&scope);
    let budget = WalkBudget::default();
    let mut executor = |item: &super::work_items::WorkItem| -> Result<ItemOutcome, String> {
        let origin = item.origin.clone();
        let revalidate = || -> Result<(), ExposureDenial> {
            let fresh = ExposureScope::load(connection, project, config)?;
            let url = reqwest::Url::parse(&format!("{origin}/"))
                .map_err(|error| ExposureDenial::new("origin_invalid", error.to_string()))?;
            fresh.authorize(&url, &super::state::utc_now()).map(|_| ())
        };
        Ok(walk_origin(
            &fetcher,
            &scope,
            item,
            &mut meter,
            &budget,
            &revalidate,
            &|| cancel_requested(connection, run_id),
        ))
    };
    run_plan(connection, run_id, owner, &mut executor)
}

fn started(
    root: &Path,
    table: Vec<(&str, Vec<String>)>,
    extra: JsonValue,
) -> (Connection, JsonValue, i64, Fixture) {
    let connection = database(root);
    let fixture = Fixture::start(routes(table));
    seed_scope(&connection, PROJECT, &format!("{}/", fixture.origin), true);
    let config = settings(extra);
    let started = start_or_reuse(&connection, PROJECT, &config).expect("启动必须冻结计划");
    (connection, config, started.run_id, fixture)
}

/// Makes every claim of a round look expired without touching the run row.
fn age_claims(connection: &Connection, run_id: i64) {
    connection
        .execute(
            "UPDATE exposure_work_items SET heartbeat_at='2020-01-01 00:00:00' WHERE run_id=?1",
            [run_id],
        )
        .unwrap();
}

#[test]
fn q3_two_simultaneous_starters_get_one_active_run() {
    let root = sandbox("start");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(
            200,
            &[("Content-Type", "text/html")],
            "<a href=\"/a\">a</a>",
        )],
    )]));
    seed_scope(&connection, 12, &format!("{}/", fixture.origin), true);
    let config = settings(json!({}));
    start_or_reuse(&connection, 12, &config).expect("首次启动");
    let second = start_or_reuse(&connection, 12, &config).expect("第二次启动必须复用");
    assert!(second.reused_active, "并发启动不能产生第二条 run");
    let rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_runs WHERE project_id=12 AND status IN ('queued','running')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);

    // Four threads at the same moment: the partial unique index is the tie-breaker.
    let db_path = connection
        .path()
        .map(|value| PathBuf::from(value.to_string()))
        .expect("临时库必须有路径");
    let results = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..4 {
            let path = db_path.clone();
            let config = config.clone();
            handles.push(scope.spawn(move || {
                let connection = crate::db::open(&path).expect("并发连接");
                ensure_exposure_schema(&connection).ok();
                start_or_reuse(&connection, 12, &config).map(|value| value.run_id)
            }));
        }
        handles
            .into_iter()
            .map(|handle| handle.join().expect("线程必须结束"))
            .collect::<Vec<Result<i64, String>>>()
    });
    let ids: Vec<i64> = results
        .iter()
        .filter_map(|row| row.as_ref().ok().copied())
        .collect();
    assert!(!ids.is_empty(), "至少要有一个启动成功：{results:?}");
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        1,
        "同时启动必须返回同一个 run：{ids:?}"
    );
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q3_the_plan_materializes_every_grant_and_names_the_unreachable_tail() {
    const CHILD: &str = "OVIRAPTOR_TEST_Q3_UNREACHABLE_LOOPBACK_CHILD";
    const TEST: &str = "commands::exposure::tests_q3::q3_the_plan_materializes_every_grant_and_names_the_unreachable_tail";
    if std::env::var_os(CHILD).is_none() {
        // Keep loopback attempts out of host proxy configuration without
        // changing the environment of any concurrent test in this process.
        let mut command = Command::new(std::env::current_exe().expect("测试二进制路径"));
        command.args(["--exact", TEST, "--test-threads=1"])
            .env(CHILD, "1").env("NO_PROXY", "*").env("no_proxy", "*");
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"] {
            command.env_remove(key);
        }
        let output = command.output().expect("Q3 本机不可达目标子进程必须启动");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success() && stdout.contains("1 passed; 0 failed"),
            "{TEST}: isolated loopback child failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr));
        return;
    }
    let root = sandbox("plan");
    let connection = database(&root);
    // 250 origins: the old collector quietly kept 200 of them.
    // Literal loopback skips DNS; destination port zero cannot host a listener.
    for index in 0..250 {
        seed_scope(
            &connection,
            21,
            &format!("http://127.0.1.{}:0/", index + 1),
            true,
        );
    }
    let config = settings(json!({"exposureMaxRequests": 5}));
    let started = start_or_reuse(&connection, 21, &config).expect("计划启动");
    assert_eq!(
        started.planned_items, 250,
        "计划必须是全量工作项，不是静默截断"
    );
    assert_eq!(started.grants, 250);
    let scope = scope_for(&connection, 21, &config);
    let fetcher = client(4_000);
    let mut meter = RequestMeter::for_scope(&scope);
    let budget = WalkBudget::default();
    let report = run_plan(&connection, started.run_id, "owner-1", &mut |item| {
        let origin = item.origin.clone();
        let revalidate = || -> Result<(), ExposureDenial> {
            let url = reqwest::Url::parse(&format!("{origin}/"))
                .map_err(|error| ExposureDenial::new("origin_invalid", error.to_string()))?;
            scope.authorize(&url, &super::state::utc_now()).map(|_| ())
        };
        Ok(walk_origin(
            &fetcher,
            &scope,
            item,
            &mut meter,
            &budget,
            &revalidate,
            &|| false,
        ))
    })
    .expect("计划必须跑到终态");
    assert_eq!(report.counts.total, 250);
    assert_eq!(report.counts.open, 0, "所有工作项都要有终态");
    assert_eq!(report.counts.completed, 0, "无法解析的地址不能算完成");
    assert!(report.counts.failed > 0, "被真实尝试过的项必须留下失败原因");
    assert!(
        report.counts.quota > 0,
        "额度用尽后的尾部必须记为超额而不是完成：{:?}",
        report.counts
    );
    assert_eq!(report.aggregation.status, RunStatus::Partial);
    let gaps: Vec<&String> = report.aggregation.gaps.iter().collect();
    assert!(
        gaps.iter().any(|gap| gap.starts_with("work_item_quota")),
        "超额缺口必须点名：{gaps:?}"
    );
    let reason: String = connection
        .query_row(
            "SELECT reason FROM exposure_work_items WHERE status='blocked_quota' AND run_id=?1 LIMIT 1",
            [started.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(reason.contains("额度"), "{reason}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q3_an_unauthorized_destination_is_named_not_fetched() {
    let root = sandbox("denied");
    let (connection, _config, run_id, fixture) = started(
        &root,
        vec![(
            "/",
            vec![response(200, &[("Content-Type", "text/html")], "page")],
        )],
        json!({"exposureAllowPrivateDestinations": false}),
    );
    let served = fixture.request_count();
    let config = settings(json!({"exposureAllowPrivateDestinations": false}));
    let report = run_round(&connection, &config, run_id, "owner-1", PROJECT).expect("必须收敛");
    assert_eq!(report.counts.authorization, 1, "拒绝必须记为授权缺口");
    assert_eq!(report.counts.completed, 0);
    assert_eq!(fixture.request_count(), served, "越界地址一个包都不能发");
    assert!(report
        .aggregation
        .gaps
        .iter()
        .any(|gap| gap.starts_with("work_item_authorization:")));
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q3_a_late_worker_cannot_close_an_item_it_no_longer_owns() {
    let root = sandbox("lease");
    let connection = database(&root);
    let run_id = 7_001;
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(31,'范围','active')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(7001,31,'running','executing')",
            [],
        )
        .unwrap();
    plan(
        &connection,
        run_id,
        31,
        1,
        &seeds(&["one".to_string(), "two".to_string()]),
    )
    .expect("计划写入");
    let first = claim_next(&connection, run_id, "slow", 3_600)
        .unwrap()
        .expect("必须领到一项");
    assert_eq!(first.work_key, "one");
    // The owner stops beating; the lease expires and another worker takes over.
    age_claims(&connection, run_id);
    reclaim_expired(&connection, run_id, 60).expect("过期认领必须回收");
    let taken = claim_next(&connection, run_id, "fast", 3_600)
        .unwrap()
        .expect("接管者必须能领到同一项");
    assert_eq!(taken.id, first.id);
    assert_eq!(taken.claim_version, first.claim_version + 1);
    assert!(
        !finish(
            &connection,
            first.id,
            "slow",
            first.claim_version,
            WorkItemStatus::Completed,
            "",
            0,
            0,
            0
        )
        .expect("写入本身不能失败"),
        "失去认领的 worker 不能写终态"
    );
    assert!(finish(
        &connection,
        taken.id,
        "fast",
        taken.claim_version,
        WorkItemStatus::Completed,
        "",
        0,
        1,
        10,
    )
    .expect("接管者必须能收尾"));
    let closed = counts(&connection, run_id).unwrap();
    assert_eq!(closed.completed, 1);
    assert_eq!(closed.orphaned, 0, "回收后的项目不能留在孤儿状态");
    let still_open = claim_next(&connection, run_id, "fast", 3_600).unwrap();
    assert_eq!(
        still_open.map(|item| item.work_key),
        Some("two".to_string())
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q3_a_crash_after_a_closed_item_does_not_fetch_it_again() {
    let root = sandbox("crash");
    let connection = database(&root);
    let closed = Fixture::start(routes(vec![(
        "/",
        vec![response(
            200,
            &[("Content-Type", "text/html")],
            "first origin",
        )],
    )]));
    let abandoned = Fixture::start(routes(vec![(
        "/",
        vec![response(
            200,
            &[("Content-Type", "text/html")],
            "second origin",
        )],
    )]));
    // Work items are claimed by work_key, whose shared prefix is followed by
    // origin. Ephemeral port allocation does not preserve fixture creation order.
    let (closed, abandoned) = if closed.origin < abandoned.origin {
        (closed, abandoned)
    } else {
        (abandoned, closed)
    };
    seed_scope(&connection, PROJECT, &format!("{}/", closed.origin), true);
    seed_scope(
        &connection,
        PROJECT,
        &format!("{}/", abandoned.origin),
        true,
    );
    let config = settings(json!({}));
    let started = start_or_reuse(&connection, PROJECT, &config).expect("启动");
    assert_eq!(started.planned_items, 2);

    // Round one closes the first item and then the process dies on the second one.
    let scope = scope_for(&connection, PROJECT, &config);
    let fetcher = client(8_000);
    let mut meter = RequestMeter::for_scope(&scope);
    let budget = WalkBudget::default();
    let mut calls = 0usize;
    let crashed = run_plan(&connection, started.run_id, "owner-a", &mut |item| {
        calls += 1;
        if calls > 1 {
            return Err("模拟进程在第二项期间退出".to_string());
        }
        let origin = item.origin.clone();
        let revalidate = || -> Result<(), ExposureDenial> {
            let url = reqwest::Url::parse(&format!("{origin}/"))
                .map_err(|error| ExposureDenial::new("origin_invalid", error.to_string()))?;
            scope.authorize(&url, &super::state::utc_now()).map(|_| ())
        };
        Ok(walk_origin(
            &fetcher,
            &scope,
            item,
            &mut meter,
            &budget,
            &revalidate,
            &|| false,
        ))
    });
    assert!(crashed.is_err(), "执行器报错必须让本轮失败而不是假装完成");
    let after = counts(&connection, started.run_id).expect("计数可读");
    assert_eq!(after.total, 2);
    assert_eq!(after.completed, 1, "已闭合的一项必须保持闭合");
    assert_eq!(after.open, 1, "崩溃的那一项必须仍未闭合");

    // Restart: the expired claim is taken over and only the unfinished object is fetched.
    age_claims(&connection, started.run_id);
    assert!(
        mark_interrupted_if_dead(&connection, started.run_id, 60).expect("中断判定"),
        "没有活跃认领时必须标 interrupted"
    );
    let closed_served = closed.request_count();
    let abandoned_served = abandoned.request_count();
    let report = run_round(&connection, &config, started.run_id, "owner-b", PROJECT)
        .unwrap_or_else(|error| panic!("恢复必须跑完：{error}"));
    assert_eq!(report.counts.completed, 2);
    assert_eq!(report.aggregation.status, RunStatus::Completed);
    assert_eq!(
        closed.request_count(),
        closed_served,
        "已闭合工作项不能因为重启被重新抓取"
    );
    assert!(
        abandoned.request_count() > abandoned_served,
        "未完成项必须被补齐"
    );
    drop(closed);
    drop(abandoned);
    let _ = std::fs::remove_dir_all(root);
}

/// A real SIGKILL, not a simulated one: the child is a second instance of this test binary
/// that claims the second work item, writes a file to say so, and is then killed while its
/// claim is still held. Recovery is measured by what a new owner has to do afterwards.
const KILL_DB: &str = "OVIRAPTOR_TEST_EXPOSURE_KILL_DB";
const KILL_MARKER: &str = "OVIRAPTOR_TEST_EXPOSURE_KILL_MARKER";

#[test]
fn q3_sigkill_of_a_worker_between_claim_and_close_leaves_the_plan_resumable() {
    if std::env::var_os(KILL_DB).is_some() {
        exposure_sigkill_child_driver();
        return;
    }
    let root = sandbox("sigkill");
    std::fs::create_dir_all(&root).unwrap();
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    {
        let connection = crate::db::open(&db_path).expect("连接");
        ensure_exposure_schema(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO projects(id,name,status) VALUES(131,'范围','active')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(9101,131,'queued','planned')",
                [],
            )
            .unwrap();
        connection
            .execute(
                // G1-0: production always freezes settings; run_plan refuses a plan without them.
                "INSERT INTO exposure_run_plans(run_id,project_id,sources_json,budget_requests,budget_bytes,planned_items,settings_version,settings_json)
                 VALUES(9101,131,'[\"passive_web\"]',1000,1000000,2,1,'{\"exposureSources\":[\"passive_web\"]}')",
                [],
            )
            .unwrap();
        plan(
            &connection,
            9101,
            131,
            1,
            &seeds(&["one".to_string(), "two".to_string()]),
        )
        .expect("计划写入");
    }
    let marker = root.join("claimed-second");
    let mut child = Command::new(std::env::current_exe().expect("测试二进制路径"))
        .args([
            "--exact",
            "commands::exposure::tests_q3::q3_sigkill_of_a_worker_between_claim_and_close_leaves_the_plan_resumable",
            "--test-threads=1",
        ])
        .env(KILL_DB, &db_path)
        .env(KILL_MARKER, &marker)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("子进程必须能启动");
    let mut waited = 0u32;
    while !marker.exists() {
        std::thread::sleep(std::time::Duration::from_millis(50));
        waited += 1;
        if waited > 240 {
            let _ = child.kill();
            let _ = child.wait();
            panic!("子进程没有在 12 秒内认领第二个工作项");
        }
    }
    // SIGKILL: the child cannot run any cleanup, Drop cannot roll back anything.
    let _ = child.kill();
    let _ = child.wait();

    let connection = crate::db::open(&db_path).expect("重启后连接");
    let counted = counts(&connection, 9101).expect("计数可读");
    assert_eq!(counted.total, 2);
    assert_eq!(counted.completed, 1, "已闭合的一项必须已提交：{counted:?}");
    assert_eq!(counted.open, 1, "被杀进程持有的认领必须仍是未闭合");
    let committed: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE run_id=9101",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(committed, 1, "子进程提交过的观察必须还在：{committed}");

    // A new owner takes over after the lease expires and only the unfinished object is redone.
    age_claims(&connection, 9101);
    assert!(mark_interrupted_if_dead(&connection, 9101, 60).expect("判定"));
    let config = settings(json!({}));
    let report = {
        let scope = scope_for(&connection, 131, &config);
        let _ = client(8_000);
        let _ = RequestMeter::for_scope(&scope);
        let mut handled = Vec::new();
        let report = run_plan(&connection, 9101, "owner-after-kill", &mut |item| {
            handled.push(item.work_key.clone());
            Ok(ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                source_status: Some(super::state::SourceStatus::Completed),
                ..Default::default()
            })
        })
        .expect("SIGKILL 之后必须能继续");
        assert_eq!(handled, vec!["two".to_string()], "只能重做未完成的那一项");
        report
    };
    assert_eq!(report.counts.completed, 2);
    assert_eq!(report.aggregation.status, RunStatus::Completed);
    let still_committed: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE run_id=9101",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(still_committed, 1, "恢复不得重复写入已提交的观察");
    let _ = std::fs::remove_dir_all(root);
}

/// What the child instance actually runs. Selected by name with `--exact`, never in a
/// normal test pass.
fn exposure_sigkill_child_driver() {
    let db_path = PathBuf::from(std::env::var(KILL_DB).expect("子进程必须带库路径"));
    let marker = PathBuf::from(std::env::var(KILL_MARKER).expect("子进程必须带标记路径"));
    let connection = crate::db::open(&db_path).expect("子进程连接");
    let mut first = true;
    let _ = run_plan(&connection, 9101, "child-victim", &mut |item| {
        if first {
            first = false;
            let mut observation = super::observations::NewObservation::new(
                "passive_web",
                "resource",
                "https://one.invalid/",
            );
            observation.body = b"first".to_vec();
            observation.body_bytes = 5;
            let (id, _) =
                super::observations::record(&connection, 9101, 131, Some(item.id), &observation)
                    .expect("观察必须能提交");
            super::observations::merge_finding(
                &connection,
                9101,
                131,
                &super::observations::FindingDraft {
                    category: "document".to_string(),
                    title: "子进程看到的线索".to_string(),
                    url: "https://one.invalid/".to_string(),
                    source_type: "passive_web".to_string(),
                    evidence: "已提交".to_string(),
                    severity: "medium".to_string(),
                    confidence: 60,
                    asset_id: None,
                    content_hash: String::new(),
                },
                id,
            )
            .expect("线索必须能归并");
            return Ok(ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                source_status: Some(super::state::SourceStatus::Completed),
                observed: 1,
                ..Default::default()
            });
        }
        // Claimed, not closed: the parent kills us right here.
        std::fs::write(&marker, item.work_key.as_bytes()).expect("标记必须落盘");
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

#[test]
fn q3_sigkill_child_driver_is_a_noop_without_the_child_environment() {
    // The driver only acts when the parent handed it a database; a normal test pass must
    // not touch anything.
    assert!(std::env::var_os(KILL_DB).is_none());
}

#[test]
fn q3_resume_refuses_a_moved_plan_and_labels_a_legacy_run() {
    let root = sandbox("frozen");
    let connection = database(&root);
    let fixture = Fixture::start(routes(vec![(
        "/",
        vec![response(200, &[("Content-Type", "text/html")], "page")],
    )]));
    seed_scope(&connection, 13, &format!("{}/", fixture.origin), true);
    let config = settings(json!({}));
    let started = start_or_reuse(&connection, 13, &config).expect("启动");
    let plan_row = load_plan(&connection, started.run_id, 13).expect("计划必须已冻结");
    // §E2：恢复只认这一轮自己冻结的配置，页面当前的原始输入不参与复用。
    let frozen = super::recovery::frozen_settings(&plan_row).to_settings_json();
    verify_frozen(&plan_row, &scope_for(&connection, 13, &frozen), &frozen)
        .expect("冻结配置必须允许继续");

    // 用别的配置去复用同一冻结计划：计划身份对不上，必须拒绝。
    let moved = settings(json!({"exposureMaxRequests": 7}));
    let error = verify_frozen(&plan_row, &scope_for(&connection, 13, &moved), &moved)
        .expect_err("配置变化必须拒绝复用");
    assert!(error.contains("exposure_plan_frozen_mismatch"), "{error}");

    // 归属撤销：范围变了，同样拒绝。
    connection
        .execute(
            "UPDATE asset_ownership_decisions SET exposure_eligible=0 WHERE project_id=13",
            [],
        )
        .unwrap();
    let error = verify_frozen(&plan_row, &scope_for(&connection, 13, &frozen), &frozen)
        .expect_err("范围变化必须拒绝复用");
    assert!(error.contains("授权范围"), "{error}");
    assert!(
        !error.contains("配置"),
        "撤权场景只能报范围变化，不能把配置的账也算上：{error}"
    );
    assert!(
        !error.contains("冻结计划与记录的配置不一致"),
        "撤权拒绝不能混入配置原因：{error}"
    );

    // 历史任务没有计划行：只能标注为遗留，不能被解释成新合同下的覆盖。
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status) VALUES(9001,13,'interrupted')",
            [],
        )
        .unwrap();
    let missing = load_plan(&connection, 9001, 13).expect_err("历史任务没有冻结计划");
    assert!(missing.contains("exposure_plan_missing"), "{missing}");
    let label = legacy_projection(&connection, 9001)
        .unwrap()
        .expect("必须给出遗留投影说明");
    assert!(label.contains("legacy_passive_web"), "{label}");
    // 跨项目读取不能拿到别人的计划。
    let other = load_plan(&connection, started.run_id, 99).expect_err("项目必须匹配");
    assert!(other.contains("exposure_plan_missing"), "{other}");
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q3_cancel_stops_claiming_and_keeps_saved_observations() {
    let root = sandbox("cancel");
    let (connection, config, run_id, fixture) = started(
        &root,
        vec![(
            "/",
            vec![response(200, &[("Content-Type", "text/html")], "page")],
        )],
        json!({"exposureSources": ["passive_web", "certificate_transparency"]}),
    );
    let report =
        run_round(&connection, &config, run_id, "owner-1", PROJECT).expect("第一轮必须跑完");
    assert_eq!(report.counts.completed, 1);
    let observations = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1",
            [run_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert!(observations > 0, "取消前采集到的证据必须已落盘");

    // Ask for cancellation and prove that no further object is fetched at all.
    connection
        .execute(
            "UPDATE exposure_runs SET status='running',cancel_requested=1 WHERE id=?1",
            [run_id],
        )
        .unwrap();
    let served = fixture.request_count();
    let cancelled =
        run_round(&connection, &config, run_id, "owner-2", PROJECT).expect("取消也必须收敛");
    assert_eq!(cancelled.aggregation.status, RunStatus::Cancelled);
    assert_eq!(
        fixture.request_count(),
        served,
        "取消后不得再有任何出站请求"
    );
    let kept = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1",
            [run_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(kept, observations, "取消必须保留已采集结果");
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q3_the_budget_is_persisted_so_a_restart_cannot_spend_it_twice() {
    let root = sandbox("budget");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(41,'范围','active')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status) VALUES(8001,41,'running')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_run_plans(run_id,project_id,scope_hash,options_hash,budget_requests,budget_bytes,requests_used,bytes_used,planned_items,sources_json)
             VALUES(8001,41,'h','o',10,1000,6,500,3,'[\"passive_web\"]')",
            [],
        )
        .unwrap();
    record_usage(&connection, 8001, 3, 200).expect("用量必须累计");
    let plan_row = load_plan(&connection, 8001, 41).expect("计划可读");
    assert_eq!(plan_row.requests_used, 9);
    let meter = meter_for(&plan_row);
    assert_eq!(meter.requests_used, 9, "恢复轮必须从已花掉的额度继续");
    assert_eq!(meter.remaining_requests(), 1);
    let stored = usage(&connection, 8001).unwrap().expect("用量可读回");
    assert_eq!(stored.bytes_used, 700);
    // 判定为中断后不能反复改写终态。
    assert!(mark_interrupted_if_dead(&connection, 8001, 60).expect("判定"));
    let status: String = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=8001",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "interrupted");
    assert!(!mark_interrupted_if_dead(&connection, 8001, 60).expect("二次判定不能报错"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q3_an_active_worker_is_never_declared_dead() {
    let root = sandbox("alive");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(51,'范围','active')",
            [],
        )
        .unwrap();
    connection
        .execute("INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(8101,51,'queued','planned')", [])
        .unwrap();
    plan(&connection, 8101, 51, 1, &seeds(&["live".to_string()])).expect("计划");
    claim_next(&connection, 8101, "worker", 3_600)
        .unwrap()
        .expect("必须领到工作项");
    assert!(
        !mark_interrupted_if_dead(&connection, 8101, 3_600).expect("判定不能失败"),
        "心跳仍在的认领不能被判定为已死"
    );
    let status: String = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=8101",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "queued", "活跃 worker 的行不能被重写");
    age_claims(&connection, 8101);
    assert!(
        mark_interrupted_if_dead(&connection, 8101, 60).expect("判定"),
        "租约到期后必须能判定"
    );
    let counted = counts(&connection, 8101).unwrap();
    assert_eq!(counted.orphaned, 1, "孤儿必须点名而不是消失");
    assert_eq!(counted.open, 0);
    let error_text: String = connection
        .query_row("SELECT error FROM exposure_runs WHERE id=8101", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert!(error_text.contains("restart_orphans"), "{error_text}");
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── T37 剩余两处真实 SIGKILL（2026-10-09） ─────────────────────────────

const REM_KILL_DB: &str = "OVIRAPTOR_TEST_REM_KILL_DB";
const REM_KILL_MARKER: &str = "OVIRAPTOR_TEST_REM_KILL_MARKER";
const REM_KILL_MODE: &str = "OVIRAPTOR_TEST_REM_KILL_MODE";

fn rem_kill_setup(tag: &str, run_id: i64, project: i64) -> (PathBuf, PathBuf) {
    let root = sandbox(tag);
    std::fs::create_dir_all(&root).unwrap();
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("连接");
    ensure_exposure_schema(&connection).unwrap();
    connection
        .execute("INSERT INTO projects(id,name,status) VALUES(?1,'范围','active')", [project])
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(?1,?2,'queued','planned')",
            [run_id, project],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_run_plans(run_id,project_id,sources_json,budget_requests,budget_bytes,planned_items,settings_version,settings_json)
             VALUES(?1,?2,'[\"passive_web\"]',1000,1000000,2,1,'{\"exposureSources\":[\"passive_web\"]}')",
            [run_id, project],
        )
        .unwrap();
    plan(&connection, run_id, project, 1, &seeds(&["one".to_string(), "two".to_string()])).expect("计划写入");
    (root, db_path)
}

fn rem_spawn_victim(test: &str, db_path: &Path, marker: &Path, mode: &str) -> std::process::Child {
    Command::new(std::env::current_exe().expect("测试二进制路径"))
        .args(["--exact", test, "--test-threads=1"])
        .env(REM_KILL_DB, db_path)
        .env(REM_KILL_MARKER, marker)
        .env(REM_KILL_MODE, mode)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("子进程必须能启动")
}

fn rem_wait_marker(child: &mut std::process::Child, marker: &Path, what: &str) {
    let mut waited = 0u32;
    while !marker.exists() {
        std::thread::sleep(std::time::Duration::from_millis(50));
        waited += 1;
        if waited > 400 {
            let _ = child.kill();
            let _ = child.wait();
            panic!("子进程没有在 20 秒内到达：{what}");
        }
    }
}

fn rem_observation(connection: &Connection, run_id: i64, project: i64, item_id: i64) {
    let mut observation = super::observations::NewObservation::new("passive_web", "resource", "https://one.invalid/");
    observation.body = b"first".to_vec();
    observation.body_bytes = 5;
    let (id, _) = super::observations::record(connection, run_id, project, Some(item_id), &observation).expect("观察必须能提交");
    super::observations::merge_finding(
        connection,
        run_id,
        project,
        &super::observations::FindingDraft {
            category: "document".to_string(),
            title: "被杀前已保存的线索".to_string(),
            url: "https://one.invalid/".to_string(),
            source_type: "passive_web".to_string(),
            evidence: "已提交".to_string(),
            severity: "medium".to_string(),
            confidence: 60,
            asset_id: None,
            content_hash: String::new(),
        },
        id,
    )
    .expect("线索必须能归并");
}

fn rem_count(connection: &Connection, sql: &str, run_id: i64) -> i64 {
    connection.query_row(sql, [run_id], |row| row.get(0)).unwrap()
}

/// What the victim runs. `before_claim`: enters the production claim loop while the parent holds
/// the write lock, so it is killed before it could claim anything. `after_save`: saves the
/// observation and finding of its first item, then is killed before that item is finished.
fn rem_kill_child_driver(run_id: i64, project: i64) {
    let db_path = PathBuf::from(std::env::var(REM_KILL_DB).expect("子进程必须带库路径"));
    let marker = PathBuf::from(std::env::var(REM_KILL_MARKER).expect("子进程必须带标记路径"));
    let mode = std::env::var(REM_KILL_MODE).unwrap_or_default();
    let connection = crate::db::open(&db_path).expect("子进程连接");
    if mode == "before_claim" {
        std::fs::write(&marker, b"entering run_plan").expect("标记必须落盘");
        let _ = run_plan(&connection, run_id, "child-before-claim", &mut |item| {
            std::fs::write(marker.with_extension("executed"), item.work_key.as_bytes()).expect("不该到这里");
            Ok(ItemOutcome { status: Some(WorkItemStatus::Completed), ..Default::default() })
        });
        return;
    }
    let _ = run_plan(&connection, run_id, "child-after-save", &mut |item| {
        rem_observation(&connection, run_id, project, item.id);
        std::fs::write(&marker, item.work_key.as_bytes()).expect("标记必须落盘");
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    });
}

/// T37 ①：worker 在认领任何工作项之前被 SIGKILL。父进程先拿住写锁，子进程进入生产认领循环后
/// 只能等锁；此时被杀。结果：没有任何认领、没有任何观察，新 owner 两项都做，轮次正常收口。
#[test]
fn rem_t37_sigkill_before_the_first_claim_leaves_nothing_half_done() {
    const RUN: i64 = 9201;
    const PID: i64 = 132;
    if std::env::var_os(REM_KILL_DB).is_some() {
        rem_kill_child_driver(RUN, PID);
        return;
    }
    let (root, db_path) = rem_kill_setup("rem-kill-before-claim", RUN, PID);
    let marker = root.join("entering");
    let lock = crate::db::open(&db_path).expect("锁连接");
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let mut child = rem_spawn_victim(
        "commands::exposure::tests_q3::rem_t37_sigkill_before_the_first_claim_leaves_nothing_half_done",
        &db_path,
        &marker,
        "before_claim",
    );
    rem_wait_marker(&mut child, &marker, "进入认领循环");
    std::thread::sleep(std::time::Duration::from_millis(600));
    assert!(child.try_wait().unwrap().is_none(), "子进程必须还卡在认领前（等写锁）");
    let _ = child.kill();
    let _ = child.wait();
    lock.execute_batch("ROLLBACK").unwrap();
    assert!(!marker.with_extension("executed").exists(), "被杀前不可能执行任何工作项");

    let connection = crate::db::open(&db_path).expect("重启后连接");
    let counted = counts(&connection, RUN).expect("计数可读");
    assert_eq!((counted.total, counted.completed, counted.open), (2, 0, 2), "{counted:?}");
    assert_eq!(rem_count(&connection, "SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND owner<>''", RUN), 0, "没有遗留认领");
    assert_eq!(rem_count(&connection, "SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1", RUN), 0);
    let mut handled = Vec::new();
    let report = run_plan(&connection, RUN, "owner-after-kill", &mut |item| {
        handled.push(item.work_key.clone());
        Ok(ItemOutcome {
            status: Some(WorkItemStatus::Completed),
            source_status: Some(super::state::SourceStatus::Completed),
            ..Default::default()
        })
    })
    .expect("被杀之后必须能继续");
    assert_eq!(handled, vec!["one".to_string(), "two".to_string()]);
    assert_eq!(report.counts.completed, 2);
    assert_eq!(report.aggregation.status, RunStatus::Completed);
    let _ = std::fs::remove_dir_all(root);
}

/// T37 ②：worker 已把观察和线索保存，但在这一项 finish 之前被 SIGKILL。结果：已保存的证据还在；
/// 租约过期后新 owner 重做这一项，同一观察不重复写、线索不重复、关联不重复，轮次正常收口。
#[test]
fn rem_t37_sigkill_after_save_before_finish_redoes_the_item_without_duplicates() {
    const RUN: i64 = 9301;
    const PID: i64 = 133;
    if std::env::var_os(REM_KILL_DB).is_some() {
        rem_kill_child_driver(RUN, PID);
        return;
    }
    let (root, db_path) = rem_kill_setup("rem-kill-after-save", RUN, PID);
    let marker = root.join("saved-first");
    let mut child = rem_spawn_victim(
        "commands::exposure::tests_q3::rem_t37_sigkill_after_save_before_finish_redoes_the_item_without_duplicates",
        &db_path,
        &marker,
        "after_save",
    );
    rem_wait_marker(&mut child, &marker, "保存第一项的观察");
    let _ = child.kill();
    let _ = child.wait();

    let connection = crate::db::open(&db_path).expect("重启后连接");
    let counted = counts(&connection, RUN).expect("计数可读");
    assert_eq!((counted.total, counted.completed, counted.open), (2, 0, 2), "保存了但没 finish 的一项仍未闭合：{counted:?}");
    assert_eq!(rem_count(&connection, "SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1", RUN), 1, "被杀前保存的观察必须还在");
    assert_eq!(rem_count(&connection, "SELECT COUNT(*) FROM exposure_finding_observations WHERE run_id=?1", RUN), 1);
    age_claims(&connection, RUN);
    assert!(mark_interrupted_if_dead(&connection, RUN, 60).expect("判定"));
    let mut handled = Vec::new();
    let report = run_plan(&connection, RUN, "owner-after-kill", &mut |item| {
        handled.push(item.work_key.clone());
        if item.work_key == "one" {
            rem_observation(&connection, RUN, PID, item.id);
        }
        Ok(ItemOutcome {
            status: Some(WorkItemStatus::Completed),
            source_status: Some(super::state::SourceStatus::Completed),
            observed: 1,
            ..Default::default()
        })
    })
    .expect("被杀之后必须能继续");
    assert_eq!(handled, vec!["one".to_string(), "two".to_string()], "保存后未 finish 的一项要重做");
    assert_eq!(report.counts.completed, 2);
    assert_eq!(report.aggregation.status, RunStatus::Completed);
    assert_eq!(rem_count(&connection, "SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1", RUN), 1, "重做不重复写观察");
    assert_eq!(rem_count(&connection, "SELECT COUNT(*) FROM exposure_finding_observations WHERE run_id=?1", RUN), 1, "重做不重复关联");
    assert_eq!(connection.query_row("SELECT COUNT(*) FROM exposure_findings WHERE project_id=?1", [PID], |row| row.get::<_, i64>(0)).unwrap(), 1, "重做不重复建线索");
    let _ = std::fs::remove_dir_all(root);
}
