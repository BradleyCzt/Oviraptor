// G1 — settings/limits truthfulness (G1-0), PSL clusters (G1-1), unit projection (G1-2),
// classification and procurement exclusion (G1-3) and the closed loop (G1-4). Every test
// here runs real code against a temporary database and, where the network is involved, a
// localhost fixture; nothing touches the user's database.

use super::adapters::certificate_transparency::execute_bounded as fetch_ct_bounded;
use super::adapters::http::{PassiveClient, PassiveHttpLimits, RequestMeter};
use super::fixture::{response, routes, seed_scope, Fixture};
use super::orchestrator::{run_plan, start_or_reuse, ItemOutcome, KIND_CERTIFICATE_TRANSPARENCY, KIND_PASSIVE_WEB};
use super::settings::{save, ExposureSettings};
use super::state::{ensure_exposure_schema, SourceStatus, WorkItemStatus};
use super::summary::exposure_scope_summary;
use super::work_items::WorkItem;
use rusqlite::{params, Connection};
use serde_json::Value as JsonValue;
use std::cell::Cell;
use std::path::{Path, PathBuf};

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("oviraptor-exposure-g1-{tag}-{}", uuid::Uuid::new_v4()))
}

fn database(root: &Path) -> (Connection, PathBuf) {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    (connection, db_path)
}

fn project(connection: &Connection, id: i64) {
    connection
        .execute(
            "INSERT OR IGNORE INTO projects(id,name,status) VALUES(?1,?2,'active')",
            params![id, format!("g1-{id}")],
        )
        .unwrap();
}

fn client() -> PassiveClient {
    PassiveClient::new(PassiveHttpLimits {
        timeout: std::time::Duration::from_secs(5),
        ..PassiveHttpLimits::default()
    })
    .expect("客户端必须可用")
}

fn ct_item(entity: &str) -> WorkItem {
    WorkItem {
        id: 1,
        run_id: 1,
        project_id: 61,
        work_key: format!("{KIND_CERTIFICATE_TRANSPARENCY}|{entity}"),
        kind: KIND_CERTIFICATE_TRANSPARENCY.to_string(),
        source_key: KIND_CERTIFICATE_TRANSPARENCY.to_string(),
        asset_id: None,
        origin: String::new(),
        entity: entity.to_string(),
        input_hash: String::new(),
        status: WorkItemStatus::Pending,
        owner: String::new(),
        claim_version: 0,
        attempts: 0,
        observations: 0,
        requests: 0,
        bytes: 0,
    }
}

fn never_cancelled() -> bool {
    false
}

/// Saves passive-web-only settings that allow the localhost fixture and returns the JSON the
/// real start command would hand to the worker.
fn passive_settings(connection: &Connection, project_id: i64) -> JsonValue {
    let mut value = ExposureSettings::for_project(project_id);
    value.sources = vec![KIND_PASSIVE_WEB.to_string()];
    value.allow_private_destinations = true;
    value.timeout_seconds = 5;
    let current: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(version),0) FROM exposure_settings WHERE project_id=?1",
            [project_id],
            |row| row.get(0),
        )
        .unwrap();
    save(connection, &value, current).expect("配置必须能保存");
    super::super::exposure_scan_settings(connection, project_id).expect("配置必须可读")
}

