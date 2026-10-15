// §E4 — scale and failure behaviour, measured rather than asserted in prose. The numbers this
// file prints (plan build time, database size, paging latency) are test evidence on a temporary
// SQLite file with reproducible seeds; they are not a claim about the user's real database or
// about any third-party network.

use super::cluster_repository::{page_cluster_members, page_clusters, persist_for_observations};
use super::contract::{
    decode_id_cursor, encode_id_cursor, ExposureFindingPageQuery, ExposureSortKey,
};
use super::export::{verify_written, write_csv};
use super::fixture::seed_scope;
use super::observations::{merge_finding, record, FindingDraft, NewObservation};
use super::orchestrator::{run_plan, start_or_reuse, ItemOutcome, KIND_PASSIVE_WEB};
use super::recovery::{load_plan, mark_interrupted_if_dead, meter_for};
use super::repository::page_exposure_findings;
use super::settings::{save, ExposureSettings};
use super::state::{ensure_exposure_schema, RunStatus, SourceStatus, WorkItemStatus};
use super::work_items::{claim_next, counts, enumerate_gaps, finish, plan, WorkItem, WorkItemSeed};
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oviraptor-exposure-e4-{tag}-{}",
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

fn current_settings(connection: &Connection, project: i64, items: i64) -> JsonValue {
    let mut value = ExposureSettings::for_project(project);
    value.sources = vec![KIND_PASSIVE_WEB.to_string()];
    value.max_items = items;
    save(connection, &value, 0).expect("配置必须能保存");
    super::exposure_scan_settings(connection, project).expect("配置必须可读")
}

/// The file size of the temporary database, as the page would have to carry it.
fn database_bytes(connection: &Connection) -> i64 {
    let path = connection.path().expect("临时库必须有路径");
    std::fs::metadata(path)
        .map(|meta| meta.len() as i64)
        .unwrap_or_default()
}

fn seeds(count: i64) -> Vec<WorkItemSeed> {
    (0..count)
        .map(|index| WorkItemSeed {
            work_key: format!("{KIND_PASSIVE_WEB}|scale-{index}"),
            kind: KIND_PASSIVE_WEB.to_string(),
            source_key: KIND_PASSIVE_WEB.to_string(),
            asset_id: None,
            origin: String::new(),
            entity: format!("item-{index}"),
            input_hash: format!("seed-{index}"),
        })
        .collect()
}

