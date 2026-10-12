// Phase 1 gateway tests. Everything runs against an in-process mock provider;
// no real endpoint and no Strix process is involved.
use super::context::{compact_messages, digest_text, ContextLayers};
use super::gateway::{
    classify_status, looks_like_context_overflow, looks_like_unsupported_tools, CancelToken,
    ModelError, ModelGateway, ModelRequest,
};
use super::openai_compatible::{extract_message_text, extract_tool_calls, OpenAiCompatibleGateway};
use super::profile::{GatewayProfile, LocalResourcePolicy, ResolvedModelProfile};
use super::usage::{estimate_tokens, tool_schema_hash, usage_from_response, ToolSchema};
use serde_json::Value as JsonValue;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

#[path = "admission_tests.rs"]
mod admission_tests;

/// A scripted OpenAI-compatible endpoint. Each connection is served on its own
/// thread and every request body is recorded, so assertions stay on the test
/// thread instead of panicking inside a mock.
struct MockProvider {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}

impl MockProvider {
    fn spawn(handler: impl Fn(usize, &str) -> (u16, String) + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let hits = Arc::new(AtomicUsize::new(0));
        let handler = Arc::new(handler);
        let (loop_stop, loop_requests, loop_hits, loop_handler) =
            (stop.clone(), requests.clone(), hits.clone(), handler);
        thread::spawn(move || {
            while !loop_stop.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                };
                // macOS hands the accepted socket the listener's O_NONBLOCK flag;
                // without clearing it a read that wins the race against the client
                // returns WouldBlock and the request is silently dropped.
                let _ = stream.set_nonblocking(false);
                let (requests, stop, hits) =
                    (loop_requests.clone(), loop_stop.clone(), loop_hits.clone());
                let handler = loop_handler.clone();
                thread::spawn(move || {
                    let Some(request) = read_request(&mut stream) else {
                        return;
                    };
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let index = hits.fetch_add(1, Ordering::SeqCst);
                    let (status, body) = handler(index, &request);
                    requests.lock().unwrap().push(request);
                    let response = format!(
                        "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                });
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            requests,
            hits,
            stop,
        }
    }

    fn bodies(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(size) => {
                buffer.extend_from_slice(&chunk[..size]);
                let text = String::from_utf8_lossy(&buffer).to_string();
                let Some(index) = text.find("\r\n\r\n") else {
                    continue;
                };
                let want = text[..index]
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|rest| rest.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                if buffer.len() >= index + 4 + want {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    if buffer.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&buffer).to_string())
}

fn completion(text: &str, calls: &[(&str, JsonValue)], total_tokens: i64) -> String {
    let list: Vec<JsonValue> = calls
        .iter()
        .enumerate()
        .map(|(index, (name, arguments))| {
            serde_json::json!({
                "id": format!("c{index}"),
                "type": "function",
                "function": {"name": name, "arguments": arguments.to_string()}
            })
        })
        .collect();
    serde_json::json!({
        "choices": [{
            "message": {"content": text, "tool_calls": list},
            "finish_reason": if list.is_empty() { "stop" } else { "tool_calls" }
        }],
        "usage": {
            "prompt_tokens": total_tokens - 20,
            "completion_tokens": 20,
            "total_tokens": total_tokens,
            "prompt_tokens_details": {"cached_tokens": total_tokens / 4}
        }
    })
    .to_string()
}

fn raw(body: String) -> (u16, String) {
    (200, body)
}

fn profile(base_url: &str, local: bool) -> GatewayProfile {
    GatewayProfile::resolve(
        &ResolvedModelProfile {
            // the caller strips the provider prefix, exactly as the existing
            // profile resolver does before handing the gateway a name
            model: "mock-model".into(),
            api_base: base_url.to_string(),
            api_key: if local {
                String::new()
            } else {
                "cloud-key".into()
            },
            deployment: if local { "local" } else { "cloud" }.into(),
        },
        &LocalResourcePolicy {
            max_output_tokens: if local { Some(2_048) } else { None },
            max_context_tokens: if local { 32_768 } else { 0 },
        },
        None,
    )
    .unwrap()
}

fn specs() -> Vec<ToolSchema> {
    vec![ToolSchema::new(
        "http.replay",
        "Replay one captured request.",
        serde_json::json!({
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"],
            "additionalProperties": false
        }),
    )]
}

fn gateway(base_url: &str, local: bool) -> OpenAiCompatibleGateway {
    OpenAiCompatibleGateway::new(profile(base_url, local), &specs())
}

#[test]
fn cloud_round_trip_sends_tools_and_parses_usage() {
    let provider = MockProvider::spawn(|_, _| {
        raw(completion(
            "",
            &[(
                "http.replay",
                serde_json::json!({"url": "https://app.example.invalid/api/orders"}),
            )],
            1_000,
        ))
    });
    let client = gateway(&provider.base_url, false);
    let request = ModelRequest::new(
        vec![serde_json::json!({"role": "user", "content": "investigate"})],
        specs(),
    );
    let response = client
        .complete(&request, &CancelToken::new())
        .expect("mock round must succeed");
    assert_eq!(response.tool_calls.len(), 1);
    assert_eq!(response.tool_calls[0].name, "http.replay");
    assert_eq!(response.tool_calls[0].id, "c0");
    assert_eq!(response.usage.total_tokens, 1_000);
    assert_eq!(response.usage.model_requests, 1);
    assert_eq!(response.finish_reason, "tool_calls");
    assert!(!client.profile().local);
    assert_eq!(client.schema_hash().len(), 64);

    let bodies = provider.bodies();
    assert_eq!(bodies.len(), 1);
    let sent = &bodies[0];
    assert!(sent.contains("POST /v1/chat/completions"));
    assert!(
        sent.to_ascii_lowercase()
            .contains("authorization: bearer cloud-key"),
        "the resolved key must be sent: {sent}"
    );
    assert!(sent.contains("\"tools\""));
    assert!(sent.contains("\"tool_choice\":\"auto\""));
    assert!(sent.contains("\"model\":\"mock-model\""), "{sent}");
    assert!(sent.contains("http.replay"));
}

#[test]
fn cloud_provider_failure_retries_exactly_once() {
    let provider = MockProvider::spawn(|index, _| match index {
        0 => (
            429,
            "{\"error\":{\"message\":\"rate limited\"}}".to_string(),
        ),
        _ => raw(completion("done", &[], 200)),
    });
    let client = gateway(&provider.base_url, false);
    let response = client
        .complete(&ModelRequest::default(), &CancelToken::new())
        .expect("one retry must recover");
    assert_eq!(provider.hits(), 2);
    assert_eq!(response.usage.total_tokens, 200);
}

#[test]
fn cloud_gives_up_after_the_single_retry() {
    let provider = MockProvider::spawn(|_, _| (503, "{\"error\":\"upstream busy\"}".to_string()));
    let client = gateway(&provider.base_url, false);
    let error = client
        .complete(&ModelRequest::default(), &CancelToken::new())
        .unwrap_err();
    assert!(matches!(error, ModelError::Server { status: 503, .. }));
    assert_eq!(
        provider.hits(),
        2,
        "never more than one provider-level retry"
    );
}

#[test]
fn authentication_failure_is_never_retried_or_relabelled() {
    let provider =
        MockProvider::spawn(|_, _| (401, "{\"error\":{\"message\":\"invalid key\"}}".to_string()));
    let client = gateway(&provider.base_url, false);
    let error = client
        .complete(&ModelRequest::default(), &CancelToken::new())
        .unwrap_err();
    assert!(matches!(error, ModelError::Authentication(_)));
    assert_eq!(provider.hits(), 1);
    assert!(!error.retryable());
    assert!(!error.detail().contains("超时"));
}

#[test]
fn endpoints_without_tool_support_report_unsupported_capability() {
    let provider = MockProvider::spawn(|_, _| {
        (
            400,
            "{\"error\":{\"message\":\"'tools' not supported\"}}".to_string(),
        )
    });
    let client = gateway(&provider.base_url, false);
    let error = client
        .complete(&ModelRequest::new(vec![], specs()), &CancelToken::new())
        .unwrap_err();
    assert!(matches!(error, ModelError::UnsupportedCapability(_)));
    assert_eq!(error.code(), "unsupported_capability");
    assert!(!error.retryable());
}

#[test]
fn context_overflow_is_a_distinct_class() {
    let provider = MockProvider::spawn(|_, _| {
        (
            400,
            "{\"error\":{\"message\":\"maximum context length is 8192 tokens\"}}".to_string(),
        )
    });
    let client = gateway(&provider.base_url, false);
    let error = client
        .complete(&ModelRequest::default(), &CancelToken::new())
        .unwrap_err();
    assert!(matches!(error, ModelError::ContextOverflow(_)));
    assert!(error.detail().contains("上下文"));
}

#[test]
fn malformed_tool_arguments_and_missing_usage_are_handled() {
    let provider = MockProvider::spawn(|_, _| {
        raw(
            serde_json::json!({
                "choices": [{
                    "message": {"content": [{"type": "text", "text": "part one"}], "tool_calls": [
                        {"id": " ", "type": "function", "function": {"name": "http.replay", "arguments": "{not json"}},
                        {"id": "b", "type": "function", "function": {"name": "http.replay", "arguments": "{\"url\":\"x\"}"}},
                        {"id": "c", "type": "function", "function": {"name": "http.replay", "arguments": "{\"url\":\"x\"}"}}
                    ]},
                    "finish_reason": "tool_calls"
                }]
            })
            .to_string(),
        )
    });
    let client = gateway(&provider.base_url, false);
    let response = client
        .complete(&ModelRequest::default(), &CancelToken::new())
        .unwrap();
    assert_eq!(response.text, "part one");
    // three raw calls, one duplicate dropped, malformed JSON turned into data
    let calls = response.deduplicated_tool_calls();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].arguments.get("__parseError").is_some());
    assert!(calls[0].id.starts_with("call_"), "blank ids are replaced");
    assert_eq!(
        response.usage.total_tokens,
        response.usage.input_tokens + response.usage.output_tokens,
        "a provider without usage still accounts tokens"
    );
    assert!(response.usage.input_tokens > 0);
}

#[test]
fn local_requests_are_serialised_and_send_no_credential() {
    let concurrent = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let running = concurrent.clone();
    let highest = peak.clone();
    let provider = MockProvider::spawn(move |_, _| {
        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
        highest.fetch_max(now, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(120));
        let response = raw(completion("", &[], 300));
        running.fetch_sub(1, Ordering::SeqCst);
        response
    });
    let client = Arc::new(gateway(&provider.base_url, true));
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let client = client.clone();
            thread::spawn(move || {
                client
                    .complete(&ModelRequest::default(), &CancelToken::new())
                    .map(|response| response.usage.total_tokens)
            })
        })
        .collect();
    let mut total = 0;
    for handle in handles {
        total += handle.join().unwrap().unwrap();
    }
    assert_eq!(total, 900);
    assert_eq!(provider.hits(), 3);
    assert_eq!(
        peak.load(Ordering::SeqCst),
        1,
        "one local generation at a time (§10)"
    );
    for body in provider.bodies() {
        assert!(
            !body.to_ascii_lowercase().contains("authorization:"),
            "a local server is never given a fabricated credential"
        );
    }
}

