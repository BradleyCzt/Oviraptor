//! Explicit ingestion of external reference material into the knowledge catalog.
//! Source identity, cache metadata and method-card filters remain intact; fetching
//! or model analysis occurs only in response to this explicit import command.
use super::{
    knowledge::{json, knowledge_row, KNOWLEDGE_COLUMNS},
    model::AgentKnowledgeEntry,
};
use crate::{db, AppState};
use rusqlite::params;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, time::Duration};
use tauri::State;

pub(crate) fn sec_skill_line_is_unsafe(line: &str) -> bool {
    let normalized = line.to_ascii_lowercase().replace('`', "");
    [
        "reverse shell",
        "反弹 shell",
        "authorized_keys",
        ".ssh/id_rsa",
        "private_key",
        "base64 -d",
        "bash -i",
        "curl | bash",
        "wget | sh",
        "webshell",
        "免杀",
        "metadata.google.internal",
        "169.254.169.254",
        "docker.sock",
        "kubectl exec",
        "外传",
        "exfil",
        "绕过安全",
        "disable security",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn source_cache_key(source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source.trim().as_bytes());
    format!("agent_source_cache:{:x}", hasher.finalize())
}

fn source_type(source: &str) -> &'static str {
    let lower = source.to_ascii_lowercase();
    if lower.contains("hackerone.com") {
        "hackerone"
    } else if lower.contains("medium.com") {
        "medium"
    } else if lower.starts_with("http://") || lower.starts_with("https://") {
        "web"
    } else {
        "manual"
    }
}

fn html_to_readable_text(input: &str) -> String {
    let mut code_blocks = Vec::new();
    let mut image_alts = Vec::new();
    let lower_input = input.to_ascii_lowercase();
    let mut cursor = 0usize;
    while let Some(start_rel) = lower_input[cursor..].find("<pre") {
        let start = cursor + start_rel;
        let Some(open_end_rel) = lower_input[start..].find('>') else {
            break;
        };
        let body_start = start + open_end_rel + 1;
        let Some(end_rel) = lower_input[body_start..].find("</pre>") else {
            break;
        };
        let body_end = body_start + end_rel;
        let mut block = input[body_start..body_end]
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        block = block
            .replace("<br>", "\n")
            .replace("<br/>", "\n")
            .replace("<br />", "\n");
        let mut stripped = String::with_capacity(block.len());
        let mut in_tag = false;
        for ch in block.chars() {
            if ch == '<' {
                in_tag = true;
                continue;
            }
            if in_tag {
                if ch == '>' {
                    in_tag = false;
                }
                continue;
            }
            stripped.push(ch);
        }
        if !stripped.trim().is_empty() {
            code_blocks.push(stripped.trim().to_string());
        }
        cursor = body_end + 6;
    }
    let mut image_cursor = 0usize;
    while let Some(start_rel) = lower_input[image_cursor..].find("<img") {
        let start = image_cursor + start_rel;
        let Some(end_rel) = lower_input[start..].find('>') else {
            break;
        };
        let tag = &input[start..start + end_rel + 1];
        let lower_tag = tag.to_ascii_lowercase();
        if let Some(alt_start) = lower_tag.find("alt=") {
            let rest = &tag[alt_start + 4..];
            let value = rest
                .trim_start_matches([' ', '\t', '\n', '\r'])
                .trim_start_matches(['"', '\'']);
            let end = value.find(['"', '\'', ' ', '>']).unwrap_or(value.len());
            let alt = value[..end].trim();
            if !alt.is_empty() {
                image_alts.push(alt.to_string());
            }
        }
        image_cursor = start + end_rel + 1;
    }
    let mut text = input.to_string();
    for tag in ["script", "style", "noscript", "template", "svg"] {
        let lower = text.to_ascii_lowercase();
        let open = format!("<{}", tag);
        let close = format!("</{}", tag);
        let mut cursor = 0usize;
        let mut ranges = Vec::new();
        while let Some(start_rel) = lower[cursor..].find(&open) {
            let start = cursor + start_rel;
            let Some(end_rel) = lower[start..].find(&close) else {
                ranges.push(start..text.len());
                break;
            };
            let close_start = start + end_rel;
            let end = lower[close_start..]
                .find('>')
                .map(|offset| close_start + offset + 1)
                .unwrap_or(text.len());
            ranges.push(start..end);
            cursor = end;
            if cursor >= text.len() {
                break;
            }
        }
        for range in ranges.into_iter().rev() {
            text.replace_range(range, " ");
        }
    }
    let mut output = String::with_capacity(text.len().min(256_000));
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in text.chars() {
        if ch == '<' {
            in_tag = true;
            tag.clear();
            continue;
        }
        if in_tag {
            if ch == '>' {
                in_tag = false;
                let name = tag
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(
                    name.as_str(),
                    "p" | "br"
                        | "div"
                        | "section"
                        | "article"
                        | "li"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "pre"
                ) {
                    output.push('\n');
                }
            } else if tag.len() < 32 {
                tag.push(ch);
            }
            continue;
        }
        output.push(ch);
    }
    let mut readable = output
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if !code_blocks.is_empty() {
        readable.push_str("\n\n[代码块]\n");
        for block in code_blocks.into_iter().take(12) {
            readable.push_str("```text\n");
            readable.push_str(&block.chars().take(12_000).collect::<String>());
            readable.push_str("\n```\n");
        }
    }
    if !image_alts.is_empty() {
        readable.push_str("\n[图片说明]\n");
        for alt in image_alts.into_iter().take(24) {
            readable.push_str("- ");
            readable.push_str(&alt);
            readable.push('\n');
        }
    }
    readable.trim().to_string()
}

