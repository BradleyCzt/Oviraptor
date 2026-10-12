// §Q4 — certificate transparency. The index answer is kept as an observation and a pending
// discovery, never as an authorized asset: a name appearing in a public certificate does not
// prove the company owns it, and it certainly does not permit touching it.

use super::super::adapters::http::{FetchBlock, PassiveClient, RequestMeter};
use super::super::observations::NewObservation;
use super::super::orchestrator::ItemOutcome;
use super::super::state::{SourceStatus, WorkItemStatus};
use super::super::work_items::WorkItem;
use serde_json::Value as JsonValue;

pub const ENDPOINT: &str = "https://crt.sh";

/// Every name a crt.sh row lists, narrowed to the requested root domain.
pub fn names_for_domain(body: &str, domain: &str) -> Vec<(String, String, String)> {
    let root = domain.trim().trim_start_matches("*.").to_ascii_lowercase();
    let Ok(rows) = serde_json::from_str::<Vec<JsonValue>>(body) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for row in rows {
        let Some(raw) = row.get("name_value").and_then(JsonValue::as_str) else {
            continue;
        };
        for line in raw.lines() {
            let name = line.trim().trim_start_matches("*.").to_ascii_lowercase();
            if name.is_empty() || !(name == root || name.ends_with(&format!(".{root}"))) {
                continue;
            }
            let first_seen = row
                .get("not_before")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string();
            let last_seen = row
                .get("not_after")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_string();
            if !names.iter().any(|(existing, _, _)| existing == &name) {
                names.push((name, first_seen, last_seen));
            }
        }
    }
    names
}

pub fn query_url(endpoint: &str, domain: &str) -> String {
    format!(
        "{endpoint}/?q=%25.{}&output=json",
        domain.trim().trim_start_matches("*.")
    )
}

pub fn execute(
    client: &PassiveClient,
    item: &WorkItem,
    meter: &mut RequestMeter,
    endpoints: &[String],
    allow_private: bool,
    cancelled: &dyn Fn() -> bool,
) -> ItemOutcome {
    execute_bounded(
        client,
        item,
        meter,
        endpoints,
        allow_private,
        super::super::settings::DEFAULT_CT_PAGE_SIZE,
        cancelled,
    )
}

/// G1-0: `ct_page_size` is the number of names one domain keeps. crt.sh answers with every
/// certificate at once, so the cap is applied after parsing; the cut tail is reported as a
/// quota gap (blocked_quota + partial), never as a complete list.
pub fn execute_bounded(
    client: &PassiveClient,
    item: &WorkItem,
    meter: &mut RequestMeter,
    endpoints: &[String],
    allow_private: bool,
    max_names: i64,
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
    let url = query_url(
        endpoints.first().map(String::as_str).unwrap_or(ENDPOINT),
        &domain,
    );
    let parsed = match reqwest::Url::parse(&url) {
        Ok(value) => value,
        Err(error) => {
            outcome.status = Some(WorkItemStatus::Failed);
            outcome.reason = format!("索引查询地址无效：{error}");
            outcome.source_status = Some(SourceStatus::Failed);
            outcome.source_reason = outcome.reason.clone();
            return outcome;
        }
    };
    if cancelled() {
        outcome.status = Some(WorkItemStatus::Cancelled);
        outcome.reason = "用户取消".to_string();
        outcome.source_status = Some(SourceStatus::Cancelled);
        return outcome;
    }
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
    let mut names = names_for_domain(&body, &domain);
    let found_names = names.len();
    let name_cap = max_names.max(1) as usize;
    let names_cut = found_names > name_cap;
    names.truncate(name_cap);
    if body.trim().is_empty() || !body.trim_start().starts_with('[') {
        // Not JSON at all: an error page or a rate-limit interstitial. Saying "completed, 0
        // names" would be a lie.
        outcome
            .observations
            .push(observation(item, &url, "source_parse_error", 0, &body));
        outcome.status = Some(WorkItemStatus::Failed);
        outcome.reason = "证书透明度响应不是 JSON 数组".to_string();
        outcome.source_status = Some(SourceStatus::ParseError);
        outcome.source_reason = format!("{}：非 JSON 响应", domain);
        return outcome;
    }
    if names.is_empty() && serde_json::from_str::<Vec<JsonValue>>(&body).is_err() {
        outcome
            .observations
            .push(observation(item, &url, "source_parse_error", 0, &body));
        outcome.status = Some(WorkItemStatus::Failed);
        outcome.reason = "证书透明度 JSON 解析失败".to_string();
        outcome.source_status = Some(SourceStatus::ParseError);
        outcome.source_reason = format!("{}：解析失败", domain);
        return outcome;
    }
    for (name, first_seen, last_seen) in &names {
        outcome.observations.push(observation(
            item,
            &format!("https://{name}"),
            "ct_name",
            0,
            &format!("not_before={first_seen} not_after={last_seen}"),
        ));
        outcome
            .pending
            .push(super::super::orchestrator::PendingDiscovery {
                source_key: item.source_key.clone(),
                kind: "domain".to_string(),
                entity: name.clone(),
                canonical_key: name.clone(),
                evidence_url: url.clone(),
            });
        outcome.observed += 1;
    }
    outcome.status = Some(if fetch.truncated || names_cut {
        WorkItemStatus::BlockedQuota
    } else {
        WorkItemStatus::Completed
    });
    if fetch.truncated || names_cut {
        let mut reasons = Vec::new();
        if fetch.truncated {
            reasons.push("索引响应被响应体上限截断，名单可能不完整".to_string());
        }
        if names_cut {
            reasons.push(format!(
                "证书透明度返回 {found_names} 个名称，按配置只保留前 {name_cap} 个，其余 {} 个未记录",
                found_names - name_cap
            ));
        }
        outcome.reason = reasons.join("；");
        outcome.source_status = Some(SourceStatus::Partial);
        outcome.source_reason = outcome.reason.clone();
    } else {
        outcome.source_status = Some(SourceStatus::Completed);
        outcome.source_reason = String::new();
    }
    outcome
}

fn observation(
    item: &WorkItem,
    url: &str,
    kind: &str,
    status: i64,
    detail: &str,
) -> NewObservation {
    NewObservation {
        source_key: item.source_key.clone(),
        kind: kind.to_string(),
        url: url.to_string(),
        status_code: status,
        asset_id: item.asset_id,
        extra: serde_json::json!({"domain": item.entity, "detail": detail}),
        ..NewObservation::new(&item.source_key, kind, url)
    }
}
