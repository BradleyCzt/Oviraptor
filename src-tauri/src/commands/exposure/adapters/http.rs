// §Q2 — the only HTTP door the passive exposure surface has. Auto-redirect is off and each
// hop is re-authorized; every outbound attempt (including the failed ones) is charged to
// the deterministic request and byte budget before it is sent, so a cancelled or
// revoking run leaves zero new requests rather than "one more, then we stop".

use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, LOCATION};
use std::time::{Duration, Instant};

use crate::commands::exposure::redaction::{content_hash, redact_text};
use crate::commands::exposure::scope::{
    destination_addresses, is_private_address, ExposureDenial, ExposureScope,
};
use crate::commands::exposure::state::utc_now;

pub const DEFAULT_USER_AGENT: &str = "Oviraptor-Exposure/1.1 (+passive-public-evidence)";

#[derive(Clone, Debug)]
pub struct PassiveHttpLimits {
    pub timeout: Duration,
    pub max_hops: usize,
    pub user_agent: String,
    /// Per-response ceiling; the run-wide ceiling stays on the meter.
    pub max_response_bytes: usize,
    /// A proxy configured by the operator; never chosen by a discovered URL.
    pub proxy: Option<String>,
}

impl Default for PassiveHttpLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(12),
            max_hops: 3,
            max_response_bytes: 1_000_000,
            user_agent: DEFAULT_USER_AGENT.to_string(),
            proxy: None,
        }
    }
}

/// The meter is the plan's "确定性配额": charged before the socket is opened, so a
/// failure still costs, and a refusal costs nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestMeter {
    pub requests_used: i64,
    pub bytes_used: i64,
    pub max_requests: i64,
    pub max_bytes: i64,
}

impl RequestMeter {
    pub fn for_scope(scope: &ExposureScope) -> Self {
        Self {
            requests_used: 0,
            bytes_used: 0,
            max_requests: scope.max_requests,
            max_bytes: scope.max_total_bytes,
        }
    }

    pub fn charge_request(&mut self) -> Result<(), ExposureDenial> {
        if self.requests_used >= self.max_requests {
            return Err(ExposureDenial::new(
                "request_budget_exhausted",
                format!(
                    "本轮 {} 次请求额度已用尽，剩余工作项记为超额而不是完成",
                    self.max_requests
                ),
            ));
        }
        self.requests_used += 1;
        Ok(())
    }

    pub fn charge_bytes(&mut self, bytes: i64) -> Result<(), ExposureDenial> {
        if self.bytes_used.saturating_add(bytes) > self.max_bytes {
            return Err(ExposureDenial::new(
                "byte_budget_exhausted",
                format!(
                    "本轮 {} 字节额度已用尽（已用 {}）",
                    self.max_bytes, self.bytes_used
                ),
            ));
        }
        self.bytes_used += bytes;
        Ok(())
    }

    pub fn remaining_requests(&self) -> i64 {
        (self.max_requests - self.requests_used).max(0)
    }
}

#[derive(Clone, Debug)]
pub struct PassiveFetch {
    pub requested_url: String,
    pub final_url: String,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub truncated: bool,
    pub hops: Vec<String>,
    /// `permission` (401/403), `challenge`, `rate_limited` or empty. A boundary is never
    /// written as "安全": it says the target refused, not that nothing is exposed.
    pub boundary: String,
    pub content_hash: String,
    pub headers_hash: String,
    pub requests_charged: i64,
    pub bytes_charged: i64,
    pub elapsed_ms: u64,
    pub observed_at: String,
    /// G2: the few response headers a provider adapter is allowed to read (rate limit,
    /// Retry-After, pagination, redirect target), already redacted. Everything else is hashed.
    pub provider_headers: Vec<(String, String)>,
}

