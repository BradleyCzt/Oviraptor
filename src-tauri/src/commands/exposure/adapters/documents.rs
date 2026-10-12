//! G3 — 文档入口（规格 §10）。两条能力共用同一套文本分析：
//! - `document_link`（`explicit_document_url`）：用户在 exposure 自有设置里逐条指定的公开文档 URL。
//!   只读这些精确地址，GET 一次，不跟随跳转，私网/非默认端口在发送前拒绝，单响应字节取本轮冻结上限。
//!   HTML/纯文本才分析正文；PDF、Office、压缩包、图片只记元数据和能力缺口，不假装已解析。
//! - `document_import` / `public_intel_import` 的正文分析也走这里（见 `imports.rs`），但它们的观察
//!   标 `collectionMode=import`：是导入方提供的内容，不是应用从外网观察到的。
//!
//! 命中词只来自已确认的名称（已批准根域名 + 实体全称），与 G2 的查询计划同源，不扩展短词。

use super::super::observations::{FindingDraft, NewObservation};
use super::super::orchestrator::ItemOutcome;
use super::super::redaction::{content_hash, credential_sightings, evidence_excerpt, CredentialGrade};
use super::super::rules::CREDENTIAL_PLACEHOLDER_CONFIDENCE;
use super::super::settings::DocumentUrl;
use super::super::state::{SourceStatus, WorkItemStatus};
use super::super::work_items::WorkItem;
use super::http::{FetchBlock, PassiveClient, PassiveFetch, RequestMeter};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};

pub const ADAPTER_VERSION: &str = "oviraptor.exposure.documents.v1";
pub const KIND_DOCUMENT_LINK: &str = "document_link";
pub const KIND_DOCUMENT_IMPORT: &str = "document_import";
pub const KIND_INTEL_IMPORT: &str = "public_intel_import";

pub const CAPABILITY_EXPLICIT_URL: &str = "explicit_document_url";
pub const CAPABILITY_FILE_IMPORT: &str = "file_import";
pub const CAPABILITY_INTEL_IMPORT: &str = "public_intel_import";

/// The work key of the single placeholder item a round plans when the source is enabled but
/// no URL was specified: the gap is named instead of disappearing.
pub const UNCONFIGURED_ENTITY: &str = "<未指定文档 URL>";

pub const TITLE_DOCUMENT_MENTION: &str = "文档提及集团名称";
pub const TITLE_DOCUMENT_DECLARED_PUBLIC: &str = "已声明为公开材料的文档";
pub const TITLE_DOCUMENT_CREDENTIAL: &str = "文档含凭据形态值";
pub const TITLE_INTEL_CLAIM: &str = "公开情报声称涉及集团";

pub const OBS_DOCUMENT_READ: &str = "document_read";
pub const OBS_DOCUMENT_METADATA: &str = "document_metadata";
pub const OBS_DOCUMENT_IMPORT: &str = "document_import";
pub const OBS_INTEL_RECORD: &str = "intel_record";

pub const EXCERPT_CHARS: usize = 400;
pub const SCAN_LINES: usize = 20_000;
pub const ANALYZED_CONTENT_TYPES: &[&str] = &[
    "text/html",
    "application/xhtml+xml",
    "text/plain",
    "text/csv",
    "text/markdown",
    "application/json",
    "text/xml",
    "application/xml",
];
const ACCEPT: &str = "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.1";