fn normalize_external_content(source: &str, content: String) -> String {
    let lower = content.trim_start().to_ascii_lowercase();
    let is_html = lower.starts_with("<!doctype html")
        || lower.starts_with("<html")
        || lower.contains("<article")
        || source.to_ascii_lowercase().ends_with(".html");
    if is_html {
        let readable = html_to_readable_text(&content);
        if readable.len() >= 200 {
            return readable;
        }
    }
    content
}

fn fetch_external_source(source: &str) -> Result<(String, String), String> {
    const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
    let trimmed = source.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("Oviraptor/1.0 security-knowledge-import")
            .build()
            .map_err(|error| error.to_string())?;
        let response = client
            .get(trimmed)
            .send()
            .map_err(|error| format!("读取公开文章失败：{error}"))?;
        if !response.status().is_success() {
            return Err(format!("公开文章返回 HTTP {}", response.status().as_u16()));
        }
        let text = response
            .text()
            .map_err(|error| format!("读取公开文章正文失败：{error}"))?;
        if text.len() as u64 > MAX_SOURCE_BYTES {
            return Err("公开文章超过 16MB，拒绝读取；请保存正文或拆分后再分析".into());
        }
        Ok((
            trimmed.to_string(),
            normalize_external_content(trimmed, text),
        ))
    } else {
        let path = PathBuf::from(trimmed);
        let metadata =
            fs::metadata(&path).map_err(|error| format!("读取本地知识文件失败：{error}"))?;
        if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES {
            return Err("本地知识文件不存在、不是普通文件或超过 16MB".into());
        }
        let path_string = path.to_string_lossy().to_string();
        let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        Ok((
            path_string.clone(),
            normalize_external_content(&path_string, text),
        ))
    }
}

