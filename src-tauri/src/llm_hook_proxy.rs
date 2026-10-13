// The HTTP/CONNECT plumbing of the loopback listener: read a request off the
// connection, forward it to the configured upstream, and shape the record for it.
// Included from llm_hook.rs.

fn parse_http_base(value: &str) -> Result<Option<Upstream>, String> {
    let value = value.trim();
    let (scheme, rest, default_port) = if let Some(rest) = value.strip_prefix("http://") {
        ("http", rest, 80)
    } else if let Some(rest) = value.strip_prefix("https://") {
        ("https", rest, 443)
    } else {
        return Ok(None);
    };
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.contains('?') {
        return Err("本地 LLM Hook 只接受不含凭据的 HTTP 地址".into());
    }
    let base_path = rest
        .split_once('/')
        .map(|(_, path)| format!("/{path}"))
        .unwrap_or_default();
    let (host, port, host_header) = if let Some(raw) = authority.strip_prefix('[') {
        let Some((host, rest)) = raw.split_once(']') else {
            return Err("本地 LLM 地址的 IPv6 格式无效".into());
        };
        let port = rest
            .strip_prefix(':')
            .unwrap_or(if default_port == 443 { "443" } else { "80" })
            .parse::<u16>()
            .map_err(|_| "本地 LLM 端口无效")?;
        (host.to_string(), port, format!("[{host}]:{port}"))
    } else if let Some((host, raw_port)) = authority.rsplit_once(':') {
        let port = raw_port.parse::<u16>().map_err(|_| "本地 LLM 端口无效")?;
        (host.to_string(), port, authority.to_string())
    } else {
        (authority.to_string(), default_port, authority.to_string())
    };
    if host.is_empty() {
        return Err("本地 LLM 地址缺少主机名".into());
    }
    Ok(Some(Upstream {
        scheme: scheme.to_string(),
        host,
        port,
        host_header,
        base_path,
        proxy: None,
        api_key: String::new(),
    }))
}

