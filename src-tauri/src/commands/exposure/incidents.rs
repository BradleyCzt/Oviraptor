//! G4-0 泄漏事件与传播位置。
//!
//! - 自动只做“建议”：本项目里规范化全文（文档）或完整未截断响应体的哈希相同、且未被排除的线索，
//!   归成一个 `candidate` 事件的多个位置；单独一条 `risk_candidate` 线索成为单位置候选事件。
//!   标题、文件名、域名或脱敏片段相同都不合并。摘要命中、索引行不参与内容比对。
//! - 状态只由人工改（review / merge / split / link_entity / record_action），每个动作带
//!   projectId、expectedRevision、operationId、actor、reason，CAS 与写入在同一 IMMEDIATE 事务。
//! - 通知、下架、整改只“登记”人工已经做过的动作；本模块没有任何出站请求。
//! - 事件涉及单位只引用组织目录里已有的实体；`confirmed_link` 只来自单位项目绑定加已确认的
//!   资产归属，或人工附证据确认。不改 assets 归属，不新建实体。

use super::redaction::{content_hash, redact_text};
use super::state::{ensure_exposure_schema, utc_now};
use super::transaction::ImmediateTx;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const INCIDENT_SCHEMA_VERSION: i64 = 1;
pub const INCIDENT_POLICY_VERSION: &str = "oviraptor.exposure.incidents.v1";
pub const STATUSES: &[&str] = &["candidate", "reviewing", "verified", "dismissed", "excluded_public", "merged"];
pub const LINK_LEVELS: &[&str] = &["confirmed_link", "candidate_link", "unknown", "conflict"];
/// Human-recorded handling. None of these contact anyone: they record what a person did.
pub const HANDLING_ACTIONS: &[&str] = &[
    "notification_recorded",
    "takedown_requested_recorded",
    "takedown_confirmed_recorded",
    "remediation_recorded",
    "retest_linked",
    "closed_recorded",
    "reopened",
];
pub const PAGE_SIZE_MAX: i64 = 200;
const IN_CHUNK: usize = 400;

/// 旧 finding.status 的对应关系（不改写历史枚举，只在读取时说明）。
pub fn legacy_finding_status(status: &str) -> &'static str {
    match status {
        "candidate" => "new",
        "reviewing" => "reviewing",
        "verified" => "confirmed",
        "dismissed" | "excluded_public" => "dismissed",
        _ => "",
    }
}

fn storage(error: impl std::fmt::Display) -> String {
    format!("[exposure_incident_storage] {error}")
}

fn invalid(message: impl std::fmt::Display) -> String {
    format!("[exposure_incident_invalid] {message}")
}

/// `CREATE IF NOT EXISTS` cannot notice an old table that lacks a column, so the readers and
/// writers check the columns they rely on and refuse instead of returning an empty answer.
pub fn check_schema(connection: &Connection) -> Result<(), String> {
    let required: &[(&str, &[&str])] = &[
        ("exposure_incidents", &["schema_version", "stable_key", "status", "revision", "review_needed", "merged_into", "handling_state", "policy_version"]),
        ("exposure_incident_locations", &["incident_id", "finding_id", "observation_id", "content_key", "status", "collection_mode"]),
        ("exposure_incident_entities", &["incident_id", "entity_id", "link_level", "origin", "active"]),
        ("exposure_incident_events", &["operation_id", "payload_hash", "action", "result_json"]),
    ];
    for (table, columns) in required {
        let mut statement = connection.prepare(&format!("PRAGMA table_info({table})")).map_err(storage)?;
        let present: BTreeSet<String> = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(storage)?
            .collect::<Result<_, _>>()
            .map_err(storage)?;
        if present.is_empty() {
            return Err(format!("[exposure_incident_schema_mismatch] 表 {table} 不存在"));
        }
        let missing: Vec<&str> = columns.iter().copied().filter(|column| !present.contains(*column)).collect();
        if !missing.is_empty() {
            return Err(format!("[exposure_incident_schema_mismatch] 表 {table} 缺少列 {}，拒绝按空结果继续", missing.join("、")));
        }
    }
    Ok(())
}

pub fn location_kind(source_key: &str, kind: &str) -> &'static str {
    match (source_key, kind) {
        ("passive_web", _) => "first_party_url",
        ("certificate_transparency", _) => "certificate_index",
        ("web_archive", _) => "archive_index",
        ("github_repositories", _) => "repository",
        ("github_code", "github_blob_read") => "repository_file",
        ("github_code", _) => "repository_file_snippet",
        ("document_link", _) => "document_url",
        ("document_import", _) => "imported_file",
        ("public_intel_import", _) => "intel_report",
        _ => "other",
    }
}

fn default_mode(source_key: &str) -> &'static str {
    match source_key {
        "passive_web" | "document_link" => "http_read",
        "certificate_transparency" | "web_archive" => "index",
        "github_repositories" | "github_code" => "api",
        "document_import" | "public_intel_import" => "import",
        _ => "unknown",
    }
}

/// The strongest evidence an incident's locations carry, for the page's "实际读取还是索引/导入".
pub fn evidence_grade(modes: &BTreeSet<String>, kinds: &BTreeSet<String>) -> &'static str {
    if modes.iter().any(|mode| matches!(mode.as_str(), "http_read" | "blob_read")) {
        "observed_content"
    } else if kinds.contains("intel_report") {
        "claim"
    } else if modes.iter().any(|mode| mode.starts_with("import")) {
        "imported"
    } else if modes.iter().any(|mode| mode == "index" || mode == "repository_metadata") {
        "index_only"
    } else {
        "snippet_or_metadata"
    }
}

/// One finding with its latest observation, the fields the proposal needs.
#[derive(Clone, Debug)]
struct LeadRow {
    finding_id: i64,
    observation_id: i64,
    run_id: i64,
    content_key: String,
    disposition: String,
    info_category: String,
    url: String,
    title: String,
    source_key: String,
    kind: String,
    asset_id: Option<i64>,
    collection_mode: String,
}

const LEAD_SELECT: &str = "SELECT l.finding_id, o.id, o.run_id,
        CASE WHEN COALESCE(json_extract(o.evidence_json,'$.detail.normalizedTextSha256'),'')<>''
               THEN 'text:'||json_extract(o.evidence_json,'$.detail.normalizedTextSha256')
             WHEN o.truncated=0 AND o.body_bytes>0 AND o.content_hash<>''
                  AND o.source_key NOT IN ('certificate_transparency','web_archive','github_repositories')
                  AND o.kind NOT IN ('github_code_hit','github_search_page','document_metadata')
               THEN 'body:'||o.content_hash
             ELSE '' END,
        COALESCE(d.disposition,''), COALESCE(d.info_category,''), f.source_url, f.title, o.source_key, o.kind, o.asset_id,
        COALESCE(json_extract(o.evidence_json,'$.detail.collectionMode'),'')
     FROM latest l CROSS JOIN exposure_observations o ON o.id=l.obs AND o.project_id=?1
     CROSS JOIN exposure_findings f ON f.id=l.finding_id AND f.project_id=?1
     LEFT JOIN exposure_finding_dispositions d ON d.finding_id=l.finding_id";
// FIX2：`CROSS JOIN` 固定连接顺序 latest → 观察 → 线索（都按主键取）。没有统计信息时 SQLite 曾把它
// 规划成“按项目扫观察 × 按项目扫线索”，单轮建议的成本随全项目平方增长。

fn lead_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<LeadRow> {
    let source_key: String = row.get(8)?;
    let mode: String = row.get(11)?;
    Ok(LeadRow {
        finding_id: row.get(0)?,
        observation_id: row.get(1)?,
        run_id: row.get(2)?,
        content_key: row.get(3)?,
        disposition: row.get(4)?,
        info_category: row.get(5)?,
        url: row.get(6)?,
        title: row.get(7)?,
        collection_mode: if mode.is_empty() { default_mode(&source_key).to_string() } else { mode },
        source_key,
        kind: row.get(9)?,
        asset_id: row.get(10)?,
    })
}

fn leads_where(connection: &Connection, project_id: i64, finding_filter: &str, extra: &[i64]) -> Result<Vec<LeadRow>, String> {
    let sql = format!(
        "WITH latest AS (SELECT fo.finding_id, MAX(fo.observation_id) AS obs FROM exposure_finding_observations fo
                         WHERE fo.project_id=?1 {finding_filter} GROUP BY fo.finding_id)
         {LEAD_SELECT}"
    );
    let mut statement = connection.prepare(&sql).map_err(storage)?;
    let mut values: Vec<i64> = vec![project_id];
    values.extend_from_slice(extra);
    let rows = statement
        .query_map(params_from_iter(values.iter()), lead_from)
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}

fn leads_by_ids(connection: &Connection, project_id: i64, ids: &[i64]) -> Result<Vec<LeadRow>, String> {
    let mut all = Vec::new();
    for chunk in ids.chunks(IN_CHUNK) {
        let marks = (0..chunk.len()).map(|index| format!("?{}", index + 2)).collect::<Vec<_>>().join(",");
        all.extend(leads_where(connection, project_id, &format!("AND fo.finding_id IN ({marks})"), chunk)?);
    }
    Ok(all)
}

/// Findings of the project whose observation carries the same strict content key.
fn peers_of(connection: &Connection, project_id: i64, key: &str) -> Result<Vec<i64>, String> {
    let (sql, value) = if let Some(hash) = key.strip_prefix("text:") {
        (
            "SELECT DISTINCT fo.finding_id FROM exposure_observations o JOIN exposure_finding_observations fo ON fo.observation_id=o.id
             WHERE o.project_id=?1 AND json_extract(o.evidence_json,'$.detail.normalizedTextSha256')=?2",
            hash,
        )
    } else if let Some(hash) = key.strip_prefix("body:") {
        (
            "SELECT DISTINCT fo.finding_id FROM exposure_observations o JOIN exposure_finding_observations fo ON fo.observation_id=o.id
             WHERE o.project_id=?1 AND o.content_hash=?2 AND o.truncated=0 AND o.body_bytes>0",
            hash,
        )
    } else {
        return Ok(Vec::new());
    };
    let mut statement = connection.prepare(sql).map_err(storage)?;
    let rows = statement
        .query_map(params![project_id, value], |row| row.get::<_, i64>(0))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}

