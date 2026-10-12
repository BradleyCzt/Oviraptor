use super::*;

#[test]
fn retired_settings_do_not_create_native_credentials_profiles_or_policy() {
    let value = normalize_settings(&json!({
        "strixLlm":"old-model", "strixApiBase":"https://old.invalid",
        "strixApiKey":"old-secret", "strixLlmProfiles":[{"id":"old","apiKey":"secret"}],
        "strixActiveLlmProfileId":"old", "strixLocalFullPower":true,
        "strixQuickTokenLimit":0, "strixFutureSetting":true, "STRIXExecutable":"old-bin",
        "modelLocalFullPower":true, "modelPromptAuditMode":"full",
        "strixRunsDirectory":"/old", "historicalImportDirectories":"/intermediate",
        "legacyArtifactDirectories":"/archive", "agentBackendPolicy":"auto"
    }));
    assert_eq!(value, json!({}));
    assert_eq!(normalize_settings(&value), value);
}

#[test]
fn current_settings_and_explicit_clears_survive_retirement() {
    for profiles in [
        json!([]),
        Value::Null,
        json!({}),
        json!([{
            "id":"native", "llm":"current", "apiKey":"current-secret"
        }]),
    ] {
        let current = json!({"modelProfiles":profiles, "activeModelProfileId":"native",
            "modelApiKey":"", "modelName":null, "agentLocalFullPower":false,
            "agentQuickTokenLimit":0, "agentPromptAuditMode":"off",
            "custom":{"strixExample":"user-owned nested data"}});
        let mut mixed = current.clone();
        mixed["strixApiKey"] = json!("old-secret");
        mixed["strixLlmProfiles"] = json!([{"id":"old","llm":"old-model"}]);
        mixed["strixLocalFullPower"] = json!(true);
        assert_eq!(normalize_settings(&mixed), current);
        assert_eq!(normalize_settings(&current), current);
    }
}

#[test]
fn current_flat_settings_build_only_a_current_profile() {
    for model in [json!("native-model"), json!(""), Value::Null] {
        let value = normalize_settings(&json!({"modelName":model,
            "modelDeployment":"local", "modelApiBase":"http://127.0.0.1:18080/v1",
            "localApiKey":"current-local-key", "strixApiKey":"old-cloud-key",
            "strixLlm":"old-model", "strixActiveLlmProfileId":"old"}));
        assert_eq!(value["modelProfiles"][0]["id"], "default-model");
        assert_eq!(value["modelProfiles"][0]["llm"], model);
        assert_eq!(value["modelProfiles"][0]["deployment"], "local");
        assert_eq!(
            value["modelProfiles"][0]["apiBase"],
            "http://127.0.0.1:18080/v1"
        );
        assert_eq!(
            value["modelProfiles"][0]["localApiKey"],
            "current-local-key"
        );
        assert_eq!(value["modelApiKey"], "");
        assert_eq!(value["activeModelProfileId"], "default-model");
        assert_eq!(normalize_settings(&value), value);
    }
}

#[test]
fn partial_current_settings_cannot_borrow_an_old_model() {
    let value = normalize_settings(&json!({"modelDeployment":"local",
        "localApiKey":"current-local-key", "strixLlm":"old-model",
        "strixApiKey":"old-key", "strixApiBase":"https://old.invalid"}));
    assert_eq!(
        value,
        json!({"modelDeployment":"local", "localApiKey":"current-local-key"})
    );
}

#[test]
fn non_object_settings_are_not_reinterpreted() {
    for value in [Value::Null, json!([]), json!("user data"), json!(false)] {
        assert_eq!(normalize_settings(&value), value);
    }
}

#[test]
fn retired_host_tools_are_inert_and_saved_history_is_preserved() {
    let stored = json!({"pythonExecutable":"private-old-python", "redisCliExecutable":"private-old-cli", "modelApiBase":"https://current.invalid", "custom":{"pythonExecutable":"user document"}});
    let view = super::normalize_settings(&stored);
    assert!(view.get("pythonExecutable").is_none());
    assert!(view.get("redisCliExecutable").is_none());
    assert_eq!(view["custom"]["pythonExecutable"], "user document");
    let updated = super::settings_for_update(&stored, &json!({"pythonExecutable":"new-inert", "redisCliExecutable":"new-inert", "modelApiBase":"https://new.invalid"})).unwrap();
    assert_eq!(updated["pythonExecutable"], stored["pythonExecutable"]);
    assert_eq!(updated["redisCliExecutable"], stored["redisCliExecutable"]);
    assert!(super::normalize_settings(&updated).get("pythonExecutable").is_none());
}