#[allow(clippy::too_many_arguments)]
fn handle_connection(
    mut client: TcpStream,
    upstream: Upstream,
    output_path: &Path,
    write_lock: &Mutex<()>,
    capture_mode: &str,
    max_output_tokens: Option<u64>,
    max_context_tokens: u64,
    stop: &Arc<AtomicBool>,
    active_upstreams: Arc<Mutex<HashMap<String, TcpStream>>>,
    concurrency_gate: &ConcurrencyGate,
) {
    let request = match read_request(&mut client) {
        Ok(request) => request,
        Err(error) => {
            write_error(&mut client, 400, &error);
            return;
        }
    };
    let mut request = request;
    let original_request_value =
        serde_json::from_slice::<Value>(&request.body).unwrap_or_else(|_| json!({}));
    if let Some(limit) = max_output_tokens {
        // The model gateway performs a real provider health request before the scan. A
        // large default generation allowance is pointless for “OK” and can
        // amplify first-load CPU time on reasoning-oriented 27B models.
        let effective_limit = if is_health_check_request(&original_request_value) {
            limit.min(64)
        } else {
            limit
        };
        request.body = clamp_output_tokens(&request.body, effective_limit);
    }
    let (guarded_body, context_guard) =
        guard_local_model_context(&request.body, max_context_tokens);
    request.body = guarded_body;
    let request_value =
        serde_json::from_slice::<Value>(&request.body).unwrap_or_else(|_| json!({}));
    let request_id = Uuid::new_v4().to_string();
    let call_type = call_type_for_request(&request_value);
    let Some(_permit) = concurrency_gate.acquire(stop) else {
        append_cancelled_record(output_path, write_lock, &request_id, &request_value);
        write_error(&mut client, 503, "任务已停止，本地模型请求未执行");
        return;
    };
    append_record(
        output_path,
        write_lock,
        &json!({
            "kind":"model_call_started",
            "requestId":request_id,
            "callType":call_type,
            "recordedAt":chrono::Utc::now().to_rfc3339(),
            "path":request.path,
            "model":request_value.get("model").cloned().unwrap_or(Value::Null),
            "stream":request_value.get("stream").cloned().unwrap_or(Value::Bool(false)),
            "requestChars":request.body.len(),
            "estimatedInputTokens":((request.body.len() as i64 + 3) / 4).max(1),
            // Structural telemetry contains no prompt text, credentials or
            // tool arguments, but keeps context growth diagnosable when full
            // prompt auditing is disabled.
            "requestSummary":request_summary(&request_value),
            "contextGuard":context_guard,
        }),
    );
    if upstream.scheme == "https" {
        handle_https_request(
            client,
            request,
            upstream,
            output_path,
            write_lock,
            capture_mode,
            max_output_tokens,
            &request_id,
            stop,
        );
        return;
    }
    let mut upstream_stream = match (upstream.host.as_str(), upstream.port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| {
            addresses.find_map(|address| {
                TcpStream::connect_timeout(&address, Duration::from_secs(15)).ok()
            })
        }) {
        Some(stream) => stream,
        None => {
            append_failure_record(
                output_path,
                write_lock,
                &request_id,
                &request_value,
                502,
                "无法连接本地 LLM 上游地址",
            );
            write_error(&mut client, 502, "无法连接本地 LLM 上游地址");
            return;
        }
    };
    let _active_upstream =
        ActiveUpstream::register(&request_id, &upstream_stream, active_upstreams);
    // A short read poll lets task cancellation close an inference immediately.
    // The previous four-hour blocking timeout left detached 27B generations
    // consuming CPU after the owning Native Agent task had already stopped.
    let _ = upstream_stream.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = upstream_stream.set_write_timeout(Some(Duration::from_secs(60)));
    let outbound = build_request(&request, &upstream.host_header, &upstream.api_key);
    if upstream_stream.write_all(&outbound).is_err() || upstream_stream.flush().is_err() {
        append_failure_record(
            output_path,
            write_lock,
            &request_id,
            &request_value,
            502,
            "无法转发本地 LLM 请求",
        );
        write_error(&mut client, 502, "无法转发本地 LLM 请求");
        return;
    }
    let mut response = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        if stop.load(Ordering::Acquire) {
            append_cancelled_record(output_path, write_lock, &request_id, &request_value);
            return;
        }
        match upstream_stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if client.write_all(&buffer[..count]).is_err() {
                    let _ = upstream_stream.shutdown(Shutdown::Both);
                    append_client_disconnected_record(
                        output_path,
                        write_lock,
                        &request_id,
                        &request_value,
                    );
                    return;
                }
                if response.len() < MAX_CAPTURE_BYTES {
                    response.extend_from_slice(
                        &buffer[..count.min(MAX_CAPTURE_BYTES - response.len())],
                    );
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(_) => break,
        }
    }
    if stop.load(Ordering::Acquire) {
        append_cancelled_record(output_path, write_lock, &request_id, &request_value);
        return;
    }
    let _ = client.flush();
    let record = build_record(
        &request,
        &request_value,
        &response,
        capture_mode,
        &request_id,
    );
    append_record(output_path, write_lock, &record);
}

