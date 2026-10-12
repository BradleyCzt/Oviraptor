// G4-4 scale tiers on temporary databases only (4 000 / 20 000 / 100 000). One tier per
// process: `G4_SCALE_TIER=<n> cargo test --release --lib tests_g4_scale -- --ignored`.
// Seeds the production table shapes in bulk, then measures plan build, cancel response,
// resume re-execution, proposals, summary/coverage/unit statistics, keyset pages and exports.
// Never touches a real user database: everything lives under the OS temp directory.

use super::incidents::{page_incidents, propose};
use super::orchestrator::{run_plan, start_or_reuse, ItemOutcome};
use super::settings::{save, ExposureSettings};
use super::state::ensure_exposure_schema;
use rusqlite::{params, Connection};
use serde_json::{json, Value as JsonValue};
use std::time::Instant;

fn ms(start: Instant) -> f64 {
    (start.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0
}

fn max_rss_bytes() -> i64 {
    // SAFETY: getrusage only writes into the struct we pass.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    if rc != 0 {
        return -1;
    }
    // macOS reports bytes, Linux kilobytes.
    if cfg!(target_os = "macos") {
        usage.ru_maxrss
    } else {
        usage.ru_maxrss * 1024
    }
}

fn file_size(path: &std::path::Path) -> i64 {
    std::fs::metadata(path).map(|meta| meta.len() as i64).unwrap_or(0)
}

/// Assets, decisions, one seed round with one observation + finding + disposition per asset.
/// Every 10 assets: a pair with the same full body at two URLs (pending), three unique risk
/// leads, one excluded public material, five unique pending leads.
fn seed(connection: &Connection, project_id: i64, n: i64) -> i64 {
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    connection
        .execute("INSERT INTO projects(id,name,status) VALUES(?1,'规模测试','active')", [project_id])
        .unwrap();
    connection
        .execute("INSERT INTO asset_ownership_profiles(project_id,legal_name,policy_version) VALUES(?1,'样本公司',1)", [project_id])
        .unwrap();
    connection
        .execute("INSERT INTO exposure_runs(project_id,status,stage,completed_at) VALUES(?1,'completed','completed','2026-10-09 00:00:00')", [project_id])
        .unwrap();
    let run_id = connection.last_insert_rowid();
    {
        let mut asset = connection.prepare("INSERT INTO assets(asset_key,link,host) VALUES(?1,?1,?2)").unwrap();
        let mut member = connection.prepare("INSERT INTO project_assets(project_id,asset_id) VALUES(?1,?2)").unwrap();
        let mut decision = connection
            .prepare("INSERT INTO asset_ownership_decisions(project_id,asset_id,status,authorization_status,exposure_eligible,confidence) VALUES(?1,?2,'confirmed','allowed',1,90)")
            .unwrap();
        let mut observation = connection
            .prepare(
                "INSERT INTO exposure_observations(run_id,project_id,source_key,asset_id,kind,url,canonical_url,status_code,content_hash,body_bytes,truncated,evidence_json)
                 VALUES(?1,?2,'passive_web',?3,'page',?4,?4,200,?5,4096,0,?6)",
            )
            .unwrap();
        let mut finding = connection
            .prepare(
                "INSERT INTO exposure_findings(project_id,asset_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,status,fingerprint)
                 VALUES(?1,?2,'secret',?3,'passive_web',?4,?5,?6,?7,'new',?8)",
            )
            .unwrap();
        let mut link = connection
            .prepare("INSERT INTO exposure_finding_observations(finding_id,observation_id,run_id,project_id) VALUES(?1,?2,?3,?4)")
            .unwrap();
        let mut disposition = connection
            .prepare(
                "INSERT INTO exposure_finding_dispositions(finding_id,project_id,disposition,info_category,source_channel,reason,policy_version,classification_id,updated_at)
                 VALUES(?1,?2,?3,?4,'first_party_web','规模测试','scale',0,'2026-10-09 00:00:00')",
            )
            .unwrap();
        let excerpt = "样本公司 内部接口 ".repeat(20);
        for index in 0..n {
            let origin = format!("https://h{index}.scale.example.test");
            asset.execute(params![origin, format!("h{index}.scale.example.test")]).unwrap();
            let asset_id = connection.last_insert_rowid();
            member.execute(params![project_id, asset_id]).unwrap();
            decision.execute(params![project_id, asset_id]).unwrap();
            let slot = index % 10;
            let (body, kind, category, severity) = match slot {
                0 | 1 => (format!("dup{}", index / 10), "classification_pending", "internal_business", "medium"),
                2..=4 => (format!("risk{index}"), "risk_candidate", "credential", "high"),
                5 => (format!("pub{index}"), "excluded_public_material", "unclassified", "low"),
                _ => (format!("pend{index}"), "classification_pending", "technical_network", "low"),
            };
            let url = format!("{origin}/file-{slot}.txt");
            let evidence = json!({"detail": {"collectionMode": "http_read", "excerpt": excerpt}}).to_string();
            observation.execute(params![run_id, project_id, asset_id, url, body, evidence]).unwrap();
            let observation_id = connection.last_insert_rowid();
            finding
                .execute(params![project_id, asset_id, format!("线索 {slot}"), url, excerpt, severity, 40 + slot, format!("fp-{index}")])
                .unwrap();
            let finding_id = connection.last_insert_rowid();
            link.execute(params![finding_id, observation_id, run_id, project_id]).unwrap();
            disposition.execute(params![finding_id, project_id, kind, category]).unwrap();
        }
    }
    connection.execute_batch("COMMIT").unwrap();
    run_id
}

fn page_findings(connection: &Connection, project_id: i64, cursor: &str) -> (f64, String, i64) {
    let start = Instant::now();
    let input = super::contract::ExposureFindingPageInput { project_id, page_size: 50, cursor: cursor.to_string(), ..Default::default() };
    let query = input.normalize().unwrap();
    let page = super::super::finding_page_view(connection, &query).unwrap();
    let elapsed = ms(start);
    let value = serde_json::to_value(&page).unwrap();
    (elapsed, value["nextCursor"].as_str().unwrap_or_default().to_string(), value["total"].as_i64().unwrap_or(-1))
}

