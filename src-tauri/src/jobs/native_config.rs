use serde_json::Value;
use std::path::PathBuf;

// Configuration compatibility only: this does not execute the old worker or
// persist another plaintext runtime config file.
pub(super) fn fofa_key(settings: &Value) -> Result<String, String> {
    let explicit = settings
        .get("fofaKey")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if explicit == "__CONFIGURED__" {
        return Err("FOFA Key 是快照标记；请读取当前配置的真实凭据".into());
    }
    if !explicit.is_empty() {
        return Ok(explicit.into());
    }
    let config = settings
        .get("configPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if config.is_empty() {
        return Err("请在配置中心填写 FOFA Key".into());
    }
    let path = if let Some(relative) = config.strip_prefix("~/") {
        PathBuf::from(
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                .ok_or("无法定位配置目录")?,
        )
        .join(relative)
    } else {
        PathBuf::from(config)
    };
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("无法读取已配置的 FOFA 文件：{error}"))?;
    parse_fofa_key(&source).ok_or_else(|| "FOFA 配置缺少 [fofa] key；请在配置中心更新 Key".into())
}

fn parse_fofa_key(source: &str) -> Option<String> {
    let mut section = "";
    let mut key = None;
    for line in source.trim_start_matches('\u{feff}').lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim();
            continue;
        }
        if section != "fofa" {
            continue;
        }
        if let Some((name, value)) = line.split_once(['=', ':']) {
            if name.trim().eq_ignore_ascii_case("key") && !value.trim().is_empty() {
                key = Some(value.trim().to_string());
            }
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_fofa_config_is_read_without_executing_a_worker() {
        assert_eq!(
            parse_fofa_key("\u{feff}[other]\nkey=wrong\n[fofa]\n; note\nKEY = fixture-key==\n"),
            Some("fixture-key==".into())
        );
        assert_eq!(parse_fofa_key("[other]\nkey=wrong"), None);
        assert_eq!(
            fofa_key(&serde_json::json!({"fofaKey":" explicit ","configPath":"/does/not/exist"}))
                .unwrap(),
            "explicit"
        );
        assert!(fofa_key(&serde_json::json!({})).is_err());
    }
}
