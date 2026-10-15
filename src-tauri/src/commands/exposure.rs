//! Exposure collection, evidence and review, with explicit ownership read dependencies.
#![allow(dead_code)]
use crate::{
    asset_models::{
        ExportResult, ExposureFinding, ExposureFindingInput, ExposureRun, ExposureSourceResult,
        ExposureSummary,
    },
    db,
    AppState,
};
use rusqlite::{params, OptionalExtension, Row};
use serde_json::Value as JsonValue;
use std::{fs, path::PathBuf, thread};
use tauri::State;
use uuid::Uuid;

// The facade and its collection kernel have the same actual module owner.
use crate::commands::exposure as exposure;

pub mod cache;
pub mod classify;
pub mod cluster_repository;
pub mod clusters;
pub mod remediation;
pub mod adapters {
    pub mod certificate_transparency;
    pub mod documents;
    pub mod github;
    pub mod http;
    pub mod passive_web;
    pub mod web_archive;
}
pub mod contract;
pub mod coverage;
pub mod export;
pub mod fingerprint;
pub mod imports;
pub mod incidents;
pub mod live;
pub mod observations;
pub mod orchestrator;
pub mod recovery;
pub mod redaction;
pub mod repository;
pub mod rules;
pub mod scope;
pub mod settings;
pub mod state;
pub mod summary;
pub mod transaction;
pub mod units;
pub mod work_items;

#[cfg(test)]
pub mod fixture;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_e2;
#[cfg(test)]
mod tests_e3;
#[cfg(test)]
mod tests_e4;
#[cfg(test)]
mod tests_e5;
#[cfg(test)]
mod tests_q2;
#[cfg(test)]
mod tests_q3;
#[cfg(test)]
mod tests_q4;
#[cfg(test)]
mod tests_q5;
#[cfg(test)]
mod tests_q6;
#[cfg(test)]
mod tests_g0;
#[cfg(test)]
mod tests_g1;
#[cfg(test)]
mod tests_g2;
#[cfg(test)]
mod tests_g3;
#[cfg(test)]
mod tests_g4;
#[cfg(test)]
mod tests_g4_scale;

use exposure::{
    contract::{
        passive_known_limits, ExposureFindingPageInput, ExposureLimit, ExposurePage,
        ExposureQueryError,
    },
    repository::{
        load_exposure_finding, page_exposure_findings as read_finding_page,
        page_exposure_runs as read_run_page, read_exposure_source_results,
    },
    summary::{exposure_scope_summary as read_scope_summary, ExposureScopeSummary},
};

/// Errors keep their stable code in the message so the page can branch on it while the
/// existing IPC contract stays `Result<_, String>`.
#[allow(dead_code)]
fn exposure_query_error(error: ExposureQueryError) -> String {
    format!("[{}] {}", error.code, error.message)
}

/// One page of exposure findings with its real filtered total and the cursor for the page
/// after it. Read-only: it never creates a run and never opens a socket.
// 主控集成前这些命令尚未进 invoke_handler，允许暂时未被调用。
#[allow(dead_code)]
#[tauri::command]
pub fn page_exposure_findings(
    state: State<AppState>,
    input: ExposureFindingPageInput,
) -> Result<ExposureFindingPageView, String> {
    let connection = exposure_connection(&state.db_path)?;
    let query = input.normalize().map_err(exposure_query_error)?;
    finding_page_view(&connection, &query)
}

/// G1-3 the shared `ExposureFinding` row cannot grow a field (models_appsec.rs is outside the
/// exposure whitelist), so the page carries a side map: finding id → current disposition.
/// The page fields stay flat, so an older reader of the page keeps working unchanged.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureFindingPageView {
    #[serde(flatten)]
    pub page: ExposurePage<ExposureFinding>,
    pub dispositions: std::collections::BTreeMap<i64, exposure::classify::DispositionView>,
    pub classify_policy_version: &'static str,
}

pub fn finding_page_view(
    connection: &rusqlite::Connection,
    query: &exposure::contract::ExposureFindingPageQuery,
) -> Result<ExposureFindingPageView, String> {
    let page = read_finding_page(connection, query).map_err(exposure_query_error)?;
    let ids: Vec<i64> = page.items.iter().map(|row| row.id).collect();
    let dispositions = exposure::classify::dispositions_for(connection, query.project_id, &ids)?;
    Ok(ExposureFindingPageView {
        page,
        dispositions,
        classify_policy_version: exposure::classify::CLASSIFY_POLICY_VERSION,
    })
}

// 主控集成前这些命令尚未进 invoke_handler，允许暂时未被调用。
#[allow(dead_code)]
#[tauri::command]
pub fn page_exposure_runs(
    state: State<AppState>,
    project_id: i64,
    page_size: i64,
    cursor: String,
) -> Result<ExposurePage<ExposureRun>, String> {
    let connection = db::open(&state.db_path)?;
    read_run_page(&connection, project_id, page_size, &cursor).map_err(exposure_query_error)
}

/// The sources of a run, verified to belong to the project the caller claims.
// 主控集成前这些命令尚未进 invoke_handler，允许暂时未被调用。
#[allow(dead_code)]
#[tauri::command]
pub fn list_exposure_source_results_for_project(
    state: State<AppState>,
    run_id: i64,
    expected_project_id: i64,
) -> Result<Vec<ExposureSourceResult>, String> {
    let connection = db::open(&state.db_path)?;
    read_exposure_source_results(&connection, run_id, expected_project_id)
        .map_err(exposure_query_error)
}

// 主控集成前这些命令尚未进 invoke_handler，允许暂时未被调用。
#[allow(dead_code)]
#[tauri::command]
pub fn read_exposure_finding(
    state: State<AppState>,
    finding_id: i64,
    expected_project_id: i64,
) -> Result<ExposureFinding, String> {
    let connection = db::open(&state.db_path)?;
    load_exposure_finding(&connection, finding_id, expected_project_id)
        .map_err(exposure_query_error)
}

/// The overview numbers, with the part of the scope this round cannot reach made visible.
// 主控集成前这些命令尚未进 invoke_handler，允许暂时未被调用。
#[allow(dead_code)]
#[tauri::command]
pub fn exposure_scope_summary(
    state: State<AppState>,
    project_id: i64,
) -> Result<ExposureScopeSummary, String> {
    // G1-0: 汇总读取项目配置，先确保附加表就绪（失败即稳定错误码）。
    let connection = exposure_connection(&state.db_path)?;
    read_scope_summary(&connection, project_id).map_err(exposure_query_error)
}

/// The caps the current passive collector applies, so the UI wording cannot outrun them.
// 主控集成前这些命令尚未进 invoke_handler，允许暂时未被调用。
#[allow(dead_code)]
#[tauri::command]
pub fn exposure_known_limits() -> Result<Vec<ExposureLimit>, String> {
    Ok(passive_known_limits())
}

fn exposure_text(value: String, limit: usize) -> String {
    value.trim().chars().take(limit).collect()
}

/// G0/P1-3: every exposure command opens its connection here. The additive schema has to be
/// usable before anything is read or written; a migration/table failure is returned with a
/// stable code instead of being dropped with `.ok()` and shown as empty records or defaults.
fn exposure_connection(db_path: &std::path::Path) -> Result<rusqlite::Connection, String> {
    let connection = db::open(db_path)?;
    exposure_schema_ready(&connection)?;
    Ok(connection)
}

fn exposure_schema_ready(connection: &rusqlite::Connection) -> Result<(), String> {
    exposure::state::ensure_exposure_schema(connection)
        .map(|_| ())
        .map_err(|error| {
            format!("[exposure_schema_unavailable] 暴露面结构不可用，本次未读写任何暴露面数据：{error}")
        })
}