#[test]
#[ignore]
fn g4_4_scale_tier() {
    let n: i64 = std::env::var("G4_SCALE_TIER").ok().and_then(|value| value.parse().ok()).unwrap_or(4000);
    let root = std::env::temp_dir().join(format!("oviraptor-exposure-g4-scale-{n}-{}", uuid::Uuid::new_v4()));
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    assert!(db_path.starts_with(std::env::temp_dir()), "只用临时库");
    let connection = crate::db::open(&db_path).unwrap();
    ensure_exposure_schema(&connection).unwrap();
    let pid = 900;
    let mut out = serde_json::Map::new();
    out.insert("tier".into(), json!(n));
    out.insert("profile".into(), json!(if cfg!(debug_assertions) { "debug" } else { "release" }));
    let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0)).unwrap();
    let journal: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
    out.insert("pragma".into(), json!({"synchronous": synchronous, "journalMode": journal}));

    let start = Instant::now();
    seed(&connection, pid, n);
    out.insert("seedMs".into(), json!(ms(start)));

    // Degradation notice with default settings: the plan is capped and the rest is visible.
    let summary = super::summary::exposure_scope_summary(&connection, pid).unwrap();
    out.insert("defaultCap".into(), json!({"planned": summary.eligible_assets_planned, "beyondPlan": summary.assets_beyond_plan}));
    assert_eq!(summary.eligible_assets_planned + summary.assets_beyond_plan, n);

    let mut settings = ExposureSettings::for_project(pid);
    settings.sources = vec!["passive_web".into()];
    settings.max_items = n;
    save(&connection, &settings, 0).unwrap();
    let frozen = super::super::exposure_scan_settings(&connection, pid).unwrap();
    let start = Instant::now();
    let started = start_or_reuse(&connection, pid, &frozen).unwrap();
    out.insert("planBuildMs".into(), json!(ms(start)));
    let planned: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1", [started.run_id], |row| row.get(0)).unwrap();
    out.insert("plannedItems".into(), json!(planned));

    // Cancel response: a second connection asks to cancel after K items.
    let k = (n / 10).clamp(50, 500);
    let canceller = crate::db::open(&db_path).unwrap();
    let mut calls = 0i64;
    let mut cancel_at: Option<Instant> = None;
    let mut calls_at_cancel = 0i64;
    let start = Instant::now();
    run_plan(&connection, started.run_id, "scale-worker", &mut |_item| {
        calls += 1;
        if calls == k {
            canceller.execute("UPDATE exposure_runs SET cancel_requested=1 WHERE id=?1", [started.run_id]).unwrap();
            cancel_at = Some(Instant::now());
            calls_at_cancel = calls;
        }
        Ok(ItemOutcome::default())
    })
    .unwrap();
    let cancel_ms = cancel_at.map(ms).unwrap_or(-1.0);
    let first_leg_ms = ms(start);
    let completed_before: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND status='completed'", [started.run_id], |row| row.get(0))
        .unwrap();
    out.insert("cancel".into(), json!({"requestedAfterItems": k, "responseMs": cancel_ms, "itemsAfterCancel": calls - calls_at_cancel, "firstLegMs": first_leg_ms, "completed": completed_before}));
    assert!(calls - calls_at_cancel <= 1, "取消后最多再处理一个工作项");

    // Resume: continue the same frozen plan; completed items are not executed again.
    connection.execute("UPDATE exposure_runs SET status='running',cancel_requested=0 WHERE id=?1", [started.run_id]).unwrap();
    let mut resumed = 0i64;
    let start = Instant::now();
    run_plan(&connection, started.run_id, "scale-worker-2", &mut |_item| {
        resumed += 1;
        Ok(ItemOutcome::default())
    })
    .unwrap();
    out.insert("resume".into(), json!({"reExecutedItems": resumed, "expected": planned - completed_before, "completedRereadItems": resumed - (planned - completed_before), "drainMs": ms(start)}));
    assert_eq!(resumed, planned - completed_before, "恢复不重跑已完成项");

    let start = Instant::now();
    let report = propose(&connection, pid, None).unwrap();
    out.insert("proposeAllMs".into(), json!(ms(start)));
    assert_eq!(report.created, n / 10 + 3 * n / 10, "{report:?}");
    out.insert("incidents".into(), json!({"created": report.created, "locations": report.attached_locations}));

    let start = Instant::now();
    let summary = super::summary::exposure_scope_summary(&connection, pid).unwrap();
    let summary_cold = ms(start);
    let start = Instant::now();
    let summary = {
        let _ = summary;
        super::summary::exposure_scope_summary(&connection, pid).unwrap()
    };
    let summary_hot = ms(start);
    assert!(summary.coverage.is_some() && summary.unit_statistics.is_some(), "{:?} {:?}", summary.coverage_error, summary.unit_statistics_error);
    assert_eq!(summary.unit_statistics.as_ref().unwrap()["groupUnique"]["open"], n / 10 + 3 * n / 10);
    let start = Instant::now();
    super::coverage::coverage_matrix(&connection, pid, summary.unit_projection.as_ref(), 0).unwrap();
    let coverage_ms = ms(start);
    let start = Instant::now();
    super::coverage::unit_statistics(&connection, pid, summary.unit_projection.as_ref()).unwrap();
    let units_ms = ms(start);
    let start = Instant::now();
    super::live::live_run(&connection, pid).unwrap();
    let live_ms = ms(start);
    out.insert("summary".into(), json!({"coldMs": summary_cold, "hotMs": summary_hot, "coverageMs": coverage_ms, "unitStatisticsMs": units_ms, "liveRunMs": live_ms}));

    let (first, mut cursor, total) = page_findings(&connection, pid, "");
    assert_eq!(total, n);
    let (hot, _, _) = page_findings(&connection, pid, "");
    let mut later = Vec::new();
    for _ in 0..5 {
        let (elapsed, next, _) = page_findings(&connection, pid, &cursor);
        later.push(elapsed);
        cursor = next;
    }
    let later_max = later.iter().cloned().fold(0.0, f64::max);
    out.insert("findingPages".into(), json!({"firstMs": first, "hotFirstMs": hot, "subsequentMs": later, "subsequentMaxMs": later_max, "targetsMet": hot <= 2000.0 && later_max <= 1000.0}));

    let start = Instant::now();
    let page = page_incidents(&connection, pid, "", false, 50, "").unwrap();
    let incidents_first = ms(start);
    let start = Instant::now();
    let page_hot = page_incidents(&connection, pid, "", false, 50, "").unwrap();
    let incidents_hot = ms(start);
    let _ = page_hot;
    let mut cursor = page["nextCursor"].as_str().unwrap().to_string();
    let mut incident_later = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let next = page_incidents(&connection, pid, "", false, 50, &cursor).unwrap();
        incident_later.push(ms(start));
        cursor = next["nextCursor"].as_str().unwrap_or_default().to_string();
    }
    let incident_later_max = incident_later.iter().cloned().fold(0.0, f64::max);
    out.insert("incidentPages".into(), json!({"firstMs": incidents_first, "hotFirstMs": incidents_hot, "subsequentMs": incident_later, "subsequentMaxMs": incident_later_max, "total": page["total"], "targetsMet": incidents_hot <= 2000.0 && incident_later_max <= 1000.0}));

    let exports = root.join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let now = chrono::Local::now();
    let start = Instant::now();
    let (findings_path, rows) = super::super::export_exposure_in(&connection, &exports, pid, "findings", now).unwrap();
    let findings_ms = ms(start);
    assert_eq!(rows as i64, n);
    let verified = super::export::verify_written(&findings_path).unwrap();
    let start = Instant::now();
    let (ledger_path, ledger_rows) = super::super::export_exposure_in(&connection, &exports, pid, "ledger", now).unwrap();
    let ledger_ms = ms(start);
    let ledger_check = super::export::verify_written(&ledger_path).unwrap();
    assert_eq!(ledger_check["rowKinds"]["incident"], n / 10 + 3 * n / 10);
    out.insert("exports".into(), json!({
        "findingsMs": findings_ms, "findingsRows": rows, "findingsBytes": file_size(&findings_path), "findingsSha256": verified["sha256"],
        "ledgerMs": ledger_ms, "ledgerRows": ledger_rows, "ledgerBytes": file_size(&ledger_path), "ledgerRowKinds": ledger_check["rowKinds"],
    }));

    let evidence: i64 = connection.query_row("SELECT COALESCE(SUM(LENGTH(evidence_json)),0) FROM exposure_observations", [], |row| row.get(0)).unwrap();
    let mut wal = db_path.clone().into_os_string();
    wal.push("-wal");
    out.insert("storage".into(), json!({"dbBytes": file_size(&db_path), "walBytes": file_size(std::path::Path::new(&wal)), "evidenceJsonBytes": evidence}));
    out.insert("maxRssBytes".into(), json!(max_rss_bytes()));
    let line = JsonValue::Object(out).to_string();
    println!("G4_SCALE_RESULT {line}");
    if let Ok(path) = std::env::var("G4_SCALE_OUT") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        writeln!(file, "{line}").unwrap();
    }
    drop(canceller);
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// P1 验收（2026-10-09）：加入归属模块逐资产投影后的规模验证。集团台账绑定 + 4/5 资产有组确认
/// 的单位依据，测投影、汇总（含投影）、事件建议（逐资产自动关联单位）、事件/位置分页（第 51 条
/// 以后）、拆分/合并、覆盖钻取（时间窗口 × 单位）。工作项状态用 SQL 直接收口（不发网络，不跑
/// 21 分钟的合成执行器）；测的是读写路径本身。
#[test]
#[ignore]
fn rem_scale_tier_with_per_asset_projection() {
    let n: i64 = std::env::var("REM_SCALE_TIER").ok().and_then(|value| value.parse().ok()).unwrap_or(4000);
    let root = std::env::temp_dir().join(format!("oviraptor-exposure-rem-scale-{n}-{}", uuid::Uuid::new_v4()));
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    assert!(db_path.starts_with(std::env::temp_dir()), "只用临时库");
    let connection = crate::db::open(&db_path).unwrap();
    ensure_exposure_schema(&connection).unwrap();
    let pid = 901;
    let mut out = serde_json::Map::new();
    out.insert("tier".into(), json!(n));
    out.insert("profile".into(), json!(if cfg!(debug_assertions) { "debug" } else { "release" }));
    let total_start = Instant::now();

    let start = Instant::now();
    let run_id = seed(&connection, pid, n);
    // 一个 200 处传播位置的大事件：同一份字节出现在 200 个资产上。
    // REM_SCALE_SKIP_FULL_PROPOSE=1：大事件放在单独一轮里，只对这一轮做建议（10 万档全量 propose 另计）。
    let skip_full = std::env::var("REM_SCALE_SKIP_FULL_PROPOSE").as_deref() == Ok("1");
    let big_run;
    {
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        big_run = if skip_full {
            connection
                .execute("INSERT INTO exposure_runs(project_id,status,stage,completed_at) VALUES(?1,'completed','completed','2026-10-09 00:00:00')", [pid])
                .unwrap();
            connection.last_insert_rowid()
        } else {
            run_id
        };
        let excerpt = "样本公司 内部接口 ".repeat(20);
        let evidence = json!({"detail": {"collectionMode": "http_read", "excerpt": excerpt}}).to_string();
        for index in 0..200i64 {
            let asset_id: i64 = connection.query_row("SELECT id FROM assets WHERE asset_key=?1", [format!("https://h{index}.scale.example.test")], |row| row.get(0)).unwrap();
            let url = format!("https://h{index}.scale.example.test/big/leak.txt");
            connection
                .execute(
                    "INSERT INTO exposure_observations(run_id,project_id,source_key,asset_id,kind,url,canonical_url,status_code,content_hash,body_bytes,truncated,evidence_json)
                     VALUES(?1,?2,'passive_web',?3,'page',?4,?4,200,'bigdup',4096,0,?5)",
                    params![big_run, pid, asset_id, url, evidence],
                )
                .unwrap();
            let observation = connection.last_insert_rowid();
            connection
                .execute(
                    "INSERT INTO exposure_findings(project_id,asset_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,status,fingerprint)
                     VALUES(?1,?2,'secret','大事件','passive_web',?3,?4,'high',80,'new',?5)",
                    params![pid, asset_id, url, excerpt, format!("big-{index}")],
                )
                .unwrap();
            let finding = connection.last_insert_rowid();
            connection
                .execute("INSERT INTO exposure_finding_observations(finding_id,observation_id,run_id,project_id) VALUES(?1,?2,?3,?4)", params![finding, observation, big_run, pid])
                .unwrap();
            connection
                .execute(
                    "INSERT INTO exposure_finding_dispositions(finding_id,project_id,disposition,info_category,source_channel,reason,policy_version,classification_id,updated_at)
                     VALUES(?1,?2,'risk_candidate','credential','first_party_web','规模测试','scale',0,'2026-10-09 00:00:00')",
                    params![finding, pid],
                )
                .unwrap();
        }
        // 组织目录 + 集团台账绑定 + 组确认（4 个单位轮流，每 5 个资产留 1 个没有单位依据）。
        connection.execute("INSERT OR REPLACE INTO org_directory_state(id,version) VALUES(1,5)", []).unwrap();
        for (id, kind, name, parent, region) in [
            (1, "group", "中国移动通信集团有限公司", None, ""),
            (2, "province_company", "中国移动通信集团浙江有限公司", Some(1), "330000"),
            (3, "professional_subsidiary", "咪咕文化科技有限公司", Some(1), ""),
            (4, "city_branch", "咪咕浙江杭州分公司", Some(3), "330100"),
            (5, "city_branch", "中国移动杭州分公司", Some(2), "330100"),
            (6, "county_branch", "中国移动余杭分公司", Some(5), "330110"),
        ] {
            connection
                .execute(
                    "INSERT INTO org_entities(id,group_id,parent_id,entity_kind,legal_name,normalized_name,region_code,directory_version) VALUES(?1,?2,?3,?4,?5,?6,?7,1)",
                    params![id, if id == 1 { None } else { Some(1) }, parent, kind, name, format!("{name}-{id}"), region],
                )
                .unwrap();
        }
        connection
            .execute("INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,binding_version) VALUES(?1,'group_ledger',1,3)", [pid])
            .unwrap();
        connection
            .execute(
                "INSERT INTO ownership_analysis_runs(project_id,status,scope_version,data_revision,policy_version,directory_version,binding_version,algorithm_version) VALUES(?1,'completed','s','d',1,5,3,'a')",
                [pid],
            )
            .unwrap();
        let analysis = connection.last_insert_rowid();
        for (slot, entity) in [(0i64, 2i64), (1, 4), (2, 5), (3, 6)] {
            connection
                .execute(
                    "INSERT INTO ownership_review_groups(run_id,project_id,group_key,lane,nav_bucket,candidate_entity_id,candidate_source,relation_kind,boundary_kind,support_kind,decision_status,decision_source,authorization_status,exposure_eligible) VALUES(?1,?2,?3,'consistent','province',?4,'directory','official_host','exact_host','directory_official_host','auto','auto','unknown',0)",
                    params![analysis, pid, format!("scale-{slot}"), entity],
                )
                .unwrap();
            let group = connection.last_insert_rowid();
            let op = format!("op-scale-group-{slot}");
            connection
                .execute(
                    "INSERT INTO ownership_operations(operation_id,project_id,run_id,group_id,kind,action,status,payload_hash,expected_action_hash,result_json) VALUES(?1,?2,?3,?4,'group_decision','confirm_ownership','completed','h','h','{}')",
                    params![op, pid, analysis, group],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO ownership_operation_items(operation_id,asset_id,outcome) SELECT ?1,asset_id,'applied' FROM project_assets WHERE project_id=?2 AND (asset_id % 5)=?3",
                    params![op, pid, slot],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO asset_events(project_id,asset_id,event_type,summary) SELECT ?1,asset_id,'ownership',?2 FROM project_assets WHERE project_id=?1 AND (asset_id % 5)=?3",
                    params![pid, format!("confirmed: 组确认（复核组 #{group}，快照 #{analysis}，操作 {op}）"), slot],
                )
                .unwrap();
        }
        connection.execute("UPDATE asset_ownership_decisions SET source='manual' WHERE project_id=?1", [pid]).unwrap();
        connection.execute_batch("COMMIT").unwrap();
    }
    out.insert("seedMs".into(), json!(ms(start)));

    let start = Instant::now();
    let projection = crate::commands::ownership_asset_unit_projection(&connection, pid).unwrap();
    let projection_cold = ms(start);
    let start = Instant::now();
    let projection_hot = crate::commands::ownership_asset_unit_projection(&connection, pid).unwrap();
    let projection_hot_ms = ms(start);
    assert_eq!(projection.edges.len(), projection_hot.edges.len());
    let expected_edges: i64 = connection.query_row("SELECT COUNT(*) FROM project_assets WHERE project_id=?1 AND (asset_id % 5)<4", [pid], |row| row.get(0)).unwrap();
    assert_eq!(projection.edges.len() as i64, expected_edges, "4/5 资产有单位依据");
    let start = Instant::now();
    let one = crate::commands::ownership_asset_unit_edge(&connection, pid, projection.edges[projection.edges.len() / 2].asset_id).unwrap();
    let single_edge_ms = ms(start);
    assert!(one.is_some());
    out.insert("projection".into(), json!({"coldMs": projection_cold, "hotMs": projection_hot_ms, "edges": projection.edges.len(), "unassigned": projection.unassigned, "singleAssetEdgeMs": single_edge_ms}));
    // 诊断：REM_SCALE_KEEP_DB=<路径> 把建议之前的临时库检查点后复制出去，供 EXPLAIN / 计时定位热点。
    if let Ok(keep) = std::env::var("REM_SCALE_KEEP_DB") {
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
        std::fs::copy(&db_path, &keep).unwrap();
        println!("REM_SCALE_KEPT {keep} bigRun={big_run} runId={run_id}");
        if std::env::var("REM_SCALE_STOP_AFTER_KEEP").as_deref() == Ok("1") {
            drop(connection);
            let _ = std::fs::remove_dir_all(root);
            return;
        }
    }

    let start = Instant::now();
    let report = propose(&connection, pid, if skip_full { Some(big_run) } else { None }).unwrap();
    let propose_ms = ms(start);
    out.insert("fullProposeSkipped".into(), json!(skip_full));
    let linked: i64 = connection
        .query_row("SELECT COUNT(DISTINCT incident_id) FROM exposure_incident_entities WHERE project_id=?1 AND origin='ownership_projection' AND active=1", [pid], |row| row.get(0))
        .unwrap();
    out.insert("proposeAll".into(), json!({"ms": propose_ms, "created": report.created, "locations": report.attached_locations, "incidentsLinkedFromProjection": linked}));
    assert!(linked > 0, "逐资产投影必须产生单位关联");

    let start = Instant::now();
    let summary = super::summary::exposure_scope_summary(&connection, pid).unwrap();
    let summary_cold = ms(start);
    let start = Instant::now();
    let summary = {
        let _ = summary;
        super::summary::exposure_scope_summary(&connection, pid).unwrap()
    };
    let summary_hot = ms(start);
    assert!(summary.unit_projection.is_some() && summary.coverage.is_some() && summary.unit_statistics.is_some());
    let start = Instant::now();
    super::coverage::unit_statistics(&connection, pid, summary.unit_projection.as_ref()).unwrap();
    let units_ms = ms(start);
    out.insert("summary".into(), json!({"coldMs": summary_cold, "hotMs": summary_hot, "unitStatisticsMs": units_ms}));

    // 事件分页与大事件的第 51 条以后的位置。
    let start = Instant::now();
    let page = page_incidents(&connection, pid, "", false, 50, "").unwrap();
    let incidents_first = ms(start);
    let mut cursor = page["nextCursor"].as_str().unwrap_or_default().to_string();
    let mut later = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        let next = page_incidents(&connection, pid, "", false, 50, &cursor).unwrap();
        later.push(ms(start));
        cursor = next["nextCursor"].as_str().unwrap_or_default().to_string();
    }
    let big: i64 = connection
        .query_row("SELECT incident_id FROM exposure_incident_locations WHERE project_id=?1 AND status='active' GROUP BY incident_id ORDER BY COUNT(*) DESC LIMIT 1", [pid], |row| row.get(0))
        .unwrap();
    let start = Instant::now();
    let read = super::incidents::read_incident(&connection, pid, big).unwrap();
    let read_ms = ms(start);
    assert_eq!(read["locations"]["total"], 200);
    let mut cursor = read["locations"]["nextCursor"].as_str().unwrap_or_default().to_string();
    let mut seen = read["locations"]["items"].as_array().unwrap().len();
    let mut location_pages = Vec::new();
    while !cursor.is_empty() {
        let start = Instant::now();
        let next = super::incidents::page_locations(&connection, pid, big, 50, &cursor).unwrap();
        location_pages.push(ms(start));
        seen += next["items"].as_array().unwrap().len();
        cursor = next["nextCursor"].as_str().unwrap_or_default().to_string();
    }
    assert_eq!(seen, 200, "第 51 条以后都能读到");
    out.insert("incidentPages".into(), json!({"firstMs": incidents_first, "subsequentMs": later, "total": page["total"], "bigIncidentReadMs": read_ms, "locationPagesMs": location_pages}));

    // 拆分 10 个位置再合并回去。
    let ids: Vec<i64> = read["locations"]["items"].as_array().unwrap().iter().take(10).map(|row| row["id"].as_i64().unwrap()).collect();
    let revision: i64 = connection.query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [big], |row| row.get(0)).unwrap();
    let start = Instant::now();
    let split = super::incidents::act(&connection, pid, &json!({"operationId": "op-scale-split-1", "action": "split", "incidentId": big, "expectedRevision": revision, "locationIds": ids, "actor": "规模", "reason": "规模测试拆分"})).unwrap();
    let split_ms = ms(start);
    let fresh = split["newIncidentId"].as_i64().unwrap();
    let rev_big: i64 = connection.query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [big], |row| row.get(0)).unwrap();
    let rev_fresh: i64 = connection.query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [fresh], |row| row.get(0)).unwrap();
    let start = Instant::now();
    super::incidents::act(&connection, pid, &json!({"operationId": "op-scale-merge-1", "action": "merge", "incidentId": fresh, "expectedRevision": rev_fresh, "targetIncidentId": big, "targetExpectedRevision": rev_big, "actor": "规模", "reason": "规模测试合并回去"})).unwrap();
    let merge_ms = ms(start);
    out.insert("manualActions".into(), json!({"split10Ms": split_ms, "movedUnitLinks": split["movedUnitLinks"], "mergeBackMs": merge_ms}));

    // 覆盖钻取：全量计划（max_items=n）后用 SQL 收口为 95% 完成 / 5% 配额阻塞，再按窗口与单位读。
    let mut settings = ExposureSettings::for_project(pid);
    settings.sources = vec!["passive_web".into()];
    settings.max_items = n;
    save(&connection, &settings, 0).unwrap();
    let frozen = super::super::exposure_scan_settings(&connection, pid).unwrap();
    let start = Instant::now();
    let started = start_or_reuse(&connection, pid, &frozen).unwrap();
    let plan_ms = ms(start);
    connection
        .execute("UPDATE exposure_work_items SET status=CASE WHEN COALESCE(asset_id,0) % 10=2 THEN 'blocked_quota' ELSE 'completed' END WHERE run_id=?1", [started.run_id])
        .unwrap();
    connection.execute("UPDATE exposure_runs SET status='partial' WHERE id=?1", [started.run_id]).unwrap();
    let mut drill = serde_json::Map::new();
    let start = Instant::now();
    let first = super::coverage::coverage_drilldown(&connection, pid, "passive_web", 0, None, false, 50, "").unwrap();
    drill.insert("latestRoundFirstPageMs".into(), json!(ms(start)));
    assert_eq!(first["total"], n);
    let start = Instant::now();
    let second = super::coverage::coverage_drilldown(&connection, pid, "passive_web", 0, None, false, 50, first["nextCursor"].as_str().unwrap_or_default()).unwrap();
    drill.insert("latestRoundSecondPageMs".into(), json!(ms(start)));
    assert_eq!(second["items"].as_array().unwrap().len(), 50);
    let start = Instant::now();
    let unit = super::coverage::coverage_drilldown(&connection, pid, "passive_web", 0, Some(5), true, 50, "").unwrap();
    drill.insert("unitGapsFirstPageMs".into(), json!(ms(start)));
    drill.insert("unitGapsTotal".into(), unit["total"].clone());
    assert!(unit["total"].as_i64().unwrap() > 0, "单位 5 的缺口必须能钻到");
    let start = Instant::now();
    let window = super::coverage::coverage_drilldown(&connection, pid, "passive_web", 30, None, true, 50, "").unwrap();
    drill.insert("window30dGapsFirstPageMs".into(), json!(ms(start)));
    drill.insert("window30dRounds".into(), json!(window["rounds"].as_array().map(|rows| rows.len())));
    drill.insert("unitAxisRows".into(), json!(first["unitAxis"]["rows"].as_array().map(|rows| rows.len())));
    out.insert("coverageDrilldown".into(), JsonValue::Object(drill));
    out.insert("planBuildMs".into(), json!(plan_ms));

    let mut wal = db_path.clone().into_os_string();
    wal.push("-wal");
    out.insert("storage".into(), json!({"dbBytes": file_size(&db_path), "walBytes": file_size(std::path::Path::new(&wal))}));
    out.insert("maxRssBytes".into(), json!(max_rss_bytes()));
    out.insert("totalMs".into(), json!(ms(total_start)));
    let line = JsonValue::Object(out).to_string();
    println!("REM_SCALE_RESULT {line}");
    if let Ok(path) = std::env::var("REM_SCALE_OUT") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        writeln!(file, "{line}").unwrap();
    }
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── FIX2（2026-10-09 13:20）propose 规模 ─────────────────────────────

