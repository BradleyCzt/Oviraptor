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
