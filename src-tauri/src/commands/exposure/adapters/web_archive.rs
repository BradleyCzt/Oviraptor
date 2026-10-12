// §Q4 — web archive. A snapshot in an index is a statement about the past; it says nothing
// about whether the URL is reachable now, so both dimensions are stored separately.

use super::super::adapters::http::{FetchBlock, PassiveClient, RequestMeter};
use super::super::observations::{FindingDraft, NewObservation};
use super::super::orchestrator::{ItemOutcome, PendingDiscovery};
use super::super::redaction::redact_text;
use super::super::rules::document_suspect;
use super::super::state::{SourceStatus, WorkItemStatus};
use super::super::work_items::WorkItem;
use serde_json::Value as JsonValue;

pub const ENDPOINT: &str = "https://web.archive.org";
pub const INDEX_LIMIT: i64 = 200;

/// One CDX row: `(original url, status code, mimetype, timestamp)`.
pub fn rows(body: &str) -> Vec<(String, String, String, String)> {
    let Ok(parsed) = serde_json::from_str::<Vec<Vec<JsonValue>>>(body) else {
        return Vec::new();
    };
    parsed
        .iter()
        .skip(1)
        .filter_map(|row| {
            let text = |index: usize| {
                row.get(index)
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let url = text(0);
            if url.trim().is_empty() {
                return None;
            }
            Some((url, text(1), text(2), text(3)))
        })
        .collect()
}

pub fn query_url(endpoint: &str, domain: &str, limit: i64) -> String {
    format!(
        "{endpoint}/cdx/search/cdx?url=*.{domain}/*&output=json&fl=original,statuscode,mimetype,timestamp&filter=statuscode:200&collapse=urlkey&limit={limit}",
        domain = domain.trim().trim_start_matches("*.")
    )
}

fn high_value_path(url: &str) -> bool {
    let lowered = url.to_ascii_lowercase();
    [
        ".map",
        "/.git/",
        ".env",
        "config.json",
        "backup.zip",
        "web.config",
        ".sql",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

#[allow(clippy::too_many_arguments)]
pub fn execute(
    client: &PassiveClient,
    item: &WorkItem,
    meter: &mut RequestMeter,
    endpoints: &[String],
    allow_private: bool,
    limit: i64,
    cancelled: &dyn Fn() -> bool,
) -> ItemOutcome {
    let mut outcome = ItemOutcome::default();
    let domain = item.entity.clone();
    if domain.trim().is_empty() {
        outcome.status = Some(WorkItemStatus::SkippedUnconfigured);
        outcome.reason = "没有已批准的根域名，来源未配置".to_string();
        outcome.source_status = Some(SourceStatus::Unconfigured);
        outcome.source_reason = "没有已批准的根域名".to_string();
        return outcome;
    }
    let endpoint = endpoints
        .first()
        .cloned()
        .unwrap_or_else(|| ENDPOINT.to_string());
    let url = query_url(&endpoint, &domain, limit);
    let parsed = match reqwest::Url::parse(&url) {
        Ok(value) => value,
        Err(error) => {
            outcome.status = Some(WorkItemStatus::Failed);
            outcome.reason = format!("归档查询地址无效：{error}");
            outcome.source_status = Some(SourceStatus::Failed);
            outcome.source_reason = outcome.reason.clone();
            return outcome;
        }
    };
    let response =
        client.fetch_source_endpoint(&parsed, endpoints, meter, allow_private, cancelled);
    outcome.requests = meter.requests_used;
    outcome.bytes = meter.bytes_used;
    let fetch = match response {
        Ok(value) => value,
        Err(FetchBlock::Cancelled) => {
            outcome.status = Some(WorkItemStatus::Cancelled);
            outcome.reason = "用户取消".to_string();
            outcome.source_status = Some(SourceStatus::Cancelled);
            return outcome;
        }
        Err(block) => {
            let quota = matches!(
                block.code(),
                "request_budget_exhausted" | "byte_budget_exhausted"
            );
            outcome.status = Some(if quota {
                WorkItemStatus::BlockedQuota
            } else {
                WorkItemStatus::Failed
            });
            outcome.reason = format!("{}:{}", block.code(), block.message());
            outcome.source_status = Some(if quota {
                SourceStatus::Partial
            } else {
                SourceStatus::Failed
            });
            outcome.source_reason = outcome.reason.clone();
            return outcome;
        }
    };
    let body = String::from_utf8_lossy(&fetch.body).to_string();
    if !body.trim_start().starts_with('[') {
        outcome
            .observations
            .push(observation(item, &url, "source_parse_error", &body));
        outcome.status = Some(WorkItemStatus::Failed);
        outcome.reason = "归档索引响应不是 JSON 数组".to_string();
        outcome.source_status = Some(SourceStatus::ParseError);
        outcome.source_reason = format!("{}：非 JSON 响应", domain);
        return outcome;
    }
    let captured = rows(&body);
    for (archived, status_code, mimetype, timestamp) in &captured {
        outcome.observations.push(NewObservation {
            source_key: item.source_key.clone(),
            kind: "archive_url".to_string(),
            url: archived.clone(),
            final_url: archived.clone(),
            status_code: status_code.parse().unwrap_or(0),
            content_type: mimetype.clone(),
            boundary: String::new(),
            // -1 keeps "current accessibility" unknown; an index hit is not a live fetch.
            current_accessible: None,
            indexed_at: timestamp.clone(),
            asset_id: item.asset_id,
            extra: serde_json::json!({"domain": domain, "indexLimit": limit, "provenance": "wayback_cdx"}),
            ..NewObservation::new(&item.source_key, "archive_url", archived)
        });
        outcome.observed += 1;
        outcome.pending.push(PendingDiscovery {
            source_key: item.source_key.clone(),
            kind: "url".to_string(),
            entity: archived.clone(),
            canonical_key: super::super::fingerprint::canonical_url(archived),
            evidence_url: url.clone(),
        });
        if document_suspect(archived) {
            outcome.findings.push(FindingDraft {
                category: "document".to_string(),
                title: "历史归档中存在疑似内部文档".to_string(),
                url: archived.clone(),
                source_type: "web_archive".to_string(),
                evidence: format!(
                    "Wayback 索引于 {timestamp} 收录该路径，文件语义命中内部关键词；当前可访问性未验证（本轮没有重新请求该地址）",
                ),
                severity: "medium".to_string(),
                confidence: 60,
                asset_id: item.asset_id,
                content_hash: String::new(),
            });
        }
        if high_value_path(archived) {
            outcome.findings.push(FindingDraft {
                category: "source_code".to_string(),
                title: "历史归档中存在高价值公开路径".to_string(),
                url: archived.clone(),
                source_type: "web_archive".to_string(),
                evidence: redact_text(&format!(
                    "索引命中源码/配置/备份语义路径，收录时间 {timestamp}，MIME {mimetype}；仅作为复核线索"
                )),
                severity: "medium".to_string(),
                confidence: 55,
                asset_id: item.asset_id,
                content_hash: String::new(),
            });
        }
    }
    let truncated = fetch.truncated || captured.len() as i64 >= limit;
    outcome.status = Some(if truncated {
        WorkItemStatus::BlockedQuota
    } else {
        WorkItemStatus::Completed
    });
    if truncated {
        outcome.reason = format!(
            "索引返回达到 {limit} 条上限{}",
            if fetch.truncated {
                "且响应体被截断"
            } else {
                ""
            }
        );
        outcome.source_status = Some(SourceStatus::Partial);
        outcome.source_reason = format!("{}：{}", domain, outcome.reason);
    } else {
        outcome.source_status = Some(SourceStatus::Completed);
    }
    outcome
}

fn observation(item: &WorkItem, url: &str, kind: &str, detail: &str) -> NewObservation {
    NewObservation {
        source_key: item.source_key.clone(),
        kind: kind.to_string(),
        url: url.to_string(),
        asset_id: item.asset_id,
        extra: serde_json::json!({"domain": item.entity, "detail": detail.chars().take(400).collect::<String>()}),
        ..NewObservation::new(&item.source_key, kind, url)
    }
}
