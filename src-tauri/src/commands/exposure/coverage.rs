//! G4-1 覆盖矩阵与跨单位统计（只读）。
//!
//! - 覆盖矩阵：来源能力 × 信息类别，每格是一个可区分的状态（未配置、未实施、待授权、计划中、
//!   已配置未执行、已执行无命中、存在候选、存在已核实事件、部分执行、失败、取消/中断、不适用）。
//!   每一行写明分母单位与分母（计划/已执行/未覆盖），失败与阻塞留在分母里。只有“全部工作项
//!   完成且无命中”才是 green，任何缺口都不会变绿。
//! - 单位统计：集团层面按事件唯一计数；单位涉及数按 entity 链接计，一个事件涉及多个单位时
//!   每个单位各计一次，因此单位之和可以大于集团唯一事件数，页面同时给出两者与说明。
//!   专业公司单列，不并入省公司；没有单位关联的事件进“单位未知”。

use super::units::{place_unit, UnitProjection};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const COVERAGE_POLICY_VERSION: &str = "oviraptor.exposure.coverage.v1";

pub const CELL_STATES: &[&str] = &[
    "unconfigured",
    "not_implemented",
    "pending_authorization",
    "planned",
    "not_run",
    "executed_no_hit",
    "candidate",
    "verified_incident",
    "partial",
    "failed",
    "cancelled_interrupted",
    "not_applicable",
];

pub fn state_label(state: &str) -> &'static str {
    match state {
        "unconfigured" => "未配置",
        "not_implemented" => "未实施",
        "pending_authorization" => "待授权",
        "planned" => "计划中",
        "not_run" => "已配置·尚未执行",
        "executed_no_hit" => "已执行无命中",
        "executed" => "已执行",
        "candidate" => "存在候选",
        "verified_incident" => "存在已核实事件",
        "partial" => "部分执行",
        "failed" => "失败",
        "cancelled_interrupted" => "取消/中断",
        "not_applicable" => "不适用",
        _ => "未知",
    }
}

