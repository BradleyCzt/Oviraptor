use serde_json::Value;
use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) struct Failure {
    pub(super) category: &'static str,
    pub(super) detail: String,
    pub(super) retryable: bool,
    pub(super) stop_collection: bool,
    pub(super) attempts: usize,
}

impl Failure {
    pub(super) fn cancelled() -> Self {
        Self {
            category: "cancelled",
            detail: "任务已取消；已完成页面保留供原地恢复".into(),
            retryable: false,
            stop_collection: true,
            attempts: 0,
        }
    }

    pub(super) fn checkpoint(detail: String) -> Self {
        let limited = detail.contains("resource_limited")
            || detail.starts_with("receipt_retention_limit:")
            || detail.starts_with("receipt_metadata_limit:");
        Self {
            category: if limited {
                "resource_limited"
            } else {
                "checkpoint_io"
            },
            detail,
            retryable: false,
            stop_collection: true,
            attempts: 0,
        }
    }
}

pub(super) fn sanitize(detail: &str, key: &str, proxy: &str) -> String {
    let mut result = detail.to_string();
    if !key.is_empty() {
        result = result.replace(key, "[REDACTED]");
        if let Ok(mut encoded) = reqwest::Url::parse("http://redaction.invalid/") {
            encoded.query_pairs_mut().append_pair("key", key);
            if let Some(query) = encoded.query() {
                let encoded_key = query.trim_start_matches("key=");
                if !encoded_key.is_empty() {
                    result = result.replace(encoded_key, "[REDACTED]");
                }
            }
        }
    }
    if !proxy.is_empty() {
        result = result.replace(proxy, "[PROXY REDACTED]");
    }
    // reqwest's nested sources can contain the authenticated request URL even
    // after removing the top-level URL. Keep the cause, remove all URLs.
    if let Ok(urls) = regex::Regex::new(r#"(?i)https?://[^\s<>"']+"#) {
        result = urls.replace_all(&result, "[URL REDACTED]").into_owned();
    }
    result.chars().take(1800).collect()
}

// Success receipts retain useful endpoint URLs. Only configured credentials,
// including their URL-encoded forms, are replaced; error sanitization above is
// deliberately stricter. Hashes over receipts describe this redacted JSON.
pub(super) fn redact_payload(payload: &Value, key: &str, proxy: &str) -> (Value, bool) {
    fn percent_decoded(raw: &str) -> String {
        let bytes = raw.as_bytes();
        let mut decoded = Vec::new();
        let mut offset = 0;
        while offset < bytes.len() {
            if bytes[offset] == b'%' && offset + 2 < bytes.len() {
                let hex = |byte: u8| (byte as char).to_digit(16).map(|v| v as u8);
                if let (Some(high), Some(low)) = (hex(bytes[offset + 1]), hex(bytes[offset + 2])) {
                    decoded.push(high * 16 + low);
                    offset += 3;
                    continue;
                }
            }
            decoded.push(bytes[offset]);
            offset += 1;
        }
        String::from_utf8_lossy(&decoded).into_owned()
    }
    let mut secrets = Vec::new();
    for secret in [key, proxy] {
        if !secret.is_empty() {
            secrets.push(secret.to_string());
        }
    }
    if let Ok(url) = reqwest::Url::parse(proxy) {
        for secret in [Some(url.username()), url.password()].into_iter().flatten() {
            if !secret.is_empty() {
                secrets.push(secret.to_string());
                secrets.push(percent_decoded(secret));
            }
        }
    }
    let raw = secrets.clone();
    for secret in raw {
        if let Ok(mut url) = reqwest::Url::parse("http://redaction.invalid/") {
            url.query_pairs_mut().append_pair("value", &secret);
            if let Some(encoded) = url.query().and_then(|q| q.strip_prefix("value=")) {
                if !encoded.is_empty() {
                    secrets.push(encoded.to_string());
                }
            }
        }
    }
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    secrets.dedup();
    fn replace(text: &str, secrets: &[String]) -> String {
        // Even a one-character configured credential can be embedded in an
        // echo. Empty replacement cannot itself reintroduce another secret.
        // Repeat because removing one token can join fragments of another.
        let mut out = text.to_string();
        loop {
            let before = out.len();
            for secret in secrets {
                out = out.replace(secret, "");
            }
            if out.len() == before {
                return out;
            }
        }
    }
    fn walk(value: &Value, secrets: &[String]) -> Value {
        match value {
            Value::String(text) => Value::String(replace(text, secrets)),
            Value::Array(rows) => Value::Array(rows.iter().map(|row| walk(row, secrets)).collect()),
            Value::Object(object) => Value::Object(
                object
                    .iter()
                    .map(|(key, value)| {
                        let key = replace(key, secrets);
                        (key, walk(value, secrets))
                    })
                    .collect(),
            ),
            _ => value.clone(),
        }
    }
    let redacted = walk(payload, &secrets);
    let changed = redacted != *payload;
    (redacted, changed)
}

