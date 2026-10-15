//! Shared model configuration and gateway-profile resolution.
//! Reads the caller's existing snapshot; no scan, tool grant or runtime mutation
//! is started by resolving a profile.
use crate::{
    agent_runtime::model::{GatewayProfile, LocalResourcePolicy, ResolvedModelProfile},
    db,
};
use serde_json::Value as JsonValue;
use std::process::Command;

pub(crate) fn default_settings(connection: &rusqlite::Connection) -> JsonValue {
    connection
        .query_row(
            "SELECT settings_json FROM config_profiles ORDER BY is_default DESC,id LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

/// Normalize stored aliases once, before either connection selection or a
/// caller's domain-specific settings are inspected. This carries no authority.
pub(crate) struct NormalizedModelSettings(JsonValue);

impl NormalizedModelSettings {
    pub(crate) fn new(settings: &JsonValue) -> Self {
        Self(db::normalize_settings(settings))
    }
    pub(crate) fn as_value(&self) -> &JsonValue {
        &self.0
    }
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct ModelConnection {
    pub(crate) llm: String,
    pub(crate) api_key: String,
    pub(crate) api_base: String,
    pub(crate) deployment: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalModelResourcePolicy {
    pub(crate) parameter_billions: Option<u32>,
    pub(crate) unified_memory_gb: u64,
    pub(crate) max_concurrent_requests: usize,
    pub(crate) max_context_tokens: u64,
    pub(crate) max_output_tokens: Option<u64>,
    pub(crate) memory_guard_tier: &'static str,
}

pub(crate) fn model_parameter_billions(model: &str) -> Option<u32> {
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

pub(crate) fn system_unified_memory_gb() -> u64 {
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

pub(crate) fn model_resource_policy_for_memory(
    environment: &ModelConnection,
    unified_memory_gb: u64,
) -> LocalModelResourcePolicy {
    if environment.deployment != "local" {
        return LocalModelResourcePolicy {
            parameter_billions: None,
            unified_memory_gb,
            max_concurrent_requests: 4,
            max_context_tokens: 0,
            max_output_tokens: None,
            memory_guard_tier: "balanced",
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
    let max_output_tokens = match parameter_billions {
        Some(size) if size >= 60 => 3_072,
        Some(size) if size >= 20 => 3_072,
        Some(size) if size >= 13 => 3_072,
        Some(_) => 2_048,
        None => 3_072,
    };
    let memory_guard_tier = if unified_memory_gb <= 18 && size <= 10 {
        // A 9B Q4 model's first prefill sits just above oMLX Balanced on a
        // 16 GB Mac. Aggressive still remains below the
        // physical Metal ceiling and keeps the guard enabled.
        "aggressive"
    } else {
        "balanced"
    };
    LocalModelResourcePolicy {
        parameter_billions,
        unified_memory_gb,
        // Keep local prefills serial under the existing memory policy.
        max_concurrent_requests: 1,
        max_context_tokens,
        max_output_tokens: Some(max_output_tokens),
        memory_guard_tier,
    }
}

pub(crate) fn model_resource_policy(environment: &ModelConnection) -> LocalModelResourcePolicy {
    model_resource_policy_for_memory(environment, system_unified_memory_gb())
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

pub(crate) fn model_connection(
    settings: &NormalizedModelSettings,
) -> Result<ModelConnection, String> {
    let settings = settings.as_value();
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
    Ok(ModelConnection {
        llm,
        api_key,
        api_base,
        deployment,
    })
}

pub(crate) fn openai_chat_completion_model(llm: &str) -> String {
    let value = llm.trim();
    value
        .strip_prefix("openai/")
        .unwrap_or(value)
        .trim()
        .to_string()
}

pub(crate) fn gateway_profile(
    environment: &ModelConnection,
    proxy: Option<&str>,
) -> Result<GatewayProfile, String> {
    let policy = model_resource_policy(environment);
    GatewayProfile::resolve(
        &ResolvedModelProfile {
            model: openai_chat_completion_model(&environment.llm),
            api_base: environment.api_base.clone(),
            api_key: environment.api_key.clone(),
            deployment: environment.deployment.clone(),
        },
        &LocalResourcePolicy {
            max_output_tokens: policy.max_output_tokens,
            max_context_tokens: policy.max_context_tokens,
        },
        proxy,
    )
}

/// Resolve the application-selected model inside the caller's connection.
/// Callers retain their own endpoint, consent and execution policies.
pub(crate) fn gateway_profile_on(
    connection: &rusqlite::Connection,
    proxy: Option<&str>,
) -> Result<GatewayProfile, String> {
    let settings = NormalizedModelSettings::new(&default_settings(connection));
    let environment = model_connection(&settings)?;
    gateway_profile(&environment, proxy)
}

#[cfg(test)]
#[path = "model_configuration_tests.rs"]
mod tests;
