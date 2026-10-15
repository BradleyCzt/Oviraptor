// G0 — baseline checks the spec asks for before any new capability: the P1 configuration /
// error-truthfulness fixes (settings CAS on two real connections, a real history window,
// schema failures that are reported instead of hidden) and the read-only reachability of the
// shared PSL / organization directory that later slices reuse instead of copying.

use std::sync::{Arc, Barrier};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::json;

use super::settings::{save, ExposureSettings};

struct TempDb {
    root: std::path::PathBuf,
    path: std::path::PathBuf,
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn temp_db(project: i64) -> (TempDb, Connection) {
    let root = std::env::temp_dir().join(format!("oviraptor-exposure-g0-{}", uuid::Uuid::new_v4()));
    let path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&path).expect("连接");
    super::state::ensure_exposure_schema(&connection).expect("附加结构必须可用");
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(?1,'G0','active')",
            [project],
        )
        .unwrap();
    (TempDb { root, path }, connection)
}

fn count(connection: &Connection, sql: &str) -> i64 {
    connection.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn current_version(connection: &Connection, project: i64) -> i64 {
    connection
        .query_row(
            "SELECT COALESCE((SELECT version FROM exposure_settings WHERE project_id=?1),0)",
            [project],
            |row| row.get(0),
        )
        .unwrap()
}

fn settings_with(project: i64, max_requests: i64) -> ExposureSettings {
    let mut value = ExposureSettings::for_project(project);
    value.max_requests = max_requests;
    value
}

/// T25 / P1-1: two independent SQLite connections save against the same expectedVersion at
/// the same instant, round after round (round 0 is the very first save, v0). Exactly one
/// wins each round, the other gets the stable conflict code (never a lock error), and the
/// version and history advance exactly once per round.
#[test]
fn g0_two_connections_saving_the_same_version_write_exactly_once() {
    let (db, connection) = temp_db(141);
    let rounds = 12;
    for round in 0..rounds {
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|worker| {
                let path = db.path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let own = crate::db::open(&path).expect("独立连接");
                    let value = settings_with(141, 100 + round * 10 + worker);
                    barrier.wait();
                    save(&own, &value, round)
                })
            })
            .collect();
        let results: Vec<Result<i64, String>> =
            handles.into_iter().map(|handle| handle.join().unwrap()).collect();
        let wins: Vec<_> = results.iter().filter_map(|row| row.as_ref().ok()).collect();
        assert_eq!(wins, vec![&(round + 1)], "round {round}: {results:?}");
        let loser = results.iter().find_map(|row| row.as_ref().err()).expect("一方必须失败");
        assert!(loser.contains("[exposure_setting_conflict]"), "round {round}: {loser}");
        assert_eq!(current_version(&connection, 141), round + 1);
    }
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM exposure_setting_versions WHERE project_id=141"),
        rounds - 1,
        "每个被覆盖的版本恰好留档一次"
    );
}

/// P1-1, deterministic interleaving: another connection is in the middle of a save (holds the
/// write lock, has already archived v1 and moved the row to v2). A save that read v1 must
/// wait, re-read inside its own transaction and refuse — it must not archive or overwrite.
#[test]
fn g0_a_save_behind_a_held_write_lock_rereads_and_conflicts() {
    let (db, connection) = temp_db(141);
    assert_eq!(save(&connection, &settings_with(141, 400), 0).unwrap(), 1);
    let other = crate::db::open(&db.path).expect("另一连接");
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    other
        .execute(
            "INSERT INTO exposure_setting_versions(project_id,version,settings_json,actor) SELECT project_id,version,settings_json,'other' FROM exposure_settings WHERE project_id=141",
            [],
        )
        .unwrap();
    other
        .execute(
            "UPDATE exposure_settings SET version=2,settings_json=?1 WHERE project_id=141",
            [settings_with(141, 777).to_settings_json().to_string()],
        )
        .unwrap();
    let path = db.path.clone();
    let racing = std::thread::spawn(move || {
        let own = crate::db::open(&path).expect("编辑器连接");
        save(&own, &settings_with(141, 555), 1)
    });
    std::thread::sleep(Duration::from_millis(300));
    other.execute_batch("COMMIT").unwrap();
    let error = racing.join().unwrap().expect_err("读到 v1 的保存必须被拒");
    assert!(error.contains("[exposure_setting_conflict]") && error.contains("v2"), "{error}");
    assert_eq!(current_version(&connection, 141), 2);
    assert_eq!(super::settings::load(&connection, 141).unwrap().max_requests, 777);
    assert_eq!(
        count(&connection, "SELECT COUNT(*) FROM exposure_setting_versions WHERE project_id=141"),
        1,
        "失败的保存不新增历史"
    );
}