fn eligible(row: &LeadRow) -> bool {
    matches!(row.disposition.as_str(), "risk_candidate" | "classification_pending")
}

/// The unit link a location's asset can justify from the project binding alone.
struct BindingContext {
    unit_entity_id: Option<i64>,
    /// IR-02/IR-11: a group-ledger project reads the ownership module's per-asset edge.
    group_ledger: bool,
    directory_version: i64,
    binding_version: i64,
}

fn binding_context(connection: &Connection, project_id: i64) -> Result<BindingContext, String> {
    let directory_version = crate::commands::ownership::org_directory_version(connection).map_err(storage)?;
    let entities = crate::commands::ownership::load_org_entities(connection).map_err(storage)?;
    let binding = crate::commands::ownership::load_project_org_binding(connection, project_id).map_err(storage)?;
    let mut context = BindingContext { unit_entity_id: None, group_ledger: false, directory_version, binding_version: 0 };
    if let Some(row) = binding {
        context.binding_version = row.binding_version;
        if row.project_kind == "unit" {
            context.unit_entity_id = row.unit_entity_id.filter(|id| entities.iter().any(|entity| entity.id == *id));
        } else if row.project_kind == "group_ledger" {
            context.group_ledger = entities.iter().any(|entity| entity.id == row.group_entity_id);
        }
    }
    Ok(context)
}

/// IR-02/IR-11: in a group-ledger project the unit of one asset is read from the ownership
/// module's projection (a person confirmed a directory-backed review group). No edge, no link:
/// the incident stays "unit unknown"; nothing is inferred from names or region codes. An older
/// projection link for this asset that no longer holds is deactivated, never kept as current.
fn auto_link_from_ownership(tx: &Connection, project_id: i64, incident_id: i64, row: &LeadRow, asset_id: i64, context: &BindingContext) -> Result<(), String> {
    let edge = crate::commands::ownership::ownership_asset_unit_edge(tx, project_id, asset_id).map_err(storage)?;
    reconcile_ownership_link(tx, project_id, incident_id, asset_id, row.observation_id, edge.as_ref(), context)
}