#[tauri::command]
pub fn exposure_summary(
    state: State<AppState>,
    project_id: i64,
) -> Result<ExposureSummary, String> {
    let connection = db::open(&state.db_path)?;
    connection.query_row(
        "SELECT (SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=?1 AND exposure_eligible=1),COUNT(*),COALESCE(SUM(status='new'),0),COALESCE(SUM(status='reviewing'),0),COALESCE(SUM(status='confirmed'),0),COALESCE(SUM(status='dismissed'),0),COALESCE(SUM(status<>'dismissed' AND severity IN ('critical','high')),0) FROM exposure_findings WHERE project_id=?1",
        [project_id],
        |row| Ok(ExposureSummary { eligible_assets: row.get(0)?, total: row.get(1)?, new_count: row.get(2)?, reviewing: row.get(3)?, confirmed: row.get(4)?, dismissed: row.get(5)?, high_risk: row.get(6)? }),
    ).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_exposure_findings(
    state: State<AppState>,
    project_id: i64,
    status: String,
    search: String,
) -> Result<Vec<ExposureFinding>, String> {
    let connection = db::open(&state.db_path)?;
    let status = if ["new", "reviewing", "confirmed", "dismissed"].contains(&status.as_str()) {
        status
    } else {
        String::new()
    };
    let search = format!("%{}%", search.trim());
    let mut statement = connection.prepare(
        "SELECT f.id,f.project_id,f.asset_id,f.category,f.title,f.source_type,f.source_url,f.evidence_excerpt,f.severity,f.confidence,f.status,f.note,COALESCE(NULLIF(a.domain,''),NULLIF(a.host,''),NULLIF(a.link,''),a.ip,''),COALESCE(o.exposure_eligible,0),f.first_seen_at,f.last_seen_at FROM exposure_findings f LEFT JOIN assets a ON a.id=f.asset_id LEFT JOIN asset_ownership_decisions o ON o.project_id=f.project_id AND o.asset_id=f.asset_id WHERE f.project_id=?1 AND (?2='' OR f.status=?2) AND (?3='%%' OR f.title LIKE ?3 OR f.source_url LIKE ?3 OR f.evidence_excerpt LIKE ?3) ORDER BY CASE f.severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 ELSE 1 END DESC,f.confidence DESC,f.last_seen_at DESC LIMIT 500"
    ).map_err(|error| error.to_string())?;
    let findings = statement
        .query_map(params![project_id, status, search], |row| {
            Ok(ExposureFinding {
                id: row.get(0)?,
                project_id: row.get(1)?,
                asset_id: row.get(2)?,
                category: row.get(3)?,
                title: row.get(4)?,
                source_type: row.get(5)?,
                source_url: row.get(6)?,
                evidence_excerpt: row.get(7)?,
                severity: row.get(8)?,
                confidence: row.get(9)?,
                status: row.get(10)?,
                note: row.get(11)?,
                asset_label: row.get(12)?,
                exposure_eligible: row.get::<_, i64>(13)? != 0,
                first_seen_at: row.get(14)?,
                last_seen_at: row.get(15)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(findings)
}

#[tauri::command]
pub fn save_exposure_finding(
    state: State<AppState>,
    input: ExposureFindingInput,
) -> Result<i64, String> {
    let connection = db::open(&state.db_path)?;
    save_exposure_finding_in(&connection, input)
}

/// FIX3-R8（自查）：人工录入的线索标题/来源地址/摘录可能粘进口令或令牌，落库前统一脱敏；
/// 指纹按脱敏后的文本计算，脱敏改写过时并入原文的不可逆摘要，两条秘密不同的线索不会被合并。
pub fn save_exposure_finding_in(
    connection: &rusqlite::Connection,
    mut input: ExposureFindingInput,
) -> Result<i64, String> {
    let categories = [
        "source_code",
        "document",
        "credential",
        "storage",
        "repository",
        "metadata",
        "other",
    ];
    let severities = ["low", "medium", "high", "critical"];
    if !categories.contains(&input.category.as_str())
        || !severities.contains(&input.severity.as_str())
    {
        return Err("暴露面类别或风险等级无效".into());
    }
    input.title = exposure_text(input.title, 240);
    input.source_type = exposure_text(input.source_type, 40);
    input.source_url = exposure_text(input.source_url, 2000);
    input.evidence_excerpt = exposure_text(input.evidence_excerpt, 4000);
    input.confidence = input.confidence.clamp(0, 100);
    if input.title.is_empty() || input.source_url.is_empty() {
        return Err("请填写标题和公开来源地址".into());
    }
    let raw_title = input.title.to_lowercase();
    let raw_url = input.source_url.to_lowercase();
    input.title = exposure::redaction::redact_text(&input.title);
    input.source_url = exposure::redaction::redact_text(&input.source_url);
    input.evidence_excerpt = exposure::redaction::redact_text(&input.evidence_excerpt);
    if let Some(asset_id) = input.asset_id {
        let eligible: i64=connection.query_row("SELECT COUNT(*) FROM asset_ownership_decisions WHERE project_id=?1 AND asset_id=?2 AND exposure_eligible=1",params![input.project_id,asset_id],|row|row.get(0)).map_err(|error|error.to_string())?;
        if eligible == 0 {
            return Err("该资产尚未通过归属与授权门禁，不能进入暴露面".into());
        }
    }
    let fingerprint = exposure::redaction::idempotency_hash(
        &format!("{}|{}|{}", input.category, input.source_url.to_lowercase(), input.title.to_lowercase()),
        &[&raw_url, &raw_title],
    );
    connection.execute("INSERT INTO exposure_findings(project_id,asset_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(project_id,fingerprint) DO UPDATE SET asset_id=COALESCE(excluded.asset_id,exposure_findings.asset_id),evidence_excerpt=excluded.evidence_excerpt,severity=excluded.severity,confidence=MAX(exposure_findings.confidence,excluded.confidence),last_seen_at=datetime('now','localtime'),updated_at=datetime('now','localtime')",params![input.project_id,input.asset_id,input.category,input.title,input.source_type,input.source_url,input.evidence_excerpt,input.severity,input.confidence,fingerprint]).map_err(|error|error.to_string())?;
    connection
        .query_row(
            "SELECT id FROM exposure_findings WHERE project_id=?1 AND fingerprint=?2",
            params![input.project_id, fingerprint],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

fn exposure_run_from_row(row: &Row<'_>) -> rusqlite::Result<ExposureRun> {
    Ok(ExposureRun {
        id: row.get(0)?,
        project_id: row.get(1)?,
        status: row.get(2)?,
        eligible_assets: row.get(3)?,
        scanned_assets: row.get(4)?,
        fetched_resources: row.get(5)?,
        findings: row.get(6)?,
        stage: row.get(7)?,
        current_source: row.get(8)?,
        error: row.get(9)?,
        created_at: row.get(10)?,
        started_at: row.get(11)?,
        completed_at: row.get(12)?,
    })
}

#[tauri::command]
pub fn list_exposure_runs(
    state: State<AppState>,
    project_id: i64,
) -> Result<Vec<ExposureRun>, String> {
    let connection = db::open(&state.db_path)?;
    let mut statement=connection.prepare("SELECT id,project_id,status,eligible_assets,scanned_assets,fetched_resources,findings,stage,current_source,error,created_at,started_at,completed_at FROM exposure_runs WHERE project_id=?1 ORDER BY id DESC LIMIT 20").map_err(|error|error.to_string())?;
    let rows = statement
        .query_map([project_id], exposure_run_from_row)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[tauri::command]
pub fn list_exposure_source_results(
    state: State<AppState>,
    run_id: i64,
    expected_project_id: i64,
) -> Result<Vec<ExposureSourceResult>, String> {
    // FIX3-R3：旧入口也必须带项目并校验轮次归属；不再按裸 runId 读别的项目的来源结果。
    let connection = db::open(&state.db_path)?;
    read_exposure_source_results(&connection, run_id, expected_project_id).map_err(exposure_query_error)
}

#[tauri::command]
pub fn cancel_exposure_scan(state: State<AppState>, project_id: i64, run_id: i64) -> Result<(), String> {
    cancel_exposure_scan_in(&db::open(&state.db_path)?, project_id, run_id)
}

/// FIX2-4：先核对任务属于调用方的项目，不属于或不存在就拒绝，不写库；更新语句本身也带项目条件。
pub fn cancel_exposure_scan_in(connection: &rusqlite::Connection, project_id: i64, run_id: i64) -> Result<(), String> {
    let owner: Option<i64> = connection
        .query_row("SELECT project_id FROM exposure_runs WHERE id=?1", [run_id], |row| row.get(0))
        .optional()
        .map_err(|error| error.to_string())?;
    match owner {
        None => return Err(format!("[exposure_run_not_found] 扫描任务 {run_id} 不存在")),
        Some(owner) if owner != project_id => {
            return Err(format!("[exposure_run_project_mismatch] 扫描任务 {run_id} 不属于项目 {project_id}，拒绝取消"))
        }
        Some(_) => {}
    }
    connection.execute("UPDATE exposure_runs SET cancel_requested=1,stage='cancelling' WHERE id=?1 AND project_id=?2 AND status IN ('queued','running')",params![run_id, project_id]).map_err(|error|error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn export_exposure_findings(
    state: State<AppState>,
    project_id: i64,
    kind: Option<String>,
) -> Result<ExportResult, String> {
    let connection = exposure_connection(&state.db_path)?;
    fs::create_dir_all(&state.export_dir).map_err(|error| error.to_string())?;
    let (path, written) = export_exposure_in(
        &connection,
        &state.export_dir,
        project_id,
        kind.as_deref().unwrap_or("findings"),
        chrono::Local::now(),
    )?;
    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
        rows: written as i64,
    })
}

/// G4-3 the export body: `findings` (legacy 14 columns + classification/incident/remediation
/// context) or `ledger` (rowKind rows that reconcile). The file is named
/// `<项目名>_暴露面[_事件台账]_<本地时间+时区>.csv`, made unique, never overwritten. Hash and row
/// count come from `verify_exposure_export` reading the landed file back.
pub fn export_exposure_in(
    connection: &rusqlite::Connection,
    dir: &std::path::Path,
    project_id: i64,
    kind: &str,
    now: chrono::DateTime<chrono::Local>,
) -> Result<(std::path::PathBuf, usize), String> {
    let name: String = connection
        .query_row("SELECT name FROM projects WHERE id=?1", [project_id], |row| row.get(0))
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("[exposure_project_invalid] 项目 {project_id} 不存在"))?;
    let (suffix, headers, rows): (&str, Vec<&str>, Vec<Vec<String>>) = match kind {
        "findings" => ("", exposure::export::headers_v2(), exposure::export::finding_rows_v2(connection, project_id)?),
        "ledger" => {
            let coverage = read_scope_summary(connection, project_id).ok().and_then(|summary| summary.coverage);
            (
                "_事件台账",
                exposure::export::LEDGER_HEADERS.to_vec(),
                exposure::export::ledger_rows(connection, project_id, coverage.as_ref())?,
            )
        }
        other => return Err(format!("[exposure_export_kind_invalid] 未知导出类型 {other}，只能是 findings 或 ledger")),
    };
    let file = exposure::export::export_file_name(&name, project_id, suffix, now, &|candidate| {
        dir.join(candidate).symlink_metadata().is_ok()
    });
    let path = dir.join(file);
    let written = exposure::export::write_csv_with(&path, &headers, &rows)?;
    Ok((path, written))
}

/// G4-3 one page of the deduplicated incident queue (keyset). Not registered yet (IR-15).
#[allow(dead_code)]
#[tauri::command]
pub fn page_exposure_incidents(
    state: State<AppState>,
    project_id: i64,
    status: Option<String>,
    review_needed_only: Option<bool>,
    page_size: i64,
    cursor: Option<String>,
) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::incidents::page_incidents(
        &connection,
        project_id,
        status.as_deref().unwrap_or(""),
        review_needed_only.unwrap_or(false),
        page_size,
        cursor.as_deref().unwrap_or(""),
    )
}

/// G4-0 one incident with locations, unit links and its action trail. Not registered (IR-15).
#[allow(dead_code)]
#[tauri::command]
pub fn read_exposure_incident(state: State<AppState>, project_id: i64, incident_id: i64) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::incidents::read_incident(&connection, project_id, incident_id)
}

/// G4-3 the locations of one incident, a page at a time. Not registered (IR-15).
#[allow(dead_code)]
#[tauri::command]
pub fn page_exposure_incident_locations(
    state: State<AppState>,
    project_id: i64,
    incident_id: i64,
    page_size: i64,
    cursor: Option<String>,
) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::incidents::page_locations(&connection, project_id, incident_id, page_size, cursor.as_deref().unwrap_or(""))
}

/// P1 UI 闭环：覆盖矩阵一行的钻取（时间窗口、单位维度、工作项明细分页）。只读。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn exposure_coverage_drilldown(
    state: State<AppState>,
    project_id: i64,
    source_key: String,
    window_days: i64,
    unit_entity_id: Option<i64>,
    gaps_only: bool,
    page_size: i64,
    cursor: Option<String>,
) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::coverage::coverage_drilldown(
        &connection,
        project_id,
        &source_key,
        window_days,
        unit_entity_id,
        gaps_only,
        page_size,
        cursor.as_deref().unwrap_or(""),
    )
}

/// G4-0 one human action on an incident (review / merge / split / link_entity /
/// record_action). Handling actions only record what a person did: nothing is sent to
/// anyone. Not registered (IR-15).
#[allow(dead_code)]
#[tauri::command]
pub fn act_on_exposure_incident(state: State<AppState>, project_id: i64, action: JsonValue) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::incidents::act(&connection, project_id, &action)
}

/// G4-0 proposes incidents over the whole project's history (rounds before G4 never got
/// proposals). Only candidates; human verdicts stay. Not registered (IR-15).
#[allow(dead_code)]
#[tauri::command]
pub fn propose_exposure_incidents(state: State<AppState>, project_id: i64) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    let report = exposure::incidents::propose(&connection, project_id, None)?;
    Ok(serde_json::json!({
        "created": report.created,
        "attachedLocations": report.attached_locations,
        "reviewFlagged": report.review_flagged,
        "groupsConsidered": report.groups_considered,
    }))
}

