// §Q3 — the passive web walker, executed as one work item per authorized origin. It is the
// only place in this slice that opens a socket at a target, and every send goes through the
// Q2 client, which authorizes the hop, charges the budget and redacts the evidence first.

use super::super::observations::NewObservation;
use super::super::orchestrator::ItemOutcome;
use super::super::redaction::{content_hash, redact_json};
use super::super::rules::{findings_for, natural_links, resource_kind, KIND_TRANSPORT};
use super::super::scope::{ExposureDenial, ExposureGrant, ExposureScope};
use super::super::state::{utc_now, SourceStatus, WorkItemStatus};
use super::super::work_items::WorkItem;
use super::http::{FetchBlock, PassiveClient, PassiveFetch, RequestMeter};
use serde_json::json;
use std::collections::HashSet;

/// Per-origin ceilings. They are named gaps instead of a silent stop, and an attempt costs
/// even when the response is an error or a transport failure.
#[derive(Clone, Copy, Debug)]
pub struct WalkBudget {
    pub max_attempts: usize,
    pub max_queued: usize,
}

impl Default for WalkBudget {
    fn default() -> Self {
        Self {
            max_attempts: 20,
            max_queued: 80,
        }
    }
}

fn seed_urls(base: &reqwest::Url) -> Vec<reqwest::Url> {
    [
        "/",
        "/robots.txt",
        "/sitemap.xml",
        "/.well-known/security.txt",
    ]
    .iter()
    .filter_map(|path| base.join(path).ok())
    .collect()
}

/// The grant origin of a URL, or `None` when it is not an http(s) origin at all.
fn origin_of(candidate: &reqwest::Url) -> Option<String> {
    ExposureGrant::from_url(candidate).map(|grant| grant.origin)
}

fn inside_origin(origin: &str, candidate: &reqwest::Url) -> bool {
    origin_of(candidate).as_deref() == Some(origin)
}

fn observation_for(fetch: &PassiveFetch, asset_id: Option<i64>, item: &WorkItem) -> NewObservation {
    NewObservation {
        source_key: item.source_key.clone(),
        kind: resource_kind(&fetch.final_url, &fetch.content_type).to_string(),
        url: fetch.requested_url.clone(),
        final_url: fetch.final_url.clone(),
        status_code: i64::from(fetch.status),
        content_type: fetch.content_type.clone(),
        body_bytes: fetch.body.len() as i64,
        truncated: fetch.truncated,
        boundary: fetch.boundary.clone(),
        body: fetch.body.clone(),
        headers: redact_json(&json!({
            "contentType": fetch.content_type,
            "contentHash": content_hash(&fetch.body),
            "headersHash": fetch.headers_hash,
        })),
        asset_id,
        current_accessible: Some(fetch.status < 400),
        indexed_at: String::new(),
        method: "GET".to_string(),
        extra: json!({
            "origin": item.origin,
            "workKey": item.work_key,
            "hops": fetch.hops,
            "requestsCharged": fetch.requests_charged,
            "elapsedMs": fetch.elapsed_ms,
            "observedAt": fetch.observed_at,
        }),
    }
}

fn reference_observation(
    item: &WorkItem,
    url: &str,
    kind: &str,
    detail: serde_json::Value,
) -> NewObservation {
    NewObservation {
        source_key: item.source_key.clone(),
        kind: kind.to_string(),
        url: url.to_string(),
        asset_id: item.asset_id,
        extra: detail,
        ..NewObservation::new(&item.source_key, kind, url)
    }
}

