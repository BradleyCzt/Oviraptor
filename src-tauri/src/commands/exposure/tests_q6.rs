// §Q6 — clusters, cache keys and the scale checks. The assertions are about what a cluster
// does *not* do (grant, delete, absorb) as much as what it stores.

use super::cache::{
    entry_count, invalidate_before_rule, invalidate_content, lookup, store, CacheKey,
};
use super::cluster_repository::{
    direct_members, invalidate_inherited_from, read, save, EVIDENCE_DIRECT, EVIDENCE_INHERITED,
    EVIDENCE_STALE,
};
use super::clusters::{content_clusters, domain_clusters, inheritance_for, ClusterMember};
use super::orchestrator::{run_plan, ItemOutcome, KIND_PASSIVE_WEB};
use super::scope::ExposureScope;
use super::state::{ensure_exposure_schema, RunStatus, WorkItemStatus};
use super::work_items::{counts, enumerate_gaps, plan, WorkItem, WorkItemSeed};
use rusqlite::{params, Connection};
use serde_json::json;
use std::path::{Path, PathBuf};

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oviraptor-exposure-q6-{tag}-{}",
        uuid::Uuid::new_v4()
    ))
}

fn database(root: &Path) -> Connection {
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).expect("临时库连接");
    ensure_exposure_schema(&connection).expect("附加结构必须可用");
    // 临时测试库不需要落盘强度，用来把 4,000 次短事务压到可接受时长。
    connection
        .pragma_update(None, "synchronous", "OFF")
        .expect("测试库 pragma 必须可用");
    connection
}

fn asset(connection: &Connection, id: i64, host: &str) {
    connection
        .execute(
            "INSERT OR IGNORE INTO assets(id,asset_key,link,host) VALUES(?1,?2,?3,?4)",
            params![id, format!("q6-{id}"), format!("https://{host}/"), host],
        )
        .unwrap();
}

fn member(id: i64, host: &str, hash: &str) -> ClusterMember {
    ClusterMember {
        asset_id: id,
        origin: format!("https://{host}/"),
        content_hash: hash.to_string(),
        confidence: 80,
    }
}

