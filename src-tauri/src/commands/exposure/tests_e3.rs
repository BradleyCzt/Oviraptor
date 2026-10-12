// §E3 — the cluster graph has to be produced by the round itself: written between the
// observation commit and the item's terminal state, idempotent across a retry, failing closed
// instead of claiming inherited coverage, and readable page by page with the cache time of
// the bytes it groups. Nothing here authorizes anything.

use super::cache::{entry_count, lookup, lookup_guarded, store_expiring, CacheKey};
use super::cluster_repository::{
    page_cluster_members, page_clusters, persist_for_observations, read, EVIDENCE_DIRECT,
    EVIDENCE_INHERITED, EVIDENCE_STALE,
};
use super::fixture::seed_scope;
use super::observations::{FindingDraft, NewObservation};
use super::orchestrator::{run_plan, start_or_reuse, ItemOutcome, KIND_PASSIVE_WEB};
use super::settings::{save, ExposureSettings};
use super::state::{ensure_exposure_schema, SourceStatus, WorkItemStatus};
use super::work_items::{claim_next, counts, WorkItem};
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};
use std::path::{Path, PathBuf};

/// How many work items the plan materialized for this run.
fn report_total(connection: &Connection, run_id: i64) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap_or(0)
}

fn sandbox(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "oviraptor-exposure-e3-{tag}-{}",
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

fn settings(connection: &Connection, project: i64, items: i64) -> JsonValue {
    let mut value = ExposureSettings::for_project(project);
    value.sources = vec![KIND_PASSIVE_WEB.to_string()];
    value.max_items = items;
    value.max_requests = 1_000;
    value.authorization_days = 30;
    save(connection, &value, 0).expect("配置必须能保存");
    super::super::exposure_scan_settings(connection, project).expect("配置必须可读")
}

fn observation(item: &WorkItem, body: &str) -> NewObservation {
    NewObservation {
        asset_id: item.asset_id,
        body: body.as_bytes().to_vec(),
        body_bytes: body.len() as i64,
        status_code: 200,
        content_type: "text/plain".to_string(),
        current_accessible: Some(true),
        ..NewObservation::new(&item.source_key, "resource", &item.origin)
    }
}

fn draft(item: &WorkItem) -> FindingDraft {
    FindingDraft {
        category: "document".to_string(),
        title: format!("公开线索 {}", item.entity),
        url: item.origin.clone(),
        source_type: item.source_key.clone(),
        evidence: "示例路径 /public/report.txt".to_string(),
        severity: "medium".to_string(),
        confidence: 70,
        asset_id: item.asset_id,
        content_hash: String::new(),
    }
}

/// Two origins serve identical bytes, a third is alone on its host, and a fourth is the only
/// asset under another registered domain: the pass has to group the first two by bytes and
/// all three by domain, and leave the singleton out of both.
#[test]
fn e3_a_real_round_persists_clusters_and_a_cache_entry_without_any_manual_write() {
    let root = sandbox("round");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(161,'范围','active')",
            [],
        )
        .unwrap();
    for origin in [
        "http://a.e3.invalid/",
        "http://b.e3.invalid/",
        "http://c.e3.invalid/",
        "http://solo.other.invalid/",
    ] {
        seed_scope(&connection, 161, origin, true);
    }
    let started = start_or_reuse(&connection, 161, &settings(&connection, 161, 10)).expect("启动");
    let report = run_plan(
        &connection,
        started.run_id,
        "owner-e3",
        &mut |item: &WorkItem| -> Result<ItemOutcome, String> {
            // `c` shares the registered domain but serves different bytes, and is observed
            // without ever being classified: it may only inherit, never claim.
            let shared = !item.origin.contains("solo") && !item.origin.contains("c.e3");
            let classified = shared;
            // The unclassified assets each serve their own bytes, so only `a` and `b` group by
            // content while all three of them share one registered domain.
            let body = if shared {
                "same-bytes".to_string()
            } else {
                item.origin.clone()
            };
            Ok(ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                source_status: Some(SourceStatus::Completed),
                observed: 1,
                observations: vec![observation(item, &body)],
                findings: if classified {
                    vec![draft(item)]
                } else {
                    Vec::new()
                },
                ..Default::default()
            })
        },
    )
    .expect("真实一轮必须收敛");
    assert_eq!(report.counts.total, 4, "{:?}", report.counts);
    assert_eq!(report.counts.open, 0);

    let clusters = read(&connection, 161).expect("簇必须已持久化");
    let keys: Vec<String> = clusters.iter().map(|row| row.cluster_key.clone()).collect();
    assert_eq!(clusters.len(), 2, "簇键={keys:?}");
    let content = clusters
        .iter()
        .find(|row| row.kind == "content")
        .expect("相同字节必须成簇");
    let domain = clusters
        .iter()
        .find(|row| row.kind == "domain")
        .expect("同一注册域必须成簇");
    assert_eq!(content.members.len(), 2, "{:?}", content.members);
    assert_eq!(domain.members.len(), 3, "{:?}", domain.members);
    assert!(
        !keys.iter().any(|row| row.contains("solo")),
        "单例不能成簇：{keys:?}"
    );

    // Provenance: which observation, which round, which configuration version.
    let (representative_observation, representative_run, settings_version, rule_version, direct, inherited, gap): (
        Option<i64>, i64, i64, String, i64, i64, String,
    ) = connection
        .query_row(
            "SELECT representative_observation_id,representative_run_id,settings_version,rule_version,direct_count,inherited_count,gap_note \
             FROM exposure_clusters WHERE project_id=161 AND kind='content'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .unwrap();
    assert!(
        representative_observation.is_some(),
        "代表必须指回真实观察行"
    );
    assert_eq!(representative_run, started.run_id);
    assert_eq!(settings_version, 1, "冻结的配置版本必须记录");
    assert!(
        rule_version.starts_with("oviraptor.exposure.rules."),
        "{rule_version}"
    );
    assert_eq!(direct, 2, "两个成员都自带结论");
    assert_eq!(inherited, 0);
    assert!(gap.is_empty(), "无缺口时不该编造说明：{gap}");

    // The domain cluster inherits for `c`, which was observed but never classified.
    let domain_rows = page_clusters(&connection, 161, "domain", "", "", 10).expect("域簇分页");
    assert_eq!(domain_rows.items[0].direct_count, 2);
    assert_eq!(domain_rows.items[0].inherited_count, 1);
    let members =
        page_cluster_members(&connection, 161, domain_rows.items[0].id, "", 10).expect("成员分页");
    let inherited_member = members
        .items
        .iter()
        .find(|row| row.evidence_source == EVIDENCE_INHERITED)
        .expect("没有自带结论的成员必须是继承");
    assert_eq!(inherited_member.inherited_run_id, started.run_id);
    assert!(inherited_member.admitted_now, "仍准入的成员不该失效");
    assert_eq!(members.items[0].membership, "representative");
    assert_eq!(
        members
            .items
            .iter()
            .filter(|row| row.evidence_source == EVIDENCE_DIRECT)
            .count(),
        2
    );
    // A member that inherits says so instead of pretending it was verified.
    assert!(
        members
            .items
            .iter()
            .any(|row| row.evidence_source == EVIDENCE_INHERITED && row.verified_at.is_empty()),
        "{:?}",
        members.items
    );

    // The classification that was actually applied is now cached under this contract, and the
    // cluster view reports the time it was cached at.
    assert!(
        entry_count(&connection, 161).unwrap() >= 2,
        "本轮结论必须入缓存"
    );
    let page = page_clusters(&connection, 161, "content", "", "", 10).expect("簇分页");
    assert!(
        !page.items[0].cache_time.is_empty(),
        "同字节的簇应显示缓存时间"
    );
    assert!(
        !page.items[0].cache_expires_at.is_empty(),
        "有授权期限就要有期限"
    );
    assert_eq!(page.items[0].representative_observed_at.len(), 19);
    assert_eq!(page.total, 1);
    assert!(!page.has_more, "{:?}", page.next_cursor);
    let _ = std::fs::remove_dir_all(root);
}

