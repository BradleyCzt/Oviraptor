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
        Self {
            category: "checkpoint_io",
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
                Ok(payload) => {
                    return Ok(PageReceipt {
                        payload,
                        attempts: attempt + 1,
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

    async fn request_once(&self, endpoint: &reqwest::Url) -> Result<Value, Failure> {
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
        let payload: Value = serde_json::from_slice(&body).map_err(|error| Failure {
            category: "response_json",
            detail: format!("FOFA 响应 JSON 解析失败：{error}"),
            retryable: true,
            stop_collection: false,
            attempts: 0,
        })?;
        if payload.get("error").and_then(Value::as_bool) == Some(true) {
            let failure = provider_failure(&payload, &self.key, &self.proxy);
            if failure.retryable && !field_restricted(&failure.detail) {
                return Err(failure);
            }
            return Ok(payload);
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
        Ok(payload)
    }
}
