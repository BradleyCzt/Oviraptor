//! G3-0 文件导入（`document_import`）与 G3-2 公开情报导入（`public_intel_import`），规格 §10。
//!
//! 一次导入 = 一个导入批次 + 一轮 exposure run（来源只有该导入渠道）。每个文件是一个工作项，
//! 复用生产 `run_plan`：观察 → 线索 → 分类 → 聚合 → 终态，与网络来源同一条持久化路径。
//! - 观察的 `method` 是 `IMPORT`、`collectionMode` 是 `import`、`networkObserved=false`：这是导入方
//!   提供的内容，不是应用从外网观察到的。导入方填写的来源 URL 只是声明（`declaredUrlVerified=false`）。
//! - 只写 exposure 自有表，不创建 assets、org_entities 或 asset_ownership_decisions。
//! - 正文不入库：观察里只有脱敏摘录、字节数和哈希。
//! - PDF/Office/图片/压缩包按魔数识别后拒绝分析（`unsupported_format`），不伪装解析成功。
//! - 同一项目同一 operationId：同载荷重放返回原报告，异载荷拒绝且零写。
//!
//! 入口命令尚未在 lib.rs 注册（IR-14）；注册前页面显示 integration_pending。

use super::adapters::documents::{
    analyze_text, attach_same_document, document_findings, group_terms, magic_type, DocumentFacts, KIND_DOCUMENT_IMPORT, KIND_INTEL_IMPORT, OBS_DOCUMENT_IMPORT, OBS_INTEL_RECORD,
    TITLE_INTEL_CLAIM,
};
use super::observations::{FindingDraft, NewObservation};
use super::orchestrator::{active_run, run_plan, ItemOutcome};
use super::redaction::{content_hash, credential_sightings, evidence_excerpt, redact_text, CredentialGrade};
use super::state::{ensure_exposure_schema, utc_now, SourceStatus, WorkItemStatus};
use super::transaction::ImmediateTx;
use super::work_items::{plan, WorkItemSeed};
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;

pub const IMPORT_VERSION: &str = "oviraptor.exposure.import.v1";
pub const FORMATS: &[&str] = &["text", "json", "csv", "extracted_text"];
pub const INTEL_FORMATS: &[&str] = &["json", "csv"];
pub const IMPORT_MAX_FILES: usize = 20;
pub const IMPORT_FILE_MAX_BYTES: usize = 2 * 1024 * 1024;
pub const IMPORT_BATCH_MAX_BYTES: usize = 8 * 1024 * 1024;
pub const INTEL_MAX_RECORDS: usize = 500;

