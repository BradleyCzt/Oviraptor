// §Q5 — the export. Every cell is redacted and formula-escaped here, so no call site has to
// remember; the export carries provenance (how many observations, which rule version, whether
// a human conclusion now needs revisiting) instead of a bare summary.

use super::redaction::{escape_csv_cell, redact_text};
use super::state::EXPOSURE_RULE_VERSION;
use rusqlite::OptionalExtension;
use serde_json::{json, Value as JsonValue};

pub const HEADERS: &[&str] = &[
    "类别",
    "标题",
    "风险",
    "置信度",
    "状态",
    "来源类型",
    "来源 URL",
    "脱敏证据",
    "关联观察数",
    "需复核",
    "规则版本",
    "脱敏版本",
    "首次发现",
    "最近发现",
];

/// One row per finding, all cells already safe to open in a spreadsheet.
pub fn finding_rows(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<Vec<Vec<String>>, String> {
    let mut statement = connection
        .prepare(
            "SELECT f.category,f.title,f.severity,f.confidence,f.status,f.source_type,f.source_url,
                    f.evidence_excerpt,
                    (SELECT COUNT(*) FROM exposure_finding_observations o WHERE o.finding_id=f.id),
                    (SELECT COUNT(*) FROM exposure_finding_versions v WHERE v.finding_id=f.id AND v.review_needed=1),
                    f.first_seen_at,f.last_seen_at
             FROM exposure_findings f WHERE f.project_id=?1
             ORDER BY CASE f.severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 ELSE 1 END DESC,
                      f.confidence DESC, f.id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([project_id], |row| {
            Ok(vec![
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?.to_string(),
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, i64>(8)?.to_string(),
                row.get::<_, i64>(9)?.to_string(),
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
            ])
        })
        .map_err(|error| error.to_string())?;
    let mut collected = Vec::new();
    for row in rows.flatten() {
        let mut cells: Vec<String> = row
            .into_iter()
            .map(|value| escape_csv_cell(&redact_text(&value)))
            .collect();
        // FIX4 P2 规则版本/脱敏版本插在“需复核”（第 10 列，下标 9）之后：10..10。
        cells.splice(
            10..10,
            [
                EXPOSURE_RULE_VERSION.to_string(),
                super::redaction::REDACTION_VERSION.to_string(),
            ],
        );
        collected.push(cells);
    }
    Ok(collected)
}

pub fn review_needed_count(
    connection: &rusqlite::Connection,
    project_id: i64,
) -> Result<i64, String> {
    connection
        .query_row(
            "SELECT COUNT(DISTINCT v.finding_id) FROM exposure_finding_versions v
             JOIN exposure_findings f ON f.id=v.finding_id
             WHERE f.project_id=?1 AND v.review_needed=1",
            [project_id],
            |row| row.get(0),
        )
        .optional()
        .map(|value| value.unwrap_or(0))
        .map_err(|error| error.to_string())
}

/// Writes the CSV. The header row goes through the same escaping as the data.
///
/// §E4 the file is created new and never followed through a symlink: an existing entry, a
/// directory or a link pointing elsewhere is refused instead of written through, so a planted
/// name cannot turn an export into a write outside the export directory.
pub fn write_csv(path: &std::path::Path, rows: &[Vec<String>]) -> Result<usize, String> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(
                "exposure_export_symlink:导出目标是指向别处的符号链接，已拒绝写入".to_string(),
            );
        }
        if !meta.file_type().is_file() {
            return Err("exposure_export_target_unsafe:导出目标不是普通文件".to_string());
        }
    }
    if path.parent().is_none() {
        return Err("exposure_export_target_unsafe:导出路径没有可校验的父目录".to_string());
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("导出文件创建失败：{error}"))?;
    let mut writer = csv::WriterBuilder::new().from_writer(file);
    writer
        .write_record(
            HEADERS
                .iter()
                .map(|value| escape_csv_cell(redact_text(value).as_str())),
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        writer
            .write_record(row.iter().map(|value| value.as_str()))
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    let landed = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !landed.file_type().is_file() {
        return Err("exposure_export_target_unsafe:写盘后目标不再是普通文件".to_string());
    }
    Ok(rows.len())
}