/// 100,000 work items: materialised, counted exactly, and read page by page without any
/// 200/500-row window deciding the total. Timings and file size are printed as evidence.
#[test]
fn e4_one_hundred_thousand_items_are_planned_counted_and_paged() {
    let root = sandbox("hundred-k");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(181,'规模','active')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(18100,181,'queued','planned')",
            [],
        )
        .unwrap();
    let started = Instant::now();
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .expect("批量计划事务");
    let inserted = plan(&connection, 18100, 181, 1, &seeds(100_000)).expect("十万项必须能物化");
    connection.execute_batch("COMMIT").expect("计划提交");
    let plan_ms = started.elapsed().as_millis();
    assert_eq!(inserted, 100_000);

    let counted = counts(&connection, 18100).expect("计数");
    assert_eq!(counted.total, 100_000, "总数必须是全量，不是前 200/500 条");
    assert_eq!(counted.open, 100_000);

    // 600 findings: more than the legacy 500-row window the old list commands still apply.
    for index in 0..600 {
        let base = NewObservation::new(
            KIND_PASSIVE_WEB,
            "resource",
            &format!("https://scale-{index}.invalid/"),
        );
        let (observation_id, _) = record(
            &connection,
            18100,
            181,
            None,
            &NewObservation {
                body: format!("body-{index}").as_bytes().to_vec(),
                body_bytes: 10,
                ..base
            },
        )
        .expect("观察写入");
        merge_finding(
            &connection,
            18100,
            181,
            &FindingDraft {
                category: "document".to_string(),
                title: format!("规模线索 {index}"),
                url: format!("https://scale-{index}.invalid/"),
                source_type: KIND_PASSIVE_WEB.to_string(),
                evidence: "示例".to_string(),
                severity: "medium".to_string(),
                confidence: 60,
                asset_id: None,
                content_hash: String::new(),
            },
            observation_id,
        )
        .expect("线索归并");
    }
    let page_started = Instant::now();
    let mut seen: Vec<i64> = Vec::new();
    let mut after = ExposureSortKey::decode("").expect("首页游标");
    let mut pages = 0usize;
    loop {
        let query = ExposureFindingPageQuery {
            project_id: 181,
            status: None,
            search: None,
            page_size: 50,
            after: after.clone(),
            disposition: None,
        };
        let page = page_exposure_findings(&connection, &query).expect("线索分页");
        assert_eq!(page.total, 600, "过滤总数必须与页数无关");
        seen.extend(page.items.iter().map(|row| row.id));
        pages += 1;
        if !page.has_more {
            break;
        }
        let cursor = page.next_cursor.clone();
        after = ExposureSortKey::decode(&cursor).expect("下一页游标");
        if pages > 40 {
            panic!("分页没有前进：{} 页", pages);
        }
    }
    let paging_ms = page_started.elapsed().as_millis();
    assert_eq!(pages, 12, "{pages} 页读完 600 条");
    assert_eq!(seen.len(), 600, "每页追加后必须拿到全部，且不重复");
    assert_eq!(
        seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
        600,
        "同一过滤条件下不得重复"
    );
    let db_bytes = database_bytes(&connection);
    println!(
        "E4 SCALE project=181 items=100000 plan_ms={plan_ms} findings_pages={pages} paging_ms={paging_ms} db_bytes={db_bytes}"
    );
    assert!(db_bytes > 0, "数据库大小必须可测量");
    let _ = std::fs::remove_dir_all(root);
}

/// At scale the item ceiling has to leave an explained tail: the round says "500 collected,
/// 9,500 not collected because of the ceiling" instead of looking complete.
#[test]
fn e4_a_ceiling_at_scale_leaves_an_enumerable_named_tail() {
    let root = sandbox("tail");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(182,'规模','active')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(18200,182,'queued','planned')",
            [],
        )
        .unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").expect("事务");
    plan(&connection, 18200, 182, 1, &seeds(10_000)).expect("一万项物化");
    connection.execute_batch("COMMIT").expect("提交");
    // The round's own frozen ceiling is the only thing deciding how many items get claimed.
    let mut capped = ExposureSettings::for_project(182);
    capped.sources = vec![KIND_PASSIVE_WEB.to_string()];
    capped.max_items = 200;
    let frozen = capped.to_settings_json();
    connection
        .execute(
            r#"INSERT INTO exposure_run_plans(run_id,project_id,sources_json,budget_requests,budget_bytes,planned_items,settings_version,settings_json,options_hash)
               VALUES(18200,182,'["passive_web"]',100000,100000000,10000,1,?1,?2)"#,
            params![
                frozen.to_string(),
                super::orchestrator::options_hash(&frozen)
            ],
        )
        .unwrap();
    let report = run_plan(
        &connection,
        18200,
        "owner-tail",
        &mut |_item: &WorkItem| -> Result<ItemOutcome, String> {
            Ok(ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                source_status: Some(SourceStatus::Completed),
                observed: 1,
                ..Default::default()
            })
        },
    )
    .expect("收敛");
    assert_eq!(report.counts.total, 10_000, "{:?}", report.counts);
    assert_eq!(report.counts.completed, 200, "额度内必须真的采满");
    assert_eq!(report.counts.quota, 9_800, "剩下的必须是可命名的额度缺口");
    assert_eq!(report.counts.open, 0, "缺口也要落到终态，不能悬着");
    assert_eq!(report.aggregation.status, RunStatus::Partial);
    let gaps = enumerate_gaps(&connection, 18200, 500).expect("缺口枚举");
    assert!(gaps.len() <= 500, "缺口枚举本身也要有界");
    let note = gaps
        .iter()
        .find(|(_, _, reason)| reason.contains("本轮工作项额度 200"))
        .expect("缺口必须写明额度与未采集数量");
    assert!(note.2.contains("剩余 9800 项未采集"), "{note:?}");
    assert_eq!(note.1, "blocked_quota", "缺口行必须落在额度终态上");
    let _ = std::fs::remove_dir_all(root);
}