/// §E4 只读核验一份已导出的暴露面证据：必须是普通文件、不得经由符号链接、列数与表头一致，
/// 并回给字节数、SHA-256 与规则/脱敏版本，让页面能说明“这份 CSV 来自哪套规则”。
/// 路径必须落在本项目的导出目录内，越界或指向别处一律拒绝。
#[allow(dead_code)]
#[tauri::command]
pub fn verify_exposure_export(state: State<AppState>, path: String) -> Result<JsonValue, String> {
    let requested = std::path::Path::new(&path);
    let name = requested
        .file_name()
        .ok_or_else(|| "exposure_export_target_unsafe:没有可核验的文件名".to_string())?;
    let parent = requested
        .parent()
        .filter(|row| !row.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    // Canonicalizing resolves every link on the way in, so a symlinked "export dir" that
    // points at some other part of the disk no longer equals the app's own export root.
    let root = std::fs::canonicalize(&state.export_dir)
        .map_err(|error| format!("暴露面导出目录不可读：{error}"))?;
    let resolved = std::fs::canonicalize(parent)
        .map_err(|error| format!("exposure_export_target_unsafe：目录不可解析：{error}"))?;
    if resolved != root {
        return Err(
            "exposure_export_target_unsafe:只允许核验本应用导出目录里的文件，其他位置一律拒绝"
                .to_string(),
        );
    }
    exposure::export::verify_written(&root.join(name))
}

/// §E5 带版本冲突检查的人工复核：调用方必须交回它读到的 revision、操作者与理由，
/// 队列已前进就整笔拒绝；观察历史只读，不会被这次写入改写。
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn review_exposure_finding_scoped(
    state: State<AppState>,
    project_id: i64,
    finding_id: i64,
    expected_revision: i64,
    status: String,
    actor: String,
    reason: String,
    references: Vec<i64>,
    expected_review_event: i64,
) -> Result<i64, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::remediation::review_scoped_cas(
        &connection,
        project_id,
        finding_id,
        expected_revision,
        Some(expected_review_event),
        &status,
        &actor,
        &reason,
        &references,
    )
}