/// 集团台账绑定 + 4/5 资产有组确认的单位依据（与规模测试同一形状）。
fn rem2_ledger(connection: &Connection, pid: i64) {
    connection.execute("INSERT OR REPLACE INTO org_directory_state(id,version) VALUES(1,5)", []).unwrap();
    for (id, kind, name, parent, region) in [
        (1, "group", "中国移动通信集团有限公司", None, ""),
        (2, "province_company", "中国移动通信集团浙江有限公司", Some(1), "330000"),
        (3, "professional_subsidiary", "咪咕文化科技有限公司", Some(1), ""),
        (4, "city_branch", "咪咕浙江杭州分公司", Some(3), "330100"),
        (5, "city_branch", "中国移动杭州分公司", Some(2), "330100"),
        (6, "county_branch", "中国移动余杭分公司", Some(5), "330110"),
    ] {
        connection
            .execute(
                "INSERT OR IGNORE INTO org_entities(id,group_id,parent_id,entity_kind,legal_name,normalized_name,region_code,directory_version) VALUES(?1,?2,?3,?4,?5,?6,?7,1)",
                params![id, if id == 1 { None } else { Some(1) }, parent, kind, name, format!("{name}-{id}"), region],
            )
            .unwrap();
    }
    connection
        .execute("INSERT INTO project_org_bindings(project_id,project_kind,group_entity_id,binding_version) VALUES(?1,'group_ledger',1,3)", [pid])
        .unwrap();
    connection
        .execute(
            "INSERT INTO ownership_analysis_runs(project_id,status,scope_version,data_revision,policy_version,directory_version,binding_version,algorithm_version) VALUES(?1,'completed','s','d',1,5,3,'a')",
            [pid],
        )
        .unwrap();
    let analysis = connection.last_insert_rowid();
    for (slot, entity) in [(0i64, 2i64), (1, 4), (2, 5), (3, 6)] {
        connection
            .execute(
                "INSERT INTO ownership_review_groups(run_id,project_id,group_key,lane,nav_bucket,candidate_entity_id,candidate_source,relation_kind,boundary_kind,support_kind,decision_status,decision_source,authorization_status,exposure_eligible) VALUES(?1,?2,?3,'consistent','province',?4,'directory','official_host','exact_host','directory_official_host','auto','auto','unknown',0)",
                params![analysis, pid, format!("rem2-{pid}-{slot}"), entity],
            )
            .unwrap();
        let group = connection.last_insert_rowid();
        let op = format!("op-rem2-{pid}-{slot}");
        connection
            .execute(
                "INSERT INTO ownership_operations(operation_id,project_id,run_id,group_id,kind,action,status,payload_hash,expected_action_hash,result_json) VALUES(?1,?2,?3,?4,'group_decision','confirm_ownership','completed','h','h','{}')",
                params![op, pid, analysis, group],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO ownership_operation_items(operation_id,asset_id,outcome) SELECT ?1,asset_id,'applied' FROM project_assets WHERE project_id=?2 AND (asset_id % 5)=?3",
                params![op, pid, slot],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO asset_events(project_id,asset_id,event_type,summary) SELECT ?1,asset_id,'ownership',?2 FROM project_assets WHERE project_id=?1 AND (asset_id % 5)=?3",
                params![pid, format!("confirmed: 组确认（复核组 #{group}，快照 #{analysis}，操作 {op}）"), slot],
            )
            .unwrap();
    }
    connection.execute("UPDATE asset_ownership_decisions SET source='manual' WHERE project_id=?1", [pid]).unwrap();
}