#[test]
fn cancellation_is_honoured_before_and_during_a_slow_local_generation() {
    let token = CancelToken::new();
    token.cancel();
    let provider = MockProvider::spawn(|_, _| raw(completion("", &[], 10)));
    let client = gateway(&provider.base_url, true);
    let error = client
        .complete(&ModelRequest::default(), &token)
        .unwrap_err();
    assert_eq!(error, ModelError::Cancelled);
    assert_eq!(provider.hits(), 0, "a cancelled run sends nothing");

    let provider = MockProvider::spawn(|_, _| {
        thread::sleep(Duration::from_millis(1_500));
        raw(completion("too late", &[], 10))
    });
    let client = Arc::new(gateway(&provider.base_url, true));
    let cancel = CancelToken::new();
    let armed = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(250));
        armed.cancel();
    });
    let started = Instant::now();
    let error = client
        .complete(&ModelRequest::default(), &cancel)
        .unwrap_err();
    assert_eq!(error, ModelError::Cancelled);
    assert!(
        started.elapsed() < Duration::from_secs(1_400),
        "cancel must not wait for the generation to finish"
    );
}

#[test]
fn profile_resolution_is_explicit() {
    let cloud = profile("https://api.example.invalid/v1/", false);
    assert_eq!(
        cloud.endpoint,
        "https://api.example.invalid/v1/chat/completions"
    );
    assert_eq!(cloud.bearer(), Some("cloud-key"));
    assert_eq!(cloud.max_context_tokens, 0);
    assert!(cloud.request_budget_bytes() > 1_000_000);
    assert_eq!(cloud.summary()["credential"], "configured");
    assert!(!cloud.summary().to_string().contains("cloud-key"));

    let local = profile("http://127.0.0.1:11434/v1", true);
    assert!(local.local);
    assert!(!local.has_key());
    assert!(local.bearer().is_none());
    assert_eq!(local.max_context_tokens, 32_768);
    assert!(local.request_budget_bytes() < 128_000);

    // A local profile without a base URL would silently point at a public
    // endpoint, so it is refused instead of guessing.
    let missing = GatewayProfile::resolve(
        &ResolvedModelProfile {
            model: "openai/m".into(),
            api_base: String::new(),
            api_key: String::new(),
            deployment: "local".into(),
        },
        &LocalResourcePolicy::default(),
        None,
    );
    assert!(missing.unwrap_err().contains("Base URL"));
    let no_model = GatewayProfile::resolve(
        &ResolvedModelProfile {
            model: "   ".into(),
            api_base: "https://x/v1".into(),
            api_key: "k".into(),
            deployment: "cloud".into(),
        },
        &LocalResourcePolicy::default(),
        None,
    );
    assert!(no_model.is_err());
}