/// §E5 只读复核轨迹：谁在什么时候按哪些证据把结论从什么改成了什么。
#[allow(dead_code)]
#[tauri::command]
pub fn list_exposure_review_trail(
    state: State<AppState>,
    project_id: i64,
    finding_id: i64,
) -> Result<Vec<JsonValue>, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::remediation::review_trail(&connection, project_id, finding_id)
}

/// §E5 登记整改：绑定原 finding、原观察与来源轮次；同一证据重复登记只更新说明。
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub fn save_exposure_remediation(
    state: State<AppState>,
    project_id: i64,
    finding_id: i64,
    observation_id: Option<i64>,
    run_id: i64,
    actor: String,
    note: String,
    status: String,
) -> Result<i64, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::remediation::record_remediation(
        &connection,
        project_id,
        finding_id,
        observation_id,
        run_id,
        &actor,
        &note,
        &status,
    )
}

/// §E5 一页整改记录（含复测轮次 id），只读、按项目过滤、游标分页。
#[allow(dead_code)]
#[tauri::command]
pub fn page_exposure_remediations(
    state: State<AppState>,
    project_id: i64,
    status: String,
    page_size: i64,
    cursor: String,
) -> Result<exposure::contract::ExposurePage<exposure::remediation::RemediationRow>, String> {
    let connection = db::open(&state.db_path)?;
    exposure::state::ensure_exposure_schema(&connection).map_err(|error| {
        exposure_query_error(exposure::contract::ExposureQueryError::storage(&error))
    })?;
    exposure::remediation::page_remediations(&connection, project_id, &status, &cursor, page_size)
        .map_err(exposure_query_error)
}

/// §E5 复测另起一轮：先按当前范围重新验权，再冻结当前配置建新计划，并把新轮绑定到整改上。
/// 旧观察一行都不改；撤权或没有准入资产时直接拒绝启动。
#[allow(dead_code)]
#[tauri::command]
pub fn start_exposure_retest(
    state: State<AppState>,
    project_id: i64,
    remediation_id: i64,
) -> Result<i64, String> {
    let connection = exposure_connection(&state.db_path)?;
    let settings = exposure_scan_settings(&connection, project_id)?;
    let started =
        exposure::remediation::start_retest(&connection, project_id, remediation_id, &settings)?;
    if !started.reused_active {
        let db_path = state.db_path.clone();
        let owner = format!("exposure-retest-{}", Uuid::new_v4());
        thread::spawn(move || {
            execute_exposure_run(db_path, started.run_id, project_id, owner, settings)
        });
    }
    Ok(started.run_id)
}

/// §E5 只读复测比对结果：本轮看到了什么字节、与原结论基线是否一致，以及能不能关闭的依据。
#[allow(dead_code)]
#[tauri::command]
pub fn read_exposure_retest_outcome(
    state: State<AppState>,
    project_id: i64,
    remediation_id: i64,
) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::remediation::retest_outcome(&connection, project_id, remediation_id)
}

/// §E5 人工关闭或驳回整改：关闭必须拿到复测依据（内容确实变了），否则拒绝。
#[allow(dead_code)]
#[tauri::command]
pub fn mark_exposure_remediation(
    state: State<AppState>,
    project_id: i64,
    remediation_id: i64,
    status: String,
    actor: String,
    reason: String,
) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::remediation::mark_remediation(
        &connection,
        project_id,
        remediation_id,
        &status,
        &actor,
        &reason,
    )
}

/// §E5 结论分类：已确认 / 待复核 / 来源不可达 / 未覆盖 / 规则命中 / 证据不足 / 已排除。
/// 每个数字都来自真实行，"没有证据"绝不能被读成"没有问题"。只读。
#[allow(dead_code)]
#[tauri::command]
pub fn exposure_review_taxonomy(
    state: State<AppState>,
    project_id: i64,
) -> Result<Vec<exposure::remediation::TaxonomyRow>, String> {
    let connection = db::open(&state.db_path)?;
    exposure::state::ensure_exposure_schema(&connection).map_err(|error| {
        exposure_query_error(exposure::contract::ExposureQueryError::storage(&error))
    })?;
    exposure::remediation::taxonomy(&connection, project_id).map_err(|error| {
        exposure_query_error(exposure::contract::ExposureQueryError::storage(&error))
    })
}

// `exposure_tests` 固定这两个旧解析器的历史行为；生产路径已改走
// `exposure::rules`，那里会真正解析 Source Map 并按内容分级。
#[allow(dead_code)]
fn quoted_resource_urls(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    for marker in ["href=\"", "src=\"", "href='", "src='", "sourceMappingURL="] {
        let mut rest = text;
        while let Some(index) = rest.find(marker) {
            rest = &rest[index + marker.len()..];
            let end = if marker.ends_with('=') {
                rest.find(|c: char| c.is_whitespace() || c == '\"' || c == '\'')
                    .unwrap_or(rest.len())
            } else {
                rest.find(if marker.ends_with('\"') { '\"' } else { '\'' })
                    .unwrap_or(rest.len())
            };
            let value = rest[..end].trim();
            if !value.is_empty() && value.len() <= 2000 {
                values.push(value.to_string());
            }
            rest = &rest[end.min(rest.len())..];
            if values.len() >= 200 {
                break;
            }
        }
    }
    values
}

#[allow(dead_code)]
fn masked_sensitive_evidence(text: &str) -> Option<String> {
    let keys = [
        "api_key",
        "apikey",
        "access_key",
        "secret_key",
        "client_secret",
        "private_token",
        "authorization",
    ];
    for line in text.lines().take(4000) {
        let lower = line.to_lowercase();
        if !keys.iter().any(|key| lower.contains(key))
            || (!line.contains('=') && !line.contains(':'))
        {
            continue;
        }
        if [
            "example",
            "placeholder",
            "your_",
            "xxxx",
            "test_key",
            "process.env",
        ]
        .iter()
        .any(|term| lower.contains(term))
        {
            continue;
        }
        let separator = line.find('=').or_else(|| line.find(':'))?;
        let key = line[..separator]
            .trim()
            .chars()
            .take(60)
            .collect::<String>();
        let value = line[separator + 1..]
            .trim()
            .trim_matches(['\"', '\'', ';', ',', ' ']);
        if value.len() < 8 || value.contains(' ') || value.len() > 500 {
            continue;
        }
        let prefix = value.chars().take(3).collect::<String>();
        let suffix = value
            .chars()
            .rev()
            .take(2)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        return Some(format!(
            "公开资源出现字段 {key}，值已脱敏：{prefix}***{suffix}"
        ));
    }
    None
}

