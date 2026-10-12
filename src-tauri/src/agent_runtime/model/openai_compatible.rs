//! OpenAI-compatible gateway implementation (§5).
//!
//! Cloud: connect timeout, request timeout, one provider-level retry and stable
//! error classes. Local: no response timeout while a generation is running, one
//! in-flight request per process, cancellation drops the request itself (§6.2).
use super::admission;
use super::diagnostics::{StageObserver, TransportStage};
#[path = "openai_lifecycle.rs"]
mod lifecycle;
use super::gateway::{
    classify_status, looks_like_unsupported_tools, truncate, CancelToken, ModelError, ModelGateway,
    ModelRequest, ModelResponse, ToolCall,
};
use super::profile::GatewayProfile;
use super::transport::{self, CancelObservation, TransportError, TransportRequest};
use super::usage::{cached_usage_is_reported, tool_schema_hash, usage_from_response, usage_is_reported, usage_wire_is_unambiguous, ToolSchema};
use serde_json::Value as JsonValue;
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
pub const CLOUD_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
pub const PROVIDER_RETRY_WAIT: Duration = Duration::from_secs(2);
pub const USAGE_LEDGER_FILE: &str = "native-agent-usage.jsonl";

/// A durable caller may release a dispatch claim only for BeforeTransport.
/// Once `round` begins, even cancellation or setup failure is conservatively
/// treated as a possible provider effect.
#[derive(Debug)]
pub enum OneShotFailure {
    BeforeTransport(ModelError),
    TransportOutcomeUnknown(ModelError),
}

impl OneShotFailure {
    #[cfg(test)]
    pub fn into_error(self) -> ModelError {
        match self {
            Self::BeforeTransport(error) | Self::TransportOutcomeUnknown(error) => error,
        }
    }
}

pub struct OpenAiCompatibleGateway {
    profile: GatewayProfile,
    schema_hash: String,
    /// The last transport-level cancel, so the loop can persist
    /// `cancel_requested_at` / `transport_closed_at` with its terminal event.
    last_cancel: Mutex<Option<CancelObservation>>,
}

impl OpenAiCompatibleGateway {
    /// One reserved provider request. Durable child dispatchers own retries;
    /// they must not inherit the general gateway's implicit cloud retry.
    #[cfg(test)]
    pub fn complete_once(
        &self,
        request: &ModelRequest,
        cancel: &CancelToken,
    ) -> Result<ModelResponse, ModelError> {
        self.complete_once_observed(request, cancel)
            .map_err(OneShotFailure::into_error)
    }

    pub fn complete_once_observed(
        &self,
        request: &ModelRequest,
        cancel: &CancelToken,
    ) -> Result<ModelResponse, OneShotFailure> {
        self.complete_once_lifecycle(request, cancel, None)
    }

    pub fn new(profile: GatewayProfile, specs: &[ToolSchema]) -> Self {
        Self {
            schema_hash: tool_schema_hash(specs),
            profile,
            last_cancel: Mutex::new(None),
        }
    }

    // Phase 2/3 consumes this; the rules around it are covered by tests now.
    #[allow(dead_code)]
    pub fn schema_hash(&self) -> &str {
        &self.schema_hash
    }