#[test]
fn schema_hash_usage_and_classification_helpers() {
    let schemas = specs();
    let hash = tool_schema_hash(&schemas);
    assert_eq!(hash.len(), 64);
    assert_eq!(hash, tool_schema_hash(&specs()));
    assert_ne!(
        hash,
        tool_schema_hash(&[ToolSchema::new(
            "http.replay",
            "d",
            serde_json::json!({"type": "object"})
        )])
    );

    let body = serde_json::json!({"usage": {"prompt_tokens": 900, "completion_tokens": 100, "total_tokens": 1_000, "prompt_tokens_details": {"cached_tokens": 300}}});
    let usage = usage_from_response(&body, 4_000, 1_000);
    assert_eq!(
        (usage.input_tokens, usage.output_tokens, usage.total_tokens),
        (900, 100, 1_000)
    );
    assert_eq!(usage.uncached_input(), 600);
    assert_eq!(usage.billable_uncached(), 700);
    let estimated = usage_from_response(&JsonValue::Null, 4_000, 800);
    assert_eq!(estimated.input_tokens, estimate_tokens(4_000));
    assert_eq!(
        estimated.total_tokens,
        estimated.input_tokens + estimated.output_tokens
    );
    let merged = estimated.plus(&usage);
    assert_eq!(merged.model_requests, 2);

    assert!(looks_like_unsupported_tools(
        400,
        "Invalid 'tools': unsupported"
    ));
    assert!(!looks_like_unsupported_tools(429, "'tools' not supported"));
    assert!(looks_like_context_overflow(
        400,
        "maximum context length exceeded: tokens"
    ));
    assert!(!looks_like_context_overflow(400, "invalid api key"));
    assert_eq!(classify_status(403, "no").code(), "authentication");
    assert_eq!(
        classify_status(500, "boom").code(),
        "model_provider_failure"
    );
    assert_eq!(extract_message_text(&json_with_text("hi")), "hi");
    assert!(extract_tool_calls(&json_with_text("hi")).is_empty());
}

