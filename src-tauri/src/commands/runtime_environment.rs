fn platform_user_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            let drive = std::env::var_os("HOMEDRIVE")?;
            let path = std::env::var_os("HOMEPATH")?;
            Some(PathBuf::from(drive).join(path))
        })
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct ModelRuntimeEnv {
    llm: String,
    api_key: String,
    api_base: String,
    deployment: String,
    full_power: bool,
    prompt_audit_mode: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalModelRuntimePolicy {
    parameter_billions: Option<u32>,
    unified_memory_gb: u64,
    max_concurrent_requests: usize,
    max_context_tokens: u64,
    max_output_tokens: Option<u64>,
    frontend_packet_budget_bytes: usize,
    memory_guard_tier: &'static str,
    startup_idle_seconds: u64,
    startup_hard_seconds: u64,
}

fn model_parameter_billions(model: &str) -> Option<u32> {
    model
        .to_ascii_lowercase()
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '.'))
        .filter_map(|token| token.strip_suffix('b'))
        .filter_map(|number| number.parse::<f64>().ok())
        .filter(|number| *number >= 1.0 && *number <= 1000.0)
        .map(f64::ceil)
        .map(|number| number as u32)
        .max()
}

fn system_unified_memory_gb() -> u64 {
    if cfg!(target_os = "macos") {
        if let Ok(output) = Command::new("sysctl").args(["-n", "hw.memsize"]).output() {
            if output.status.success() {
                if let Ok(bytes) = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<u64>()
                {
                    return ((bytes + (1 << 30) - 1) >> 30).max(1);
                }
            }
        }
    }
    32
}

fn local_model_runtime_policy_for_memory(
    environment: &ModelRuntimeEnv,
    unified_memory_gb: u64,
) -> LocalModelRuntimePolicy {
    if environment.deployment != "local" {
        return LocalModelRuntimePolicy {
            parameter_billions: None,
            unified_memory_gb,
            max_concurrent_requests: 4,
            max_context_tokens: 0,
            max_output_tokens: None,
            frontend_packet_budget_bytes: 24 * 1024,
            memory_guard_tier: "balanced",
            startup_idle_seconds: 90,
            startup_hard_seconds: 300,
        };
    }
    let parameter_billions = model_parameter_billions(&environment.llm);
    let size = parameter_billions.unwrap_or(20);
    let max_context_tokens = match unified_memory_gb {
        0..=18 if size <= 10 => 49_152,
        0..=18 if size <= 20 => 24_576,
        0..=18 => 16_384,
        19..=24 if size <= 10 => 65_536,
        19..=24 if size <= 20 => 49_152,
        19..=24 => 32_768,
        25..=32 if size <= 10 => 65_536,
        25..=32 if size <= 20 => 57_344,
        25..=32 if size <= 39 => 49_152,
        25..=32 => 32_768,
        33..=48 if size <= 10 => 98_304,
        33..=48 if size <= 20 => 65_536,
        33..=48 if size <= 39 => 57_344,
        33..=48 => 49_152,
        _ if size <= 10 => 131_072,
        _ if size <= 20 => 98_304,
        _ if size <= 39 => 65_536,
        _ if size <= 70 => 49_152,
        _ => 32_768,
    };
    let (max_output_tokens, startup_idle_seconds, startup_hard_seconds) = match parameter_billions {
        Some(size) if size >= 60 => (3_072, 300, 1_800),
        Some(size) if size >= 20 => (3_072, 240, 1_200),
        Some(size) if size >= 13 => (3_072, 210, 1_200),
        Some(_) => (2_048, 180, 900),
        None => (3_072, 240, 1_200),
    };
    let memory_guard_tier = if unified_memory_gb <= 18 && size <= 10 {
        // A 9B Q4 model plus the native Agent's large first-turn schema sits just above
        // oMLX Balanced on a 16 GB Mac. Aggressive still remains below the
        // physical Metal ceiling and keeps the guard enabled.
        "aggressive"
    } else {
        "balanced"
    };
    LocalModelRuntimePolicy {
        parameter_billions,
        unified_memory_gb,
        // Full-power increases investigation budget, never simultaneous local
        // prefills. Serializing requests prevents the CPU/Metal spike seen on
        // 27B and 35B machines.
        max_concurrent_requests: 1,
        max_context_tokens,
        max_output_tokens: Some(max_output_tokens),
        frontend_packet_budget_bytes: if unified_memory_gb <= 18 {
            6 * 1024
        } else if unified_memory_gb <= 32 {
            8 * 1024
        } else {
            12 * 1024
        },
        memory_guard_tier,
        startup_idle_seconds,
        startup_hard_seconds,
    }
}