#[allow(clippy::too_many_arguments)]
fn handle_https_request(
    client: TcpStream,
    request: Request,
    upstream: Upstream,
    output_path: &Path,
    write_lock: &Mutex<()>,
    capture_mode: &str,
    max_output_tokens: Option<u64>,
    request_id: &str,
    stop: &Arc<AtomicBool>,
) {
    let mut request = request;
    if let Some(limit) = max_output_tokens {
        request.body = clamp_output_tokens(&request.body, limit);
    }
    let request_value =
        serde_json::from_slice::<Value>(&request.body).unwrap_or_else(|_| json!({}));
    let mut spec = TransportRequest {
        endpoint: format!("https://{}{}", upstream.host_header, request.path),
        method: request
            .method
            .parse::<reqwest::Method>()
            .unwrap_or(reqwest::Method::POST)
            .as_str()
            .to_string(),
        headers: Vec::new(),
        body: request.body.clone(),
        connect_timeout: Duration::from_secs(15),
        // A long generation is allowed; what bounds it is the task stop flag, which
        // now drops the request instead of only stopping the waiting.
        total_timeout: Some(Duration::from_secs(14_400)),
        proxy: upstream.proxy.clone(),
        reject_redirects: false,
        max_response_bytes: None,
    };
    for (key, value) in &request.headers {
        if key.eq_ignore_ascii_case("host")
            || key.eq_ignore_ascii_case("content-length")
            || key.eq_ignore_ascii_case("connection")
            || key.eq_ignore_ascii_case("accept-encoding")
            || key.eq_ignore_ascii_case("authorization")
        {
            continue;
        }
        spec.headers.push((key.clone(), value.clone()));
    }
    if !upstream.api_key.is_empty() {
        spec.headers.push((
            "authorization".to_string(),
            format!("Bearer {}", upstream.api_key),
        ));
    }
    let stop_flag = Arc::clone(stop);
    let cancel: Arc<dyn Fn() -> bool + Send + Sync> =
        Arc::new(move || stop_flag.load(Ordering::Acquire));
    let head_bytes: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let client = Arc::new(Mutex::new(client));
    let writer = Arc::clone(&client);
    let captured_head = Arc::clone(&head_bytes);
    let mut head_written = false;
    let sink: Box<dyn FnMut(TransportEvent<'_>) -> bool + Send> = Box::new(move |event| {
        let payload = match event {
            TransportEvent::Head { status, headers } => {
                if head_written {
                    return true;
                }
                // The transport reads the upstream body to the end and decompresses
                // it, so a `content-length` that described the compressed bytes no
                // longer matches what is forwarded; the closing connection bounds it.
                let compressed = headers
                    .iter()
                    .any(|(key, _)| key.eq_ignore_ascii_case("content-encoding"));
                let reason = reqwest::StatusCode::from_u16(status)
                    .ok()
                    .and_then(|code| code.canonical_reason())
                    .unwrap_or("");
                let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
                for (key, value) in headers {
                    let lowered = key.to_ascii_lowercase();
                    if matches!(lowered.as_str(), "connection" | "transfer-encoding")
                        || (compressed && lowered == "content-length")
                    {
                        continue;
                    }
                    head.push_str(key);
                    head.push_str(": ");
                    head.push_str(value);
                    head.push_str("\r\n");
                }
                head.push_str("Connection: close\r\n\r\n");
                head_written = true;
                if let Ok(mut guard) = captured_head.lock() {
                    guard.extend_from_slice(head.as_bytes());
                }
                head.into_bytes()
            }
            TransportEvent::Body(chunk) => chunk.to_vec(),
        };
        let Ok(mut stream) = writer.lock() else {
            return false;
        };
        stream.write_all(&payload).is_ok() && stream.flush().is_ok()
    });
    match transport::send(spec, cancel, sink) {
        Ok(reply) => {
            let mut captured = head_bytes
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default();
            let take = reply
                .body
                .len()
                .min(MAX_CAPTURE_BYTES.saturating_sub(captured.len()));
            captured.extend_from_slice(&reply.body[..take]);
            if let Ok(mut stream) = client.lock() {
                let _ = stream.flush();
            }
            let record = build_record(
                &request,
                &request_value,
                &captured,
                capture_mode,
                request_id,
            );
            append_record(output_path, write_lock, &record);
        }
        Err(TransportError::Cancelled(observation)) => {
            // The upstream request went with the future, so the provider sees the
            // connection close instead of being paid for a finished generation.
            if observation.initiator == CancelInitiator::Peer {
                append_client_disconnected_record(
                    output_path,
                    write_lock,
                    request_id,
                    &request_value,
                );
            } else {
                append_cancelled_record(output_path, write_lock, request_id, &request_value);
            }
            append_record(
                output_path,
                write_lock,
                &json!({
                    "kind": "model_call_cancelled",
                    "requestId": request_id,
                    "recordedAt": chrono::Utc::now().to_rfc3339(),
                    "initiator": if observation.initiator == CancelInitiator::Peer { "peer" } else { "task" },
                    "cancelRequestedAt": observation.cancel_requested_at,
                    "transportClosedAt": observation.transport_closed_at,
                    "settleMillis": observation.settle_millis,
                }),
            );
        }
        Err(error) => {
            append_failure_record(
                output_path,
                write_lock,
                request_id,
                &request_value,
                502,
                &format!("无法连接云端 LLM 上游地址：{error:?}"),
            );
            if let Ok(mut stream) = client.lock() {
                write_error(&mut stream, 502, "无法连接云端 LLM 上游地址");
            }
        }
    }
}

