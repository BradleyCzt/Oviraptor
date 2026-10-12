use super::*;

include!("protection_witness.rs");
include!("protection_fresh.rs");
include!("protection_capture.rs");

pub(super) fn agent_http_exchange(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    request: &AgentHttpRequest,
) -> Result<JsonValue, JsonValue> {
    if let Some(stop) = &runtime.target_request_stop {
        return Err(stop.clone());
    }
    let result = agent_http_exchange_inner(context, runtime, request);
    if let Err(error) = &result {
        if agent_tool_terminal_outcome(error).is_some() {
            runtime.target_request_stop = Some(error.clone());
        }
    }
    result
}

fn agent_http_exchange_inner(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    request: &AgentHttpRequest,
) -> Result<JsonValue, JsonValue> {
    // A single model tool can fan out into several HTTP exchanges (identity
    // comparison, discovery, browser actions). Re-check the lease, task-bound
    // identities, cancellation and request ceiling at the actual send boundary:
    // the check at tool dispatch cannot authorize later requests in the batch.
    agent_authorize_tool(context, &request.tool)
        .map_err(|code| serde_json::json!({"error":"目标请求权限已失效","code":code}))?;
    agent_single_require_fresh(context).map_err(agent_http_claim_error)?;
    if runtime.cancelled() {
        return Err(serde_json::json!({"error":"任务已暂停或取消","code":"cancelled"}));
    }
    if runtime.protected_stop.is_none() {
        let persisted = db::open(&context.db_path)
            .and_then(|db| agent_protection_stop_on(&db, context));
        match persisted {
            Ok(stop) => runtime.protected_stop = stop,
            Err(_) => return Err(serde_json::json!({"error":"无法核验目标的持久保护状态","code":"tool_authorization_unavailable"})),
        }
    }
    if let Some(stop) = &runtime.protected_stop {
        return Err(serde_json::json!({"code":"protected","stop":stop}));
    }
    if runtime.target_requests >= agent_target_request_ceiling(context) {
        return Err(
            serde_json::json!({"error":"目标请求总数已达到本轮硬上限","code":"request_budget_exhausted"}),
        );
    }
    if AGENT_DENY_METHODS.contains(&request.method.as_str()) {
        return Err(serde_json::json!({
            "error": format!("{} 不允许执行；请先从调用点、表单或实际运行时恢复 method", request.method),
            "code": "unknown_method",
        }));
    }
    let (url, scope_class) =
        agent_scope_check(context, &request.url, &request.method, request.source)
            .map_err(|error| serde_json::json!({"error": error, "code": "scope_denied"}))?;
    let headers = agent_http_wire_headers(context, request).map_err(
        |error| serde_json::json!({"error": error, "code": "identity_session_unavailable"}),
    )?;
    let mut client_builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(request.timeout_seconds.clamp(5, 30)));
    if let Some(proxy) = &context.proxy {
        client_builder = match reqwest::Proxy::all(proxy) {
            Ok(parsed) => client_builder.proxy(parsed),
            Err(error) => {
                return Err(
                    serde_json::json!({"error": error.to_string(), "code": "proxy_invalid"}),
                )
            }
        };
    }
    let client = client_builder.build().map_err(
        |error| serde_json::json!({"error": error.to_string(), "code": "client_failed"}),
    )?;
    let mut call = client
        .request(
            reqwest::Method::from_bytes(request.method.as_bytes())
                .map_err(|_| serde_json::json!({"error":"无效 method","code":"bad_method"}))?,
            &url,
        )
        .header(reqwest::header::USER_AGENT, "oviraptor-native-agent/1.0");
    for (name, value) in &headers {
        if let Ok(parsed) = name.parse::<reqwest::header::HeaderName>() {
            call = call.header(parsed, value.clone());
        }
    }
    if let Some(body) = request.body.as_deref().filter(|value| !value.is_empty()) {
        call = call.body(body.to_string());
    }
    // Commit the request claim before any send. The journal is authoritative
    // even if the process dies before finish_tool or the Native checkpoint.
    let mut sealed_transport = claim_agent_http_sealed_transport_with_headers(context, runtime, request, &url, &headers)
        .map_err(agent_http_claim_error)?;
    let claim = sealed_transport.claim();
    runtime.spend_request();
    runtime.current_request_index += 1;
    if let Some(run) = &context.run {
        let timeout = db::open(&context.db_path)
            .and_then(|db| {
                let ceiling = Duration::from_secs(request.timeout_seconds.clamp(5, 30));
                match claim.and_then(|claim| claim.root_target.as_ref()) {
                    Some(original) => original.transport_timeout(&db, ceiling),
                    None => crate::agent_runtime::multi_agent::budget::target::transport_timeout(
                        &db,
                        &run.run_id,
                        ceiling,
                    ),
                }
            })
            .map_err(agent_http_claim_error)?;
        call = call.timeout(timeout);
    }
    let pre_send = protection_pre_send_witness(context, request, claim)
        .map_err(|_| json!({"code":terminal_code::REQUEST_RECONCILIATION_REQUIRED,
            "phase":"before_send","error":"原发送授权无法冻结；已占用请求不得重放"}))?;
    let started = Instant::now();
    let mut response = call.send().map_err(|error| {
        if claim.is_some() {
            // A transport error does not prove that the target did not execute
            // the request. Do not expose the reqwest URL (which may be secret).
            serde_json::json!({"error":"请求已占用预算，但未取得响应回执；目标效果未知",
                "code":terminal_code::REQUEST_RECONCILIATION_REQUIRED,"phase":"awaiting_headers"})
        } else {
            serde_json::json!({"error": error.to_string(), "code": "request_failed"})
        }
    })?;
    let status = response.status().as_u16();
    if claim.is_some() {
        sealed_transport.receive_headers(context, status)
            .map_err(|reason| serde_json::json!({"error":reason,"code":"evidence_write_failed"}))?;
    }
    let claim = sealed_transport.claim();
    // A saved original bill is not permission to use a late response.
    agent_single_require_fresh(context).map_err(agent_http_claim_error)?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let security_headers = agent_security_relevant_headers(response.headers());
    let cors_acao = response
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let cors_acac = response
        .headers()
        .get("access-control-allow-credentials")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let header_names: Vec<String> = response
        .headers()
        .keys()
        .map(|key| key.as_str().to_string())
        .collect();
    let redirect_target = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parsed_redirect(&url, value));
    let declared_bytes = response.content_length();
    // Read at most the cap: an archive or a stream that never ends must not be
    // buffered whole before it is thrown away.
    use std::io::Read;
    let mut body: Vec<u8> = Vec::new();
    let mut chunk = [0u8; AGENT_RESPONSE_CHUNK_BYTES];
    let mut read_failure: Option<String> = None;
    let mut truncated_early = false;
    loop {
        let room = AGENT_MAX_RESPONSE_BYTES.saturating_sub(body.len());
        if room == 0 {
            break;
        }
        if runtime.cancelled() {
            // Stop pulling bytes; dropping the reader closes the transfer.
            truncated_early = true;
            break;
        }
        let take = room.min(chunk.len());
        match response.read(&mut chunk[..take]) {
            Ok(0) => break,
            Ok(size) => body.extend_from_slice(&chunk[..size]),
            Err(error) => {
                read_failure = Some(error.to_string());
                break;
            }
        }
    }
    if let Some(error) = read_failure {
        if claim.is_some() {
            return Err(
                serde_json::json!({"error":"已收到响应头，但响应体未完整读取；不能据此确认执行结果",
                "code":terminal_code::REQUEST_RECONCILIATION_REQUIRED,"phase":"reading_body"}),
            );
        }
        return Err(serde_json::json!({"error": error, "code": "body_failed"}));
    }
    let captured = body.len();
    let truncated = truncated_early
        || match declared_bytes {
            Some(declared) => declared > captured as u64,
            // Filling the cap with no declared length means bytes may still be on
            // the wire; one probe byte tells the difference from an exact fit.
            None if captured < AGENT_MAX_RESPONSE_BYTES => false,
            None => {
                let mut probe = [0u8; 1];
                matches!(response.read(&mut probe), Ok(1))
            }
        };
    let elapsed_ms = started.elapsed().as_millis();
    let mut hasher = Sha256::new();
    hasher.update(&body);
    let digest = format!("{:x}", hasher.finalize());
    let text = String::from_utf8_lossy(&body).to_string();
    let structure = agent_response_structure(&text, &content_type);
    agent_single_require_fresh(context).map_err(agent_http_claim_error)?;
    runtime.record_request(AgentRequestTrace {
        method: request.method.to_ascii_uppercase(),
        origin: agent_origin_of(&url),
        path: normalized_investigation_path(
            &reqwest::Url::parse(&url)
                .map(|parsed| parsed.path().to_string())
                .unwrap_or_else(|_| url.clone()),
        ),
        identity: request.identity.key.clone(),
        status: status as i64,
        family: request.family.clone(),
        contract_key: request.contract_key.clone(),
        tool: request.tool.clone(),
        scope_class: scope_class.as_str().to_string(),
        parameters: agent_request_parameters(&url, request.body.as_deref()),
        ..Default::default()
    });
    // §5.4: a 3xx is only ever followed by the browser, and even there an
    // identity-provider jump is observation rather than a test. Out of the frozen
    // scope it stops the action outright.
    let redirect_code = match status {
        301 | 302 | 303 | 307 | 308 => redirect_target
            .as_deref()
            .map(|target| agent_redirect_verdict(context, target, &url))
            .unwrap_or_else(|| "redirect_without_location".to_string()),
        _ => String::new(),
    };
    let protected_response = ProtectionCapturedResponse {
        url: &url, headers: &headers, body: &body, status,
        content_type: &content_type, truncated,
    };
    if status != 404
        && is_directory_block_signal(&format!(
            "http {status} {content_type} {}",
            text.chars().take(4_000).collect::<String>()
        ))
    {
        let stop = AgentStop::new(
            AGENT_STOP_WAF,
            format!("目标返回明确的 WAF、验证码或机器人挑战信号（HTTP {status}）"),
        );
        runtime.protected_stop = Some(stop.clone());
        return Err(capture_protected_http_response(
            context, runtime, request, claim, pre_send.as_ref(), protected_response, stop,
        ));
    }
    // §11: the site keeps refusing. One 429 is a throttle to work around; two in a
    // row inside the same attempt means this run will not be answered, so it stops
    // as a protection signal rather than burning the remaining budget.
    if status == 429 {
        runtime.rate_limited_streak += 1;
        if runtime.rate_limited_streak >= AGENT_PERSISTENT_RATE_LIMITS {
            let stop = AgentStop::new(
                AGENT_STOP_RATE_LIMIT,
                format!(
                    "目标持续限流（连续 {} 次 HTTP 429），本轮不再继续消耗预算",
                    runtime.rate_limited_streak
                ),
            );
            runtime.protected_stop = Some(stop.clone());
            return Err(capture_protected_http_response(
                context, runtime, request, claim, pre_send.as_ref(), protected_response, stop,
            ));
        }
    } else {
        runtime.rate_limited_streak = 0;
    }
    let summary = serde_json::json!({
        "identity": request.identity.key,
        "method": request.method,
        "url": url,
        "status": status,
        "contentType": content_type,
        "bytes": captured,
        "declaredBytes": declared_bytes,
        "maxResponseBytes": AGENT_MAX_RESPONSE_BYTES as u64,
        "elapsedMs": elapsed_ms,
        "bodySha256": digest,
        "truncated": truncated,
        "responseHeaderNames": header_names,
        "securityRelevantHeaders": security_headers
            .iter()
            .map(|(name, value)| serde_json::json!({"name": name, "value": value}))
            .collect::<Vec<_>>(),
        "structureKeys": structure,
        "fields": agent_field_fingerprints(&text, &content_type, &runtime.redaction),
        "preview": agent_text_truncated(&text, AGENT_MAX_PREVIEW_CHARS),
        "contractKey": request.contract_key,
        "requestId": runtime
            .requests
            .last()
            .map(|trace| trace.id.clone())
            .unwrap_or_default(),
        "scopeClass": scope_class.as_str(),
        "redirectCode": redirect_code,
        "redirectTarget": redirect_target,
        // A cached answer is not evidence about the live authorization behaviour.
        "cacheState": agent_cache_state(&headers),
        "pathMissing": status == 404,
        "note": if status == 404 {
            "这条路径不存在。继续队列里已经发现的接口和入口，不要因为这一次 404 结束目标。"
        } else {
            ""
        },
    });
    // §9.3: an input that comes back verbatim is recorded as a reflection point
    // before the field fingerprints are replaced by markers.
    let reflected = agent_reflects_input(
        request,
        summary.get("fields").and_then(JsonValue::as_object),
    );
    // §6.1: the model view is derived first, so the audit record's metadata holds
    // the same redacted facts the model saw and the untouched bytes stay in the
    // payload files next to it.
    let mut view = agent_model_view(summary, runtime);
    // A03: only the actual transport can supply this durable claim identity.
    // It is captured in the hashed artifact, never reconstructed from a graph row.
    if let Some(claim) = &claim {
        view["httpClaim"] = json!({"schemaVersion":1,"runId":claim.run_id,
            "invocationId":claim.invocation_id,"requestIndex":claim.request_index,
            "budgetAttempt":claim.budget_attempt,"ordinal":claim.ordinal,
            "requestHash":claim.request_hash});
    }

    let artifact_id = match agent_write_http_record(
        context,
        runtime.target_requests,
        &serde_json::json!({
            "identity": request.identity.key,
            "method": request.method,
            "url": url,
            "headers": headers.iter().map(|(name, value)| serde_json::json!({"name": name, "value": value})).collect::<Vec<_>>(),
            "body": request.body,
            "contractKey": request.contract_key,
        }),
        &view,
        &body,
    ) {
        Ok(artifact) => artifact,
        Err(error) => {
            return Err(serde_json::json!({"error": error, "code": "evidence_write_failed"}))
        }
    };
    if let Err(error) = agent_record_http_observation(
        context,
        &request.tool,
        &artifact_id,
        &view,
        runtime
            .requests
            .last()
            .map(|trace| trace.id.as_str())
            .unwrap_or(""),
    ) {
        return Err(agent_http_claim_error(error));
    }
    runtime.note_artifact(&artifact_id);
    runtime.attach_artifact(&artifact_id, &digest[..12]);
    // §9.2: the family a request was executed for is credited here, from the
    // ledger's own facts — never from what the model said.
    let request_id = runtime
        .requests
        .last()
        .map(|trace| trace.id.clone())
        .unwrap_or_default();
    if reflected && !request_id.is_empty() {
        runtime.reflections.insert(request_id.clone());
    }
    runtime.credit_coverage(
        &request.family,
        "request",
        &request.contract_key,
        std::slice::from_ref(&request_id),
    );
    let emitted = agent_http_observations(
        context,
        runtime,
        AgentHttpObservedResponse {
            request,
            request_id: &request_id,
            url: &url,
            digest: &digest,
            text: &text,
            content_type: &content_type,
            security_headers: &security_headers,
            cors_acao: &cors_acao,
            cors_acac: &cors_acac,
        },
    )?;
    if let Some(object) = view.as_object_mut() {
        object.insert(
            "rawArtifactId".to_string(),
            JsonValue::String(artifact_id.clone()),
        );
        if !emitted.is_empty() {
            object.insert(
                "observationFindings".to_string(),
                serde_json::json!(emitted),
            );
        }
    }
    Ok(view)
}

include!("protection_witness_tests.rs");