/// §E4 reads an exported CSV back and proves what landed on disk is what the surface wrote:
/// the file must be a regular file reached without following a link, the header must be the
/// agreed one, every row must have the agreed column count, and the bytes are digested so a
/// caller can compare the summary later. Nothing here re-reads the database.
pub fn verify_written(path: &std::path::Path) -> Result<JsonValue, String> {
    use sha2::{Digest, Sha256};
    let meta = std::fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("exposure_export_symlink:证据包入口是符号链接，不能当作可核验来源".to_string());
    }
    if !meta.file_type().is_file() {
        return Err("exposure_export_target_unsafe:证据包入口不是普通文件".to_string());
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(bytes.as_slice());
    let header: Vec<String> = reader
        .headers()
        .map_err(|error| format!("证据包表头不可读：{error}"))?
        .iter()
        .map(str::to_string)
        .collect();
    // G4-3 three agreed headers (legacy findings, findings v2, incident ledger), matched by
    // exact column names, so a file with the right count but other columns is refused too.
    let Some(format) = known_header(&header) else {
        return Err(format!(
            "exposure_export_shape：表头列数 {} 或列名与约定（{} / {} / {} 列）不一致",
            header.len(),
            HEADERS.len(),
            headers_v2().len(),
            LEDGER_HEADERS.len()
        ));
    };
    let mut rows = 0i64;
    let mut row_kinds: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for record in reader.records() {
        let record = record.map_err(|error| format!("证据包行不可读：{error}"))?;
        if record.len() != header.len() {
            return Err(format!(
                "exposure_export_shape：第 {rows} 行有 {} 列，约定 {} 列",
                record.len(),
                header.len()
            ));
        }
        if format == "ledger_v1" {
            *row_kinds.entry(record.get(0).unwrap_or_default().to_string()).or_insert(0) += 1;
        }
        rows += 1;
    }
    Ok(json!({
        "path": path.to_string_lossy(),
        "bytes": bytes.len(),
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
        "rows": rows,
        "format": format,
        "rowKinds": row_kinds,
        "ruleVersion": EXPOSURE_RULE_VERSION,
        "redactionVersion": super::redaction::REDACTION_VERSION,
    }))
}

// ───────────────────────────── G4-3 ─────────────────────────────

/// G4-3 findings export, v2: the legacy 14 columns unchanged, then the classification,
/// incident and remediation context §15 asks an export to keep.
pub const HEADERS_V2_EXTRA: &[&str] = &[
    "信息类别",
    "来源渠道",
    "处置",
    "处置理由",
    "分类策略版本",
    "事件编号",
    "位置编号",
    "事件状态",
    "事件版本",
    "单位关联",
    "整改状态",
    "采集时间",
    "索引时间",
];

/// G4-3 the incident ledger export: one file, every row tagged with its `rowKind`, so the
/// summary rows reconcile with the incident/location/entity rows of the same file.
pub const LEDGER_HEADERS: &[&str] = &[
    "rowKind",
    "编号",
    "上级编号",
    "名称",
    "状态",
    "信息类别",
    "来源",
    "采集方式",
    "地址",
    "单位关联级别",
    "数量",
    "分母单位",
    "版本",
    "时间",
    "说明",
];

pub fn headers_v2() -> Vec<&'static str> {
    HEADERS.iter().chain(HEADERS_V2_EXTRA.iter()).copied().collect()
}

fn cell(value: &str) -> String {
    escape_csv_cell(&redact_text(value))
}

