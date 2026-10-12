// §Q1 — the read-only pagination contract, proved against a temporary SQLite file.
// Nothing in here opens a socket or creates a run; the last test asserts exactly that.

use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::contract::{
    decode_id_cursor, ExposureFindingPageInput, ExposureQueryError, ExposureSortKey,
    EXPOSURE_MAX_PAGE_SIZE, EXPOSURE_MIN_PAGE_SIZE,
};
use super::repository::{
    eligible_asset_scope, load_exposure_finding, page_exposure_findings, page_exposure_runs,
    read_exposure_source_results,
};
use super::summary::exposure_scope_summary;

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("oviraptor-exposure-{tag}-{}", Uuid::new_v4()))
}

fn open_database(root: &Path) -> (Connection, PathBuf) {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库必须能初始化");
    (
        crate::db::open(&db_path).expect("临时库必须能打开"),
        db_path,
    )
}

fn make_project(connection: &Connection, id: i64, name: &str, status: &str) {
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(?1,?2,?3)",
            params![id, name, status],
        )
        .unwrap();
}

/// `count` findings with the given severity, all sharing one confidence and timestamp so
/// the only thing that can order them is the id tie-break.
fn seed_findings(
    connection: &Connection,
    project_id: i64,
    from: i64,
    count: i64,
    severity: &str,
    confidence: i64,
    last_seen: &str,
) {
    for index in from..from + count {
        connection
            .execute(
                "INSERT INTO exposure_findings(project_id,category,title,source_type,source_url,severity,confidence,status,fingerprint,last_seen_at)
                 VALUES(?1,'source_code',?2,'passive_web',?3,?4,?5,'new',?6,?7)",
                params![
                    project_id,
                    format!("线索 {index}"),
                    format!("https://public.invalid/{index}"),
                    severity,
                    confidence,
                    format!("fp-{project_id}-{index}"),
                    last_seen
                ],
            )
            .unwrap();
    }
}

fn page_input(project_id: i64, page_size: i64, cursor: &str) -> ExposureFindingPageInput {
    ExposureFindingPageInput {
        project_id,
        status: String::new(),
        search: String::new(),
        page_size,
        cursor: cursor.to_string(),
        disposition: String::new(),
    }
}

/// Walk every page and collect ids in read order.
fn read_all_ids(connection: &Connection, project_id: i64, page_size: i64) -> Vec<i64> {
    let mut cursor = String::new();
    let mut seen: Vec<i64> = Vec::new();
    loop {
        let page = page_exposure_findings(
            connection,
            &page_input(project_id, page_size, &cursor)
                .normalize()
                .unwrap(),
        )
        .unwrap();
        seen.extend(page.items.iter().map(|row| row.id));
        if !page.has_more {
            assert!(
                page.next_cursor.is_empty(),
                "最后一页不能再给出游标：{}",
                page.next_cursor
            );
            assert_eq!(page.total, seen.len() as i64, "总数与读到的行数不一致");
            return seen;
        }
        cursor = page.next_cursor;
    }
}