fn json_with_text(text: &str) -> JsonValue {
    serde_json::json!({"choices": [{"message": {"content": text, "tool_calls": null}, "finish_reason": "stop"}]})
}

#[test]
fn context_layers_and_compaction_keep_the_layers_that_matter() {
    let schemas = specs();
    let hash = tool_schema_hash(&schemas);
    let layers = ContextLayers {
        system_rules: "只调查授权目标".into(),
        schema_hash: hash.clone(),
        agent_slice: serde_json::json!({"role": "coordinator", "lease": {"tokens": 1_000}}),
        facts: serde_json::json!({"confirmed": [], "pending": ["/api/orders"]}),
        recent_rounds: vec![serde_json::json!({"role": "tool", "content": "ok"})],
        artifact_refs: vec!["a".repeat(64)],
    };
    let messages = layers.messages();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["oviraptorKeep"], true);
    assert!(messages[0]["content"]
        .as_str()
        .unwrap()
        .contains(&hash[..16]));
    assert!(messages[1]["content"]
        .as_str()
        .unwrap()
        .contains("artifactRefs"));
    assert!(messages[1]["content"]
        .as_str()
        .unwrap()
        .contains("agentSlice"));

    let big = "x".repeat(4_000);
    let mut history =
        vec![serde_json::json!({"role": "system", "content": "rules", "oviraptorKeep": true})];
    for index in 0..8 {
        history.push(
            serde_json::json!({"role": "assistant", "content": format!("round {index} {big}")}),
        );
        history.push(serde_json::json!({"role": "tool", "tool_call_id": index, "content": big}));
    }
    let before: usize = history.iter().map(|value| value.to_string().len()).sum();
    let budget = before / 4;
    let (compact, changed) = compact_messages(&history, budget);
    assert!(changed);
    let after: usize = compact.iter().map(|value| value.to_string().len()).sum();
    assert!(after <= budget, "{after} > {budget}");
    assert_eq!(compact[0]["content"], "rules");
    let tool_rounds = compact
        .iter()
        .filter(|value| value.get("role").and_then(JsonValue::as_str) == Some("tool"))
        .count();
    assert_eq!(tool_rounds, 8, "old results are digested, never dropped");
    let first_tool = compact
        .iter()
        .find(|value| value.get("role").and_then(JsonValue::as_str) == Some("tool"))
        .expect("tool layer survives compaction");
    assert!(first_tool["content"]
        .as_str()
        .unwrap()
        .contains("已收口证据摘要"));
    assert_eq!(digest_text(&big, 40), digest_text(&big, 40));
    assert!(!compact_messages(&history, before).1);
}

