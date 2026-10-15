// Seam between the flat `commands` module and the runtime model gateway.
//
// The gateway implementation lives in `agent_runtime::model`; this file only
// maps the shared model-profile resolution onto its plain input types, so
// there is exactly one model client in the codebase.
use crate::agent_runtime::model::{
    compact_messages, CancelToken, GatewayProfile, ModelError, ModelGateway, ModelRequest,
    ToolSchema, UsageDelta,
};

pub type AgentModelProfile = GatewayProfile;
pub type AgentModelClient = crate::agent_runtime::model::OpenAiCompatibleGateway;
pub type AgentModelError = ModelError;
pub type AgentToolSpec = ToolSchema;
pub type AgentTokenUsage = UsageDelta;

/// Single profile source: `model_runtime_env` already resolved the model name,
/// base URL, key and deployment for this app, so no second credential lookup
/// happens here.
fn agent_model_profile(
    environment: &ModelRuntimeEnv,
    proxy: Option<&str>,
) -> Result<AgentModelProfile, String> {
    crate::model_configuration::gateway_profile(&environment.as_connection(), proxy)
}

/// Pause and cancel are database facts, so the gateway stays storage-agnostic.
/// A pause must count as cancelled: otherwise an in-flight local generation
/// keeps running to completion after the user asked the scan to stop.
fn agent_scan_cancel_token(db_path: &Path, scan_id: &str, attempt: i64) -> CancelToken {
    let database = db_path.to_path_buf();
    let scan = scan_id.to_string();
    CancelToken::from_checker(move || !native_web_attempt_active(&database, &scan, attempt))
}

fn agent_model_request(messages: Vec<JsonValue>, specs: &[AgentToolSpec]) -> ModelRequest {
    ModelRequest::new(messages, specs.to_vec())
}

fn agent_compact_messages(messages: &[JsonValue], budget_bytes: usize) -> (Vec<JsonValue>, bool) {
    compact_messages(messages, budget_bytes)
}

fn agent_text_truncated(text: &str, limit: usize) -> String {
    crate::agent_runtime::model::gateway::truncate(text, limit)
}