/// 一条新观察（可以挂在已有线索上，`finding` 为 Some 时只加观察与关联）。
#[allow(clippy::too_many_arguments)]
fn rem2_lead(connection: &Connection, pid: i64, run: i64, asset_id: i64, url: &str, body: &str, disposition: &str, finding: Option<i64>) -> i64 {
    let evidence = json!({"detail": {"collectionMode": "http_read", "excerpt": "样本公司 内部接口"}}).to_string();
    connection
        .execute(
            "INSERT INTO exposure_observations(run_id,project_id,source_key,asset_id,kind,url,canonical_url,status_code,content_hash,body_bytes,truncated,evidence_json)
             VALUES(?1,?2,'passive_web',?3,'page',?4,?4,200,?5,4096,0,?6)",
            params![run, pid, asset_id, url, body, evidence],
        )
        .unwrap();
    let observation = connection.last_insert_rowid();
    let finding = match finding {
        Some(id) => id,
        None => {
            connection
                .execute(
                    "INSERT INTO exposure_findings(project_id,asset_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,status,fingerprint)
                     VALUES(?1,?2,'secret','新线索','passive_web',?3,'摘录','high',70,'new',?4)",
                    params![pid, asset_id, url, format!("rem2-{run}-{url}")],
                )
                .unwrap();
            let id = connection.last_insert_rowid();
            connection
                .execute(
                    "INSERT INTO exposure_finding_dispositions(finding_id,project_id,disposition,info_category,source_channel,reason,policy_version,classification_id,updated_at)
                     VALUES(?1,?2,?3,'credential','first_party_web','规模测试','scale',0,'2026-10-09 00:00:00')",
                    params![id, pid, disposition],
                )
                .unwrap();
            id
        }
    };
    connection
        .execute("INSERT INTO exposure_finding_observations(finding_id,observation_id,run_id,project_id) VALUES(?1,?2,?3,?4)", params![finding, observation, run, pid])
        .unwrap();
    finding
}