fn run_row(connection: &Connection, run_id: i64) -> (String, String) {
    connection
        .query_row(
            "SELECT status,error FROM exposure_runs WHERE id=?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

// ───────────────────────────── G1-0 ─────────────────────────────

#[test]
fn g1_0_run_plan_refuses_a_missing_empty_or_corrupt_frozen_plan() {
    let root = sandbox("plan");
    let (connection, _) = database(&root);
    project(&connection, 401);
    let calls = Cell::new(0);
    let mut executor = |_item: &WorkItem| -> Result<ItemOutcome, String> {
        calls.set(calls.get() + 1);
        Ok(ItemOutcome { status: Some(WorkItemStatus::Completed), source_status: Some(SourceStatus::Completed), ..Default::default() })
    };
    let cases: [(i64, Option<&str>, &str, &str); 4] = [
        (40101, None, "[]", "[exposure_plan_missing]"),
        (40102, Some("{}"), "[]", "[exposure_plan_settings_missing]"),
        (40103, Some("{broken"), "[]", "[exposure_plan_settings_corrupt]"),
        (40104, Some(r#"{"exposureSources":["passive_web"]}"#), "not-json", "[exposure_plan_sources_corrupt]"),
    ];
    for (run_id, settings, sources, code) in cases {
        connection
            .execute(
                "INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(?1,401,'queued','planned')",
                [run_id],
            )
            .unwrap();
        if let Some(raw) = settings {
            connection
                .execute(
                    "INSERT INTO exposure_run_plans(run_id,project_id,sources_json,budget_requests,budget_bytes,planned_items,settings_version,settings_json)
                     VALUES(?1,401,?2,10,10000,0,3,?3)",
                    params![run_id, sources, raw],
                )
                .unwrap();
        }
        let error = run_plan(&connection, run_id, "g1", &mut executor)
            .err()
            .unwrap_or_else(|| panic!("第 {run_id} 轮必须被拒绝，不能按默认配置执行"));
        assert!(error.contains(code), "{run_id}: {error}");
        // 调用方（工作线程）随后收尾：每一轮都必须落到 failed，而不是停在 running。
        let _ = super::orchestrator::finalize(&connection, run_id, Some(&error));
        let (status, stored) = run_row(&connection, run_id);
        assert_eq!(status, "failed", "{run_id}: {stored}");
        assert!(stored.contains(code.trim_matches(|c| c == '[' || c == ']')), "{run_id}: {stored}");
    }
    assert_eq!(calls.get(), 0, "被拒绝的计划不能执行任何工作项");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_0_a_worker_with_a_corrupt_frozen_plan_fails_the_round_without_sending() {
    let root = sandbox("worker");
    let (connection, db_path) = database(&root);
    let fixture = Fixture::start(routes(vec![("/", vec![response(200, &[("Content-Type", "text/html")], "<html>ok</html>")])]));
    seed_scope(&connection, 402, &fixture.origin, true);
    let settings = passive_settings(&connection, 402);
    let started = start_or_reuse(&connection, 402, &settings).expect("本轮必须能开始");
    connection
        .execute("UPDATE exposure_run_plans SET settings_json='{broken' WHERE run_id=?1", [started.run_id])
        .unwrap();
    super::super::execute_exposure_run(db_path.clone(), started.run_id, 402, "g1-worker".into(), settings.clone());
    let (status, error) = run_row(&connection, started.run_id);
    assert_eq!(status, "failed", "{error}");
    assert!(error.contains("exposure_plan_settings_corrupt"), "{error}");
    assert_eq!(fixture.request_count(), 0, "冻结配置损坏时不能按默认值出站");

    // 正向对照：同一项目、未损坏的计划真正出站并完成。
    let second = start_or_reuse(&connection, 402, &settings).expect("新一轮");
    assert_ne!(second.run_id, started.run_id);
    super::super::execute_exposure_run(db_path, second.run_id, 402, "g1-worker".into(), settings);
    let (status, error) = run_row(&connection, second.run_id);
    assert!(fixture.request_count() > 0, "完好的计划必须真的出站");
    assert_ne!(status, "failed", "{error}");
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_0_ct_names_beyond_the_configured_cap_are_a_named_quota_gap() {
    let body = r#"[{"name_value":"a.corp.invalid\nb.corp.invalid\nc.corp.invalid","not_before":"2026-01-01","not_after":"2026-04-01"},{"name_value":"d.corp.invalid\ne.corp.invalid","not_before":"2026-01-01","not_after":"2026-04-01"}]"#;
    let fixture = Fixture::start(routes(vec![("/", vec![response(200, &[("Content-Type", "application/json")], body)])]));
    let mut meter = RequestMeter { max_requests: 10, max_bytes: 1_000_000, ..Default::default() };
    let capped = fetch_ct_bounded(&client(), &ct_item("corp.invalid"), &mut meter, std::slice::from_ref(&fixture.origin), true, 2, &never_cancelled);
    let names: Vec<&str> = capped.observations.iter().filter(|row| row.kind == "ct_name").map(|row| row.url.as_str()).collect();
    assert_eq!(names, vec!["https://a.corp.invalid", "https://b.corp.invalid"]);
    assert_eq!(capped.status, Some(WorkItemStatus::BlockedQuota));
    assert_eq!(capped.source_status, Some(SourceStatus::Partial));
    assert!(capped.reason.contains("返回 5 个名称") && capped.reason.contains("其余 3 个未记录"), "{}", capped.reason);
    assert_eq!(capped.pending.len(), 2, "被截掉的名称也不能进入待确认队列");

    let full = fetch_ct_bounded(&client(), &ct_item("corp.invalid"), &mut meter, std::slice::from_ref(&fixture.origin), true, 5, &never_cancelled);
    assert_eq!(full.observations.iter().filter(|row| row.kind == "ct_name").count(), 5);
    assert_eq!(full.status, Some(WorkItemStatus::Completed), "恰好等于上限不是截断：{}", full.reason);
    assert_eq!(full.source_status, Some(SourceStatus::Completed));
    drop(fixture);
}

#[test]
fn g1_0_summary_limits_follow_the_project_settings_and_are_not_invented_when_corrupt() {
    let root = sandbox("summary");
    let (connection, _) = database(&root);
    seed_scope(&connection, 403, "http://127.0.0.1:9", true);
    let defaults = exposure_scope_summary(&connection, 403).unwrap();
    assert_eq!(defaults.limits_basis, "defaults");
    assert_eq!((defaults.eligible_assets_total, defaults.eligible_assets_planned), (1, 1));
    let cap = |rows: &[super::contract::ExposureLimit], key: &str| rows.iter().find(|row| row.key == key).map(|row| row.cap);
    assert_eq!(cap(&defaults.known_limits, "passive_assets"), Some(500));
    assert_eq!(cap(&defaults.known_limits, "visit_queue"), Some(80));

    let mut edited = ExposureSettings::for_project(403);
    edited.ct_page_size = 7;
    edited.archive_page_size = 9;
    save(&connection, &edited, 0).unwrap();
    let saved = exposure_scope_summary(&connection, 403).unwrap();
    assert_eq!(saved.limits_basis, "project_settings_v1");
    assert_eq!(cap(&saved.known_limits, "ct_names_per_domain"), Some(7));
    assert_eq!(cap(&saved.known_limits, "wayback_rows"), Some(9));

    connection
        .execute("UPDATE exposure_settings SET settings_json='{oops' WHERE project_id=403", [])
        .unwrap();
    let corrupt = exposure_scope_summary(&connection, 403).unwrap();
    assert_eq!(corrupt.limits_basis, "settings_corrupt");
    assert!(corrupt.limits_error.as_deref().unwrap_or("").contains("exposure_settings_corrupt"));
    assert!(corrupt.known_limits.is_empty(), "损坏的配置不能报告一组默认上限");
    assert_eq!((corrupt.eligible_assets_planned, corrupt.assets_beyond_plan), (0, 1));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_0_settings_view_says_execution_is_serial() {
    let root = sandbox("view");
    let (connection, _) = database(&root);
    project(&connection, 404);
    let mut edited = ExposureSettings::for_project(404);
    edited.concurrency = 6;
    save(&connection, &edited, 0).unwrap();
    let view = super::super::exposure_settings_view_in(&connection, 404).unwrap();
    assert!(view.get("exposureConcurrency").is_none(), "并行度不再出现在采集合同里");
    assert_eq!(view["effective"]["concurrency"], 1, "实际并行度必须报告为 1");
    assert_eq!(view["effective"]["concurrencyImplemented"], false);
    assert_eq!(view["effective"]["knownLimits"].as_array().map(Vec::len), Some(7));
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G1-1 ─────────────────────────────

#[test]
fn g1_1_cluster_boundaries_come_from_the_shared_psl() {
    use super::clusters::{boundary_label, legacy_last_two_labels, registrable_boundary, registered_domain, BOUNDARY_EXACT_HOST};
    let psl = format!("psl:{}", crate::commands::ownership::OWNERSHIP_PSL_VERSION);
    assert_eq!(registrable_boundary("shop.a.example.com.cn"), ("example.com.cn".to_string(), psl.clone()));
    assert_eq!(legacy_last_two_labels("shop.a.example.com.cn"), "com.cn", "旧规则会把所有 .com.cn 合并");
    assert_eq!(registrable_boundary("team.github.io"), ("team.github.io".to_string(), psl.clone()));
    assert_ne!(registered_domain("team.github.io"), registered_domain("club.github.io"), "github.io 私有后缀下的租户不能合并");
    assert_eq!(registrable_boundary("WWW.Example.CO.UK."), ("example.co.uk".to_string(), psl.clone()));
    assert_eq!(registrable_boundary("*.API.Corp.Invalid"), ("corp.invalid".to_string(), psl));
    for suffix_or_ip in ["github.io", "com.cn", "10.0.0.8"] {
        assert_eq!(
            registrable_boundary(suffix_or_ip),
            (suffix_or_ip.to_string(), BOUNDARY_EXACT_HOST.to_string()),
            "{suffix_or_ip} 没有可注册域，只能按精确主机分组"
        );
    }
    assert_eq!(registrable_boundary("  "), (String::new(), String::new()));
    assert!(boundary_label("domain", "", "").contains("旧版"));
    assert!(boundary_label("domain", "psl", "psl-x").contains("psl-x"));
}

fn asset_id_of(connection: &Connection, origin: &str) -> i64 {
    connection
        .query_row("SELECT id FROM assets WHERE asset_key=?1", [origin], |row| row.get(0))
        .unwrap()
}

fn observe(connection: &Connection, run_id: i64, project_id: i64, origin: &str, body: &str) -> i64 {
    let mut row = super::observations::NewObservation::new(KIND_PASSIVE_WEB, "page", &format!("{origin}/"));
    row.asset_id = Some(asset_id_of(connection, origin));
    row.status_code = 200;
    row.body = body.as_bytes().to_vec();
    row.body_bytes = body.len() as i64;
    super::observations::record(connection, run_id, project_id, None, &row).unwrap().0
}

#[test]
fn g1_1_new_clusters_use_psl_boundaries_and_never_absorb_legacy_rows() {
    let root = sandbox("psl");
    let (connection, _) = database(&root);
    let origins = [
        "https://a.example.com.cn",
        "https://b.example.com.cn",
        "https://x.other.com.cn",
        "https://team.github.io",
        "https://club.github.io",
        "http://10.0.0.8:8080",
        "http://10.0.0.8:9090",
    ];
    for origin in origins {
        seed_scope(&connection, 411, origin, true);
    }
    connection
        .execute("INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(41100,411,'running','executing')", [])
        .unwrap();
    let ids: Vec<i64> = origins
        .iter()
        .enumerate()
        .map(|(index, origin)| observe(&connection, 41100, 411, origin, &format!("body-{index}")))
        .collect();
    let basis: String = connection
        .query_row("SELECT root_basis FROM exposure_observations WHERE id=?1", [ids[0]], |row| row.get(0))
        .unwrap();
    assert_eq!(basis, format!("psl:{}", crate::commands::ownership::OWNERSHIP_PSL_VERSION), "观察必须记录边界依据");
    super::cluster_repository::persist_for_observations(&connection, 41100, 411, &ids, 1).unwrap();
    let clusters = |connection: &Connection| -> Vec<(String, String, String, i64)> {
        let mut statement = connection
            .prepare("SELECT cluster_key,boundary_basis,psl_version,member_count FROM exposure_clusters WHERE project_id=411 AND kind='domain' ORDER BY cluster_key")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert_eq!(
        clusters(&connection),
        vec![
            ("domain:example.com.cn".to_string(), "psl".to_string(), crate::commands::ownership::OWNERSHIP_PSL_VERSION.to_string(), 2),
            ("host:10.0.0.8".to_string(), "exact_host".to_string(), String::new(), 2),
        ],
        "other.com.cn 与 github.io 租户都不能被合并；IP 只按精确主机分组"
    );

    // 旧历史：一条 G1 之前写入的观察恰好拼出同名注册域，但边界依据为空（末两段）。
    connection
        .execute("UPDATE exposure_observations SET root_domain='example.com.cn',root_basis='' WHERE id=?1", [ids[2]])
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_clusters(project_id,cluster_key,kind,rule_version,member_count) VALUES(411,'domain:com.cn','domain','oviraptor.exposure.rules.v1',3)",
            [],
        )
        .unwrap();
    super::cluster_repository::persist_for_observations(&connection, 41100, 411, &[ids[0], ids[2]], 1).unwrap();
    let after = clusters(&connection);
    assert!(after.contains(&("domain:example.com.cn".to_string(), "psl".to_string(), crate::commands::ownership::OWNERSHIP_PSL_VERSION.to_string(), 2)), "旧行不能被并入新簇：{after:?}");
    let (direct, inherited): (i64, i64) = connection
        .query_row("SELECT direct_count,inherited_count FROM exposure_clusters WHERE project_id=411 AND cluster_key='domain:example.com.cn'", [], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!((direct, inherited), (0, 2), "旧边界的观察不能进入新簇的成员统计");
    // 更早一轮对同一批资产的旧观察（末两段 com.cn、依据为空）：新的分组只能由新观察代表。
    connection
        .execute("INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(41090,411,'completed','completed')", [])
        .unwrap();
    let legacy_a = observe(&connection, 41090, 411, origins[0], "old-a");
    let legacy_b = observe(&connection, 41090, 411, origins[1], "old-b");
    connection
        .execute("UPDATE exposure_observations SET root_domain='com.cn',root_basis='' WHERE id IN (?1,?2)", [legacy_a, legacy_b])
        .unwrap();
    super::cluster_repository::persist_for_observations(&connection, 41100, 411, &[ids[0], legacy_a, legacy_b], 1).unwrap();
    let representative: i64 = connection
        .query_row("SELECT representative_observation_id FROM exposure_clusters WHERE project_id=411 AND cluster_key='domain:example.com.cn'", [], |row| row.get(0))
        .unwrap();
    assert!([ids[0], ids[1]].contains(&representative), "代表观察 {representative} 来自旧边界的观察");
    let page = super::cluster_repository::page_clusters(&connection, 411, "domain", "", "", 50).unwrap();
    let legacy = page.items.iter().find(|row| row.cluster_key == "domain:com.cn").expect("旧簇必须仍可读");
    assert_eq!(legacy.boundary_basis, "");
    assert!(legacy.boundary_label.contains("旧版末两段"), "{}", legacy.boundary_label);
    let fresh = page.items.iter().find(|row| row.cluster_key == "domain:example.com.cn").unwrap();
    assert!(fresh.boundary_label.contains(crate::commands::ownership::OWNERSHIP_PSL_VERSION), "{}", fresh.boundary_label);
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G1-2 ─────────────────────────────

fn entity(connection: &Connection, id: i64, kind: &str, name: &str, group: Option<i64>, parent: Option<i64>, region: &str) {
    connection
        .execute(
            "INSERT INTO org_entities(id,group_id,parent_id,entity_kind,legal_name,normalized_name,region_code,directory_version) VALUES(?1,?2,?3,?4,?5,?6,?7,1)",
            params![id, group, parent, kind, name, format!("{name}-{id}"), region],
        )
        .unwrap();
}

fn bind(connection: &Connection, project_id: i64, kind: &str, group: i64, unit: Option<i64>) {
    connection
        .execute(
            "INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,unit_entity_id,binding_version) VALUES(?1,?2,?3,?4,3)",
            params![project_id, kind, group, unit],
        )
        .unwrap();
}

/// A small directory: group 1; province 2 (浙江); professional subsidiary 3 under the group;
/// city 4 under the professional subsidiary but named and coded like Zhejiang; city 5 and
/// county 6 under the province; professional subsidiary 7 registered *under the province*
/// with its own city branch 8 — the case a parent-chain walk must not turn into "浙江".
fn directory(connection: &Connection) {
    connection
        .execute("INSERT OR REPLACE INTO org_directory_state(id,version) VALUES(1,5)", [])
        .unwrap();
    entity(connection, 1, "group", "中国移动通信集团有限公司", None, None, "");
    entity(connection, 2, "province_company", "中国移动通信集团浙江有限公司", Some(1), Some(1), "330000");
    entity(connection, 3, "professional_subsidiary", "咪咕文化科技有限公司", Some(1), Some(1), "");
    entity(connection, 4, "city_branch", "咪咕浙江杭州分公司", Some(1), Some(3), "330100");
    entity(connection, 5, "city_branch", "中国移动杭州分公司", Some(1), Some(2), "330100");
    entity(connection, 6, "county_branch", "中国移动余杭分公司", Some(1), Some(5), "330110");
    entity(connection, 7, "professional_subsidiary", "浙江和创专业公司", Some(1), Some(2), "330000");
    entity(connection, 8, "city_branch", "浙江和创杭州分公司", Some(1), Some(7), "330100");
}

fn members(connection: &Connection, project_id: i64, prefix: &str, count: usize) -> Vec<String> {
    (0..count)
        .map(|index| {
            let origin = format!("https://{prefix}{index}.example.com.cn");
            seed_scope(connection, project_id, &origin, true);
            origin
        })
        .collect()
}

#[test]
fn g1_2_an_unbound_project_counts_confirmed_assets_as_unassigned_without_guessing() {
    let root = sandbox("unbound");
    let (connection, _) = database(&root);
    let origins = members(&connection, 421, "u", 4);
    let ids: Vec<i64> = origins.iter().map(|origin| asset_id_of(&connection, origin)).collect();
    connection.execute("UPDATE asset_ownership_decisions SET exposure_eligible=0 WHERE project_id=421 AND asset_id=?1", [ids[1]]).unwrap();
    connection.execute("UPDATE asset_ownership_decisions SET status='attributed',exposure_eligible=0 WHERE project_id=421 AND asset_id=?1", [ids[2]]).unwrap();
    connection.execute("UPDATE project_assets SET is_deleted=1 WHERE project_id=421 AND asset_id=?1", [ids[3]]).unwrap();
    let before = connection.total_changes();
    let projection = super::units::project_units(&connection, 421).unwrap();
    assert_eq!(connection.total_changes(), before, "单位投影只读");
    assert_eq!(projection.binding_status, "unbound");
    assert_eq!(projection.unit_bucket, "none");
    assert!(projection.unit_province.is_none());
    let counts = &projection.counts;
    assert_eq!((counts.members, counts.candidate_linked, counts.eligible), (3, 1, 1));
    assert_eq!((counts.confirmed_group, counts.confirmed_unit), (None, None), "未绑定项目不能声称集团或单位资产");
    assert_eq!((counts.unassigned, counts.pending_authorization), (2, 1));
    assert_eq!((counts.planned, counts.executed, counts.uncovered, counts.latest_run_id), (0, 0, 1, None));
    assert_eq!(projection.per_asset_edge, "ownership_projection");
    assert!(projection.per_unit.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_2_a_professional_subsidiary_is_never_placed_under_a_province() {
    let root = sandbox("units");
    let (connection, _) = database(&root);
    directory(&connection);
    let entities = crate::commands::ownership::load_org_entities(&connection).unwrap();
    let by_id: std::collections::HashMap<i64, &crate::commands::ownership::OrgEntityView> = entities.iter().map(|row| (row.id, row)).collect();
    let place = |id: i64| super::units::place_unit(by_id[&id], &by_id);
    let (bucket, province, note) = place(4);
    assert_eq!((bucket.as_str(), province.is_none()), ("professional_subsidiary", true), "名字和地区码像浙江也不能归入浙江省公司");
    assert!(note.unwrap_or_default().contains("专业公司"));
    let (bucket, province, _) = place(8);
    assert_eq!((bucket.as_str(), province.is_none()), ("professional_subsidiary", true), "挂在省公司下的专业公司，其分公司也不归省");
    assert_eq!(place(7).0, "professional_subsidiary");
    assert!(place(7).1.is_none());
    let (bucket, province, _) = place(6);
    assert_eq!(bucket, "county");
    assert_eq!(province.map(|row| row.entity_id), Some(2), "县公司经地市公司归入省公司");
    assert_eq!(place(2).1.map(|row| row.entity_id), Some(2));

    // 单位项目：绑定专业公司的地市分公司。
    members(&connection, 422, "p", 2);
    bind(&connection, 422, "unit", 1, Some(4));
    let projection = super::units::project_units(&connection, 422).unwrap();
    assert_eq!(projection.binding_status, "unit");
    assert_eq!(projection.unit_bucket, "professional_subsidiary");
    assert!(projection.unit_province.is_none());
    assert_eq!((projection.counts.confirmed_unit, projection.counts.unassigned), (Some(2), 0));
    assert_eq!(projection.directory_version, 5);

    // 集团总账：这些确认没有经过目录复核组决定，没有权威单位依据，全部“未分配单位”，不按名称分到任何省。
    members(&connection, 423, "g", 3);
    bind(&connection, 423, "group_ledger", 1, None);
    let ledger = super::units::project_units(&connection, 423).unwrap();
    assert_eq!(ledger.binding_status, "group_ledger");
    assert_eq!((ledger.counts.confirmed_group, ledger.counts.confirmed_unit, ledger.counts.unassigned), (Some(3), Some(0), 3));
    assert!(ledger.per_unit.is_empty());
    assert_eq!(ledger.scope_entities, 8);

    // 绑定的单位已退役：按未绑定处理，不保留旧的单位结论。
    members(&connection, 424, "r", 1);
    bind(&connection, 424, "unit", 1, Some(5));
    connection.execute("UPDATE org_entities SET status='retired' WHERE id=5", []).unwrap();
    let broken = super::units::project_units(&connection, 424).unwrap();
    assert_eq!(broken.binding_status, "binding_broken");
    assert_eq!((broken.counts.confirmed_group, broken.counts.confirmed_unit, broken.counts.unassigned), (None, None, 1));

    // 父级成环：不判定省公司，并说明原因。
    entity(&connection, 9, "city_branch", "环甲", Some(1), None, "");
    entity(&connection, 10, "city_branch", "环乙", Some(1), Some(9), "");
    connection.execute("UPDATE org_entities SET parent_id=10 WHERE id=9", []).unwrap();
    let entities = crate::commands::ownership::load_org_entities(&connection).unwrap();
    let by_id: std::collections::HashMap<i64, &crate::commands::ownership::OrgEntityView> = entities.iter().map(|row| (row.id, row)).collect();
    let (_, province, note) = super::units::place_unit(by_id[&9], &by_id);
    assert!(province.is_none());
    assert!(note.unwrap_or_default().contains("成环"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_2_planned_executed_and_uncovered_follow_the_latest_round() {
    let root = sandbox("coverage");
    let (connection, _) = database(&root);
    let fixture_origin = "http://127.0.0.1:9";
    seed_scope(&connection, 425, fixture_origin, true);
    let settings = passive_settings(&connection, 425);
    let started = start_or_reuse(&connection, 425, &settings).expect("本轮必须能开始");
    let mut executor = |_item: &WorkItem| -> Result<ItemOutcome, String> {
        Ok(ItemOutcome { status: Some(WorkItemStatus::Completed), source_status: Some(SourceStatus::Completed), ..Default::default() })
    };
    run_plan(&connection, started.run_id, "g1", &mut executor).unwrap();
    let counts = super::units::project_units(&connection, 425).unwrap().counts;
    assert_eq!((counts.planned, counts.executed, counts.uncovered, counts.latest_run_id), (1, 1, 0, Some(started.run_id)));
    // 采集后：原资产撤销准入，新增一个准入资产 → 已执行的那个不再算覆盖，新资产未覆盖。
    let first = asset_id_of(&connection, fixture_origin);
    connection.execute("UPDATE asset_ownership_decisions SET exposure_eligible=0 WHERE project_id=425 AND asset_id=?1", [first]).unwrap();
    seed_scope(&connection, 425, "http://127.0.0.1:10", true);
    let counts = super::units::project_units(&connection, 425).unwrap().counts;
    assert_eq!((counts.eligible, counts.executed, counts.uncovered), (1, 1, 1), "撤销准入的资产不能顶替新资产的覆盖");
    let summary = exposure_scope_summary(&connection, 425).unwrap();
    assert!(summary.unit_projection_error.is_none());
    assert_eq!(summary.unit_projection.expect("汇总必须带单位投影").counts.uncovered, 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_0_a_resume_in_a_later_second_still_matches_its_frozen_scope() {
    let root = sandbox("resume");
    let (connection, _) = database(&root);
    seed_scope(&connection, 405, "http://127.0.0.1:9", true);
    let settings = passive_settings(&connection, 405);
    let started = start_or_reuse(&connection, 405, &settings).expect("本轮必须能开始");
    let plan_row = super::recovery::load_plan(&connection, started.run_id, 405).unwrap();
    assert!(plan_row.scope_hash.starts_with(super::orchestrator::SCOPE_HASH_PREFIX));
    // 跨过一个整秒：G0 的范围指纹含签发时间，这里必然被误判为“范围已变化”。
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    let frozen = super::recovery::frozen_settings_strict(&plan_row).unwrap().to_settings_json();
    let scope = super::scope::ExposureScope::load(&connection, 405, &frozen).unwrap();
    super::recovery::verify_frozen(&plan_row, &scope, &frozen).expect("同一范围晚一秒恢复必须通过");
    // 真实的范围变化仍被拒绝：新增一个准入资产。
    seed_scope(&connection, 405, "http://127.0.0.1:10", true);
    let widened = super::scope::ExposureScope::load(&connection, 405, &frozen).unwrap();
    let error = super::recovery::verify_frozen(&plan_row, &widened, &frozen).unwrap_err();
    assert!(error.contains("exposure_plan_frozen_mismatch"), "{error}");
    // G1 之前写入的旧指纹：明确拒绝并说明原因，而不是说成范围变化。
    let mut legacy = plan_row.clone();
    legacy.scope_hash = "0f".repeat(32);
    let error = super::recovery::verify_frozen(&legacy, &scope, &frozen).unwrap_err();
    assert!(error.contains("exposure_plan_scope_hash_legacy"), "{error}");
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G1-3 ─────────────────────────────

fn draft(category: &str, title: &str, url: &str, severity: &str, confidence: i64) -> super::observations::FindingDraft {
    super::observations::FindingDraft {
        category: category.to_string(),
        title: title.to_string(),
        url: url.to_string(),
        source_type: KIND_PASSIVE_WEB.to_string(),
        evidence: format!("{title} @ {url}"),
        severity: severity.to_string(),
        confidence,
        asset_id: None,
        content_hash: String::new(),
    }
}

const NOTICE: &str = "<html><head><title>某省公司2026年核心网设备采购项目招标公告</title></head><body>\
<h1>招标公告</h1><p>项目编号：ZJ-2026-0815</p><p>采购人：某省公司</p><p>代理机构：某招标代理有限公司</p>\
<p>投标截止时间：2026-11-01 09:30</p><p>采购需求：核心网设备一批</p></body></html>";

#[test]
fn g1_3_procurement_is_judged_by_the_page_and_mixed_pages_stay_with_a_human() {
    use super::classify::{classify, page_context, DISPOSITION_EXCLUDED_MATERIAL, DISPOSITION_EXCLUDED_PROCUREMENT, DISPOSITION_PENDING, DISPOSITION_RISK};
    use super::rules::{CREDENTIAL_PLACEHOLDER_CONFIDENCE, TITLE_MAP_BUSINESS, TITLE_MAP_VENDOR};
    let notice_url = "https://www.example.com.cn/notice/2026-0815.html";
    let notice = page_context(notice_url, NOTICE.as_bytes(), "text/html; charset=utf-8");
    assert!(notice.procurement_confirmed(), "{notice:?}");
    assert!(notice.structural_markers.len() >= 2);
    let doc = draft("document", "公开路径疑似内部文档", notice_url, "high", 60);
    let verdict = classify(&doc, KIND_PASSIVE_WEB, &notice);
    assert_eq!(verdict.disposition, DISPOSITION_EXCLUDED_PROCUREMENT, "{verdict:?}");
    assert_eq!((verdict.info_category, verdict.source_channel), ("internal_business", "first_party_web"));
    assert!(verdict.markers.iter().any(|m| m == "title:招标公告"), "{:?}", verdict.markers);

    // 混合页面：招采公告里同时出现凭据候选或业务源码图 → 只能留人工。
    let cred = draft("credential", "公开资源出现凭据字段", notice_url, "high", CREDENTIAL_PLACEHOLDER_CONFIDENCE + 37);
    assert_eq!(classify(&cred, KIND_PASSIVE_WEB, &notice).disposition, DISPOSITION_PENDING);
    let map = draft("source_code", TITLE_MAP_BUSINESS, notice_url, "high", 68);
    let mixed = classify(&map, KIND_PASSIVE_WEB, &notice);
    assert_eq!(mixed.disposition, DISPOSITION_PENDING);
    assert!(mixed.reason.contains("混合"), "{}", mixed.reason);

    // 只有标题、结构字段不足 2 个 → 待分类。
    let thin = "<title>设备采购公告</title><p>采购人：某公司</p>";
    let thin_context = page_context(notice_url, thin.as_bytes(), "text/html");
    assert!(!thin_context.procurement_confirmed());
    assert_eq!(classify(&doc, KIND_PASSIVE_WEB, &thin_context).disposition, DISPOSITION_PENDING);
    // 正文里提到招标公告、但标题是新闻 → 不是招采页面。
    let news = "<title>公司新闻</title><p>本月发布招标公告 3 篇，项目编号、采购人见附件</p>";
    let news_context = page_context("https://www.example.com.cn/news/1.html", news.as_bytes(), "text/html");
    assert!(news_context.title_markers.is_empty() && !news_context.procurement_signal());
    assert_eq!(classify(&doc, KIND_PASSIVE_WEB, &news_context).disposition, DISPOSITION_RISK);
    // 公认招采平台 + 一个结构字段 → 排除；平台首页没有任何字样 → 按普通规则。
    let platform = page_context("https://www.ccgp.gov.cn/cggg/dfgg/1.htm", "<p>项目编号：A-1</p>".as_bytes(), "text/html");
    assert_eq!(platform.canonical_host.as_deref(), Some("ccgp.gov.cn"));
    assert_eq!(classify(&doc, KIND_PASSIVE_WEB, &platform).disposition, DISPOSITION_EXCLUDED_PROCUREMENT);
    let lookalike = page_context("https://ccgp.gov.cn.attacker.invalid/1.htm", "<p>项目编号：A-1</p>".as_bytes(), "text/html");
    assert_eq!(lookalike.canonical_host, None, "仿冒主机不能算招采平台");
    // 只有文件名含招采字样（PDF 正文不可读）→ 待分类，绝不排除。
    let pdf_url = "https://www.example.com.cn/files/%E6%8B%9B%E6%A0%87%E6%96%87%E4%BB%B6-%E5%86%85%E9%83%A8%E6%9E%B6%E6%9E%84.pdf";
    let pdf = page_context(pdf_url, b"%PDF-1.7 binary", "application/pdf");
    assert_eq!(pdf.filename_hint.as_deref(), Some("招标"));
    let pdf_verdict = classify(&draft("document", "公开路径疑似内部文档", pdf_url, "medium", 50), KIND_PASSIVE_WEB, &pdf);
    assert_eq!(pdf_verdict.disposition, DISPOSITION_PENDING, "{pdf_verdict:?}");

    // 公开材料：第三方 Source Map、占位凭据。
    let plain = page_context("https://www.example.com.cn/app.js.map", b"{}", "application/json");
    let vendor = draft("source_code", TITLE_MAP_VENDOR, "https://www.example.com.cn/app.js.map", "low", 35);
    assert_eq!(classify(&vendor, KIND_PASSIVE_WEB, &plain).disposition, DISPOSITION_EXCLUDED_MATERIAL);
    let placeholder = draft("credential", "公开资源出现凭据字段", "https://www.example.com.cn/config.json", "low", CREDENTIAL_PLACEHOLDER_CONFIDENCE);
    assert_eq!(classify(&placeholder, KIND_PASSIVE_WEB, &plain).disposition, DISPOSITION_EXCLUDED_MATERIAL);
    let real = draft("credential", "公开资源出现凭据字段", "https://www.example.com.cn/config.json", "high", 62);
    let real_verdict = classify(&real, KIND_PASSIVE_WEB, &plain);
    assert_eq!((real_verdict.disposition, real_verdict.info_category), (DISPOSITION_RISK, "credential"));
    // 员工资料、未知规则类别、未知来源。
    let staff = draft("document", "公开路径疑似内部文档", "https://www.example.com.cn/hr/%E5%91%98%E5%B7%A5%E5%90%8D%E5%8D%95.xlsx", "medium", 50);
    assert_eq!(classify(&staff, KIND_PASSIVE_WEB, &plain).info_category, "employee_data");
    let odd = classify(&draft("weird", "?", "https://www.example.com.cn/x", "low", 10), KIND_PASSIVE_WEB, &plain);
    assert_eq!((odd.disposition, odd.info_category), (DISPOSITION_PENDING, "unclassified"));
    // G2 made `github_code` a known code_platform source; an unmapped key still stays pending.
    let unknown_source = classify(&real, "pastebin_feed", &plain);
    assert_eq!((unknown_source.disposition, unknown_source.source_channel), (DISPOSITION_PENDING, "unrecognized_source"));
    assert_eq!(classify(&real, "web_archive", &plain).source_channel, "archive_index");
}

/// Runs one round whose single passive-web item returns `pages` (url, content type, body)
/// with `findings`, then finalizes it.
fn round_with(
    connection: &Connection,
    project_id: i64,
    origin: &str,
    pages: &[(String, &str, String)],
    findings: Vec<super::observations::FindingDraft>,
) -> i64 {
    let settings = passive_settings(connection, project_id);
    let started = start_or_reuse(connection, project_id, &settings).expect("本轮必须能开始");
    let asset = asset_id_of(connection, origin);
    let mut executor = |item: &WorkItem| -> Result<ItemOutcome, String> {
        if item.kind != KIND_PASSIVE_WEB {
            return Ok(ItemOutcome { status: Some(WorkItemStatus::Completed), source_status: Some(SourceStatus::Completed), ..Default::default() });
        }
        let observations = pages
            .iter()
            .map(|(url, content_type, body)| {
                let mut row = super::observations::NewObservation::new(KIND_PASSIVE_WEB, "page", url);
                row.asset_id = Some(asset);
                row.status_code = 200;
                row.content_type = content_type.to_string();
                row.body = body.as_bytes().to_vec();
                row.body_bytes = body.len() as i64;
                row
            })
            .collect();
        let findings = findings
            .iter()
            .cloned()
            .map(|mut row| {
                row.asset_id = Some(asset);
                row
            })
            .collect();
        Ok(ItemOutcome {
            status: Some(WorkItemStatus::Completed),
            source_status: Some(SourceStatus::Completed),
            observations,
            findings,
            ..Default::default()
        })
    };
    run_plan(connection, started.run_id, "g1", &mut executor).expect("本轮必须跑完");
    let _ = super::orchestrator::finalize(connection, started.run_id, None);
    started.run_id
}

fn finding_id_of(connection: &Connection, project_id: i64, url: &str) -> i64 {
    connection
        .query_row(
            "SELECT id FROM exposure_findings WHERE project_id=?1 AND source_url=?2",
            params![project_id, url],
            |row| row.get(0),
        )
        .unwrap()
}

fn page_of(connection: &Connection, project_id: i64, disposition: &str) -> Result<super::super::ExposureFindingPageView, String> {
    let query = super::contract::ExposureFindingPageInput {
        project_id,
        page_size: 50,
        disposition: disposition.to_string(),
        ..Default::default()
    }
    .normalize()
    .map_err(|error| format!("[{}] {}", error.code, error.message))?;
    super::super::finding_page_view(connection, &query)
}

#[test]
fn g1_3_excluded_findings_stay_listed_but_leave_high_risk_and_remediation() {
    let root = sandbox("classify");
    let (connection, _) = database(&root);
    let origin = "http://127.0.0.1:9";
    seed_scope(&connection, 431, origin, true);
    let notice_url = format!("{origin}/notice/2026-0815.html");
    let config_url = format!("{origin}/config.json");
    let config_body = "{\"db_password\": \"Zx9!kq2Lm8#Pw4Rt\"}".to_string();
    let mut drafts = super::rules::findings_for(&config_url, "application/json", config_body.as_bytes(), None);
    assert!(drafts.iter().any(|row| row.category == "credential" && row.severity == "high"), "{drafts:?}");
    drafts.push(draft("document", "公开路径疑似内部文档", &notice_url, "high", 60));
    // 一条没有页面正文的索引线索（走合成观察）：只有文件名含招采字样 → 待分类，不排除。
    let pdf_url = format!("{origin}/files/%E6%8B%9B%E6%A0%87%E6%96%87%E4%BB%B6.pdf");
    drafts.push(draft("document", "公开路径疑似内部文档", &pdf_url, "medium", 50));
    let pages = vec![
        (notice_url.clone(), "text/html; charset=utf-8", NOTICE.to_string()),
        (config_url.clone(), "application/json", config_body.clone()),
    ];
    let first_run = round_with(&connection, 431, origin, &pages, drafts.clone());
    let notice_id = finding_id_of(&connection, 431, &notice_url);
    let config_id = finding_id_of(&connection, 431, &config_url);
    let pdf_id = finding_id_of(&connection, 431, &pdf_url);

    let summary = exposure_scope_summary(&connection, 431).unwrap();
    assert_eq!(summary.findings_total, 3);
    assert_eq!((summary.high_risk_open, summary.findings_excluded), (1, 1), "公开招采不能算高风险待办");
    assert_eq!((summary.findings_classification_pending, summary.findings_unclassified), (1, 0), "索引线索也必须分类");
    assert_eq!(summary.classify_policy_version, super::classify::CLASSIFY_POLICY_VERSION);

    // 列表仍可见，并带处置旁表；筛选按处置。
    let all = page_of(&connection, 431, "").unwrap();
    assert_eq!(all.page.total, 3, "排除的线索仍在列表里可查");
    assert_eq!(all.dispositions[&pdf_id].disposition, "classification_pending");
    assert!(all.dispositions[&pdf_id].reason.contains("文件名"), "{:?}", all.dispositions[&pdf_id]);
    assert_eq!(all.dispositions[&notice_id].disposition, "excluded_public_procurement");
    assert_eq!(all.dispositions[&config_id].disposition, "risk_candidate");
    assert_eq!(all.dispositions[&config_id].info_category, "credential");
    let excluded = page_of(&connection, 431, "excluded").unwrap();
    assert_eq!(excluded.page.items.iter().map(|row| row.id).collect::<Vec<_>>(), vec![notice_id]);
    let open = page_of(&connection, 431, "not_excluded").unwrap();
    assert_eq!(open.page.items.iter().map(|row| row.id).collect::<Vec<_>>(), vec![config_id, pdf_id]);
    let refused = page_of(&connection, 431, "excludd").err().expect("未知处置筛选必须被拒绝");
    assert!(refused.contains("exposure_disposition_invalid"), "{refused}");
    let serialized = serde_json::to_value(&all).unwrap();
    assert!(serialized["items"].is_array() && serialized["total"] == 3, "分页字段保持平铺，旧读取方不受影响");
    assert_eq!(serialized["dispositions"][notice_id.to_string()]["disposition"], "excluded_public_procurement");

    // 证据页：当前处置 + 每次观察的分类记录。
    let evidence = super::repository::exposure_evidence(&connection, notice_id, 431).unwrap();
    assert_eq!(evidence["disposition"]["disposition"], "excluded_public_procurement");
    assert_eq!(evidence["classifications"].as_array().map(Vec::len), Some(1));
    assert_eq!(evidence["classifications"][0]["runId"], first_run);
    assert!(evidence["classifications"][0]["markers"].as_array().unwrap().iter().any(|m| m == "title:招标公告"));
    assert!(evidence["classificationError"].is_null());

    // 整改：排除的线索被拒绝，风险候选可以登记。
    let refusal = super::remediation::record_remediation(&connection, 431, notice_id, None, first_run, "张三", "整改", "open").unwrap_err();
    assert!(refusal.contains("exposure_remediation_excluded"), "{refusal}");
    super::remediation::record_remediation(&connection, 431, config_id, None, first_run, "张三", "轮换口令", "open").expect("风险候选可以登记整改");
    let taxonomy = super::remediation::taxonomy(&connection, 431).unwrap();
    assert_eq!(taxonomy.iter().find(|row| row.code == "auto_excluded").map(|row| row.count), Some(1));

    // 第二轮：同一地址的页面变成普通页面 → 排除不自动撤销，只置冲突标记，新分类另起一行。
    let plain = "<html><head><title>关于我们</title></head><body>公司简介</body></html>".to_string();
    let pages = vec![(notice_url.clone(), "text/html", plain), (config_url.clone(), "application/json", config_body)];
    round_with(&connection, 431, origin, &pages, drafts);
    let view = super::classify::disposition_of(&connection, 431, notice_id).unwrap().unwrap();
    assert_eq!(view.disposition, "excluded_public_procurement", "排除是粘性的");
    assert!(view.conflict && view.conflict_note.contains("不自动撤销"), "{view:?}");
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_classifications WHERE finding_id=?1", [notice_id], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 2, "每次观察各留一行分类，旧行不改写");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_3_a_human_confirmation_is_never_overridden_and_old_findings_stay_readable() {
    use super::classify::{record, Classification, DISPOSITION_EXCLUDED_PROCUREMENT};
    let root = sandbox("human");
    let (connection, _) = database(&root);
    project(&connection, 432);
    connection
        .execute(
            "INSERT INTO exposure_findings(id,project_id,category,title,source_url,severity,confidence,fingerprint,status)
             VALUES(43201,432,'document','人工确认的文档','https://x.example.com.cn/a','high',60,'fp-a','confirmed'),
                   (43202,432,'document','G1 之前的旧线索','https://x.example.com.cn/b','high',60,'fp-b','new')",
            [],
        )
        .unwrap();
    let excluded = Classification {
        info_category: "internal_business",
        source_channel: "first_party_web",
        disposition: DISPOSITION_EXCLUDED_PROCUREMENT,
        reason: "公开招采公告".to_string(),
        markers: vec!["title:招标公告".to_string()],
    };
    let effective = record(&connection, 432, 43201, None, 0, &excluded).unwrap();
    assert_eq!(effective, "classification_pending", "人工确认的暴露不能被自动排除");
    let view = super::classify::disposition_of(&connection, 432, 43201).unwrap().unwrap();
    assert!(view.conflict && view.conflict_note.contains("人工"), "{view:?}");
    // 人工确认过的线索仍可登记整改。
    super::remediation::record_remediation(&connection, 432, 43201, None, 0, "李四", "已下线", "open").expect("人工确认优先");

    // 旧线索：没有分类行，列表照常可读，汇总单列“未分类”，筛选可单独取出。
    let summary = exposure_scope_summary(&connection, 432).unwrap();
    assert_eq!((summary.findings_unclassified, summary.findings_classification_pending), (1, 1));
    assert_eq!(summary.high_risk_open, 1, "旧线索仍按原规则计入待办");
    let legacy = page_of(&connection, 432, "unclassified").unwrap();
    assert_eq!(legacy.page.items.iter().map(|row| row.id).collect::<Vec<_>>(), vec![43202]);
    assert!(!legacy.dispositions.contains_key(&43202));
    // 更老的库：分类表还不存在时，未筛选的列表与旁表照样可读（不会报“表不存在”）。
    connection.execute_batch("DROP TABLE exposure_finding_dispositions; DROP TABLE exposure_classifications;").unwrap();
    let older = page_of(&connection, 432, "").unwrap();
    assert_eq!((older.page.total, older.dispositions.len()), (2, 0));
    let evidence = super::repository::exposure_evidence(&connection, 43202, 432).unwrap();
    assert!(evidence["disposition"].is_null() && evidence["classifications"].as_array().unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── G1-4 ─────────────────────────────
// The closed loop on a real (localhost) authorized site, driven through the production worker
// `execute_exposure_run`: site → observation → finding → review → remediation → a new round
// as retest → close only on a basis the retest produced.

const VENDOR_MAP: &str = r#"{"version":3,"sources":["webpack:///node_modules/core-js/modules/es.array.js","webpack:///node_modules/react-dom/cjs/react-dom.production.min.js"],"mappings":"AAAA"}"#;

fn html(body: &str) -> String {
    response(200, &[("Content-Type", "text/html; charset=utf-8")], body)
}

fn json_reply(body: &str) -> String {
    response(200, &[("Content-Type", "application/json")], body)
}

fn notice_page() -> String {
    // A real procurement notice that happens to carry a template credential line.
    html(&format!("{NOTICE}\n<p>api_key: \"your_api_key_here\"</p>"))
}

fn run_worker(db_path: &Path, run_id: i64, project_id: i64, settings: &JsonValue) {
    super::super::execute_exposure_run(db_path.to_path_buf(), run_id, project_id, "g1-4-worker".into(), settings.clone());
}

fn observation_of(connection: &Connection, run_id: i64, url: &str) -> Option<(i64, String, i64)> {
    connection
        .query_row(
            "SELECT id,content_hash,status_code FROM exposure_observations WHERE run_id=?1 AND url=?2 ORDER BY id DESC LIMIT 1",
            params![run_id, url],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok()
}

#[test]
fn g1_4_the_closed_loop_runs_on_a_real_authorized_site() {
    use super::remediation::{mark_remediation, record_remediation, retest_outcome, review_scoped, start_retest, top_revision};
    let leaked = r#"{"db_password": "Zx9kq2Lm8Pw4Rt"}"#;
    let fixture = Fixture::start(routes(vec![
        ("/", vec![html(r#"<html><title>首页</title><a href="/notice/2026-0815.html">公告</a><a href="/config.json">配置</a><script src="/static/vendor.js.map"></script></html>"#)]),
        ("/notice/2026-0815.html", vec![notice_page()]),
        ("/static/vendor.js.map", vec![json_reply(VENDOR_MAP)]),
        // round 1 leaks; retest 1 still serves the same bytes; retest 2 the file is gone.
        ("/config.json", vec![json_reply(leaked), json_reply(leaked), response(404, &[], "not found")]),
    ]));
    let root = sandbox("loop");
    let (connection, db_path) = database(&root);
    let project_id = 441;
    seed_scope(&connection, project_id, &fixture.origin, true);
    // 单位：项目绑定到专业公司下的地市分公司（名字像浙江）。
    directory(&connection);
    bind(&connection, project_id, "unit", 1, Some(4));
    let settings = passive_settings(&connection, project_id);

    // ① 授权站点 → 观察 → 线索。
    let first = start_or_reuse(&connection, project_id, &settings).expect("第一轮必须能开始");
    run_worker(&db_path, first.run_id, project_id, &settings);
    assert_eq!(run_row(&connection, first.run_id).0, "completed", "{:?}", run_row(&connection, first.run_id));
    let config_url = fixture.url("/config.json");
    let notice_url = fixture.url("/notice/2026-0815.html");
    let map_url = fixture.url("/static/vendor.js.map");
    let (first_obs, first_hash, _) = observation_of(&connection, first.run_id, &config_url).expect("配置文件必须被真实取回");
    let config_id = finding_id_of(&connection, project_id, &config_url);
    let notice_id = finding_id_of(&connection, project_id, &notice_url);
    let map_id = finding_id_of(&connection, project_id, &map_url);
    let page = page_of(&connection, project_id, "").unwrap();
    assert_eq!(page.dispositions[&config_id].disposition, "risk_candidate");
    assert_eq!(page.dispositions[&config_id].info_category, "credential");
    assert_eq!(page.dispositions[&notice_id].disposition, "excluded_public_procurement", "{:?}", page.dispositions[&notice_id]);
    assert_eq!(page.dispositions[&map_id].disposition, "excluded_public_material");
    let summary = exposure_scope_summary(&connection, project_id).unwrap();
    assert_eq!((summary.high_risk_open, summary.findings_excluded), (1, 2), "只有真实凭据算高风险待办");
    let units = summary.unit_projection.expect("单位投影");
    assert_eq!((units.unit_bucket.as_str(), units.unit_province.is_none()), ("professional_subsidiary", true), "专业公司不误分省");
    assert_eq!((units.counts.confirmed_unit, units.counts.executed, units.counts.uncovered), (Some(1), 1, 0));

    // ② 复核：引用真实观察确认。
    review_scoped(&connection, project_id, config_id, top_revision(&connection, config_id).unwrap(), "confirmed", "张三", "配置文件公开了数据库口令", &[first_obs]).expect("复核写入");
    // ③ 整改：排除的招采公告不能挂整改；真实凭据可以。
    let refused = record_remediation(&connection, project_id, notice_id, None, first.run_id, "张三", "整改", "open").unwrap_err();
    assert!(refused.contains("exposure_remediation_excluded"), "{refused}");
    let remediation = record_remediation(&connection, project_id, config_id, Some(first_obs), first.run_id, "张三", "已轮换口令并删除配置文件", "fixing").expect("整改登记");
    assert_eq!(retest_outcome(&connection, project_id, remediation).unwrap()["basis"], "no_retest_round");
    assert!(mark_remediation(&connection, project_id, remediation, "closed", "张三", "应该好了").unwrap_err().contains("exposure_remediation_unclosed"));

    // ④ 复测 1：新一轮仍取回同样字节 → 不能关闭。
    let retest1 = start_retest(&connection, project_id, remediation, &settings).expect("复测 1");
    assert_ne!(retest1.run_id, first.run_id, "复测必须是新的一轮");
    run_worker(&db_path, retest1.run_id, project_id, &settings);
    let outcome = retest_outcome(&connection, project_id, remediation).unwrap();
    assert_eq!((outcome["basis"].as_str(), outcome["canClose"].as_bool()), (Some("retest_content_identical"), Some(false)), "{outcome}");
    assert!(mark_remediation(&connection, project_id, remediation, "closed", "张三", "复测过了").unwrap_err().contains("retest_content_identical"));

    // ⑤ 复测 2：文件已下线（404，新字节）→ 有依据，可以关闭；线索回到复核而不是被宣布“已修复”。
    let retest2 = start_retest(&connection, project_id, remediation, &settings).expect("复测 2");
    run_worker(&db_path, retest2.run_id, project_id, &settings);
    let (_, gone_hash, gone_status) = observation_of(&connection, retest2.run_id, &config_url).expect("复测 2 的新观察");
    assert_eq!(gone_status, 404);
    assert_ne!(gone_hash, first_hash);
    let outcome = retest_outcome(&connection, project_id, remediation).unwrap();
    assert_eq!((outcome["basis"].as_str(), outcome["canClose"].as_bool()), (Some("retest_content_changed"), Some(true)), "{outcome}");
    mark_remediation(&connection, project_id, remediation, "closed", "张三", "复测 2 取回 404，新字节").expect("按依据关闭");
    let status: String = connection.query_row("SELECT status FROM exposure_findings WHERE id=?1", [config_id], |row| row.get(0)).unwrap();
    assert_eq!(status, "reviewing", "关闭整改后线索回到复核，按新证据重新定性");

    // ⑥ 旧历史可读：第一轮观察一个字节都没改；三轮的分类各有记录；排除在复测轮里保持。
    let (_, still, _) = observation_of(&connection, first.run_id, &config_url).unwrap();
    assert_eq!(still, first_hash);
    let evidence = super::repository::exposure_evidence(&connection, config_id, project_id).unwrap();
    assert!(evidence["observations"].as_array().unwrap().len() >= 2, "{}", evidence["observations"]);
    assert!(evidence["classifications"].as_array().unwrap().len() >= 2);
    assert_eq!(super::classify::disposition_of(&connection, project_id, notice_id).unwrap().unwrap().disposition, "excluded_public_procurement");
    connection
        .execute(
            "INSERT INTO exposure_findings(project_id,category,title,source_url,severity,confidence,fingerprint,status)
             VALUES(?1,'document','G1 之前的旧线索','http://legacy.invalid/a','high',60,'fp-legacy-441','new')",
            [project_id],
        )
        .unwrap();
    let legacy_id = connection.last_insert_rowid();
    let page = page_of(&connection, project_id, "unclassified").unwrap();
    assert_eq!(page.page.items.iter().map(|row| row.id).collect::<Vec<_>>(), vec![legacy_id]);
    assert!(super::repository::exposure_evidence(&connection, legacy_id, project_id).unwrap()["disposition"].is_null());
    let summary = exposure_scope_summary(&connection, project_id).unwrap();
    assert_eq!(summary.findings_unclassified, 1);
    // 每一轮都只访问授权站点上的真实路径。
    assert!(fixture.requests().iter().all(|path| path.starts_with('/')));
    assert!(fixture.requests().iter().filter(|path| path.as_str() == "/config.json").count() >= 3);
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_4_a_retest_against_a_site_that_went_down_cannot_close() {
    use super::remediation::{mark_remediation, record_remediation, retest_outcome, start_retest};
    let fixture = Fixture::start(routes(vec![
        ("/", vec![html(r#"<a href="/config.json">配置</a>"#)]),
        ("/config.json", vec![json_reply(r#"{"secret_key": "Qw8eRt5yUi3oPa1s"}"#)]),
    ]));
    let root = sandbox("down");
    let (connection, db_path) = database(&root);
    let project_id = 442;
    seed_scope(&connection, project_id, &fixture.origin, true);
    let settings = passive_settings(&connection, project_id);
    let first = start_or_reuse(&connection, project_id, &settings).unwrap();
    run_worker(&db_path, first.run_id, project_id, &settings);
    let config_url = fixture.url("/config.json");
    let (obs, _, _) = observation_of(&connection, first.run_id, &config_url).expect("第一轮取回配置");
    let finding = finding_id_of(&connection, project_id, &config_url);
    let remediation = record_remediation(&connection, project_id, finding, Some(obs), first.run_id, "李四", "已删除", "fixing").unwrap();
    drop(fixture); // 站点下线：端口不再监听。
    let retest = start_retest(&connection, project_id, remediation, &settings).expect("复测仍按授权启动");
    run_worker(&db_path, retest.run_id, project_id, &settings);
    let outcome = retest_outcome(&connection, project_id, remediation).unwrap();
    assert_eq!(outcome["canClose"], false, "{outcome}");
    assert!(
        matches!(outcome["basis"].as_str(), Some("retest_not_seen" | "retest_unreachable" | "retest_round_incomplete")),
        "取不到不能当成已下线：{outcome}"
    );
    let refused = mark_remediation(&connection, project_id, remediation, "closed", "李四", "站点打不开了，应该修好了").unwrap_err();
    assert!(refused.contains("exposure_remediation_unclosed"), "{refused}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_4_a_retest_that_cannot_fetch_the_url_is_not_a_content_change() {
    use super::remediation::{mark_remediation, record_remediation, retest_outcome, start_retest};
    // 站点仍在线，但复测时这一个地址连接被直接断开（空响应）：这是“取不到”，不是“字节变了”。
    let fixture = Fixture::start(routes(vec![
        ("/", vec![html(r#"<a href="/config.json">配置</a>"#)]),
        ("/config.json", vec![json_reply(r#"{"secret_key": "Qw8eRt5yUi3oPa1s"}"#), String::new()]),
    ]));
    let root = sandbox("unreachable");
    let (connection, db_path) = database(&root);
    let project_id = 443;
    seed_scope(&connection, project_id, &fixture.origin, true);
    let settings = passive_settings(&connection, project_id);
    let first = start_or_reuse(&connection, project_id, &settings).unwrap();
    run_worker(&db_path, first.run_id, project_id, &settings);
    let config_url = fixture.url("/config.json");
    let (obs, _, _) = observation_of(&connection, first.run_id, &config_url).expect("第一轮取回配置");
    let finding = finding_id_of(&connection, project_id, &config_url);
    let remediation = record_remediation(&connection, project_id, finding, Some(obs), first.run_id, "王五", "已删除", "fixing").unwrap();
    let retest = start_retest(&connection, project_id, remediation, &settings).unwrap();
    run_worker(&db_path, retest.run_id, project_id, &settings);
    let (_, _, status_code) = observation_of(&connection, retest.run_id, &config_url).expect("复测轮必须留下这条地址的传输失败观察");
    assert_eq!(status_code, 0, "传输失败的观察没有 HTTP 状态");
    let outcome = retest_outcome(&connection, project_id, remediation).unwrap();
    assert_eq!((outcome["basis"].as_str(), outcome["canClose"].as_bool()), (Some("retest_unreachable"), Some(false)), "{outcome}");
    assert!(mark_remediation(&connection, project_id, remediation, "closed", "王五", "复测哈希变了").unwrap_err().contains("exposure_remediation_unclosed"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn g1_3_reclassify_is_cas_audited_and_then_remediation_is_allowed() {
    let root = sandbox("reclass");
    let (connection, _) = database(&root);
    project(&connection, 433);
    connection.execute("INSERT INTO projects(id,name,status) VALUES(434,'other','active')", []).unwrap();
    connection
        .execute(
            "INSERT INTO exposure_findings(id,project_id,category,title,source_url,severity,confidence,fingerprint,status)
             VALUES(43301,433,'document','招标公告','https://x.example.com.cn/n','medium',40,'fp-n','new')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_classifications(project_id,finding_id,observation_id,run_id,policy_version,info_category,source_channel,disposition,reason,markers_json,created_at)
             VALUES(433,43301,NULL,1,'oviraptor.exposure.classify.v1','internal_business','first_party_web','excluded_public_procurement','公开招采','[]',datetime('now'))",
            [],
        )
        .unwrap();
    let class_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO exposure_finding_dispositions(finding_id,project_id,disposition,info_category,source_channel,reason,policy_version,classification_id,conflict,conflict_note,updated_at)
             VALUES(43301,433,'excluded_public_procurement','internal_business','first_party_web','公开招采','oviraptor.exposure.classify.v1',?1,0,'',datetime('now'))",
            [class_id],
        )
        .unwrap();
    let before: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_classifications", [], |row| row.get(0)).unwrap();
    let body = serde_json::json!({
        "operationId": "op-12345678",
        "findingId": 43301,
        "expectedClassificationId": class_id,
        "disposition": "risk_candidate",
        "infoCategory": "credential",
        "actor": "张三",
        "reason": "公告里有口令形态，改回风险",
    });
    let wrong = super::classify::reclassify(&connection, 434, &body).unwrap_err();
    assert!(wrong.contains("exposure_reclassify_not_found"), "{wrong}");
    let after_foreign: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_classifications", [], |row| row.get(0)).unwrap();
    assert_eq!(after_foreign, before, "跨项目零写");
    let mut stale = body.clone();
    stale["expectedClassificationId"] = serde_json::json!(class_id + 9);
    let conflict = super::classify::reclassify(&connection, 433, &stale).unwrap_err();
    assert!(conflict.contains("exposure_reclassify_conflict"), "{conflict}");
    let after_conflict: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_classifications", [], |row| row.get(0)).unwrap();
    assert_eq!(after_conflict, before, "CAS 不符零写");
    for (key, value, code) in [
        ("reason", serde_json::json!("短"), "改判必须写明理由"),
        ("reason", serde_json::json!(""), "改判必须写明理由"),
        ("operationId", serde_json::json!("x"), "operationId"),
        ("disposition", serde_json::json!("maybe"), "处置只能是"),
        ("actor", serde_json::json!(""), "操作人必填"),
    ] {
        let mut bad = body.clone();
        bad[key] = value;
        let refused = super::classify::reclassify(&connection, 433, &bad).unwrap_err();
        assert!(refused.contains("exposure_reclassify_invalid") && refused.contains(code), "{key}: {refused}");
    }
    let after_invalid: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_classifications", [], |row| row.get(0)).unwrap();
    assert_eq!(after_invalid, before, "非法请求零写");
    let done = super::classify::reclassify(&connection, 433, &body).unwrap();
    assert_eq!(done["disposition"], "risk_candidate");
    assert_eq!(done["replayed"], false);
    assert!(done["policyVersion"].as_str().unwrap().starts_with("manual:"));
    let again = super::classify::reclassify(&connection, 433, &body).unwrap();
    assert_eq!(again["replayed"], true);
    assert_eq!(again["classificationId"], done["classificationId"]);
    let mut other = body.clone();
    other["reason"] = serde_json::json!("另一份理由不能复用同一操作编号");
    let clash = super::classify::reclassify(&connection, 433, &other).unwrap_err();
    assert!(clash.contains("exposure_reclassify_operation_conflict"), "{clash}");
    super::remediation::record_remediation(&connection, 433, 43301, None, 0, "张三", "已改判后登记整改", "open").expect("改判为风险后可以登记整改");
    let audit: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_reclassifications WHERE project_id=433", [], |row| row.get(0)).unwrap();
    assert_eq!(audit, 1);
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── P1 修复回归（2026-10-09） ─────────────────────────────

/// A finding with one observation of its own, written the way the collector leaves it.
fn rem_finding(connection: &Connection, project_id: i64, run_id: i64, url: &str, status_code: i64, body: &str) -> (i64, i64) {
    connection
        .execute(
            "INSERT INTO exposure_observations(run_id,project_id,source_key,kind,url,canonical_url,status_code,content_hash,body_bytes,truncated,evidence_json)
             VALUES(?1,?2,'passive_web','page',?3,?3,?4,?5,?6,0,'{}')",
            params![run_id, project_id, url, status_code, format!("sha-{body}"), body.len() as i64],
        )
        .unwrap();
    let observation = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT OR IGNORE INTO exposure_findings(project_id,category,title,source_type,source_url,severity,confidence,status,fingerprint)
             VALUES(?1,'secret','疑似凭据','passive_web',?2,'high',80,'new',?3)",
            params![project_id, url, format!("rem|{url}")],
        )
        .unwrap();
    let finding: i64 = connection
        .query_row("SELECT id FROM exposure_findings WHERE project_id=?1 AND fingerprint=?2", params![project_id, format!("rem|{url}")], |row| row.get(0))
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_finding_observations(finding_id,observation_id,run_id,project_id) VALUES(?1,?2,?3,?4)",
            params![finding, observation, run_id, project_id],
        )
        .unwrap();
    (finding, observation)
}

fn rem_run(connection: &Connection, project_id: i64, status: &str) -> i64 {
    project(connection, project_id);
    connection
        .execute("INSERT INTO exposure_runs(project_id,status,stage) VALUES(?1,?2,?2)", params![project_id, status])
        .unwrap();
    connection.last_insert_rowid()
}

fn rem_count(connection: &Connection, table: &str) -> i64 {
    connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap()
}

/// P1-1：整改只能绑定这条线索自己的观察与本项目的轮次，挂错对象零写入。
#[test]
fn rem_p1_1_a_remediation_binds_only_its_own_finding_observation_and_project_run() {
    use super::remediation::record_remediation;
    let root = sandbox("rem-target");
    let (connection, _db) = database(&root);
    let pid = 481;
    let r1 = rem_run(&connection, pid, "completed");
    let (mine, my_obs) = rem_finding(&connection, pid, r1, "https://a.example.test/config.json", 200, "A");
    let (_other, other_obs) = rem_finding(&connection, pid, r1, "https://b.example.test/config.json", 200, "B");
    let foreign_run = rem_run(&connection, 482, "completed");
    let before = rem_count(&connection, "exposure_remediations");
    let wrong_observation = record_remediation(&connection, pid, mine, Some(other_obs), r1, "张三", "已删除配置", "fixing").unwrap_err();
    assert!(wrong_observation.contains("exposure_remediation_evidence_missing"), "别的线索的观察不能挂到这条线索上：{wrong_observation}");
    let wrong_run = record_remediation(&connection, pid, mine, Some(my_obs), foreign_run, "张三", "已删除配置", "fixing").unwrap_err();
    assert!(wrong_run.contains("exposure_remediation_run_invalid"), "别的项目的轮次不能作为整改来源：{wrong_run}");
    assert_eq!(rem_count(&connection, "exposure_remediations"), before, "挂错对象零写入");
    let id = record_remediation(&connection, pid, mine, Some(my_obs), r1, "张三", "已删除配置", "fixing").expect("正确对象可以登记");
    assert!(id > 0);
    let _ = std::fs::remove_dir_all(root);
}

/// P1-4：复测轮失败/进行中，或复测看到的是 5xx/403，都不能作为关闭依据；404 仍可。
#[test]
fn rem_p1_4_a_failed_or_refused_retest_never_lets_a_remediation_close() {
    use super::remediation::{mark_remediation, record_remediation, retest_outcome};
    let root = sandbox("rem-retest");
    let (connection, _db) = database(&root);
    let pid = 483;
    let url = "https://a.example.test/config.json";
    let r1 = rem_run(&connection, pid, "completed");
    let (finding, observation) = rem_finding(&connection, pid, r1, url, 200, "LEAK");
    let remediation = record_remediation(&connection, pid, finding, Some(observation), r1, "张三", "已轮换口令", "fixing").unwrap();
    let link = |run_id: i64| {
        connection
            .execute(
                "INSERT INTO exposure_remediation_retests(remediation_id,project_id,run_id,finding_id,observation_url) VALUES(?1,?2,?3,?4,?5)",
                params![remediation, pid, run_id, finding, url],
            )
            .unwrap();
    };
    for (status, code, body) in [("failed", 200, "HALF"), ("running", 200, "EARLY"), ("completed", 500, "OOPS"), ("completed", 403, "DENY"), ("completed", 429, "SLOW")] {
        let retest = rem_run(&connection, pid, status);
        rem_finding(&connection, pid, retest, url, code, body);
        link(retest);
        let outcome = retest_outcome(&connection, pid, remediation).unwrap();
        assert_eq!(outcome["canClose"], false, "复测轮 {status} / HTTP {code} 不能作为关闭依据：{outcome}");
        let refused = mark_remediation(&connection, pid, remediation, "closed", "张三", "复测过了").unwrap_err();
        assert!(refused.contains("exposure_remediation_unclosed"), "{refused}");
        let state: String = connection.query_row("SELECT status FROM exposure_remediations WHERE id=?1", [remediation], |row| row.get(0)).unwrap();
        assert_eq!(state, "fixing", "拒绝关闭零写入");
    }
    let gone = rem_run(&connection, pid, "completed");
    rem_finding(&connection, pid, gone, url, 404, "NOTFOUND");
    link(gone);
    let outcome = retest_outcome(&connection, pid, remediation).unwrap();
    assert_eq!((outcome["basis"].as_str(), outcome["canClose"].as_bool()), (Some("retest_content_changed"), Some(true)), "{outcome}");
    let _ = std::fs::remove_dir_all(root);
}

/// P1-2：已经关闭/驳回的整改不能被旧页面再写回；重复登记同一证据也不能把它重新打开。
#[test]
fn rem_p1_2_a_terminal_remediation_is_never_overwritten() {
    use super::remediation::{mark_remediation, record_remediation};
    let root = sandbox("rem-terminal");
    let (connection, _db) = database(&root);
    let pid = 484;
    let r1 = rem_run(&connection, pid, "completed");
    let (finding, observation) = rem_finding(&connection, pid, r1, "https://a.example.test/x.sql", 200, "DUMP");
    let remediation = record_remediation(&connection, pid, finding, Some(observation), r1, "张三", "已下线", "fixing").unwrap();
    mark_remediation(&connection, pid, remediation, "rejected", "张三", "不是我方资产").unwrap();
    let row = |connection: &Connection| -> (String, String, String) {
        connection
            .query_row("SELECT status,note,actor FROM exposure_remediations WHERE id=?1", [remediation], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
    };
    let frozen = row(&connection);
    let again = mark_remediation(&connection, pid, remediation, "fixing", "李四", "旧页面上的按钮").unwrap_err();
    assert!(again.contains("exposure_remediation_terminal"), "{again}");
    let reopened = record_remediation(&connection, pid, finding, Some(observation), r1, "李四", "旧页面重新登记", "open").unwrap_err();
    assert!(reopened.contains("exposure_remediation_terminal"), "{reopened}");
    assert_eq!(row(&connection), frozen, "终态整改零写入");
    let _ = std::fs::remove_dir_all(root);
}

/// P1-2：复核的版本核对必须和写入在同一个写事务里。另一条连接在核对之后、写入之前推进了
/// 线索版本，这次复核必须被拒绝，而不是把结论写在旧版本上。
#[test]
fn rem_p1_2_a_review_checked_against_an_old_revision_writes_nothing() {
    use super::remediation::{review_scoped, top_revision};
    let root = sandbox("rem-review");
    let (connection, db_path) = database(&root);
    let pid = 485;
    let r1 = rem_run(&connection, pid, "completed");
    let (finding, observation) = rem_finding(&connection, pid, r1, "https://a.example.test/y.sql", 200, "Y");
    connection
        .execute("INSERT INTO exposure_finding_versions(finding_id,run_id,revision,content_hash) VALUES(?1,?2,1,'h1')", params![finding, r1])
        .unwrap();
    let read = top_revision(&connection, finding).unwrap();
    // 采集写入者先拿到写锁，准备写下 r2。
    let writer = crate::db::open(&db_path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer
        .execute("INSERT INTO exposure_finding_versions(finding_id,run_id,revision,content_hash) VALUES(?1,?2,2,'h2')", params![finding, r1])
        .unwrap();
    let reviewer_db = db_path.clone();
    let reviewer = std::thread::spawn(move || {
        let mine = crate::db::open(&reviewer_db).unwrap();
        review_scoped(&mine, pid, finding, read, "confirmed", "张三", "按 r1 的证据确认", &[observation])
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    writer.execute_batch("COMMIT").unwrap();
    let result = reviewer.join().unwrap();
    assert!(result.as_ref().is_err_and(|error| error.contains("exposure_review_conflict")), "按旧版本核对过的复核不能写入：{result:?}");
    let status: String = connection.query_row("SELECT status FROM exposure_findings WHERE id=?1", [finding], |row| row.get(0)).unwrap();
    assert_eq!(status, "new", "旧版本零写");
    assert_eq!(rem_count(&connection, "exposure_review_events"), 0);
    let _ = std::fs::remove_dir_all(root);
}

/// P1-3 / T30：撤销了被复测资产的授权后，复测直接拒绝：不建新轮、不发任何请求。
#[test]
fn rem_p1_3_t30_a_retest_of_a_revoked_asset_starts_nothing_and_sends_nothing() {
    use super::remediation::{record_remediation, start_retest};
    let leaked = r#"{"db_password": "Rv8kq2Lm8Pw4Tt"}"#;
    let a = Fixture::start(routes(vec![("/", vec![html(r#"<a href="/config.json">配置</a>"#)]), ("/config.json", vec![json_reply(leaked)])]));
    let b = Fixture::start(routes(vec![("/", vec![html("<title>B</title>")])]));
    let root = sandbox("rem-t30");
    let (connection, db_path) = database(&root);
    let pid = 486;
    seed_scope(&connection, pid, &a.origin, true);
    seed_scope(&connection, pid, &b.origin, true);
    let settings = passive_settings(&connection, pid);
    let first = start_or_reuse(&connection, pid, &settings).unwrap();
    run_worker(&db_path, first.run_id, pid, &settings);
    let config_url = a.url("/config.json");
    let (obs, _, _) = observation_of(&connection, first.run_id, &config_url).expect("第一轮取回配置");
    let finding = finding_id_of(&connection, pid, &config_url);
    let remediation = record_remediation(&connection, pid, finding, Some(obs), first.run_id, "张三", "已删除配置", "fixing").unwrap();
    // 撤销 A 的授权（B 仍有授权）。
    let asset_a: i64 = connection.query_row("SELECT id FROM assets WHERE asset_key=?1", [&a.origin], |row| row.get(0)).unwrap();
    connection
        .execute("UPDATE asset_ownership_decisions SET exposure_eligible=0,authorization_status='revoked' WHERE project_id=?1 AND asset_id=?2", params![pid, asset_a])
        .unwrap();
    let (runs, sent_a, sent_b) = (rem_count(&connection, "exposure_runs"), a.request_count(), b.request_count());
    let refused = start_retest(&connection, pid, remediation, &settings).unwrap_err();
    assert!(refused.contains("exposure_retest_unauthorized"), "{refused}");
    assert_eq!(rem_count(&connection, "exposure_runs"), runs, "撤权后复测不建新轮");
    assert_eq!(rem_count(&connection, "exposure_remediation_retests"), 0);
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!((a.request_count(), b.request_count()), (sent_a, sent_b), "撤权后零新请求");
    let _ = std::fs::remove_dir_all(root);
}

/// P1-5：改判理由里贴进来的口令只以脱敏形态落库。
#[test]
fn rem_p1_5_a_reclassify_reason_never_stores_a_pasted_secret() {
    let root = sandbox("rem-reclass-secret");
    let (connection, _) = database(&root);
    project(&connection, 487);
    connection
        .execute(
            "INSERT INTO exposure_findings(id,project_id,category,title,source_url,severity,confidence,fingerprint,status)
             VALUES(48701,487,'document','招标公告','https://x.example.com.cn/n','medium',40,'fp-rem','new')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_classifications(project_id,finding_id,observation_id,run_id,policy_version,info_category,source_channel,disposition,reason,markers_json,created_at)
             VALUES(487,48701,NULL,1,'oviraptor.exposure.classify.v1','internal_business','first_party_web','excluded_public_procurement','公开招采','[]',datetime('now'))",
            [],
        )
        .unwrap();
    let class_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO exposure_finding_dispositions(finding_id,project_id,disposition,info_category,source_channel,reason,policy_version,classification_id,conflict,conflict_note,updated_at)
             VALUES(48701,487,'excluded_public_procurement','internal_business','first_party_web','公开招采','oviraptor.exposure.classify.v1',?1,0,'',datetime('now'))",
            [class_id],
        )
        .unwrap();
    let secret = "Rc5Hunter2SecretValue55";
    let body = serde_json::json!({
        "operationId": "op-rem-reclass-1",
        "findingId": 48701,
        "expectedClassificationId": class_id,
        "disposition": "risk_candidate",
        "infoCategory": "credential",
        "actor": "张三",
        "reason": format!("公告附件里有 password={secret}，改回风险"),
    });
    super::classify::reclassify(&connection, 487, &body).expect("改判成功");
    let replay = super::classify::reclassify(&connection, 487, &body).expect("同一载荷重放");
    assert_eq!(replay["replayed"], true, "{replay}");
    let mut stored = String::new();
    for table in ["exposure_classifications", "exposure_reclassifications", "exposure_finding_dispositions", "exposure_findings"] {
        let mut statement = connection.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let columns = statement.column_count();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for index in 0..columns {
                if let Ok(text) = row.get::<_, String>(index) {
                    stored.push_str(&text);
                }
            }
        }
    }
    assert!(!stored.contains(secret), "改判理由的口令原文落库了");
    assert!(stored.contains("Rc5***"));
    let _ = std::fs::remove_dir_all(root);
}

/// P1 验收（2026-10-09）：隔离环境完整闭环，全部走生产代码路径——导入 → 真实（本机夹具）站点
/// 发现 → 归属组确认（逐资产单位投影）→ 事件建议与单位关联 → 核实 → 复测轮（403）不可关闭 →
/// 复测轮（404）可人工关闭；全程无通知/下架出站。
#[test]
fn rem_acceptance_full_loop_import_discover_own_incident_retest_close() {
    use super::incidents::{act, propose, read_incident};
    use base64::Engine;
    let leaked = r#"{"db_password": "Lp4Wd9Qe7Rt2Yx"}"#;
    let fixture = Fixture::start(routes(vec![
        ("/", vec![html(r#"<html><title>首页</title><a href="/config.json">配置</a></html>"#)]),
        ("/config.json", vec![json_reply(leaked), response(403, &[], "forbidden"), response(404, &[], "not found")]),
    ]));
    let root = sandbox("rem-loop");
    let (connection, db_path) = database(&root);
    let pid = 481;
    seed_scope(&connection, pid, &fixture.origin, true);
    directory(&connection);
    bind(&connection, pid, "group_ledger", 1, None);
    let asset: i64 = connection.query_row("SELECT id FROM assets WHERE asset_key=?1", [&fixture.origin], |row| row.get(0)).unwrap();

    // ① 导入：用户提供的内部文件，登记来源声明，不发任何请求。
    let file = base64::engine::general_purpose::STANDARD.encode("样本公司 内部值班表（用户提供）");
    let imported = super::imports::import_batch(
        &connection,
        pid,
        &serde_json::json!({"operationId": "op-rem-loop-import-1", "channel": "document_import", "operator": "王五", "sourceDeclaration": "用户从共享盘导出", "files": [{"name": "duty.txt", "format": "text", "contentBase64": file}]}),
    )
    .expect("导入成功");
    let batches: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_import_batches WHERE project_id=?1", [pid], |row| row.get(0)).unwrap();
    assert_eq!(batches, 1, "{imported}");
    assert_eq!(fixture.request_count(), 0, "导入不出站");

    // ② 归属：组确认把该资产归到杭州分公司（经单位投影只读使用）。归属在采集之前确认——
    // 执行器在轮次结束时自动建议事件并挂单位关联（晚于事件的归属决定不回溯，见交付文档未满足项）。
    connection
        .execute("INSERT INTO ownership_analysis_runs(project_id,status,scope_version,data_revision,policy_version,directory_version,binding_version,algorithm_version) VALUES(?1,'completed','s','d',1,5,3,'a')", [pid])
        .unwrap();
    let analysis = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO ownership_review_groups(run_id,project_id,group_key,lane,nav_bucket,candidate_entity_id,candidate_source,relation_kind,boundary_kind,support_kind,decision_status,decision_source,authorization_status,exposure_eligible) VALUES(?1,?2,'k-loop','consistent','province',5,'directory','official_host','exact_host','directory_official_host','auto','auto','unknown',0)",
            params![analysis, pid],
        )
        .unwrap();
    let group = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO ownership_operations(operation_id,project_id,run_id,group_id,kind,action,status,payload_hash,expected_action_hash,result_json) VALUES('op-rem-loop-own',?1,?2,?3,'group_decision','confirm_ownership','completed','h','h','{}')",
            params![pid, analysis, group],
        )
        .unwrap();
    connection.execute("INSERT INTO ownership_operation_items(operation_id,asset_id,outcome) VALUES('op-rem-loop-own',?1,'applied')", [asset]).unwrap();
    connection.execute("UPDATE asset_ownership_decisions SET status='confirmed',source='manual' WHERE project_id=?1 AND asset_id=?2", params![pid, asset]).unwrap();
    connection
        .execute(
            "INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership',?3)",
            params![pid, asset, format!("confirmed: 组确认（复核组 #{group}，快照 #{analysis}，操作 op-rem-loop-own）")],
        )
        .unwrap();
    let edge = crate::commands::ownership::ownership_asset_unit_edge(&connection, pid, asset).unwrap().expect("资产有单位依据");
    assert_eq!(edge.owner_entity_id, 5);

    // ③ 发现：生产执行器对授权站点做一轮被动采集。
    let settings = passive_settings(&connection, pid);
    let first = start_or_reuse(&connection, pid, &settings).unwrap();
    run_worker(&db_path, first.run_id, pid, &settings);
    assert_eq!(run_row(&connection, first.run_id).0, "completed");
    let config_url = fixture.url("/config.json");
    let finding = finding_id_of(&connection, pid, &config_url);

    // ④ 事件：建议 → 单位关联来自归属投影 → 人工核实。
    propose(&connection, pid, Some(first.run_id)).unwrap();
    let incident: i64 = connection
        .query_row("SELECT incident_id FROM exposure_incident_locations WHERE finding_id=?1 AND status='active'", [finding], |row| row.get(0))
        .expect("真实线索进入事件");
    let (entity, origin): (i64, String) = connection
        .query_row("SELECT entity_id,origin FROM exposure_incident_entities WHERE incident_id=?1 AND active=1", [incident], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert_eq!((entity, origin.as_str()), (5, "ownership_projection"));
    let rev = |connection: &Connection| -> i64 { connection.query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [incident], |row| row.get(0)).unwrap() };
    let action = |op: &str, extra: serde_json::Value| -> serde_json::Value {
        let mut body = serde_json::json!({"operationId": op, "incidentId": incident, "expectedRevision": rev(&connection), "actor": "张三"});
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        body
    };
    act(&connection, pid, &action("op-rem-loop-01", serde_json::json!({"action": "review", "status": "verified", "reason": "配置文件公开了数据库口令"}))).expect("核实");
    act(&connection, pid, &action("op-rem-loop-02", serde_json::json!({"action": "record_action", "handling": "notification_recorded", "channel": "电话通知杭州分公司", "reason": "人工通知"}))).unwrap();

    // ⑤ 复测 1：新一轮真实请求得到 403 → 关联复测后不可关闭，拒绝零写入。
    let retest1 = start_or_reuse(&connection, pid, &settings).unwrap();
    assert_ne!(retest1.run_id, first.run_id);
    run_worker(&db_path, retest1.run_id, pid, &settings);
    act(&connection, pid, &action("op-rem-loop-03", serde_json::json!({"action": "record_action", "handling": "retest_linked", "retestRunId": retest1.run_id, "reason": "关联复测 1"}))).unwrap();
    let before = (rev(&connection), connection.query_row("SELECT COUNT(*) FROM exposure_incident_events", [], |row| row.get::<_, i64>(0)).unwrap());
    let refused = act(&connection, pid, &action("op-rem-loop-04", serde_json::json!({"action": "record_action", "handling": "closed_recorded", "reason": "应该好了"}))).unwrap_err();
    assert!(refused.contains("exposure_incident_retest_failed"), "{refused}");
    assert_eq!((rev(&connection), connection.query_row("SELECT COUNT(*) FROM exposure_incident_events", [], |row| row.get::<_, i64>(0)).unwrap()), before);

    // ⑥ 复测 2：404 → 可由人工关闭，记为有复测依据。
    let retest2 = start_or_reuse(&connection, pid, &settings).unwrap();
    run_worker(&db_path, retest2.run_id, pid, &settings);
    let linked = act(&connection, pid, &action("op-rem-loop-05", serde_json::json!({"action": "record_action", "handling": "retest_linked", "retestRunId": retest2.run_id, "reason": "关联复测 2"}))).unwrap();
    assert_eq!(linked["retest"]["autoClosed"], false, "复测从不自动关闭");
    let closed = act(&connection, pid, &action("op-rem-loop-06", serde_json::json!({"action": "record_action", "handling": "closed_recorded", "reason": "复测 404，人工关闭"}))).unwrap();
    assert_eq!(closed["closureBasis"], "manual_with_retest");
    let handling: String = connection.query_row("SELECT handling_state FROM exposure_incidents WHERE id=?1", [incident], |row| row.get(0)).unwrap();
    assert_eq!(handling, "closed", "处置状态登记为已关闭（事件结论仍是“已核实”）");
    let view = read_incident(&connection, pid, incident).unwrap();
    assert!(view["events"].as_array().unwrap().len() >= 6);
    // 请求只发往授权站点；/config.json 三轮各一次以上；登记通知不产生任何出站。
    assert!(fixture.requests().iter().filter(|path| path.as_str() == "/config.json").count() >= 3);
    assert!(fixture.requests().iter().all(|path| path.starts_with('/')));
    drop(fixture);
    let _ = std::fs::remove_dir_all(root);
}

/// FIX2-5 / T08：广告、新闻、公开领导信息、招聘、客服联系方式、公开产品文档各自记录正常公开理由；
/// 只有标题、只有结构字段、出现非公开字样、页面里同时有凭据或业务源码图都不排除；招采与文件名规则仍然优先。
#[test]
fn t08_ordinary_public_pages_are_excluded_with_their_own_reason_and_mixed_pages_stay_with_a_human() {
    use super::classify::{classify, page_context, DISPOSITION_EXCLUDED_MATERIAL, DISPOSITION_EXCLUDED_PROCUREMENT, DISPOSITION_PENDING, DISPOSITION_RISK};
    use super::rules::{CREDENTIAL_PLACEHOLDER_CONFIDENCE, TITLE_MAP_BUSINESS};
    let url = "https://www.example.com.cn/about/page.html";
    let doc = draft("document", "公开路径疑似内部文档", url, "high", 60);
    let cases = [
        ("<title>公司领导</title><h1>领导班子</h1><p>张三，现任总经理，分管市场工作。出生于1970年，研究生学历。</p>", "公开领导介绍"),
        ("<title>人才招聘</title><p>岗位职责：负责网络运维。任职要求：本科。工作地点：杭州。简历投递：hr@example.com.cn</p>", "公开招聘信息"),
        ("<title>联系我们</title><p>客服热线：10086 服务时间：7×24 小时 在线客服</p>", "公开客服联系方式"),
        ("<title>公司新闻</title><p>发布时间：2026-10-01 来源：办公室 责任编辑：李四</p>", "公开新闻"),
        ("<title>用户手册</title><h1>产品说明</h1><p>快速入门、操作步骤、常见问题</p>", "公开产品文档"),
        ("<title>优惠活动</title><p>活动时间：10 月；活动规则：见下；立即办理</p>", "公开广告"),
    ];
    for (body, label) in cases {
        let context = page_context(url, body.as_bytes(), "text/html");
        let verdict = classify(&doc, KIND_PASSIVE_WEB, &context);
        assert_eq!(verdict.disposition, DISPOSITION_EXCLUDED_MATERIAL, "{label}: {verdict:?}");
        assert!(verdict.reason.contains(label), "{label}: {}", verdict.reason);
        assert!(verdict.markers.iter().any(|m| m.starts_with("public:")), "{label}: {:?}", verdict.markers);
    }
    let verdict_of = |body: &str, url: &str, lead: &super::observations::FindingDraft| classify(lead, KIND_PASSIVE_WEB, &page_context(url, body.as_bytes(), "text/html"));
    // 只有标题、结构字段不足 2 个 → 按原规则，不排除。
    assert_eq!(verdict_of("<title>领导班子</title><p>欢迎访问，现任领导见下</p>", url, &doc).disposition, DISPOSITION_RISK);
    // 正文有结构字段、标题不是公开页面 → 不排除；标题字样只出现在正文里也不算。
    assert_eq!(verdict_of("<title>首页</title><p>领导班子：现任总经理，分管市场，出生于1970年</p>", url, &doc).disposition, DISPOSITION_RISK);
    // 公开页面里出现非公开字样 → 混合，留人工。
    let private = verdict_of("<title>领导班子</title><p>张三 现任总经理 分管市场 工号 A001 身份证 330106197001010000</p>", url, &doc);
    assert_eq!(private.disposition, DISPOSITION_PENDING, "{private:?}");
    assert!(private.reason.contains("非公开"), "{}", private.reason);
    // 公开页面里同时有凭据候选或业务源码图 → 混合，留人工。
    let leaders = "<title>领导班子</title><p>张三 现任总经理 分管市场 出生于1970年</p>";
    let cred = draft("credential", "公开资源出现凭据字段", url, "high", CREDENTIAL_PLACEHOLDER_CONFIDENCE + 37);
    let mixed = verdict_of(leaders, url, &cred);
    assert_eq!(mixed.disposition, DISPOSITION_PENDING, "{mixed:?}");
    assert!(mixed.reason.contains("混合"), "{}", mixed.reason);
    assert_eq!(verdict_of(leaders, url, &draft("source_code", TITLE_MAP_BUSINESS, url, "high", 68)).disposition, DISPOSITION_PENDING);
    // 招采公告仍按招采排除；地址里有招采字样仍是待分类。
    assert_eq!(
        verdict_of("<title>招标公告 领导班子</title><p>项目编号：A-1 采购人：某公司 现任 分管</p>", url, &doc).disposition,
        DISPOSITION_EXCLUDED_PROCUREMENT
    );
    assert_eq!(verdict_of(leaders, "https://www.example.com.cn/zbgg/leaders.html", &doc).disposition, DISPOSITION_PENDING);
}

// ───────────────────────────── FIX3（2026-10-09 17:29） ─────────────────────────────

/// FIX3-R1（后端 TOCTOU）：归属在写事务之外读。另一条连接在“读归属”和“写结论”之间把线索
/// 改成别的项目，这次复核必须按事务内的最新归属拒绝，零写入。
#[test]
fn fix3_r1_review_reads_ownership_inside_the_write_transaction() {
    use super::remediation::{review_scoped, top_revision};
    let root = sandbox("fix3-r1");
    let (connection, db_path) = database(&root);
    let pid = 486;
    let r1 = rem_run(&connection, pid, "completed");
    let (finding, observation) = rem_finding(&connection, pid, r1, "https://a.example.test/r1.sql", 200, "R1");
    let read = top_revision(&connection, finding).unwrap();
    rem_run(&connection, 487, "completed");
    let writer = crate::db::open(&db_path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer.execute("UPDATE exposure_findings SET project_id=487 WHERE id=?1", [finding]).unwrap();
    let reviewer_db = db_path.clone();
    let reviewer = std::thread::spawn(move || {
        let mine = crate::db::open(&reviewer_db).unwrap();
        review_scoped(&mine, pid, finding, read, "confirmed", "张三", "按页面证据确认", &[observation])
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    writer.execute_batch("COMMIT").unwrap();
    let result = reviewer.join().unwrap();
    assert!(result.as_ref().is_err_and(|error| error.contains("exposure_scope_mismatch")), "事务内归属已变化：{result:?}");
    assert_eq!(rem_count(&connection, "exposure_review_events"), 0, "零写入");
    let _ = std::fs::remove_dir_all(root);
}

/// FIX3-R2：两名复核员读到同一版本（同一复核事件）。先提交者成功；后提交者冲突、零写入。
#[test]
fn fix3_r2_two_reviewers_on_the_same_version_only_one_wins() {
    use super::remediation::{review_scoped_cas, review_trail, top_revision};
    let root = sandbox("fix3-r2");
    let (connection, _db) = database(&root);
    let pid = 488;
    let r1 = rem_run(&connection, pid, "completed");
    let (finding, observation) = rem_finding(&connection, pid, r1, "https://a.example.test/r2.sql", 200, "R2");
    let read = top_revision(&connection, finding).unwrap();
    let seen = review_trail(&connection, pid, finding).unwrap().last().map(|row| row["eventId"].as_i64().unwrap()).unwrap_or(0);
    review_scoped_cas(&connection, pid, finding, read, Some(seen), "confirmed", "张三", "按页面证据确认", &[observation]).unwrap();
    let before = (rem_count(&connection, "exposure_review_events"), connection.total_changes());
    let status_before: String = connection.query_row("SELECT status FROM exposure_findings WHERE id=?1", [finding], |row| row.get(0)).unwrap();
    let late = review_scoped_cas(&connection, pid, finding, read, Some(seen), "dismissed", "李四", "我看是误报", &[observation]).unwrap_err();
    assert!(late.contains("exposure_review_conflict"), "{late}");
    assert_eq!((rem_count(&connection, "exposure_review_events"), connection.total_changes()), before, "冲突零写入");
    let status: String = connection.query_row("SELECT status FROM exposure_findings WHERE id=?1", [finding], |row| row.get(0)).unwrap();
    assert_eq!(status, status_before);
    // 重新读取后（带上最新事件 id）可以写。
    let fresh = review_trail(&connection, pid, finding).unwrap().last().unwrap()["eventId"].as_i64().unwrap();
    review_scoped_cas(&connection, pid, finding, read, Some(fresh), "dismissed", "李四", "重新读取后改判", &[observation]).unwrap();
    let _ = std::fs::remove_dir_all(root);
}

/// FIX3-R7（自查）：整改状态的终态检查在写事务之外。另一人已把整改驳回（持锁未提交），
/// 这边按旧快照“仍在整改中”改状态，不能覆盖已驳回的终态。
#[test]
fn fix3_r7_a_remediation_rejected_meanwhile_cannot_be_overwritten() {
    use super::remediation::{mark_remediation, record_remediation};
    let root = sandbox("fix3-r7");
    let (connection, db_path) = database(&root);
    let pid = 490;
    let r1 = rem_run(&connection, pid, "completed");
    let (finding, observation) = rem_finding(&connection, pid, r1, "https://a.example.test/r7.sql", 200, "R7");
    let remediation = record_remediation(&connection, pid, finding, Some(observation), r1, "张三", "已删除文件", "fixing").unwrap();
    let writer = crate::db::open(&db_path).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    writer.execute("UPDATE exposure_remediations SET status='rejected' WHERE id=?1", [remediation]).unwrap();
    let other_db = db_path.clone();
    let other = std::thread::spawn(move || {
        let mine = crate::db::open(&other_db).unwrap();
        mark_remediation(&mine, pid, remediation, "awaiting_retest", "李四", "等待复测")
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    writer.execute_batch("COMMIT").unwrap();
    let result = other.join().unwrap();
    assert!(result.as_ref().is_err_and(|error| error.contains("exposure_remediation_terminal")), "{result:?}");
    let status: String = connection.query_row("SELECT status FROM exposure_remediations WHERE id=?1", [remediation], |row| row.get(0)).unwrap();
    assert_eq!(status, "rejected", "已驳回的终态被覆盖");
    let _ = std::fs::remove_dir_all(root);
}

/// FIX3-R8（自查）：人工录入线索的标题/地址/摘录原样落库，粘进去的秘密以明文留在库里；
/// 两条秘密不同、脱敏后相同的线索也不能被合并成一条。同一输入再存仍是同一条。
#[test]
fn fix3_r8_a_manual_finding_is_stored_redacted_and_distinct_secrets_stay_distinct() {
    let root = sandbox("fix3-r8");
    let (connection, _db) = database(&root);
    let pid = 491;
    rem_run(&connection, pid, "completed");
    let first = "SNTmanual+Qw3/Er8Ty2Ui";
    let twin = "SNTmanual+Qw4/Er8Ty2Ui";
    let input = |secret: &str| -> super::super::ExposureFindingInput {
        serde_json::from_value(serde_json::json!({
            "projectId": pid, "assetId": null, "category": "credential", "severity": "high", "confidence": 70,
            "title": format!("配置泄漏 password={secret}"), "sourceType": "manual",
            // 真实 URL 的口令里 `+`、`/` 必须百分号编码；扫描同时覆盖原文和编码形态。
            "sourceUrl": format!("https://ops:{}@a.example.test/cfg", secret.replace('+', "%2B").replace('/', "%2F")),
            "evidenceExcerpt": format!("token={secret}"),
        }))
        .unwrap()
    };
    let a = super::super::save_exposure_finding_in(&connection, input(first)).unwrap();
    let again = super::super::save_exposure_finding_in(&connection, input(first)).unwrap();
    assert_eq!(a, again, "同一输入仍是同一条");
    let b = super::super::save_exposure_finding_in(&connection, input(twin)).unwrap();
    assert_ne!(a, b, "脱敏后相同、原文不同的两条线索被合并");
    let mut statement = connection.prepare("SELECT title||source_url||evidence_excerpt||fingerprint FROM exposure_findings WHERE project_id=?1").unwrap();
    let rows: Vec<String> = statement.query_map([pid], |row| row.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        let hits = fix3_hits(row.as_bytes(), &[first, twin]);
        assert!(hits.is_empty(), "线索行含原值：{hits:?} {row}");
    }
    let _ = std::fs::remove_dir_all(root);
}

// ───────── FIX3 隔离环境完整业务闭环 + 哨兵秘密全量扫描 ─────────

/// 一个秘密原值可能以哪些形态出现：明文、URL 编码、hex（大小写）、base64（标准/URL 安全，三种字节对齐）。
fn fix3_secret_forms(secret: &str) -> Vec<(String, String)> {
    use base64::Engine;
    let mut forms = vec![("plain".to_string(), secret.to_string())];
    let encoded: String = secret
        .bytes()
        .map(|byte| if byte.is_ascii_alphanumeric() { (byte as char).to_string() } else { format!("%{byte:02X}") })
        .collect();
    forms.push(("url".to_string(), encoded.clone()));
    forms.push(("url_lower".to_string(), encoded.to_lowercase().replace("snt", "SNT")));
    let hex: String = secret.bytes().map(|byte| format!("{byte:02x}")).collect();
    forms.push(("hex".to_string(), hex.clone()));
    forms.push(("HEX".to_string(), hex.to_uppercase()));
    for (label, engine) in [("b64", base64::engine::general_purpose::STANDARD), ("b64url", base64::engine::general_purpose::URL_SAFE)] {
        for pad in 0..3usize {
            let mut bytes = vec![b'#'; pad];
            bytes.extend_from_slice(secret.as_bytes());
            let text = engine.encode(&bytes);
            // 只取完全由秘密字节决定的中段：去掉混入前缀的首组和可能混入后续字节的末组。
            let start = if pad == 0 { 0 } else { 4 };
            let end = text.trim_end_matches('=').len().saturating_sub(4);
            if end > start + 8 {
                forms.push((format!("{label}@{pad}"), text[start..end].to_string()));
            }
        }
    }
    forms
}

fn fix3_hits(haystack: &[u8], secrets: &[&str]) -> Vec<String> {
    let text = String::from_utf8_lossy(haystack);
    let mut hits = Vec::new();
    for secret in secrets {
        for (label, form) in fix3_secret_forms(secret) {
            if text.contains(&form) {
                hits.push(format!("{secret}:{label}"));
            }
        }
    }
    hits
}

/// FIX3 闭环：授权站点 → 采集 → 线索 → 事件 → 复核（含同版本冲突）→ 整改 → 复测 → 关闭；
/// 导入（含脱敏碰撞重放）→ 改判 → 导出。人工输入、站点内容、导入文件里都放了哨兵秘密，
/// 最后扫描临时库每张表每个字段、所有首次/重放/冲突响应和导出文件：任何形态出现即失败。
#[test]
fn fix3_full_business_loop_with_sentinel_secrets_leaves_no_original_anywhere() {
    use super::remediation::{mark_remediation, record_remediation, retest_outcome, review_scoped_cas, review_trail, start_retest, top_revision};
    use base64::Engine;
    let site_secret = "SNTsite+Pw/9qXk2Lm7Rt4Vz";
    let header_secret = "SNTbearerQ8w2Ze5Rk9Tn3Yh6";
    let typed = "SNTtyped+Zq4/Wx8Jc2Nv6Bm1";
    let typed_twin = "SNTtyped+Zq5/Wx8Jc2Nv6Bm1"; // 与 typed 脱敏后相同（同前三位、同后两位、同长度）
    let import_secret = "SNTimport+Hd3/Kp7Fs1Gw5Qa";
    let file_secret = "SNTfileQ2w9Er4Ty7Ui1Op3As";
    let secrets = [site_secret, header_secret, typed, typed_twin, import_secret, file_secret];
    assert_eq!(super::redaction::redact_text(&format!("password={typed}")), super::redaction::redact_text(&format!("password={typed_twin}")));
    let leaked = format!(r#"{{"db_password": "{site_secret}", "note": "Authorization: Bearer {header_secret}"}}"#);
    let fixture = Fixture::start(routes(vec![
        ("/", vec![html(r#"<html><title>首页</title><a href="/config.json">配置</a></html>"#)]),
        ("/config.json", vec![json_reply(&leaked), response(404, &[], "not found")]),
    ]));
    let root = sandbox("fix3-loop");
    let (connection, db_path) = database(&root);
    let pid = 489;
    seed_scope(&connection, pid, &fixture.origin, true);
    let settings = passive_settings(&connection, pid);
    let mut responses: Vec<(String, String)> = Vec::new();
    let mut keep = |label: &str, value: String| responses.push((label.to_string(), value));

    // ① 采集 → 线索 → 事件。
    let first = start_or_reuse(&connection, pid, &settings).expect("第一轮");
    run_worker(&db_path, first.run_id, pid, &settings);
    assert_eq!(run_row(&connection, first.run_id).0, "completed");
    let config_url = fixture.url("/config.json");
    let finding = finding_id_of(&connection, pid, &config_url);
    let proposed = super::incidents::propose(&connection, pid, Some(first.run_id)).unwrap();
    keep("propose", format!("{proposed:?}"));
    let incident: i64 = connection
        .query_row("SELECT incident_id FROM exposure_incident_locations WHERE finding_id=?1 AND status='active'", [finding], |row| row.get(0))
        .unwrap();
    keep("read_incident", super::incidents::read_incident(&connection, pid, incident).unwrap().to_string());
    keep("evidence", super::repository::exposure_evidence(&connection, finding, pid).unwrap().to_string());

    // ② 事件复核：理由里粘了秘密；重放返回首次结果；脱敏碰撞的另一秘密冲突。
    let revision: i64 = connection.query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [incident], |row| row.get(0)).unwrap();
    let verify = |secret: &str| serde_json::json!({"operationId": "op-f3-loop-01", "action": "review", "incidentId": incident, "expectedRevision": revision, "actor": "张三", "reason": format!("页面公开了 password={secret}"), "status": "verified"});
    keep("act_first", super::incidents::act(&connection, pid, &verify(typed)).unwrap().to_string());
    let replay = super::incidents::act(&connection, pid, &verify(typed)).unwrap();
    assert_eq!(replay["replayed"], true);
    keep("act_replay", replay.to_string());
    let collided = super::incidents::act(&connection, pid, &verify(typed_twin)).unwrap_err();
    assert!(collided.contains("exposure_incident_operation_conflict"), "{collided}");
    keep("act_collision", collided);

    // ③ 线索复核：两名复核员同一版本，只有一个成功。
    let read = top_revision(&connection, finding).unwrap();
    let seen = review_trail(&connection, pid, finding).unwrap().last().map(|row| row["eventId"].as_i64().unwrap()).unwrap_or(0);
    let observation: i64 = connection.query_row("SELECT MAX(id) FROM exposure_observations WHERE run_id=?1 AND url=?2", params![first.run_id, config_url], |row| row.get(0)).unwrap();
    keep("review_a", format!("{:?}", review_scoped_cas(&connection, pid, finding, read, Some(seen), "confirmed", "张三", &format!("确认 token={typed}"), &[observation])));
    let loser = review_scoped_cas(&connection, pid, finding, read, Some(seen), "dismissed", "李四", &format!("误报 token={typed_twin}"), &[observation]).unwrap_err();
    assert!(loser.contains("exposure_review_conflict"), "{loser}");
    keep("review_b", loser);
    keep("trail", serde_json::to_string(&review_trail(&connection, pid, finding).unwrap()).unwrap());

    // ④ 整改 → 复测（404）→ 按依据关闭整改；事件关联复测后登记关闭。
    let remediation = record_remediation(&connection, pid, finding, Some(observation), first.run_id, "张三", &format!("已轮换 password={typed}"), "fixing").expect("整改");
    let retest = start_retest(&connection, pid, remediation, &settings).expect("复测");
    run_worker(&db_path, retest.run_id, pid, &settings);
    let outcome = retest_outcome(&connection, pid, remediation).unwrap();
    assert_eq!(outcome["canClose"], true, "{outcome}");
    keep("retest_outcome", outcome.to_string());
    keep("mark", mark_remediation(&connection, pid, remediation, "closed", "张三", &format!("复测 404 secret={typed}")).unwrap().to_string());
    let revision: i64 = connection.query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [incident], |row| row.get(0)).unwrap();
    keep("link_retest", super::incidents::act(&connection, pid, &serde_json::json!({"operationId": "op-f3-loop-02", "action": "record_action", "incidentId": incident, "expectedRevision": revision, "actor": "张三", "reason": "关联复测轮次", "handling": "retest_linked", "retestRunId": retest.run_id})).unwrap().to_string());
    let revision: i64 = connection.query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [incident], |row| row.get(0)).unwrap();
    let closed = super::incidents::act(&connection, pid, &serde_json::json!({"operationId": "op-f3-loop-03", "action": "record_action", "incidentId": incident, "expectedRevision": revision, "actor": "张三", "reason": format!("复测 404，已关闭 api_key={typed}"), "handling": "closed_recorded", "channel": format!("工单 token={typed}"), "externalReference": format!("REF password={typed}")})).unwrap();
    assert_eq!(closed["closureBasis"], "manual_with_retest");
    keep("close", closed.to_string());

    // ⑤ 导入：来源声明和文件正文里都有秘密；重放；碰撞冲突。
    let file = format!("样本公司 内部资料\npassword={file_secret}\n");
    let import = |secret: &str| serde_json::json!({
        "operationId": "op-f3-loop-imp1", "channel": "document_import", "operator": "王五",
        "sourceDeclaration": format!("共享盘 token={secret} 导出"),
        "files": [{"name": "a.txt", "format": "text", "contentBase64": base64::engine::general_purpose::STANDARD.encode(file.as_bytes())}],
    });
    keep("import_first", super::imports::import_batch(&connection, pid, &import(import_secret)).unwrap().to_string());
    let again = super::imports::import_batch(&connection, pid, &import(import_secret)).unwrap();
    assert_eq!(again["replayed"], true);
    keep("import_replay", again.to_string());

    // ⑥ 人工改判（理由里粘秘密）+ 重放。
    let classification: i64 = connection
        .query_row("SELECT classification_id FROM exposure_finding_dispositions WHERE finding_id=?1", [finding], |row| row.get(0))
        .unwrap_or(0);
    let reclass = serde_json::json!({"operationId": "op-f3-loop-rc1", "findingId": finding, "expectedClassificationId": classification, "disposition": "risk_candidate", "infoCategory": "credential", "actor": "张三", "reason": format!("人工改判 password={typed}")});
    keep("reclassify_first", format!("{:?}", super::classify::reclassify(&connection, pid, &reclass)));
    keep("reclassify_replay", format!("{:?}", super::classify::reclassify(&connection, pid, &reclass)));

    // ⑦ 导出。
    let dir = root.join("exports");
    std::fs::create_dir_all(&dir).unwrap();
    let now = chrono::Local::now();
    let mut exported = Vec::new();
    for kind in ["findings", "ledger"] {
        let (path, rows) = super::super::export_exposure_in(&connection, &dir, pid, kind, now).unwrap();
        assert!(rows > 0, "{kind}");
        exported.push((kind, std::fs::read(&path).unwrap()));
    }

    // ⑧ 扫描：临时库每张表每个字段。
    let mut failures = Vec::new();
    let tables: Vec<String> = {
        let mut statement = connection.prepare("SELECT name FROM sqlite_master WHERE type='table'").unwrap();
        let rows = statement.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    };
    let mut cells = 0usize;
    for table in &tables {
        let mut statement = connection.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let columns: Vec<String> = statement.column_names().iter().map(|name| name.to_string()).collect();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for (index, column) in columns.iter().enumerate() {
                let bytes: Vec<u8> = match row.get_ref(index).unwrap() {
                    rusqlite::types::ValueRef::Text(value) | rusqlite::types::ValueRef::Blob(value) => value.to_vec(),
                    _ => continue,
                };
                cells += 1;
                for hit in fix3_hits(&bytes, &secrets) {
                    failures.push(format!("db {table}.{column}: {hit}"));
                }
            }
        }
    }
    for (label, value) in &responses {
        for hit in fix3_hits(value.as_bytes(), &secrets) {
            failures.push(format!("response {label}: {hit}"));
        }
    }
    for (kind, bytes) in &exported {
        for hit in fix3_hits(bytes, &secrets) {
            failures.push(format!("export {kind}: {hit}"));
        }
    }
    let forms = fix3_secret_forms(site_secret).len();
    eprintln!(
        "FIX3_SCAN tables={} text_cells={} responses={} exports={} secrets={} forms_per_secret={} failures={}",
        tables.len(), cells, responses.len(), exported.len(), secrets.len(), forms, failures.len()
    );
    assert!(failures.is_empty(), "秘密原值出现：{failures:#?}");
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── FIX4（2026-10-09 20:37 / 21:07 原六项） ─────────────────────────────

// ───────── FIX4 R8 导入元数据秘密扫描：按列类型、解码后、递归 JSON、文件名/URL/头 ─────────

fn fix4_percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2])) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn fix4_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek().copied() {
                Some('u') => {
                    chars.next();
                    let hex: String = chars.by_ref().take(4).collect();
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(decoded) => out.push(decoded),
                        None => out.push_str(&hex),
                    }
                }
                Some(next @ ('/' | '"' | '\\')) => {
                    chars.next();
                    out.push(next);
                }
                _ => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out.replace("&amp;", "&").replace("&#43;", "+").replace("&#x2B;", "+").replace("&#x2F;", "/").replace("&#47;", "/")
}

/// 一个单元格/响应/文件可能藏秘密的所有解码形态：原字节、gzip/zlib/brotli 解压、百分号解码（两层）、
/// JSON/HTML 转义还原；若是 JSON，再递归取出每个键和值并同样解码。
fn fix4_decoded_views(bytes: &[u8], json_cells: &mut usize, nested: &mut usize) -> Vec<String> {
    use std::io::Read;
    let mut raw = vec![bytes.to_vec()];
    let mut inflated = Vec::new();
    if flate2::read::GzDecoder::new(bytes).read_to_end(&mut inflated).is_ok() && !inflated.is_empty() {
        raw.push(inflated.clone());
    }
    inflated.clear();
    if flate2::read::ZlibDecoder::new(bytes).read_to_end(&mut inflated).is_ok() && !inflated.is_empty() {
        raw.push(inflated.clone());
    }
    inflated.clear();
    if brotli::Decompressor::new(bytes, 4096).read_to_end(&mut inflated).is_ok() && !inflated.is_empty() {
        raw.push(inflated);
    }
    let mut views = Vec::new();
    for chunk in raw {
        let text = String::from_utf8_lossy(&chunk).to_string();
        if let Ok(value) = serde_json::from_str::<JsonValue>(&text) {
            if value.is_object() || value.is_array() {
                *json_cells += 1;
                let mut strings = Vec::new();
                fix4_json_strings(&value, &mut strings);
                *nested += strings.len();
                for item in strings {
                    views.push(fix4_percent_decode(&fix4_percent_decode(&item)));
                    views.push(item);
                }
            }
        }
        views.push(fix4_unescape(&text));
        views.push(fix4_percent_decode(&text));
        views.push(fix4_percent_decode(&fix4_percent_decode(&fix4_unescape(&text))));
        views.push(text);
    }
    views
}

fn fix4_json_strings(value: &JsonValue, out: &mut Vec<String>) {
    match value {
        JsonValue::String(text) => {
            out.push(text.clone());
            if let Ok(inner) = serde_json::from_str::<JsonValue>(text) {
                if inner.is_object() || inner.is_array() {
                    fix4_json_strings(&inner, out);
                }
            }
        }
        JsonValue::Array(rows) => rows.iter().for_each(|row| fix4_json_strings(row, out)),
        JsonValue::Object(map) => {
            for (key, item) in map {
                out.push(key.clone());
                fix4_json_strings(item, out);
            }
        }
        _ => {}
    }
}

#[derive(Default, Debug)]
struct Fix4Scan {
    tables: usize,
    columns_by_type: std::collections::BTreeMap<String, usize>,
    text_cells: usize,
    blob_cells: usize,
    json_cells: usize,
    nested_strings: usize,
    failures: Vec<String>,
}

impl Fix4Scan {
    fn bytes(&mut self, label: &str, bytes: &[u8], secrets: &[&str]) {
        let views = fix4_decoded_views(bytes, &mut self.json_cells, &mut self.nested_strings);
        let mut seen = std::collections::BTreeSet::new();
        for view in views {
            for hit in fix3_hits(view.as_bytes(), secrets) {
                if seen.insert(hit.clone()) {
                    self.failures.push(format!("{label}: {hit}"));
                }
            }
        }
    }

    fn database(&mut self, connection: &Connection, secrets: &[&str]) {
        let tables: Vec<String> = {
            let mut statement = connection.prepare("SELECT name FROM sqlite_master WHERE type='table'").unwrap();
            let rows = statement.query_map([], |row| row.get::<_, String>(0)).unwrap();
            rows.collect::<Result<_, _>>().unwrap()
        };
        self.tables = tables.len();
        for table in &tables {
            let declared: Vec<(String, String)> = {
                let mut statement = connection.prepare(&format!("PRAGMA table_info(\"{table}\")")).unwrap();
                let rows = statement.query_map([], |row| Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))).unwrap();
                rows.collect::<Result<_, _>>().unwrap()
            };
            for (_, kind) in &declared {
                *self.columns_by_type.entry(if kind.is_empty() { "(none)".to_string() } else { kind.to_uppercase() }).or_default() += 1;
            }
            let mut statement = connection.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
            let columns: Vec<String> = statement.column_names().iter().map(|name| name.to_string()).collect();
            let mut rows = statement.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                for (index, column) in columns.iter().enumerate() {
                    let bytes: Vec<u8> = match row.get_ref(index).unwrap() {
                        rusqlite::types::ValueRef::Text(value) => {
                            self.text_cells += 1;
                            value.to_vec()
                        }
                        rusqlite::types::ValueRef::Blob(value) => {
                            self.blob_cells += 1;
                            value.to_vec()
                        }
                        _ => continue,
                    };
                    self.bytes(&format!("db {table}.{column}"), &bytes, secrets);
                }
            }
        }
    }
}

/// FIX4 P1／R8：导入的人工元数据（operator、declaredUrl 里的 token、文件名、来源声明、批次名）带哨兵秘密，
/// 走完整导入 → 线索 → 导出。扫描每张表每一列（按类型、BLOB 解码、JSON 递归）、导入响应（首次/重放）、
/// 证据响应（嵌套）、导出内容与导出文件名：任何形态出现即失败。
#[test]
fn fix4_r8_import_metadata_secrets_never_reach_storage_responses_or_exports() {
    use base64::Engine;
    let url_secret = "SNTurltok9Xc4Vb7Nm2Lk5Jh8";
    let operator_secret = "SNToper3Fg6Hj9Kl2Zx5Cv";
    let fname_secret = "SNTfname4Az7Sx1Dc5Fv8Gb";
    let decl_secret = "SNTdecl+Ws3/Ed6Rf9Tg2Yh5";
    let label_secret = "SNTlabel2Qa5Ws8Ed1Rf4Tg";
    let secrets = [url_secret, operator_secret, fname_secret, decl_secret, label_secret];
    let root = sandbox("fix4-r8");
    let (connection, _db_path) = database(&root);
    let pid = 596;
    seed_scope(&connection, pid, "https://a.example.test", true);
    let mut responses: Vec<(String, String)> = Vec::new();
    let file = "样本公司 内部资料\npassword=SNTbodyQ2w9Er4Ty7Ui1Op3As\n";
    let import = serde_json::json!({
        "operationId": "op-f4-r8-imp1", "channel": "document_import",
        "operator": format!("王五 token={operator_secret}"),
        "sourceDeclaration": format!("共享盘 secret={decl_secret} 导出"),
        "batchLabel": format!("批次 api_key={label_secret}"),
        "declaredUrl": format!("https://a.example.test/share/doc.txt?access_token={url_secret}"),
        "files": [{"name": format!("token={fname_secret}.txt"), "format": "text", "contentBase64": base64::engine::general_purpose::STANDARD.encode(file.as_bytes())}],
    });
    let first = super::imports::import_batch(&connection, pid, &import);
    responses.push(("import_first".into(), format!("{first:?}")));
    let first = first.expect("导入必须成功");
    let replay = super::imports::import_batch(&connection, pid, &import).expect("重放");
    assert_eq!(replay["replayed"], true, "{replay}");
    responses.push(("import_replay".into(), replay.to_string()));
    let findings: Vec<i64> = {
        let mut statement = connection.prepare("SELECT id FROM exposure_findings WHERE project_id=?1").unwrap();
        let rows = statement.query_map([pid], |row| row.get(0)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    };
    for finding in &findings {
        responses.push((format!("evidence {finding}"), super::repository::exposure_evidence(&connection, *finding, pid).unwrap().to_string()));
    }
    let dir = root.join("exports");
    std::fs::create_dir_all(&dir).unwrap();
    let now = chrono::Local::now();
    let mut scan = Fix4Scan::default();
    for kind in ["findings", "ledger"] {
        let (path, _rows) = super::super::export_exposure_in(&connection, &dir, pid, kind, now).unwrap();
        scan.bytes(&format!("export-name {kind}"), path.to_string_lossy().as_bytes(), &secrets);
        scan.bytes(&format!("export {kind}"), &std::fs::read(&path).unwrap(), &secrets);
    }
    scan.database(&connection, &secrets);
    for (label, value) in &responses {
        scan.bytes(&format!("response {label}"), value.as_bytes(), &secrets);
    }
    eprintln!(
        "FIX4_R8_SCAN tables={} columns_by_type={:?} text_cells={} blob_cells={} json_cells={} nested_strings={} responses={} findings={} secrets={} failures={}",
        scan.tables, scan.columns_by_type, scan.text_cells, scan.blob_cells, scan.json_cells, scan.nested_strings, responses.len(), findings.len(), secrets.len(), scan.failures.len()
    );
    assert!(scan.json_cells > 0 && scan.nested_strings > 0, "扫描器必须真的走到 JSON 元数据");
    assert!(scan.failures.is_empty(), "秘密原值出现：{:#?}", scan.failures);
    // 脱敏后仍可读：操作者与声明 URL 保留可辨认的打码形态。
    let (operator, declared): (String, String) = connection
        .query_row("SELECT operator,declared_url FROM exposure_import_batches WHERE project_id=?1", [pid], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap();
    assert!(operator.starts_with("王五") && operator.contains("***"), "{operator}");
    assert!(declared.contains("access_token=SNT***h8(len=25)"), "{declared}");
    // 原文不同、脱敏后相同的另一份元数据不能被当成同一载荷重放。
    let mut twin = import.clone();
    twin["operator"] = serde_json::json!("王五 token=SNTopex9Fg6Hj9Kl2Zx5Cv");
    let collided = super::imports::import_batch(&connection, pid, &twin).unwrap_err();
    assert!(collided.contains("conflict") || collided.contains("冲突"), "{collided} / first={first}");
    let _ = std::fs::remove_dir_all(root);
}

/// FIX4-R8 扫描器自检：BLOB（gzip）、嵌套 JSON、百分号编码、JSON 转义里的秘密都要被找到。
#[test]
fn fix4_r8_the_scanner_finds_secrets_in_blobs_nested_json_and_encodings() {
    use std::io::Write;
    let secret = "SNTselfcheck+Ab3/Cd5Ef7Gh";
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(format!("x={secret}").as_bytes()).unwrap();
    let gzipped = gz.finish().unwrap();
    let nested = serde_json::json!({"meta": {"extra": serde_json::json!({"details": [secret]}).to_string()}}).to_string();
    let percent: String = format!("q={}", secret.replace('+', "%2B").replace('/', "%2F"));
    let escaped = format!("{{\"k\":\"{}\"}}", secret.replace('/', "\\/").replace('+', "\\u002b"));
    for (label, bytes) in [("gzip", gzipped), ("nested", nested.into_bytes()), ("percent", percent.into_bytes()), ("escaped", escaped.into_bytes())] {
        let mut scan = Fix4Scan::default();
        scan.bytes(label, &bytes, &[secret]);
        assert!(!scan.failures.is_empty(), "{label}: 扫描器必须发现");
    }
}

/// 最小 CSV 解析（引号、转义引号、引号内换行），只用于核对导出文件。
fn fix4_parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            other => field.push(other),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// FIX4 P2：线索 CSV 按表头逐列核对字段语义（实际生成的导出文件）。
#[test]
fn fix4_p2_csv_every_findings_column_holds_what_its_header_says() {
    let root = sandbox("fix4-csv");
    let (connection, _db) = database(&root);
    let pid = 598;
    let r1 = rem_run(&connection, pid, "completed");
    let (finding, _observation) = rem_finding(&connection, pid, r1, "https://a.example.test/csv.sql", 200, "CSV");
    for revision in [1, 2] {
        connection
            .execute("INSERT INTO exposure_finding_versions(finding_id,run_id,revision,review_needed) VALUES(?1,?2,?3,1)", params![finding, r1, revision])
            .unwrap();
    }
    let dir = root.join("exports");
    std::fs::create_dir_all(&dir).unwrap();
    let (path, rows) = super::super::export_exposure_in(&connection, &dir, pid, "findings", chrono::Local::now()).unwrap();
    assert_eq!(rows, 1);
    let parsed = fix4_parse_csv(&std::fs::read_to_string(&path).unwrap());
    let header = &parsed[0];
    let data = &parsed[1];
    assert_eq!(header.len(), data.len(), "表头与数据列数一致");
    let col = |name: &str| -> String { data[header.iter().position(|h| h == name).unwrap_or_else(|| panic!("缺列 {name}"))].clone() };
    let db: (String, String, String, i64, String, String, String, String, String, String) = connection
        .query_row(
            "SELECT category,title,severity,confidence,status,source_type,source_url,evidence_excerpt,first_seen_at,last_seen_at FROM exposure_findings WHERE id=?1",
            [finding],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?)),
        )
        .unwrap();
    let got: Vec<(&str, String)> = super::export::HEADERS.iter().map(|name| (*name, col(name))).collect();
    let expected: Vec<(&str, String)> = vec![
        ("类别", db.0.clone()),
        ("标题", db.1.clone()),
        ("风险", db.2.clone()),
        ("置信度", db.3.to_string()),
        ("状态", db.4.clone()),
        ("来源类型", db.5.clone()),
        ("来源 URL", db.6.clone()),
        ("脱敏证据", super::redaction::redact_text(&db.7)),
        ("关联观察数", "1".to_string()),
        ("需复核", "2".to_string()),
        ("规则版本", super::state::EXPOSURE_RULE_VERSION.to_string()),
        ("脱敏版本", super::redaction::REDACTION_VERSION.to_string()),
        ("首次发现", db.8.clone()),
        ("最近发现", db.9.clone()),
    ];
    assert_eq!(got, expected, "逐列语义");
    let _ = std::fs::remove_dir_all(root);
}