#[test]
fn cancel_token_tracks_flag_and_external_state() {
    let token = CancelToken::new();
    assert!(!token.is_cancelled());
    token.cancel();
    assert!(token.is_cancelled());
    let flag = Arc::new(AtomicBool::new(false));
    let seen = flag.clone();
    let token = CancelToken::from_checker(move || seen.load(Ordering::SeqCst));
    assert!(!token.is_cancelled());
    flag.store(true, Ordering::SeqCst);
    assert!(token.is_cancelled());
}

/// A provider that takes the request and then keeps the connection open without
/// finishing, recording the moment the client's side disappears. This is what
/// proves a stop reaches the wire instead of only stopping the waiting (§6.3).
struct HangingProvider {
    port: u16,
    hits: Arc<AtomicUsize>,
    disconnected: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl HangingProvider {
    /// `streams_first_bytes` answers with a head and one chunk before hanging,
    /// which is what a proxy has to notice a vanished peer on.
    fn spawn(streams_first_bytes: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let disconnected = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (loop_hits, loop_disconnected, loop_stop) =
            (hits.clone(), disconnected.clone(), stop.clone());
        thread::spawn(move || {
            while !loop_stop.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(1));
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                if read_request(&mut stream).is_none() {
                    continue;
                }
                loop_hits.fetch_add(1, Ordering::SeqCst);
                if streams_first_bytes {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n5\r\nhello\r\n",
                    );
                    let _ = stream.flush();
                }
                // Hold the generation the way a slow cloud provider does, and watch
                // for the client giving up.
                let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                let deadline = Instant::now() + Duration::from_secs(30);
                let mut buffer = [0u8; 64];
                while Instant::now() < deadline && !loop_stop.load(Ordering::SeqCst) {
                    match stream.read(&mut buffer) {
                        Ok(0) => {
                            loop_disconnected.store(true, Ordering::SeqCst);
                            break;
                        }
                        Ok(_) => continue,
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue;
                        }
                        Err(_) => {
                            loop_disconnected.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            }
        });
        Self {
            port,
            hits,
            disconnected,
            stop,
        }
    }

    fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    /// The provider needs one poll to notice; waiting is bounded.
    fn wait_for_disconnect(&self) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.disconnected.load(Ordering::SeqCst) {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.disconnected.load(Ordering::SeqCst)
    }
}

impl Drop for HangingProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Phase 2 §6.3: a provider that holds the connection for a minute must not be
/// able to keep it. The round has to settle in well under two seconds, the
/// provider has to see the socket close, and one stop must never send twice.
#[test]
fn cancelling_a_cloud_request_closes_the_connection_at_the_provider() {
    let provider = HangingProvider::spawn(false);
    let client = Arc::new(gateway(&provider.base_url(), false));
    let token = CancelToken::new();
    let armed = token.clone();
    let canceller = thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        armed.cancel();
    });
    let started = Instant::now();
    let error = client
        .complete(&ModelRequest::default(), &token)
        .unwrap_err();
    let elapsed = started.elapsed();
    assert_eq!(error, ModelError::Cancelled);
    assert!(
        elapsed < Duration::from_secs(2),
        "the round only settled after {elapsed:?}"
    );
    assert!(
        provider.wait_for_disconnect(),
        "the provider never saw the connection close"
    );
    assert_eq!(
        provider.hits(),
        1,
        "a stop must not turn into a second billed request"
    );
    let observation = client
        .take_cancel_observation()
        .expect("the transport records when the stop settled");
    assert!(!observation.cancel_requested_at.is_empty());
    assert!(!observation.transport_closed_at.is_empty());
    assert!(observation.settle_millis < 2_000);
    canceller.join().unwrap();
}