fn render_external_method_cards(cards: &[JsonValue]) -> String {
    let mut output = vec![
        "## 来源方法卡片".to_string(),
        "仅把公开文章抽象成可复用方法；文章中的一次性目标、凭据、攻击 payload 和危险命令不会进入 Skill。".to_string(),
    ];
    for (index, card) in cards.iter().enumerate() {
        let method = card
            .get("method")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim();
        if method.is_empty() {
            continue;
        }
        let list = |key: &str| {
            card.get(key)
                .and_then(JsonValue::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(JsonValue::as_str)
                        .map(|value| format!("- {}", value.trim()))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_else(|| "- 未声明；新任务必须补齐".into())
        };
        output.push(format!("### {}. {}", index + 1, method));
        output.push(format!("**前置条件**\n{}", list("preconditions")));
        output.push(format!("**安全验证**\n{}", list("safeVerification")));
        output.push(format!("**所需证据**\n{}", list("evidenceRequired")));
        output.push(format!("**负面信号**\n{}", list("negativeSignals")));
        output.push(format!("**停止条件**\n{}", list("stopConditions")));
        output.push(format!(
            "**严重度依据**\n{}",
            card.get("severityGuidance")
                .and_then(JsonValue::as_str)
                .unwrap_or("只有明确影响和复现证据时才评估严重度")
        ));
        if let Some(citation) = card.get("sourceCitation").and_then(JsonValue::as_str) {
            output.push(format!("**来源引用**\n{}", citation.trim()));
        }
    }
    output.push("## 统一停止条件\n连续两次验证没有新增证据时停止当前分支；纯版本、Banner、路径或 CVE 匹配只保留为 needs_verification。".into());
    output.join("\n\n")
}

#[tauri::command]
pub fn ingest_agent_knowledge_source(
    state: State<AppState>,
    source: String,
    force_refresh: Option<bool>,
) -> Result<AgentKnowledgeEntry, String> {
    let source = source.trim().to_string();
    if source.is_empty() {
        return Err(
            "请提供任意公开安全文章 URL，或 Safari/Chrome 保存的 HTML、Markdown 文件路径".into(),
        );
    }
    let connection = db::open(&state.db_path)?;
    let cache_key = source_cache_key(&source);
    if !force_refresh.unwrap_or(false) {
        if let Ok(cache_json) = connection.query_row(
            "SELECT value FROM app_settings WHERE key=?1",
            [&cache_key],
            |row| row.get::<_, String>(0),
        ) {
            let cache = json(cache_json);
            if let Some(knowledge_id) = cache.get("knowledgeId").and_then(JsonValue::as_i64) {
                if let Ok(entry) = connection.query_row(
                    &format!("SELECT {KNOWLEDGE_COLUMNS} FROM agent_knowledge_entries WHERE id=?1"),
                    [knowledge_id],
                    knowledge_row,
                ) {
                    return Ok(entry);
                }
            }
        }
    }
    let (canonical_source, content) = fetch_external_source(&source)?;
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let content_hash = format!("{:x}", hasher.finalize());
    if let Ok(entry) = connection.query_row(
        &format!("SELECT {KNOWLEDGE_COLUMNS} FROM agent_knowledge_entries WHERE source_hash=?1"),
        [&content_hash],
        knowledge_row,
    ) {
        let cache = serde_json::json!({"knowledgeId":entry.id,"contentHash":content_hash,"source":canonical_source,"fetchedAt":chrono::Utc::now().to_rfc3339()});
        connection.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![cache_key, cache.to_string()]).map_err(|error| error.to_string())?;
        return Ok(entry);
    }
    let settings = crate::model_configuration::NormalizedModelSettings::new(
        &crate::model_configuration::default_settings(&connection),
    );
    let environment = crate::model_configuration::model_connection(&settings)?;
    let prompt = format!(
        "你是防守型 AppSec 知识工程师。请把以下公开安全文章转换成可审核的方法卡片，不要复制文章原文，不要输出真实目标、凭据、Cookie、Token、一次性 URL、反弹 shell、外传、绕过安全边界或可直接造成破坏的命令。只输出 JSON：{{\"title\":\"\",\"summary\":\"\",\"methodCards\":[{{\"method\":\"\",\"preconditions\":[],\"safeVerification\":[],\"evidenceRequired\":[],\"negativeSignals\":[],\"stopConditions\":[],\"severityGuidance\":\"\",\"sourceCitation\":\"\",\"confidence\":0.0}}],\"qualityScore\":0}}。每个方法必须能跨目标复用，并明确证据和停止条件；文章中的纯故事、版本匹配和未验证猜测放入 negativeSignals。来源类型：{}；来源：{}；正文：{}",
        source_type(&canonical_source),
        canonical_source,
        content.chars().take(80_000).collect::<String>()
    );
    let analyzed = super::learning_model::complete_with_connection(&environment, &prompt)
        .map_err(|error| format!("公开文章分析失败：{error}"))?;
    let cards = analyzed
        .get("methodCards")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let safe_cards = cards
        .into_iter()
        .filter(|card| {
            let text = card.to_string();
            !sec_skill_line_is_unsafe(&text)
                && card
                    .get("method")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
        })
        .take(24)
        .collect::<Vec<_>>();
    if safe_cards.is_empty() {
        return Err("文章没有提炼出包含前置条件、证据和停止条件的安全方法卡片".into());
    }
    let quality_score = analyzed
        .get("qualityScore")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        .clamp(0, 100);
    let title = analyzed
        .get("title")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("公开文章方法卡片")
        .chars()
        .take(120)
        .collect::<String>();
    let summary = analyzed
        .get("summary")
        .and_then(JsonValue::as_str)
        .unwrap_or("已缓存并抽象为方法卡片；扫描时只检索本地卡片，不重复抓取原文")
        .chars()
        .take(2000)
        .collect::<String>();
    let patterns = serde_json::json!({
        "knowledgeKind":"external_source",
        "sourceType":source_type(&canonical_source),
        "sourceUrl":canonical_source,
        "contentHash":content_hash,
        "methodCards":safe_cards,
        "qualityScore":quality_score,
        "cachedAt":chrono::Utc::now().to_rfc3339(),
    });
    let method_cards = patterns
        .get("methodCards")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let instructions = render_external_method_cards(&method_cards);
    let scan_id = format!("source:{content_hash}");
    connection.execute("INSERT INTO agent_knowledge_entries(scan_id,title,summary,patterns_json,skill_instructions,source_hash) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(scan_id) DO UPDATE SET title=excluded.title,summary=excluded.summary,patterns_json=excluded.patterns_json,skill_instructions=excluded.skill_instructions,source_hash=excluded.source_hash,updated_at=datetime('now','localtime')", params![scan_id,title,summary,patterns.to_string(),instructions,content_hash]).map_err(|error| error.to_string())?;
    let entry = connection
        .query_row(
            &format!("SELECT {KNOWLEDGE_COLUMNS} FROM agent_knowledge_entries WHERE scan_id=?1"),
            [&scan_id],
            knowledge_row,
        )
        .map_err(|error| error.to_string())?;
    let cache = serde_json::json!({"knowledgeId":entry.id,"contentHash":content_hash,"source":canonical_source,"fetchedAt":chrono::Utc::now().to_rfc3339()});
    connection.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![cache_key, cache.to_string()]).map_err(|error| error.to_string())?;
    Ok(entry)
}