impl PassiveFetch {
    pub fn provider_header(&self, name: &str) -> Option<&str> {
        self.provider_headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// G2: response headers copied into `PassiveFetch::provider_headers`.
pub const PROVIDER_RESPONSE_HEADERS: &[&str] = &[
    "link",
    "location",
    "retry-after",
    "x-ratelimit-limit",
    "x-ratelimit-remaining",
    "x-ratelimit-reset",
    "x-ratelimit-resource",
];

/// G2: an Authorization value bound to exactly one provider origin. It has no `Display`, its
/// `Debug` never prints the value, and `fetch_provider_api` refuses to send it anywhere else.
pub struct ProviderAuthorization {
    origin: String,
    value: String,
}

impl ProviderAuthorization {
    pub fn bearer(origin: &str, token: &str) -> Self {
        Self {
            origin: origin.trim_end_matches('/').to_ascii_lowercase(),
            value: format!("Bearer {token}"),
        }
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Removes the secret from any text that is about to be stored or shown.
    pub fn scrub(&self, text: &str) -> String {
        let secret = self.value.trim_start_matches("Bearer ");
        if secret.is_empty() {
            return text.to_string();
        }
        text.replace(secret, "[已脱敏]")
    }
}

impl std::fmt::Debug for ProviderAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "ProviderAuthorization {{ origin: {}, value: <redacted> }}", self.origin)
    }
}

#[derive(Clone, Debug)]
pub enum FetchBlock {
    /// The request was refused before it left the process.
    Denied(ExposureDenial),
    /// A hop left the authorized scope; the walk stops and the observation is kept.
    RedirectStopped {
        from: String,
        location: String,
        hops: Vec<String>,
        denial: ExposureDenial,
    },
    Transport {
        message: String,
        hops: Vec<String>,
    },
    Cancelled,
}

impl FetchBlock {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Denied(denial) => denial.code,
            Self::RedirectStopped { denial, .. } => denial.code,
            Self::Transport { .. } => "transport_failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Denied(denial) | Self::RedirectStopped { denial, .. } => denial.message.clone(),
            Self::Transport { message, .. } => message.clone(),
            Self::Cancelled => "任务已取消，未再发出请求".to_string(),
        }
    }
}

pub struct PassiveClient {
    client: Client,
    limits: PassiveHttpLimits,
}