fn rem2_run(connection: &Connection, pid: i64) -> i64 {
    connection
        .execute("INSERT INTO exposure_runs(project_id,status,stage,completed_at) VALUES(?1,'completed','completed','2026-10-09 00:00:00')", [pid])
        .unwrap();
    connection.last_insert_rowid()
}

fn rem2_asset(connection: &Connection, index: i64) -> i64 {
    connection.query_row("SELECT id FROM assets WHERE asset_key=?1", [format!("https://h{index}.scale.example.test")], |row| row.get(0)).unwrap()
}

/// 本轮之后的第二轮：已有内容的新传播位置、已核实事件里的线索内容变化与被排除、新的单条风险线索。
fn rem2_second_round(connection: &Connection, pid: i64, leads: i64) -> i64 {
    let run = rem2_run(connection, pid);
    for index in 0..leads {
        let asset_id = rem2_asset(connection, index * 7);
        // 与第一轮 dup{k} 相同的内容 → 已有事件的新位置；以及新的单条风险线索。
        rem2_lead(connection, pid, run, asset_id, &format!("https://h{}.scale.example.test/copy-{index}.txt", index * 7), &format!("dup{}", index % 10), "classification_pending", None);
        rem2_lead(connection, pid, run, asset_id, &format!("https://h{}.scale.example.test/new-{index}.env", index * 7), &format!("new{index}"), "risk_candidate", None);
    }
    run
}