pub(super) const REDACTION_RULE_VERSION: &str = "configured-secret-redaction-v2-locator-gate";

/// A sanitized locator is not a newly discovered endpoint. Keep the rejected
/// row indices alongside the sanitized payload so replay cannot admit it.
pub(super) fn redact_page(
    payload: &Value,
    key: &str,
    proxy: &str,
    fields: &[String],
) -> (Value, bool, Vec<usize>) {
    let (redacted, changed) = redact_payload(payload, key, proxy);
    let mut rejected = Vec::new();
    if let (Some(before), Some(after)) = (
        payload.get("results").and_then(Value::as_array),
        redacted.get("results").and_then(Value::as_array),
    ) {
        for (index, (before, after)) in before.iter().zip(after).enumerate() {
            let altered = ["host", "link", "ip", "port", "protocol", "domain"]
                .iter()
                .any(|field| {
                    if before.is_object() {
                        before.get(*field) != after.get(*field)
                    } else if let Some(position) =
                        fields.iter().position(|name| name.as_str() == *field)
                    {
                        before.get(position) != after.get(position)
                    } else {
                        false
                    }
                });
            if altered {
                rejected.push(index);
            }
        }
    }
    (redacted, changed, rejected)
}

pub(super) fn field_restricted(detail: &str) -> bool {
    let lower = detail.to_lowercase();
    ["field", "字段", "会员", "权限"]
        .iter()
        .any(|value| lower.contains(value))
}

pub(super) fn provider_failure(payload: &Value, key: &str, proxy: &str) -> Failure {
    let detail = sanitize(
        payload
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or("FOFA 返回未知错误"),
        key,
        proxy,
    );
    let lower = detail.to_lowercase();
    let quota = [
        "额度",
        "余额",
        "次数不足",
        "quota",
        "credit",
        "rate limit",
        "频率",
    ]
    .iter()
    .any(|value| lower.contains(value));
    let authorization = [
        "api key",
        "apikey",
        "key无效",
        "key 无效",
        "key invalid",
        "key expired",
        "unauthorized",
        "鉴权",
        "认证失败",
    ]
    .iter()
    .any(|value| lower.contains(value));
    let transient = [
        "timeout",
        "超时",
        "内部错误",
        "服务暂时",
        "temporarily",
        "busy",
        "繁忙",
    ]
    .iter()
    .any(|value| lower.contains(value));
    Failure {
        category: if quota {
            "provider_quota"
        } else if authorization {
            "provider_auth"
        } else {
            "provider_error"
        },
        detail,
        retryable: transient || quota && !lower.contains("余额") && !lower.contains("credit"),
        stop_collection: quota || authorization,
        attempts: 0,
    }
}

fn network_failure(error: reqwest::Error, key: &str, proxy: &str) -> Failure {
    let mut causes = Vec::new();
    let mut source: Option<&dyn Error> = Some(&error);
    while let Some(cause) = source {
        let detail = sanitize(&cause.to_string(), key, proxy);
        if causes.last() != Some(&detail) {
            causes.push(detail);
        }
        if causes.len() >= 6 {
            break;
        }
        source = cause.source();
    }
    let detail = sanitize(&causes.join(" → "), key, proxy);
    let lower = detail.to_lowercase();
    let tls = lower.contains("certificate") || lower.contains("tls") || lower.contains("ssl");
    Failure {
        category: if error.is_timeout() {
            "timeout"
        } else if tls {
            "tls"
        } else if error.is_connect() {
            "connect"
        } else if error.is_body() {
            "response_body"
        } else {
            "request"
        },
        retryable: !tls
            && (error.is_timeout() || error.is_connect() || error.is_body() || error.is_request()),
        detail,
        stop_collection: tls,
        attempts: 0,
    }
}

pub(super) struct PageReceipt {
    pub(super) payload: Value,
    pub(super) attempts: usize,
    pub(super) received_at: String,
    pub(super) secrets_redacted: bool,
    pub(super) redacted_locator_rows: Vec<usize>,
}

pub(super) struct Transport {
    client: reqwest::Client,
    runtime: &'static tokio::runtime::Runtime,
    next_request: Instant,
    interval: Duration,
    key: String,
    proxy: String,
}