/// A plan that cannot be committed must not leave a round that looks active, and the project
/// must still be able to start one afterwards.
#[test]
fn e4_a_failed_plan_commit_leaves_no_active_round() {
    let root = sandbox("commit");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(183,'范围','active')",
            [],
        )
        .unwrap();
    seed_scope(&connection, 183, "http://commit.e4.invalid/", true);
    let settings = current_settings(&connection, 183, 10);
    // Fault injection at the plan-commit seam: the run row is inserted, then the plan write
    // aborts, so the whole start transaction must roll back. A trigger is used instead of
    // dropping the table because the start path re-creates missing structure itself.
    connection
        .execute(
            r#"CREATE TRIGGER exposure_e4_plan_commit_fault
               BEFORE INSERT ON exposure_run_plans
               BEGIN
                 SELECT RAISE(ABORT, '注入的计划提交故障');
               END"#,
            [],
        )
        .unwrap();
    let error =
        start_or_reuse(&connection, 183, &settings).expect_err("计划写不进去就必须拒绝启动");
    assert!(error.contains("计划冻结失败"), "{error}");
    assert!(error.contains("注入的计划提交故障"), "{error}");
    let active: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_runs WHERE project_id=183 AND status IN ('queued','running')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, 0, "启动失败不能留下一个看起来在跑的轮次");
    let items: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_work_items WHERE project_id=183",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(items, 0, "也不能留下半截计划");
    connection
        .execute("DROP TRIGGER exposure_e4_plan_commit_fault", [])
        .unwrap();
    // The same saved settings, retried once the injected fault is gone.
    let retry = start_or_reuse(&connection, 183, &settings).expect("重试");
    assert!(retry.run_id > 0);
    let _ = std::fs::remove_dir_all(root);
}