fn local_model_runtime_policy(environment: &ModelRuntimeEnv) -> LocalModelRuntimePolicy {
    local_model_runtime_policy_for_memory(environment, system_unified_memory_gb())
}

fn set_json_field(root: &mut JsonValue, section: &str, key: &str, value: JsonValue) {
    if !root.get(section).is_some_and(JsonValue::is_object) {
        root[section] = serde_json::json!({});
    }
    if let Some(object) = root.get_mut(section).and_then(JsonValue::as_object_mut) {
        object.insert(key.to_string(), value);
    }
}

fn omlx_origin_for_environment(
    settings: &JsonValue,
    environment: &ModelRuntimeEnv,
) -> Option<String> {
    if environment.deployment != "local" {
        return None;
    }
    let url = reqwest::Url::parse(&environment.api_base).ok()?;
    let host = url.host_str()?;
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return None;
    }
    let configured_port = settings
        .pointer("/server/port")
        .and_then(JsonValue::as_u64)
        .unwrap_or(8000) as u16;
    let actual_port = url.port_or_known_default()?;
    if actual_port != configured_port {
        return None;
    }
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    Some(format!("{}://{host}:{actual_port}", url.scheme()))
}

fn persist_omlx_settings(path: &Path, settings: &JsonValue) -> Result<(), String> {
    let temporary = path.with_extension(format!("oviraptor-{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.write_all(b"\n").map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

fn hot_apply_omlx_policy(
    origin: &str,
    settings: &JsonValue,
    payload: &JsonValue,
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    let skip_auth = settings
        .pointer("/auth/skip_api_key_verification")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let cookie = if skip_auth {
        None
    } else {
        let api_key = settings
            .pointer("/auth/api_key")
            .and_then(JsonValue::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "oMLX 未配置管理 API Key，策略已落盘但需要重启 oMLX 生效".to_string())?;
        let response = client
            .post(format!("{origin}/admin/api/login"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::json!({"api_key":api_key,"remember":false}).to_string())
            .send()
            .map_err(|error| format!("oMLX 管理接口不可用：{error}"))?;
        if !response.status().is_success() {
            return Err(format!("oMLX 管理登录返回 {}", response.status()));
        }
        response
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::to_string)
    };
    let mut request = client
        .post(format!("{origin}/admin/api/global-settings"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload.to_string());
    if let Some(cookie) = cookie {
        request = request.header(reqwest::header::COOKIE, cookie);
    }
    let response = request
        .send()
        .map_err(|error| format!("oMLX 策略热更新失败：{error}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("oMLX 策略热更新返回 {}", response.status()))
    }
}

fn apply_omlx_local_resource_policy(
    environment: &ModelRuntimeEnv,
) -> Result<Option<String>, String> {
    let Some(home) = platform_user_home() else {
        return Ok(None);
    };
    let path = home.join(".omlx/settings.json");
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&path).map_err(|error| error.to_string())?;
    let mut settings = serde_json::from_slice::<JsonValue>(&bytes)
        .map_err(|error| format!("oMLX settings.json 无法解析：{error}"))?;
    let Some(origin) = omlx_origin_for_environment(&settings, environment) else {
        return Ok(None);
    };
    let policy = local_model_runtime_policy(environment);
    let original = settings.clone();
    set_json_field(
        &mut settings,
        "memory",
        "prefill_memory_guard",
        JsonValue::Bool(true),
    );
    set_json_field(
        &mut settings,
        "memory",
        "memory_guard_tier",
        JsonValue::String(policy.memory_guard_tier.into()),
    );
    set_json_field(
        &mut settings,
        "scheduler",
        "max_concurrent_requests",
        JsonValue::from(policy.max_concurrent_requests as u64),
    );
    set_json_field(
        &mut settings,
        "scheduler",
        "chunked_prefill",
        JsonValue::Bool(true),
    );
    set_json_field(
        &mut settings,
        "scheduler",
        "prefill_priority",
        JsonValue::String("context".into()),
    );
    set_json_field(
        &mut settings,
        "sampling",
        "max_context_window",
        JsonValue::from(policy.max_context_tokens),
    );
    set_json_field(
        &mut settings,
        "sampling",
        "max_context_window_policy",
        JsonValue::from(policy.max_context_tokens),
    );
    if let Some(max_output_tokens) = policy.max_output_tokens {
        set_json_field(
            &mut settings,
            "sampling",
            "max_tokens",
            JsonValue::from(max_output_tokens),
        );
    }
    if settings != original {
        persist_omlx_settings(&path, &settings)?;
    }
    let live_payload = serde_json::json!({
        "memory_prefill_memory_guard": true,
        "memory_guard_tier": policy.memory_guard_tier,
        "max_concurrent_requests": policy.max_concurrent_requests,
        "chunked_prefill": true,
        "prefill_priority": "context",
        "sampling_max_context_window": policy.max_context_tokens,
        "sampling_max_context_window_policy": policy.max_context_tokens,
        "sampling_max_tokens": policy.max_output_tokens
    });
    hot_apply_omlx_policy(&origin, &settings, &live_payload)?;
    Ok(Some(format!(
        "oMLX 自动资源策略：{} GB / {}B · 输入 {} Token · 输出 {} Token · 单并发 · {} 门禁 · Context 预填充",
        policy.unified_memory_gb,
        policy.parameter_billions.unwrap_or(0),
        policy.max_context_tokens,
        policy.max_output_tokens.unwrap_or(0),
        policy.memory_guard_tier
    )))
}

fn profile_with_id<'a>(
    profiles: Option<&'a JsonValue>,
    active_id: Option<&JsonValue>,
) -> Option<&'a JsonValue> {
    let profiles = profiles?.as_array()?;
    if profiles.is_empty() {
        return None;
    }
    let active_id = active_id
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    profiles
        .iter()
        .find(|profile| {
            !active_id.is_empty()
                && profile.get("id").and_then(JsonValue::as_str) == Some(active_id.as_str())
        })
        .or_else(|| profiles.first())
}

/// Settings aliases are normalized at the migration boundary, not selected by
/// the model runtime. Only the neutral profile list is authoritative here.
fn active_model_profile(settings: &JsonValue) -> Option<&JsonValue> {
    profile_with_id(
        settings.get("modelProfiles"),
        settings.get("activeModelProfileId"),
    )
}

fn model_runtime_env(settings: &JsonValue) -> Result<ModelRuntimeEnv, String> {
    let normalized = db::normalize_settings(settings);
    let settings = &normalized;
    let active_profile = active_model_profile(settings);
    if settings.get("modelProfiles").is_some() && active_profile.is_none() {
        return Err("模型未配置：模型列表为空或无效，请在配置中心选择模型后再启动".into());
    }
    // Once a profile is selected it owns the whole endpoint configuration.
    // Empty/missing fields must not borrow a previous profile's flat cache.
    let profile_value = |key: &str, neutral_key: &str| match active_profile {
        Some(profile) => profile.get(key),
        None => settings.get(neutral_key),
    };
    // A profile is normal, but a settings blob that only carries the flat neutral
    // keys must still be able to say which side it belongs to (§8.1).
    let deployment = profile_value("deployment", "modelDeployment")
        .and_then(JsonValue::as_str)
        .filter(|value| *value == "local")
        .unwrap_or("cloud")
        .to_string();
    let full_power = deployment == "local"
        && settings
            .get("agentLocalFullPower")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
    let prompt_audit_mode = match settings
        .get("agentPromptAuditMode")
        .and_then(JsonValue::as_str)
        .unwrap_or("off")
    {
        "metadata" => "metadata",
        "full" => "full",
        _ => "off",
    }
    .to_string();
    let profile_setting = |key: &str, neutral_key: &str| {
        profile_value(key, neutral_key)
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let llm = profile_setting("llm", "modelName");
    if llm.is_empty() {
        return Err(
            "模型未配置：请在配置中心填写当前模型名称和 API 地址后再启动。任务尚未发出任何请求"
                .into(),
        );
    }
    let api_key = if deployment == "local" {
        // Keep local authentication separate from a profile's cloud key. This
        // avoids sending a hidden stale cloud credential to localhost while
        // still supporting self-hosted gateways that require their own token.
        // Neither the cloud key nor the environment fallback below may reach a
        // local endpoint, in either key spelling (§8.1).
        profile_value("localApiKey", "localApiKey")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("local")
            .to_string()
    } else {
        let configured = profile_setting("apiKey", "modelApiKey");
        if !configured.is_empty() {
            configured
        } else {
            std::env::var("OPENAI_API_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_default()
        }
    };
    let api_base = if deployment == "local" {
        profile_setting("apiBase", "modelApiBase")
    } else {
        let configured = profile_setting("apiBase", "modelApiBase");
        if !configured.is_empty() {
            configured
        } else {
            std::env::var("OPENAI_BASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_default()
        }
    };
    if deployment == "local" && api_base.is_empty() {
        return Err(
            "本地模型必须配置明确的 API Base URL；安全策略不会回退读取 OPENAI_BASE_URL，防止误用云端环境变量或凭据".into(),
        );
    }
    if api_key.is_empty() {
        return Err(
            "模型 API 凭据未配置：请在配置中心填写 API Key（第三方 OpenAI 兼容服务同时填写 Base URL）".into(),
        );
    }
    Ok(ModelRuntimeEnv {
        llm,
        api_key,
        api_base,
        deployment,
        full_power,
        prompt_audit_mode,
    })
}

fn write_model_prompt_audit(
    work_dir: &Path,
    instruction: &str,
    environment: &ModelRuntimeEnv,
) -> Result<(), String> {
    if environment.prompt_audit_mode == "off" {
        return Ok(());
    }
    let mut hasher = Sha256::new();
    hasher.update(instruction.as_bytes());
    let audit = ModelPromptAudit {
        capture_mode: environment.prompt_audit_mode.clone(),
        source: "oviraptor_generated_instruction".into(),
        capture_level: "generated_instruction".into(),
        exact_model_request: false,
        model: environment.llm.clone(),
        deployment: environment.deployment.clone(),
        full_power: environment.full_power,
        recorded_at: chrono::Utc::now().to_rfc3339(),
        instruction_sha256: format!("{:x}", hasher.finalize()),
        instruction_chars: instruction.chars().count() as i64,
        instruction: (environment.prompt_audit_mode == "full")
            .then(|| retained_trace_text(instruction)),
        notice: "这是 Oviraptor 生成并交给原生调查的 instruction 快照；full 模式展示内容会脱敏，不是原文，也不是模型最终收到的完整请求。system、developer、tool schema 与最终对话组装需要请求层记录才能精确捕获。".into(),
    };
    fs::write(
        work_dir.join("model-prompt-audit.json"),
        serde_json::to_vec_pretty(&audit).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn test_model_connection(
    input: ModelProfileTestInput,
) -> Result<ModelProfileTestResult, String> {
    let deployment = match input.deployment.trim() {
        "local" => "local",
        "cloud" => "cloud",
        _ => return Err("模型部署类型必须是 cloud 或 local".into()),
    };
    let model = input.llm.trim().to_string();
    if model.is_empty() {
        return Err("请填写模型名（例如 openai/模型名）".into());
    }
    let base = input.api_base.trim().trim_end_matches('/').to_string();
    let base = if !base.is_empty() {
        base
    } else if deployment == "cloud" {
        std::env::var("OPENAI_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_default()
            .trim()
            .trim_end_matches('/')
            .to_string()
    } else {
        String::new()
    };
    let base = if base.is_empty() && deployment == "cloud" {
        "https://api.openai.com/v1".to_string()
    } else {
        base
    };
    if base.is_empty()
        || !(base.starts_with("http://") || base.starts_with("https://"))
        || base.chars().any(char::is_control)
    {
        return Err("请填写有效的 Base URL（例如 https://api.example.com/v1）".into());
    }
    let api_key = if deployment == "local" {
        let configured = input.api_key.trim();
        if configured.is_empty() {
            "local".to_string()
        } else {
            configured.to_string()
        }
    } else if input.api_key.trim().is_empty() {
        std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_default()
    } else {
        input.api_key.trim().to_string()
    };
    if api_key.is_empty() {
        return Err("云端模型测试必须填写 API Key，或配置 OPENAI_API_KEY".into());
    }
    let chat_model = openai_chat_completion_model(&model);
    if chat_model.is_empty() {
        return Err("模型名不能为空".into());
    }
    let endpoint = format!("{base}/chat/completions");
    let temp_id = Uuid::new_v4();
    let temp_dir = std::env::temp_dir();
    let header_path = temp_dir.join(format!("oviraptor-llm-test-{temp_id}.headers"));
    let body_path = temp_dir.join(format!("oviraptor-llm-test-{temp_id}.json"));
    let response_path = temp_dir.join(format!("oviraptor-llm-test-{temp_id}.response"));
    write_private_temp_file(
        &header_path,
        format!("Authorization: Bearer {api_key}\nContent-Type: application/json\n").as_bytes(),
    )
    .map_err(|error| format!("无法创建模型测试 Header：{error}"))?;
    // §8.2: a chat answer is not enough. The native agent works only through tool
    // calls, so the connectivity test must declare one and see whether the endpoint
    // refuses the schema outright.
    let request_body = serde_json::to_vec(&serde_json::json!({
        "model": chat_model,
        "messages": [{"role": "user", "content": "Use the ok_probe tool with value OK."}],
        "temperature": 0,
        "max_tokens": 64,
        "stream": false,
        "tools": [{
            "type": "function",
            "function": {
                "name": "ok_probe",
                "description": "Report that this endpoint accepts tool calls.",
                "parameters": {
                    "type": "object",
                    "properties": {"value": {"type": "string"}},
                    "required": ["value"],
                    "additionalProperties": false
                }
            }
        }],
        "tool_choice": "auto"
    }))
    .map_err(|error| error.to_string())?;
    if let Err(error) = write_private_temp_file(&body_path, &request_body) {
        let _ = fs::remove_file(&header_path);
        return Err(format!("无法创建模型测试请求：{error}"));
    }
    let output_result = Command::new("curl")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--connect-timeout")
        .arg("5")
        .arg("--max-time")
        .arg("30")
        .arg("--request")
        .arg("POST")
        .arg("--output")
        .arg(&response_path)
        .arg("--write-out")
        .arg("%{http_code}")
        .arg("-H")
        .arg(format!("@{}", header_path.to_string_lossy()))
        .arg("--data-binary")
        .arg(format!("@{}", body_path.to_string_lossy()))
        .arg(endpoint)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let response_body = fs::read_to_string(&response_path).unwrap_or_default();
    let _ = fs::remove_file(&header_path);
    let _ = fs::remove_file(&body_path);
    let _ = fs::remove_file(&response_path);
    let output = output_result.map_err(|error| format!("无法启动 curl：{error}"))?;
    let code_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let code = code_text.parse::<u16>().unwrap_or(0);
    if crate::agent_runtime::model::gateway::looks_like_unsupported_tools(code, &response_body) {
        // Reported as a refusal rather than a generic failure: `auto` and native
        // both need this answered before any target is touched (§7.2).
        return Err(format!(
            "模型端点不支持工具调用（HTTP {code}），无法用于原生 Agent；请改用支持 tools 的端点或显式选择兼容后端"
        ));
    }
    if (200..300).contains(&code) {
        let tool_note = if response_body.contains("tool_calls") {
            "，已收到工具调用"
        } else {
            "（端点接受工具声明，本轮未返回工具调用）"
        };
        Ok(ModelProfileTestResult {
            ok: true,
            status: code.to_string(),
            message: format!("模型 Chat Completions 与工具调用测试通过（HTTP {code}）{tool_note}"),
            model,
            deployment: deployment.into(),
        })
    } else {
        let body_error = llm_test_error_message(&response_body);
        let stderr_error = String::from_utf8_lossy(&output.stderr)
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(220)
            .collect::<String>();
        let error_text = if !body_error.is_empty() {
            body_error
        } else if !stderr_error.is_empty() {
            stderr_error
        } else {
            "服务未返回成功响应".into()
        };
        Err(if code > 0 {
            format!("模型 Chat Completions 测试失败（HTTP {code}）：{error_text}")
        } else if error_text.is_empty() {
            "模型连通性测试失败，请检查地址、网络和服务状态".into()
        } else {
            format!("模型连通性测试失败：{error_text}")
        })
    }
}

fn fofa_transport_error(error: &reqwest::Error, proxy_url: &str) -> String {
    if error.is_timeout() {
        return if proxy_url.is_empty() {
            "FOFA 连接超时，请检查当前网络后重试".into()
        } else {
            format!("FOFA 连接超时；当前配置的代理 {proxy_url} 没有正常转发请求")
        };
    }
    if !proxy_url.is_empty() {
        return format!("无法通过代理 {proxy_url} 连接 FOFA；请确认代理程序已启动且端口正确");
    }
    "无法连接 FOFA，请检查 DNS、网络或防火墙后重试".into()
}

pub(crate) fn request_fofa_account(key: &str, proxy_url: &str) -> Result<JsonValue, String> {
    if !key
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("FOFA Key 格式无效".into());
    }
    let mut builder = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));
    if !proxy_url.is_empty() {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|_| "代理地址格式无效，应类似 http://127.0.0.1:7890".to_string())?;
        builder = builder.proxy(proxy);
    }
    let client = builder
        .build()
        .map_err(|_| "无法初始化 FOFA 网络客户端".to_string())?;
    let mut response = client
        .get(format!("https://fofa.info/api/v1/info/my?key={key}"))
        .send()
        .map_err(|error| fofa_transport_error(&error, proxy_url))?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(|_| "读取 FOFA 响应失败".to_string())?;
    let payload: JsonValue = serde_json::from_slice(&body)
        .map_err(|_| format!("FOFA 返回了无法解析的响应（HTTP {}）", status.as_u16()))?;
    if !status.is_success()
        || payload
            .get("error")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
    {
        let message = payload
            .get("errmsg")
            .or_else(|| payload.get("message"))
            .and_then(JsonValue::as_str)
            .unwrap_or("Key 无效或 FOFA 拒绝了请求");
        return Err(format!("FOFA 鉴权失败：{message}"));
    }
    Ok(payload)
}

#[tauri::command]
pub async fn test_fofa_api(input: FofaApiTestInput) -> Result<FofaApiTestResult, String> {
    let key = input.key.trim().to_string();
    let proxy_url = input.proxy_url.trim().to_string();
    if key.is_empty() {
        return Err("请先填写 FOFA key".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let payload = request_fofa_account(&key, &proxy_url)?;
        let account = payload
            .get("email")
            .or_else(|| payload.get("username"))
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .to_string();
        let plan = payload
            .get("vip_level")
            .or_else(|| payload.get("vipLevel"))
            .or_else(|| payload.get("level"))
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_default();
        Ok(FofaApiTestResult {
            ok: true,
            status: "available".into(),
            message: if proxy_url.is_empty() {
                "FOFA API 可用，鉴权与直连网络均正常".into()
            } else {
                format!("FOFA API 可用，代理 {proxy_url} 工作正常")
            },
            account,
            plan,
        })
    })
    .await
    .map_err(|error| format!("FOFA 测试任务异常：{error}"))?
}

fn openai_chat_completion_model(llm: &str) -> String {
    let value = llm.trim();
    value
        .strip_prefix("openai/")
        .unwrap_or(value)
        .trim()
        .to_string()
}

fn write_private_temp_file(path: &Path, content: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| error.to_string())?;
        file.write_all(content).map_err(|error| error.to_string())?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, content).map_err(|error| error.to_string())
    }
}

fn llm_test_error_message(response_body: &str) -> String {
    let trimmed = response_body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let parsed = serde_json::from_str::<JsonValue>(trimmed).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .or_else(|| value.get("message"))
                .and_then(JsonValue::as_str)
        })
        .unwrap_or(trimmed);
    message
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(260)
        .collect()
}