/// T41-ish / P1-2: more than 200 history versions. `truncated` is true exactly when older
/// versions exist beyond the window, at and around the boundary, and a corrupt history row is
/// labelled instead of being shown as an empty configuration.
#[test]
fn g0_history_window_reports_real_truncation_at_the_boundary() {
    let (_db, connection) = temp_db(141);
    for version in 0..251 {
        save(&connection, &settings_with(141, 100 + version), version).unwrap();
    }
    assert_eq!(current_version(&connection, 141), 251);
    let page = |limit: i64| super::super::exposure_setting_versions_in(&connection, 141, limit).unwrap();
    let full = page(500);
    assert_eq!(full["limit"], json!(200), "窗口上限 200");
    assert_eq!(full["items"].as_array().unwrap().len(), 200);
    assert_eq!(full["truncated"], json!(true), "250 个历史版本只显示 200 个");
    assert_eq!(full["items"][0]["version"], json!(250));
    assert_eq!(full["items"][199]["version"], json!(51));
    connection
        .execute("DELETE FROM exposure_setting_versions WHERE project_id=141 AND version<=50", [])
        .unwrap();
    assert_eq!(page(200)["truncated"], json!(false), "恰好 200 个：不截断");
    assert_eq!(page(199)["truncated"], json!(true), "199 的窗口少 1 个：截断");
    assert_eq!(page(199)["items"].as_array().unwrap().len(), 199);
    connection
        .execute("UPDATE exposure_setting_versions SET settings_json='{broken' WHERE project_id=141 AND version=250", [])
        .unwrap();
    let flagged = page(1);
    assert_eq!(flagged["items"][0]["corrupt"], json!(true));
    assert!(flagged["items"][0]["settings"].is_null(), "{flagged}");
    assert_eq!(flagged["truncated"], json!(true));
    let empty = super::super::exposure_setting_versions_in(&connection, 999, 20).unwrap();
    assert_eq!((empty["items"].as_array().unwrap().len(), &empty["truncated"]), (0, &json!(false)));
}

/// P1-3: a migration/table failure on the command path is an explicit error with a stable
/// code; the scan entry refuses instead of starting with defaults, and existing rows stay.
#[test]
fn g0_schema_failures_are_reported_and_existing_data_is_kept() {
    let (db, connection) = temp_db(141);
    save(&connection, &settings_with(141, 4_000), 0).unwrap();
    assert!(super::super::exposure_connection(&db.path).is_ok(), "健康的库可以打开");
    // 损坏：一个附加表被同名视图占位，迁移无法补列。
    connection
        .execute_batch("DROP TABLE exposure_review_events; CREATE VIEW exposure_review_events AS SELECT 1 AS id;")
        .unwrap();
    let error = super::super::exposure_connection(&db.path).expect_err("损坏结构必须报错");
    assert!(error.starts_with("[exposure_schema_unavailable]"), "{error}");
    let scan = super::super::exposure_scan_settings(&connection, 141).expect_err("不能带着损坏结构开始采集");
    assert!(scan.contains("[exposure_schema_unavailable]"), "{scan}");
    assert_eq!(current_version(&connection, 141), 1, "原数据保留");
    assert_eq!(super::settings::load(&connection, 141).unwrap().max_requests, 4_000);
    // 不可写：全新库上附加结构无法建立。
    let root = std::env::temp_dir().join(format!("oviraptor-exposure-g0ro-{}", uuid::Uuid::new_v4()));
    let path = crate::db::initialize(&root.join("app")).expect("临时库");
    let readonly = crate::db::open(&path).expect("连接");
    readonly.pragma_update(None, "query_only", "ON").unwrap();
    let error = super::super::exposure_schema_ready(&readonly).expect_err("不可写结构必须报错");
    assert!(error.starts_with("[exposure_schema_unavailable]"), "{error}");
    let error = super::super::exposure_scan_settings(&readonly, 141).expect_err("不可写结构不能开始采集");
    assert!(error.contains("[exposure_schema_unavailable]"), "{error}");
    drop(readonly);
    let _ = std::fs::remove_dir_all(root);
}