/// 一个事件里一个资产的单位关联与归属投影对齐：投影给出的单位没有就补上，不再成立的旧关联置为失效。
fn reconcile_ownership_link(
    tx: &Connection,
    project_id: i64,
    incident_id: i64,
    asset_id: i64,
    observation_id: i64,
    edge: Option<&crate::commands::ownership::OwnershipAssetUnitEdge>,
    context: &BindingContext,
) -> Result<(), String> {
    let keep = edge.map(|edge| edge.owner_entity_id).unwrap_or(0);
    // FIX2：`+project_id` 让规划器按事件索引取这个事件的几条关联，而不是按项目扫全部关联（条件不变）。
    tx.execute(
        "UPDATE exposure_incident_entities SET active=0 WHERE incident_id=?1 AND +project_id=?2 AND origin='ownership_projection' AND active=1
         AND json_extract(evidence_json,'$[0].assetId')=?3 AND entity_id<>?4",
        params![incident_id, project_id, asset_id, keep],
    )
    .map_err(storage)?;
    let Some(edge) = edge else {
        return Ok(());
    };
    let existing: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM exposure_incident_entities WHERE incident_id=?1 AND entity_id=?2 AND origin='ownership_projection' AND active=1
             AND json_extract(evidence_json,'$[0].assetId')=?3",
            params![incident_id, edge.owner_entity_id, asset_id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if existing > 0 {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO exposure_incident_entities(incident_id,project_id,entity_id,link_level,origin,basis,evidence_json,directory_version,binding_version,active,created_at)
         VALUES(?1,?2,?3,'confirmed_link','ownership_projection',?4,?5,?6,?7,1,?8)",
        params![
            incident_id,
            project_id,
            edge.owner_entity_id,
            format!("归属模块逐资产投影（{}）：{}", edge.version, edge.relation),
            json!([{"assetId": asset_id, "observationId": observation_id, "ownership": edge.evidence}]).to_string(),
            context.directory_version,
            context.binding_version,
            utc_now()
        ],
    )
    .map_err(storage)?;
    Ok(())
}

fn auto_link(tx: &Connection, project_id: i64, incident_id: i64, row: &LeadRow, context: &BindingContext) -> Result<(), String> {
    if context.group_ledger {
        if let Some(asset_id) = row.asset_id {
            return auto_link_from_ownership(tx, project_id, incident_id, row, asset_id, context);
        }
        return Ok(());
    }
    let (Some(unit_id), Some(asset_id)) = (context.unit_entity_id, row.asset_id) else {
        return Ok(());
    };
    let decision: Option<String> = tx
        .query_row(
            "SELECT status FROM asset_ownership_decisions WHERE project_id=?1 AND asset_id=?2",
            params![project_id, asset_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    let level = match decision.as_deref() {
        Some("confirmed") => "confirmed_link",
        Some("attributed") | Some("related") => "candidate_link",
        _ => return Ok(()),
    };
    let existing: Option<(i64, String)> = tx
        .query_row(
            "SELECT id,link_level FROM exposure_incident_entities WHERE incident_id=?1 AND entity_id=?2 AND origin='project_binding' AND active=1",
            params![incident_id, unit_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    match existing {
        Some((_, current)) if current == level || current == "confirmed_link" => return Ok(()),
        Some((id, _)) => {
            tx.execute("UPDATE exposure_incident_entities SET active=0 WHERE id=?1", [id]).map_err(storage)?;
        }
        None => {}
    }
    tx.execute(
        "INSERT INTO exposure_incident_entities(incident_id,project_id,entity_id,link_level,origin,basis,evidence_json,directory_version,binding_version,active,created_at)
         VALUES(?1,?2,?3,?4,'project_binding',?5,?6,?7,?8,1,?9)",
        params![
            incident_id,
            project_id,
            unit_id,
            level,
            format!("单位项目绑定 v{} 的单位，资产归属状态 {}", context.binding_version, decision.unwrap_or_default()),
            json!([{"assetId": asset_id, "observationId": row.observation_id}]).to_string(),
            context.directory_version,
            context.binding_version,
            utc_now()
        ],
    )
    .map_err(storage)?;
    Ok(())
}

fn insert_location(tx: &Connection, project_id: i64, incident_id: i64, row: &LeadRow, relation: &str) -> Result<(), String> {
    tx.execute(
        "INSERT INTO exposure_incident_locations(incident_id,project_id,finding_id,observation_id,run_id,location_kind,canonical_url,source_key,collection_mode,relation,content_key,status,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'active',?12)
         ON CONFLICT(incident_id,finding_id) DO UPDATE SET status='active',relation=excluded.relation,observation_id=excluded.observation_id,run_id=excluded.run_id",
        params![
            incident_id,
            project_id,
            row.finding_id,
            row.observation_id,
            row.run_id,
            location_kind(&row.source_key, &row.kind),
            row.url,
            row.source_key,
            row.collection_mode,
            relation,
            row.content_key,
            utc_now()
        ],
    )
    .map_err(storage)?;
    Ok(())
}

struct EventRecord<'a> {
    operation_id: &'a str,
    action: &'a str,
    actor: &'a str,
    reason: &'a str,
    expected_revision: i64,
    from_status: &'a str,
    to_status: &'a str,
    refs: &'a JsonValue,
    result: &'a JsonValue,
    payload_hash: &'a str,
}

fn log_event(tx: &Connection, project_id: i64, incident_id: i64, event: EventRecord<'_>) -> Result<i64, String> {
    tx.execute(
        "INSERT INTO exposure_incident_events(project_id,incident_id,operation_id,action,payload_hash,actor,reason,expected_revision,from_status,to_status,refs_json,result_json,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![
            project_id,
            incident_id,
            event.operation_id,
            event.action,
            event.payload_hash,
            event.actor,
            event.reason,
            event.expected_revision,
            event.from_status,
            event.to_status,
            event.refs.to_string(),
            event.result.to_string(),
            utc_now()
        ],
    )
    .map_err(storage)?;
    Ok(tx.last_insert_rowid())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProposalReport {
    pub created: i64,
    pub attached_locations: i64,
    pub review_flagged: i64,
    pub groups_considered: i64,
}

fn follow_stable_key(tx: &Connection, project_id: i64, stable_key: &str) -> Result<Option<i64>, String> {
    let mut existing: Option<(i64, String, Option<i64>)> = tx
        .query_row(
            "SELECT id,status,merged_into FROM exposure_incidents WHERE project_id=?1 AND stable_key=?2",
            params![project_id, stable_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(storage)?;
    let mut hops = 0;
    while let Some((_, status, Some(next))) = existing.clone() {
        if status != "merged" || hops > 16 {
            break;
        }
        hops += 1;
        existing = tx
            .query_row(
                "SELECT id,status,merged_into FROM exposure_incidents WHERE id=?1 AND project_id=?2",
                params![next, project_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(storage)?;
    }
    Ok(existing.filter(|(_, status, _)| status != "merged").map(|(id, _, _)| id))
}

/// G4-0 automatic proposals for the findings a round touched (or the whole project with
/// `run_id = None`). Only `candidate` incidents are created; a human decision is never
/// changed, a new location on a decided incident only raises `review_needed`.
pub fn propose(connection: &Connection, project_id: i64, run_id: Option<i64>) -> Result<ProposalReport, String> {
    check_schema(connection)?;
    let touched = match run_id {
        Some(run) => leads_where(
            connection,
            project_id,
            "AND fo.finding_id IN (SELECT finding_id FROM exposure_finding_observations WHERE run_id=?2 AND project_id=?1)",
            &[run],
        )?,
        None => leads_where(connection, project_id, "", &[])?,
    };
    let context = binding_context(connection, project_id)?;
    // Group by strict content key. Singles are risk leads without a key.
    let mut groups: BTreeMap<String, Vec<LeadRow>> = BTreeMap::new();
    let mut known: HashMap<i64, ()> = HashMap::new();
    for row in touched.iter().filter(|row| eligible(row)) {
        known.insert(row.finding_id, ());
        let key = if row.content_key.is_empty() { format!("lead:{}", row.finding_id) } else { row.content_key.clone() };
        groups.entry(key).or_default().push(row.clone());
    }
    if run_id.is_some() {
        // Peers from earlier rounds share the key but were not touched now.
        let keys: Vec<String> = groups.keys().filter(|key| !key.starts_with("lead:")).cloned().collect();
        for key in keys {
            let missing: Vec<i64> = peers_of(connection, project_id, &key)?
                .into_iter()
                .filter(|id| !known.contains_key(id))
                .collect();
            for peer in leads_by_ids(connection, project_id, &missing)? {
                if eligible(&peer) && peer.content_key == key {
                    known.insert(peer.finding_id, ());
                    groups.entry(key.clone()).or_default().push(peer);
                }
            }
        }
    }
    let mut report = ProposalReport::default();
    let transaction = ImmediateTx::begin(connection)?;
    let tx = transaction.connection();
    for (key, mut rows) in groups {
        rows.sort_by_key(|row| row.finding_id);
        rows.dedup_by_key(|row| row.finding_id);
        let lead = key.starts_with("lead:");
        let urls: BTreeSet<&str> = rows.iter().map(|row| row.url.as_str()).collect();
        let has_risk = rows.iter().any(|row| row.disposition == "risk_candidate");
        if (!lead && urls.len() < 2 && !has_risk) || (lead && !has_risk) {
            continue;
        }
        report.groups_considered += 1;
        // Which of these findings already sit in a live incident?
        let mut placed: BTreeMap<i64, i64> = BTreeMap::new();
        for row in &rows {
            let incident: Option<i64> = tx
                .query_row(
                    "SELECT l.incident_id FROM exposure_incident_locations l JOIN exposure_incidents i ON i.id=l.incident_id
                     WHERE l.project_id=?1 AND l.finding_id=?2 AND l.status='active' AND i.status<>'merged' ORDER BY l.incident_id LIMIT 1",
                    params![project_id, row.finding_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            if let Some(id) = incident {
                placed.insert(row.finding_id, id);
            }
        }
        let fresh: Vec<&LeadRow> = rows.iter().filter(|row| !placed.contains_key(&row.finding_id)).collect();
        if fresh.is_empty() {
            continue;
        }
        let stable_key = if lead { key.clone() } else { format!("content:{key}") };
        let target = match placed.values().min().copied() {
            Some(id) => Some(id),
            None => follow_stable_key(tx, project_id, &stable_key)?,
        };
        let first_run = fresh.iter().map(|row| row.run_id).min().unwrap_or(0);
        let last_run = rows.iter().map(|row| row.run_id).max().unwrap_or(0);
        let incident_id = match target {
            Some(id) => {
                let status: String = tx
                    .query_row("SELECT status FROM exposure_incidents WHERE id=?1", [id], |r| r.get(0))
                    .map_err(storage)?;
                let flag = status != "candidate";
                tx.execute(
                    "UPDATE exposure_incidents SET revision=revision+1,last_run_id=MAX(last_run_id,?2),updated_at=?3,
                       review_needed=CASE WHEN ?4 THEN 1 ELSE review_needed END,
                       review_needed_reason=CASE WHEN ?4 THEN ?5 ELSE review_needed_reason END WHERE id=?1",
                    params![id, last_run, utc_now(), flag, format!("新增 {} 个相同内容的传播位置，需重新复核", fresh.len())],
                )
                .map_err(storage)?;
                if flag {
                    report.review_flagged += 1;
                }
                id
            }
            None => {
                let first = rows.iter().find(|row| row.disposition == "risk_candidate").unwrap_or(&rows[0]);
                let basis = if lead {
                    "single_risk_lead"
                } else if key.starts_with("text:") {
                    "identical_normalized_full_text"
                } else {
                    "identical_full_response_body"
                };
                tx.execute(
                    "INSERT INTO exposure_incidents(project_id,schema_version,stable_key,title,info_category,status,revision,basis,policy_version,first_run_id,last_run_id,created_at,updated_at)
                     VALUES(?1,?2,?3,?4,?5,'candidate',1,?6,?7,?8,?9,?10,?10)",
                    params![
                        project_id,
                        INCIDENT_SCHEMA_VERSION,
                        stable_key,
                        first.title,
                        first.info_category,
                        basis,
                        INCIDENT_POLICY_VERSION,
                        first_run,
                        last_run,
                        utc_now()
                    ],
                )
                .map_err(storage)?;
                report.created += 1;
                tx.last_insert_rowid()
            }
        };
        for row in &fresh {
            insert_location(tx, project_id, incident_id, row, if lead { "origin" } else { "identical_content" })?;
            auto_link(tx, project_id, incident_id, row, &context)?;
            report.attached_locations += 1;
        }
        let revision: i64 = tx
            .query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [incident_id], |r| r.get(0))
            .map_err(storage)?;
        let refs = json!({"findingIds": fresh.iter().map(|row| row.finding_id).collect::<Vec<_>>(), "contentKey": key, "runId": run_id});
        let result = json!({"incidentId": incident_id, "revision": revision});
        let operation_id = format!("auto:{}:{incident_id}:{revision}", run_id.unwrap_or(0));
        log_event(
            tx,
            project_id,
            incident_id,
            EventRecord {
                operation_id: &operation_id,
                action: if target.is_some() { "auto_attach" } else { "auto_propose" },
                actor: "system",
                reason: "相同内容或单条风险线索的自动建议，待人工复核",
                expected_revision: 0,
                from_status: "",
                to_status: "candidate",
                refs: &refs,
                result: &result,
                payload_hash: "",
            },
        )?;
    }
    if let Some(run) = run_id {
        // Decided incidents whose locations changed content/rule this round, or whose lead was
        // excluded since, ask for a review again. The old verdict stays.
        // FIX2：先从本轮的线索出发（按轮次/线索索引）找到受影响的事件，再按主键逐个标记；
        // 不再扫全项目的事件、位置和版本。条件与原来逐条相同。
        let candidates: Vec<i64> = {
            let mut statement = tx
                .prepare(
                    "SELECT DISTINCT l.incident_id FROM (
                       SELECT v.finding_id AS finding_id FROM exposure_finding_versions v WHERE v.run_id=?2 AND v.revision>1
                       UNION
                       SELECT fo.finding_id FROM exposure_finding_observations fo CROSS JOIN exposure_finding_dispositions d ON d.finding_id=fo.finding_id
                       WHERE fo.run_id=?2 AND d.disposition LIKE 'excluded_%'
                     ) touched CROSS JOIN exposure_incident_locations l ON l.project_id=?1 AND l.finding_id=touched.finding_id AND l.status='active'
                     ORDER BY l.incident_id",
                )
                .map_err(storage)?;
            let rows = statement
                .query_map(params![project_id, run], |r| r.get::<_, i64>(0))
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            rows
        };
        let now = utc_now();
        for incident in candidates {
            let flagged = tx
                .execute(
                    "UPDATE exposure_incidents SET review_needed=1,review_needed_reason='传播位置的内容/规则变化或线索已被排除，需重新复核',revision=revision+1,updated_at=?3
                     WHERE id=?2 AND project_id=?1 AND status NOT IN ('candidate','merged') AND review_needed=0",
                    params![project_id, incident, now],
                )
                .map_err(storage)?;
            report.review_flagged += flagged as i64;
        }
    }
    // FIX2-3：归属决定晚于事件创建时，单位关联在下一次建议时补上，不再成立的及时失效。
    refresh_ownership_links(tx, project_id, &context, run_id.is_none())?;
    transaction.commit()?;
    Ok(report)
}

// ───────────────────────── 单位关联随归属投影刷新（FIX2-3） ─────────────────────────

fn binding_kind_of(context: &BindingContext) -> &'static str {
    if context.group_ledger {
        "group_ledger"
    } else if context.unit_entity_id.is_some() {
        "unit"
    } else {
        "none"
    }
}

/// 事件活跃位置上的（事件, 资产, 观察）。按事件、位置顺序，同一事件同一资产只取第一条。
fn ownership_link_targets(tx: &Connection, sql: &str, values: &[i64]) -> Result<Vec<(i64, i64, i64)>, String> {
    let mut statement = tx.prepare(sql).map_err(storage)?;
    let rows = statement
        .query_map(params_from_iter(values.iter()), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    let mut seen: BTreeSet<(i64, i64)> = BTreeSet::new();
    Ok(rows.into_iter().filter(|(incident, asset, _)| seen.insert((*incident, *asset))).collect())
}

const TARGETS_ALL: &str = "SELECT l.incident_id, o.asset_id, l.observation_id FROM exposure_incident_locations l
     CROSS JOIN exposure_incidents i ON i.id=l.incident_id AND i.status<>'merged'
     CROSS JOIN exposure_observations o ON o.id=l.observation_id AND o.asset_id IS NOT NULL
     WHERE l.project_id=?1 AND l.status='active' ORDER BY l.incident_id, l.id";
const TARGETS_OF_ASSET: &str = "SELECT l.incident_id, o.asset_id, l.observation_id FROM exposure_findings f
     CROSS JOIN exposure_incident_locations l ON l.project_id=?1 AND l.finding_id=f.id AND l.status='active'
     CROSS JOIN exposure_incidents i ON i.id=l.incident_id AND i.status<>'merged'
     CROSS JOIN exposure_observations o ON o.id=l.observation_id AND o.asset_id=?2
     WHERE f.project_id=?1 AND f.asset_id=?2 ORDER BY l.incident_id, l.id";
const TARGETS_OF_INCIDENT: &str = "SELECT l.incident_id, o.asset_id, l.observation_id FROM exposure_incident_locations l
     CROSS JOIN exposure_incidents i ON i.id=l.incident_id AND i.status<>'merged'
     CROSS JOIN exposure_observations o ON o.id=l.observation_id AND o.asset_id IS NOT NULL
     WHERE l.incident_id=?2 AND l.project_id=?1 AND l.status='active' ORDER BY l.incident_id, l.id";

/// 不是集团台账项目时，归属投影产生的关联一律失效（`incident` 为 Some 时只处理这一个事件）。
fn retire_ownership_links(tx: &Connection, project_id: i64, incident: Option<i64>) -> Result<(), String> {
    match incident {
        Some(id) => tx.execute(
            "UPDATE exposure_incident_entities SET active=0 WHERE incident_id=?1 AND +project_id=?2 AND origin='ownership_projection' AND active=1",
            params![id, project_id],
        ),
        None => tx.execute(
            "UPDATE exposure_incident_entities SET active=0 WHERE project_id=?1 AND origin='ownership_projection' AND active=1",
            [project_id],
        ),
    }
    .map_err(storage)?;
    Ok(())
}

/// 增量：只看水位之后的归属事件和组决定明细涉及的资产；没有水位、目录/绑定版本或项目类型变了，
/// 或 `full` 为真时，用整项目投影（批量）对齐全部活跃位置。进行中/受阻的组操作不推进明细水位，
/// 它完成后这些资产会再被检查一次。
fn refresh_ownership_links(tx: &Connection, project_id: i64, context: &BindingContext, full: bool) -> Result<(), String> {
    // FIX3-P2-2：资产进回收站/从回收站恢复只改 project_assets，不写 asset_events，增量看不到。
    // 把回收站集合的摘要并入水位：集合一变就按整项目投影对齐一次（回收站操作是低频人工动作）。
    let bin: (i64, i64, i64) = tx
        .query_row(
            "SELECT COUNT(*),COALESCE(SUM(asset_id),0),COALESCE(SUM((asset_id*2654435761) % 1000000007),0) FROM project_assets WHERE project_id=?1 AND is_deleted=1",
            [project_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(storage)?;
    let kind = format!("{}|bin:{}:{}:{}", binding_kind_of(context), bin.0, bin.1, bin.2);
    let mark: Option<(i64, i64, i64, i64, String)> = tx
        .query_row(
            "SELECT asset_event_id,item_rowid,directory_version,binding_version,binding_kind FROM exposure_unit_link_watermarks WHERE project_id=?1",
            [project_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(storage)?;
    let events_top: i64 = tx.query_row("SELECT COALESCE(MAX(id),0) FROM asset_events", [], |r| r.get(0)).map_err(storage)?;
    let items_top: i64 = tx.query_row("SELECT COALESCE(MAX(rowid),0) FROM ownership_operation_items", [], |r| r.get(0)).map_err(storage)?;
    let unfinished: Option<i64> = tx
        .query_row(
            "SELECT MIN(i.rowid) FROM ownership_operations op CROSS JOIN ownership_operation_items i ON i.operation_id=op.operation_id
             WHERE op.status IN ('applying','blocked') AND op.project_id=?1",
            [project_id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    let rebuild = full
        || match &mark {
            None => true,
            Some((_, _, directory, binding, previous)) => *directory != context.directory_version || *binding != context.binding_version || *previous != kind,
        };
    if !context.group_ledger {
        if rebuild {
            retire_ownership_links(tx, project_id, None)?;
        }
    } else if rebuild {
        let projection = crate::commands::ownership::ownership_asset_unit_projection(tx, project_id).map_err(storage)?;
        let edges: HashMap<i64, &crate::commands::ownership::OwnershipAssetUnitEdge> = projection.edges.iter().map(|edge| (edge.asset_id, edge)).collect();
        for (incident, asset, observation) in ownership_link_targets(tx, TARGETS_ALL, &[project_id])? {
            reconcile_ownership_link(tx, project_id, incident, asset, observation, edges.get(&asset).copied(), context)?;
        }
    } else if let Some((event_mark, item_mark, _, _, _)) = &mark {
        let mut assets: BTreeSet<i64> = BTreeSet::new();
        {
            let mut statement = tx
                // `+project_id`：只按主键区间读水位之后的新事件，不让规划器改走 (project_id, asset_id) 索引扫全项目。
                .prepare("SELECT asset_id FROM asset_events WHERE id>?1 AND +project_id=?2 AND event_type='ownership' AND asset_id IS NOT NULL")
                .map_err(storage)?;
            let rows = statement.query_map(params![event_mark, project_id], |r| r.get::<_, i64>(0)).map_err(storage)?;
            for row in rows {
                assets.insert(row.map_err(storage)?);
            }
            let mut statement = tx
                .prepare(
                    "SELECT i.asset_id FROM ownership_operation_items i CROSS JOIN ownership_operations op ON op.operation_id=i.operation_id
                     WHERE i.rowid>?1 AND op.project_id=?2",
                )
                .map_err(storage)?;
            let rows = statement.query_map(params![item_mark, project_id], |r| r.get::<_, i64>(0)).map_err(storage)?;
            for row in rows {
                assets.insert(row.map_err(storage)?);
            }
        }
        // FIX3-P2-1：这批资产的边一次取齐（目录只读一次）。
        let wanted: Vec<i64> = assets.iter().copied().collect();
        let edges = crate::commands::ownership::ownership_asset_unit_edges(tx, project_id, &wanted).map_err(storage)?;
        for asset in assets {
            for (incident, asset, observation) in ownership_link_targets(tx, TARGETS_OF_ASSET, &[project_id, asset])? {
                reconcile_ownership_link(tx, project_id, incident, asset, observation, edges.get(&asset), context)?;
            }
        }
    }
    let item_mark = match unfinished {
        Some(first) => items_top.min(first - 1),
        None => items_top,
    };
    tx.execute(
        "INSERT INTO exposure_unit_link_watermarks(project_id,asset_event_id,item_rowid,directory_version,binding_version,binding_kind,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(project_id) DO UPDATE SET asset_event_id=excluded.asset_event_id,item_rowid=excluded.item_rowid,
           directory_version=excluded.directory_version,binding_version=excluded.binding_version,binding_kind=excluded.binding_kind,updated_at=excluded.updated_at",
        params![project_id, events_top, item_mark, context.directory_version, context.binding_version, kind, utc_now()],
    )
    .map_err(storage)?;
    Ok(())
}

/// 读一个事件时，它的单位关联先按当前归属投影对齐（补上晚到的、失效不再成立的），再返回。
fn refresh_incident_ownership_links(connection: &Connection, project_id: i64, incident_id: i64) -> Result<(), String> {
    let context = binding_context(connection, project_id)?;
    let transaction = ImmediateTx::begin(connection)?;
    let tx = transaction.connection();
    if context.group_ledger {
        // FIX3-P2-1：大事件按位置逐个取边会重复加载整份目录；先收齐资产，一次批量取边。
        let targets = ownership_link_targets(tx, TARGETS_OF_INCIDENT, &[project_id, incident_id])?;
        let wanted: Vec<i64> = targets.iter().map(|(_, asset, _)| *asset).collect();
        let edges = crate::commands::ownership::ownership_asset_unit_edges(tx, project_id, &wanted).map_err(storage)?;
        for (incident, asset, observation) in targets {
            reconcile_ownership_link(tx, project_id, incident, asset, observation, edges.get(&asset), &context)?;
        }
    } else {
        retire_ownership_links(tx, project_id, Some(incident_id))?;
    }
    transaction.commit()
}

// ───────────────────────────── human actions ─────────────────────────────

#[derive(Deserialize, Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IncidentAction {
    pub operation_id: String,
    pub action: String,
    pub incident_id: i64,
    pub expected_revision: i64,
    pub actor: String,
    pub reason: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub target_incident_id: Option<i64>,
    #[serde(default)]
    pub target_expected_revision: Option<i64>,
    #[serde(default)]
    pub location_ids: Option<Vec<i64>>,
    #[serde(default)]
    pub entity_id: Option<i64>,
    #[serde(default)]
    pub link_level: Option<String>,
    #[serde(default)]
    pub evidence_observation_ids: Option<Vec<i64>>,
    #[serde(default)]
    pub handling: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub external_reference: Option<String>,
    #[serde(default)]
    pub retest_run_id: Option<i64>,
}

struct IncidentRow {
    status: String,
    revision: i64,
    handling_state: String,
    info_category: String,
    title: String,
}

fn load_incident(tx: &Connection, project_id: i64, incident_id: i64) -> Result<IncidentRow, String> {
    tx.query_row(
        "SELECT status,revision,handling_state,info_category,title FROM exposure_incidents WHERE id=?1 AND project_id=?2",
        params![incident_id, project_id],
        |r| Ok(IncidentRow { status: r.get(0)?, revision: r.get(1)?, handling_state: r.get(2)?, info_category: r.get(3)?, title: r.get(4)? }),
    )
    .optional()
    .map_err(storage)?
    .ok_or_else(|| format!("[exposure_incident_not_found] 项目 {project_id} 没有事件 {incident_id}"))
}

fn cas(row: &IncidentRow, expected: i64, incident_id: i64) -> Result<(), String> {
    if row.status == "merged" {
        return Err(format!("[exposure_incident_state] 事件 {incident_id} 已并入其它事件，不能再操作"));
    }
    if row.revision != expected {
        return Err(format!(
            "[exposure_incident_conflict] 事件 {incident_id} 当前版本是 {}，不是 {expected}；请刷新后再操作",
            row.revision
        ));
    }
    Ok(())
}

fn text_ok(value: &Option<String>, limit: usize) -> bool {
    value.as_ref().is_none_or(|text| text.chars().count() <= limit)
}

fn validate(action: &IncidentAction) -> Result<(), String> {
    if !super::imports::operation_id_valid(&action.operation_id) {
        return Err(invalid("operationId 必须是 8~80 位字母、数字、- 或 _"));
    }
    if !(1..=80).contains(&action.actor.chars().count()) {
        return Err(invalid("actor 必填（≤80 字）"));
    }
    if !(4..=500).contains(&action.reason.chars().count()) {
        return Err(invalid("每个动作都必须写明理由（4~500 字）"));
    }
    if !text_ok(&action.channel, 120) || !text_ok(&action.external_reference, 200) {
        return Err(invalid("channel ≤120 字，externalReference ≤200 字"));
    }
    match action.action.as_str() {
        "review" => {
            let status = action.status.as_deref().unwrap_or_default();
            if status == "merged" || !STATUSES.contains(&status) {
                return Err(invalid("review 的 status 只能是 candidate/reviewing/verified/dismissed/excluded_public"));
            }
        }
        "merge" => {
            if action.target_incident_id.is_none() || action.target_expected_revision.is_none() {
                return Err(invalid("merge 需要 targetIncidentId 与 targetExpectedRevision"));
            }
            if action.target_incident_id == Some(action.incident_id) {
                return Err(invalid("不能把事件并入自己"));
            }
        }
        "split" => {
            if action.location_ids.as_ref().is_none_or(|ids| ids.is_empty() || ids.len() > 500) {
                return Err(invalid("split 需要 1~500 个 locationIds"));
            }
        }
        "link_entity" => {
            if action.entity_id.is_none() || !LINK_LEVELS.contains(&action.link_level.as_deref().unwrap_or_default()) {
                return Err(invalid("link_entity 需要 entityId 与 linkLevel（confirmed_link/candidate_link/unknown/conflict）"));
            }
        }
        "record_action" => {
            let handling = action.handling.as_deref().unwrap_or_default();
            if !HANDLING_ACTIONS.contains(&handling) {
                return Err(invalid(format!("handling 只能是 {}", HANDLING_ACTIONS.join("/"))));
            }
            if handling == "notification_recorded" && action.channel.as_deref().unwrap_or_default().trim().is_empty() {
                return Err(invalid("登记通知必须写明已通过什么渠道通知了谁（应用不代为联系）"));
            }
            if handling == "retest_linked" && action.retest_run_id.is_none() {
                return Err(invalid("retest_linked 需要 retestRunId"));
            }
        }
        other => return Err(invalid(format!("未知动作 {other}"))),
    }
    Ok(())
}

fn active_location_ids(tx: &Connection, incident_id: i64) -> Result<Vec<i64>, String> {
    let mut statement = tx
        .prepare("SELECT id FROM exposure_incident_locations WHERE incident_id=?1 AND status='active' ORDER BY id")
        .map_err(storage)?;
    let rows = statement
        .query_map([incident_id], |r| r.get::<_, i64>(0))
        .map_err(storage)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage)?;
    Ok(rows)
}

/// Copies a location row onto another incident (merge/split keep the original row, marked).
fn move_location(tx: &Connection, location_id: i64, to_incident: i64, relation: &str, old_status: &str) -> Result<(), String> {
    tx.execute(
        "INSERT INTO exposure_incident_locations(incident_id,project_id,finding_id,observation_id,run_id,location_kind,canonical_url,source_key,collection_mode,relation,content_key,status,created_at)
         SELECT ?2,project_id,finding_id,observation_id,run_id,location_kind,canonical_url,source_key,collection_mode,?3,content_key,'active',?4
         FROM exposure_incident_locations WHERE id=?1
         ON CONFLICT(incident_id,finding_id) DO UPDATE SET status='active',relation=excluded.relation",
        params![location_id, to_incident, relation, utc_now()],
    )
    .map_err(storage)?;
    tx.execute("UPDATE exposure_incident_locations SET status=?2 WHERE id=?1", params![location_id, old_status])
        .map_err(storage)?;
    Ok(())
}

fn evidence_observation(item: &JsonValue) -> Option<i64> {
    item.as_i64().or_else(|| item.get("observationId").and_then(JsonValue::as_i64))
}

/// P1-7 拆分时单位关联跟着证据走：证据里有被拆走位置的观察，就把这部分证据带到新事件；
/// 原事件只保留剩余证据，一条不剩就失效。没有证据的关联（候选）留在原事件。
fn split_unit_links(tx: &Connection, from: i64, to: i64, moved: &BTreeSet<i64>, now: &str) -> Result<i64, String> {
    let mut statement = tx
        .prepare(
            "SELECT id,project_id,entity_id,link_level,origin,basis,evidence_json,directory_version,binding_version
             FROM exposure_incident_entities WHERE incident_id=?1 AND active=1 ORDER BY id",
        )
        .map_err(storage)?;
    type LinkRow = (i64, i64, i64, String, String, String, String, i64, i64);
    let rows: Vec<LinkRow> = statement
        .query_map([from], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)))
        .map_err(storage)?
        .collect::<Result<_, _>>()
        .map_err(storage)?;
    let mut carried = 0;
    for (id, project_id, entity_id, level, origin, basis, evidence, directory_version, binding_version) in rows {
        let items: Vec<JsonValue> = serde_json::from_str(&evidence).unwrap_or_default();
        let (taken, kept): (Vec<JsonValue>, Vec<JsonValue>) =
            items.into_iter().partition(|item| evidence_observation(item).is_some_and(|observation| moved.contains(&observation)));
        if taken.is_empty() {
            continue;
        }
        let insert = |incident: i64, basis: &str, evidence: &[JsonValue]| {
            tx.execute(
                "INSERT INTO exposure_incident_entities(incident_id,project_id,entity_id,link_level,origin,basis,evidence_json,directory_version,binding_version,active,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10)",
                params![incident, project_id, entity_id, level, origin, basis, json!(evidence).to_string(), directory_version, binding_version, now],
            )
            .map_err(storage)
        };
        insert(to, &format!("{basis}（随拆分带入）"), &taken)?;
        tx.execute("UPDATE exposure_incident_entities SET active=0 WHERE id=?1", [id]).map_err(storage)?;
        if !kept.is_empty() {
            insert(from, &basis, &kept)?;
        }
        carried += 1;
    }
    Ok(carried)
}

/// P1-4 登记关闭前看最近一次关联的复测：轮次必须已完成，每个位置都必须是内容已变化，
/// 或明确 404/410。失败、未观察到、同内容、只取到部分内容都不能作为关闭依据。
fn closable_after_retest(tx: &Connection, incident_id: i64) -> Result<(), String> {
    let latest: Option<Option<String>> = tx
        .query_row(
            "SELECT json_extract(result_json,'$.retest') FROM exposure_incident_events
             WHERE incident_id=?1 AND action='record_action' AND json_extract(refs_json,'$.handling')='retest_linked'
             ORDER BY id DESC LIMIT 1",
            [incident_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(storage)?;
    let retest: JsonValue = latest.flatten().and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(JsonValue::Null);
    let run_id = retest["runId"].as_i64().unwrap_or_default();
    let run_status = retest["runStatus"].as_str().unwrap_or("unknown");
    if !matches!(run_status, "completed" | "partial") {
        return Err(format!("[exposure_incident_retest_failed] 最近一次复测（第 {run_id} 轮）状态是 {run_status}，不能作为关闭依据"));
    }
    let locations = retest["locations"].as_array().cloned().unwrap_or_default();
    if locations.is_empty() {
        return Err(format!("[exposure_incident_retest_failed] 最近一次复测（第 {run_id} 轮）没有位置结果，不能作为关闭依据"));
    }
    for location in &locations {
        let outcome = location["outcome"].as_str().unwrap_or("");
        let code = location["statusCode"].as_i64().unwrap_or_default();
        let removed = outcome == "unreachable_or_failed" && matches!(code, 404 | 410);
        if outcome != "changed_content" && !removed {
            return Err(format!(
                "[exposure_incident_retest_failed] 第 {run_id} 轮里位置 {} 的复测结果是 {outcome}（HTTP {code}），不能作为关闭依据",
                location["url"].as_str().unwrap_or("")
            ));
        }
    }
    // FIX3-R5：复测之后新增的位置（或复测轮之后又被看到的位置）没有被这次复测覆盖，旧复测不能关闭。
    let covered: std::collections::HashSet<i64> = locations.iter().filter_map(|l| l["locationId"].as_i64()).collect();
    let mut current = tx
        .prepare("SELECT id,canonical_url,run_id FROM exposure_incident_locations WHERE incident_id=?1 AND status='active' ORDER BY id")
        .map_err(storage)?;
    let rows: Vec<(i64, String, i64)> = current
        .query_map([incident_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(storage)?
        .collect::<Result<_, _>>()
        .map_err(storage)?;
    if let Some((_, url, _)) = rows.iter().find(|(id, _, seen)| !covered.contains(id) || *seen > run_id) {
        return Err(format!("[exposure_incident_retest_stale] 第 {run_id} 轮复测之后出现了新位置 {url}，需要重新复测后才能关闭"));
    }
    Ok(())
}

fn bump(tx: &Connection, incident_id: i64) -> Result<(), String> {
    tx.execute("UPDATE exposure_incidents SET revision=revision+1,updated_at=?2 WHERE id=?1", params![incident_id, utc_now()])
        .map_err(storage)?;
    Ok(())
}

/// Applies one human action. Replays by operationId. CAS on expectedRevision runs in the same
/// IMMEDIATE transaction as the write. Nothing here sends anything anywhere.
pub fn act(connection: &Connection, project_id: i64, raw: &JsonValue) -> Result<JsonValue, String> {
    ensure_exposure_schema(connection)?;
    check_schema(connection)?;
    let mut action: IncidentAction = serde_json::from_value(raw.clone()).map_err(invalid)?;
    action.actor = action.actor.trim().to_string();
    let raw_reason = action.reason.trim().to_string();
    let raw_channel = action.channel.clone().unwrap_or_default();
    let raw_reference = action.external_reference.clone().unwrap_or_default();
    // P1-5 人工输入的理由/渠道/外部编号可能粘进口令或令牌：先脱敏，再算幂等哈希和落库。
    action.reason = redact_text(action.reason.trim());
    action.channel = action.channel.take().map(|value| redact_text(&value));
    action.external_reference = action.external_reference.take().map(|value| redact_text(&value));
    validate(&action)?;
    let payload_hash = super::redaction::idempotency_hash(
        &serde_json::to_string(&action).map_err(invalid)?,
        &[&raw_reason, &raw_channel, &raw_reference],
    );
    let transaction = ImmediateTx::begin(connection)?;
    let tx = transaction.connection();
    let replay: Option<(String, String)> = tx
        .query_row(
            "SELECT payload_hash,result_json FROM exposure_incident_events WHERE project_id=?1 AND operation_id=?2",
            params![project_id, action.operation_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(storage)?;
    if let Some((hash, result)) = replay {
        if hash != payload_hash {
            return Err(format!("[exposure_incident_operation_conflict] operationId {} 已用于另一份载荷", action.operation_id));
        }
        let mut value: JsonValue = serde_json::from_str(&result).map_err(storage)?;
        value["replayed"] = json!(true);
        return Ok(value);
    }
    let row = load_incident(tx, project_id, action.incident_id)?;
    cas(&row, action.expected_revision, action.incident_id)?;
    let now = utc_now();
    let mut refs = json!({});
    let (to_status, result) = match action.action.as_str() {
        "review" => {
            let status = action.status.clone().unwrap_or_default();
            if status == row.status {
                return Err(format!("[exposure_incident_state] 事件已是 {status}"));
            }
            if status == "verified" {
                let evidence: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM exposure_incident_locations WHERE incident_id=?1 AND status='active' AND observation_id IS NOT NULL",
                        [action.incident_id],
                        |r| r.get(0),
                    )
                    .map_err(storage)?;
                if evidence == 0 {
                    return Err("[exposure_incident_evidence_required] 没有可追溯观察的事件不能核实".to_string());
                }
            }
            tx.execute(
                "UPDATE exposure_incidents SET status=?2,revision=revision+1,review_needed=0,review_needed_reason='',updated_at=?3 WHERE id=?1",
                params![action.incident_id, status, now],
            )
            .map_err(storage)?;
            (status.clone(), json!({"legacyFindingStatus": legacy_finding_status(&status)}))
        }
        "merge" => {
            let target_id = action.target_incident_id.unwrap_or_default();
            let target = load_incident(tx, project_id, target_id)?;
            cas(&target, action.target_expected_revision.unwrap_or_default(), target_id)?;
            let moved = active_location_ids(tx, action.incident_id)?;
            for location_id in &moved {
                move_location(tx, *location_id, target_id, &format!("merged_from:{}", action.incident_id), "merged")?;
            }
            tx.execute(
                "INSERT INTO exposure_incident_entities(incident_id,project_id,entity_id,link_level,origin,basis,evidence_json,directory_version,binding_version,active,created_at)
                 SELECT ?2,project_id,entity_id,link_level,origin,basis||'（随合并带入）',evidence_json,directory_version,binding_version,1,?3
                 FROM exposure_incident_entities WHERE incident_id=?1 AND active=1",
                params![action.incident_id, target_id, now],
            )
            .map_err(storage)?;
            tx.execute(
                "UPDATE exposure_incidents SET status='merged',merged_into=?2,revision=revision+1,updated_at=?3 WHERE id=?1",
                params![action.incident_id, target_id, now],
            )
            .map_err(storage)?;
            bump(tx, target_id)?;
            refs = json!({"targetIncidentId": target_id, "targetExpectedRevision": target.revision, "movedLocations": moved.len()});
            ("merged".to_string(), json!({"mergedInto": target_id, "movedLocations": moved.len()}))
        }
        "split" => {
            let unique: BTreeSet<i64> = action.location_ids.clone().unwrap_or_default().into_iter().collect();
            let active = active_location_ids(tx, action.incident_id)?;
            if unique.iter().any(|id| !active.contains(id)) {
                return Err(invalid("locationIds 必须都是本事件的有效位置"));
            }
            if unique.len() >= active.len() {
                return Err(invalid("拆分后原事件至少要保留一个位置"));
            }
            tx.execute(
                "INSERT INTO exposure_incidents(project_id,schema_version,stable_key,title,info_category,status,revision,basis,policy_version,created_at,updated_at)
                 VALUES(?1,?2,?3,?4,?5,'candidate',1,'manual_split',?6,?7,?7)",
                params![project_id, INCIDENT_SCHEMA_VERSION, format!("split:{}", action.operation_id), row.title, row.info_category, INCIDENT_POLICY_VERSION, now],
            )
            .map_err(storage)?;
            let new_id = tx.last_insert_rowid();
            let mut moved_observations = BTreeSet::new();
            for location_id in &unique {
                let observation: Option<i64> = tx
                    .query_row("SELECT observation_id FROM exposure_incident_locations WHERE id=?1", [location_id], |r| r.get(0))
                    .map_err(storage)?;
                moved_observations.extend(observation);
                move_location(tx, *location_id, new_id, &format!("split_from:{}", action.incident_id), "split_out")?;
            }
            let moved_links = split_unit_links(tx, action.incident_id, new_id, &moved_observations, &now)?;
            tx.execute(
                "UPDATE exposure_incidents SET first_run_id=(SELECT COALESCE(MIN(run_id),0) FROM exposure_incident_locations WHERE incident_id=?1),
                   last_run_id=(SELECT COALESCE(MAX(run_id),0) FROM exposure_incident_locations WHERE incident_id=?1) WHERE id=?1",
                [new_id],
            )
            .map_err(storage)?;
            bump(tx, action.incident_id)?;
            let split_refs = json!({"fromIncidentId": action.incident_id, "locationIds": unique});
            let split_result = json!({"incidentId": new_id});
            let split_operation = format!("{}#split", action.operation_id);
            log_event(
                tx,
                project_id,
                new_id,
                EventRecord {
                    operation_id: &split_operation,
                    action: "split_created",
                    actor: &action.actor,
                    reason: &action.reason,
                    expected_revision: 0,
                    from_status: "",
                    to_status: "candidate",
                    refs: &split_refs,
                    result: &split_result,
                    payload_hash: &payload_hash,
                },
            )?;
            refs = json!({"newIncidentId": new_id, "locationIds": unique, "movedUnitLinks": moved_links});
            (row.status.clone(), json!({"newIncidentId": new_id, "movedLocations": unique.len(), "movedUnitLinks": moved_links}))
        }
        "link_entity" => {
            let entity_id = action.entity_id.unwrap_or_default();
            let level = action.link_level.clone().unwrap_or_default();
            let entities = crate::commands::ownership::load_org_entities(tx).map_err(storage)?;
            if !entities.iter().any(|entity| entity.id == entity_id) {
                return Err(format!("[exposure_incident_entity_invalid] 组织目录里没有实体 {entity_id}，不会新建"));
            }
            let evidence = action.evidence_observation_ids.clone().unwrap_or_default();
            for observation_id in &evidence {
                let ok: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM exposure_incident_locations WHERE incident_id=?1 AND status='active' AND observation_id=?2",
                        params![action.incident_id, observation_id],
                        |r| r.get(0),
                    )
                    .map_err(storage)?;
                if ok == 0 {
                    return Err(format!("[exposure_incident_evidence_invalid] 观察 {observation_id} 不是本事件有效位置的证据"));
                }
            }
            if level == "confirmed_link" && evidence.is_empty() {
                return Err("[exposure_incident_evidence_required] confirmed_link 需要本事件里的观察作为证据".to_string());
            }
            let directory_version = crate::commands::ownership::org_directory_version(tx).map_err(storage)?;
            tx.execute(
                "UPDATE exposure_incident_entities SET active=0 WHERE incident_id=?1 AND entity_id=?2 AND origin='manual' AND active=1",
                params![action.incident_id, entity_id],
            )
            .map_err(storage)?;
            tx.execute(
                "INSERT INTO exposure_incident_entities(incident_id,project_id,entity_id,link_level,origin,basis,evidence_json,directory_version,binding_version,active,created_at)
                 VALUES(?1,?2,?3,?4,'manual',?5,?6,?7,0,1,?8)",
                params![action.incident_id, project_id, entity_id, level, format!("{}：{}", action.actor, action.reason), json!(evidence).to_string(), directory_version, now],
            )
            .map_err(storage)?;
            bump(tx, action.incident_id)?;
            refs = json!({"entityId": entity_id, "linkLevel": level, "evidenceObservationIds": evidence});
            (row.status.clone(), json!({"entityId": entity_id, "linkLevel": level}))
        }
        "record_action" => {
            let handling = action.handling.clone().unwrap_or_default();
            let mut outcome = json!({"handling": handling, "contactedByApplication": false});
            let next_state = match handling.as_str() {
                "notification_recorded" => "notified",
                "takedown_requested_recorded" => "takedown_requested",
                "takedown_confirmed_recorded" => "takedown_reported",
                "remediation_recorded" => "remediating",
                "retest_linked" => "retest_recorded",
                "closed_recorded" => "closed",
                _ => "none",
            };
            if handling == "closed_recorded" && row.status == "candidate" {
                return Err("[exposure_incident_state] 候选事件未经复核，不能登记关闭".to_string());
            }
            if handling == "reopened" && row.handling_state != "closed" {
                return Err("[exposure_incident_state] 只有已登记关闭的事件才能重新打开".to_string());
            }
            if handling != "reopened" && row.handling_state == "closed" {
                return Err("[exposure_incident_state] 事件已登记关闭，先重新打开".to_string());
            }
            if handling == "retest_linked" {
                outcome["retest"] = retest_outcome(tx, project_id, action.incident_id, action.retest_run_id.unwrap_or_default())?;
            }
            if handling == "closed_recorded" {
                let retests: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM exposure_incident_events WHERE incident_id=?1 AND action='record_action' AND json_extract(refs_json,'$.handling')='retest_linked'",
                        [action.incident_id],
                        |r| r.get(0),
                    )
                    .map_err(storage)?;
                if retests > 0 {
                    closable_after_retest(tx, action.incident_id)?;
                }
                outcome["closureBasis"] = json!(if retests > 0 { "manual_with_retest" } else { "manual_without_retest" });
            }
            tx.execute(
                "UPDATE exposure_incidents SET handling_state=?2,revision=revision+1,updated_at=?3 WHERE id=?1",
                params![action.incident_id, next_state, now],
            )
            .map_err(storage)?;
            refs = json!({"handling": handling, "channel": action.channel, "externalReference": action.external_reference, "retestRunId": action.retest_run_id});
            outcome["handlingState"] = json!(next_state);
            (row.status.clone(), outcome)
        }
        _ => return Err(invalid("未知动作")),
    };
    let revision: i64 = tx
        .query_row("SELECT revision FROM exposure_incidents WHERE id=?1", [action.incident_id], |r| r.get(0))
        .map_err(storage)?;
    let mut value = json!({
        "incidentId": action.incident_id,
        "action": action.action,
        "revision": revision,
        "previousRevision": row.revision,
        "status": to_status,
        "replayed": false,
    });
    if let (Some(map), Some(extra)) = (value.as_object_mut(), result.as_object()) {
        for (key, item) in extra {
            map.insert(key.clone(), item.clone());
        }
    }
    let event_id = log_event(
        tx,
        project_id,
        action.incident_id,
        EventRecord {
            operation_id: &action.operation_id,
            action: &action.action,
            actor: &action.actor,
            reason: &action.reason,
            expected_revision: action.expected_revision,
            from_status: &row.status,
            to_status: &to_status,
            refs: &refs,
            result: &value,
            payload_hash: &payload_hash,
        },
    )?;
    value["eventId"] = json!(event_id);
    tx.execute("UPDATE exposure_incident_events SET result_json=?2 WHERE id=?1", params![event_id, value.to_string()])
        .map_err(storage)?;
    transaction.commit()?;
    Ok(value)
}

/// Compares a later round with the incident's locations. A location the round did not see,
/// or saw as 401/403/404/5xx/transport failure, is never reported as "gone": the outcome is
/// `not_observed` / `unreachable_or_failed`, and nothing is closed automatically.
fn retest_outcome(tx: &Connection, project_id: i64, incident_id: i64, run_id: i64) -> Result<JsonValue, String> {
    let run: Option<String> = tx
        .query_row("SELECT status FROM exposure_runs WHERE id=?1 AND project_id=?2", params![run_id, project_id], |r| r.get(0))
        .optional()
        .map_err(storage)?;
    let Some(status) = run else {
        return Err(format!("[exposure_incident_retest_invalid] 第 {run_id} 轮不属于本项目"));
    };
    if matches!(status.as_str(), "queued" | "running") {
        return Err(format!("[exposure_incident_retest_invalid] 第 {run_id} 轮还在进行"));
    }
    let mut statement = tx
        .prepare("SELECT id,canonical_url,run_id,content_key FROM exposure_incident_locations WHERE incident_id=?1 AND status='active' ORDER BY id")
        .map_err(storage)?;
    let locations: Vec<(i64, String, i64, String)> = statement
        .query_map([incident_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(storage)?
        .collect::<Result<_, _>>()
        .map_err(storage)?;
    if locations.iter().all(|(_, _, seen, _)| *seen >= run_id) {
        return Err(format!("[exposure_incident_retest_invalid] 第 {run_id} 轮不晚于事件位置的观察，不是复测"));
    }
    let mut rows = Vec::new();
    for (location_id, url, _, key) in locations {
        let seen: Option<(i64, i64, String, String, i64)> = tx
            .query_row(
                "SELECT id,status_code,content_hash,COALESCE(json_extract(evidence_json,'$.detail.normalizedTextSha256'),''),truncated
                 FROM exposure_observations WHERE run_id=?1 AND project_id=?2 AND url=?3 ORDER BY id DESC LIMIT 1",
                params![run_id, project_id, url],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(storage)?;
        let outcome = match &seen {
            None => "not_observed",
            Some((_, code, _, _, _)) if !(200..300).contains(code) => "unreachable_or_failed",
            Some((_, _, body, text, truncated)) => {
                let current = if !text.is_empty() {
                    format!("text:{text}")
                } else if *truncated == 0 && !body.is_empty() {
                    format!("body:{body}")
                } else {
                    String::new()
                };
                if current.is_empty() {
                    "partial_content"
                } else if current == key {
                    "same_content"
                } else {
                    "changed_content"
                }
            }
        };
        rows.push(json!({"locationId": location_id, "url": url, "observationId": seen.as_ref().map(|s| s.0), "statusCode": seen.as_ref().map(|s| s.1), "outcome": outcome}));
    }
    Ok(json!({"runId": run_id, "runStatus": status, "locations": rows, "autoClosed": false}))
}

// ───────────────────────────── reads ─────────────────────────────

pub fn filter_hash(parts: &[&str]) -> String {
    content_hash(parts.join("\u{1f}").as_bytes()).chars().take(12).collect()
}

/// Keyset cursor bound to its filter: a cursor from another filter is refused, never reused.
pub fn parse_cursor(cursor: &str, bound: &str) -> Result<Option<i64>, String> {
    if cursor.is_empty() {
        return Ok(None);
    }
    let parts: Vec<&str> = cursor.splitn(3, ':').collect();
    match parts.as_slice() {
        ["v1", hash, id] if *hash == bound => id.parse::<i64>().map(Some).map_err(|_| "[exposure_cursor_invalid] 游标损坏".to_string()),
        ["v1", _, _] => Err("[exposure_cursor_mismatch] 游标属于另一组筛选条件，请从第一页重新读取".to_string()),
        _ => Err("[exposure_cursor_invalid] 游标损坏".to_string()),
    }
}

pub fn page_size(value: i64) -> Result<i64, String> {
    if !(1..=PAGE_SIZE_MAX).contains(&value) {
        return Err(format!("[exposure_page_size_invalid] pageSize 只能是 1~{PAGE_SIZE_MAX}"));
    }
    Ok(value)
}

fn incident_item(connection: &Connection, row: &rusqlite::Row<'_>) -> rusqlite::Result<JsonValue> {
    let id: i64 = row.get(0)?;
    let status: String = row.get(4)?;
    let mut kinds = BTreeSet::new();
    let mut modes = BTreeSet::new();
    let mut samples = Vec::new();
    {
        let mut statement = connection.prepare_cached(
            "SELECT location_kind,collection_mode,canonical_url FROM exposure_incident_locations WHERE incident_id=?1 AND status='active' ORDER BY id LIMIT 50",
        )?;
        let mut rows = statement.query([id])?;
        while let Some(location) = rows.next()? {
            kinds.insert(location.get::<_, String>(0)?);
            modes.insert(location.get::<_, String>(1)?);
            if samples.len() < 3 {
                samples.push(location.get::<_, String>(2)?);
            }
        }
    }
    let active: i64 = connection.query_row(
        "SELECT COUNT(*) FROM exposure_incident_locations WHERE incident_id=?1 AND status='active'",
        [id],
        |r| r.get(0),
    )?;
    let (confirmed, candidate, conflict): (i64, i64, i64) = connection.query_row(
        "SELECT COUNT(DISTINCT CASE WHEN link_level='confirmed_link' THEN entity_id END),
                COUNT(DISTINCT CASE WHEN link_level='candidate_link' THEN entity_id END),
                COUNT(DISTINCT CASE WHEN link_level='conflict' THEN entity_id END)
         FROM exposure_incident_entities WHERE incident_id=?1 AND active=1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    Ok(json!({
        "id": id,
        "schemaVersion": row.get::<_, i64>(1)?,
        "stableKey": row.get::<_, String>(2)?,
        "title": row.get::<_, String>(3)?,
        "status": status,
        "legacyFindingStatus": legacy_finding_status(&status),
        "revision": row.get::<_, i64>(5)?,
        "infoCategory": row.get::<_, String>(6)?,
        "basis": row.get::<_, String>(7)?,
        "policyVersion": row.get::<_, String>(8)?,
        "reviewNeeded": row.get::<_, i64>(9)? != 0,
        "reviewNeededReason": row.get::<_, String>(10)?,
        "mergedInto": row.get::<_, Option<i64>>(11)?,
        "handlingState": row.get::<_, String>(12)?,
        "firstRunId": row.get::<_, i64>(13)?,
        "lastRunId": row.get::<_, i64>(14)?,
        "updatedAt": row.get::<_, String>(15)?,
        "activeLocations": active,
        "locationKinds": kinds.iter().collect::<Vec<_>>(),
        "collectionModes": modes.iter().collect::<Vec<_>>(),
        "evidenceGrade": evidence_grade(&modes, &kinds),
        "sampleUrls": samples,
        "units": {"confirmed": confirmed, "candidate": candidate, "conflict": conflict, "unknown": confirmed + candidate + conflict == 0},
    }))
}

const INCIDENT_COLUMNS: &str = "id,schema_version,stable_key,title,status,revision,info_category,basis,policy_version,review_needed,review_needed_reason,merged_into,handling_state,first_run_id,last_run_id,updated_at";

fn next_cursor(items: &[JsonValue], has_more: bool, bound: &str) -> String {
    if !has_more {
        return String::new();
    }
    items.last().and_then(|row| row["id"].as_i64()).map(|id| format!("v1:{bound}:{id}")).unwrap_or_default()
}

/// G4-3 one page of the deduplicated incident queue, newest first, keyset on id.
/// `status` is "" (all but merged), "open" (candidate/reviewing/verified) or one status.
pub fn page_incidents(connection: &Connection, project_id: i64, status: &str, review_needed_only: bool, size: i64, cursor: &str) -> Result<JsonValue, String> {
    check_schema(connection)?;
    let size = page_size(size)?;
    if !status.is_empty() && status != "open" && !STATUSES.contains(&status) {
        return Err(invalid(format!("未知状态筛选 {status}")));
    }
    let bound = filter_hash(&[&project_id.to_string(), status, if review_needed_only { "1" } else { "0" }]);
    let after = parse_cursor(cursor, &bound)?;
    let status_sql = match status {
        "" => "(?2='' AND status<>'merged')",
        "open" => "(?2='open' AND status IN ('candidate','reviewing','verified'))",
        _ => "status=?2",
    };
    let review_sql = if review_needed_only { " AND review_needed=1" } else { "" };
    let total: i64 = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM exposure_incidents WHERE project_id=?1 AND {status_sql}{review_sql}"),
            params![project_id, status],
            |r| r.get(0),
        )
        .map_err(storage)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT {INCIDENT_COLUMNS} FROM exposure_incidents WHERE project_id=?1 AND {status_sql}{review_sql} AND id<?3 ORDER BY id DESC LIMIT ?4"
        ))
        .map_err(storage)?;
    let items: Vec<JsonValue> = statement
        .query_map(params![project_id, status, after.unwrap_or(i64::MAX), size + 1], |row| incident_item(connection, row))
        .map_err(storage)?
        .collect::<Result<_, _>>()
        .map_err(storage)?;
    let has_more = items.len() as i64 > size;
    let items: Vec<JsonValue> = items.into_iter().take(size as usize).collect();
    let next = next_cursor(&items, has_more, &bound);
    Ok(json!({"items": items, "total": total, "pageSize": size, "hasMore": has_more, "nextCursor": next, "policyVersion": INCIDENT_POLICY_VERSION}))
}

/// G4-3 the locations of one incident, a page at a time (组内分页).
pub fn page_locations(connection: &Connection, project_id: i64, incident_id: i64, size: i64, cursor: &str) -> Result<JsonValue, String> {
    check_schema(connection)?;
    let size = page_size(size)?;
    load_incident(connection, project_id, incident_id)?;
    let bound = filter_hash(&[&project_id.to_string(), &incident_id.to_string()]);
    let after = parse_cursor(cursor, &bound)?;
    let total: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_incident_locations WHERE incident_id=?1 AND project_id=?2 AND status='active'",
            params![incident_id, project_id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    let mut statement = connection
        .prepare(
            "SELECT l.id,l.finding_id,l.observation_id,l.run_id,l.location_kind,l.canonical_url,l.source_key,l.collection_mode,l.relation,l.content_key,l.created_at,
                    COALESCE(o.queried_at,''),COALESCE(o.indexed_at,''),COALESCE(o.status_code,0),COALESCE(o.truncated,0),COALESCE(d.disposition,''),f.title
             FROM exposure_incident_locations l
             JOIN exposure_findings f ON f.id=l.finding_id AND f.project_id=l.project_id
             LEFT JOIN exposure_observations o ON o.id=l.observation_id AND o.project_id=l.project_id
             LEFT JOIN exposure_finding_dispositions d ON d.finding_id=l.finding_id
             WHERE l.incident_id=?1 AND l.project_id=?2 AND l.status='active' AND l.id>?3 ORDER BY l.id LIMIT ?4",
        )
        .map_err(storage)?;
    let items: Vec<JsonValue> = statement
        .query_map(params![incident_id, project_id, after.unwrap_or(0), size + 1], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "findingId": r.get::<_, i64>(1)?,
                "observationId": r.get::<_, Option<i64>>(2)?,
                "runId": r.get::<_, i64>(3)?,
                "locationKind": r.get::<_, String>(4)?,
                "canonicalUrl": r.get::<_, String>(5)?,
                "sourceKey": r.get::<_, String>(6)?,
                "collectionMode": r.get::<_, String>(7)?,
                "relation": r.get::<_, String>(8)?,
                "contentKey": r.get::<_, String>(9)?,
                "createdAt": r.get::<_, String>(10)?,
                "observedAt": r.get::<_, String>(11)?,
                "indexedAt": r.get::<_, String>(12)?,
                "statusCode": r.get::<_, i64>(13)?,
                "truncated": r.get::<_, i64>(14)? != 0,
                "disposition": r.get::<_, String>(15)?,
                "findingTitle": r.get::<_, String>(16)?,
            }))
        })
        .map_err(storage)?
        .collect::<Result<_, _>>()
        .map_err(storage)?;
    let has_more = items.len() as i64 > size;
    let items: Vec<JsonValue> = items.into_iter().take(size as usize).collect();
    let next = next_cursor(&items, has_more, &bound);
    Ok(json!({"items": items, "total": total, "pageSize": size, "hasMore": has_more, "nextCursor": next}))
}

/// G4-0 one incident with the keys §12 asks for, its unit links and its action trail.
pub fn read_incident(connection: &Connection, project_id: i64, incident_id: i64) -> Result<JsonValue, String> {
    check_schema(connection)?;
    let mut item = connection
        .query_row(
            &format!("SELECT {INCIDENT_COLUMNS} FROM exposure_incidents WHERE id=?1 AND project_id=?2"),
            params![incident_id, project_id],
            |row| incident_item(connection, row),
        )
        .optional()
        .map_err(storage)?
        .ok_or_else(|| format!("[exposure_incident_not_found] 项目 {project_id} 没有事件 {incident_id}"))?;
    // FIX2-3：先把这个事件的单位关联与归属投影对齐，下面读到的就是当前成立的关联。
    refresh_incident_ownership_links(connection, project_id, incident_id)?;
    let refs: Vec<(i64, Option<i64>, i64)> = {
        let mut statement = connection
            .prepare("SELECT finding_id,observation_id,run_id FROM exposure_incident_locations WHERE incident_id=?1 AND status='active' ORDER BY id")
            .map_err(storage)?;
        let rows = statement
            .query_map([incident_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows
    };
    let names: HashMap<i64, (String, String)> = crate::commands::ownership::load_org_entities(connection)
        .map_err(storage)?
        .into_iter()
        .map(|entity| (entity.id, (entity.legal_name, entity.entity_kind)))
        .collect();
    let entities: Vec<JsonValue> = {
        let mut statement = connection
            .prepare("SELECT entity_id,link_level,origin,basis,evidence_json,directory_version,binding_version,created_at FROM exposure_incident_entities WHERE incident_id=?1 AND active=1 ORDER BY id")
            .map_err(storage)?;
        let rows = statement
            .query_map([incident_id], |r| {
                let entity_id: i64 = r.get(0)?;
                let (name, kind) = names.get(&entity_id).cloned().unwrap_or_else(|| ("（目录中已不存在）".to_string(), "missing".to_string()));
                Ok(json!({
                    "entityId": entity_id,
                    "legalName": name,
                    "entityKind": kind,
                    "linkLevel": r.get::<_, String>(1)?,
                    "origin": r.get::<_, String>(2)?,
                    "basis": r.get::<_, String>(3)?,
                    "evidence": serde_json::from_str::<JsonValue>(&r.get::<_, String>(4)?).unwrap_or(JsonValue::Null),
                    "directoryVersion": r.get::<_, i64>(5)?,
                    "bindingVersion": r.get::<_, i64>(6)?,
                    "createdAt": r.get::<_, String>(7)?,
                }))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows
    };
    let mut events: Vec<JsonValue> = {
        let mut statement = connection
            .prepare("SELECT id,operation_id,action,actor,reason,expected_revision,from_status,to_status,refs_json,created_at FROM exposure_incident_events WHERE incident_id=?1 ORDER BY id DESC LIMIT 51")
            .map_err(storage)?;
        let rows = statement
            .query_map([incident_id], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "operationId": r.get::<_, String>(1)?,
                    "action": r.get::<_, String>(2)?,
                    "actor": r.get::<_, String>(3)?,
                    "reason": r.get::<_, String>(4)?,
                    "expectedRevision": r.get::<_, i64>(5)?,
                    "fromStatus": r.get::<_, String>(6)?,
                    "toStatus": r.get::<_, String>(7)?,
                    "refs": serde_json::from_str::<JsonValue>(&r.get::<_, String>(8)?).unwrap_or(JsonValue::Null),
                    "createdAt": r.get::<_, String>(9)?,
                }))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows
    };
    let truncated = events.len() > 50;
    events.truncate(50);
    let binding_version = entities.iter().filter_map(|row| row["bindingVersion"].as_i64()).max().unwrap_or(0);
    let directory_version = entities.iter().filter_map(|row| row["directoryVersion"].as_i64()).max().unwrap_or(0);
    let runs: BTreeSet<i64> = refs.iter().map(|row| row.2).collect();
    item["projectId"] = json!(project_id);
    item["findingIds"] = json!(refs.iter().map(|row| row.0).collect::<Vec<_>>());
    item["observationRefs"] = json!(refs.iter().filter_map(|row| row.1).collect::<Vec<_>>());
    item["runIds"] = json!(runs);
    item["directoryVersion"] = json!(directory_version);
    item["bindingVersion"] = json!(binding_version);
    item["entities"] = json!(entities);
    item["events"] = json!(events);
    item["eventsTruncated"] = json!(truncated);
    item["locations"] = page_locations(connection, project_id, incident_id, 50, "")?;
    item["statusMapping"] = json!({"candidate": "new", "reviewing": "reviewing", "verified": "confirmed", "dismissed": "dismissed", "excluded_public": "dismissed"});
    Ok(item)
}
