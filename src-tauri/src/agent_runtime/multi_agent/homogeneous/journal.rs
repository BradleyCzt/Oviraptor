//! Root tick 账本用的摘要投影。
//!
//! 真实决策（snake_case `CoordinatorDecision`）只落在同质控制面的表里；
//! 现网 Root 模型账本仍要求一份 camelCase `DecisionSummary`。这里只做**投影**：
//! 不改 `DecisionSummary` 的结构、不复用它承载新语义，也不新增 reason_code。

use super::{decision, reason, types};
use crate::agent_runtime::model::gateway::ModelResponse;
use crate::agent_runtime::multi_agent::budget::root::model::tick::decision::DecisionSummary;
use serde_json::json;

/// 账本里代表「这一轮是同质主管决策」的固定 step 串，与 frame step 同名。
pub const LEDGER_STEP: &str = "assess:homogeneous_decision";

/// 严格解析工具保留受控拒绝码。生产付费轮通过 ledger_response 发布拒绝摘要，
/// 再把原始决策交给 Rust 护栏，避免缺少 publication 而阻断下一轮。
pub fn parse(model_text: &str) -> Result<types::CoordinatorDecision, String> {
    decision::parse_decision_envelope(model_text)
        .map_err(|error| format!("root_homogeneous_{}", error.reason_code))
}

/// 把已解析的动作投影成账本摘要（字符串全部为受控常量，不含模型原文）。
pub fn ledger_summary(action: &str) -> Result<DecisionSummary, String> {
    DecisionSummary::from_json(&json!({
        "schemaVersion": 1,
        "observed": [format!("action:{action}")],
        "missing": [],
        "suggestions": [LEDGER_STEP],
        "costNotes": ["实际成本以 Root 回执为准"],
        "risks": [],
    }))
}

/// A paid invalid reply still needs a bounded publication so the next original
/// round can proceed. This records a refusal, never an executable action; the
/// original reply reaches Rust's decision gate only after financial publication.
pub fn ledger_response(response: &ModelResponse) -> Result<DecisionSummary, String> {
    let refused = if !response.tool_calls.is_empty() {
        Some(reason::TOOL_NOT_AUTHORIZED)
    } else if response.finish_reason != "stop" || response.text.len() > 48 * 1024 {
        Some(reason::OUTPUT_SCHEMA_INVALID)
    } else {
        match decision::parse_decision_envelope(&response.text) {
            Ok(parsed) => return ledger_summary(&parsed.action),
            Err(error) => Some(error.reason_code),
        }
    };
    let code = refused.expect("invalid original response has a refusal code");
    if !reason::ALL.contains(&code) {
        return Err("root_tick_decision_invalid".into());
    }
    DecisionSummary::from_json(&json!({
        "schemaVersion": 1,
        "observed": [format!("refused:{code}")],
        "missing": [],
        "suggestions": [LEDGER_STEP],
        "costNotes": ["实际成本以 Root 回执为准"],
        "risks": ["主管输出被拒；Rust 护栏决定是否补意图"],
    }))
}