#[derive(Clone, Copy)]
enum Support {
    Supported,
    IntegrationPending(&'static str),
    Unsupported(&'static str),
}

struct CapabilityRow {
    capability: &'static str,
    source_key: &'static str,
    channel: &'static str,
    unit: &'static str,
    support: Support,
    categories: &'static [&'static str],
}

const ALL_BUT_INTEL: &[&str] = &[
    "internal_business",
    "technical_network",
    "source_code",
    "credential",
    "employee_data",
    "customer_data",
    "storage_misexposure",
    "unclassified",
];
const ALL: &[&str] = &[
    "internal_business",
    "technical_network",
    "source_code",
    "credential",
    "employee_data",
    "customer_data",
    "storage_misexposure",
    "sold_intel",
    "unclassified",
];

const ROWS: &[CapabilityRow] = &[
    CapabilityRow { capability: "first_party_web", source_key: "passive_web", channel: "first_party_web", unit: "资产", support: Support::Supported, categories: ALL_BUT_INTEL },
    CapabilityRow { capability: "certificate_index", source_key: "certificate_transparency", channel: "certificate_index", unit: "域名", support: Support::Supported, categories: &["technical_network"] },
    CapabilityRow { capability: "archive_index", source_key: "web_archive", channel: "archive_index", unit: "域名", support: Support::Supported, categories: ALL_BUT_INTEL },
    CapabilityRow { capability: "repository_metadata", source_key: "github_repositories", channel: "code_platform", unit: "查询", support: Support::Supported, categories: ALL_BUT_INTEL },
    CapabilityRow { capability: "code_search", source_key: "github_code", channel: "code_platform", unit: "查询", support: Support::Supported, categories: ALL_BUT_INTEL },
    CapabilityRow { capability: "explicit_document_url", source_key: "document_link", channel: "document_link", unit: "URL", support: Support::Supported, categories: ALL_BUT_INTEL },
    CapabilityRow { capability: "file_import", source_key: "document_import", channel: "document_import", unit: "文件", support: Support::Supported, categories: ALL_BUT_INTEL },
    CapabilityRow { capability: "public_intel_import", source_key: "public_intel_import", channel: "public_intel_import", unit: "报告记录", support: Support::Supported, categories: ALL },
    CapabilityRow { capability: "pdf_docx_ocr", source_key: "document_import", channel: "document_import", unit: "文件", support: Support::Unsupported("PDF/Office/OCR 没有解析器"), categories: ALL_BUT_INTEL },
    CapabilityRow { capability: "netdisk_keyword_search", source_key: "netdisk", channel: "unrecognized_source", unit: "查询", support: Support::Unsupported("网盘/文库关键词搜索不在本期"), categories: ALL_BUT_INTEL },
    CapabilityRow { capability: "underground_feed", source_key: "underground", channel: "public_intel_import", unit: "报告记录", support: Support::Unsupported("暗网/地下市场接入不在本期，只接受用户导入的脱敏报告"), categories: &["sold_intel", "credential", "customer_data", "employee_data"] },
];

fn storage(error: impl std::fmt::Display) -> String {
    format!("[exposure_coverage_storage] {error}")
}

#[derive(Default, Clone, Debug)]
struct ItemCounts {
    total: i64,
    completed: i64,
    failed: i64,
    cancelled: i64,
    unconfigured: i64,
    quota: i64,
    authorization: i64,
    open: i64,
}

struct RunInfo {
    run_id: i64,
    status: String,
    created_at: String,
    completed_at: String,
    counts: ItemCounts,
    /// P1-6 the source's own verdict for this round (partial / rate_limited / …); `None` for
    /// rounds older than source results.
    source_result: Option<String>,
}

fn latest_runs(connection: &Connection, project_id: i64) -> Result<HashMap<String, RunInfo>, String> {
    let mut statement = connection
        .prepare(
            "SELECT w.source_key, w.run_id, r.status, r.created_at, r.completed_at, w.status, COUNT(*)
             FROM exposure_work_items w JOIN exposure_runs r ON r.id=w.run_id AND r.project_id=?1
             WHERE w.project_id=?1 AND w.run_id=(SELECT MAX(x.run_id) FROM exposure_work_items x WHERE x.project_id=?1 AND x.source_key=w.source_key)
             GROUP BY w.source_key, w.run_id, w.status",
        )
        .map_err(storage)?;
    let mut rows = statement.query(params![project_id]).map_err(storage)?;
    let mut map: HashMap<String, RunInfo> = HashMap::new();
    while let Some(row) = rows.next().map_err(storage)? {
        let source: String = row.get(0).map_err(storage)?;
        let entry = map.entry(source).or_insert(RunInfo {
            run_id: row.get(1).map_err(storage)?,
            status: row.get(2).map_err(storage)?,
            created_at: row.get(3).map_err(storage)?,
            completed_at: row.get(4).map_err(storage)?,
            counts: ItemCounts::default(),
            source_result: None,
        });
        let status: String = row.get(5).map_err(storage)?;
        let count: i64 = row.get(6).map_err(storage)?;
        entry.counts.total += count;
        match status.as_str() {
            "completed" => entry.counts.completed += count,
            "failed" | "orphaned" => entry.counts.failed += count,
            "cancelled" => entry.counts.cancelled += count,
            "skipped_unconfigured" => entry.counts.unconfigured += count,
            "blocked_quota" => entry.counts.quota += count,
            "blocked_authorization" => entry.counts.authorization += count,
            _ => entry.counts.open += count,
        }
    }
    for (source, info) in map.iter_mut() {
        info.source_result = connection
            .query_row(
                "SELECT status FROM exposure_source_results WHERE run_id=?1 AND source_key=?2",
                params![info.run_id, source],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?;
    }
    Ok(map)
}

/// The row-level state for one source capability, from configuration and its latest round.
fn row_state(support: Support, enabled: bool, run: Option<&RunInfo>) -> (&'static str, String) {
    match support {
        Support::Unsupported(reason) => return ("not_implemented", reason.to_string()),
        Support::IntegrationPending(reason) if run.is_none() => return ("not_implemented", reason.to_string()),
        _ => {}
    }
    let Some(run) = run else {
        return if enabled {
            ("not_run", "已启用，但还没有一轮采集计划过这个来源".to_string())
        } else {
            ("unconfigured", "本项目未启用这个来源".to_string())
        };
    };
    let counts = &run.counts;
    if matches!(run.status.as_str(), "queued" | "running") {
        return ("planned", format!("第 {} 轮进行中：{} / {} 个工作项已完成", run.run_id, counts.completed, counts.total));
    }
    if counts.total > 0 && counts.unconfigured == counts.total {
        return ("unconfigured", format!("第 {} 轮里这个来源未配置（凭据或地址缺失），全部跳过", run.run_id));
    }
    let effective = counts.total - counts.unconfigured;
    if counts.authorization > 0 && counts.completed == 0 {
        return ("pending_authorization", format!("第 {} 轮有 {} 个工作项等待授权", run.run_id, counts.authorization));
    }
    if matches!(run.status.as_str(), "cancelled" | "interrupted") && counts.completed < effective {
        return ("cancelled_interrupted", format!("第 {} 轮已{}，{} / {} 个工作项完成", run.run_id, if run.status == "cancelled" { "取消" } else { "中断" }, counts.completed, effective));
    }
    if effective > 0 && counts.completed == effective && counts.unconfigured > 0 {
        return ("partial", format!("第 {} 轮完成 {} 个工作项，另有 {} 个未配置跳过：未配置部分是缺口，不算已执行", run.run_id, effective, counts.unconfigured));
    }
    if let Some(result) = run.source_result.as_deref().filter(|result| !matches!(*result, "completed" | "completed_no_matches")) {
        if counts.completed > 0 {
            return ("partial", format!("第 {} 轮工作项完成 {} / {}，但来源结果是 {result}，不算已执行", run.run_id, counts.completed, effective));
        }
    }
    if effective > 0 && counts.completed == effective {
        return ("executed", format!("第 {} 轮 {} 个工作项全部完成", run.run_id, effective));
    }
    if counts.completed == 0 {
        return ("failed", format!("第 {} 轮这个来源没有一个工作项完成（失败 {}，配额阻塞 {}）", run.run_id, counts.failed, counts.quota));
    }
    ("partial", format!("第 {} 轮完成 {} / {}，失败 {}，配额阻塞 {}，授权阻塞 {}，取消 {}", run.run_id, counts.completed, effective, counts.failed, counts.quota, counts.authorization, counts.cancelled))
}

fn tone(state: &str) -> &'static str {
    match state {
        "executed_no_hit" => "green",
        "verified_incident" | "failed" => "red",
        "candidate" | "partial" | "cancelled_interrupted" | "pending_authorization" => "amber",
        _ => "grey",
    }
}

type HitMap = HashMap<(String, String), i64>;

fn hit_counts(connection: &Connection, project_id: i64) -> Result<(HitMap, HitMap, HitMap), String> {
    let mut risks = HashMap::new();
    // FIX4 P1／R3 待分类线索也是未决线索：计入候选，不能让格子显示“已执行无命中”。
    let mut statement = connection
        .prepare("SELECT source_channel, info_category, COUNT(*) FROM exposure_finding_dispositions WHERE project_id=?1 AND disposition IN ('risk_candidate','classification_pending') GROUP BY 1,2")
        .map_err(storage)?;
    let mut rows = statement.query([project_id]).map_err(storage)?;
    while let Some(row) = rows.next().map_err(storage)? {
        risks.insert((row.get::<_, String>(0).map_err(storage)?, row.get::<_, String>(1).map_err(storage)?), row.get::<_, i64>(2).map_err(storage)?);
    }
    let mut verified = HashMap::new();
    let mut candidates = HashMap::new();
    let mut statement = connection
        .prepare(
            "SELECT l.source_key, i.info_category, i.status, COUNT(DISTINCT i.id) FROM exposure_incidents i
             JOIN exposure_incident_locations l ON l.incident_id=i.id AND l.status='active'
             WHERE i.project_id=?1 AND i.status IN ('candidate','reviewing','verified') GROUP BY 1,2,3",
        )
        .map_err(storage)?;
    let mut rows = statement.query([project_id]).map_err(storage)?;
    while let Some(row) = rows.next().map_err(storage)? {
        let key = (row.get::<_, String>(0).map_err(storage)?, row.get::<_, String>(1).map_err(storage)?);
        let status: String = row.get(2).map_err(storage)?;
        let count: i64 = row.get(3).map_err(storage)?;
        let target = if status == "verified" { &mut verified } else { &mut candidates };
        *target.entry(key).or_insert(0) += count;
    }
    Ok((risks, verified, candidates))
}

/// G4-1 the coverage matrix for one project. `projection` is the unit projection the summary
/// already read; `None` shows the unit axis as "未报告", never as zero.
pub fn coverage_matrix(connection: &Connection, project_id: i64, projection: Option<&UnitProjection>, beyond_plan: i64) -> Result<JsonValue, String> {
    super::incidents::check_schema(connection)?;
    let enabled: BTreeSet<String> = match super::settings::load(connection, project_id) {
        Ok(settings) => settings.sources.into_iter().collect(),
        Err(_) => BTreeSet::new(),
    };
    let runs = latest_runs(connection, project_id)?;
    let (risks, verified, candidates) = hit_counts(connection, project_id)?;
    let mut rows = Vec::new();
    let mut state_totals: BTreeMap<String, i64> = BTreeMap::new();
    for row in ROWS {
        let run = match row.support {
            Support::Unsupported(_) => None,
            _ => runs.get(row.source_key),
        };
        let (mut state, mut reason) = row_state(row.support, enabled.contains(row.source_key), run);
        // FIX4 P1／R3 计划外缺口参与状态：被动网页有计划外待查项时，本行不能是“已执行”。
        if row.source_key == "passive_web" && beyond_plan > 0 && state == "executed" {
            state = "partial";
            reason = format!("{reason}；另有计划外 {beyond_plan} 项未执行，是缺口，不算已执行");
        }
        let counts = run.map(|info| info.counts.clone()).unwrap_or_default();
        let effective = counts.total - counts.unconfigured;
        let mut cells = Vec::new();
        for category in ALL {
            let cell_state = if !row.categories.contains(category) {
                "not_applicable"
            } else if matches!(state, "executed" | "partial") {
                let key = (row.channel.to_string(), category.to_string());
                let verified_here = verified.get(&(row.source_key.to_string(), category.to_string())).copied().unwrap_or(0);
                let candidate_here = candidates.get(&(row.source_key.to_string(), category.to_string())).copied().unwrap_or(0)
                    + risks.get(&key).copied().unwrap_or(0);
                if verified_here > 0 {
                    "verified_incident"
                } else if candidate_here > 0 {
                    "candidate"
                } else if state == "executed" {
                    "executed_no_hit"
                } else {
                    "partial"
                }
            } else {
                state
            };
            *state_totals.entry(cell_state.to_string()).or_insert(0) += 1;
            cells.push(json!({
                "infoCategory": category,
                "state": cell_state,
                "label": state_label(cell_state),
                "tone": tone(cell_state),
                "verifiedIncidents": verified.get(&(row.source_key.to_string(), category.to_string())).copied().unwrap_or(0),
                "candidateIncidents": candidates.get(&(row.source_key.to_string(), category.to_string())).copied().unwrap_or(0),
                "riskLeads": risks.get(&(row.channel.to_string(), category.to_string())).copied().unwrap_or(0),
            }));
        }
        let integration = match row.support {
            Support::IntegrationPending(_) => "integration_pending",
            Support::Unsupported(_) => "unsupported",
            Support::Supported => "available",
        };
        rows.push(json!({
            "capability": row.capability,
            "sourceKey": row.source_key,
            "sourceChannel": row.channel,
            "integration": integration,
            "enabled": enabled.contains(row.source_key),
            "state": state,
            "label": state_label(state),
            "reason": reason,
            "denominator": {
                "unit": row.unit,
                "planned": effective.max(0),
                "executed": counts.completed,
                "uncovered": (effective - counts.completed).max(0),
                "failed": counts.failed,
                "blockedQuota": counts.quota,
                "blockedAuthorization": counts.authorization,
                "cancelled": counts.cancelled,
                "open": counts.open,
                "skippedUnconfigured": counts.unconfigured,
                "outsidePlan": if row.source_key == "passive_web" { beyond_plan } else { 0 },
            },
            "window": run.map(|info| json!({"runId": info.run_id, "runStatus": info.status, "createdAt": info.created_at, "completedAt": info.completed_at})),
            "cells": cells,
        }));
    }
    let unit_axis = match projection {
        None => json!({"state": "not_reported", "label": "未报告", "reason": "单位投影读取失败，覆盖不按单位显示，也不显示为 0"}),
        Some(projection) => json!({
            "state": "reported",
            "bindingStatus": projection.binding_status,
            "unit": projection.unit,
            "group": projection.group,
            "unitBucket": projection.unit_bucket,
            "province": projection.unit_province,
            "firstParty": {
                "unit": "资产",
                "eligible": projection.counts.eligible,
                "planned": projection.counts.planned,
                "executed": projection.counts.executed,
                "uncovered": projection.counts.uncovered,
                "latestRunId": projection.counts.latest_run_id,
            },
            "externalSources": "证书/归档按项目域名、代码平台按集团名称查询，不按单位拆分",
            "perAssetEdge": projection.per_asset_edge,
        }),
    };
    Ok(json!({
        "policyVersion": COVERAGE_POLICY_VERSION,
        "infoCategories": ALL,
        "rows": rows,
        "stateTotals": state_totals,
        "unitAxis": unit_axis,
        "rule": "只有全部工作项完成且无命中的格子才是 green；失败、阻塞、取消、超出上限都留在分母里",
    }))
}

fn level_rank(level: &str) -> i32 {
    match level {
        "confirmed_link" => 3,
        "conflict" => 2,
        "candidate_link" => 1,
        _ => 0,
    }
}

/// G4-1 cross-unit statistics for one project, deduplicated at group level.
pub fn unit_statistics(connection: &Connection, project_id: i64, projection: Option<&UnitProjection>) -> Result<JsonValue, String> {
    super::incidents::check_schema(connection)?;
    let mut by_status: BTreeMap<String, i64> = BTreeMap::new();
    {
        let mut statement = connection
            .prepare("SELECT status, COUNT(*) FROM exposure_incidents WHERE project_id=?1 AND status<>'merged' GROUP BY status")
            .map_err(storage)?;
        let mut rows = statement.query([project_id]).map_err(storage)?;
        while let Some(row) = rows.next().map_err(storage)? {
            by_status.insert(row.get(0).map_err(storage)?, row.get(1).map_err(storage)?);
        }
    }
    let open_statuses = ["candidate", "reviewing", "verified"];
    let group_unique_open: i64 = open_statuses.iter().map(|status| by_status.get(*status).copied().unwrap_or(0)).sum();
    let group_unique_all: i64 = by_status.values().sum();
    // Strongest active link per (incident, entity) among open incidents.
    let mut best: HashMap<(i64, i64), String> = HashMap::new();
    {
        // IR-02/IR-11: a projection link counts only while the ownership projection still
        // gives that asset that unit (a revert, retirement or rebinding drops it at once).
        let current: HashMap<i64, i64> = crate::commands::ownership_asset_unit_projection(connection, project_id)
            .map_err(storage)?
            .edges
            .into_iter()
            .map(|edge| (edge.asset_id, edge.owner_entity_id))
            .collect();
        let mut statement = connection
            .prepare(
                "SELECT e.incident_id, e.entity_id, e.link_level, e.origin, json_extract(e.evidence_json,'$[0].assetId') FROM exposure_incident_entities e
                 JOIN exposure_incidents i ON i.id=e.incident_id AND i.project_id=?1 AND i.status IN ('candidate','reviewing','verified')
                 WHERE e.project_id=?1 AND e.active=1",
            )
            .map_err(storage)?;
        let mut rows = statement.query([project_id]).map_err(storage)?;
        while let Some(row) = rows.next().map_err(storage)? {
            let key = (row.get::<_, i64>(0).map_err(storage)?, row.get::<_, i64>(1).map_err(storage)?);
            let level: String = row.get(2).map_err(storage)?;
            let origin: String = row.get(3).map_err(storage)?;
            let asset: Option<i64> = row.get(4).map_err(storage)?;
            if origin == "ownership_projection" && asset.and_then(|asset| current.get(&asset)) != Some(&key.1) {
                continue;
            }
            let replace = best.get(&key).is_none_or(|current| level_rank(&level) > level_rank(current));
            if replace {
                best.insert(key, level);
            }
        }
    }
    let entities = crate::commands::load_org_entities(connection).map_err(storage)?;
    let by_id: HashMap<i64, &crate::commands::OrgEntityView> = entities.iter().map(|entity| (entity.id, entity)).collect();
    // entity -> level -> incidents
    let mut per_entity: BTreeMap<i64, BTreeMap<String, BTreeSet<i64>>> = BTreeMap::new();
    let mut linked_incidents: BTreeSet<i64> = BTreeSet::new();
    for ((incident, entity), level) in &best {
        if level == "unknown" {
            continue;
        }
        linked_incidents.insert(*incident);
        per_entity.entry(*entity).or_default().entry(level.clone()).or_default().insert(*incident);
    }
    let mut units = Vec::new();
    let mut professional = Vec::new();
    let mut other = Vec::new();
    let mut provinces: BTreeMap<i64, (String, BTreeMap<String, BTreeSet<i64>>)> = BTreeMap::new();
    let mut unit_sum = 0i64;
    for (entity_id, levels) in &per_entity {
        let count = |level: &str| levels.get(level).map(|set| set.len() as i64).unwrap_or(0);
        let involvement = count("confirmed_link") + count("candidate_link") + count("conflict");
        unit_sum += involvement;
        let (bucket, province, note, name) = match by_id.get(entity_id) {
            Some(entity) => {
                let (bucket, province, note) = place_unit(entity, &by_id);
                (bucket, province, note, entity.legal_name.clone())
            }
            None => ("missing".to_string(), None, Some("实体已不在组织目录中".to_string()), format!("实体 {entity_id}")),
        };
        let row = json!({
            "entityId": entity_id,
            "legalName": name,
            "bucket": bucket,
            "province": province,
            "note": note,
            "confirmed": count("confirmed_link"),
            "candidate": count("candidate_link"),
            "conflict": count("conflict"),
        });
        if let Some(province) = &province {
            let entry = provinces.entry(province.entity_id).or_insert_with(|| (province.legal_name.clone(), BTreeMap::new()));
            for (level, incidents) in levels {
                entry.1.entry(level.clone()).or_default().extend(incidents.iter().copied());
            }
        }
        match bucket.as_str() {
            "professional_subsidiary" => professional.push(row),
            "province" | "city" | "county" => units.push(row),
            _ => other.push(row),
        }
    }
    let province_rows: Vec<JsonValue> = provinces
        .into_iter()
        .map(|(id, (name, levels))| {
            let count = |level: &str| levels.get(level).map(|set| set.len() as i64).unwrap_or(0);
            let all: BTreeSet<i64> = levels.values().flatten().copied().collect();
            json!({"provinceId": id, "legalName": name, "confirmed": count("confirmed_link"), "candidate": count("candidate_link"), "conflict": count("conflict"), "uniqueIncidents": all.len()})
        })
        .collect();
    let open_ids: BTreeSet<i64> = {
        let mut statement = connection
            .prepare("SELECT id FROM exposure_incidents WHERE project_id=?1 AND status IN ('candidate','reviewing','verified')")
            .map_err(storage)?;
        let rows = statement
            .query_map([project_id], |row| row.get::<_, i64>(0))
            .map_err(storage)?
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(storage)?;
        rows
    };
    let unknown_unit = open_ids.difference(&linked_incidents).count() as i64;
    let (risk_leads, located_findings, active_locations): (i64, i64, i64) = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM exposure_finding_dispositions WHERE project_id=?1 AND disposition='risk_candidate'),
                    (SELECT COUNT(DISTINCT l.finding_id) FROM exposure_incident_locations l JOIN exposure_incidents i ON i.id=l.incident_id WHERE l.project_id=?1 AND l.status='active' AND i.status<>'merged'),
                    (SELECT COUNT(*) FROM exposure_incident_locations l JOIN exposure_incidents i ON i.id=l.incident_id WHERE l.project_id=?1 AND l.status='active' AND i.status<>'merged')",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(storage)?;
    let assets = match projection {
        None => json!({"state": "not_reported", "label": "未报告"}),
        Some(projection) => json!({
            "state": "reported",
            "bindingStatus": projection.binding_status,
            "members": projection.counts.members,
            "candidateLinked": projection.counts.candidate_linked,
            "confirmedGroup": projection.counts.confirmed_group,
            "confirmedUnit": projection.counts.confirmed_unit,
            "unassigned": projection.counts.unassigned,
            "pendingAuthorization": projection.counts.pending_authorization,
            "eligible": projection.counts.eligible,
            "planned": projection.counts.planned,
            "executed": projection.counts.executed,
            "uncovered": projection.counts.uncovered,
            "perAssetEdge": projection.per_asset_edge,
            "ownershipVersion": projection.ownership_version,
            "perUnit": projection.per_unit,
            "unassignedReasons": projection.unassigned_reasons,
        }),
    };
    Ok(json!({
        "policyVersion": COVERAGE_POLICY_VERSION,
        "groupUnique": {"open": group_unique_open, "all": group_unique_all, "byStatus": by_status},
        "units": units,
        "professionalSubsidiaries": professional,
        "otherEntities": other,
        "provinces": province_rows,
        "unknownUnitIncidents": unknown_unit,
        "sumOfUnitInvolvement": unit_sum,
        "involvementNote": "单位涉及数之和按单位各计一次，一起事件涉及多个单位时会大于集团唯一事件数；集团总数只按事件唯一计",
        "dedup": {"riskLeads": risk_leads, "findingsInIncidents": located_findings, "activeLocations": active_locations, "openIncidents": group_unique_open},
        "assets": assets,
        "unitLinkBasis": "confirmed_link 只来自单位项目绑定加已确认资产归属、集团总账的归属模块逐资产投影（人工确认的目录复核组），或人工附证据确认；没有权威单位依据的事件进入“单位未知”",
    }))
}