    /// When a stop cut a request short, the timestamps the transport measured.
    /// Reading it consumes the record, so one cancel cannot describe two rounds.
    pub fn take_cancel_observation(&self) -> Option<CancelObservation> {
        self.last_cancel
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn request_body(&self, request: &ModelRequest) -> JsonValue {
        let mut body = serde_json::json!({
            "model": self.profile.model,
            "messages": request.messages,
            "temperature": request.temperature,
            "stream": false,
        });
        if let Some(max_output) = self.profile.max_output_tokens {
            body["max_tokens"] = serde_json::json!(max_output);
        }
        for (key, value) in self.profile.bounded_wire_options().as_object().unwrap() {
            body[key] = value.clone();
        }
        if !request.tools.is_empty() {
            body["tools"] = serde_json::json!(request
                .tools
                .iter()
                .map(ToolSchema::as_function_spec)
                .collect::<Vec<_>>());
            body["tool_choice"] = serde_json::json!("auto");
        }
        body
    }

    fn admitted_round(
        &self,
        body: &JsonValue,
        request: &ModelRequest,
        cancel: &CancelToken,
    ) -> Result<ModelResponse, ModelError> {
        let _permit =
            admission::gate_for(self.profile.local, &self.profile.endpoint).acquire(cancel)?;
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        self.round(body, request, cancel, None)
    }

    /// One request, cancellable end to end. When the token fires the future is
    /// dropped, so the response body and its connection go with it, and this only
    /// returns after that worker has settled (§6.2).
    fn exchange(
        &self,
        body: &JsonValue,
        cancel: &CancelToken,
        timeout: Option<Duration>,
        observer: Option<StageObserver>,
    ) -> Result<(u16, String, usize), ModelError> {
        let request_text = body.to_string();
        let request_bytes = request_text.len();
        let mut request =
            TransportRequest::post(self.profile.endpoint.clone(), request_text.into_bytes())
                .header("content-type", "application/json");
        if let Some(key) = self.profile.bearer() {
            request = request.header("authorization", format!("Bearer {key}"));
        }
        request.connect_timeout = CONNECT_TIMEOUT;
        // Local generations may legitimately run for many minutes; the only bound
        // on them is cancellation, which now really cancels them.
        request.total_timeout = if self.profile.local {
            None
        } else {
            Some(timeout.unwrap_or(CLOUD_REQUEST_TIMEOUT))
        };
        request.proxy = self.profile.proxy.clone();
        let token = cancel.clone();
        let sent = observer.clone();
        let outcome = transport::send_observed(
            request,
            Arc::new(move || token.is_cancelled()),
            Box::new(|_| true),
            Box::new(move || { if let Some(observer) = sent { observer(TransportStage::Sent); } }),
        );
        match outcome {
            Ok(reply) => {
                if let Some(observer) = observer { observer(TransportStage::ResponseReceived); }
                Ok((reply.status, String::from_utf8_lossy(&reply.body).to_string(), request_bytes))
            }
            Err(error) => {
                if let TransportError::Cancelled(observation) = &error {
                    if let Ok(mut slot) = self.last_cancel.lock() {
                        *slot = Some(observation.clone());
                    }
                }
                Err(match error {
                    TransportError::Setup(message) => {
                        if message.contains("代理") {
                            ModelError::Authentication(message)
                        } else {
                            ModelError::Network(message)
                        }
                    }
                    TransportError::Network(message) => ModelError::Network(message),
                    TransportError::Timeout(message) => ModelError::Timeout(message),
                    TransportError::Interrupted(message) => ModelError::Network(message),
                    TransportError::Cancelled(_) => ModelError::Cancelled,
                })
            }
        }
    }

    /// Per-request usage line, kept beside the target so a run stays auditable
    /// even when the loop dies before its terminal event.
    pub fn record_usage_line(&self, target_dir: &Path, response: &ModelResponse) {
        let line = serde_json::json!({
            "at": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            "model": self.profile.model,
            "deployment": if self.profile.local { "local" } else { "cloud" },
            "schemaHash": self.schema_hash,
            "finishReason": response.finish_reason,
            "toolCalls": response.tool_calls.len(),
            "usage": response.usage.as_json(),
            "usageReported": response.usage_reported,
            "cachedUsageReported": response.cached_usage_reported,
            "usageBasis": if response.usage_reported { "provider_reported" } else { "unverified" },
        });
        let Ok(mut encoded) = serde_json::to_vec(&line) else { return; };
        encoded.push(b'\n');
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(target_dir.join(USAGE_LEDGER_FILE))
        {
            // JSON Display emits multiple writes. Encode one complete frame,
            // and serialize appenders (including short writes) on this file.
            // Closing this distinct handle releases its advisory lock.
            // This secondary diagnostic must not indefinitely delay the
            // caller's durable cost receipt when a different writer is stuck.
            let deadline = Instant::now() + Duration::from_millis(100);
            let locked = loop {
                match file.try_lock() {
                    Ok(()) => break true,
                    Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => break false,
                }
            };
            if locked { let _ = file.write_all(&encoded); }
        }
    }
}

impl ModelGateway for OpenAiCompatibleGateway {
    fn profile(&self) -> &GatewayProfile {
        &self.profile
    }