/// The same pass over the same observations produces one graph, not a second copy of it:
/// a retry after a crash cannot inflate coverage or member rows.
#[test]
fn e3_the_cluster_pass_is_idempotent_when_a_round_repeats_it() {
    let root = sandbox("idempotent");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(163,'范围','active')",
            [],
        )
        .unwrap();
    for origin in ["http://x.e3.invalid/", "http://y.e3.invalid/"] {
        seed_scope(&connection, 163, origin, true);
    }
    let started = start_or_reuse(&connection, 163, &settings(&connection, 163, 10)).expect("启动");
    let ids: Vec<i64> = {
        let mut out = Vec::new();
        while let Some(item) = claim_next(&connection, started.run_id, "owner-idem", 60).unwrap() {
            let (id, _) = super::observations::record(
                &connection,
                started.run_id,
                163,
                Some(item.id),
                &observation(&item, "same-bytes"),
            )
            .expect("观察写入");
            out.push(id);
            super::work_items::finish(
                &connection,
                item.id,
                "owner-idem",
                item.claim_version,
                WorkItemStatus::Completed,
                "",
                1,
                0,
                0,
            )
            .expect("终态写入");
        }
        out
    };
    let first = persist_for_observations(&connection, started.run_id, 163, &ids, 1).expect("首次");
    let clusters_before: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_clusters WHERE project_id=163",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let members_before: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_cluster_members WHERE project_id=163",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let second = persist_for_observations(&connection, started.run_id, 163, &ids, 1).expect("重试");
    let clusters_after: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_clusters WHERE project_id=163",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let members_after: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_cluster_members WHERE project_id=163",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // Two assets with identical bytes on one registered domain group twice: by bytes and by name.
    assert_eq!(first.clusters, 2, "{first:?}");
    assert_eq!(second.clusters, first.clusters);
    assert_eq!(second.members, first.members);
    assert_eq!(clusters_before, clusters_after, "簇行不得重复累积");
    assert_eq!(members_before, members_after, "成员行不得重复累积");
    let _ = std::fs::remove_dir_all(root);
}