/// Crash after the observation was written but before the item closed: the restart re-claims
/// the same item, does not duplicate the evidence, does not reset what was spent, and the
/// cluster pass converges on one graph.
#[test]
fn e4_a_crash_after_the_observation_reuses_it_instead_of_refetching_or_respending() {
    let root = sandbox("after-observation");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(184,'范围','active')",
            [],
        )
        .unwrap();
    for origin in ["http://r1.e4.invalid/", "http://r2.e4.invalid/"] {
        seed_scope(&connection, 184, origin, true);
    }
    let started =
        start_or_reuse(&connection, 184, &current_settings(&connection, 184, 10)).expect("启动");
    let mut first_ids: Vec<i64> = Vec::new();
    while let Some(item) = claim_next(&connection, started.run_id, "owner-crash", 60).expect("认领")
    {
        let (id, _) = record(
            &connection,
            started.run_id,
            184,
            Some(item.id),
            &NewObservation {
                asset_id: item.asset_id,
                body: b"same-bytes".to_vec(),
                body_bytes: 10,
                ..NewObservation::new(&item.source_key, "resource", &item.origin)
            },
        )
        .expect("观察写入");
        first_ids.push(id);
        // The worker dies here: the observation is durable, the item is not closed.
    }
    super::work_items::record_usage(&connection, started.run_id, 3, 30_000).expect("用量累计");
    let before =
        persist_for_observations(&connection, started.run_id, 184, &first_ids, 1).expect("簇写入");
    // The dead worker's heartbeats are aged past the lease instead of waiting in real time.
    connection
        .execute(
            "UPDATE exposure_work_items SET heartbeat_at=?2 WHERE run_id=?1 AND status IN ('claimed','in_progress')",
            params![started.run_id, super::state::shift_seconds_back(600)],
        )
        .unwrap();
    let interrupted = mark_interrupted_if_dead(&connection, started.run_id, 60).expect("中断判定");
    assert!(interrupted, "没有心跳的认领必须被判为中断");
    // A restart re-claims the same items and re-observes the same URLs: no second row.
    let mut again: Vec<i64> = Vec::new();
    while let Some(item) =
        claim_next(&connection, started.run_id, "owner-resume", 60).expect("重新认领")
    {
        let (id, fresh) = record(
            &connection,
            started.run_id,
            184,
            Some(item.id),
            &NewObservation {
                asset_id: item.asset_id,
                body: b"same-bytes".to_vec(),
                body_bytes: 10,
                ..NewObservation::new(&item.source_key, "resource", &item.origin)
            },
        )
        .expect("观察写入");
        again.push(id);
        assert!(!fresh, "同一轮的同一 URL 不该再算一次新观察：{item:?}");
        finish(
            &connection,
            item.id,
            "owner-resume",
            item.claim_version,
            WorkItemStatus::Completed,
            "",
            1,
            0,
            0,
        )
        .expect("终态");
    }
    let rows: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1",
            [started.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 2, "两轮写入同一证据，行数不翻倍");
    let after =
        persist_for_observations(&connection, started.run_id, 184, &again, 1).expect("重新分组");
    assert_eq!(after.clusters, before.clusters, "簇必须收敛，不累积");
    assert_eq!(after.members, before.members);
    let plan_row = load_plan(&connection, started.run_id, 184).expect("计划");
    let meter = meter_for(&plan_row);
    assert_eq!(meter.requests_used, 3, "恢复不得把已花掉的请求额度清零");
    assert_eq!(meter.max_bytes, plan_row.max_bytes);
    let attempts: i64 = connection
        .query_row(
            "SELECT MAX(attempts) FROM exposure_work_items WHERE run_id=?1",
            [started.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(attempts >= 2, "重开必须留下真实尝试次数：{attempts}");
    let still: String = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=?1",
            [started.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        still, "interrupted",
        "只有 run_plan/finalize 才有权改终态：{still}"
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Cluster paging has to stay cheap when a project has many clusters: one page reads a bounded
/// number of rows and a member page never walks the whole graph.
#[test]
fn e4_cluster_paging_stays_bounded_when_the_graph_is_large() {
    let root = sandbox("cluster-paging");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(185,'范围','active')",
            [],
        )
        .unwrap();
    let mut ids = Vec::new();
    connection
        .execute("INSERT OR IGNORE INTO exposure_runs(id,project_id,status) VALUES(18500,185,'completed')", [])
        .unwrap();
    for group in 0..60 {
        for slot in 0..2 {
            let index = group * 2 + slot;
            let origin = format!("http://c{index}.e4.invalid/");
            seed_scope(&connection, 185, &origin, true);
            let asset_id: i64 = connection
                .query_row(
                    "SELECT id FROM assets WHERE asset_key=?1",
                    [&origin],
                    |row| row.get(0),
                )
                .unwrap();
            let (id, _) = record(
                &connection,
                18500,
                185,
                None,
                &NewObservation {
                    asset_id: Some(asset_id),
                    body: format!("group-{group}").as_bytes().to_vec(),
                    body_bytes: 8,
                    ..NewObservation::new(KIND_PASSIVE_WEB, "resource", &origin)
                },
            )
            .expect("观察写入");
            ids.push(id);
        }
    }
    let pass = persist_for_observations(&connection, 18500, 185, &ids, 1).expect("分组");
    assert!(pass.clusters >= 60, "{pass:?}");
    let started = Instant::now();
    let page = page_clusters(&connection, 185, "", "", "", 20).expect("第一页");
    assert_eq!(page.items.len(), 20, "页长就是硬上限，不是全量");
    assert!(page.has_more);
    assert_eq!(page.total, pass.clusters as i64, "总数必须是全簇口径");
    let second = page_clusters(&connection, 185, "", "", &page.next_cursor, 20).expect("第二页");
    assert_eq!(second.total, page.total, "第二页的过滤总数不能缩水");
    let paging_ms = started.elapsed().as_millis();
    let members_started = Instant::now();
    let members = page_cluster_members(&connection, 185, page.items[0].id, "", 20).expect("成员页");
    let members_ms = members_started.elapsed().as_millis();
    assert_eq!(members.items.len(), 2);
    assert!(
        decode_id_cursor(&encode_id_cursor(page.items[1].id)).unwrap_or_default()
            == page.items[1].id,
        "游标必须可回解"
    );
    println!(
        "E4 CLUSTER PAGING project=185 clusters={} clusters_page_ms={paging_ms} members_ms={members_ms}",
        page.total
    );
    let _ = std::fs::remove_dir_all(root);
}

/// Real disk evidence: a symlink or a non-regular target is refused instead of written
/// through, and a written package reports its own digest and row count.
#[test]
fn e4_disk_evidence_refuses_symlinks_and_reports_a_verifiable_digest() {
    let root = sandbox("disk");
    std::fs::create_dir_all(&root).expect("目录");
    let export = root.join("export");
    std::fs::create_dir_all(&export).expect("导出目录");
    let outside = root.join("outside");
    std::fs::create_dir_all(&outside).expect("外部目录");
    let rows: Vec<Vec<String>> = (0..2)
        .map(|index| {
            (0..super::export::HEADERS.len())
                .map(|column| format!("r{index}c{column}"))
                .collect()
        })
        .collect();
    let good = export.join("exposure-good.csv");
    assert_eq!(write_csv(&good, &rows).expect("写入"), 2);
    let summary = verify_written(&good).expect("可核验");
    assert_eq!(summary["rows"], json!(2));
    assert_eq!(
        summary["ruleVersion"],
        json!(super::state::EXPOSURE_RULE_VERSION)
    );
    assert_eq!(
        summary["redactionVersion"],
        json!(super::redaction::REDACTION_VERSION)
    );
    let digest = summary["sha256"].as_str().unwrap_or_default().to_string();
    assert_eq!(digest.len(), 64, "{digest}");
    // Writing the same name again is refused instead of silently truncating or following.
    assert!(write_csv(&good, &rows).is_err(), "已存在的目标不能被覆盖");
    let symlink = export.join("exposure-link.csv");
    std::os::unix::fs::symlink(outside.join("victim.csv"), &symlink).expect("符号链接");
    let error = write_csv(&symlink, &rows).expect_err("符号链接目标必须拒绝");
    assert!(error.contains("exposure_export_symlink"), "{error}");
    assert!(
        !outside.join("victim.csv").exists(),
        "被拒绝的写入不能落在链接指向的位置"
    );
    let verify_error = verify_written(&symlink).expect_err("核验也不得跟随链接");
    assert!(
        verify_error.contains("exposure_export_symlink"),
        "{verify_error}"
    );
    // A tampered package fails its own shape check.
    let tampered = export.join("exposure-tampered.csv");
    std::fs::write(&tampered, "类别,标题,多余\n a,b,c,d\n").expect("写入篡改文件");
    let shape = verify_written(&tampered).expect_err("列数不一致必须报错");
    assert!(shape.contains("exposure_export_shape"), "{shape}");
    // And a directory is never treated as evidence.
    let dir_error = verify_written(&export).expect_err("目录不能当作证据包");
    assert!(dir_error.contains("不是普通文件"), "{dir_error}");
    let _ = std::fs::remove_dir_all(root);
}