/// 一个事件的完整可比形态（不含时间戳）。
fn rem2_dump(connection: &Connection, pid: i64) -> String {
    let rows = |sql: &str| -> Vec<JsonValue> {
        let mut statement = connection.prepare(sql).unwrap();
        let columns = statement.column_count();
        statement
            .query_map([pid], |row| {
                let mut cells = Vec::new();
                for index in 0..columns {
                    let value: rusqlite::types::Value = row.get(index)?;
                    cells.push(match value {
                        rusqlite::types::Value::Null => JsonValue::Null,
                        rusqlite::types::Value::Integer(v) => json!(v),
                        rusqlite::types::Value::Real(v) => json!(v),
                        rusqlite::types::Value::Text(v) => json!(rem2_without_decision_time(&v)),
                        rusqlite::types::Value::Blob(v) => json!(v.len()),
                    });
                }
                Ok(JsonValue::Array(cells))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    json!({
        "incidents": rows("SELECT id,stable_key,title,info_category,status,revision,basis,first_run_id,last_run_id,review_needed,review_needed_reason,merged_into,handling_state FROM exposure_incidents WHERE project_id=?1 ORDER BY id"),
        "locations": rows("SELECT id,incident_id,finding_id,observation_id,run_id,location_kind,canonical_url,source_key,collection_mode,relation,content_key,status FROM exposure_incident_locations WHERE project_id=?1 ORDER BY id"),
        "entities": rows("SELECT id,incident_id,entity_id,link_level,origin,basis,evidence_json,directory_version,binding_version,active FROM exposure_incident_entities WHERE project_id=?1 ORDER BY id"),
        "events": rows("SELECT id,incident_id,operation_id,action,actor,expected_revision,from_status,to_status,refs_json,result_json FROM exposure_incident_events WHERE project_id=?1 ORDER BY id"),
    })
    .to_string()
}

/// 证据里的归属决定时间是夹具写入时的墙钟，与建议逻辑无关，比较前抹掉。
fn rem2_without_decision_time(text: &str) -> String {
    let marker = "\"decidedAt\":\"";
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(marker) {
        out.push_str(&rest[..start + marker.len()]);
        rest = &rest[start + marker.len()..];
        let end = rest.find('"').unwrap_or(rest.len());
        out.push('T');
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn rem2_db(tag: &str) -> (std::path::PathBuf, Connection) {
    let root = std::env::temp_dir().join(format!("oviraptor-exposure-rem2-{tag}-{}", uuid::Uuid::new_v4()));
    let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
    let connection = crate::db::open(&db_path).unwrap();
    ensure_exposure_schema(&connection).unwrap();
    (root, connection)
}

/// FIX2-1 等价：同一组输入，建议结果（事件、位置、单位关联、事件日志）逐条与修复前完全一致。
/// 期望值是修复前的实现在这个夹具上跑出来的全量形态的哈希（见交付文档）。
#[test]
fn rem2_propose_results_are_identical_to_the_pre_fix_implementation() {
    let (root, connection) = rem2_db("equiv");
    let pid = 911;
    let run1 = seed(&connection, pid, 300);
    rem2_ledger(&connection, pid);
    for k in 0..3 {
        rem2_lead(&connection, pid, run1, rem2_asset(&connection, 100 + k), &format!("https://solo{k}.example.test/x.env"), &format!("solo{k}"), "risk_candidate", None);
    }
    propose(&connection, pid, Some(run1)).unwrap();
    // 人工把前三个事件核实，之后的轮次只能标“需复核”。
    let decided: Vec<i64> = {
        let mut statement = connection.prepare("SELECT id FROM exposure_incidents WHERE project_id=?1 AND stable_key LIKE 'content:body:solo%' ORDER BY id LIMIT 3").unwrap();
        let rows = statement.query_map([pid], |row| row.get(0)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        rows
    };
    for (index, incident) in decided.iter().enumerate() {
        let revision: i64 = connection.query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [incident], |row| row.get(0)).unwrap();
        super::incidents::act(&connection, pid, &json!({"operationId": format!("op-rem2-review-{index}"), "action": "review", "incidentId": incident, "expectedRevision": revision, "actor": "张三", "reason": "人工核实无误", "status": "verified"})).unwrap();
    }
    assert_eq!(decided.len(), 3, "夹具需要三个单线索事件");
    let run2 = rem2_second_round(&connection, pid, 30);
    // 三个已核实的单线索事件（本轮的相同内容副本不会碰到它们）：一条线索内容变化（版本 2），
    // 一条被排除，第三条的位置已拆出（非活跃）且内容变化——只有前两个该标“需复核”。
    let in_decided: Vec<(i64, i64)> = decided
        .iter()
        .map(|incident| {
            connection
                .query_row("SELECT finding_id,incident_id FROM exposure_incident_locations WHERE incident_id=?1 ORDER BY id LIMIT 1", [incident], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
        })
        .collect();
    connection.execute("UPDATE exposure_incident_locations SET status='split_out' WHERE incident_id=?1 AND finding_id=?2", params![in_decided[2].1, in_decided[2].0]).unwrap();
    let asset_of = |finding: i64| -> i64 { connection.query_row("SELECT asset_id FROM exposure_findings WHERE id=?1", [finding], |row| row.get(0)).unwrap() };
    rem2_lead(&connection, pid, run2, asset_of(in_decided[0].0), "https://changed.example.test/a", "changed-bytes", "risk_candidate", Some(in_decided[0].0));
    connection.execute("INSERT INTO exposure_finding_versions(finding_id,run_id,revision) VALUES(?1,?2,2)", params![in_decided[0].0, run2]).unwrap();
    rem2_lead(&connection, pid, run2, asset_of(in_decided[1].0), "https://excluded.example.test/b", "excluded-bytes", "risk_candidate", Some(in_decided[1].0));
    connection.execute("UPDATE exposure_finding_dispositions SET disposition='excluded_public_material' WHERE finding_id=?1", [in_decided[1].0]).unwrap();
    rem2_lead(&connection, pid, run2, asset_of(in_decided[2].0), "https://split.example.test/c", "split-bytes", "risk_candidate", Some(in_decided[2].0));
    connection.execute("INSERT INTO exposure_finding_versions(finding_id,run_id,revision) VALUES(?1,?2,2)", params![in_decided[2].0, run2]).unwrap();
    let second = propose(&connection, pid, Some(run2)).unwrap();
    assert!(second.attached_locations > 0 && second.review_flagged > 0, "{second:?}");
    let flagged: Vec<i64> = decided
        .iter()
        .map(|incident| connection.query_row("SELECT review_needed FROM exposure_incidents WHERE id=?1", [incident], |row| row.get(0)).unwrap())
        .collect();
    assert_eq!(flagged, vec![1, 1, 0], "内容变化与被排除的标需复核，拆出的位置不算");
    let third = propose(&connection, pid, None).unwrap();
    let dump = rem2_dump(&connection, pid);
    let digest = super::redaction::content_hash(dump.as_bytes());
    println!("REM2_EQUIV report2={second:?} report3={third:?} digest={digest} bytes={}", dump.len());
    if let Ok(path) = std::env::var("REM2_EQUIV_DUMP") {
        std::fs::write(path, &dump).unwrap();
    }
    assert_eq!(digest, REM2_EQUIV_DIGEST, "建议结果与修复前不一致");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

const REM2_EQUIV_DIGEST: &str = "ecd10d559a4c2fbc80247448b09aad7af8236db6fe1e37d1102992f5c9610e3f";

/// SQLite 虚拟机步数：与机器快慢无关的“做了多少工作”。
fn rem2_vm_steps(connection: &Connection, work: impl FnOnce()) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    let steps = std::sync::Arc::new(AtomicU64::new(0));
    let counter = steps.clone();
    connection.progress_handler(100, Some(move || {
        counter.fetch_add(100, Ordering::Relaxed);
        false
    }))
    .unwrap();
    work();
    connection.progress_handler(100, None::<fn() -> bool>).unwrap();
    steps.load(Ordering::Relaxed)
}

fn rem2_round_cost(background: i64) -> u64 {
    let (root, connection) = rem2_db(&format!("cost-{background}"));
    let pid = 912;
    let run1 = seed(&connection, pid, background);
    rem2_ledger(&connection, pid);
    propose(&connection, pid, Some(run1)).unwrap();
    let run2 = rem2_second_round(&connection, pid, 20);
    let steps = rem2_vm_steps(&connection, || {
        propose(&connection, pid, Some(run2)).unwrap();
    });
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
    steps
}

/// FIX2-1 红测：同样 40 条新线索的一轮，项目从 1000 涨到 4000 个资产，单轮建议的工作量不能跟着涨。
#[test]
fn rem2_a_single_round_proposal_costs_what_the_round_holds_not_what_the_project_holds() {
    let small = rem2_round_cost(1000);
    let large = rem2_round_cost(4000);
    println!("REM2_COST small={small} large={large} ratio={:.2}", large as f64 / small as f64);
    assert!(large * 10 <= small * 12, "单轮建议的工作量随项目规模增长：1000 资产 {small} 步，4000 资产 {large} 步");
}

/// FIX2-2：暴露面结构为 asset_events 建 (project_id, asset_id, event_type) 索引，归属投影的逐资产
/// 事件查询走索引而不是扫全项目；表不存在时结构初始化照常成功、只跳过这一条。
#[test]
fn rem2_asset_events_are_looked_up_by_asset_through_an_index() {
    let (root, connection) = rem2_db("idx");
    let plan: Vec<String> = {
        let mut statement = connection
            .prepare(
                "EXPLAIN QUERY PLAN SELECT MAX(id) FROM asset_events WHERE project_id=1 AND asset_id=2 AND event_type='ownership' AND summary NOT LIKE 'confirmed%'",
            )
            .unwrap();
        let rows = statement.query_map([], |row| row.get::<_, String>(3)).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        rows
    };
    assert!(plan.iter().any(|line| line.contains("idx_asset_events_project_asset")), "逐资产事件查询没有走索引：{plan:?}");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rem2_the_exposure_schema_still_initializes_when_asset_events_is_missing() {
    let (root, connection) = rem2_db("noevents");
    connection.execute_batch("DROP TABLE asset_events").unwrap();
    let warnings = ensure_exposure_schema(&connection).expect("asset_events 不存在时暴露面结构仍能初始化");
    assert!(warnings.iter().any(|row| row.contains("asset_events")), "{warnings:?}");
    let index: i64 = connection
        .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_asset_events_project_asset'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(index, 0);
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

fn rem2_ownership_links(connection: &Connection, pid: i64, slot: i64) -> (i64, i64) {
    // (应有关联的 (事件, 资产) 对数, 实际有效的归属投影关联数)，只看 asset_id % 5 == slot 的资产。
    let pairs: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM (SELECT DISTINCT l.incident_id, o.asset_id FROM exposure_incident_locations l
               JOIN exposure_incidents i ON i.id=l.incident_id AND i.status<>'merged'
               JOIN exposure_observations o ON o.id=l.observation_id
             WHERE l.project_id=?1 AND l.status='active' AND (o.asset_id % 5)=?2)",
            params![pid, slot],
            |row| row.get(0),
        )
        .unwrap();
    let links: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_incident_entities WHERE project_id=?1 AND origin='ownership_projection' AND active=1
             AND (json_extract(evidence_json,'$[0].assetId') % 5)=?2",
            params![pid, slot],
            |row| row.get(0),
        )
        .unwrap();
    (pairs, links)
}

/// FIX2-3 红测：归属组决定晚于事件创建，下一轮建议要把单位关联补上；之后归属被人工改掉，
/// 读事件时关联立即失效、下一轮建议也会失效；项目不再是集团台账时全量建议让投影关联全部失效。
#[test]
fn rem2_unit_links_catch_up_with_ownership_decided_after_the_incident() {
    let (root, connection) = rem2_db("late");
    let pid = 913;
    let run1 = seed(&connection, pid, 200);
    rem2_ledger(&connection, pid);
    // 第 3 组（余杭分公司）的组决定此时还没有做。
    connection
        .execute("DELETE FROM ownership_operation_items WHERE operation_id=?1", [format!("op-rem2-{pid}-3")])
        .unwrap();
    connection
        .execute("DELETE FROM asset_events WHERE project_id=?1 AND instr(summary,?2)>0", params![pid, format!("op-rem2-{pid}-3）")])
        .unwrap();
    propose(&connection, pid, Some(run1)).unwrap();
    let (pairs, links) = rem2_ownership_links(&connection, pid, 3);
    assert!(pairs > 0, "夹具里第 3 组资产必须有事件位置");
    assert_eq!(links, 0, "组决定之前不应有单位关联");
    let (other_pairs, other_links) = rem2_ownership_links(&connection, pid, 2);
    assert!(other_pairs > 0 && other_links == other_pairs);

    // 事件创建之后才做组决定。
    connection
        .execute(
            "INSERT INTO ownership_operation_items(operation_id,asset_id,outcome) SELECT ?1,asset_id,'applied' FROM project_assets WHERE project_id=?2 AND (asset_id % 5)=3",
            params![format!("op-rem2-{pid}-3"), pid],
        )
        .unwrap();
    let group: i64 = connection.query_row("SELECT group_id FROM ownership_operations WHERE operation_id=?1", [format!("op-rem2-{pid}-3")], |row| row.get(0)).unwrap();
    connection
        .execute(
            "INSERT INTO asset_events(project_id,asset_id,event_type,summary) SELECT ?1,asset_id,'ownership',?2 FROM project_assets WHERE project_id=?1 AND (asset_id % 5)=3",
            params![pid, format!("confirmed: 组确认（复核组 #{group}，快照 #1，操作 op-rem2-{pid}-3）")],
        )
        .unwrap();
    let run2 = rem2_second_round(&connection, pid, 2);
    propose(&connection, pid, Some(run2)).unwrap();
    let (pairs, links) = rem2_ownership_links(&connection, pid, 3);
    assert_eq!(links, pairs, "下一轮建议应补上晚到的单位关联");
    let unit: i64 = connection
        .query_row(
            "SELECT MIN(entity_id) FROM exposure_incident_entities WHERE project_id=?1 AND origin='ownership_projection' AND active=1 AND (json_extract(evidence_json,'$[0].assetId') % 5)=3",
            [pid],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unit, 6, "单位来自那次组决定的目录实体");

    // 之后人工把一个资产的归属改掉：读这个事件时关联立刻失效。
    let (incident, asset): (i64, i64) = connection
        .query_row(
            "SELECT incident_id, json_extract(evidence_json,'$[0].assetId') FROM exposure_incident_entities WHERE project_id=?1 AND origin='ownership_projection' AND active=1
             AND (json_extract(evidence_json,'$[0].assetId') % 5)=3 ORDER BY id LIMIT 1",
            [pid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    connection
        .execute("INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership','related: 人工改为关联资产')", params![pid, asset])
        .unwrap();
    let view = super::incidents::read_incident(&connection, pid, incident).unwrap();
    let shown = view.to_string();
    assert!(!shown.contains("\"origin\":\"ownership_projection\"") || !view["entities"].as_array().unwrap().iter().any(|row| row["origin"] == "ownership_projection" && row["evidence"][0]["assetId"] == asset), "读事件时已不成立的单位关联仍显示：{shown}");
    let still: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_incident_entities WHERE incident_id=?1 AND origin='ownership_projection' AND active=1 AND json_extract(evidence_json,'$[0].assetId')=?2",
            params![incident, asset],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(still, 0);

    // 另一个资产也被改掉，不读事件，下一轮建议同样让它失效。
    let other: i64 = connection
        .query_row(
            "SELECT json_extract(evidence_json,'$[0].assetId') FROM exposure_incident_entities WHERE project_id=?1 AND origin='ownership_projection' AND active=1
             AND (json_extract(evidence_json,'$[0].assetId') % 5)=2 ORDER BY id LIMIT 1",
            [pid],
            |row| row.get(0),
        )
        .unwrap();
    connection
        .execute("INSERT INTO asset_events(project_id,asset_id,event_type,summary) VALUES(?1,?2,'ownership','third_party: 人工改为第三方')", params![pid, other])
        .unwrap();
    let run3 = rem2_second_round(&connection, pid, 1);
    propose(&connection, pid, Some(run3)).unwrap();
    let stale: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_incident_entities WHERE project_id=?1 AND origin='ownership_projection' AND active=1 AND json_extract(evidence_json,'$[0].assetId')=?2",
            params![pid, other],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale, 0, "归属改掉后下一轮建议应让旧关联失效");

    // 第 4 组的组决定正在执行（applying）时跑了一轮：还不能关联；操作完成后（没有新明细、新事件）
    // 下一轮仍要补上——进行中的操作不推进明细水位。
    let analysis: i64 = connection.query_row("SELECT run_id FROM ownership_operations WHERE operation_id=?1", [format!("op-rem2-{pid}-0")], |row| row.get(0)).unwrap();
    connection
        .execute(
            "INSERT INTO ownership_review_groups(run_id,project_id,group_key,lane,nav_bucket,candidate_entity_id,candidate_source,relation_kind,boundary_kind,support_kind,decision_status,decision_source,authorization_status,exposure_eligible) VALUES(?1,?2,?3,'consistent','province',5,'directory','official_host','exact_host','directory_official_host','auto','auto','unknown',0)",
            params![analysis, pid, format!("rem2-{pid}-4")],
        )
        .unwrap();
    let group4 = connection.last_insert_rowid();
    let op4 = format!("op-rem2-{pid}-4");
    connection
        .execute(
            "INSERT INTO ownership_operations(operation_id,project_id,run_id,group_id,kind,action,status,payload_hash,expected_action_hash,result_json) VALUES(?1,?2,?3,?4,'group_decision','confirm_ownership','applying','h','h','{}')",
            params![op4, pid, analysis, group4],
        )
        .unwrap();
    connection
        .execute("INSERT INTO ownership_operation_items(operation_id,asset_id,outcome) SELECT ?1,asset_id,'applied' FROM project_assets WHERE project_id=?2 AND (asset_id % 5)=4", params![op4, pid])
        .unwrap();
    connection
        .execute(
            "INSERT INTO asset_events(project_id,asset_id,event_type,summary) SELECT ?1,asset_id,'ownership',?2 FROM project_assets WHERE project_id=?1 AND (asset_id % 5)=4",
            params![pid, format!("confirmed: 组确认（复核组 #{group4}，快照 #{analysis}，操作 {op4}）")],
        )
        .unwrap();
    let run4 = rem2_second_round(&connection, pid, 1);
    propose(&connection, pid, Some(run4)).unwrap();
    let (pairs4, links4) = rem2_ownership_links(&connection, pid, 4);
    assert!(pairs4 > 0);
    assert_eq!(links4, 0, "组操作还在执行，不应关联");
    connection.execute("UPDATE ownership_operations SET status='completed' WHERE operation_id=?1", [&op4]).unwrap();
    let run5 = rem2_second_round(&connection, pid, 1);
    propose(&connection, pid, Some(run5)).unwrap();
    let (pairs4, links4) = rem2_ownership_links(&connection, pid, 4);
    assert_eq!(links4, pairs4, "组操作完成后下一轮应补上关联");

    // 项目解除集团台账绑定：下一轮建议（增量）就让归属投影关联全部失效。
    connection.execute("DELETE FROM project_org_bindings WHERE project_id=?1", [pid]).unwrap();
    let run6 = rem2_second_round(&connection, pid, 1);
    propose(&connection, pid, Some(run6)).unwrap();
    let left: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_incident_entities WHERE project_id=?1 AND origin='ownership_projection' AND active=1", [pid], |row| row.get(0))
        .unwrap();
    assert_eq!(left, 0);
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// FIX2-4 红测：用 A 项目的身份取消 B 项目的扫描必须被拒绝，且数据库零写入；不存在的任务同样拒绝；
/// 本项目的取消照常生效。
#[test]
fn rem2_cancelling_another_projects_scan_is_refused_without_writing() {
    let (root, connection) = rem2_db("cancel");
    for pid in [921, 922] {
        connection.execute("INSERT INTO projects(id,name,status) VALUES(?1,'取消测试-'||?1,'active')", [pid]).unwrap();
    }
    connection.execute("INSERT INTO exposure_runs(project_id,status,stage) VALUES(922,'running','collecting')", []).unwrap();
    let run_b = connection.last_insert_rowid();
    let snapshot = |c: &Connection| -> String {
        c.query_row("SELECT json_group_array(json_array(id,project_id,status,stage,cancel_requested)) FROM exposure_runs", [], |r| r.get(0)).unwrap()
    };
    let before = snapshot(&connection);
    let changes = connection.total_changes();
    let refused = crate::commands::cancel_exposure_scan_in(&connection, 921, run_b);
    assert!(refused.as_ref().is_err_and(|error| error.contains("exposure_run_project_mismatch")), "{refused:?}");
    assert_eq!(connection.total_changes(), changes, "被拒绝的取消不能写库");
    assert_eq!(snapshot(&connection), before);
    let missing = crate::commands::cancel_exposure_scan_in(&connection, 921, run_b + 100);
    assert!(missing.as_ref().is_err_and(|error| error.contains("exposure_run_not_found")), "{missing:?}");
    assert_eq!(connection.total_changes(), changes);
    crate::commands::cancel_exposure_scan_in(&connection, 922, run_b).unwrap();
    let (flag, stage): (i64, String) = connection
        .query_row("SELECT cancel_requested,stage FROM exposure_runs WHERE id=?1", [run_b], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((flag, stage.as_str()), (1, "cancelling"));
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

// ───────────────────────────── FIX3（2026-10-09 17:29） ─────────────────────────────

fn fix3_active_unit_links(connection: &Connection, pid: i64, asset: i64) -> i64 {
    connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_incident_entities WHERE project_id=?1 AND origin='ownership_projection' AND active=1 AND json_extract(evidence_json,'$[0].assetId')=?2",
            params![pid, asset],
            |row| row.get(0),
        )
        .unwrap()
}

/// FIX3-P2-1 红测：读一个有 40 个不同资产位置的事件，单位关联对齐不能按位置重复加载整份组织目录。
/// 目录多 3000 个实体时，读事件多出的工作量应与“取一次单资产边”多出的量同级，而不是 40 倍。
#[test]
fn fix3_p2_1_reading_a_large_incident_loads_the_directory_once() {
    let (root, connection) = rem2_db("p2-1");
    let pid = 931;
    let run1 = seed(&connection, pid, 200);
    rem2_ledger(&connection, pid);
    propose(&connection, pid, Some(run1)).unwrap();
    let run2 = rem2_run(&connection, pid);
    let assets: Vec<i64> = {
        let mut statement = connection.prepare("SELECT asset_id FROM project_assets WHERE project_id=?1 AND (asset_id % 5)=2 ORDER BY asset_id LIMIT 40").unwrap();
        let rows = statement.query_map([pid], |row| row.get::<_, i64>(0)).unwrap();
        rows.collect::<Result<_, _>>().unwrap()
    };
    assert_eq!(assets.len(), 40);
    for asset in &assets {
        rem2_lead(&connection, pid, run2, *asset, &format!("https://bulk{asset}.scale.example.test/leak.env"), "fix3-bigdup", "risk_candidate", None);
    }
    propose(&connection, pid, Some(run2)).unwrap();
    let incident: i64 = connection
        .query_row("SELECT incident_id FROM exposure_incident_locations WHERE canonical_url=?1", [format!("https://bulk{}.scale.example.test/leak.env", assets[0])], |row| row.get(0))
        .unwrap();
    let locations: i64 = connection.query_row("SELECT COUNT(*) FROM exposure_incident_locations WHERE incident_id=?1 AND status='active'", [incident], |row| row.get(0)).unwrap();
    assert!(locations >= 40, "夹具应得到一个大事件：{locations}");
    super::incidents::read_incident(&connection, pid, incident).unwrap();
    let read_small = rem2_vm_steps(&connection, || {
        super::incidents::read_incident(&connection, pid, incident).unwrap();
    });
    let edge_small = rem2_vm_steps(&connection, || {
        crate::commands::ownership_asset_unit_edge(&connection, pid, assets[0]).unwrap();
    });
    connection
        .execute(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<3000)
             INSERT INTO org_entities(id,group_id,parent_id,entity_kind,legal_name,normalized_name,region_code,directory_version)
             SELECT 100+i,1,2,'city_branch','填充分公司'||i,'填充分公司-'||i,'330100',1 FROM n",
            [],
        )
        .unwrap();
    let read_big = rem2_vm_steps(&connection, || {
        super::incidents::read_incident(&connection, pid, incident).unwrap();
    });
    let edge_big = rem2_vm_steps(&connection, || {
        crate::commands::ownership_asset_unit_edge(&connection, pid, assets[0]).unwrap();
    });
    let read_delta = read_big.saturating_sub(read_small);
    let edge_delta = edge_big.saturating_sub(edge_small).max(1);
    eprintln!("FIX3_P2_1 locations={locations} read_small={read_small} read_big={read_big} edge_small={edge_small} edge_big={edge_big} ratio={:.2}", read_delta as f64 / edge_delta as f64);
    // 修复前每个位置各加载一次目录，约为 40 倍；批量后是常数倍（读事件本身还要读一次绑定上下文）。
    assert!(read_delta <= edge_delta * 6, "读事件随目录增长 {read_delta}，单次取边增长 {edge_delta}：目录被按位置重复加载");
    // 批量结果与逐个取边一致。
    let batch = crate::commands::ownership_asset_unit_edges(&connection, pid, &assets).unwrap();
    for asset in &assets {
        let one = crate::commands::ownership_asset_unit_edge(&connection, pid, *asset).unwrap();
        assert_eq!(batch.get(asset).map(|edge| (edge.owner_entity_id, edge.relation.clone())), one.map(|edge| (edge.owner_entity_id, edge.relation)));
    }
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}

/// FIX3-P2-2 红测：资产移入回收站（不写 asset_events）后，下一轮增量建议要让它的单位关联失效；
/// 从回收站恢复后，再下一轮要补回来。
#[test]
fn fix3_p2_2_recycle_bin_moves_are_seen_by_the_incremental_refresh() {
    let (root, connection) = rem2_db("p2-2");
    let pid = 932;
    let run1 = seed(&connection, pid, 200);
    rem2_ledger(&connection, pid);
    propose(&connection, pid, Some(run1)).unwrap();
    let asset: i64 = connection
        .query_row(
            // 取第二轮夹具不会再碰到的资产（第二轮只在 h0 上加位置，新位置本身会触发逐资产对齐）。
            "SELECT MAX(json_extract(evidence_json,'$[0].assetId')) FROM exposure_incident_entities WHERE project_id=?1 AND origin='ownership_projection' AND active=1",
            [pid],
            |row| row.get(0),
        )
        .unwrap();
    assert!(fix3_active_unit_links(&connection, pid, asset) > 0);
    connection
        .execute("UPDATE project_assets SET is_deleted=1,deleted_at=datetime('now','localtime') WHERE project_id=?1 AND asset_id=?2", params![pid, asset])
        .unwrap();
    let run2 = rem2_second_round(&connection, pid, 1);
    propose(&connection, pid, Some(run2)).unwrap();
    assert_eq!(fix3_active_unit_links(&connection, pid, asset), 0, "回收站里的资产，单位关联仍算当前");
    connection
        .execute("UPDATE project_assets SET is_deleted=0,deleted_at=NULL WHERE project_id=?1 AND asset_id=?2", params![pid, asset])
        .unwrap();
    let run3 = rem2_second_round(&connection, pid, 1);
    propose(&connection, pid, Some(run3)).unwrap();
    assert!(fix3_active_unit_links(&connection, pid, asset) > 0, "从回收站恢复后单位关联没有补回");
    drop(connection);
    let _ = std::fs::remove_dir_all(root);
}
