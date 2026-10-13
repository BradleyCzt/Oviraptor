use super::*;
use crate::agent_runtime::model::{
    gateway::ModelResponse, LocalResourcePolicy, ModelError, ResolvedModelProfile,
};
use crate::conversations::tests::Fixture;

fn profile() -> GatewayProfile {
    GatewayProfile::resolve(
        &ResolvedModelProfile {
            model: "test-model".into(),
            api_base: "http://127.0.0.1:1/v1".into(),
            api_key: "test-secret".into(),
            deployment: "local".into(),
        },
        &LocalResourcePolicy {
            max_output_tokens: Some(2048),
            max_context_tokens: 8192,
        },
        None,
    )
    .unwrap()
}
fn input(f: &Fixture, item: &Conversation, text: &str) -> SendInput {
    let mut input = f.input(item, text);
    input.request_reply = true;
    input
}
#[test]
fn unavailable_model_saves_human_text_without_simulated_assistant() {
    let f = Fixture::new();
    let item = f.create();
    let saved = send_with(
        &f.path,
        input(&f, &item, "hello"),
        |_| Err("conversation_model_unavailable".into()),
        |_, _, _| panic!("no transport"),
    )
    .unwrap();
    assert_eq!(saved.reply.unwrap().status, "unavailable");
    assert_eq!(f.read(&item).messages.len(), 1);
    assert_eq!(f.read(&item).messages[0].role, "user");
}
#[test]
fn real_reply_contract_has_no_tools_and_idempotency_never_resends() {
    let f = Fixture::new();
    let item = f.create();
    let op = Uuid::new_v4().to_string();
    let make = || {
        let mut v = input(&f, &item, "hello");
        v.operation_id = op.clone();
        v
    };
    let saved = send_with(
        &f.path,
        make(),
        |_| Ok(profile()),
        |_, request, _| {
            assert!(request.tools.is_empty());
            assert!(request.reject_redirects);
            assert_eq!(request.max_response_bytes, Some(1_048_576));
            assert_eq!(request.request_timeout, Some(Duration::from_secs(90)));
            Ok(ModelResponse {
                text: "Real mock-provider answer".into(),
                ..Default::default()
            })
        },
    )
    .unwrap();
    assert_eq!(saved.reply.unwrap().status, "completed");
    assert_eq!(saved.conversation.revision, 3);
    let replay = send_with(
        &f.path,
        make(),
        |_| panic!("no repeated config"),
        |_, _, _| panic!("no repeated provider request"),
    )
    .unwrap();
    assert_eq!(replay.user_message.id, saved.user_message.id);
    assert_eq!(f.read(&item).messages.len(), 2);
    let mut changed = make();
    changed.content = "different".into();
    assert_eq!(
        send_with(&f.path, changed, |_| Ok(profile()), |_, _, _| panic!())
            .err()
            .unwrap(),
        "conversation_operation_conflict"
    );
}
#[test]
fn stale_project_completion_never_enters_chat_history() {
    let f = Fixture::new();
    let item = f.create();
    let saved = send_with(
        &f.path,
        input(&f, &item, "hello"),
        |_| Ok(profile()),
        |_, _, _| {
            db::open(&f.path)
                .unwrap()
                .execute("UPDATE projects SET status='archived' WHERE id=901", [])
                .unwrap();
            Ok(ModelResponse {
                text: "stale answer".into(),
                ..Default::default()
            })
        },
    )
    .unwrap();
    let reply = saved.reply.unwrap();
    assert_eq!(reply.status, "cancelled");
    assert_eq!(reply.code, "reply_stale_completion");
    assert!(!reply.outcome_unknown);
    assert_eq!(f.read(&item).messages.len(), 1);
}
#[test]
fn cancellation_remains_requested_until_transport_owner_returns() {
    let f = Fixture::new();
    let item = f.create();
    let saved = send_with(
        &f.path,
        input(&f, &item, "hello"),
        |_| Ok(profile()),
        |_, _, token| {
            let running = f.read(&item).latest_reply.unwrap();
            assert_eq!(running.status, "running");
            let requested = cancel(
                &f.path,
                CancelInput {
                    project_id: 901,
                    conversation_id: item.id.clone(),
                    reply_id: running.id,
                },
            )
            .unwrap();
            assert_eq!(requested.status, "cancel_requested");
            assert_eq!(
                f.read(&item).latest_reply.unwrap().status,
                "cancel_requested"
            );
            assert!(token.is_cancelled());
            Err(OneShotFailure::TransportOutcomeUnknown(
                ModelError::Cancelled,
            ))
        },
    )
    .unwrap();
    let reply = saved.reply.unwrap();
    assert_eq!(reply.status, "cancelled");
    assert!(reply.outcome_unknown);
    assert_eq!(f.read(&item).messages.len(), 1);
}
#[test]
fn provider_errors_never_persist_secret_bearing_error_details() {
    let f = Fixture::new();
    let item = f.create();
    let saved = send_with(
        &f.path,
        input(&f, &item, "hello"),
        |_| Ok(profile()),
        |_, _, _| {
            Err(OneShotFailure::TransportOutcomeUnknown(
                ModelError::Authentication("Bearer secret-do-not-store".into()),
            ))
        },
    )
    .unwrap();
    let reply = saved.reply.unwrap();
    assert_eq!(reply.status, "failed");
    assert_eq!(reply.code, "authentication");
    assert!(!reply.message.contains("secret"));
    assert!(reply.outcome_unknown);
}
#[test]
fn ignored_assistant_insert_rolls_back_and_never_acknowledges_completion() {
    let f = Fixture::new();
    let item = f.create();
    db::open(&f.path).unwrap().execute_batch("CREATE TRIGGER reject_assistant BEFORE INSERT ON conversation_messages WHEN NEW.role='assistant' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(send_with(
        &f.path,
        input(&f, &item, "hello"),
        |_| Ok(profile()),
        |_, _, _| Ok(ModelResponse {
            text: "answer".into(),
            ..Default::default()
        })
    )
    .is_err());
    let view = f.read(&item);
    assert_eq!(view.messages.len(), 1);
    assert_eq!(view.latest_reply.unwrap().status, "interrupted");
}