// 旧的 ad-hoc 采集器（一次线程、静默 LIMIT 200、只按成功响应计数、终态靠线程正常退出推断）
// 已由 `exposure/` 下的工作项 + 授权范围 + 有界客户端 + 逐项终态聚合取代。
// `quoted_resource_urls` 与 `masked_sensitive_evidence` 保留原样，由 `exposure_tests`
// 固定历史解析行为作为对照；生产路径不再调用它们。

/// §E2 采集配置来自该项目自己的持久化设置（`exposure_settings` 表），没有保存过才是代码默认值。
/// G0/P1-3：结构不可用或已保存配置损坏时拒绝启动，绝不按默认预算/来源悄悄继续采集。
#[allow(dead_code)]
fn exposure_scan_settings(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<JsonValue, String> {
    exposure_schema_ready(connection)?;
    Ok(exposure::settings::load(connection, project_id)?.to_settings_json())
}


/// 每次出站前重新确认：项目仍是 active、归属仍是 eligible、origin 仍在被批准范围内。
/// 撤销、移出项目、归档和范围过期都在这一层被拦住，而不是等到下一轮。
#[allow(dead_code)]
fn revalidate_origin(
    connection: &rusqlite::Connection,
    project_id: i64,
    settings: &JsonValue,
    origin: &str,
) -> Result<(), exposure::scope::ExposureDenial> {
    let fresh = exposure::scope::ExposureScope::load(connection, project_id, settings)?;
    let url = reqwest::Url::parse(&format!("{origin}/")).map_err(|error| {
        exposure::scope::ExposureDenial::new("origin_invalid", format!("origin 无法解析：{error}"))
    })?;
    fresh
        .authorize(&url, &exposure::state::utc_now())
        .map(|_| ())
}

/// P1-3: 外部来源（证书/归档/代码平台/文档链接）的查询词来自整个工作空间的确认域名，
/// 不对应单个 origin。每一项发出请求前重读授权：项目归档、或本轮计划时的任一准入资产
/// 已被撤权/移出，本项记为授权阻塞，零请求。
fn external_scope_still_granted(
    connection: &rusqlite::Connection,
    project_id: i64,
    settings: &JsonValue,
    planned_assets: &[i64],
) -> Result<(), String> {
    let fresh = exposure::scope::ExposureScope::load(connection, project_id, settings)
        .map_err(|denial| format!("{}:{}", denial.code, denial.message))?;
    if fresh.grants.is_empty() {
        return Err("scope_empty:当前工作空间没有准入资产，外部来源不再查询".to_string());
    }
    if let Some(missing) = planned_assets
        .iter()
        .find(|asset| !fresh.grants.iter().any(|grant| grant.asset_id == Some(**asset)))
    {
        return Err(format!(
            "authorization_changed:本轮计划时的准入资产 {missing} 已撤权或移出，外部来源不再以本轮授权查询"
        ));
    }
    Ok(())
}

/// G1-0: HTTP 客户端的超时与单响应上限只取自本轮冻结配置。
fn exposure_http_limits(
    frozen: &exposure::settings::ExposureSettings,
) -> exposure::adapters::http::PassiveHttpLimits {
    exposure::adapters::http::PassiveHttpLimits {
        timeout: std::time::Duration::from_secs(frozen.timeout_seconds.max(1) as u64),
        max_response_bytes: frozen.max_response_bytes.max(1) as usize,
        ..Default::default()
    }
}

/// G2-0 来源端点。生产版本只有常量：crt.sh、Wayback、api.github.com，私网出口关闭，
/// 凭据端口未装配（IR-04）。没有任何配置项、环境变量或 IPC 能改写它们。
pub(crate) struct SourceEndpoints {
    pub ct: Vec<String>,
    pub archive: Vec<String>,
    pub github: exposure::adapters::github::GithubEndpoint,
    pub allow_private: bool,
    pub github_credential: exposure::adapters::github::CredentialLookup,
    /// G3: explicit document URLs may use only the scheme's port in production; the fixture
    /// (random port on 127.0.0.1) needs this test-only switch.
    pub document_any_port: bool,
}

impl SourceEndpoints {
    fn production() -> Self {
        Self {
            ct: vec![exposure::adapters::certificate_transparency::ENDPOINT.to_string()],
            archive: vec![exposure::adapters::web_archive::ENDPOINT.to_string()],
            github: exposure::adapters::github::GithubEndpoint::production(),
            allow_private: false,
            github_credential: exposure::adapters::github::CredentialLookup::production(),
            document_any_port: false,
        }
    }
}

#[cfg(not(test))]
fn source_endpoints() -> SourceEndpoints {
    SourceEndpoints::production()
}

// 测试构建（cfg(test)）才有的覆盖：同一线程里的生产执行器改走 localhost fixture。
// 发布构建里这段代码不存在，生产路径只能拿到上面的常量。
#[cfg(test)]
thread_local! {
    static TEST_SOURCE_ENDPOINTS: std::cell::RefCell<Option<SourceEndpoints>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn source_endpoints() -> SourceEndpoints {
    TEST_SOURCE_ENDPOINTS
        .with(|slot| slot.borrow_mut().take())
        .unwrap_or_else(SourceEndpoints::production)
}

/// 仅测试：下一次 `execute_exposure_run` 在本线程使用这些端点（用一次即清空）。
#[cfg(test)]
pub(crate) fn use_test_endpoints_once(endpoints: SourceEndpoints) {
    TEST_SOURCE_ENDPOINTS.with(|slot| *slot.borrow_mut() = Some(endpoints));
}

/// 一个采集任务的工作线程：领取冻结计划里的工作项，逐条执行到终态。
#[allow(dead_code)]
fn execute_exposure_run(
    db_path: PathBuf,
    run_id: i64,
    project_id: i64,
    owner: String,
    settings: JsonValue,
) {
    let result = (|| -> Result<exposure::orchestrator::RunReport, String> {
        let connection = db::open(&db_path)?;
        // G1-0: 计划与冻结配置必须可读；缺失或损坏时本轮以稳定错误码失败，
        // 不再按 12 秒 / 1MB / 200 行这些默认值继续采集。
        let plan = exposure::recovery::load_plan(&connection, run_id, project_id)?;
        let frozen = exposure::recovery::frozen_settings_strict(&plan)?;
        let scope = exposure::scope::ExposureScope::load(&connection, project_id, &settings)
            .map_err(|denial| format!("{}:{}", denial.code, denial.message))?;
        let client = exposure::adapters::http::PassiveClient::new(exposure_http_limits(&frozen))?;
        // 恢复轮从计划里已花掉的额度继续，不重置预算。
        let mut meter = exposure::recovery::meter_for(&plan);
        let budget = exposure::adapters::passive_web::WalkBudget::default();
        #[cfg_attr(test, allow(unused_mut))]
        let mut endpoints = source_endpoints();
        // IR-04: a release build reads the keychain for this round's reference. Tests keep
        // the injected endpoint, so a fixture token never comes from the login keychain.
        #[cfg(not(test))]
        {
            endpoints.github_credential =
                exposure::adapters::github::CredentialLookup::from_ref(&frozen.github_credential_ref);
        }
        let ct_endpoints = endpoints.ct.clone();
        let archive_endpoints = endpoints.archive.clone();
        let archive_limit = frozen.archive_page_size;
        let ct_limit = frozen.ct_page_size;
        let mut github_session = exposure::adapters::github::GithubSession::default();
        let official_owners = frozen.github_official_owners.clone();
        // G3: the exact URLs frozen with this round and the confirmed names they are read for.
        let document_urls = frozen.document_urls.clone();
        let document_terms = exposure::adapters::documents::group_terms(&connection, project_id)?;
        let planned_assets: Vec<i64> = {
            let mut statement = connection
                .prepare("SELECT DISTINCT asset_id FROM exposure_work_items WHERE run_id=?1 AND kind=?2 AND asset_id IS NOT NULL")
                .map_err(|error| error.to_string())?;
            let rows = statement
                .query_map(rusqlite::params![run_id, exposure::orchestrator::KIND_PASSIVE_WEB], |row| row.get::<_, i64>(0))
                .map_err(|error| error.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|error| error.to_string())?
        };
        let mut executor = |item: &exposure::work_items::WorkItem|
         -> Result<exposure::orchestrator::ItemOutcome, String> {
            let cancelled = || exposure::orchestrator::cancel_requested(&connection, run_id);
            let origin = item.origin.clone();
            let revalidate = || revalidate_origin(&connection, project_id, &settings, &origin);
            // G2: 适配器报的是整轮计量器的累计值；这里换成本项自己的增量，
            // 否则第二个工作项起每一项都会把前面的请求/字节再记一遍。
            let before = (meter.requests_used, meter.bytes_used);
            let external = !matches!(
                item.kind.as_str(),
                exposure::orchestrator::KIND_PASSIVE_WEB
                    | exposure::adapters::documents::KIND_DOCUMENT_IMPORT
                    | exposure::adapters::documents::KIND_INTEL_IMPORT
            );
            if external {
                if let Err(reason) = external_scope_still_granted(&connection, project_id, &settings, &planned_assets) {
                    return Ok(exposure::orchestrator::ItemOutcome {
                        status: Some(exposure::state::WorkItemStatus::BlockedAuthorization),
                        reason,
                        ..Default::default()
                    });
                }
            }
            // FIX3-R4：外部来源在本项内部每一页/每一次请求前都重读授权：撤权后不再发出新请求。
            // FIX4 P1／新增 T21：记下最后一次确认授权仍有效的时间，撤权后保留的结果据此标注。
            let verified_at = std::cell::RefCell::new(chrono::Local::now().to_rfc3339());
            let stop = || {
                if cancelled() {
                    return true;
                }
                if !external {
                    return false;
                }
                let withdrawn = external_scope_still_granted(&connection, project_id, &settings, &planned_assets).is_err();
                if !withdrawn {
                    *verified_at.borrow_mut() = chrono::Local::now().to_rfc3339();
                }
                withdrawn
            };
            let mut outcome = match item.kind.as_str() {
                exposure::orchestrator::KIND_PASSIVE_WEB => exposure::adapters::passive_web::execute(
                    &client,
                    &scope,
                    item,
                    &mut meter,
                    &budget,
                    &revalidate,
                    &cancelled,
                ),
                exposure::orchestrator::KIND_CERTIFICATE_TRANSPARENCY => {
                    exposure::adapters::certificate_transparency::execute_bounded(
                        &client,
                        item,
                        &mut meter,
                        &ct_endpoints,
                        endpoints.allow_private,
                        ct_limit,
                        &stop,
                    )
                }
                exposure::orchestrator::KIND_WEB_ARCHIVE => exposure::adapters::web_archive::execute(
                    &client,
                    item,
                    &mut meter,
                    &archive_endpoints,
                    endpoints.allow_private,
                    archive_limit,
                    &stop,
                ),
                exposure::orchestrator::KIND_GITHUB_REPOSITORIES => {
                    exposure::adapters::github::execute_repositories(
                        &client,
                        item,
                        &mut meter,
                        &endpoints.github,
                        &mut github_session,
                        &official_owners,
                        &stop,
                    )
                }
                exposure::orchestrator::KIND_GITHUB_CODE => exposure::adapters::github::execute_code(
                    &client,
                    item,
                    &mut meter,
                    &endpoints.github,
                    &mut github_session,
                    &endpoints.github_credential,
                    &stop,
                ),
                exposure::orchestrator::KIND_DOCUMENT_LINK => {
                    let context = exposure::adapters::documents::LinkContext {
                        client: &client,
                        terms: &document_terms,
                        allow_private: endpoints.allow_private,
                        any_port: endpoints.document_any_port,
                        cancelled: &stop,
                    };
                    let entry = document_urls.iter().find(|row| row.url == item.entity);
                    exposure::adapters::documents::execute_link(&context, item, &mut meter, entry)
                }
                // G3: an import keeps its text in memory only. If its round is ever resumed by
                // the network worker, the file is gone; say so instead of "unknown source".
                exposure::adapters::documents::KIND_DOCUMENT_IMPORT
                | exposure::adapters::documents::KIND_INTEL_IMPORT => exposure::orchestrator::ItemOutcome {
                    status: Some(exposure::state::WorkItemStatus::SkippedUnconfigured),
                    reason: "import_not_resumable：导入正文不保存在库中，中断的导入批次不能恢复，请重新导入".to_string(),
                    source_status: Some(exposure::state::SourceStatus::Unsupported),
                    source_reason: "import_not_resumable：中断的导入批次不能恢复，请重新导入".to_string(),
                    ..Default::default()
                },
                other => exposure::orchestrator::ItemOutcome {
                    status: Some(exposure::state::WorkItemStatus::SkippedUnconfigured),
                    reason: format!("未知来源 {other}，本轮未执行"),
                    source_status: Some(exposure::state::SourceStatus::Unsupported),
                    source_reason: format!("{other} 尚未实现"),
                    ..Default::default()
                },
            };
            // FIX3-R4 / FIX4 P1／新增 T21：本项进行中被撤权（含最后一个在途请求返回之后）：记为授权阻塞，
            // 撤权后零新请求；撤权前已按授权发出的请求所取得的观察保留，并逐条标注授权时间点；
            // 待跟进的新发现不入队；已发出的请求照实记账。
            let withdrawn = if external {
                external_scope_still_granted(&connection, project_id, &settings, &planned_assets).err()
            } else if item.kind == exposure::orchestrator::KIND_PASSIVE_WEB && revalidate().is_err() {
                Some("authorization_changed：本项进行中该站点授权被撤销".to_string())
            } else {
                None
            };
            if let Some(reason) = withdrawn {
                let verified = verified_at.borrow().clone();
                let detected = chrono::Local::now().to_rfc3339();
                let kept = outcome.observations.len();
                for observation in &mut outcome.observations {
                    let mut extra = match observation.extra.take() {
                        serde_json::Value::Null => serde_json::json!({}),
                        value @ serde_json::Value::Object(_) => value,
                        other => serde_json::json!({ "detail": other }),
                    };
                    extra["scopeGrant"] = serde_json::json!({
                        "verifiedAt": verified,
                        "withdrawnDetectedAt": detected,
                        "basis": "撤权前已按授权发出的请求所得，撤权后未再发请求",
                    });
                    observation.extra = extra;
                }
                outcome.pending.clear();
                outcome.status = Some(exposure::state::WorkItemStatus::BlockedAuthorization);
                outcome.reason = format!(
                    "{reason}；撤权后零新请求，撤权前已取得的 {kept} 条观察已保留并标注授权时间点（最后确认授权 {verified}）"
                );
                outcome.source_status = if kept > 0 { Some(exposure::state::SourceStatus::Partial) } else { None };
                outcome.source_reason = if kept > 0 { outcome.reason.clone() } else { String::new() };
            }
            exposure::adapters::documents::attach_same_document(&connection, project_id, &mut outcome.observations)?;
            outcome.requests = (meter.requests_used - before.0).max(0);
            outcome.bytes = (meter.bytes_used - before.1).max(0);
            Ok(outcome)
        };
        exposure::orchestrator::run_plan(&connection, run_id, &owner, &mut executor)
    })();
    if let Err(error) = result {
        // 基础设施失败只能报 failed，并把原因留给页面；绝不写 completed。
        if let Ok(connection) = db::open(&db_path) {
            let _ = exposure::orchestrator::finalize(&connection, run_id, Some(&error));
        }
    }
}

#[tauri::command]
pub fn start_exposure_scan(state: State<AppState>, project_id: i64) -> Result<i64, String> {
    let connection = exposure_connection(&state.db_path)?;
    let settings = exposure_scan_settings(&connection, project_id)?;
    let started = exposure::orchestrator::start_or_reuse(&connection, project_id, &settings)?;
    if started.reused_active {
        // 同一项目并发启动返回同一个活跃任务，而不是第二条 run。
        return Ok(started.run_id);
    }
    let db_path = state.db_path.clone();
    let owner = format!("exposure-{}", Uuid::new_v4());
    thread::spawn(move || {
        execute_exposure_run(db_path, started.run_id, project_id, owner, settings)
    });
    Ok(started.run_id)
}

/// 恢复一个被中断的采集：冻结的计划、配置或授权范围一旦变化就拒绝复用，
/// 只允许另起新一轮；已保存的确定性观察不会重新抓取。
#[allow(dead_code)]
#[tauri::command]
pub fn resume_exposure_scan(
    state: State<AppState>,
    run_id: i64,
    expected_project_id: i64,
) -> Result<i64, String> {
    // 结构是附加的：恢复入口先确保自己需要的表存在（失败即拒绝），再谈冻结计划。
    let connection = exposure_connection(&state.db_path)?;
    let plan_row = exposure::recovery::load_plan(&connection, run_id, expected_project_id)?;
    // §E2 恢复用这一轮自己冻结的配置，而不是当前配置：改配置只影响新轮，
    // 不能把已花掉的额度清零，也不能悄悄把旧轮指向新的目标。
    let settings = exposure::recovery::frozen_settings_strict(&plan_row)?.to_settings_json();
    let scope = exposure::scope::ExposureScope::load(&connection, expected_project_id, &settings)
        .map_err(|denial| format!("{}:{}", denial.code, denial.message))?;
    exposure::recovery::verify_frozen(&plan_row, &scope, &settings)?;
    let status: String = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !matches!(
        status.as_str(),
        "interrupted" | "cancelled" | "queued" | "running"
    ) {
        return Err(format!("任务 {run_id} 已是终态 {status}，不能重复恢复"));
    }
    let db_path = state.db_path.clone();
    let owner = format!("exposure-{}", Uuid::new_v4());
    thread::spawn(move || {
        execute_exposure_run(db_path, run_id, expected_project_id, owner, settings)
    });
    Ok(run_id)
}

/// §E2 读取该项目当前的暴露面采集配置（只读）。没有保存过时返回代码默认值与 version=0，
/// 页面据此知道"这是默认，不是用户设定"。
#[allow(dead_code)]
#[tauri::command]
pub fn get_exposure_settings(state: State<AppState>, project_id: i64) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure_settings_view_in(&connection, project_id)
}

/// 只读配置视图。已保存配置损坏时返回 `exposure_settings_corrupt`，不显示成默认值。
fn exposure_settings_view_in(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<JsonValue, String> {
    let saved: Option<i64> = connection
        .query_row(
            "SELECT version FROM exposure_settings WHERE project_id=?1",
            [project_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let settings = exposure::settings::load(connection, project_id)?;
    let mut value = settings.to_settings_json();
    value["saved"] = serde_json::json!(saved.is_some());
    value["storedVersion"] = serde_json::json!(saved.unwrap_or(0));
    value["schemaVersion"] = serde_json::json!(exposure::settings::SETTINGS_SCHEMA_VERSION);
    // G1-0: 并行度字段保留兼容，但执行器是串行的；页面据此显示“未实现，实际串行”。
    value["effective"] = serde_json::json!({
        "concurrency": exposure::settings::EFFECTIVE_CONCURRENCY,
        "concurrencyImplemented": false,
        "concurrencyNote": "执行器串行：同一时刻只处理一个工作项、一个请求。并行度已从采集合同移除（IR-08）",
        "ctPageSizeMeaning": "每个域名保留的证书名称上限（crt.sh 一次返回全部，超出部分不记录并标为配额阻塞）",
        "archivePageSizeMeaning": "每个域名读取的 Wayback 索引行数（只读一页）",
        "knownLimits": exposure::contract::known_limits_for(&settings),
    });
    // G2: 每个 GitHub 能力单独说明是否就绪；元数据可用不代表代码搜索可用。
    let mut capabilities = exposure::adapters::github::capabilities(
        &exposure::adapters::github::CredentialLookup::from_ref(&settings.github_credential_ref),
    );
    // G3: document URL, file import, intel import and the named unsupported directions.
    if let (Some(rows), JsonValue::Array(extra)) = (
        capabilities.as_array_mut(),
        exposure::adapters::documents::capabilities(settings.document_urls.len()),
    ) {
        rows.extend(extra);
    }
    value["sourceCapabilities"] = capabilities;
    value["defaultSources"] = serde_json::json!(exposure::settings::DEFAULT_SOURCES);
    value["bounds"] = serde_json::json!({
        "maxRequests": exposure::settings::MAX_REQUESTS_LIMIT,
        "maxResponseBytes": exposure::settings::MAX_RESPONSE_BYTES_LIMIT,
        "maxTotalBytes": exposure::settings::MAX_TOTAL_BYTES_LIMIT,
        "maxItems": exposure::settings::MAX_ITEMS_LIMIT,
        "concurrency": exposure::settings::CONCURRENCY_LIMIT,
        "timeoutSeconds": exposure::settings::TIMEOUT_SECONDS_LIMIT,
        "pageSize": exposure::settings::PAGE_SIZE_LIMIT,
        "authorizationDays": exposure::settings::AUTHORIZATION_DAYS_LIMIT,
        "sources": exposure::settings::SOURCES,
        "units": {
            "maxRequests": "次/轮",
            "maxResponseBytes": "字节/单响应",
            "maxTotalBytes": "字节/轮",
            "maxItems": "工作项/轮",
            "timeoutSeconds": "秒",
            "ctPageSize": "名称/域名",
            "archivePageSize": "行/域名",
            "authorizationDays": "天，0 表示不设期限"
        }
    });
    Ok(value)
}

/// §E2 保存为新版本（写入）。expected_version 是编辑器实际读到的版本，落后必须被拒绝，
/// 旧值进版本历史；这一步不会改任何已经启动的轮次。
#[allow(dead_code)]
#[tauri::command]
pub fn save_exposure_settings(
    state: State<AppState>,
    project_id: i64,
    expected_version: i64,
    settings: JsonValue,
) -> Result<i64, String> {
    let connection = exposure_connection(&state.db_path)?;
    save_exposure_settings_in(&connection, project_id, expected_version, &settings)
}

/// G0/P1-1：版本比较只在 `settings::save` 的 IMMEDIATE 事务里做一次；这里不再先读后比，
/// 也不再先解析当前（可能已损坏的）配置——损坏的旧值原样进历史，新值仍可保存修复。
fn save_exposure_settings_in(
    connection: &rusqlite::Connection,
    project_id: i64,
    expected_version: i64,
    settings: &JsonValue,
) -> Result<i64, String> {
    let active: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM projects WHERE id=?1 AND status='active'",
            [project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if active == 0 {
        return Err(
            "[exposure_project_invalid] 工作空间不存在或已归档，不能写入采集配置".to_string(),
        );
    }
    let parsed = exposure::settings::ExposureSettings::from_json(project_id, settings)?;
    exposure::settings::save(connection, &parsed, expected_version)
}

/// §E2 配置历史（只读）：改了什么、什么时候、当前是哪一版。
#[allow(dead_code)]
#[tauri::command]
pub fn list_exposure_setting_versions(
    state: State<AppState>,
    project_id: i64,
    limit: i64,
) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure_setting_versions_in(&connection, project_id, limit)
}

/// G0/P1-2：`truncated` 来自真实的 limit+1 读取；无法解析的历史行标 `corrupt`，不伪装成空配置。
fn exposure_setting_versions_in(
    connection: &rusqlite::Connection,
    project_id: i64,
    limit: i64,
) -> Result<JsonValue, String> {
    let limit = limit.clamp(1, 200);
    let (rows, truncated) = exposure::settings::history_window(connection, project_id, limit)?;
    Ok(serde_json::json!({
        "items": rows
            .into_iter()
            .map(|(version, raw, created_at)| {
                let parsed = serde_json::from_str::<JsonValue>(&raw).ok();
                serde_json::json!({
                    "version": version,
                    "corrupt": parsed.is_none(),
                    "settings": parsed.unwrap_or(JsonValue::Null),
                    "createdAt": created_at,
                })
            })
            .collect::<Vec<_>>(),
        "limit": limit,
        "truncated": truncated,
    }))
}

/// 外部索引带回的新名字/新 URL：只读队列，默认 unreviewed，不会自动成为资产或获得授权。
#[allow(dead_code)]
#[tauri::command]
pub fn page_exposure_pending_discoveries(
    state: State<AppState>,
    project_id: i64,
    status: String,
    page_size: i64,
    cursor: String,
) -> Result<exposure::contract::ExposurePage<exposure::contract::ExposurePending>, String> {
    let connection = db::open(&state.db_path)?;
    exposure::state::ensure_exposure_schema(&connection).map_err(|error| {
        exposure_query_error(exposure::contract::ExposureQueryError::new(
            "exposure_query_failed",
            &error,
        ))
    })?;
    let input = exposure::contract::ExposurePendingPageInput {
        project_id,
        status,
        page_size,
        cursor,
    }
    .normalize()
    .map_err(exposure_query_error)?;
    exposure::repository::page_pending_discoveries(&connection, &input)
        .map_err(exposure_query_error)
}

/// §E3 一页内容簇/域名簇：代表资产、它来自哪一次观察与哪一轮、成员与直接/继承计数、
/// 缺口说明、规则与配置版本，以及该字节是否已有缓存结论（没有缓存时返回空串）。
/// 只读；按项目过滤，游标是簇行 id 的 keyset，页面拿不到全量簇。
#[allow(dead_code)]
#[tauri::command]
pub fn page_exposure_clusters(
    state: State<AppState>,
    project_id: i64,
    kind: String,
    search: String,
    page_size: i64,
    cursor: String,
) -> Result<exposure::contract::ExposurePage<exposure::cluster_repository::ClusterSummaryRow>, String>
{
    let connection = db::open(&state.db_path)?;
    exposure::state::ensure_exposure_schema(&connection).map_err(|error| {
        exposure_query_error(exposure::contract::ExposureQueryError::storage(&error))
    })?;
    exposure::cluster_repository::page_clusters(
        &connection,
        project_id,
        &kind,
        &search,
        &cursor,
        page_size,
    )
    .map_err(exposure_query_error)
}

/// §E3 展开一个簇的成员：每个成员自带结论还是继承、继承自谁、依据哪一轮与哪条观察，
/// 以及它现在是否仍满足准入。撤权后的成员立刻显示为失效，而不是继续算作已覆盖。
#[allow(dead_code)]
#[tauri::command]
pub fn page_exposure_cluster_members(
    state: State<AppState>,
    project_id: i64,
    cluster_id: i64,
    page_size: i64,
    cursor: String,
) -> Result<exposure::contract::ExposurePage<exposure::cluster_repository::ClusterMemberRow>, String>
{
    let connection = db::open(&state.db_path)?;
    exposure::state::ensure_exposure_schema(&connection).map_err(|error| {
        exposure_query_error(exposure::contract::ExposureQueryError::storage(&error))
    })?;
    exposure::cluster_repository::page_cluster_members(
        &connection,
        project_id,
        cluster_id,
        &cursor,
        page_size,
    )
    .map_err(exposure_query_error)
}

/// 一条线索的证据包：不可变观察 + 版本轨迹 + 是否需要复核。只读，且只返回脱敏内容。
#[allow(dead_code)]
#[tauri::command]
pub fn read_exposure_evidence(
    state: State<AppState>,
    finding_id: i64,
    expected_project_id: i64,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    exposure::state::ensure_exposure_schema(&connection).map_err(|error| {
        exposure_query_error(exposure::contract::ExposureQueryError::new(
            "exposure_query_failed",
            &error,
        ))
    })?;
    exposure::repository::exposure_evidence(&connection, finding_id, expected_project_id)
        .map_err(exposure_query_error)
}

/// G3-0 / G3-2 import entry. Registered in lib.rs (IR-14 landed 2026-10-09).
/// The whole body lives in `exposure/imports.rs`. A declared URL is never fetched.
#[tauri::command]
pub fn import_exposure_documents(
    state: State<AppState>,
    project_id: i64,
    request: JsonValue,
) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::imports::import_batch(&connection, project_id, &request)
}

/// IR-07 manual reclassification of an automatic disposition: CAS on the current
/// classification id, idempotent operation id, mandatory reason, audit row. Records only.
#[tauri::command]
pub fn reclassify_exposure_finding(
    state: State<AppState>,
    project_id: i64,
    request: JsonValue,
) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::classify::reclassify(&connection, project_id, &request)
}

/// G3-2 manual intel grade. Registered in lib.rs (IR-14 landed 2026-10-09).
#[tauri::command]
pub fn grade_exposure_intel(
    state: State<AppState>,
    project_id: i64,
    request: JsonValue,
) -> Result<JsonValue, String> {
    let connection = exposure_connection(&state.db_path)?;
    exposure::imports::grade_intel_finding(&connection, project_id, &request)
}

#[cfg(test)]
mod exposure_tests {
    use super::{masked_sensitive_evidence, quoted_resource_urls};

    #[test]
    fn passive_parser_keeps_natural_resources_and_source_maps() {
        let values = quoted_resource_urls(
            "<script src=\"/app.js\"></script>\n//# sourceMappingURL=app.js.map",
        );
        assert!(values.contains(&"/app.js".to_string()));
        assert!(values.contains(&"app.js.map".to_string()));
    }

    #[test]
    fn credential_evidence_is_masked_and_placeholders_are_ignored() {
        let evidence = masked_sensitive_evidence("client_secret = abcdefgh123456").unwrap();
        assert!(evidence.contains("abc***56"));
        assert!(!evidence.contains("abcdefgh123456"));
        assert!(masked_sensitive_evidence("api_key = YOUR_API_KEY").is_none());
    }
}