/// v2 rows: the legacy row, then the extra columns, all redacted and formula-escaped.
pub fn finding_rows_v2(connection: &rusqlite::Connection, project_id: i64) -> Result<Vec<Vec<String>>, String> {
    let legacy = finding_rows(connection, project_id)?;
    let mut statement = connection
        .prepare(
            "SELECT f.id,
                    COALESCE(d.info_category,''), COALESCE(d.source_channel,''), COALESCE(d.disposition,''), COALESCE(d.reason,''), COALESCE(d.policy_version,''),
                    COALESCE(l.incident_id,''), COALESCE(l.id,''), COALESCE(i.status,''), COALESCE(i.revision,''),
                    COALESCE((SELECT GROUP_CONCAT(DISTINCT e.link_level) FROM exposure_incident_entities e WHERE e.incident_id=l.incident_id AND e.active=1),''),
                    COALESCE((SELECT r.status FROM exposure_remediations r WHERE r.finding_id=f.id AND r.project_id=f.project_id ORDER BY r.id DESC LIMIT 1),''),
                    COALESCE((SELECT o.queried_at FROM exposure_finding_observations fo JOIN exposure_observations o ON o.id=fo.observation_id WHERE fo.finding_id=f.id ORDER BY fo.observation_id DESC LIMIT 1),''),
                    COALESCE((SELECT o.indexed_at FROM exposure_finding_observations fo JOIN exposure_observations o ON o.id=fo.observation_id WHERE fo.finding_id=f.id ORDER BY fo.observation_id DESC LIMIT 1),'')
             FROM exposure_findings f
             LEFT JOIN exposure_finding_dispositions d ON d.finding_id=f.id
             LEFT JOIN exposure_incident_locations l ON l.finding_id=f.id AND l.project_id=f.project_id AND l.status='active'
                  AND l.incident_id=(SELECT MIN(x.incident_id) FROM exposure_incident_locations x JOIN exposure_incidents xi ON xi.id=x.incident_id
                                     WHERE x.finding_id=f.id AND x.status='active' AND xi.status<>'merged')
             LEFT JOIN exposure_incidents i ON i.id=l.incident_id
             WHERE f.project_id=?1
             ORDER BY CASE f.severity WHEN 'critical' THEN 4 WHEN 'high' THEN 3 WHEN 'medium' THEN 2 ELSE 1 END DESC,
                      f.confidence DESC, f.id",
        )
        .map_err(|error| error.to_string())?;
    let extras: Vec<Vec<String>> = statement
        .query_map([project_id], |row| {
            let mut cells = Vec::new();
            for index in 1..=13 {
                let value: rusqlite::types::Value = row.get(index)?;
                cells.push(match value {
                    rusqlite::types::Value::Integer(number) => number.to_string(),
                    rusqlite::types::Value::Text(text) => text,
                    rusqlite::types::Value::Real(number) => number.to_string(),
                    _ => String::new(),
                });
            }
            Ok(cells)
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    if extras.len() != legacy.len() {
        return Err(format!("exposure_export_shape：线索行 {} 与扩展行 {} 不一致", legacy.len(), extras.len()));
    }
    Ok(legacy
        .into_iter()
        .zip(extras)
        .map(|(mut row, extra)| {
            row.extend(extra.iter().map(|value| cell(value)));
            row
        })
        .collect())
}

fn ledger_row(values: [&str; 15]) -> Vec<String> {
    values.iter().map(|value| cell(value)).collect()
}

/// G4-3 the ledger rows. Totals in the `summary` rows equal the number of `incident` /
/// `location` / `entity_link` rows in the same file, so a reader can reconcile them.
pub fn ledger_rows(connection: &rusqlite::Connection, project_id: i64, coverage: Option<&JsonValue>) -> Result<Vec<Vec<String>>, String> {
    super::incidents::check_schema(connection)?;
    let mut incidents: Vec<Vec<String>> = Vec::new();
    {
        let mut statement = connection
            .prepare("SELECT id,title,status,info_category,basis,revision,updated_at,handling_state,review_needed FROM exposure_incidents WHERE project_id=?1 AND status<>'merged' ORDER BY id")
            .map_err(|error| error.to_string())?;
        let mut rows = statement.query([project_id]).map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let id: i64 = row.get(0).map_err(|error| error.to_string())?;
            let title: String = row.get(1).map_err(|error| error.to_string())?;
            let status: String = row.get(2).map_err(|error| error.to_string())?;
            let category: String = row.get(3).map_err(|error| error.to_string())?;
            let basis: String = row.get(4).map_err(|error| error.to_string())?;
            let revision: i64 = row.get(5).map_err(|error| error.to_string())?;
            let updated: String = row.get(6).map_err(|error| error.to_string())?;
            let handling: String = row.get(7).map_err(|error| error.to_string())?;
            let review: i64 = row.get(8).map_err(|error| error.to_string())?;
            let note = format!("依据 {basis}；处置登记 {handling}{}", if review != 0 { "；需重新复核" } else { "" });
            incidents.push(ledger_row([
                "incident", &id.to_string(), "", &title, &status, &category, "", "", "", "", "", "", &revision.to_string(), &updated, &note,
            ]));
        }
    }
    let mut locations: Vec<Vec<String>> = Vec::new();
    {
        let mut statement = connection
            .prepare(
                "SELECT l.id,l.incident_id,l.source_key,l.collection_mode,l.canonical_url,l.location_kind,l.relation,COALESCE(o.queried_at,''),COALESCE(o.indexed_at,'')
                 FROM exposure_incident_locations l JOIN exposure_incidents i ON i.id=l.incident_id AND i.status<>'merged'
                 LEFT JOIN exposure_observations o ON o.id=l.observation_id
                 WHERE l.project_id=?1 AND l.status='active' ORDER BY l.incident_id,l.id",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = statement.query([project_id]).map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let get = |index: usize| -> Result<String, String> {
                let value: rusqlite::types::Value = row.get(index).map_err(|error| error.to_string())?;
                Ok(match value {
                    rusqlite::types::Value::Integer(number) => number.to_string(),
                    rusqlite::types::Value::Text(text) => text,
                    _ => String::new(),
                })
            };
            let indexed = get(8)?;
            let time = if indexed.is_empty() { get(7)? } else { format!("{}（索引 {indexed}）", get(7)?) };
            let note = format!("{}；{}", get(5)?, get(6)?);
            locations.push(ledger_row([
                "location", &get(0)?, &get(1)?, "", "active", "", &get(2)?, &get(3)?, &get(4)?, "", "", "", "", &time, &note,
            ]));
        }
    }
    let mut links: Vec<Vec<String>> = Vec::new();
    {
        let names: std::collections::HashMap<i64, String> = crate::commands::load_org_entities(connection)
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|entity| (entity.id, entity.legal_name))
            .collect();
        let mut statement = connection
            .prepare(
                "SELECT e.id,e.incident_id,e.entity_id,e.link_level,e.origin,e.directory_version,e.binding_version,e.created_at
                 FROM exposure_incident_entities e JOIN exposure_incidents i ON i.id=e.incident_id AND i.status<>'merged'
                 WHERE e.project_id=?1 AND e.active=1 ORDER BY e.incident_id,e.id",
            )
            .map_err(|error| error.to_string())?;
        let mut rows = statement.query([project_id]).map_err(|error| error.to_string())?;
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let id: i64 = row.get(0).map_err(|error| error.to_string())?;
            let incident: i64 = row.get(1).map_err(|error| error.to_string())?;
            let entity: i64 = row.get(2).map_err(|error| error.to_string())?;
            let level: String = row.get(3).map_err(|error| error.to_string())?;
            let origin: String = row.get(4).map_err(|error| error.to_string())?;
            let directory: i64 = row.get(5).map_err(|error| error.to_string())?;
            let binding: i64 = row.get(6).map_err(|error| error.to_string())?;
            let created: String = row.get(7).map_err(|error| error.to_string())?;
            let name = names.get(&entity).cloned().unwrap_or_else(|| format!("实体 {entity}（目录中已不存在）"));
            links.push(ledger_row([
                "entity_link", &id.to_string(), &incident.to_string(), &name, "active", "", &origin, "", "", &level, "", "",
                &format!("目录 v{directory} / 绑定 v{binding}"), &created, "",
            ]));
        }
    }
    let mut coverage_rows: Vec<Vec<String>> = Vec::new();
    if let Some(rows) = coverage.and_then(|value| value["rows"].as_array()) {
        for row in rows {
            let denominator = &row["denominator"];
            let text = |value: &JsonValue| value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
            let run = row["window"]["runId"].as_i64().map(|id| format!("第 {id} 轮")).unwrap_or_default();
            coverage_rows.push(ledger_row([
                "coverage",
                &text(&row["capability"]),
                "",
                &text(&row["label"]),
                &text(&row["state"]),
                "",
                &text(&row["sourceKey"]),
                &text(&row["integration"]),
                "",
                "",
                &format!("计划 {} / 已执行 {} / 未覆盖 {}", denominator["planned"], denominator["executed"], denominator["uncovered"]),
                &text(&denominator["unit"]),
                super::coverage::COVERAGE_POLICY_VERSION,
                &run,
                &text(&row["reason"]),
            ]));
        }
    }
    let mut rows = vec![
        ledger_row(["summary", "incidents_total", "", "事件（集团唯一，不含已合并）", "", "", "", "", "", "", &incidents.len().to_string(), "事件", super::incidents::INCIDENT_POLICY_VERSION, "", "等于本文件 rowKind=incident 的行数"]),
        ledger_row(["summary", "locations_total", "", "有效传播位置", "", "", "", "", "", "", &locations.len().to_string(), "位置", super::incidents::INCIDENT_POLICY_VERSION, "", "等于本文件 rowKind=location 的行数"]),
        ledger_row(["summary", "entity_links_total", "", "有效单位关联", "", "", "", "", "", "", &links.len().to_string(), "关联", super::incidents::INCIDENT_POLICY_VERSION, "", "等于本文件 rowKind=entity_link 的行数；单位之和可大于事件唯一数"]),
        ledger_row(["summary", "coverage_rows_total", "", "覆盖矩阵行", "", "", "", "", "", "", &coverage_rows.len().to_string(), "来源能力", super::coverage::COVERAGE_POLICY_VERSION, "", if coverage.is_some() { "等于本文件 rowKind=coverage 的行数" } else { "覆盖矩阵读取失败，未导出（不是 0）" }]),
    ];
    rows.extend(incidents);
    rows.extend(locations);
    rows.extend(links);
    rows.extend(coverage_rows);
    Ok(rows)
}

