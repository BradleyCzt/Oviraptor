//! Gateway contract, request/response shapes, error classes and cancellation.
use super::usage::{ToolSchema, UsageDelta};
use serde_json::Value as JsonValue;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct ModelRequest {
    pub messages: Vec<JsonValue>,
    pub tools: Vec<ToolSchema>,
    pub temperature: f64,
    pub request_timeout: Option<Duration>,
}

impl Default for ModelRequest {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            tools: Vec::new(),
            temperature: 0.0,
            request_timeout: None,
        }
    }
}

impl ModelRequest {
    pub fn new(messages: Vec<JsonValue>, tools: Vec<ToolSchema>) -> Self {
        Self {
            messages,
            tools,
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: JsonValue,
}

#[derive(Clone, Debug, Default)]
pub struct ModelResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: UsageDelta,
    /// Provider supplied a complete, consistent breakdown. Otherwise usage is
    /// only an estimate and cannot close an append-only billing obligation.
    pub usage_reported: bool,
    /// The provider explicitly reported a consistent cache subset. An absent
    /// cache field retains legacy token compatibility but cannot prove zero
    /// cache usage for a rate card with distinct input prices.
    pub cached_usage_reported: bool,
    pub finish_reason: String,
}

impl ModelResponse {
    /// One round may carry a group of actions but never duplicates (§6 rule 4).
    pub fn deduplicated_tool_calls(&self) -> Vec<ToolCall> {
        let mut seen = Vec::new();
        let mut calls = Vec::new();
        for call in &self.tool_calls {
            let key = format!("{}:{}", call.name, call.arguments);
            if !seen.contains(&key) {
                seen.push(key);
                calls.push(call.clone());
            }
        }
        calls
    }
}

/// Stable error classes (§5). The class decides retry, fallback and terminal
/// state; the message stays for the audit log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelError {
    /// The endpoint cannot do tool calling. Only this may switch an `auto` run
    /// to another backend, and only before any token was spent.
    UnsupportedCapability(String),
    Authentication(String),
    ContextOverflow(String),
    RateLimited(String),
    Server {
        status: u16,
        message: String,
    },
    Network(String),
    Timeout(String),
    Protocol(String),
    Cancelled,
}

impl ModelError {
    // Phase 2/3 consumes this; the rules around it are covered by tests now.
    #[allow(dead_code)]
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedCapability(_) => "unsupported_capability",
            Self::Authentication(_) => "authentication",
            Self::ContextOverflow(_) => "context_overflow",
            Self::RateLimited(_) => "persistent_rate_limit",
            Self::Server { .. } => "model_provider_failure",
            Self::Network(_) => "model_network",
            Self::Timeout(_) => "model_timeout",
            Self::Protocol(_) => "model_protocol",
            Self::Cancelled => "user_cancelled",
        }
    }

    /// Cloud transport trouble gets exactly one provider-level retry (§5).
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited(_)
                | Self::Network(_)
                | Self::Timeout(_)
                | Self::Server {
                    status: 500..=599,
                    ..
                }
        )
    }

    pub fn detail(&self) -> String {
        match self {
            Self::UnsupportedCapability(message) => {
                format!("unsupported_capability:{message}")
            }
            Self::Authentication(message) => format!("模型认证失败：{message}"),
            Self::ContextOverflow(message) => format!("模型上下文窗口不足：{message}"),
            Self::RateLimited(message) => format!("模型限流 HTTP 429：{message}"),
            Self::Server { status, message } => format!("模型 HTTP {status}：{message}"),
            Self::Network(message) => format!("模型连接失败：{message}"),
            Self::Timeout(message) => format!("模型响应超时：{message}"),
            Self::Protocol(message) => format!("模型响应无法解析：{message}"),
            Self::Cancelled => "用户已停止任务".to_string(),
        }
    }
}

pub fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut cut = text.chars().take(limit).collect::<String>();
    cut.push('…');
    cut
}

pub fn classify_status(status: u16, body: &str) -> ModelError {
    if looks_like_unsupported_tools(status, body) {
        return ModelError::UnsupportedCapability(truncate(body, 300));
    }
    if looks_like_context_overflow(status, body) {
        return ModelError::ContextOverflow(truncate(body, 400));
    }
    match status {
        401 | 403 => ModelError::Authentication(truncate(body, 300)),
        429 => ModelError::RateLimited(truncate(body, 300)),
        other => ModelError::Server {
            status: other,
            message: truncate(body, 300),
        },
    }
}

/// A local window guard is reported as a 4xx about context size.
pub fn looks_like_context_overflow(status: u16, text: &str) -> bool {
    if !(400..=499).contains(&status) {
        return false;
    }
    let lowered = text.to_ascii_lowercase();
    (lowered.contains("context") || lowered.contains("num_ctx") || lowered.contains("maximum"))
        && (lowered.contains("token") || lowered.contains("window") || lowered.contains("ctx"))
}

/// Providers that cannot do function calling say so explicitly.
pub fn looks_like_unsupported_tools(status: u16, text: &str) -> bool {
    if !matches!(status, 400 | 404 | 422) {
        return false;
    }
    let lowered = text.to_ascii_lowercase();
    (lowered.contains("tool") || lowered.contains("function"))
        && [
            "not support",
            "unsupported",
            "not allowed",
            "unknown",
            "invalid parameter",
        ]
        .iter()
        .any(|needle| lowered.contains(needle))
}

/// A storage-agnostic "should I stop?" probe the caller refreshes per check.
type ExternalChecker = Arc<dyn Fn() -> bool + Send + Sync>;

/// A non-blocking cancellation handle. The gateway polls it while a request is
/// in flight so a long local generation can still be stopped (§5).
#[derive(Clone)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
    external: Arc<Mutex<Option<ExternalChecker>>>,
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            external: Arc::new(Mutex::new(None)),
        }
    }

    pub fn from_checker(check: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        let token = Self::new();
        if let Ok(mut slot) = token.external.lock() {
            *slot = Some(Arc::new(check));
        }
        token
    }

    // Phase 2/3 consumes this; the rules around it are covered by tests now.
    #[allow(dead_code)]
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        if self.cancelled.load(Ordering::SeqCst) {
            return true;
        }
        let checker = self.external.lock().ok().and_then(|slot| slot.clone());
        match checker {
            Some(check) => check(),
            None => false,
        }
    }
}

pub trait ModelGateway: Send + Sync {
    fn complete(
        &self,
        request: &ModelRequest,
        cancel: &CancelToken,
    ) -> Result<ModelResponse, ModelError>;

    fn profile(&self) -> &super::profile::GatewayProfile;
}