/// One blocking read that tolerates a socket left non-blocking by its listener and
/// a request that arrives in several packets. An async client writes the head and
/// the body separately, so a single EAGAIN must never end a valid request.
fn read_chunk(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<usize, String> {
    loop {
        match stream.read(buffer) {
            Ok(count) => return Ok(count),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                if Instant::now() > deadline {
                    return Err("LLM 请求读取超时".to_string());
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 32 * 1024];
    let deadline = Instant::now() + Duration::from_secs(60);
    let header_end = loop {
        let count = read_chunk(stream, &mut buffer, deadline)?;
        if count == 0 {
            return Err("LLM 请求提前关闭".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("LLM 请求过大".into());
        }
        if let Some(position) = bytes.windows(4).position(|item| item == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or("缺少 LLM 请求行")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("POST").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    let length = headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    if length > MAX_REQUEST_BYTES {
        return Err("LLM 请求正文过大".into());
    }
    while bytes.len() < header_end + length {
        let count = read_chunk(stream, &mut buffer, deadline)?;
        if count == 0 {
            return Err("LLM 请求正文不完整".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(Request {
        method,
        path,
        headers,
        body: bytes[header_end..header_end + length].to_vec(),
    })
}

fn build_request(request: &Request, host: &str, api_key: &str) -> Vec<u8> {
    let mut output = format!("{} {} HTTP/1.1\r\n", request.method, request.path).into_bytes();
    for (key, value) in &request.headers {
        if key.eq_ignore_ascii_case("host")
            || key.eq_ignore_ascii_case("content-length")
            || key.eq_ignore_ascii_case("connection")
            || key.eq_ignore_ascii_case("accept-encoding")
            || key.eq_ignore_ascii_case("authorization")
        {
            continue;
        }
        output.extend_from_slice(format!("{key}: {value}\r\n").as_bytes());
    }
    if !api_key.is_empty() {
        output.extend_from_slice(format!("Authorization: Bearer {api_key}\r\n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "Host: {host}\r\nAccept-Encoding: identity\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            request.body.len()
        )
        .as_bytes(),
    );
    output.extend_from_slice(&request.body);
    output
}

fn build_record(
    request: &Request,
    request_value: &Value,
    response: &[u8],
    capture_mode: &str,
    request_id: &str,
) -> Value {
    let response_text = String::from_utf8_lossy(response);
    let status = response_text
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .unwrap_or("");
    let (body_bytes, _) = decode_response_body(response);
    let body = String::from_utf8_lossy(&body_bytes);
    let response_values = body
        .lines()
        .filter_map(|line| {
            let data = line.strip_prefix("data: ")?.trim();
            if data == "[DONE]" {
                None
            } else {
                serde_json::from_str::<Value>(data).ok()
            }
        })
        .collect::<Vec<_>>();
    let response_json = serde_json::from_str::<Value>(&body)
        .ok()
        .or_else(|| response_values.last().cloned())
        .unwrap_or_else(|| json!({}));
    let status_code = status.parse::<u16>().unwrap_or(0);
    let mut usage = response_values
        .iter()
        .rev()
        .find_map(response_usage)
        .or_else(|| response_usage(&response_json))
        .unwrap_or_else(|| json!({}));
    let mut estimated = number(&usage, &["total_tokens"]) <= 0
        && number(&usage, &["input_tokens", "prompt_tokens"]) <= 0
        && number(&usage, &["output_tokens", "completion_tokens"]) <= 0;
    if !(200..300).contains(&status_code) {
        // A rejected request did not consume a valid model turn. Do not turn
        // its request bytes into fake token usage or no-progress evidence.
        usage = json!({});
        estimated = false;
    } else if estimated {
        let input = ((request.body.len() as i64 + 3) / 4).max(1);
        let output = ((body.len() as i64 + 3) / 4).max(1);
        usage = json!({
            "input_tokens": input,
            "output_tokens": output,
            "total_tokens": input + output,
            "estimated": true,
        });
    }
    let mut hasher = Sha256::new();
    hasher.update(&request.body);
    let request_hash = format!("{:x}", hasher.finalize());
    let call_type = call_type_for_request(request_value);
    let mut record = json!({
        "kind": "model_call",
        "requestId": request_id,
        "callType": call_type,
        "recordedAt": chrono::Utc::now().to_rfc3339(),
        "path": request.path,
        "status": status,
        "model": request_value.get("model").cloned().unwrap_or(Value::Null),
        "stream": request_value.get("stream").cloned().unwrap_or(Value::Bool(false)),
        "usage": usage,
        "usageEstimated": estimated,
    });
    if !(200..300).contains(&status_code) {
        let error = response_json
            .pointer("/error/message")
            .or_else(|| response_json.get("error"))
            .and_then(Value::as_str)
            .unwrap_or_else(|| body.trim())
            .chars()
            .take(500)
            .collect::<String>();
        record["error"] = Value::String(error);
    }
    if capture_mode != "off" {
        record["requestHash"] = Value::String(request_hash);
        record["requestChars"] = Value::from(request.body.len() as i64);
        record["requestSummary"] = request_summary(request_value);
    }
    if capture_mode == "full" {
        record["request"] = bounded_capture(request_value);
        record["response"] = bounded_capture(&response_json);
    }
    record
}