/// The confirmed names a document is scanned for: approved root domains plus the profile's
/// legal name (two characters or more). Same origin as the G2 query plan.
pub fn group_terms(connection: &Connection, project_id: i64) -> Result<Vec<String>, String> {
    let row: Option<(String, String)> = connection
        .query_row(
            "SELECT COALESCE(approved_domains_json,'[]'),COALESCE(legal_name,'') FROM asset_ownership_profiles WHERE project_id=?1",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("[exposure_document_terms] 读取已确认名称失败：{error}"))?;
    let Some((domains, legal_name)) = row else {
        return Ok(Vec::new());
    };
    let mut terms: Vec<String> = serde_json::from_str::<Vec<String>>(&domains)
        .unwrap_or_default()
        .into_iter()
        .map(|domain| domain.trim().trim_start_matches("*.").to_ascii_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect();
    let legal_name = legal_name.trim().to_string();
    if legal_name.chars().count() >= 2 {
        terms.push(legal_name);
    }
    let mut unique = Vec::new();
    for term in terms {
        if !unique.contains(&term) {
            unique.push(term);
        }
    }
    Ok(unique)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentClass {
    Html,
    Text,
    Unsupported(String),
}

/// Formats recognised by their leading bytes, whatever name or type they were given.
pub fn magic_type(body: &[u8]) -> Option<&'static str> {
    let magic: &[(&[u8], &str)] = &[
        (b"%PDF", "pdf"),
        (b"PK\x03\x04", "zip_or_office"),
        (&[0xD0, 0xCF, 0x11, 0xE0], "ole_office"),
        (b"\x89PNG", "image"),
        (&[0xFF, 0xD8, 0xFF], "image"),
        (b"GIF8", "image"),
        (&[0x1F, 0x8B], "gzip"),
        (b"Rar!", "rar"),
        (b"7z\xBC\xAF", "7z"),
    ];
    magic.iter().find(|(prefix, _)| body.starts_with(prefix)).map(|(_, label)| *label)
}

/// Magic bytes win over the declared type: a PDF served as text/html is still a PDF.
pub fn content_class(content_type: &str, body: &[u8]) -> ContentClass {
    if let Some(label) = magic_type(body) {
        return ContentClass::Unsupported(label.to_string());
    }
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match mime.as_str() {
        "text/html" | "application/xhtml+xml" => ContentClass::Html,
        "text/plain" | "text/csv" | "text/markdown" | "application/json" | "text/xml" | "application/xml" => {
            ContentClass::Text
        }
        "" => {
            let head = String::from_utf8_lossy(&body[..body.len().min(2048)]).to_ascii_lowercase();
            if head.contains("<html") || head.contains("<!doctype html") {
                ContentClass::Html
            } else if std::str::from_utf8(&body[..body.len().min(4096)]).is_ok() {
                ContentClass::Text
            } else {
                ContentClass::Unsupported("unknown_binary".to_string())
            }
        }
        "application/pdf" => ContentClass::Unsupported("pdf".to_string()),
        other => ContentClass::Unsupported(other.to_string()),
    }
}

/// A declared charset other than UTF-8 is not guessed at.
pub fn declared_charset(content_type: &str) -> Option<String> {
    content_type.split(';').skip(1).find_map(|part| {
        let (name, value) = part.split_once('=')?;
        (name.trim().eq_ignore_ascii_case("charset"))
            .then(|| value.trim().trim_matches('"').to_ascii_lowercase())
    })
}

/// Strict UTF-8 (BOM stripped). A body cut at the byte cap may end inside a character; only
/// that incomplete tail is dropped, anything else invalid is refused.
pub fn utf8_text(body: &[u8], truncated: bool) -> Result<String, String> {
    let body = body.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(body);
    match std::str::from_utf8(body) {
        Ok(text) => Ok(text.to_string()),
        Err(error) if truncated && error.error_len().is_none() => {
            Ok(String::from_utf8_lossy(&body[..error.valid_up_to()]).to_string())
        }
        Err(error) => Err(format!("第 {} 字节起不是有效 UTF-8", error.valid_up_to())),
    }
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        // Byte search: `;` is ASCII, so the slice below always lands on a char boundary.
        let end = tail.bytes().take(12).position(|byte| byte == b';');
        let decoded = end.and_then(|end| {
            let name = &tail[1..end];
            let value = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ if name.starts_with("#x") || name.starts_with("#X") => {
                    u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32)
                }
                _ if name.starts_with('#') => name[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            }?;
            Some((value, end + 1))
        });
        match decoded {
            Some((value, used)) => {
                out.push(value);
                rest = &tail[used..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Visible text of an HTML page plus its `<title>`. Scripts, styles and comments are dropped,
/// never executed; tags become spaces so words do not glue together.
pub fn html_text(html: &str) -> (String, String) {
    let lower = html.to_ascii_lowercase();
    let title = lower
        .find("<title")
        .and_then(|start| {
            let open = start + lower[start..].find('>')? + 1;
            let close = open + lower[open..].find("</title")?;
            html.get(open..close)
        })
        .map(|raw| normalize_text(&decode_entities(raw)))
        .unwrap_or_default();
    let mut out = String::with_capacity(html.len() / 2);
    let mut index = 0;
    let skip_to = |from: usize, marker: &str| -> usize {
        lower[from..]
            .find(marker)
            .map(|offset| from + offset + marker.len())
            .unwrap_or(html.len())
    };
    while index < html.len() {
        let rest = &lower[index..];
        if rest.starts_with("<script") {
            index = skip_to(index, "</script>");
            out.push(' ');
            continue;
        }
        if rest.starts_with("<style") {
            index = skip_to(index, "</style>");
            out.push(' ');
            continue;
        }
        if rest.starts_with("<!--") {
            index = skip_to(index, "-->");
            continue;
        }
        if rest.starts_with('<') {
            index = skip_to(index, ">");
            out.push(' ');
            continue;
        }
        let character = html[index..].chars().next().unwrap_or(' ');
        out.push(character);
        index += character.len_utf8();
    }
    (title, decode_entities(&out))
}

/// Whitespace-collapsed text. Its hash is the "same normalized full text" identity; it is
/// only claimed for a body that was read whole.
pub fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Clone, Debug, Default)]
pub struct TextAnalysis {
    pub mentions: Vec<String>,
    pub excerpt: String,
    pub credential: Option<(String, String, usize)>,
    pub placeholder: Option<(String, String)>,
    pub normalized_sha256: String,
    pub normalized_chars: usize,
}

pub fn analyze_text(text: &str, terms: &[String]) -> TextAnalysis {
    let normalized = normalize_text(text);
    let lower = normalized.to_lowercase();
    let mut mentions = Vec::new();
    let mut first: Option<usize> = None;
    for term in terms {
        let needle = term.to_lowercase();
        if needle.is_empty() {
            continue;
        }
        if let Some(position) = lower.find(&needle) {
            mentions.push(term.clone());
            let chars_before = lower[..position].chars().count();
            first = Some(first.map_or(chars_before, |current| current.min(chars_before)));
        }
    }
    let excerpt = match first {
        Some(at) => {
            let window: String = normalized.chars().skip(at.saturating_sub(80)).take(320).collect();
            evidence_excerpt(&window, EXCERPT_CHARS)
        }
        None => String::new(),
    };
    let sightings = credential_sightings(text, SCAN_LINES);
    let credential = sightings
        .iter()
        .find(|row| row.grade == CredentialGrade::Candidate)
        .map(|row| (row.key.clone(), row.masked.clone(), row.line));
    let placeholder = sightings
        .iter()
        .find(|row| row.grade == CredentialGrade::Placeholder)
        .map(|row| (row.key.clone(), row.masked.clone()));
    TextAnalysis {
        mentions,
        excerpt,
        credential,
        placeholder,
        normalized_sha256: content_hash(normalized.as_bytes()),
        normalized_chars: normalized.chars().count(),
    }
}

/// How the text reached the application, written into every finding so a reader never
/// mistakes an imported file for a network observation.
pub struct DocumentFacts<'a> {
    pub source_key: &'a str,
    pub url: &'a str,
    pub mode_note: &'a str,
    pub declared_public: Option<&'a str>,
}

pub fn document_findings(facts: &DocumentFacts<'_>, analysis: &TextAnalysis) -> Vec<FindingDraft> {
    let mut drafts = Vec::new();
    match (&analysis.credential, &analysis.placeholder) {
        (Some((key, masked, line)), _) => drafts.push(FindingDraft {
            category: "credential".to_string(),
            title: TITLE_DOCUMENT_CREDENTIAL.to_string(),
            url: facts.url.to_string(),
            source_type: facts.source_key.to_string(),
            evidence: format!("字段 {key} = {masked}（第 {line} 行）；{}", facts.mode_note),
            severity: "high".to_string(),
            confidence: CREDENTIAL_PLACEHOLDER_CONFIDENCE + 40,
            asset_id: None,
            content_hash: content_hash(format!("{key}|{masked}").as_bytes()),
        }),
        (None, Some((key, masked))) => drafts.push(FindingDraft {
            category: "credential".to_string(),
            title: TITLE_DOCUMENT_CREDENTIAL.to_string(),
            url: facts.url.to_string(),
            source_type: facts.source_key.to_string(),
            evidence: format!("字段 {key} 只有占位值 {masked}；{}", facts.mode_note),
            severity: "low".to_string(),
            confidence: CREDENTIAL_PLACEHOLDER_CONFIDENCE,
            asset_id: None,
            content_hash: content_hash(format!("{key}|{masked}").as_bytes()),
        }),
        (None, None) => {}
    }
    if !analysis.mentions.is_empty() {
        let public_note = facts
            .declared_public
            .map(|note| format!("；声明为公开材料：{note}"))
            .unwrap_or_default();
        drafts.push(FindingDraft {
            category: "document".to_string(),
            title: if facts.declared_public.is_some() {
                TITLE_DOCUMENT_DECLARED_PUBLIC
            } else {
                TITLE_DOCUMENT_MENTION
            }
            .to_string(),
            url: facts.url.to_string(),
            source_type: facts.source_key.to_string(),
            evidence: format!(
                "提及 {}；摘录：{}；{}{public_note}",
                analysis.mentions.join("、"),
                analysis.excerpt,
                facts.mode_note
            ),
            severity: "low".to_string(),
            confidence: 30,
            asset_id: None,
            content_hash: analysis.normalized_sha256.clone(),
        });
    }
    drafts
}

/// §13 a document whose whole normalized text equals an earlier one in the same project is a
/// `same_document` *candidate*: recorded on the new observation, never merged. Truncated
/// bodies carry no normalized hash, so they can never be called "the same full text".
pub fn attach_same_document(
    connection: &Connection,
    project_id: i64,
    observations: &mut [NewObservation],
) -> Result<(), String> {
    for observation in observations.iter_mut() {
        let Some(hash) = observation
            .extra
            .get("normalizedTextSha256")
            .and_then(JsonValue::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            continue;
        };
        let mut statement = connection
            .prepare(
                "SELECT id,url,source_key FROM exposure_observations
                 WHERE project_id=?1 AND source_key IN ('document_link','document_import','public_intel_import')
                   AND json_extract(evidence_json,'$.detail.normalizedTextSha256')=?2 AND url<>?3
                 ORDER BY id LIMIT 20",
            )
            .map_err(|error| format!("[exposure_same_document] {error}"))?;
        let rows: Vec<JsonValue> = statement
            .query_map(params![project_id, hash, observation.url], |row| {
                Ok(json!({
                    "observationId": row.get::<_, i64>(0)?,
                    "url": row.get::<_, String>(1)?,
                    "sourceKey": row.get::<_, String>(2)?,
                    "relation": "same_document",
                    "status": "candidate",
                    "basis": "normalized_full_text_sha256",
                }))
            })
            .map_err(|error| format!("[exposure_same_document] {error}"))?
            .collect::<Result<_, _>>()
            .map_err(|error| format!("[exposure_same_document] {error}"))?;
        if let Some(map) = observation.extra.as_object_mut() {
            map.insert("sameDocumentCandidates".to_string(), json!(rows));
        }
    }
    Ok(())
}

fn outcome_with(status: WorkItemStatus, source: SourceStatus, reason: String) -> ItemOutcome {
    ItemOutcome {
        status: Some(status),
        source_status: Some(source),
        source_reason: reason.clone(),
        reason,
        ..Default::default()
    }
}

/// The source is on, no URL is configured: one named gap, no request.
pub fn unconfigured_link() -> ItemOutcome {
    outcome_with(
        WorkItemStatus::SkippedUnconfigured,
        SourceStatus::Unconfigured,
        "document_link_unconfigured：未指定任何公开文档 URL（设置 → 指定公开文档），本来源未发送请求".to_string(),
    )
}

/// Parses an operator-specified document URL: http(s), a host, no userinfo; the fragment is
/// never sent and is dropped. Returns the canonical form stored in the settings.
pub fn canonical_document_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 2048 {
        return Err("地址为空或超过 2048 字符".to_string());
    }
    let mut url = reqwest::Url::parse(trimmed).map_err(|error| format!("无法解析：{error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("只支持 http/https，收到 {}", url.scheme()));
    }
    if url.host_str().unwrap_or_default().is_empty() {
        return Err("缺少主机名".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("地址不能带用户名或密码".to_string());
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

pub struct LinkContext<'a> {
    pub client: &'a PassiveClient,
    pub terms: &'a [String],
    /// Test switch only (fixture on 127.0.0.1); production is always false.
    pub allow_private: bool,
    /// Test switch only (fixture on a random port); production allows the scheme's port only.
    pub any_port: bool,
    pub cancelled: &'a dyn Fn() -> bool,
}

/// One explicit document URL: one GET, no redirect, bounded body.
pub fn execute_link(
    context: &LinkContext<'_>,
    item: &WorkItem,
    meter: &mut RequestMeter,
    entry: Option<&DocumentUrl>,
) -> ItemOutcome {
    if item.entity == UNCONFIGURED_ENTITY {
        return unconfigured_link();
    }
    let Some(entry) = entry else {
        return outcome_with(
            WorkItemStatus::SkippedUnconfigured,
            SourceStatus::Unconfigured,
            format!("document_link_not_in_frozen_list：{} 不在本轮冻结的指定列表里，未发送请求", item.entity),
        );
    };
    let url = match canonical_document_url(&entry.url).and_then(|text| {
        reqwest::Url::parse(&text).map_err(|error| error.to_string())
    }) {
        Ok(url) => url,
        Err(reason) => {
            return outcome_with(
                WorkItemStatus::BlockedAuthorization,
                SourceStatus::Failed,
                format!("document_url_invalid：{reason}"),
            )
        }
    };
    // `port()` is None for the scheme's default port.
    if url.port().is_some() && !context.any_port {
        return outcome_with(
            WorkItemStatus::BlockedAuthorization,
            SourceStatus::Failed,
            format!("document_port_denied：{} 使用非默认端口，指定文档只允许 80/443，未发送请求", url),
        );
    }
    let origin = url.origin().ascii_serialization();
    let fetched = context.client.fetch_provider_api(
        &url,
        std::slice::from_ref(&origin),
        &[("Accept", ACCEPT)],
        None,
        meter,
        context.allow_private,
        context.cancelled,
    );
    match fetched {
        Err(FetchBlock::Cancelled) => outcome_with(
            WorkItemStatus::Cancelled,
            SourceStatus::Cancelled,
            "任务已取消，未再发出请求".to_string(),
        ),
        Err(block) if matches!(block.code(), "request_budget_exhausted" | "byte_budget_exhausted") => outcome_with(
            WorkItemStatus::BlockedQuota,
            SourceStatus::Partial,
            format!("document_budget_exhausted：{}", block.message()),
        ),
        Err(FetchBlock::Denied(denial)) => {
            let code = match denial.code {
                "provider_endpoint_private_denied" => "document_private_denied",
                "provider_userinfo_denied" => "document_userinfo_denied",
                "provider_scheme_denied" => "document_scheme_denied",
                other => other,
            };
            outcome_with(
                WorkItemStatus::BlockedAuthorization,
                SourceStatus::Failed,
                format!("{code}：{}；未发送请求", denial.message),
            )
        }
        Err(block) => outcome_with(
            WorkItemStatus::Failed,
            SourceStatus::Failed,
            format!("document_transport_failed：{}", block.message()),
        ),
        Ok(fetch) => response_outcome(context, item, entry, &url, fetch),
    }
}

fn response_outcome(
    context: &LinkContext<'_>,
    item: &WorkItem,
    entry: &DocumentUrl,
    url: &reqwest::Url,
    fetch: PassiveFetch,
) -> ItemOutcome {
    let address = url.to_string();
    let status = fetch.status;
    let mut extra = json!({
        "adapterVersion": ADAPTER_VERSION,
        "capability": CAPABILITY_EXPLICIT_URL,
        "collectionMode": "http_read",
        "networkObserved": true,
        "declaredPublic": entry.public_material,
        "declaredPublicNote": entry.note,
        "receivedBytes": fetch.body.len(),
        "receivedBytesSha256": fetch.content_hash,
        "truncated": fetch.truncated,
        "contentAnalyzed": false,
    });
    let mut observation = NewObservation {
        final_url: fetch.final_url.clone(),
        status_code: i64::from(status),
        content_type: fetch.content_type.clone(),
        body_bytes: fetch.body.len() as i64,
        truncated: fetch.truncated,
        boundary: fetch.boundary.clone(),
        current_accessible: Some((200..300).contains(&status)),
        ..NewObservation::new(&item.source_key, OBS_DOCUMENT_METADATA, &address)
    };
    let failure = |observation: NewObservation, extra: JsonValue, item_status, source, reason: String| {
        let mut outcome = outcome_with(item_status, source, reason);
        outcome.observations.push(NewObservation { extra, ..observation });
        outcome
    };
    match status {
        200..=299 => {}
        300..=399 => {
            let location = fetch.provider_header("location").unwrap_or_default().to_string();
            extra["location"] = json!(location);
            return failure(
                observation,
                extra,
                WorkItemStatus::BlockedAuthorization,
                SourceStatus::Partial,
                format!("document_redirect_not_followed：{status} → {location}；跳转不跟随，如需读取请把最终地址加入指定列表"),
            );
        }
        401 | 403 => {
            return failure(
                observation,
                extra,
                WorkItemStatus::Failed,
                SourceStatus::AuthFailed,
                format!("document_access_denied_{status}：需要登录或无权访问；不尝试登录，也不换身份"),
            )
        }
        404 | 410 => {
            return failure(
                observation,
                extra,
                WorkItemStatus::Failed,
                SourceStatus::Failed,
                format!("document_not_found_{status}：地址当前不可访问"),
            )
        }
        429 => {
            return failure(
                observation,
                extra,
                WorkItemStatus::BlockedQuota,
                SourceStatus::RateLimited,
                "document_rate_limited_429：对方限流，本轮不重试".to_string(),
            )
        }
        other => {
            return failure(
                observation,
                extra,
                WorkItemStatus::Failed,
                SourceStatus::Failed,
                format!("document_http_{other}：读取失败"),
            )
        }
    }
    let class = content_class(&fetch.content_type, &fetch.body);
    let text = match class {
        ContentClass::Unsupported(label) => {
            extra["unsupportedType"] = json!(label);
            let mut outcome = failure(
                observation,
                extra,
                WorkItemStatus::Completed,
                SourceStatus::Partial,
                format!("document_unsupported_type：{label}（{}）正文未分析，只记录了状态、类型、字节和摘要哈希；对应解析器未实现", fetch.content_type),
            );
            outcome.observed = 1;
            return outcome;
        }
        ContentClass::Html | ContentClass::Text => {
            if let Some(charset) = declared_charset(&fetch.content_type).filter(|name| name != "utf-8" && name != "utf8") {
                extra["charset"] = json!(charset);
                return failure(
                    observation,
                    extra,
                    WorkItemStatus::Failed,
                    SourceStatus::ParseError,
                    format!("document_charset_unsupported：声明编码 {charset}，只分析 UTF-8，正文未分析"),
                );
            }
            match utf8_text(&fetch.body, fetch.truncated) {
                Ok(text) => text,
                Err(reason) => {
                    return failure(
                        observation,
                        extra,
                        WorkItemStatus::Failed,
                        SourceStatus::ParseError,
                        format!("document_not_utf8：{reason}，正文未分析"),
                    )
                }
            }
        }
    };
    let (title, visible) = if class == ContentClass::Html {
        html_text(&text)
    } else {
        (String::new(), text.clone())
    };
    let analysis = analyze_text(&visible, context.terms);
    extra["contentAnalyzed"] = json!(true);
    extra["title"] = json!(evidence_excerpt(&title, 200));
    extra["mentions"] = json!(analysis.mentions);
    extra["excerpt"] = json!(analysis.excerpt);
    extra["normalizedChars"] = json!(analysis.normalized_chars);
    if !fetch.truncated {
        extra["normalizedTextSha256"] = json!(analysis.normalized_sha256);
    }
    observation.kind = OBS_DOCUMENT_READ.to_string();
    observation.body = fetch.body.clone();
    observation.extra = extra;
    let declared = entry.public_material.then_some(entry.note.as_str());
    let facts = DocumentFacts {
        source_key: &item.source_key,
        url: &address,
        mode_note: "已通过 HTTP 读取正文（指定公开文档）",
        declared_public: declared,
    };
    let findings = document_findings(&facts, &analysis);
    let mut outcome = ItemOutcome {
        observed: 1,
        ..Default::default()
    };
    let (item_status, source, reason) = if fetch.truncated {
        (
            WorkItemStatus::Completed,
            SourceStatus::Partial,
            format!("document_truncated：正文超过单响应上限，只分析了前 {} 字节，不声称全文", fetch.body.len()),
        )
    } else if findings.is_empty() {
        (
            WorkItemStatus::Completed,
            SourceStatus::CompletedNoMatches,
            "已读取正文，未出现集团名称或凭据形态值；不代表没有泄漏".to_string(),
        )
    } else {
        (
            WorkItemStatus::Completed,
            SourceStatus::Completed,
            format!("已读取正文，生成 {} 条线索", findings.len()),
        )
    };
    outcome.status = Some(item_status);
    outcome.source_status = Some(source);
    outcome.reason = reason.clone();
    outcome.source_reason = reason;
    outcome.observations.push(observation);
    outcome.findings = findings;
    outcome
}

/// Each capability on its own row. IR-14 landed 2026-10-09: the import and grade commands
/// are registered, so those rows are configured. PDF/Office/OCR stay unsupported.
pub fn capabilities(document_urls: usize) -> JsonValue {
    json!([
        {
            "sourceKey": KIND_DOCUMENT_LINK,
            "capability": CAPABILITY_EXPLICIT_URL,
            "credential": "none",
            "state": if document_urls > 0 { "configured" } else { "unconfigured" },
            "reason": if document_urls > 0 {
                format!("已指定 {document_urls} 个地址；只读这些精确地址，跳转不跟随，默认不启用，需在启用来源里勾选")
            } else {
                "未指定文档 URL；默认不启用".to_string()
            },
            "bounds": {
                "maxUrls": super::super::settings::DOCUMENT_URLS_LIMIT,
                "requestsPerUrl": 1,
                "redirects": "not_followed",
                "ports": "80/443",
                "maxBytes": "本轮单响应上限",
                "analyzedContentTypes": ANALYZED_CONTENT_TYPES,
                "unsupported": ["pdf", "docx/xlsx/zip", "image/ocr"],
            },
        },
        {
            "sourceKey": KIND_DOCUMENT_IMPORT,
            "capability": CAPABILITY_FILE_IMPORT,
            "credential": "none",
            "state": "configured",
            "integration": "available",
            "reason": "导入命令已注册（IR-14 已落地）：UTF-8 文本、JSON、CSV、已提取文本。声明的 URL 只记录、不访问。PDF/Office/OCR 仍不支持",
            "bounds": {
                "formats": super::super::imports::FORMATS,
                "maxFiles": super::super::imports::IMPORT_MAX_FILES,
                "maxFileBytes": super::super::imports::IMPORT_FILE_MAX_BYTES,
                "maxBatchBytes": super::super::imports::IMPORT_BATCH_MAX_BYTES,
            },
        },
        {
            "sourceKey": KIND_INTEL_IMPORT,
            "capability": CAPABILITY_INTEL_IMPORT,
            "credential": "none",
            "state": "configured",
            "integration": "available",
            "reason": "情报导入与人工定级命令已注册（IR-14 已落地）：JSON/CSV，等级 claim_only / sample_supported / human_verified。不自动关闭线索，不访问来源",
            "bounds": {
                "formats": ["json", "csv"],
                "maxRecords": super::super::imports::INTEL_MAX_RECORDS,
            },
        },
        {
            "sourceKey": KIND_DOCUMENT_IMPORT,
            "capability": "pdf_docx_ocr",
            "credential": "none",
            "state": "unsupported",
            "reason": "PDF、Office、图片 OCR 没有解析器；这类文件导入时被拒绝、读取时只记元数据，不伪装解析成功",
        },
        {
            "sourceKey": "netdisk",
            "capability": "netdisk_keyword_search",
            "credential": "none",
            "state": "unsupported",
            "reason": "通用文库/网盘关键词搜索、登录、提取码与目录枚举不在本期",
        },
        {
            "sourceKey": "underground_intel",
            "capability": "underground_feed",
            "credential": "none",
            "state": "unsupported",
            "reason": "不部署 Tor、不登录地下论坛、不购买或下载数据；售卖类情报只支持用户提供的脱敏报告导入",
        },
    ])
}
