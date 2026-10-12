//! Current in-memory settings normalization. Retired top-level configuration is ignored,
//! never translated into active model credentials, execution policy or paths.
use serde_json::{json, Value};

pub fn normalize_settings(raw: &Value) -> Value {
    let mut normalized = raw.clone();
    let Some(object) = normalized.as_object_mut() else {
        return normalized;
    };
    // Restrict cleanup to the configuration namespace. Nested user documents
    // are not backend settings and must not be recursively rewritten.
    object.retain(|key, _| {
        !key.to_ascii_lowercase().starts_with("strix")
            && !matches!(
                key.as_str(),
                "pythonExecutable"
                    | "redisCliExecutable"
                    | "modelLocalFullPower"
                    | "modelPromptAuditMode"
                    | "legacyArtifactDirectories"
                    | "historicalImportDirectories"
                    | "agentBackendPolicy"
            )
    });

    // An explicit profile list, including null/empty, is authoritative. Never
    // resurrect a cleared selection from the flat model cache.
    if object.contains_key("modelProfiles") {
        return normalized;
    }
    if !object.contains_key("modelName") && !object.contains_key("modelApiBase") {
        return normalized;
    }

    // Current Native JSON may contain the supported flat model configuration.
    // Only current fields supply this profile; retired aliases cannot fill gaps.
    let profile = json!({
        "id":"default-model", "name":"默认模型",
        "llm":object.get("modelName").cloned().unwrap_or(json!("")),
        "apiBase":object.get("modelApiBase").cloned().unwrap_or(json!("")),
        "apiKey":object.get("modelApiKey").cloned().unwrap_or(json!("")),
        "localApiKey":object.get("localApiKey").cloned().unwrap_or(json!("")),
        "deployment":object.get("modelDeployment").cloned().unwrap_or(json!("cloud")),
    });
    for (key, field) in [
        ("activeModelProfileId", "id"),
        ("modelName", "llm"),
        ("modelDeployment", "deployment"),
        ("modelApiBase", "apiBase"),
        ("modelApiKey", "apiKey"),
        ("localApiKey", "localApiKey"),
    ] {
        object
            .entry(key.to_string())
            .or_insert_with(|| profile[field].clone());
    }
    object.insert("modelProfiles".into(), json!([profile]));
    normalized
}

/// Updating current configuration is not authorization to delete stored inert
/// fields. Preserve fields excluded from the execution view verbatim; new input
/// cannot overwrite or resurrect them as current settings.
pub fn settings_for_update(stored: &Value, incoming: &Value) -> Result<Value, String> {
    let original = stored
        .as_object()
        .ok_or("已保存的配置不是 JSON 对象，不能隐式替换")?;
    let view = normalize_settings(stored);
    let mut updated = normalize_settings(incoming);
    let object = updated
        .as_object_mut()
        .ok_or("配置 settings 必须是 JSON 对象")?;
    for (key, value) in original {
        if view.get(key).is_none() {
            object.insert(key.clone(), value.clone());
        }
    }
    Ok(updated)
}

#[cfg(test)]
#[path = "db_settings_tests.rs"]
mod tests;