/// P1-3: a corrupt saved configuration is neither shown nor executed as the defaults; it can
/// still be repaired by saving a new version, and the corrupt bytes stay in history.
#[test]
fn g0_corrupt_saved_settings_refuse_view_and_scan_but_can_be_repaired() {
    let (_db, connection) = temp_db(141);
    save(&connection, &settings_with(141, 4_000), 0).unwrap();
    connection
        .execute("UPDATE exposure_settings SET settings_json='{not json' WHERE project_id=141", [])
        .unwrap();
    let view = super::super::exposure_settings_view_in(&connection, 141).expect_err("损坏配置不能显示为默认值");
    assert!(view.contains("[exposure_settings_corrupt]"), "{view}");
    let scan = super::super::exposure_scan_settings(&connection, 141).expect_err("损坏配置不能按默认值采集");
    assert!(scan.contains("[exposure_settings_corrupt]"), "{scan}");
    let invalid = super::super::save_exposure_settings_in(&connection, 141, 1, &json!({"exposureMaxRequests": 0}));
    assert!(invalid.is_err(), "无效新值仍被拒");
    assert_eq!(current_version(&connection, 141), 1);
    let repaired = super::super::save_exposure_settings_in(&connection, 141, 1, &json!({"exposureMaxRequests": 300})).unwrap();
    assert_eq!(repaired, 2);
    let view = super::super::exposure_settings_view_in(&connection, 141).unwrap();
    assert_eq!((view["exposureMaxRequests"].clone(), view["storedVersion"].clone()), (json!(300), json!(2)));
    let archived: String = connection
        .query_row("SELECT settings_json FROM exposure_setting_versions WHERE project_id=141 AND version=1", [], |row| row.get(0))
        .unwrap();
    assert_eq!(archived, "{not json", "损坏原值原样留档");
    let stale = super::super::save_exposure_settings_in(&connection, 141, 1, &json!({"exposureMaxRequests": 301})).unwrap_err();
    assert!(stale.contains("[exposure_setting_conflict]"), "{stale}");
    let archived = super::super::save_exposure_settings_in(&connection, 141, 2, &json!({}));
    assert!(archived.is_ok());
    connection.execute("UPDATE projects SET status='archived' WHERE id=141", []).unwrap();
    let refused = super::super::save_exposure_settings_in(&connection, 141, 3, &json!({})).unwrap_err();
    assert!(refused.contains("[exposure_project_invalid]"), "{refused}");
}

/// G0 shared dependencies: the PSL boundary and the organization directory are reachable
/// read-only from the exposure module (no copy, no write). Prints the measured difference
/// between the shared PSL and the legacy last-two-labels cluster key.
#[test]
fn g0_shared_psl_and_directory_are_reachable_read_only() {
    let (_db, connection) = temp_db(141);
    let changes_before = connection.total_changes();
    let zj = crate::commands::ownership::ownership_domain_parts("www.zj.10086.cn").expect("可解析");
    assert_eq!(zj.registrable.as_deref(), Some("10086.cn"));
    let pages = crate::commands::ownership::ownership_domain_parts("team.github.io").expect("可解析");
    assert_eq!(pages.registrable.as_deref(), Some("team.github.io"));
    assert_eq!(pages.section.as_str(), "private");
    assert!(crate::commands::ownership::ownership_domain_parts("10.0.0.8").is_none(), "IP 没有注册域");
    assert_eq!(crate::commands::ownership::ownership_domain_parts("com.cn").and_then(|parts| parts.registrable), None);
    for host in ["team.github.io", "www.zj.10086.cn", "a.b.gov.cn", "shop.example.co.uk"] {
        // G1-1: registered_domain now reads the shared PSL; the legacy rule is kept by name.
        let legacy = super::clusters::legacy_last_two_labels(host);
        let shared = crate::commands::ownership::ownership_domain_parts(host).and_then(|parts| parts.registrable).unwrap_or_default();
        eprintln!("G0_MEASURE host={host} legacy_registered_domain={legacy} shared_psl={shared} psl_version={}", crate::commands::ownership::OWNERSHIP_PSL_VERSION);
    }
    assert!(crate::commands::ownership::org_directory_version(&connection).unwrap() >= 0);
    let entities = crate::commands::ownership::load_org_entities(&connection).unwrap();
    assert!(entities.iter().all(|entity| entity.directory_version >= 0));
    assert!(crate::commands::ownership::load_project_org_binding(&connection, 141).unwrap().is_none(), "未绑定项目没有单位");
    assert_eq!(connection.total_changes(), changes_before, "共享读取不写库");
}