#[test]
fn standalone_endpoint_policy_is_enforced_before_advertising_availability() {
    let mut safe = profile();
    assert!(validate_profile(safe.clone()).is_ok());
    safe.endpoint = "http://example.test/v1/chat/completions".into();
    assert!(validate_profile(safe.clone()).is_err());
    safe.endpoint = "https://example.test/v1/chat/completions".into();
    assert!(validate_profile(safe.clone()).is_ok());
    safe.endpoint = "https://user:secret@example.test/v1/chat/completions".into();
    assert!(validate_profile(safe.clone()).is_err());
    safe.endpoint = "http://127.0.0.1/v1/chat/completions".into();
    safe.local = false;
    assert!(validate_profile(safe).is_err());
}
#[test]
fn dispatch_connection_uses_full_synchronous_durability() {
    let f = Fixture::new();
    let item = f.create();
    send_with(
        &f.path,
        input(&f, &item, "hello"),
        |connection| {
            assert_eq!(
                connection
                    .query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                2
            );
            Err("conversation_model_unavailable".into())
        },
        |_, _, _| panic!("unavailable"),
    )
    .unwrap();
}

fn loopback_server(response: Vec<u8>) -> (String, std::thread::JoinHandle<Vec<u8>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut part = [0; 4096];
        loop {
            let count = stream.read(&mut part).unwrap();
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&part[..count]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&bytes[..end]);
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let _ = stream.write_all(&response);
        bytes
    });
    (url, thread)
}
#[test]
fn bounded_real_gateway_does_not_follow_redirect_or_forward_secret() {
    let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let wire=format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{}/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",target.local_addr().unwrap());
    let (url, server) = loopback_server(wire.into_bytes());
    let mut p = profile();
    p.endpoint = url;
    let mut request = ModelRequest::new(
        vec![json!({"role":"user","content":"private message"})],
        vec![],
    );
    request.reject_redirects = true;
    request.max_response_bytes = Some(1024);
    let result =
        OpenAiCompatibleGateway::new(p, &[]).complete_once_observed(&request, &CancelToken::new());
    assert!(result.is_err());
    let sent = String::from_utf8(server.join().unwrap()).unwrap();
    assert!(sent.contains("private message"));
    assert!(sent.contains("test-secret"));
    assert!(
        matches!(target.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock),
        "redirect destination must receive neither body nor credential"
    );
}
#[test]
fn bounded_real_gateway_caps_success_and_error_response_bodies() {
    for status in ["200 OK", "500 Error"] {
        let body = "x".repeat(4096);
        let wire = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let (url, server) = loopback_server(wire.into_bytes());
        let mut p = profile();
        p.endpoint = url;
        let mut request =
            ModelRequest::new(vec![json!({"role":"user","content":"bounded"})], vec![]);
        request.reject_redirects = true;
        request.max_response_bytes = Some(64);
        let result = OpenAiCompatibleGateway::new(p, &[])
            .complete_once_observed(&request, &CancelToken::new());
        assert!(
            matches!(result,Err(OneShotFailure::TransportOutcomeUnknown(ModelError::Network(message))) if message=="model_response_size_limit")
        );
        server.join().unwrap();
    }
}
#[test]
fn legacy_model_transport_defaults_remain_unbounded_and_redirect_compatible() {
    let request = ModelRequest::default();
    assert!(!request.reject_redirects);
    assert!(request.max_response_bytes.is_none());
    let transport =
        crate::agent_runtime::model::TransportRequest::post("https://example.test", vec![]);
    assert!(!transport.reject_redirects);
    assert!(transport.max_response_bytes.is_none());
}