/// Writes rows under an explicit header, with the same create-new / no-symlink rules as the
/// legacy export.
pub fn write_csv_with(path: &std::path::Path, headers: &[&str], rows: &[Vec<String>]) -> Result<usize, String> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err("exposure_export_symlink:导出目标是指向别处的符号链接，已拒绝写入".to_string());
        }
        return Err("exposure_export_target_exists:导出目标已存在，不覆盖".to_string());
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("导出文件创建失败：{error}"))?;
    let mut writer = csv::WriterBuilder::new().from_writer(file);
    writer
        .write_record(headers.iter().map(|value| escape_csv_cell(redact_text(value).as_str())))
        .map_err(|error| error.to_string())?;
    for row in rows {
        if row.len() != headers.len() {
            return Err(format!("exposure_export_shape：行有 {} 列，表头 {} 列", row.len(), headers.len()));
        }
        writer.write_record(row.iter().map(|value| value.as_str())).map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    Ok(rows.len())
}

/// `<项目名>_暴露面_<本地导出时间+时区>.csv`: the project name with path-hostile characters
/// replaced, a local timestamp that states its offset, and a numeric suffix when the name is
/// already taken (never an overwrite).
pub fn export_file_name(project_name: &str, project_id: i64, suffix: &str, now: chrono::DateTime<chrono::Local>, taken: &dyn Fn(&str) -> bool) -> String {
    let mut name: String = project_name
        .chars()
        .map(|ch| if ch.is_control() || "/\\:*?\"<>|".contains(ch) { '_' } else { ch })
        .collect::<String>()
        .trim()
        .trim_start_matches('.')
        .chars()
        .take(60)
        .collect();
    if name.trim().is_empty() {
        name = format!("项目{project_id}");
    }
    let stamp = now.format("%Y%m%d-%H%M%S%z").to_string();
    let base = format!("{name}_暴露面{suffix}_{stamp}");
    let mut candidate = format!("{base}.csv");
    let mut counter = 2;
    while taken(&candidate) && counter < 1000 {
        candidate = format!("{base}_{counter}.csv");
        counter += 1;
    }
    candidate
}

