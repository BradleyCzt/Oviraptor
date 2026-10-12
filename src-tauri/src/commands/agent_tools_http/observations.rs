// A response is recorded before these deterministic findings are emitted.
// Borrow the exact recorded response so the extraction adds no body copies.
struct AgentHttpObservedResponse<'a> {
    request: &'a AgentHttpRequest,
    request_id: &'a str,
    url: &'a str,
    digest: &'a str,
    text: &'a str,
    content_type: &'a str,
    security_headers: &'a [(String, String)],
    cors_acao: &'a str,
    cors_acac: &'a str,
}

fn agent_http_observations(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    response: AgentHttpObservedResponse<'_>,
) -> Result<Vec<String>, JsonValue> {
    let AgentHttpObservedResponse {
        request,
        request_id,
        url,
        digest,
        text,
        content_type,
        security_headers,
        cors_acao,
        cors_acac,
    } = response;
    let mut emitted = Vec::new();
    for (name, value) in security_headers {
        if !agent_header_looks_versioned(name, value) {
            continue;
        }
        match persist_agent_observation_finding(
            context,
            runtime,
            request_id,
            url,
            name,
            value,
        ) {
            Ok(true) => emitted.push(format!("version_disclosure:{name}")),
            Ok(false) => {}
            Err(error) => {
                return Err(serde_json::json!({
                    "error": error,
                    "code": "finding_persist_failed",
                }));
            }
        }
    }
    if let Some(excerpt) = agent_detect_stack_trace_leak(text) {
        match persist_agent_observation(
            context,
            runtime,
            &format!("stack:{}", &digest[..12.min(digest.len())]),
            ".NET/应用堆栈跟踪信息泄露",
            "low",
            "CWE-209",
            "stack_trace_leak",
            request_id,
            url,
            &excerpt,
            "详细堆栈与框架路径帮助攻击者定位可利用点与组件版本",
            "生产环境关闭自定义错误详细信息，避免向客户端返回堆栈",
            "error_handling",
        ) {
            Ok(true) => emitted.push("stack_trace_leak".into()),
            Ok(false) => {}
            Err(error) => {
                return Err(serde_json::json!({
                    "error": error,
                    "code": "finding_persist_failed",
                }));
            }
        }
    }
    if let Some((ver, script_url)) = agent_outdated_jquery_from_url(url) {
        match persist_agent_observation(
            context,
            runtime,
            &format!("jquery:{ver}"),
            &format!("jQuery {ver} 存在已知 XSS 风险（已停止维护/含公开漏洞）"),
            "low",
            "CWE-79",
            "outdated_jquery",
            request_id,
            &script_url,
            &format!("script url declares jquery-{ver}"),
            "旧版 jQuery 含多个已公开 XSS 相关缺陷，可在页面上下文被利用",
            "升级到 jQuery 3.5+ 或迁移至已维护的替代库，并移除页面中的旧副本",
            "input_reflection_xss",
        ) {
            Ok(true) => emitted.push(format!("jquery:{ver}")),
            Ok(false) => {}
            Err(error) => {
                return Err(serde_json::json!({
                    "error": error,
                    "code": "finding_persist_failed",
                }));
            }
        }
    }
    let sent_origin = request
        .extra_headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("origin"))
        .map(|(_, v)| v.as_str())
        .unwrap_or("");
    if !cors_acao.is_empty() {
        let reflected = !sent_origin.is_empty() && cors_acao == sent_origin;
        let wildcard = cors_acao.trim() == "*";
        if wildcard || reflected {
            let detail = if wildcard {
                format!("Access-Control-Allow-Origin: * (Origin probe={sent_origin})")
            } else {
                format!(
                    "Access-Control-Allow-Origin reflects {cors_acao}; Allow-Credentials={cors_acac}"
                )
            };
            match persist_agent_observation(
                context,
                runtime,
                &format!("cors:{cors_acao}:{cors_acac}"),
                "CORS 配置过于宽松",
                "medium",
                "CWE-942",
                "cors_misconfig",
                request_id,
                url,
                &detail,
                "跨源页面可读取本站响应；若同时允许凭据，风险更高",
                "收紧 Access-Control-Allow-Origin 白名单，避免反射任意 Origin；慎用 Allow-Credentials",
                "authorization",
            ) {
                Ok(true) => emitted.push("cors_misconfig".into()),
                Ok(false) => {}
                Err(error) => {
                    return Err(serde_json::json!({
                        "error": error,
                        "code": "finding_persist_failed",
                    }));
                }
            }
        }
    }
    if let Some(excerpt) = agent_detect_client_md5_password(text) {
        match persist_agent_observation(
            context,
            runtime,
            "md5-login",
            "登录口令在客户端使用 MD5 后提交",
            "medium",
            "CWE-916",
            "client_md5_login",
            request_id,
            url,
            &excerpt,
            "弱哈希可被离线碰撞/彩虹表攻击，且等同于传输可逆口令摘要",
            "改为服务端慢哈希（如 Argon2/bcrypt）并配合 TLS；勿在浏览器预哈希代替服务端保护",
            "authorization",
        ) {
            Ok(true) => emitted.push("client_md5_login".into()),
            Ok(false) => {}
            Err(error) => {
                return Err(serde_json::json!({
                    "error": error,
                    "code": "finding_persist_failed",
                }));
            }
        }
    }
    let looks_html = text.trim_start().starts_with('<')
        || text.to_ascii_lowercase().contains("<script");
    if looks_html {
        for src in agent_extract_script_srcs(text) {
            let Some(abs) = agent_resolve_against(url, &src) else {
                continue;
            };
            if let Some((ver, script_url)) = agent_outdated_jquery_from_url(&abs) {
                match persist_agent_observation(
                    context,
                    runtime,
                    &format!("jquery:{ver}"),
                    &format!("jQuery {ver} 存在已知 XSS 风险（已停止维护/含公开漏洞）"),
                    "low",
                    "CWE-79",
                    "outdated_jquery",
                    request_id,
                    &script_url,
                    &format!("HTML script src declares jquery-{ver}"),
                    "旧版 jQuery 含多个已公开 XSS 相关缺陷，可在页面上下文被利用",
                    "升级到 jQuery 3.5+ 或迁移至已维护的替代库，并移除页面中的旧副本",
                    "input_reflection_xss",
                ) {
                    Ok(true) => emitted.push(format!("jquery:{ver}")),
                    Ok(false) => {}
                    Err(error) => {
                        return Err(serde_json::json!({
                            "error": error,
                            "code": "finding_persist_failed",
                        }));
                    }
                }
            }
        }
    }
    let is_account_enum_probe = request.contract_key.ends_with(":account-enum-probe");
    if !is_account_enum_probe && agent_looks_like_account_check(url) {
        if let Some(token_a) = agent_membership_token(text) {
            if !runtime.account_enum_probed {
                runtime.account_enum_probed = true;
                if let Some(probe_url) = agent_account_probe_url(url) {
                    if probe_url != url {
                        let probe_request = AgentHttpRequest {
                            identity: request.identity.clone(),
                            method: request.method.clone(),
                            url: probe_url.clone(),
                            extra_headers: request.extra_headers.clone(),
                            body: None,
                            content_type: request.content_type.clone(),
                            contract_key: format!("{}:account-enum-probe", request.contract_key),
                            family: request.family.clone(),
                            source: request.source,
                            tool: request.tool.clone(),
                            timeout_seconds: request.timeout_seconds,
                        };
                        if let Ok(probe_view) = agent_http_exchange(context, runtime, &probe_request)
                        {
                            let mut token_b = None;
                            for key in ["preview", "bodyPreview", "textPreview", "bodySnippet"] {
                                if let Some(s) = probe_view.get(key).and_then(|v| v.as_str()) {
                                    if let Some(t) = agent_membership_token(s) {
                                        token_b = Some(t);
                                        break;
                                    }
                                }
                            }
                            if let Some(token_b) = token_b {
                                if token_b != token_a {
                                    match persist_agent_observation(
                                        context,
                                        runtime,
                                        "account-enum",
                                        "账号存在性可被枚举",
                                        "medium",
                                        "CWE-203",
                                        "user_enumeration",
                                        request_id,
                                        url,
                                        &format!(
                                            "control={token_a} probe={token_b} probeUrl={probe_url}"
                                        ),
                                        "攻击者可批量探测有效账号，降低撞库与定向攻击成本",
                                        "对存在/不存在账号返回相同响应；配合速率限制与通用错误文案",
                                        "authorization",
                                    ) {
                                        Ok(true) => emitted.push("user_enumeration".into()),
                                        Ok(false) => {}
                                        Err(error) => {
                                            return Err(serde_json::json!({
                                                "error": error,
                                                "code": "finding_persist_failed",
                                            }));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let looks_code = content_type.contains("javascript")
        || content_type.contains("ecmascript")
        || url.to_ascii_lowercase().contains(".js")
        || text.trim_start().starts_with('<')
        || text.to_ascii_lowercase().contains("<script")
        || text.to_ascii_lowercase().contains("$.ajax")
        || text.to_ascii_lowercase().contains("url:");
    if looks_code {
        let mut added = 0usize;
        for (method, path) in agent_harvest_surface_paths(text, url) {
            if runtime.discovered_api_keys.len() >= 80 || added >= 40 {
                break;
            }
            match persist_agent_surface_api(context, runtime, &method, &path, url) {
                Ok(true) => {
                    added += 1;
                    emitted.push(format!("surface:{method}:{path}"));
                }
                Ok(false) => {}
                Err(error) => {
                    return Err(serde_json::json!({
                        "error": error,
                        "code": "finding_persist_failed",
                    }));
                }
            }
        }
        // U98: click-explore entry is called so mode flips later without a new call site.
        // string_only → surface_click_explore_not_ready (expected, not a fault).
        match native_surface_click_explore_candidates(text, url) {
            Ok(extra) => {
                for (method, path) in extra {
                    if runtime.discovered_api_keys.len() >= 80 || added >= 40 {
                        break;
                    }
                    match persist_agent_surface_api(context, runtime, &method, &path, url) {
                        Ok(true) => {
                            added += 1;
                            emitted.push(format!("surface-click:{method}:{path}"));
                        }
                        Ok(false) => {}
                        Err(error) => {
                            return Err(serde_json::json!({
                                "error": error,
                                "code": "finding_persist_failed",
                            }));
                        }
                    }
                }
            }
            Err(code) if code == "surface_click_explore_not_ready" => {}
            Err(error) => {
                return Err(serde_json::json!({
                    "error": error,
                    "code": "surface_click_explore_failed",
                }));
            }
        }
    }
    // Also register the current non-static request itself as surface inventory.
    {
        let path = normalized_investigation_path(url);
        if !path.is_empty()
            && path != "/"
            && !agent_is_static_surface_path(&path)
            && matches!(request.method.as_str(), "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD")
        {
            persist_agent_surface_api(context, runtime, &request.method, &path, "observed-request")
                .map_err(|error| serde_json::json!({
                    "error": error,
                    "code": "finding_persist_failed",
                }))?;
        }
    }

    Ok(emitted)
}