fn collection_runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(4)
                .thread_name("asset-collection-io")
                .enable_all()
                .build()
                .map_err(|error| format!("FOFA 请求运行环境初始化失败：{error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl Transport {
    pub(super) fn new(
        key: &str,
        proxy: &str,
        timeout: Duration,
        interval: Duration,
    ) -> Result<Self, String> {
        // A cancelled request must not drop a runtime and wait for a blocking
        // OS DNS lookup. Repeated recovery borrows this one process-wide,
        // bounded collection runtime instead of creating detached DNS pools.
        let runtime = collection_runtime()?;
        let client = {
            let _entered = runtime.enter();
            let mut builder = reqwest::Client::builder().timeout(timeout);
            if !proxy.trim().is_empty() {
                builder = builder.proxy(reqwest::Proxy::all(proxy.trim()).map_err(|error| {
                    format!("代理配置无效：{}", sanitize(&error.to_string(), key, proxy))
                })?);
            }
            builder
                .build()
                .map_err(|error| sanitize(&error.to_string(), key, proxy))?
        };
        Ok(Self {
            client,
            runtime,
            next_request: Instant::now(),
            interval,
            key: key.into(),
            proxy: proxy.into(),
        })
    }

    #[cfg(test)]
    pub(super) fn runtime_for_test(&self) -> &'static tokio::runtime::Runtime {
        self.runtime
    }

    pub(super) fn request(
        &mut self,
        endpoint: &reqwest::Url,
        cancel: &AtomicBool,
        mut on_retry: impl FnMut(&str),
    ) -> Result<PageReceipt, Failure> {
        for attempt in 0..3 {
            while Instant::now() < self.next_request {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Failure::cancelled());
                }
                std::thread::sleep(
                    self.next_request
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(25)),
                );
            }
            if cancel.load(Ordering::Relaxed) {
                return Err(Failure::cancelled());
            }
            self.next_request = Instant::now() + self.interval;
            let response = self.runtime.block_on(async {
                tokio::select! {
                    biased;
                    () = async {
                        while !cancel.load(Ordering::Relaxed) {
                            tokio::time::sleep(Duration::from_millis(25)).await;
                        }
                    } => Err(Failure::cancelled()),
                    result = self.request_once(endpoint) => result,
                }
            });
            match response {
                Ok((payload, secrets_redacted, received_at, redacted_locator_rows)) => {
                    return Ok(PageReceipt {
                        payload,
                        attempts: attempt + 1,
                        received_at,
                        secrets_redacted,
                        redacted_locator_rows,
                    })
                }
                Err(mut failure) => {
                    failure.attempts = attempt + 1;
                    if attempt == 2 || !failure.retryable || failure.category == "cancelled" {
                        return Err(failure);
                    }
                    on_retry(&format!(
                        "第 {}/3 次请求失败 [{}] {}",
                        attempt + 1,
                        failure.category,
                        failure.detail
                    ));
                    self.next_request = self
                        .next_request
                        .max(Instant::now() + Duration::from_secs(1 << attempt));
                }
            }
        }
        unreachable!("bounded FOFA retry loop always returns")
    }

    async fn request_once(
        &self,
        endpoint: &reqwest::Url,
    ) -> Result<(Value, bool, String, Vec<usize>), Failure> {
        let mut response = self
            .client
            .get(endpoint.clone())
            .header("User-Agent", "Authorized-Asset-Inventory/3.0")
            .send()
            .await
            .map_err(|error| network_failure(error, &self.key, &self.proxy))?;
        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            return Err(Failure {
                category: if code == 429 {
                    "http_rate_limit"
                } else if status.is_server_error() {
                    "http_server"
                } else if code == 407 {
                    "proxy_auth"
                } else if matches!(code, 401 | 403) {
                    "http_auth"
                } else {
                    "http_client"
                },
                detail: format!(
                    "FOFA HTTP {code} {}",
                    status.canonical_reason().unwrap_or("请求失败")
                ),
                retryable: code == 429 || status.is_server_error(),
                stop_collection: matches!(code, 401 | 403 | 407 | 429),
                attempts: 0,
            });
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| network_failure(error, &self.key, &self.proxy))?
        {
            if body.len().saturating_add(chunk.len()) > 64 * 1024 * 1024 {
                return Err(Failure {
                    category: "response_too_large",
                    detail: "FOFA 单页响应超过 64 MiB，未保存为完成证据".into(),
                    retryable: false,
                    stop_collection: true,
                    attempts: 0,
                });
            }
            body.extend_from_slice(&chunk);
        }
        let received_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let payload: Value = serde_json::from_slice(&body).map_err(|error| Failure {
            category: "response_json",
            detail: format!("FOFA 响应 JSON 解析失败：{error}"),
            retryable: true,
            stop_collection: false,
            attempts: 0,
        })?;
        let fields = endpoint
            .query_pairs()
            .find(|(key, _)| key == "fields")
            .map(|(_, value)| value.split(',').map(str::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        let (payload, secrets_redacted, redacted_locator_rows) =
            redact_page(&payload, &self.key, &self.proxy, &fields);
        if payload.get("error").and_then(Value::as_bool) == Some(true) {
            let failure = provider_failure(&payload, &self.key, &self.proxy);
            if failure.retryable && !field_restricted(&failure.detail) {
                return Err(failure);
            }
            return Ok((
                payload,
                secrets_redacted,
                received_at,
                redacted_locator_rows,
            ));
        }
        if !super::native_collection_resume::valid_page(&payload) {
            return Err(Failure {
                category: "response_shape",
                detail: "FOFA 响应缺少有效 results 数组，未保存为完成证据".into(),
                retryable: false,
                stop_collection: false,
                attempts: 0,
            });
        }
        Ok((
            payload,
            secrets_redacted,
            received_at,
            redacted_locator_rows,
        ))
    }
}
