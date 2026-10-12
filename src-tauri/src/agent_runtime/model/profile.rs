//! Gateway profile (§5).
//!
//! The runtime never reads provider environment variables of its own: the
//! commands layer resolves the single existing model profile and hands these
//! plain values over, so cloud and local material can never be mixed.
use serde_json::Value as JsonValue;

/// Already-resolved profile values, produced by the existing profile resolver.
#[derive(Clone, Debug)]
pub struct ResolvedModelProfile {
    pub model: String,
    pub api_base: String,
    pub api_key: String,
    pub deployment: String,
}

/// Resource limits that apply to a local inference server.
#[derive(Clone, Debug, Default)]
pub struct LocalResourcePolicy {
    pub max_output_tokens: Option<u64>,
    pub max_context_tokens: u64,
}

#[derive(Clone, Debug)]
pub struct GatewayProfile {
    pub model: String,
    pub endpoint: String,
    api_key: String,
    pub local: bool,
    pub proxy: Option<String>,
    pub max_output_tokens: Option<u64>,
    pub max_context_tokens: u64,
}

impl GatewayProfile {
    pub fn resolve(
        resolved: &ResolvedModelProfile,
        policy: &LocalResourcePolicy,
        proxy: Option<&str>,
    ) -> Result<Self, String> {
        let model = resolved.model.trim();
        if model.is_empty() {
            return Err("当前模型档案缺少模型名，原生 Agent 无法启动".into());
        }
        let local = resolved.deployment == "local";
        let base = resolved.api_base.trim().trim_end_matches('/');
        if local && base.is_empty() {
            // A local profile without a base URL would silently fall back to a
            // public endpoint and send the local key there.
            return Err("本地模型档案必须提供 Base URL，避免误用云端凭据".into());
        }
        let base = if base.is_empty() {
            "https://api.openai.com/v1".to_string()
        } else {
            base.to_string()
        };
        Ok(Self {
            model: model.to_string(),
            endpoint: format!("{base}/chat/completions"),
            // An empty key is legal for a local server: the request then simply
            // omits Authorization instead of inventing a credential failure.
            api_key: resolved.api_key.trim().to_string(),
            local,
            proxy: proxy
                .map(str::to_string)
                .filter(|value| !value.trim().is_empty()),
            max_output_tokens: if local {
                policy.max_output_tokens
            } else {
                None
            },
            max_context_tokens: if local { policy.max_context_tokens } else { 0 },
        })
    }

    pub fn has_key(&self) -> bool {
        !self.api_key.is_empty()
    }

    /// Small, frozen structured-output calls need the answer to fit inside
    /// their original completion reservation. DeepSeek V4 defaults to thinking
    /// and can spend that entire reservation without returning any content.
    /// Larger/unbounded calls retain the configured provider default.
    pub fn bounded_wire_options(&self) -> JsonValue {
        if matches!(self.model.as_str(), "deepseek-v4-flash" | "deepseek-v4-pro" | "deepseek-flash")
            && self.max_output_tokens.is_some_and(|cap| cap > 0 && cap <= 2_048)
        {
            serde_json::json!({"thinking":{"type":"disabled"}})
        } else {
            serde_json::json!({})
        }
    }

    pub fn bearer(&self) -> Option<&str> {
        if self.has_key() {
            Some(self.api_key.as_str())
        } else {
            None
        }
    }

    /// Byte budget for one request payload. Local servers keep their window
    /// guard; cloud endpoints use a generous transport cap.
    pub fn request_budget_bytes(&self) -> usize {
        if self.max_context_tokens == 0 {
            return 6_000_000;
        }
        let reserve = self.max_output_tokens.unwrap_or(2_048);
        let usable = self.max_context_tokens.saturating_sub(reserve).saturating_sub(512).max(4_096);
        // Keep the transport ceiling even for corrupt or extreme local policy.
        // This byte approximation is not a provider token or financial bound.
        usable.saturating_mul(super::usage::CHARS_PER_TOKEN as u64).min(6_000_000) as usize
    }

    /// Deliberately key-free: safe for events and the UI.
    // Phase 2/3 consumes this; the rules around it are covered by tests now.
    #[allow(dead_code)]
    pub fn summary(&self) -> JsonValue {
        serde_json::json!({
            "model": self.model,
            "deployment": if self.local { "local" } else { "cloud" },
            "endpoint": self.endpoint,
            "credential": if self.has_key() { "configured" } else { "none" },
            "proxy": self.proxy.is_some(),
            "maxOutputTokens": self.max_output_tokens,
            "maxContextTokens": self.max_context_tokens,
        })
    }
}