/// §6.2: the same transport serves the hook's proxy path, where the reason to
/// stop is the peer underneath going away. Dropping the future must close the
/// upstream connection and report who asked.
#[test]
fn a_vanished_peer_stops_the_upstream_request() {
    use super::transport::{
        self, CancelInitiator, TransportError, TransportEvent, TransportRequest,
    };
    let provider = HangingProvider::spawn(true);
    let saw_head = Arc::new(AtomicBool::new(false));
    let head_flag = saw_head.clone();
    let sink: Box<dyn FnMut(TransportEvent<'_>) -> bool + Send> = Box::new(move |event| {
        match event {
            TransportEvent::Head { .. } => head_flag.store(true, Ordering::SeqCst),
            // The peer is gone as soon as the first byte arrives.
            TransportEvent::Body(_) => return false,
        }
        true
    });
    let cancel: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(|| false);
    let started = Instant::now();
    let outcome = transport::send(
        TransportRequest::post(
            format!("{}/chat/completions", provider.base_url()),
            b"{}".to_vec(),
        ),
        cancel,
        sink,
    );
    let elapsed = started.elapsed();
    assert!(
        saw_head.load(Ordering::SeqCst),
        "the head is streamed first"
    );
    assert!(elapsed < Duration::from_secs(2), "settled in {elapsed:?}");
    match outcome {
        Err(TransportError::Cancelled(observation)) => {
            assert_eq!(observation.initiator, CancelInitiator::Peer);
            assert!(!observation.transport_closed_at.is_empty());
        }
        other => panic!("a vanished peer must cancel the request, got {other:?}"),
    }
    assert!(
        provider.wait_for_disconnect(),
        "the upstream kept serving to a peer that had left"
    );
}

/// FRESH-SINGLE-FIX negative test: a reply that has already been read to the
/// end must not lose a coin flip to a cancel probe that fires in the same poll
/// (that turned a billed, received Root reply into an unknown outcome whose
/// reserve was forfeited as indeterminate). A reply still on the wire keeps
/// losing to cancellation.
#[test]
fn transport_settle_prefers_a_completed_reply_and_still_cancels_in_flight() {
    use super::transport::{self, CancelInitiator, TransportError, TransportReply};
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // Unbiased, 256 ties lose at least once with probability 1 - 2^-256.
    for round in 0..256 {
        let outcome = runtime.block_on(transport::settle(
            std::future::ready(Ok(TransportReply {
                status: 200,
                body: b"{\"usage\":{}}".to_vec(),
            })),
            std::future::ready(()),
        ));
        match outcome {
            Ok(reply) => assert_eq!((reply.status, reply.body.len()), (200, 12)),
            other => panic!("tie {round}: a completed reply was discarded: {other:?}"),
        }
    }
    let in_flight = runtime.block_on(transport::settle(
        std::future::pending::<Result<TransportReply, TransportError>>(),
        std::future::ready(()),
    ));
    match in_flight {
        Err(TransportError::Cancelled(observation)) => {
            assert_eq!(observation.initiator, CancelInitiator::Local)
        }
        other => panic!("an in-flight reply must still be cancelled: {other:?}"),
    }
    let source = include_str!("transport.rs");
    assert!(source.contains("settle(fetch, wait_for_cancel(&cancel)).await"));
    assert!(source.contains("        biased;\n        outcome = fetch => outcome,"));
}