impl PassiveClient {
    pub fn new(limits: PassiveHttpLimits) -> Result<Self, String> {
        let mut builder = Client::builder()
            .timeout(limits.timeout)
            // Auto-follow is the classic way to escape an approved origin.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(limits.user_agent.clone())
            .gzip(true)
            .brotli(true);
        if let Some(proxy) = limits
            .proxy
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|error| format!("暴露面代理配置无效：{error}"))?,
            );
        }
        Ok(Self {
            client: builder.build().map_err(|error| error.to_string())?,
            limits,
        })
    }

    /// One authorized GET against an approved origin, hop by hop.
    pub fn fetch(
        &self,
        scope: &ExposureScope,
        url: &reqwest::Url,
        meter: &mut RequestMeter,
        revalidate: &dyn Fn() -> Result<(), ExposureDenial>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PassiveFetch, FetchBlock> {
        let started = Instant::now();
        let mut hops = Vec::new();
        let mut current = url.clone();
        let requested_url = url.to_string();
        for hop in 0..=self.limits.max_hops {
            if cancelled() {
                return Err(FetchBlock::Cancelled);
            }
            // Re-checked per request: a revocation, an archived project or a removed
            // asset must stop the very next send, not the next run.
            revalidate().map_err(FetchBlock::Denied)?;
            if hop == 0 {
                scope
                    .authorize(&current, &utc_now())
                    .map_err(FetchBlock::Denied)?;
            }
            let charged = meter.requests_used;
            meter.charge_request().map_err(FetchBlock::Denied)?;
            let response = match self.client.get(current.clone()).send() {
                Ok(value) => value,
                Err(error) => {
                    return Err(FetchBlock::Transport {
                        message: redact_text(&error.to_string()),
                        hops,
                    })
                }
            };
            let status = response.status();
            let headers = response.headers().clone();
            if status.is_redirection() {
                let Some(location) = redirect_target(&current, &headers) else {
                    return Err(FetchBlock::Transport {
                        message: format!("{status} 重定向没有 Location"),
                        hops,
                    });
                };
                let from = current.to_string();
                match scope.authorize_redirect(&current, &location, &utc_now()) {
                    Ok(_) => {
                        hops.push(from);
                        current = location;
                        continue;
                    }
                    Err(denial) => {
                        return Err(FetchBlock::RedirectStopped {
                            from,
                            location: location.to_string(),
                            hops,
                            denial,
                        })
                    }
                }
            }
            return self.finish(
                response,
                current,
                requested_url,
                hops,
                meter,
                charged,
                status,
                headers,
                started,
            );
        }
        Err(FetchBlock::RedirectStopped {
            from: current.to_string(),
            location: String::new(),
            hops,
            denial: ExposureDenial::new(
                "redirect_hop_limit",
                format!("重定向超过 {} 跳，已停止", self.limits.max_hops),
            ),
        })
    }

    /// Public index endpoints (certificate transparency, web archive) are source
    /// endpoints: reaching them never authorizes a newly discovered target, so the
    /// origin must be on the configured endpoint list.
    pub fn fetch_source_endpoint(
        &self,
        url: &reqwest::Url,
        allowed_origins: &[String],
        meter: &mut RequestMeter,
        allow_private: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PassiveFetch, FetchBlock> {
        if cancelled() {
            return Err(FetchBlock::Cancelled);
        }
        let origin = url.origin().ascii_serialization();
        if !allowed_origins
            .iter()
            .any(|row| row.trim_end_matches('/').eq_ignore_ascii_case(&origin))
        {
            return Err(FetchBlock::Denied(ExposureDenial::new(
                "source_endpoint_not_configured",
                format!("{origin} 不在配置的公共索引端点内，已拒绝访问"),
            )));
        }
        let addresses = destination_addresses(url).map_err(FetchBlock::Denied)?;
        if !allow_private && addresses.iter().any(is_private_address) {
            return Err(FetchBlock::Denied(ExposureDenial::new(
                "source_endpoint_private_denied",
                format!("{origin} 解析到内网地址；测试镜像必须显式配置允许"),
            )));
        }
        let started = Instant::now();
        let requested_url = url.to_string();
        let charged = meter.requests_used;
        meter.charge_request().map_err(FetchBlock::Denied)?;
        let response =
            self.client
                .get(url.clone())
                .send()
                .map_err(|error| FetchBlock::Transport {
                    message: redact_text(&error.to_string()),
                    hops: Vec::new(),
                })?;
        let status = response.status();
        let headers = response.headers().clone();
        self.finish(
            response,
            url.clone(),
            requested_url,
            Vec::new(),
            meter,
            charged,
            status,
            headers,
            started,
        )
    }

    /// G2 — a third-party provider API (GitHub). Same gate as `fetch_source_endpoint` (the
    /// origin must be configured, private addresses need the explicit switch, every attempt is
    /// charged first), plus fixed request headers and an optional origin-bound Authorization.
    /// Redirects are never followed (the client has no redirect policy): a 3xx comes back as a
    /// status with its Location in `provider_headers`, for the adapter to record as a gap.
    #[allow(clippy::too_many_arguments)]
    pub fn fetch_provider_api(
        &self,
        url: &reqwest::Url,
        allowed_origins: &[String],
        headers: &[(&str, &str)],
        authorization: Option<&ProviderAuthorization>,
        meter: &mut RequestMeter,
        allow_private: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<PassiveFetch, FetchBlock> {
        if cancelled() {
            return Err(FetchBlock::Cancelled);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(FetchBlock::Denied(ExposureDenial::new(
                "provider_userinfo_denied",
                "提供方地址带用户信息，已拒绝".to_string(),
            )));
        }
        if !matches!(url.scheme(), "https" | "http") {
            return Err(FetchBlock::Denied(ExposureDenial::new(
                "provider_scheme_denied",
                format!("提供方地址协议 {} 不允许", url.scheme()),
            )));
        }
        let origin = url.origin().ascii_serialization();
        if !allowed_origins
            .iter()
            .any(|row| row.trim_end_matches('/').eq_ignore_ascii_case(&origin))
        {
            return Err(FetchBlock::Denied(ExposureDenial::new(
                "provider_endpoint_not_configured",
                format!("{origin} 不在配置的提供方端点内，已拒绝访问"),
            )));
        }
        if let Some(secret) = authorization {
            if !secret.origin().eq_ignore_ascii_case(&origin) {
                // Never "send it without the token": a credentialed request to another origin
                // is refused outright, before anything is charged or sent.
                return Err(FetchBlock::Denied(ExposureDenial::new(
                    "provider_credential_origin_mismatch",
                    format!("凭据只属于 {}，不会发往 {origin}", secret.origin()),
                )));
            }
        }
        let addresses = destination_addresses(url).map_err(FetchBlock::Denied)?;
        if !allow_private && addresses.iter().any(is_private_address) {
            return Err(FetchBlock::Denied(ExposureDenial::new(
                "provider_endpoint_private_denied",
                format!("{origin} 解析到内网地址；测试镜像必须显式配置允许"),
            )));
        }
        let started = Instant::now();
        let requested_url = url.to_string();
        let charged = meter.requests_used;
        meter.charge_request().map_err(FetchBlock::Denied)?;
        let mut request = self.client.get(url.clone());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        if let Some(secret) = authorization {
            let mut value = reqwest::header::HeaderValue::from_str(&secret.value).map_err(|_| {
                FetchBlock::Denied(ExposureDenial::new(
                    "provider_credential_invalid",
                    "凭据含非法字符，未发送".to_string(),
                ))
            })?;
            value.set_sensitive(true);
            request = request.header(reqwest::header::AUTHORIZATION, value);
        }
        let scrub = |text: String| match authorization {
            Some(secret) => secret.scrub(&redact_text(&text)),
            None => redact_text(&text),
        };
        let response = request.send().map_err(|error| FetchBlock::Transport {
            message: scrub(error.to_string()),
            hops: Vec::new(),
        })?;
        let status = response.status();
        let response_headers = response.headers().clone();
        self.finish(
            response,
            url.clone(),
            requested_url,
            Vec::new(),
            meter,
            charged,
            status,
            response_headers,
            started,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        &self,
        response: Response,
        current: reqwest::Url,
        requested_url: String,
        mut hops: Vec<String>,
        meter: &mut RequestMeter,
        charged_requests: i64,
        status: reqwest::StatusCode,
        headers: HeaderMap,
        started: Instant,
    ) -> Result<PassiveFetch, FetchBlock> {
        hops.push(current.to_string());
        let content_type = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_lowercase();
        let cap = self.limits.max_response_bytes;
        let mut body = Vec::new();
        use std::io::Read;
        // One extra byte tells "exactly at the cap" apart from "cut off at the cap".
        if let Err(error) = response.take(cap as u64 + 1).read_to_end(&mut body) {
            return Err(FetchBlock::Transport {
                message: redact_text(&error.to_string()),
                hops,
            });
        }
        let truncated = body.len() > cap;
        if truncated {
            body.truncate(cap);
        }
        let bytes = body.len() as i64;
        meter.charge_bytes(bytes).map_err(FetchBlock::Denied)?;
        let text = String::from_utf8_lossy(&body).to_ascii_lowercase();
        let boundary = if matches!(status.as_u16(), 401 | 403) {
            "permission"
        } else if status.as_u16() == 429 || status.as_u16() == 503 {
            "rate_limited"
        } else if [
            "captcha",
            "cf-browser-verification",
            "just a moment",
            "access denied",
        ]
        .iter()
        .any(|marker| text.contains(marker))
        {
            "challenge"
        } else {
            ""
        };
        let provider_headers = PROVIDER_RESPONSE_HEADERS
            .iter()
            .filter_map(|name| {
                headers
                    .get(*name)
                    .and_then(|value| value.to_str().ok())
                    .map(|value| (name.to_string(), redact_text(value)))
            })
            .collect();
        Ok(PassiveFetch {
            provider_headers,
            requested_url,
            final_url: current.to_string(),
            status: status.as_u16(),
            content_type,
            truncated,
            headers_hash: headers_of(&headers),
            content_hash: content_hash(&body),
            body,
            hops,
            boundary: boundary.to_string(),
            requests_charged: charged_requests.max(0) + 1,
            bytes_charged: bytes,
            elapsed_ms: started.elapsed().as_millis() as u64,
            observed_at: utc_now(),
        })
    }
}

fn redirect_target(current: &reqwest::Url, headers: &HeaderMap) -> Option<reqwest::Url> {
    let raw = headers.get(LOCATION)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    current.join(raw).ok()
}

fn headers_of(headers: &HeaderMap) -> String {
    let mut rows: Vec<String> = headers
        .iter()
        .map(|(name, value)| {
            let text = value.to_str().unwrap_or("<binary>");
            format!("{}={}", name.as_str().to_lowercase(), redact_text(text))
        })
        .collect();
    rows.sort();
    content_hash(rows.join("\n").as_bytes())
}