#[test]
fn real_loopback_response_is_persisted_as_assistant_after_user() {
    let body=json!({"choices":[{"message":{"content":"Loopback provider answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}}).to_string();
    let wire=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
    let (url, server) = loopback_server(wire.into_bytes());
    let mut p = profile();
    p.endpoint = url;
    let f = Fixture::new();
    let item = f.create();
    let saved = send_with(
        &f.path,
        input(&f, &item, "A real request"),
        |_| Ok(p),
        |profile, request, cancel| {
            OpenAiCompatibleGateway::new(profile, &[]).complete_once_observed(request, cancel)
        },
    )
    .unwrap();
    assert_eq!(saved.reply.unwrap().status, "completed");
    let view = f.read(&item);
    assert_eq!(view.messages.len(), 2);
    assert_eq!(view.messages[0].role, "user");
    assert_eq!(view.messages[1].role, "assistant");
    assert_eq!(view.messages[1].content, "Loopback provider answer");
    let sent = String::from_utf8(server.join().unwrap()).unwrap();
    assert!(!sent.contains("\"tools\""));
    assert!(!sent.contains("\"tool_choice\""));
    assert!(sent.contains("A real request"));
}

#[test]
fn response_cap_is_applied_before_oversized_bytes_reach_sink() {
    use crate::agent_runtime::model::transport::{
        send, TransportError, TransportEvent, TransportRequest,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let body = "x".repeat(4096);
    let wire = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let (url, server) = loopback_server(wire.into_bytes());
    let mut request = TransportRequest::post(url, b"{}".to_vec());
    request.reject_redirects = true;
    request.max_response_bytes = Some(64);
    let observed = Arc::new(AtomicUsize::new(0));
    let sink_count = observed.clone();
    let result = send(
        request,
        Arc::new(|| false),
        Box::new(move |event| {
            if let TransportEvent::Body(chunk) = event {
                sink_count.fetch_add(chunk.len(), Ordering::SeqCst);
            }
            true
        }),
    );
    assert!(
        matches!(result,Err(TransportError::Interrupted(message)) if message=="model_response_size_limit")
    );
    assert!(observed.load(Ordering::SeqCst) <= 64);
    server.join().unwrap();
}