#[test]
fn received_usage_log_distinguishes_provider_reported_and_ambiguous_numbers() {
    let provider = MockProvider::spawn(|index, _| {
        let text = completion("usage log", &[], 100);
        raw(if index == 0 { text } else { text.replacen("\"usage\":", "\"usage\":{},\"usage\":", 1) })
    });
    let client = gateway(&provider.base_url, true);
    let root = std::env::temp_dir().join(format!("oviraptor-usage-log-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    for reported in [true, false] {
        let reply = client.complete(&ModelRequest::default(), &CancelToken::new()).unwrap();
        assert_eq!(reply.usage_reported, reported);
        client.record_usage_line(&root, &reply);
    }
    let text = std::fs::read_to_string(root.join(super::openai_compatible::USAGE_LEDGER_FILE)).unwrap();
    let lines: Vec<JsonValue> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["usageReported"], true);
    assert_eq!(lines[0]["usageBasis"], "provider_reported");
    assert_eq!(lines[1]["usageReported"], false);
    assert_eq!(lines[1]["usageBasis"], "unverified");
    assert_eq!(lines[0]["usage"], lines[1]["usage"], "matching displayed numbers cannot conceal billing ambiguity");
    assert_eq!(provider.hits(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn received_cache_provenance_distinguishes_absent_zero_and_ambiguous_wire() {
    use crate::recovery_finance::{Money, RateCard};
    let binding = "a".repeat(64);
    let pricing = RateCard::new(binding.clone(), "local-test-pricing".into(),
        Money::from_nanos(2_000_000_000).unwrap(), Money::from_nanos(500_000_000).unwrap(),
        Money::from_nanos(4_000_000_000).unwrap(), Money::from_nanos(10_000_000).unwrap()).unwrap();
    let provider = MockProvider::spawn(|index, _| {
        let mut body: JsonValue = serde_json::from_str(&completion("cache provenance", &[], 100)).unwrap();
        body["usage"].as_object_mut().unwrap().remove("prompt_tokens_details");
        match index {
            0 => {},
            1 => body["usage"]["cached_tokens"] = serde_json::json!(0),
            2 => body["usage"]["input_tokens_details"] = serde_json::json!({"cached_tokens":25}),
            _ => body["usage"]["cached_tokens"] = serde_json::json!(0),
        }
        let text = body.to_string();
        raw(if index == 3 { text.replacen("\"cached_tokens\":0", "\"cached_tokens\":0,\"cached_tokens\":0", 1) } else { text })
    });
    let client = gateway(&provider.base_url, true);
    let root = std::env::temp_dir().join(format!("oviraptor-cache-provenance-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    for (reported, cache_reported, cached) in [(true, false, 0), (true, true, 0), (true, true, 25), (false, false, 0)] {
        let reply = client.complete(&ModelRequest::default(), &CancelToken::new()).unwrap();
        assert_eq!((reply.usage_reported, reply.cached_usage_reported, reply.usage.cached_input_tokens), (reported, cache_reported, cached));
        let quote = pricing.quote_reported_response(&binding, &reply);
        if !reported {
            assert_eq!(quote.unwrap_err(), "recovery_pricing_usage_unreported");
        } else if !cache_reported {
            assert_eq!(quote.unwrap_err(), "recovery_pricing_cached_usage_unreported");
        } else {
            assert_eq!(quote.unwrap().nanos(), 10_240_000 - cached * 1_500);
        }
        client.record_usage_line(&root, &reply);
    }
    let text = std::fs::read_to_string(root.join(super::openai_compatible::USAGE_LEDGER_FILE)).unwrap();
    let frames: Vec<JsonValue> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(frames.len(), 4);
    for (frame, expected) in frames.iter().zip([false, true, true, false]) {
        assert_eq!(frame["cachedUsageReported"], expected);
    }
    assert_eq!(provider.hits(), 4, "provenance inspection never resends an SDK request");
    std::fs::remove_dir_all(root).unwrap();
}


#[test]
fn concurrent_usage_logging_preserves_complete_json_frames() {
    let finish = "quoted\" newline\n终态 ".repeat(500);
    let observed = finish.clone();
    let provider = MockProvider::spawn(move |_, _| {
        let mut body: JsonValue = serde_json::from_str(&completion("log frame", &[], 100)).unwrap();
        body["choices"][0]["finish_reason"] = serde_json::json!(observed);
        raw(body.to_string())
    });
    let client = Arc::new(gateway(&provider.base_url, true));
    let response = client.complete(&ModelRequest::default(), &CancelToken::new()).unwrap();
    let root = std::env::temp_dir().join(format!("oviraptor-usage-frame-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut workers = Vec::new();
    for _ in 0..8 {
        let (client, response, root, barrier) = (client.clone(), response.clone(), root.clone(), barrier.clone());
        workers.push(thread::spawn(move || {
            barrier.wait();
            for _ in 0..4 { client.record_usage_line(&root, &response); }
        }));
    }
    for worker in workers { worker.join().unwrap(); }
    let text = std::fs::read_to_string(root.join(super::openai_compatible::USAGE_LEDGER_FILE)).unwrap();
    assert_eq!(text.lines().count(), 32);
    for line in text.lines() {
        let frame: JsonValue = serde_json::from_str(line).expect("concurrent diagnostic appends must be complete JSON frames");
        assert_eq!(frame["finishReason"], finish);
        assert_eq!(frame["usageReported"], true);
    }
    assert_eq!(provider.hits(), 1, "32 diagnostic appends are not 32 model requests");
    std::fs::remove_dir_all(root).unwrap();
}


#[test]
fn locked_usage_diagnostic_does_not_indefinitely_block_the_received_call() {
    let provider = MockProvider::spawn(|_, _| raw(completion("received", &[], 100)));
    let client = gateway(&provider.base_url, true);
    let response = client.complete(&ModelRequest::default(), &CancelToken::new()).unwrap();
    let root = std::env::temp_dir().join(format!("oviraptor-usage-held-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join(super::openai_compatible::USAGE_LEDGER_FILE);
    let held = std::fs::OpenOptions::new().create(true).append(true).open(&path).unwrap();
    held.lock().unwrap();
    let started = Instant::now();
    client.record_usage_line(&root, &response);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(std::fs::read_to_string(&path).unwrap().is_empty(), "a contended diagnostic cannot write an unlocked partial frame");
    drop(held);
    client.record_usage_line(&root, &response);
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    assert_eq!(provider.hits(), 1, "diagnostic contention cannot reissue a model request");
    std::fs::remove_dir_all(root).unwrap();
}


#[test]
fn complete_payload_budget_rejects_messages_and_tool_schema_before_transport() {
    for tools in [false, true] {
        let provider = MockProvider::spawn(|_, _| raw(completion("accepted", &[], 10)));
        let client = gateway(&provider.base_url, true);
        let size = profile(&provider.base_url, true).request_budget_bytes();
        let mut request = ModelRequest::default();
        if tools {
            request.tools = vec![ToolSchema::new("large", "schema", serde_json::json!({"description":"界".repeat(size)}))];
        } else {
            request.messages = vec![serde_json::json!({"role":"user","content":"界".repeat(size)})];
        }
        let error = client.complete_once_observed(&request, &CancelToken::new()).unwrap_err();
        assert!(matches!(error, super::openai_compatible::OneShotFailure::BeforeTransport(ModelError::ContextOverflow(_))));
        assert_eq!(provider.hits(), 0);
        client.complete(&ModelRequest::default(), &CancelToken::new()).unwrap();
        assert_eq!(provider.hits(), 1);
    }
}


#[test]
fn request_payload_budget_extreme_local_policy_never_overflows_transport_ceiling() {
    let mut local = profile("http://127.0.0.1:11434/v1", true);
    for (context, output, expected) in [
        (u64::MAX, Some(1), 6_000_000),
        (u64::MAX, Some(u64::MAX), 16_384),
        (1, Some(u64::MAX), 16_384),
        (32_768, Some(2_048), 120_832),
    ] {
        local.max_context_tokens = context;
        local.max_output_tokens = output;
        assert_eq!(local.request_budget_bytes(), expected);
    }
}


#[test]
fn configured_output_limit_is_sent_exactly_and_invalid_limits_never_dispatch() {
    for local in [false, true] {
        let provider = MockProvider::spawn(|_, _| raw(completion("bounded", &[], 10)));
        let mut bounded = profile(&provider.base_url, local);
        bounded.max_output_tokens = Some(37);
        let client = OpenAiCompatibleGateway::new(bounded.clone(), &[]);
        client.complete_once_observed(&ModelRequest::default(), &CancelToken::new()).unwrap();
        let requests = provider.bodies();
        let raw = &requests[0];
        let body: JsonValue = serde_json::from_str(raw.split("\r\n\r\n").nth(1).unwrap_or(raw)).unwrap();
        assert_eq!(body["max_tokens"], 37);
        for limit in [0, u64::MAX, i64::MAX as u64 + 1] {
            bounded.max_output_tokens = Some(limit);
            let invalid = OpenAiCompatibleGateway::new(bounded.clone(), &[]);
            assert!(matches!(invalid.complete_once_observed(&ModelRequest::default(), &CancelToken::new()),
                Err(super::openai_compatible::OneShotFailure::BeforeTransport(ModelError::Protocol(ref e))) if e == "model_output_limit_invalid"));
        }
        if local {
            bounded.max_output_tokens = Some(bounded.max_context_tokens);
            let invalid = OpenAiCompatibleGateway::new(bounded, &[]);
            assert!(matches!(invalid.complete_once_observed(&ModelRequest::default(), &CancelToken::new()),
                Err(super::openai_compatible::OneShotFailure::BeforeTransport(ModelError::Protocol(_)))));
        }
        assert_eq!(provider.hits(), 1);
    }
}

#[test]
fn small_deepseek_structured_calls_freeze_non_thinking_without_expanding_reservation() {
    for (model, cap, disabled) in [
        ("deepseek-v4-flash", Some(512), true),
        ("deepseek-v4-pro", Some(1024), true),
        ("deepseek-flash", Some(2048), true),
        ("deepseek-v4-flash", Some(4096), false),
        ("deepseek-v4-flash", None, false),
        ("other-model", Some(512), false),
    ] {
        let provider = MockProvider::spawn(|_, _| raw(completion("answer", &[], 10)));
        let mut bounded = profile(&provider.base_url, false);
        bounded.model = model.into();
        bounded.max_output_tokens = cap;
        let frozen = bounded.bounded_wire_options();
        let client = OpenAiCompatibleGateway::new(bounded, &[]);
        client.complete_once_observed(&ModelRequest::default(), &CancelToken::new()).unwrap();
        let requests = provider.bodies();
        let raw = &requests[0];
        let body: JsonValue = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body.get("max_tokens").and_then(JsonValue::as_u64), cap);
        if disabled {
            assert_eq!(body["thinking"], frozen["thinking"]);
            assert_eq!(body["thinking"]["type"], "disabled");
        } else {
            assert!(body.get("thinking").is_none());
            assert_eq!(frozen, serde_json::json!({}));
        }
        assert_eq!(provider.hits(), 1);
    }
}