/// One origin: breadth-first over natural links that stay inside the authorized origin, under
/// both the run-wide request/byte budget and this origin's attempt ceiling.
pub fn execute(
    client: &PassiveClient,
    scope: &ExposureScope,
    item: &WorkItem,
    meter: &mut RequestMeter,
    budget: &WalkBudget,
    revalidate: &dyn Fn() -> Result<(), ExposureDenial>,
    cancelled: &dyn Fn() -> bool,
) -> ItemOutcome {
    let mut outcome = ItemOutcome::default();
    let base = match reqwest::Url::parse(&format!("{}/", item.origin)) {
        Ok(value) => value,
        Err(error) => {
            outcome.status = Some(WorkItemStatus::Failed);
            outcome.source_status = Some(SourceStatus::Failed);
            outcome.reason = format!("origin 无法解析：{error}");
            return outcome;
        }
    };
    if let Err(denial) = revalidate() {
        outcome.status = Some(WorkItemStatus::BlockedAuthorization);
        outcome.reason = format!("{}:{}", denial.code, denial.message);
        return outcome;
    }
    if let Err(denial) = scope.authorize(&base, &utc_now()) {
        outcome.status = Some(WorkItemStatus::BlockedAuthorization);
        outcome.reason = format!("{}:{}", denial.code, denial.message);
        return outcome;
    }

    let mut queue: Vec<reqwest::Url> = seed_urls(&base);
    let mut visited: HashSet<String> = HashSet::new();
    let mut attempts = 0usize;
    let mut successes = 0usize;
    let mut stopped_by_budget = false;
    let mut outside_references = 0usize;

    while let Some(url) = queue.first().cloned() {
        queue.remove(0);
        if !visited.insert(url.as_str().to_string()) {
            continue;
        }
        if cancelled() {
            outcome.status = Some(WorkItemStatus::Cancelled);
            outcome.source_status = Some(SourceStatus::Cancelled);
            outcome.reason = "用户取消；已保存的观察保留".to_string();
            return outcome;
        }
        if !inside_origin(&item.origin, &url) {
            // A discovered link outside the origin is recorded as a reference, never fetched.
            outside_references += 1;
            outcome.observations.push(reference_observation(
                item,
                url.as_str(),
                "reference_out_of_scope",
                json!({"reason": "origin 未在授权范围内，只记录外链"}),
            ));
            continue;
        }
        if attempts >= budget.max_attempts {
            stopped_by_budget = true;
            break;
        }
        attempts += 1;
        match client.fetch(scope, &url, meter, revalidate, cancelled) {
            Ok(fetch) => {
                successes += 1;
                outcome
                    .observations
                    .push(observation_for(&fetch, item.asset_id, item));
                outcome.findings.extend(findings_for(
                    &fetch.final_url,
                    &fetch.content_type,
                    &fetch.body,
                    item.asset_id,
                ));
                let followable = fetch.content_type.contains("html")
                    || fetch.content_type.is_empty()
                    || fetch.content_type.contains("text/plain");
                if followable && fetch.status < 400 && !fetch.truncated {
                    let text = String::from_utf8_lossy(&fetch.body).to_string();
                    for link in natural_links(&text, &url) {
                        let Ok(parsed) = reqwest::Url::parse(&link) else {
                            continue;
                        };
                        if queue.len() + visited.len() >= budget.max_queued {
                            stopped_by_budget = true;
                            break;
                        }
                        if !visited.contains(&link) {
                            queue.push(parsed);
                        }
                    }
                }
            }
            Err(block) => match block {
                FetchBlock::Cancelled => {
                    outcome.status = Some(WorkItemStatus::Cancelled);
                    outcome.source_status = Some(SourceStatus::Cancelled);
                    outcome.reason = "用户取消；已保存的观察保留".to_string();
                    outcome.requests = meter.requests_used;
                    outcome.bytes = meter.bytes_used;
                    return outcome;
                }
                FetchBlock::Denied(denial) => {
                    outcome.requests = meter.requests_used;
                    outcome.bytes = meter.bytes_used;
                    if matches!(
                        denial.code,
                        "request_budget_exhausted" | "byte_budget_exhausted"
                    ) {
                        stopped_by_budget = true;
                        outcome.reason = format!("配额：{}", denial.message);
                        break;
                    }
                    if matches!(
                        denial.code,
                        "authorization_revoked"
                            | "scope_expired"
                            | "host_not_granted"
                            | "project_inactive"
                            | "userinfo_denied"
                            | "scheme_denied"
                            | "private_destination_denied"
                    ) {
                        outcome.status = Some(WorkItemStatus::BlockedAuthorization);
                        outcome.source_status = Some(SourceStatus::Partial);
                        outcome.source_reason = denial.code.to_string();
                        outcome.reason = format!("{}:{}", denial.code, denial.message);
                        return outcome;
                    }
                    outcome.observations.push(reference_observation(
                        item,
                        url.as_str(),
                        KIND_TRANSPORT,
                        denial.as_json(),
                    ));
                }
                FetchBlock::RedirectStopped {
                    from,
                    location,
                    denial,
                    ..
                } => {
                    outcome.observations.push(reference_observation(
                        item,
                        &from,
                        "redirect_stopped",
                        json!({"target": location, "denial": denial.as_json()}),
                    ));
                    outcome.reason =
                        format!("跳出授权 origin 已停止：{}:{}", denial.code, denial.message);
                }
                FetchBlock::Transport { message, .. } => {
                    outcome.observations.push(reference_observation(
                        item,
                        url.as_str(),
                        KIND_TRANSPORT,
                        json!({"error": message}),
                    ));
                    if attempts == 1 {
                        outcome.source_status = Some(SourceStatus::Failed);
                        outcome.status = Some(WorkItemStatus::Failed);
                        outcome.reason = format!("首页不可达：{message}");
                        outcome.requests = meter.requests_used;
                        outcome.bytes = meter.bytes_used;
                        return outcome;
                    }
                }
            },
        }
        outcome.requests = meter.requests_used;
        outcome.bytes = meter.bytes_used;
    }

    outcome.requests = meter.requests_used;
    outcome.bytes = meter.bytes_used;
    outcome.observed = outcome.observations.len() as i64;
    if stopped_by_budget {
        outcome.source_status = Some(SourceStatus::Partial);
        outcome.status = Some(WorkItemStatus::BlockedQuota);
        if outcome.reason.is_empty() {
            outcome.reason = format!(
                "达到本源 {0} 次尝试上限，仍有 {1} 个候选未读取",
                budget.max_attempts,
                queue.len()
            );
        }
    } else if successes == 0 {
        outcome.source_status = Some(SourceStatus::Failed);
        outcome.status = Some(WorkItemStatus::Failed);
        if outcome.reason.is_empty() {
            outcome.reason = "本源没有任何可读响应，只有传输失败".to_string();
        }
    } else {
        outcome.source_status = Some(if outcome.reason.is_empty() {
            SourceStatus::Completed
        } else {
            // A walk that stopped at an out-of-origin redirect did not cover the origin fully.
            SourceStatus::Partial
        });
        outcome.status = Some(WorkItemStatus::Completed);
        if !outcome.reason.is_empty() && outside_references > 0 {
            // A stopped redirect is a fact about coverage, not a completion defect.
            outcome.reason = format!("{}；外链引用 {outside_references} 个未跟随", outcome.reason);
        } else {
            outcome.reason = String::new();
        }
    }
    outcome
}