pub const GRADE_CLAIM_ONLY: &str = "claim_only";
pub const GRADE_SAMPLE_SUPPORTED: &str = "sample_supported";
pub const GRADE_HUMAN_VERIFIED: &str = "human_verified";
pub const GRADES: &[&str] = &[GRADE_CLAIM_ONLY, GRADE_SAMPLE_SUPPORTED, GRADE_HUMAN_VERIFIED];

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportRequest {
    pub operation_id: String,
    pub channel: String,
    pub operator: String,
    pub source_declaration: String,
    #[serde(default)]
    pub declared_url: String,
    #[serde(default)]
    pub batch_label: String,
    #[serde(default)]
    pub public_material: bool,
    #[serde(default)]
    pub public_material_note: String,
    /// For `extracted_text`: which original format the importer extracted the text from.
    #[serde(default)]
    pub extracted_from: String,
    pub files: Vec<ImportFile>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportFile {
    pub name: String,
    pub format: String,
    pub content_base64: String,
}

fn invalid(message: impl std::fmt::Display) -> String {
    format!("[exposure_import_invalid] {message}")
}

fn storage(error: impl std::fmt::Display) -> String {
    format!("[exposure_import_storage] {error}")
}

pub fn operation_id_valid(value: &str) -> bool {
    (8..=80).contains(&value.len())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A display name, never a path: separators and control characters are refused.
fn file_name_valid(name: &str) -> bool {
    let count = name.chars().count();
    (1..=200).contains(&count)
        && !name.contains(['/', '\\'])
        && name != "."
        && name != ".."
        && !name.chars().any(char::is_control)
}

fn url_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' })
        .collect()
}

/// RFC 4180-style CSV: quoted fields may hold commas, quotes (`""`) and newlines.
pub fn parse_csv(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    let mut line = 1usize;
    while let Some(c) = chars.next() {
        if quoted {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                '"' => quoted = false,
                '\n' => {
                    line += 1;
                    field.push(c);
                }
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                line += 1;
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if quoted {
        return Err(format!("第 {line} 行的引号没有闭合"));
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn json_text(value: &JsonValue, out: &mut Vec<String>, key: Option<&str>) {
    match value {
        JsonValue::Object(map) => {
            for (name, item) in map {
                json_text(item, out, Some(name));
            }
        }
        JsonValue::Array(rows) => {
            for item in rows {
                json_text(item, out, key);
            }
        }
        JsonValue::Null => {}
        scalar => {
            let text = match scalar {
                JsonValue::String(text) => text.clone(),
                other => other.to_string(),
            };
            out.push(match key {
                Some(name) => format!("{name}: {text}"),
                None => text,
            });
        }
    }
}

/// One intel record as supplied. Every text is a claim of the report, not a fact.
#[derive(Clone, Debug, Default)]
pub struct IntelRecord {
    pub title: String,
    pub claim: String,
    pub sample: String,
    pub source: String,
    pub source_url: String,
    pub published_at: String,
    pub declared_grade: String,
    pub entities: Vec<String>,
}

fn intel_from_object(map: &serde_json::Map<String, JsonValue>) -> IntelRecord {
    let text = |key: &str| {
        map.get(key)
            .map(|value| match value {
                JsonValue::String(text) => text.trim().to_string(),
                JsonValue::Null => String::new(),
                other => other.to_string(),
            })
            .unwrap_or_default()
    };
    IntelRecord {
        title: text("title"),
        claim: text("claim"),
        sample: text("sample"),
        source: text("source"),
        source_url: text("sourceUrl"),
        published_at: text("publishedAt"),
        declared_grade: text("grade"),
        entities: map
            .get("entities")
            .and_then(JsonValue::as_array)
            .map(|rows| rows.iter().filter_map(JsonValue::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
    }
}

pub fn parse_intel(format: &str, text: &str) -> Result<Vec<IntelRecord>, String> {
    let records = match format {
        "json" => {
            let value: JsonValue = serde_json::from_str(text).map_err(|error| format!("JSON 无效：{error}"))?;
            let rows = match &value {
                JsonValue::Array(rows) => rows.clone(),
                JsonValue::Object(map) => map
                    .get("records")
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .ok_or_else(|| "JSON 必须是记录数组或含 records 数组的对象".to_string())?,
                _ => return Err("JSON 必须是记录数组或含 records 数组的对象".to_string()),
            };
            rows.iter()
                .enumerate()
                .map(|(index, row)| {
                    row.as_object()
                        .map(intel_from_object)
                        .ok_or_else(|| format!("第 {} 条记录不是对象", index + 1))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        "csv" => {
            let rows = parse_csv(text)?;
            let Some((header, body)) = rows.split_first() else {
                return Err("CSV 为空".to_string());
            };
            let header: Vec<String> = header.iter().map(|cell| cell.trim().to_string()).collect();
            if !header.iter().any(|cell| cell == "claim") {
                return Err("CSV 表头必须含 claim 列".to_string());
            }
            body.iter()
                .filter(|row| row.iter().any(|cell| !cell.trim().is_empty()))
                .map(|row| {
                    let mut map = serde_json::Map::new();
                    for (index, name) in header.iter().enumerate() {
                        let cell = row.get(index).cloned().unwrap_or_default();
                        if name == "entities" {
                            map.insert(
                                name.clone(),
                                json!(cell.split(['|', ';']).map(str::trim).filter(|v| !v.is_empty()).collect::<Vec<_>>()),
                            );
                        } else {
                            map.insert(name.clone(), json!(cell));
                        }
                    }
                    intel_from_object(&map)
                })
                .collect()
        }
        other => return Err(format!("情报只接受 json/csv，收到 {other}")),
    };
    if records.len() > INTEL_MAX_RECORDS {
        return Err(format!("单个文件最多 {INTEL_MAX_RECORDS} 条记录，收到 {}", records.len()));
    }
    if let Some(index) = records.iter().position(|record| record.claim.trim().is_empty()) {
        return Err(format!("第 {} 条记录缺少 claim", index + 1));
    }
    Ok(records)
}

/// The grade the application itself can justify from the supplied record. A sample only
/// supports the claim when it carries a group identifier or a credential-shaped value;
/// `human_verified` is never accepted from a file.
pub fn intel_grade(record: &IntelRecord, terms: &[String]) -> (&'static str, bool, String) {
    let sample = record.sample.trim();
    let sample_mentions = !sample.is_empty() && !analyze_text(sample, terms).mentions.is_empty();
    let sample_credential = credential_sightings(sample, 2_000)
        .iter()
        .any(|row| row.grade == CredentialGrade::Candidate);
    let supported = sample_mentions || sample_credential;
    let grade = if supported { GRADE_SAMPLE_SUPPORTED } else { GRADE_CLAIM_ONLY };
    let declared = record.declared_grade.trim();
    let note = match declared {
        "" => String::new(),
        GRADE_HUMAN_VERIFIED => format!("导入方标注 {declared}，未经本系统人工复核，按 {grade} 处理"),
        GRADE_SAMPLE_SUPPORTED if !supported => {
            "导入方标注 sample_supported，但样本里没有集团标识或凭据形态值，按 claim_only 处理".to_string()
        }
        value if value == grade => String::new(),
        other => format!("导入方标注 {other}，按本系统判定的 {grade} 处理"),
    };
    (grade, supported, note)
}

/// A decoded file ready for the executor; the bytes stay in memory only.
#[derive(Clone, Debug)]
struct PreparedFile {
    index: usize,
    name: String,
    format: String,
    sha256: String,
    bytes: usize,
    work_key: String,
    url: String,
    content: Result<Prepared, (WorkItemStatus, SourceStatus, String)>,
}

#[derive(Clone, Debug)]
enum Prepared {
    Text(String),
    Intel(Vec<IntelRecord>),
}

fn prepare(request: &ImportRequest, index: usize, file: &ImportFile, bytes: &[u8]) -> PreparedFile {
    let sha256 = content_hash(bytes);
    let short = &sha256[..12];
    let url = format!("import://{}/{short}/{}", request.operation_id, url_name(&file.name));
    let work_key = format!("{}|{index:02}|{short}", request.channel);
    let unsupported = |label: String| {
        Err((
            WorkItemStatus::Completed,
            SourceStatus::Unsupported,
            format!("unsupported_format：{} 是 {label}，没有解析器，正文未分析；请导入已提取的文本（extracted_text）", file.name),
        ))
    };
    // Only the leading bytes decide "this is a PDF/Office/image/archive"; anything else must be
    // valid UTF-8 text and is refused as `invalid_utf8` otherwise.
    let content = match magic_type(bytes) {
        Some(label) => unsupported(label.to_string()),
        None => match super::adapters::documents::utf8_text(bytes, false) {
            Err(reason) => Err((
                WorkItemStatus::Failed,
                SourceStatus::ParseError,
                format!("invalid_utf8：{} {reason}，只接受 UTF-8", file.name),
            )),
            Ok(text) if request.channel == KIND_INTEL_IMPORT => parse_intel(&file.format, &text)
                .map(Prepared::Intel)
                .map_err(|reason| (WorkItemStatus::Failed, SourceStatus::ParseError, format!("parse_error：{} {reason}", file.name))),
            Ok(text) => match file.format.as_str() {
                "json" => serde_json::from_str::<JsonValue>(&text)
                    .map(|value| {
                        let mut lines = Vec::new();
                        json_text(&value, &mut lines, None);
                        Prepared::Text(lines.join("\n"))
                    })
                    .map_err(|error| (WorkItemStatus::Failed, SourceStatus::ParseError, format!("parse_error：{} JSON 无效：{error}", file.name))),
                "csv" => parse_csv(&text)
                    .map(|rows| Prepared::Text(rows.iter().map(|row| row.join("\t")).collect::<Vec<_>>().join("\n")))
                    .map_err(|reason| (WorkItemStatus::Failed, SourceStatus::ParseError, format!("parse_error：{} CSV {reason}", file.name))),
                _ => Ok(Prepared::Text(text)),
            },
        },
    };
    PreparedFile {
        index,
        name: file.name.clone(),
        format: file.format.clone(),
        sha256,
        bytes: bytes.len(),
        work_key,
        url,
        content,
    }
}

struct Validated {
    request: ImportRequest,
    files: Vec<PreparedFile>,
    payload_hash: String,
    declared_url: String,
}

fn validate(raw: &JsonValue) -> Result<Validated, String> {
    let mut request: ImportRequest = serde_json::from_value(raw.clone()).map_err(invalid)?;
    // FIX4 P1／R8 操作者也是人工输入：原文只以摘要参与幂等哈希，落库与返回只用脱敏形态。
    let raw_operator = request.operator.trim().to_string();
    request.operator = redact_text(&raw_operator);
    let raw_texts = [
        request.source_declaration.trim().to_string(),
        request.batch_label.trim().to_string(),
        request.public_material_note.trim().to_string(),
    ];
    // FIX4 P1／R8 文件名也是人工输入：贴进文件名的秘密只以脱敏形态进入观察、工作项、报告与导出；
    // 原文件名只以不可逆摘要参与幂等哈希。
    let raw_names = request.files.iter().map(|file| file.name.clone()).collect::<Vec<_>>().join("\u{1f}");
    for file in request.files.iter_mut() {
        file.name = redact_text(&file.name);
    }
    // P1-5 来源声明、批次名、公开材料说明是人工输入，贴进来的秘密只以脱敏形态落库。
    request.source_declaration = redact_text(request.source_declaration.trim());
    request.batch_label = redact_text(request.batch_label.trim());
    request.public_material_note = redact_text(request.public_material_note.trim());
    request.extracted_from = request.extracted_from.trim().to_string();
    if !operation_id_valid(&request.operation_id) {
        return Err(invalid("operationId 必须是 8~80 位字母、数字、- 或 _"));
    }
    if request.channel != KIND_DOCUMENT_IMPORT && request.channel != KIND_INTEL_IMPORT {
        return Err(invalid(format!("channel 只能是 {KIND_DOCUMENT_IMPORT} 或 {KIND_INTEL_IMPORT}")));
    }
    if !(1..=80).contains(&request.operator.chars().count()) {
        return Err(invalid("operator 必填，最长 80 字"));
    }
    if !(1..=500).contains(&request.source_declaration.chars().count()) {
        return Err(invalid("sourceDeclaration（来源声明）必填，最长 500 字"));
    }
    if request.batch_label.chars().count() > 120 || request.extracted_from.chars().count() > 40 {
        return Err(invalid("batchLabel 最长 120 字，extractedFrom 最长 40 字"));
    }
    if request.public_material {
        if request.channel == KIND_INTEL_IMPORT {
            return Err(invalid("情报导入不能声明为公开材料"));
        }
        if !(1..=200).contains(&request.public_material_note.chars().count()) {
            return Err(invalid("声明为公开材料时必须写明理由（最长 200 字）"));
        }
    }
    // FIX4 P1／R8 声明 URL 里的 token/口令参数打码后再保存和返回；原文只以摘要参与幂等哈希。
    let raw_declared_url = request.declared_url.trim().to_string();
    let declared_url = if raw_declared_url.is_empty() {
        String::new()
    } else {
        super::redaction::redact_url(
            &super::adapters::documents::canonical_document_url(&raw_declared_url)
                .map_err(|reason| invalid(format!("declaredUrl 无效：{reason}")))?,
        )
    };
    request.declared_url = declared_url.clone();
    if request.files.is_empty() || request.files.len() > IMPORT_MAX_FILES {
        return Err(invalid(format!("一次导入 1~{IMPORT_MAX_FILES} 个文件，收到 {}", request.files.len())));
    }
    let mut total = 0usize;
    let mut decoded = Vec::new();
    for (index, file) in request.files.iter().enumerate() {
        if !file_name_valid(&file.name) {
            return Err(invalid(format!("第 {} 个文件名无效（不能含路径分隔符或控制字符，最长 200 字）", index + 1)));
        }
        let formats = if request.channel == KIND_INTEL_IMPORT { INTEL_FORMATS } else { FORMATS };
        if !formats.contains(&file.format.as_str()) {
            return Err(invalid(format!("{} 的格式 {} 不在 {} 内", file.name, file.format, formats.join("/"))));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(file.content_base64.trim())
            .map_err(|error| invalid(format!("{} 的内容不是有效 base64：{error}", file.name)))?;
        if bytes.len() > IMPORT_FILE_MAX_BYTES {
            return Err(invalid(format!("{} 超过单文件上限 {IMPORT_FILE_MAX_BYTES} 字节", file.name)));
        }
        total += bytes.len();
        if total > IMPORT_BATCH_MAX_BYTES {
            return Err(invalid(format!("本批超过总上限 {IMPORT_BATCH_MAX_BYTES} 字节")));
        }
        decoded.push(bytes);
    }
    let files: Vec<PreparedFile> = request
        .files
        .iter()
        .zip(decoded.iter())
        .enumerate()
        .map(|(index, (file, bytes))| prepare(&request, index, file, bytes))
        .collect();
    let identity = json!({
        "operationId": request.operation_id,
        "channel": request.channel,
        "operator": request.operator,
        "sourceDeclaration": request.source_declaration,
        "declaredUrl": declared_url,
        "batchLabel": request.batch_label,
        "publicMaterial": request.public_material,
        "publicMaterialNote": request.public_material_note,
        "extractedFrom": request.extracted_from,
        "files": files.iter().map(|file| json!([file.name, file.format, file.sha256])).collect::<Vec<_>>(),
    });
    Ok(Validated {
        payload_hash: super::redaction::idempotency_hash(
            &identity.to_string(),
            &[&raw_texts[0], &raw_texts[1], &raw_texts[2], &raw_names, &raw_operator, &raw_declared_url],
        ),
        request,
        files,
        declared_url,
    })
}

fn file_rows(files: &[PreparedFile], results: &BTreeMap<String, JsonValue>) -> Vec<JsonValue> {
    files
        .iter()
        .map(|file| {
            let mut row = json!({
                "index": file.index,
                "name": file.name,
                "format": file.format,
                "sha256": file.sha256,
                "bytes": file.bytes,
                "url": file.url,
            });
            if let Some(result) = results.get(&file.work_key) {
                row["result"] = result.clone();
            }
            row
        })
        .collect()
}

/// Imports one batch. Validation, idempotency and the "another round is active" check all
/// happen before the first write; a refusal writes nothing.
pub fn import_batch(connection: &Connection, project_id: i64, raw: &JsonValue) -> Result<JsonValue, String> {
    ensure_exposure_schema(connection)?;
    let validated = validate(raw)?;
    let active: Option<String> = connection
        .query_row("SELECT status FROM projects WHERE id=?1", [project_id], |row| row.get(0))
        .optional()
        .map_err(storage)?;
    match active.as_deref() {
        None => return Err(format!("[exposure_import_project] 项目 {project_id} 不存在")),
        Some(status) if status != "active" => {
            return Err(format!("[exposure_import_project] 项目 {project_id} 状态为 {status}，不接受导入"))
        }
        _ => {}
    }
    let request = &validated.request;
    let existing: Option<(String, String, String)> = connection
        .query_row(
            "SELECT payload_hash,status,report_json FROM exposure_import_batches WHERE project_id=?1 AND operation_id=?2",
            params![project_id, request.operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(storage)?;
    if let Some((hash, status, report)) = existing {
        if hash != validated.payload_hash {
            return Err(format!(
                "[exposure_import_operation_conflict] operationId {} 已用于另一份载荷，拒绝覆盖",
                request.operation_id
            ));
        }
        if status != "completed" {
            return Err(format!(
                "[exposure_import_interrupted] operationId {} 的批次状态为 {status}，正文不在库中无法续导，请换新的 operationId 重新导入",
                request.operation_id
            ));
        }
        let mut value: JsonValue = serde_json::from_str(&report).map_err(storage)?;
        value["replayed"] = json!(true);
        return Ok(value);
    }
    if let Some(run_id) = active_run(connection, project_id)? {
        return Err(format!("[exposure_import_busy] 第 {run_id} 轮采集进行中，导入稍后再试"));
    }
    let settings = super::settings::load(connection, project_id)?;
    let frozen = settings.to_settings_json();
    let now = utc_now();
    let seeds: Vec<WorkItemSeed> = validated
        .files
        .iter()
        .map(|file| WorkItemSeed {
            work_key: file.work_key.clone(),
            kind: request.channel.clone(),
            source_key: request.channel.clone(),
            asset_id: None,
            origin: String::new(),
            entity: file.name.clone(),
            input_hash: file.sha256.clone(),
        })
        .collect();
    let transaction = ImmediateTx::begin(connection)?;
    let inserted = transaction.connection().execute(
        "INSERT INTO exposure_runs(project_id,status,eligible_assets,stage,created_at) VALUES(?1,'queued',?2,'planned',?3)",
        params![project_id, seeds.len() as i64, now],
    );
    if let Err(error) = inserted {
        let text = error.to_string();
        if text.contains("UNIQUE") || text.contains("ux_exposure_runs_single_active") {
            return Err("[exposure_import_busy] 另一轮采集刚刚开始，导入稍后再试".to_string());
        }
        return Err(storage(text));
    }
    let run_id = transaction.insert_row_id();
    transaction
        .connection()
        .execute(
            "INSERT INTO exposure_run_plans(run_id,project_id,attempt_number,scope_version,sources_json,budget_requests,budget_bytes,planned_items,created_at,settings_version,settings_json)
             VALUES(?1,?2,1,1,?3,0,0,?4,?5,?6,?7)",
            params![
                run_id,
                project_id,
                json!([request.channel]).to_string(),
                seeds.len() as i64,
                now,
                settings.version,
                frozen.to_string()
            ],
        )
        .map_err(storage)?;
    plan(transaction.connection(), run_id, project_id, 1, &seeds)?;
    transaction
        .connection()
        .execute(
            "INSERT INTO exposure_import_batches(project_id,run_id,operation_id,payload_hash,channel,operator,source_declaration,declared_url,batch_label,import_version,status,files_json,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'running',?11,?12)",
            params![
                project_id,
                run_id,
                request.operation_id,
                validated.payload_hash,
                request.channel,
                request.operator,
                request.source_declaration,
                validated.declared_url,
                request.batch_label,
                IMPORT_VERSION,
                json!(file_rows(&validated.files, &BTreeMap::new())).to_string(),
                now
            ],
        )
        .map_err(storage)?;
    let batch_id = transaction.insert_row_id();
    transaction.commit()?;

    let terms = group_terms(connection, project_id)?;
    let mut results: BTreeMap<String, JsonValue> = BTreeMap::new();
    let report = {
        let mut executor = |item: &super::work_items::WorkItem| -> Result<ItemOutcome, String> {
            let Some(file) = validated.files.iter().find(|file| file.work_key == item.work_key) else {
                return Ok(ItemOutcome {
                    status: Some(WorkItemStatus::Failed),
                    reason: "导入工作项找不到对应文件".to_string(),
                    source_status: Some(SourceStatus::Failed),
                    source_reason: "导入工作项找不到对应文件".to_string(),
                    ..Default::default()
                });
            };
            let mut outcome = file_outcome(&validated, batch_id, file, &terms, item);
            attach_same_document(connection, project_id, &mut outcome.observations)?;
            results.insert(
                file.work_key.clone(),
                json!({
                    "status": outcome.status.map(WorkItemStatus::as_str).unwrap_or("completed"),
                    "sourceStatus": outcome.source_status.map(SourceStatus::as_str).unwrap_or(""),
                    "reason": outcome.reason,
                    "observations": outcome.observations.len(),
                    "findings": outcome.findings.len(),
                }),
            );
            Ok(outcome)
        };
        run_plan(connection, run_id, &format!("exposure-import-{}", request.operation_id), &mut executor)
    };
    let (run_status, failure) = match &report {
        Ok(value) => (value.aggregation.status.as_str().to_string(), String::new()),
        Err(error) => ("failed".to_string(), error.clone()),
    };
    let value = json!({
        "importVersion": IMPORT_VERSION,
        "batchId": batch_id,
        "runId": run_id,
        "projectId": project_id,
        "operationId": request.operation_id,
        "channel": request.channel,
        "operator": request.operator,
        "sourceDeclaration": request.source_declaration,
        "declaredUrl": validated.declared_url,
        "declaredUrlVerified": false,
        "runStatus": run_status,
        "error": failure,
        "files": file_rows(&validated.files, &results),
        "replayed": false,
    });
    let batch_status = if failure.is_empty() { "completed" } else { "failed" };
    connection
        .execute(
            "UPDATE exposure_import_batches SET status=?2,files_json=?3,report_json=?4,completed_at=?5 WHERE id=?1",
            params![batch_id, batch_status, json!(value["files"]).to_string(), value.to_string(), utc_now()],
        )
        .map_err(storage)?;
    if !failure.is_empty() {
        let _ = super::orchestrator::finalize(connection, run_id, Some(&failure));
        return Err(failure);
    }
    Ok(value)
}

fn base_extra(validated: &Validated, batch_id: i64, file: &PreparedFile) -> JsonValue {
    let request = &validated.request;
    json!({
        "importVersion": IMPORT_VERSION,
        "collectionMode": if file.format == "extracted_text" { "import_extracted_text" } else { "import" },
        "networkObserved": false,
        "batchId": batch_id,
        "operationId": request.operation_id,
        "operator": request.operator,
        "sourceDeclaration": request.source_declaration,
        "declaredUrl": validated.declared_url,
        "declaredUrlVerified": false,
        "batchLabel": request.batch_label,
        "fileName": file.name,
        "format": file.format,
        "fileSha256": file.sha256,
        "bytes": file.bytes,
        "extractedFrom": if file.format == "extracted_text" { request.extracted_from.clone() } else { String::new() },
    })
}

fn file_outcome(
    validated: &Validated,
    batch_id: i64,
    file: &PreparedFile,
    terms: &[String],
    item: &super::work_items::WorkItem,
) -> ItemOutcome {
    let request = &validated.request;
    let mut extra = base_extra(validated, batch_id, file);
    let observation = |kind: &str, url: &str, body: Vec<u8>, extra: JsonValue| NewObservation {
        status_code: 0,
        content_type: format!("text/plain; format={}", file.format),
        body_bytes: file.bytes as i64,
        body,
        method: "IMPORT".to_string(),
        extra,
        ..NewObservation::new(&item.source_key, kind, url)
    };
    let prepared = match &file.content {
        Err((item_status, source, reason)) => {
            extra["contentAnalyzed"] = json!(false);
            return ItemOutcome {
                status: Some(*item_status),
                reason: reason.clone(),
                source_status: Some(*source),
                source_reason: reason.clone(),
                observations: vec![observation(OBS_DOCUMENT_IMPORT, &file.url, Vec::new(), extra)],
                ..Default::default()
            };
        }
        Ok(prepared) => prepared,
    };
    match prepared {
        Prepared::Text(text) => {
            let analysis = analyze_text(text, terms);
            extra["contentAnalyzed"] = json!(true);
            extra["mentions"] = json!(analysis.mentions);
            extra["excerpt"] = json!(analysis.excerpt);
            extra["normalizedChars"] = json!(analysis.normalized_chars);
            extra["normalizedTextSha256"] = json!(analysis.normalized_sha256);
            extra["declaredPublic"] = json!(request.public_material);
            extra["declaredPublicNote"] = json!(request.public_material_note);
            let mode_note = format!(
                "导入正文（来源声明：{}；{}），不是应用从外网观察到的",
                evidence_excerpt(&request.source_declaration, 120),
                if file.format == "extracted_text" { "正文由导入方提取" } else { "按原格式解析" }
            );
            let facts = DocumentFacts {
                source_key: &item.source_key,
                url: &file.url,
                mode_note: &mode_note,
                declared_public: request.public_material.then_some(request.public_material_note.as_str()),
            };
            let findings = document_findings(&facts, &analysis);
            let (source, reason) = if findings.is_empty() {
                (SourceStatus::CompletedNoMatches, format!("{} 已分析，未出现集团名称或凭据形态值；不代表没有泄漏", file.name))
            } else {
                (SourceStatus::Completed, format!("{} 已分析，生成 {} 条线索", file.name, findings.len()))
            };
            ItemOutcome {
                status: Some(WorkItemStatus::Completed),
                reason: reason.clone(),
                source_status: Some(source),
                source_reason: reason,
                observations: vec![observation(OBS_DOCUMENT_IMPORT, &file.url, text.as_bytes().to_vec(), extra)],
                findings,
                observed: 1,
                ..Default::default()
            }
        }
        Prepared::Intel(records) => {
            let mut outcome = ItemOutcome::default();
            let mut graded: BTreeMap<&str, usize> = BTreeMap::new();
            let mut unrelated = 0usize;
            for (index, record) in records.iter().enumerate() {
                let url = format!("{}/r{}", file.url, index + 1);
                let joined = format!("{}\n{}\n{}\n{}", record.title, record.claim, record.entities.join("\n"), record.sample);
                let mentions = analyze_text(&joined, terms).mentions;
                let (grade, supported, note) = intel_grade(record, terms);
                let mut row = extra.clone();
                row["recordIndex"] = json!(index + 1);
                row["evidenceGrade"] = json!(grade);
                row["declaredGrade"] = json!(record.declared_grade);
                row["gradeNote"] = json!(note);
                row["sampleSupported"] = json!(supported);
                row["sampleExcerpt"] = json!(evidence_excerpt(&record.sample, 300));
                row["claimExcerpt"] = json!(evidence_excerpt(&record.claim, 300));
                row["title"] = json!(evidence_excerpt(&record.title, 200));
                row["mentions"] = json!(mentions);
                row["reportedSource"] = json!(evidence_excerpt(&record.source, 120));
                row["reportedSourceUrl"] = json!(evidence_excerpt(&record.source_url, 300));
                row["reportedSourceVerified"] = json!(false);
                row["publishedAt"] = json!(evidence_excerpt(&record.published_at, 40));
                row["contentAnalyzed"] = json!(true);
                outcome.observations.push(observation(OBS_INTEL_RECORD, &url, joined.clone().into_bytes(), row));
                outcome.observed += 1;
                if mentions.is_empty() {
                    unrelated += 1;
                    continue;
                }
                *graded.entry(grade).or_default() += 1;
                outcome.findings.push(FindingDraft {
                    category: "intel_claim".to_string(),
                    title: TITLE_INTEL_CLAIM.to_string(),
                    url: url.clone(),
                    source_type: item.source_key.clone(),
                    evidence: format!(
                        "等级 {grade}：{}；样本：{}；出处（声明，未验证）：{}{}",
                        evidence_excerpt(&record.claim, 200),
                        if supported { evidence_excerpt(&record.sample, 160) } else if record.sample.trim().is_empty() { "无".to_string() } else { "有，但未见集团标识或凭据形态值".to_string() },
                        evidence_excerpt(&record.source, 80),
                        if note.is_empty() { String::new() } else { format!("；{note}") }
                    ),
                    severity: "medium".to_string(),
                    confidence: if supported { 40 } else { 20 },
                    asset_id: None,
                    content_hash: content_hash(format!("{}\n{}", record.claim, record.sample).as_bytes()),
                });
            }
            let summary = format!(
                "{}：{} 条记录，{} 条涉及集团（{}），{unrelated} 条未出现集团名称",
                file.name,
                records.len(),
                outcome.findings.len(),
                graded.iter().map(|(grade, count)| format!("{grade} {count}")).collect::<Vec<_>>().join("、")
            );
            outcome.status = Some(WorkItemStatus::Completed);
            outcome.source_status = Some(if outcome.findings.is_empty() {
                SourceStatus::CompletedNoMatches
            } else {
                SourceStatus::Completed
            });
            outcome.reason = summary.clone();
            outcome.source_reason = summary;
            outcome
        }
    }
}

// ───────────────────────────── G3-2 manual grade ─────────────────────────────

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GradeRequest {
    pub operation_id: String,
    pub finding_id: i64,
    pub expected_grade: String,
    pub grade: String,
    pub actor: String,
    pub reason: String,
    #[serde(default)]
    pub sample_observation_id: Option<i64>,
    #[serde(default)]
    pub entity_id: Option<i64>,
}

/// The import-time grade (from the record's observation) unless a person changed it since.
pub fn current_grade(connection: &Connection, project_id: i64, finding_id: i64) -> Result<Option<String>, String> {
    let manual: Option<String> = connection
        .query_row(
            "SELECT grade FROM exposure_intel_grades WHERE project_id=?1 AND finding_id=?2 ORDER BY id DESC LIMIT 1",
            params![project_id, finding_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage)?;
    if manual.is_some() {
        return Ok(manual);
    }
    connection
        .query_row(
            "SELECT json_extract(o.evidence_json,'$.detail.evidenceGrade') FROM exposure_finding_observations fo
             JOIN exposure_observations o ON o.id=fo.observation_id
             WHERE fo.finding_id=?1 AND fo.project_id=?2 AND o.project_id=?2 AND o.source_key=?3 AND o.kind=?4
             ORDER BY o.id LIMIT 1",
            params![finding_id, project_id, KIND_INTEL_IMPORT, OBS_INTEL_RECORD],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(storage)
}

/// Changes an intel finding's grade. `human_verified` needs a reason plus a verifiable anchor:
/// a sample observation of this very finding that supports the claim, or an existing entity
/// of the organisation directory. CAS on the expected grade; operationId replays.
pub fn grade_intel_finding(connection: &Connection, project_id: i64, raw: &JsonValue) -> Result<JsonValue, String> {
    ensure_exposure_schema(connection)?;
    let mut request: GradeRequest = serde_json::from_value(raw.clone())
        .map_err(|error| format!("[exposure_intel_grade_invalid] {error}"))?;
    request.actor = request.actor.trim().to_string();
    let raw_reason = request.reason.trim().to_string();
    request.reason = redact_text(&raw_reason);
    if !operation_id_valid(&request.operation_id) {
        return Err("[exposure_intel_grade_invalid] operationId 必须是 8~80 位字母、数字、- 或 _".to_string());
    }
    if !GRADES.contains(&request.grade.as_str()) {
        return Err(format!("[exposure_intel_grade_invalid] 等级只能是 {}", GRADES.join("/")));
    }
    if !(1..=80).contains(&request.actor.chars().count()) {
        return Err("[exposure_intel_grade_invalid] actor 必填".to_string());
    }
    if request.reason.chars().count() < 4 || request.reason.chars().count() > 500 {
        return Err("[exposure_intel_grade_invalid] 改级必须写明理由（4~500 字）".to_string());
    }
    let payload_hash = super::redaction::idempotency_hash(
        &json!([request.finding_id, request.expected_grade, request.grade, request.actor, request.reason, request.sample_observation_id, request.entity_id])
            .to_string(),
        &[&raw_reason],
    );
    let transaction = ImmediateTx::begin(connection)?;
    let tx = transaction.connection();
    let replay: Option<(String, i64, String, String)> = tx
        .query_row(
            "SELECT payload_hash,id,grade,previous_grade FROM exposure_intel_grades WHERE project_id=?1 AND operation_id=?2",
            params![project_id, request.operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(storage)?;
    if let Some((hash, event_id, grade, previous)) = replay {
        if hash != payload_hash {
            return Err(format!("[exposure_intel_grade_operation_conflict] operationId {} 已用于另一份改级", request.operation_id));
        }
        return Ok(json!({"findingId": request.finding_id, "eventId": event_id, "grade": grade, "previousGrade": previous, "replayed": true}));
    }
    let Some(current) = current_grade(tx, project_id, request.finding_id)? else {
        return Err(format!("[exposure_intel_not_found] 项目 {project_id} 没有情报线索 {}", request.finding_id));
    };
    if current != request.expected_grade {
        return Err(format!(
            "[exposure_intel_grade_conflict] 当前等级是 {current}，不是 {}；请刷新后再改",
            request.expected_grade
        ));
    }
    let sample_anchor = match request.sample_observation_id {
        None => false,
        Some(observation_id) => {
            let supported: Option<i64> = tx
                .query_row(
                    "SELECT COALESCE(json_extract(o.evidence_json,'$.detail.sampleSupported'),0) FROM exposure_finding_observations fo
                     JOIN exposure_observations o ON o.id=fo.observation_id
                     WHERE fo.finding_id=?1 AND fo.observation_id=?2 AND fo.project_id=?3 AND o.project_id=?3",
                    params![request.finding_id, observation_id, project_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(storage)?;
            match supported {
                None => {
                    return Err(format!(
                        "[exposure_intel_anchor_invalid] 观察 {observation_id} 不属于本项目的这条线索"
                    ))
                }
                Some(value) => value != 0,
            }
        }
    };
    let entity_anchor = match request.entity_id {
        None => false,
        Some(entity_id) => {
            let entities = crate::commands::ownership::load_org_entities(tx).map_err(storage)?;
            if !entities.iter().any(|entity| entity.id == entity_id) {
                return Err(format!("[exposure_intel_anchor_invalid] 组织目录里没有实体 {entity_id}"));
            }
            true
        }
    };
    match request.grade.as_str() {
        GRADE_HUMAN_VERIFIED if !sample_anchor && !entity_anchor => {
            return Err(
                "[exposure_intel_anchor_required] human_verified 需要可核验依据：本线索里支持声称的样本观察，或组织目录中的单位".to_string(),
            )
        }
        GRADE_SAMPLE_SUPPORTED if !sample_anchor => {
            return Err("[exposure_intel_anchor_required] sample_supported 需要本线索里支持声称的样本观察".to_string())
        }
        _ => {}
    }
    tx.execute(
        "INSERT INTO exposure_intel_grades(project_id,finding_id,operation_id,payload_hash,previous_grade,grade,actor,reason,sample_observation_id,entity_id,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            project_id,
            request.finding_id,
            request.operation_id,
            payload_hash,
            current,
            request.grade,
            request.actor,
            request.reason,
            request.sample_observation_id,
            request.entity_id,
            utc_now()
        ],
    )
    .map_err(storage)?;
    let event_id = transaction.insert_row_id();
    transaction.commit()?;
    Ok(json!({"findingId": request.finding_id, "eventId": event_id, "grade": request.grade, "previousGrade": current, "replayed": false}))
}