/// If the graph cannot be written, the item must not reach a terminal state that talks about
/// coverage, and the round must not be reported as completed.
#[test]
fn e3_a_failed_cluster_write_leaves_the_item_open_instead_of_a_coverage_claim() {
    let root = sandbox("failure");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(164,'范围','active')",
            [],
        )
        .unwrap();
    for origin in ["http://p.e3.invalid/", "http://q.e3.invalid/"] {
        seed_scope(&connection, 164, origin, true);
    }
    let started = start_or_reuse(&connection, 164, &settings(&connection, 164, 10)).expect("启动");
    // The write target is gone: the pass has to fail loudly.
    connection
        .execute(
            "ALTER TABLE exposure_clusters RENAME TO exposure_clusters_removed",
            [],
        )
        .unwrap();
    let error = run_plan(
        &connection,
        started.run_id,
        "owner-broken",
        &mut |item: &WorkItem| -> Result<ItemOutcome, String> {
            Ok(ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                source_status: Some(SourceStatus::Completed),
                observed: 1,
                observations: vec![observation(item, "same-bytes")],
                findings: vec![draft(item)],
                ..Default::default()
            })
        },
    )
    .expect_err("簇写不进去就不能假装这一轮收敛了");
    assert!(
        error.contains("簇写入失败")
            || error.contains("exposure_page_read_failed")
            || error.contains("exposure_clusters"),
        "失败原因必须来自簇持久化：{error}"
    );
    let open = counts(&connection, started.run_id).expect("计数").open;
    assert!(
        open > 0,
        "簇失败后仍有工作项未到终态，而不是被记成已覆盖：{open}"
    );
    let completed: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND status='completed'",
            [started.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        completed < report_total(&connection, started.run_id),
        "簇写失败后不得把整轮都记成完成：完成 {completed} 项"
    );
    let written: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_clusters_removed",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(written, 0, "簇表被改名后不该有任何簇结论");
    let status: String = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=?1",
            [started.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(status, "completed", "{status}");
    // The observation itself stays: a failed grouping never throws away what was seen.
    let kept: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_observations WHERE run_id=?1",
            [started.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(kept > 0, "证据必须保留");
    let _ = std::fs::remove_dir_all(root);
}

/// Paging, filtering and project isolation for the cluster view, plus the revocation label.
#[test]
fn e3_cluster_pages_are_paged_filtered_and_isolated_and_show_revoked_members() {
    let root = sandbox("paging");
    let connection = database(&root);
    for project in [165i64, 166i64] {
        connection
            .execute(
                "INSERT INTO projects(id,name,status) VALUES(?1,?2,'active')",
                params![project, format!("范围{project}")],
            )
            .unwrap();
    }
    // 12 content clusters in project 165: two admitted assets sharing bytes per hash.
    let mut ids = Vec::new();
    for index in 0..12 {
        for slot in 0..2 {
            let asset = index * 2 + slot;
            let origin = format!("http://h{asset}.d{asset}.invalid/Shared{}%2FPath.js", index);
            seed_scope(&connection, 165, &origin, true);
            let asset_id: i64 = connection
                .query_row(
                    "SELECT id FROM assets WHERE asset_key=?1",
                    [&origin],
                    |row| row.get(0),
                )
                .unwrap();
            // A run row is required by the observation's foreign key.
            connection
                .execute(
                    "INSERT OR IGNORE INTO exposure_runs(id,project_id,status) VALUES(?1,165,'completed')",
                    params![10000 + asset],
                )
                .unwrap();
            let (id, _) = super::observations::record(
                &connection,
                10000 + asset,
                165,
                None,
                &NewObservation {
                    asset_id: Some(asset_id),
                    body: format!("bytes-{index}").as_bytes().to_vec(),
                    body_bytes: 8,
                    ..NewObservation::new(KIND_PASSIVE_WEB, "resource", &origin)
                },
            )
            .expect("观察写入");
            ids.push(id);
        }
    }
    let pass = persist_for_observations(&connection, 10000, 165, &ids, 1).expect("批量簇写入");
    assert_eq!(pass.clusters, 12, "{pass:?}");
    let page = page_clusters(&connection, 165, "", "", "", 10).expect("第一页");
    assert_eq!(page.total, 12, "过滤总数必须是全量口径");
    assert_eq!(page.items.len(), 10);
    assert!(page.has_more);
    assert!(!page.next_cursor.is_empty());
    let next = page_clusters(&connection, 165, "", "", &page.next_cursor, 10).expect("第二页");
    assert_eq!(next.items.len(), 2);
    assert!(!next.has_more);
    assert_eq!(next.total, 12);
    let ids_first: Vec<i64> = page.items.iter().map(|row| row.id).collect();
    let ids_second: Vec<i64> = next.items.iter().map(|row| row.id).collect();
    assert!(
        ids_first.iter().all(|row| !ids_second.contains(row)),
        "两页不得重叠"
    );
    // Search hits the cluster key, and an unknown kind is refused instead of ignored.
    let searched = page_clusters(&connection, 165, "", "d0.invalid", "", 10).expect("搜索分页");
    assert_eq!(searched.total, 1, "{:?}", searched.items);
    let broken =
        page_clusters(&connection, 165, "asset", "", "", 10).expect_err("未知簇类型必须拒绝");
    assert_eq!(broken.code, "exposure_cluster_kind_invalid");
    let huge = page_clusters(&connection, 165, "", "", "", 500)
        .expect_err("页长超界必须拒绝，不能当成全量");
    assert_eq!(huge.code, "exposure_page_size_invalid");
    let tampered = page_clusters(&connection, 165, "", "", "@@bad@@", 10)
        .expect_err("损坏游标必须报错，不能静默回到第一页");
    assert_eq!(tampered.code, "exposure_cursor_invalid");
    // Another project sees nothing of this one, and cannot read its members.
    let other = page_clusters(&connection, 166, "", "", "", 10).expect("跨项目分页");
    assert_eq!(other.total, 0);
    let foreign = page_cluster_members(&connection, 166, page.items[0].id, "", 10)
        .expect_err("跨项目读成员必须拒绝");
    assert_eq!(foreign.code, "exposure_cluster_not_found");

    // Revocation: the member stops inheriting immediately, with its own labelled reason.
    let cluster_id = page.items[0].id;
    let before = page_cluster_members(&connection, 165, cluster_id, "", 10).expect("成员");
    assert!(before.items.iter().all(|row| row.admitted_now));
    let revoked_asset = before.items[0].asset_id;
    connection
        .execute(
            "UPDATE asset_ownership_decisions SET exposure_eligible=0, authorization_status='revoked' WHERE asset_id=?1",
            [revoked_asset],
        )
        .unwrap();
    let after = page_cluster_members(&connection, 165, cluster_id, "", 10).expect("撤权后成员");
    let row = after
        .items
        .iter()
        .find(|row| row.asset_id == revoked_asset)
        .expect("成员仍在图上，历史不删");
    assert!(!row.admitted_now, "撤权成员必须立刻标为不再准入");
    let _ = std::fs::remove_dir_all(root);
}

/// A cached verdict is only ever the answer to the exact question it was written for, and it
/// never stands in for permission to fetch.
#[test]
fn e3_the_cache_never_answers_for_another_scope_source_url_or_expired_window() {
    let root = sandbox("cache");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(167,'范围','active')",
            [],
        )
        .unwrap();
    let hash = "a".repeat(64);
    let base = CacheKey::new(167, 3, "public", &hash, "classification").with_identity(
        KIND_PASSIVE_WEB,
        "https://x.e3.invalid/Secret.Js?V=1",
        2,
    );
    assert!(
        lookup(&connection, &base).unwrap().is_none(),
        "首次必须未命中"
    );
    store_expiring(
        &connection,
        &base,
        &json!({"category":"document","provenance":"deterministic-rule-classification"})
            .to_string(),
        &super::state::shift_seconds_forward(3_600),
    )
    .expect("写入");
    let hit = lookup(&connection, &base)
        .expect("命中")
        .expect("未过期必须命中");
    assert_eq!(hit.hits, 1);
    assert!(hit.stored_at.len() >= 19, "{:?}", hit.stored_at);
    assert!(!hit.expires_at.is_empty(), "命中必须带期限");

    // Same bytes, different identity dimension: every one of them is a miss.
    let mut moved_scope = base.clone();
    moved_scope.scope_version = 4;
    assert!(
        lookup(&connection, &moved_scope).unwrap().is_none(),
        "范围版本变了不能串用"
    );
    let mut other_source = base.clone();
    other_source.source_key = "certificate_transparency".to_string();
    assert!(
        lookup(&connection, &other_source).unwrap().is_none(),
        "来源不同不能串用"
    );
    let mut other_case = base.clone();
    other_case.canonical_url = "https://x.e3.invalid/secret.Js?V=1".to_string();
    assert!(
        lookup(&connection, &other_case).unwrap().is_none(),
        "URL 路径大小写不得整体折叠"
    );
    let mut other_settings = base.clone();
    other_settings.settings_version = 3;
    assert!(
        lookup(&connection, &other_settings).unwrap().is_none(),
        "配置版本变了不能串用"
    );
    let mut other_project = base.clone();
    other_project.project_id = 168;
    assert!(
        lookup(&connection, &other_project).unwrap().is_none(),
        "内容相同也不得跨工作空间命中"
    );

    // Revocation wins over a hit, and does not even count as reuse.
    let hits_before: i64 = connection
        .query_row(
            "SELECT hits FROM exposure_analysis_cache WHERE cache_key=?1",
            [base.as_text()],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        lookup_guarded(&connection, &base, || false)
            .expect("撤权读取不得报错")
            .is_none(),
        "撤权后缓存不能回答"
    );
    let hits_after: i64 = connection
        .query_row(
            "SELECT hits FROM exposure_analysis_cache WHERE cache_key=?1",
            [base.as_text()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hits_before, hits_after, "被拒绝的命中不能算作复用");

    // Expiry: the row stays as history but answers nothing.
    let expired = CacheKey::new(167, 3, "public", &hash, "classification").with_identity(
        KIND_PASSIVE_WEB,
        "https://x.e3.invalid/expired",
        2,
    );
    store_expiring(
        &connection,
        &expired,
        &json!({"category":"document"}).to_string(),
        "2000-01-01 00:00:00",
    )
    .expect("写入过期项");
    assert!(
        lookup(&connection, &expired).expect("过期读取").is_none(),
        "过期条目必须按未命中处理"
    );
    let counted: i64 = connection
        .query_row(
            "SELECT hits FROM exposure_analysis_cache WHERE cache_key=?1",
            [expired.as_text()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(counted, 0, "过期条目不得被记为复用");
    // Another project's rows are never counted into this one's entry total.
    assert_eq!(entry_count(&connection, 167).unwrap(), 2);
    assert_eq!(entry_count(&connection, 168).unwrap(), 0);
    let _ = std::fs::remove_dir_all(root);
}

/// A member that lost admission is a named gap on the cluster, and its own row stops being
/// presented as direct evidence.
#[test]
fn e3_a_member_that_lost_admission_becomes_a_gap_instead_of_inherited_coverage() {
    let root = sandbox("gap");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(169,'范围','active')",
            [],
        )
        .unwrap();
    for origin in ["http://m.e3.invalid/", "http://n.e3.invalid/"] {
        seed_scope(&connection, 169, origin, true);
    }
    let started = start_or_reuse(&connection, 169, &settings(&connection, 169, 10)).expect("启动");
    run_plan(
        &connection,
        started.run_id,
        "owner-gap",
        &mut |item: &WorkItem| -> Result<ItemOutcome, String> {
            Ok(ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                source_status: Some(SourceStatus::Completed),
                observed: 1,
                observations: vec![observation(item, "same-bytes")],
                findings: vec![draft(item)],
                ..Default::default()
            })
        },
    )
    .expect("首轮收敛");
    let page = page_clusters(&connection, 169, "content", "", "", 10).expect("簇分页");
    assert_eq!(page.items[0].inherited_count, 0);
    assert!(page.items[0].gap_note.is_empty());

    connection
        .execute(
            "UPDATE asset_ownership_decisions SET exposure_eligible=0, authorization_status='revoked' WHERE project_id=169",
            [],
        )
        .unwrap();
    let ids: Vec<i64> = connection
        .prepare("SELECT id FROM exposure_observations WHERE project_id=169")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let pass =
        persist_for_observations(&connection, started.run_id, 169, &ids, 1).expect("重新分组");
    assert!(pass.dropped_members > 0, "{pass:?}");
    let refreshed = page_clusters(&connection, 169, "content", "", "", 10).expect("刷新后的簇");
    assert_eq!(refreshed.items[0].direct_count, 0, "撤权后不得再有直接依据");
    assert!(
        refreshed.items[0].gap_note.contains("不再满足准入"),
        "{}",
        refreshed.items[0].gap_note
    );
    let members =
        page_cluster_members(&connection, 169, refreshed.items[0].id, "", 10).expect("成员");
    assert!(
        members
            .items
            .iter()
            .all(|row| row.evidence_source == EVIDENCE_STALE && !row.admitted_now),
        "{:?}",
        members.items
    );
    // The history is not deleted: the cluster row is still there, just no longer claiming.
    assert_eq!(members.total, 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn e3_a_lone_observation_forms_no_cluster_at_all() {
    let root = sandbox("singleton");
    let connection = database(&root);
    connection
        .execute(
            "INSERT INTO projects(id,name,status) VALUES(171,'范围','active')",
            [],
        )
        .unwrap();
    seed_scope(&connection, 171, "http://only.e3.invalid/", true);
    connection
        .execute(
            "INSERT INTO exposure_runs(id,project_id,status) VALUES(17100,171,'completed')",
            [],
        )
        .unwrap();
    let asset_id: i64 = connection
        .query_row(
            "SELECT id FROM assets WHERE asset_key='http://only.e3.invalid/'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let (id, _) = super::observations::record(
        &connection,
        17100,
        171,
        None,
        &NewObservation {
            asset_id: Some(asset_id),
            body: b"lonely".to_vec(),
            body_bytes: 6,
            ..NewObservation::new(KIND_PASSIVE_WEB, "resource", "http://only.e3.invalid/")
        },
    )
    .expect("观察写入");
    let pass =
        persist_for_observations(&connection, 17100, 171, &[id], 1).expect("单独一次观察不该失败");
    assert_eq!(pass.clusters, 0, "单个成员不成簇：{pass:?}");
    assert!(read(&connection, 171).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(root);
}