/// Which agreed header a file carries, for `verify_written`.
pub fn known_header(header: &[String]) -> Option<&'static str> {
    let matches = |expected: &[&str]| header.len() == expected.len() && header.iter().zip(expected).all(|(left, right)| left == right);
    if matches(HEADERS) {
        Some("findings_v1")
    } else if matches(&headers_v2()) {
        Some("findings_v2")
    } else if matches(LEDGER_HEADERS) {
        Some("ledger_v1")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn formula_cells_are_neutralised_before_they_reach_a_spreadsheet() {
        assert_eq!(
            escape_csv_cell("=HYPERLINK(\"http://x\")"),
            "'=HYPERLINK(\"http://x\")"
        );
        assert_eq!(escape_csv_cell("+1234"), "'+1234");
        assert_eq!(escape_csv_cell("-5"), "'-5");
        assert_eq!(escape_csv_cell("@alert"), "'@alert");
        assert_eq!(escape_csv_cell("\t1"), "'\t1");
        assert_eq!(escape_csv_cell("普通标题"), "普通标题");
    }

    #[test]
    fn the_export_writes_every_row_and_keeps_the_provenance_columns() {
        let root =
            std::env::temp_dir().join(format!("oviraptor-exposure-q5-{}", uuid::Uuid::new_v4()));
        let db_path = crate::db::initialize(&root.join("app")).expect("临时库");
        let connection = crate::db::open(&db_path).expect("连接");
        super::super::state::ensure_exposure_schema(&connection).expect("结构");
        connection
            .execute(
                "INSERT INTO projects(id,name,status) VALUES(71,'范围','active')",
                [],
            )
            .unwrap();
        for (index, title) in ["=泄漏", "普通线索"].iter().enumerate() {
            let slot = index as i64;
            connection
                .execute(
                    "INSERT INTO exposure_findings(project_id,category,title,source_type,source_url,evidence_excerpt,severity,confidence,fingerprint)
                     VALUES(71,'source_code',?2,'passive_web',?3,?4,'high',80,?5)",
                    params![slot, title, format!("https://x.invalid/{index}"), "api_key = SUPERSECRETVALUE1", format!("fp-{index}")],
                )
                .unwrap();
        }
        let rows = finding_rows(&connection, 71).expect("导出行");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), HEADERS.len(), "{:?}", rows[0]);
        let joined = rows.iter().flatten().cloned().collect::<Vec<_>>().join("|");
        assert!(
            !joined.contains("SUPERSECRETVALUE1"),
            "导出必须已脱敏：{joined}"
        );
        assert!(joined.contains("'=泄漏"), "公式单元格必须被转义：{joined}");
        assert!(joined.contains(EXPOSURE_RULE_VERSION), "导出要带规则版本");
        let path = root.join("exposure.csv");
        let written = write_csv(&path, &rows).expect("写盘");
        assert_eq!(written, 2);
        let text = std::fs::read_to_string(path).expect("可读");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "表头 + 两行");
        assert!(lines[0].starts_with("类别,标题,风险"), "{}", lines[0]);
        assert_eq!(review_needed_count(&connection, 71).expect("计数"), 0);
        let _ = std::fs::remove_dir_all(root);
    }
}