/// P1 UI 闭环：覆盖矩阵某一行的钻取。`window_days` 为 0 时只看这个来源最近一轮，否则看最近 N 天
/// 里的每一轮（时间窗口维度）；`unit_entity_id` 按归属模块逐资产投影筛选单位（0 = 投影里没有
/// 单位的资产，None = 不筛）。外部来源的工作项按域名/关键词计划，不对应资产，单位维度如实写
/// “不按资产计划”，不分摊到任何单位。工作项按 id 键集分页。
#[allow(clippy::too_many_arguments)]
pub fn coverage_drilldown(
    connection: &Connection,
    project_id: i64,
    source_key: &str,
    window_days: i64,
    unit_entity_id: Option<i64>,
    gaps_only: bool,
    size: i64,
    cursor: &str,
) -> Result<JsonValue, String> {
    super::incidents::check_schema(connection)?;
    if !ROWS.iter().any(|row| row.source_key == source_key) {
        return Err(format!("[exposure_coverage_source_invalid] 覆盖矩阵里没有来源 {source_key}"));
    }
    if !(0..=366).contains(&window_days) {
        return Err("[exposure_coverage_window_invalid] 时间窗口只能是 0（最近一轮）或 1~366 天".to_string());
    }
    let size = super::incidents::page_size(size)?;
    let bound = super::incidents::filter_hash(&[
        &project_id.to_string(),
        source_key,
        &window_days.to_string(),
        &unit_entity_id.map(|id| id.to_string()).unwrap_or_default(),
        if gaps_only { "gaps" } else { "all" },
    ]);
    let after = super::incidents::parse_cursor(cursor, &bound)?;
    // ① 时间窗口里的轮次（只算计划过这个来源的轮次）。
    let run_ids: Vec<i64> = {
        let sql = if window_days == 0 {
            "SELECT MAX(run_id) FROM exposure_work_items WHERE project_id=?1 AND source_key=?2 HAVING MAX(run_id) IS NOT NULL"
        } else {
            "SELECT DISTINCT w.run_id FROM exposure_work_items w JOIN exposure_runs r ON r.id=w.run_id AND r.project_id=?1
             WHERE w.project_id=?1 AND w.source_key=?2 AND r.created_at>=datetime('now','localtime',?3) ORDER BY w.run_id DESC"
        };
        let mut statement = connection.prepare(sql).map_err(storage)?;
        let offset = format!("-{window_days} days");
        let rows = if window_days == 0 {
            statement.query_map(params![project_id, source_key], |row| row.get::<_, i64>(0)).map_err(storage)?.collect::<Result<Vec<_>, _>>()
        } else {
            statement.query_map(params![project_id, source_key, offset], |row| row.get::<_, i64>(0)).map_err(storage)?.collect::<Result<Vec<_>, _>>()
        };
        rows.map_err(storage)?
    };
    let runs_json = json!(run_ids).to_string();
    let mut rounds = Vec::new();
    for run_id in &run_ids {
        let (status, created_at, completed_at): (String, String, String) = connection
            .query_row("SELECT status,created_at,COALESCE(completed_at,'') FROM exposure_runs WHERE id=?1", [run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .map_err(storage)?;
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        let mut statement = connection
            .prepare("SELECT status,COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND source_key=?2 GROUP BY status")
            .map_err(storage)?;
        let mut rows = statement.query(params![run_id, source_key]).map_err(storage)?;
        while let Some(row) = rows.next().map_err(storage)? {
            counts.insert(row.get(0).map_err(storage)?, row.get(1).map_err(storage)?);
        }
        let source_result: Option<String> = connection
            .query_row("SELECT status FROM exposure_source_results WHERE run_id=?1 AND source_key=?2", params![run_id, source_key], |row| row.get(0))
            .optional()
            .map_err(storage)?;
        rounds.push(json!({"runId": run_id, "runStatus": status, "createdAt": created_at, "completedAt": completed_at, "itemCounts": counts, "sourceResult": source_result}));
    }
    // ② 单位维度：归属模块逐资产投影（只读）。读不到就写“未报告”，不显示为 0。
    let asset_scoped = source_key == "passive_web";
    let mut asset_units: HashMap<i64, i64> = HashMap::new();
    let mut unit_axis = json!({"state": "not_asset_scoped", "label": "这个来源按域名/关键词计划，不对应单个资产，不按单位分摊"});
    if asset_scoped {
        match crate::commands::ownership_asset_unit_projection(connection, project_id) {
            Ok(projection) => {
                for edge in &projection.edges {
                    asset_units.insert(edge.asset_id, edge.owner_entity_id);
                }
                let names: HashMap<i64, String> = crate::commands::load_org_entities(connection)
                    .map_err(storage)?
                    .into_iter()
                    .map(|entity| (entity.id, entity.legal_name))
                    .collect();
                let mut per_unit: BTreeMap<i64, (i64, i64)> = BTreeMap::new();
                let mut statement = connection
                    .prepare(
                        "SELECT asset_id,status FROM exposure_work_items WHERE project_id=?1 AND source_key=?2
                         AND run_id IN (SELECT value FROM json_each(?3))",
                    )
                    .map_err(storage)?;
                let mut rows = statement.query(params![project_id, source_key, runs_json]).map_err(storage)?;
                while let Some(row) = rows.next().map_err(storage)? {
                    let asset: Option<i64> = row.get(0).map_err(storage)?;
                    let status: String = row.get(1).map_err(storage)?;
                    let unit = asset.and_then(|asset| asset_units.get(&asset).copied()).unwrap_or(0);
                    let entry = per_unit.entry(unit).or_insert((0, 0));
                    entry.0 += 1;
                    if status == "completed" {
                        entry.1 += 1;
                    }
                }
                let rows: Vec<JsonValue> = per_unit
                    .iter()
                    .map(|(unit, (total, completed))| {
                        json!({
                            "entityId": unit,
                            "legalName": if *unit == 0 { "未归属到单位".to_string() } else { names.get(unit).cloned().unwrap_or_else(|| format!("实体 {unit}")) },
                            "total": total,
                            "completed": completed,
                            "gaps": total - completed,
                        })
                    })
                    .collect();
                unit_axis = json!({"state": "reported", "projectionVersion": projection.version, "rows": rows});
            }
            Err(error) => {
                unit_axis = json!({"state": "not_reported", "label": "未报告", "reason": error});
            }
        }
    }
    if unit_entity_id.is_some() && !asset_scoped {
        return Err("[exposure_coverage_unit_unsupported] 这个来源不按资产计划，不能按单位筛选".to_string());
    }
    if unit_entity_id.is_some() && unit_axis["state"] != "reported" {
        return Err("[exposure_coverage_unit_unavailable] 单位投影未报告，不能按单位筛选".to_string());
    }
    // ③ 工作项明细（键集分页）。按单位筛选时先把单位换成它的资产集合。
    let unit_assets: Option<String> = unit_entity_id.map(|unit| {
        if unit == 0 {
            json!(asset_units.keys().collect::<Vec<_>>()).to_string()
        } else {
            json!(asset_units.iter().filter(|(_, owner)| **owner == unit).map(|(asset, _)| *asset).collect::<Vec<_>>()).to_string()
        }
    });
    let unit_clause = match unit_entity_id {
        None => "",
        Some(0) => " AND (w.asset_id IS NULL OR w.asset_id NOT IN (SELECT value FROM json_each(?5)))",
        Some(_) => " AND w.asset_id IN (SELECT value FROM json_each(?5))",
    };
    let gap_clause = if gaps_only { " AND w.status<>'completed'" } else { "" };
    let base = format!(
        "FROM exposure_work_items w WHERE w.project_id=?1 AND w.source_key=?2 AND w.run_id IN (SELECT value FROM json_each(?3)) AND json_valid(?5){gap_clause}{unit_clause}"
    );
    let assets_param = unit_assets.unwrap_or_else(|| "[]".to_string());
    let total: i64 = connection
        .query_row(&format!("SELECT COUNT(*) {base} AND ?4>=0"), params![project_id, source_key, runs_json, 0, assets_param], |row| row.get(0))
        .map_err(storage)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT w.id,w.run_id,w.status,w.reason,w.entity,w.origin,w.asset_id,w.requests,w.finished_at {base} AND w.id>?4 ORDER BY w.id LIMIT ?6"
        ))
        .map_err(storage)?;
    let items: Vec<JsonValue> = statement
        .query_map(params![project_id, source_key, runs_json, after.unwrap_or(0), assets_param, size + 1], |row| {
            let asset: Option<i64> = row.get(6)?;
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "runId": row.get::<_, i64>(1)?,
                "status": row.get::<_, String>(2)?,
                "reason": row.get::<_, String>(3)?,
                "entity": row.get::<_, String>(4)?,
                "origin": row.get::<_, String>(5)?,
                "assetId": asset,
                "unitEntityId": asset.and_then(|asset| asset_units.get(&asset).copied()),
                "requests": row.get::<_, i64>(7)?,
                "finishedAt": row.get::<_, String>(8)?,
            }))
        })
        .map_err(storage)?
        .collect::<Result<_, _>>()
        .map_err(storage)?;
    let has_more = items.len() as i64 > size;
    let items: Vec<JsonValue> = items.into_iter().take(size as usize).collect();
    let next = if has_more {
        items.last().and_then(|row| row["id"].as_i64()).map(|id| format!("v1:{bound}:{id}")).unwrap_or_default()
    } else {
        String::new()
    };
    Ok(json!({
        "sourceKey": source_key,
        "window": {"days": window_days, "label": if window_days == 0 { "最近一轮".to_string() } else { format!("最近 {window_days} 天") }, "runIds": run_ids},
        "rounds": rounds,
        "unitAxis": unit_axis,
        "gapsOnly": gaps_only,
        "items": items,
        "total": total,
        "pageSize": size,
        "hasMore": has_more,
        "nextCursor": next,
    }))
}