    fn complete(
        &self,
        request: &ModelRequest,
        cancel: &CancelToken,
    ) -> Result<ModelResponse, ModelError> {
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let body = self.request_body(request);
        if self.profile.local {
            return self.admitted_round(&body, request, cancel);
        }
        match self.admitted_round(&body, request, cancel) {
            Err(error) if error.retryable() => {
                wait_or_cancel(cancel)?;
                self.admitted_round(&body, request, cancel)
            }
            result => result,
        }
    }
}

/// The single provider-level retry wait, interruptible: a stop must not be
/// followed by another stretch of waiting before it takes effect.
fn wait_or_cancel(cancel: &CancelToken) -> Result<(), ModelError> {
    let deadline = Instant::now() + PROVIDER_RETRY_WAIT;
    loop {
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(remaining.min(Duration::from_millis(100)));
    }
}

impl OpenAiCompatibleGateway {
    fn round(
        &self,
        body: &JsonValue,
        request: &ModelRequest,
        cancel: &CancelToken,
        observer: Option<StageObserver>,
    ) -> Result<ModelResponse, ModelError> {
        let (status, text, request_bytes) = self.exchange(body, cancel, request.request_timeout, observer)?;
        if !(200..300).contains(&status) {
            return Err(classify_status(status, &text));
        }
        let parsed: JsonValue = serde_json::from_str(&text).map_err(|error| {
            ModelError::Protocol(format!("{error}；响应前 200 字：{}", truncate(&text, 200)))
        })?;
        if parsed.get("error").is_some() && status == 200 {
            // Gateways that answer 200 with an error body (common on proxies).
            let message = parsed
                .pointer("/error/message")
                .and_then(JsonValue::as_str)
                .unwrap_or_default();
            if looks_like_unsupported_tools(400, message) {
                return Err(ModelError::UnsupportedCapability(truncate(message, 300)));
            }
        }
        let unambiguous_usage = usage_wire_is_unambiguous(&text);
        Ok(ModelResponse {
            text: extract_message_text(&parsed),
            tool_calls: extract_tool_calls(&parsed),
            usage: usage_from_response(&parsed, request_bytes, text.len()),
            usage_reported: usage_is_reported(&parsed) && unambiguous_usage,
            cached_usage_reported: cached_usage_is_reported(&parsed) && unambiguous_usage,
            finish_reason: parsed
                .pointer("/choices/0/finish_reason")
                .and_then(JsonValue::as_str)
                .unwrap_or("stop")
                .to_string(),
        })
    }
}

/// `content` is a string on most servers and an array of parts on some.
pub fn extract_message_text(body: &JsonValue) -> String {
    let content = match body.pointer("/choices/0/message/content") {
        Some(value) => value,
        None => return String::new(),
    };
    match content {
        JsonValue::String(text) => text.clone(),
        JsonValue::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
                    .or_else(|| part.as_str().map(str::to_string))
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

pub fn extract_tool_calls(body: &JsonValue) -> Vec<ToolCall> {
    let Some(calls) = body
        .pointer("/choices/0/message/tool_calls")
        .and_then(JsonValue::as_array)
    else {
        return Vec::new();
    };
    calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let raw = call
                .pointer("/function/arguments")
                .and_then(JsonValue::as_str)
                .unwrap_or_default();
            let arguments = match serde_json::from_str::<JsonValue>(raw) {
                Ok(JsonValue::Object(map)) => JsonValue::Object(map),
                _ if raw.trim().is_empty() => serde_json::json!({}),
                _ => serde_json::json!({"__parseError": truncate(raw, 2_000)}),
            };
            ToolCall {
                id: call
                    .get("id")
                    .and_then(JsonValue::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("call_{index}")),
                name: call
                    .pointer("/function/name")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_string(),
                arguments,
            }
        })
        .collect()
}