#[test]
fn q1_pages_201_findings_to_the_end_without_repeats_or_gaps() {
    let root = sandbox("page-201");
    let (connection, _) = open_database(&root);
    make_project(&connection, 1, "分页", "active");
    seed_findings(&connection, 1, 1, 201, "medium", 50, "2026-10-07 10:00:00");

    let first = page_exposure_findings(
        &connection,
        &page_input(1, EXPOSURE_MIN_PAGE_SIZE, "")
            .normalize()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first.items.len(), 10);
    assert!(first.has_more);
    assert_eq!(first.total, 201, "总数必须是筛选后的真实行数");
    assert_eq!(first.page_size, 10);

    let mut pages = 0;
    let mut cursor = first.next_cursor.clone();
    let mut ids = first.items.iter().map(|row| row.id).collect::<Vec<_>>();
    while !cursor.is_empty() {
        let page = page_exposure_findings(
            &connection,
            &page_input(1, EXPOSURE_MAX_PAGE_SIZE, &cursor)
                .normalize()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(page.total, 201, "游标不能改变总数口径");
        pages += 1;
        ids.extend(page.items.iter().map(|row| row.id));
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }
    assert_eq!(ids.len(), 201, "分 {} 页后必须读到全部 201 条", pages + 1);
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 201, "跨页出现重复：{ids:?}");
    assert_eq!(read_all_ids(&connection, 1, 60).len(), 201);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn q1_identical_sort_keys_still_page_in_a_stable_order() {
    let root = sandbox("ties");
    let (connection, _) = open_database(&root);
    make_project(&connection, 1, "同键", "active");
    // 40 rows that tie on severity, confidence and last_seen_at.
    seed_findings(&connection, 1, 1, 40, "high", 77, "2026-10-07 11:00:00");
    let once = read_all_ids(&connection, 1, 10);
    let twice = read_all_ids(&connection, 1, 13);
    assert_eq!(once, twice, "同样的数据、不同的页长必须给出同一个全序");
    let mut expected = once.clone();
    expected.sort_unstable_by(|left, right| right.cmp(left));
    assert_eq!(once, expected, "id tie-break 必须稳定地从大到小");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn q1_four_thousand_rows_paginate_to_the_end() {
    let root = sandbox("scale-4000");
    let (connection, _) = open_database(&root);
    make_project(&connection, 1, "规模", "active");
    seed_findings(
        &connection,
        1,
        1,
        4_000,
        "medium",
        50,
        "2026-10-07 12:00:00",
    );
    let ids = read_all_ids(&connection, 1, EXPOSURE_MAX_PAGE_SIZE);
    assert_eq!(ids.len(), 4_000);
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        4_000
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn q1_filters_search_and_status_with_the_same_predicate_as_the_total() {
    let root = sandbox("filters");
    let (connection, _) = open_database(&root);
    make_project(&connection, 1, "筛选", "active");
    seed_findings(&connection, 1, 1, 501, "high", 60, "2026-10-07 13:00:00");
    connection
        .execute(
            "UPDATE exposure_findings SET status='dismissed' WHERE project_id=1 AND id%2=0",
            [],
        )
        .unwrap();

    let dismissed = page_exposure_findings(
        &connection,
        &ExposureFindingPageInput {
            status: "dismissed".into(),
            ..page_input(1, 50, "")
        }
        .normalize()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        dismissed.total, 250,
        "偶数 id 共 250 条：{}",
        dismissed.total
    );
    assert!(dismissed.items.iter().all(|row| row.status == "dismissed"));

    // A pasted `%` must stay a literal character, not a wildcard over the whole table.
    connection
        .execute(
            "INSERT INTO exposure_findings(project_id,category,title,source_url,severity,confidence,fingerprint)
             VALUES(1,'document','100% 内部文件','https://public.invalid/percent','medium',50,'fp-percent')",
            [],
        )
        .unwrap();
    let searched = page_exposure_findings(
        &connection,
        &ExposureFindingPageInput {
            search: "100%".into(),
            ..page_input(1, 20, "")
        }
        .normalize()
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        searched.total, 1,
        "LIKE 通配符必须被转义：{}",
        searched.total
    );
    assert_eq!(searched.items[0].title, "100% 内部文件");

    // An unknown status is refused, not silently widened to "everything".
    let rejected = ExposureFindingPageInput {
        status: "archieved".into(),
        ..page_input(1, 20, "")
    }
    .normalize();
    assert_eq!(
        rejected.unwrap_err().code,
        "exposure_status_invalid",
        "未知枚举不能变空串放行"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn q1_runs_page_newest_first_and_the_cursor_stays_opaque() {
    let root = sandbox("runs");
    let (connection, _) = open_database(&root);
    make_project(&connection, 1, "任务", "active");
    for index in 1..=25 {
        connection
            .execute(
                "INSERT INTO exposure_runs(project_id,status,eligible_assets) VALUES(1,'completed',?1)",
                [index],
            )
            .unwrap();
    }
    let first = page_exposure_runs(&connection, 1, 20, "").unwrap();
    assert_eq!(first.items.len(), 20);
    assert_eq!(first.total, 25);
    assert_eq!(first.items[0].id, 25, "最新的任务必须在前");
    assert!(first.has_more);
    let after = decode_id_cursor(&first.next_cursor).unwrap();
    assert_eq!(after, 6, "游标就是本页最后一行的 id：{after}");
    let second = page_exposure_runs(&connection, 1, 20, &first.next_cursor).unwrap();
    assert_eq!(
        second.items.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![5, 4, 3, 2, 1]
    );
    assert!(!second.has_more);
    assert_eq!(
        page_exposure_runs(&connection, 1, 5, "").unwrap_err().code,
        "exposure_page_size_invalid"
    );
    assert_eq!(
        page_exposure_runs(&connection, 1, 20, "@@@not-base64@@@")
            .unwrap_err()
            .code,
        "exposure_cursor_invalid"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn q1_other_project_ids_are_refused_and_nothing_is_revealed() {
    let root = sandbox("scope");
    let (connection, db_path) = open_database(&root);
    make_project(&connection, 1, "本项目", "active");
    make_project(&connection, 2, "别的项目", "active");
    seed_findings(&connection, 1, 1, 3, "high", 90, "2026-10-07 14:00:00");
    seed_findings(
        &connection,
        2,
        100,
        2,
        "critical",
        95,
        "2026-10-07 14:00:00",
    );
    connection
        .execute("INSERT INTO exposure_runs(project_id) VALUES(2)", [])
        .unwrap();
    let foreign_run = connection
        .query_row(
            "SELECT id FROM exposure_runs WHERE project_id=2",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let foreign_finding = connection
        .query_row(
            "SELECT id FROM exposure_findings WHERE project_id=2 LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();

    assert_eq!(
        load_exposure_finding(&connection, foreign_finding, 1)
            .unwrap_err()
            .code,
        "exposure_scope_mismatch"
    );
    assert_eq!(
        read_exposure_source_results(&connection, foreign_run, 1)
            .unwrap_err()
            .code,
        "exposure_scope_mismatch"
    );
    // The foreign project keeps its own rows.
    assert_eq!(
        page_exposure_findings(&connection, &page_input(1, 20, "").normalize().unwrap())
            .unwrap()
            .total,
        3
    );
    assert!(page_exposure_runs(&connection, 1, 20, "")
        .unwrap()
        .items
        .is_empty());
    let _ = db_path;
    let _ = fs::remove_dir_all(root);
}

#[test]
fn q1_read_paths_write_nothing_and_reject_corrupt_cursors() {
    let root = sandbox("read-only");
    let (connection, _) = open_database(&root);
    make_project(&connection, 1, "只读", "active");
    seed_findings(&connection, 1, 1, 40, "high", 55, "2026-10-07 15:00:00");
    connection
        .execute(
            "INSERT INTO exposure_runs(project_id,status) VALUES(1,'completed')",
            [],
        )
        .unwrap();
    let runs_before = connection
        .query_row("SELECT COUNT(*) FROM exposure_runs", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    let findings_before = connection
        .query_row("SELECT COUNT(*) FROM exposure_findings", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    let sources_before = connection
        .query_row("SELECT COUNT(*) FROM exposure_source_results", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();

    let _ = read_all_ids(&connection, 1, 11);
    let _ = page_exposure_runs(&connection, 1, 20, "");
    let run_id = connection
        .query_row("SELECT id FROM exposure_runs LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
    let _ = read_exposure_source_results(&connection, run_id, 1);
    let _ = exposure_scope_summary(&connection, 1);

    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM exposure_runs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        runs_before,
        "只读分页不得创建 run"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM exposure_findings", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        findings_before,
        "只读分页不得写入或清除线索"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM exposure_source_results", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        sources_before,
        "只读分页不得写来源状态"
    );

    for broken in ["!!!!", "YWJjZA", "", "AA"] {
        let result = page_input(1, 20, broken).normalize();
        if broken.is_empty() {
            assert!(result.is_ok(), "空游标就是第一页");
        } else {
            assert_eq!(
                result.unwrap_err().code,
                "exposure_cursor_invalid",
                "{broken} 必须被拒绝"
            );
        }
    }
    // A legal-but-earlier cursor narrows the rows while the total stays the filtered total.
    let forged = ExposureSortKey {
        rank: 1,
        confidence: 55,
        last_seen: "2026-10-07 15:00:00".into(),
        id: 1,
    };
    let narrowed = page_exposure_findings(
        &connection,
        &page_input(1, 20, &forged.encode()).normalize().unwrap(),
    )
    .unwrap();
    assert!(
        narrowed.items.is_empty(),
        "rank=1 的游标之后不该还有更低的行：{}",
        narrowed.items.len()
    );
    assert_eq!(narrowed.total, 40, "游标只能减少页内容，不能减少筛选总数");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn q1_summary_shows_the_scope_the_current_plan_cannot_reach() {
    let root = sandbox("summary");
    let (connection, _) = open_database(&root);
    make_project(&connection, 1, "范围", "active");
    make_project(&connection, 2, "归档", "archived");
    // 250 admitted members: 200 eligible, 50 more beyond the collector's cap.
    for index in 1..=250 {
        connection
            .execute(
                "INSERT INTO assets(id,asset_key,host,domain) VALUES(?1,?2,?3,'invalid.example')",
                params![
                    index,
                    format!("asset-{index}"),
                    format!("h{index}.invalid.example")
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO project_assets(project_id,asset_id) VALUES(1,?1)",
                [index],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO asset_ownership_decisions(project_id,asset_id,status,exposure_eligible,confidence)
                 VALUES(1,?1,'confirmed',1,90)",
                [index],
            )
            .unwrap();
    }
    // One asset was moved out of the project: it must not count, and must not be collectable.
    connection
        .execute(
            "UPDATE project_assets SET is_deleted=1 WHERE project_id=1 AND asset_id=250",
            [],
        )
        .unwrap();
    // One asset is ownership-eligible but no longer a member row at all.
    connection
        .execute(
            "DELETE FROM project_assets WHERE project_id=1 AND asset_id=249",
            [],
        )
        .unwrap();
    seed_findings(&connection, 1, 1, 5, "critical", 90, "2026-10-07 16:00:00");
    seed_findings(&connection, 1, 100, 3, "low", 40, "2026-10-07 16:00:00");
    connection
        .execute(
            "UPDATE exposure_findings SET status='confirmed' WHERE id=1",
            [],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE exposure_findings SET status='dismissed' WHERE id=2",
            [],
        )
        .unwrap();

    // G1-0: the planned count follows the project's own saved work-item cap (200 here);
    // G0 asserted a hard-coded 200 that no collector enforced.
    super::state::ensure_exposure_schema(&connection).unwrap();
    let mut capped = super::settings::ExposureSettings::for_project(1);
    capped.max_items = 200;
    super::settings::save(&connection, &capped, 0).unwrap();
    let summary = exposure_scope_summary(&connection, 1).unwrap();
    assert_eq!(summary.limits_basis, "project_settings_v1");
    assert!(summary.limits_error.is_none());
    assert_eq!(
        summary.eligible_assets_total, 248,
        "移出与删除成员都不算在本轮范围内"
    );
    assert_eq!(summary.eligible_assets_planned, 200);
    assert_eq!(summary.assets_beyond_plan, 48, "全量差额必须显示出来");
    assert_eq!(summary.findings_total, 8);
    assert_eq!(summary.findings_confirmed, 1);
    assert_eq!(summary.findings_dismissed, 1);
    assert_eq!(summary.high_risk_open, 3, "已确认/已排除不算待处置高风险");
    assert!(summary.project_active);
    assert_eq!(summary.known_limits.len(), 7);
    assert!(summary
        .known_limits
        .iter()
        .any(|row| row.key == "passive_assets" && row.cap == 200));

    let (total, planned, active) = eligible_asset_scope(&connection, 2).unwrap();
    assert_eq!((total, planned), (0, 0), "归档项目没有准入范围");
    assert!(!active, "归档项目必须标记为非活动");
    assert_eq!(
        exposure_scope_summary(&connection, 0).unwrap_err().code,
        "exposure_project_invalid"
    );
    let error: ExposureQueryError = ExposureQueryError::storage("driver said no");
    assert_eq!(error.code, "exposure_query_failed");
    let _ = fs::remove_dir_all(root);
}