#[test]
fn q6_the_cache_key_binds_project_scope_privacy_content_rules_and_kind() {
    let root = sandbox("cache");
    let connection = database(&root);
    let base = CacheKey::new(71, 3, "public", "hash-a", "sourcemap");
    assert!(
        lookup(&connection, &base).unwrap().is_none(),
        "首次必须未命中"
    );
    let payload = "{\"verdict\":\"business\"}";
    store(&connection, &base, payload).expect("写入");
    let hit = lookup(&connection, &base).unwrap().expect("再次必须命中");
    assert_eq!(hit.result, payload);
    assert_eq!(hit.hits, 1);
    lookup(&connection, &base).unwrap();
    assert_eq!(lookup(&connection, &base).unwrap().unwrap().hits, 3);

    // 任何一个维度移动都必须未命中，而不是复用另一套授权的结论。
    for other in [
        CacheKey::new(72, 3, "public", "hash-a", "sourcemap"),
        CacheKey::new(71, 4, "public", "hash-a", "sourcemap"),
        CacheKey::new(71, 3, "internal", "hash-a", "sourcemap"),
        CacheKey::new(71, 3, "public", "hash-b", "sourcemap"),
        CacheKey::new(71, 3, "public", "hash-a", "credential"),
    ] {
        assert_ne!(base.as_text(), other.as_text());
        assert!(lookup(&connection, &other).unwrap().is_none(), "{other:?}");
    }
    assert_eq!(entry_count(&connection, 71).unwrap(), 1);

    // 内容变化只让对应哈希失效；别的项目不受影响。
    store(
        &connection,
        &CacheKey::new(71, 3, "public", "hash-c", "sourcemap"),
        "{}",
    )
    .unwrap();
    store(
        &connection,
        &CacheKey::new(99, 3, "public", "hash-c", "sourcemap"),
        "{}",
    )
    .unwrap();
    assert_eq!(invalidate_content(&connection, 71, "hash-c").unwrap(), 1);
    assert_eq!(entry_count(&connection, 71).unwrap(), 1);
    assert_eq!(
        entry_count(&connection, 99).unwrap(),
        1,
        "跨项目不能被连带清除"
    );

    // 规则包换版后，旧规则下的结论全部不可复用。
    assert!(invalidate_before_rule(&connection, 71, "oviraptor.exposure.rules.v999").unwrap() > 0);
    assert_eq!(entry_count(&connection, 71).unwrap(), 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q6_cluster_graph_labels_inheritance_and_survives_a_revoked_representative() {
    let root = sandbox("clusters");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(81,'范围','active')",
            [],
        )
        .unwrap();
    for (id, host) in [
        (1, "a.corp.invalid"),
        (2, "b.corp.invalid"),
        (3, "c.other.invalid"),
    ] {
        asset(&connection, id, host);
    }
    let rows = vec![
        member(1, "a.corp.invalid", "same"),
        member(2, "b.corp.invalid", "same"),
        member(3, "c.other.invalid", "unique"),
    ];
    let cluster = content_clusters(&rows).remove(0);
    let id = save(&connection, 81, &cluster, "2026-10-07 12:00:00", 555).expect("簇必须可持久化");
    let stored = read(&connection, 81).expect("簇可读回");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].id, id);
    assert_eq!(
        stored[0].members.len(),
        2,
        "成员保持完整：{:?}",
        stored[0].members
    );
    let representative = stored[0]
        .members
        .iter()
        .find(|row| row.asset_id == cluster.representative)
        .expect("代表必须在成员里");
    assert_eq!(representative.evidence_source, EVIDENCE_DIRECT);
    let inherited = stored[0]
        .members
        .iter()
        .find(|row| row.asset_id != cluster.representative)
        .expect("另一个成员必须是继承");
    assert_eq!(inherited.evidence_source, EVIDENCE_INHERITED);
    assert_eq!(
        inherited.inherited_from_asset_id,
        Some(cluster.representative),
        "继承必须注明来源资产"
    );
    assert_eq!(inherited.verified_at, "", "继承不能被标成已直接验证");
    assert_eq!(direct_members(&connection, 81).unwrap().len(), 1);

    // 代表被撤销：继承行变陈旧，但资产行一条都不能少。
    let assets_before: i64 = connection
        .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
        .unwrap();
    let stale = invalidate_inherited_from(&connection, 81, cluster.representative).unwrap();
    assert_eq!(stale, 1);
    let after = read(&connection, 81).unwrap();
    assert_eq!(
        after[0]
            .members
            .iter()
            .filter(|row| row.evidence_source == EVIDENCE_STALE)
            .count(),
        1
    );
    let assets_after: i64 = connection
        .query_row("SELECT COUNT(*) FROM assets", [], |row| row.get(0))
        .unwrap();
    assert_eq!(assets_before, assets_after, "聚类绝不删资产");

    // 域名簇与内容簇互不改写。
    let domains = domain_clusters(&rows);
    assert_eq!(domains[0].members, vec![1, 2]);
    save(&connection, 81, &domains[0], "2026-10-07 12:00:00", 556).ok();
    assert_eq!(read(&connection, 81).unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q6_a_cluster_never_grants_authorization_to_a_member() {
    let root = sandbox("scope");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(91,'范围','active')",
            [],
        )
        .unwrap();
    asset(&connection, 11, "only-a.corp.invalid");
    asset(&connection, 12, "only-b.corp.invalid");
    // 只有资产 11 通过归属门禁。
    connection
        .execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(91,11),(91,12)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO asset_ownership_decisions(project_id,asset_id,status,authorization_status,exposure_eligible,confidence)
             VALUES(91,11,'confirmed','allowed',1,90),(91,12,'confirmed','allowed',0,90)",
            [],
        )
        .unwrap();
    let rows = vec![
        member(11, "only-a.corp.invalid", "same"),
        member(12, "only-b.corp.invalid", "same"),
    ];
    let cluster = content_clusters(&rows).remove(0);
    save(&connection, 91, &cluster, "2026-10-07 12:00:00", 777).ok();
    let scope = ExposureScope::load(&connection, 91, &json!({})).expect("范围可读");
    assert_eq!(scope.grants.len(), 1, "同簇成员不能因为聚类就进入授权范围");
    assert_eq!(scope.grants[0].asset_id, Some(11));
    assert!(scope
        .excluded
        .iter()
        .any(|row| row.asset_id == 12 && row.reason == "not_exposure_eligible"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q6_four_thousand_items_run_to_terminal_state_and_twenty_thousand_are_planned() {
    let root = sandbox("scale");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(101,'范围','active')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status,stage) VALUES(6001,101,'queued','planned')",
            [],
        )
        .unwrap();
    let seeds: Vec<WorkItemSeed> = (0..4_000)
        .map(|index| WorkItemSeed {
            work_key: format!("{KIND_PASSIVE_WEB}|{index}"),
            kind: KIND_PASSIVE_WEB.to_string(),
            source_key: KIND_PASSIVE_WEB.to_string(),
            asset_id: None,
            origin: String::new(),
            entity: format!("item-{index}"),
            input_hash: index.to_string(),
        })
        .collect();
    plan(&connection, 6001, 101, 1, &seeds).expect("4,000 项必须能物化");
    // §E2：规模测试自己声明这一轮的工作项额度，冻结配置才是 run_plan 读取的上限来源。
    let frozen = super::settings::ExposureSettings::from_json(
        101,
        &json!({"exposureSources": ["passive_web"], "exposureMaxItems": 4_000}),
    )
    .expect("规模配置必须合法")
    .to_settings_json();
    connection
        .execute(
            "INSERT INTO exposure_run_plans(run_id,project_id,sources_json,budget_requests,budget_bytes,planned_items,settings_version,settings_json,options_hash)
             VALUES(6001,101,'[\"passive_web\"]',100000,100000000,4000,1,?1,?2)",
            params![frozen.to_string(), super::orchestrator::options_hash(&frozen)],
        )
        .unwrap();
    let handled = std::cell::Cell::new(0i64);
    let report = run_plan(
        &connection,
        6001,
        "owner-scale",
        &mut |item: &WorkItem| -> Result<ItemOutcome, String> {
            handled.set(handled.get() + 1);
            Ok(ItemOutcome {
                status: Some(if item.work_key.ends_with('7') {
                    WorkItemStatus::BlockedQuota
                } else {
                    WorkItemStatus::Completed
                }),
                reason: if item.work_key.ends_with('7') {
                    "额度".to_string()
                } else {
                    String::new()
                },
                source_status: Some(if item.work_key.ends_with('7') {
                    super::state::SourceStatus::Partial
                } else {
                    super::state::SourceStatus::Completed
                }),
                source_reason: String::new(),
                observed: 1,
                ..Default::default()
            })
        },
    )
    .expect("4,000 项必须跑到终态");
    assert_eq!(handled.get(), 4_000, "每项只被领取一次");
    assert_eq!(report.counts.total, 4_000);
    assert_eq!(report.counts.open, 0);
    assert_eq!(report.counts.quota, 400, "尾数 7 的 400 项必须是超额缺口");
    assert_eq!(report.aggregation.status, RunStatus::Partial);
    let gaps = enumerate_gaps(&connection, 6001, 5).unwrap();
    assert_eq!(gaps.len(), 5, "缺口枚举有界：{gaps:?}");

    // 20,000 行的计划：只物化并统计，不发请求，验证计划本身在规模上站得住。
    let seeds: Vec<WorkItemSeed> = (0..20_000)
        .map(|index| WorkItemSeed {
            work_key: format!("plan|{index}"),
            kind: "plan".to_string(),
            source_key: KIND_PASSIVE_WEB.to_string(),
            asset_id: None,
            origin: String::new(),
            entity: format!("p{index}"),
            input_hash: index.to_string(),
        })
        .collect();
    plan(&connection, 6001, 101, 2, &seeds).expect("20,000 项必须能物化");
    let counted = counts(&connection, 6001).unwrap();
    assert_eq!(counted.total, 24_000, "{counted:?}");
    let pages: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM exposure_work_items WHERE run_id=6001 ORDER BY work_key LIMIT 200)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(pages, 200, "有界分页读取必须可用");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn q6_inherited_rows_are_never_presented_as_direct_observations() {
    let rows = vec![
        member(1, "a.corp.invalid", "same"),
        member(2, "b.corp.invalid", "same"),
    ];
    let cluster = content_clusters(&rows).remove(0);
    let inherited = inheritance_for(&cluster, 2).expect("成员必须在簇里");
    assert!(!inherited.direct);
    assert!(
        inherited.note.contains("不代表本资产已被直接验证"),
        "{}",
        inherited.note
    );
}
