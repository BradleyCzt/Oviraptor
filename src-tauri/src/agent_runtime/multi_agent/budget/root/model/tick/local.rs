//! Deterministic local tools over original captured facts; no SQL/IO dispatcher.
use super::decision::DecisionSummary;
use crate::agent_runtime::{
    model::{gateway::ModelResponse, ToolSchema},
    secrets,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LocalStep {
    calls: Vec<LocalResult>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalResult {
    id: String,
    name: String,
    arguments: Value,
    result: Value,
}
// These opaque values bind only deterministic local calculation to its captured
// response and basis. They are not a paid receipt, signature, or execution grant.
pub(crate) struct OriginalLocalCalls {
    response: ModelResponse,
    basis: Value,
    binding: std::sync::Arc<()>,
}
pub(crate) struct OriginalLocalResult {
    binding: std::sync::Arc<()>,
    index: usize,
    call: LocalResult,
}
impl OriginalLocalResult {
    pub(crate) fn observation(&self) -> &Value {
        &self.call.result
    }
}
pub(crate) struct OriginalLocalProof {
    received_fact: Value,
    basis: Value,
    step: LocalStep,
}
impl OriginalLocalProof {
    pub(crate) fn verify_original(
        &self,
        response: &ModelResponse,
        basis: &Value,
    ) -> Result<LocalStep, String> {
        if self.received_fact != super::super::RootModelCall::received_fact(response)
            || self.basis != *basis
        {
            return Err("root_local_original_binding_changed".into());
        }
        Ok(self.step.clone())
    }
}
impl OriginalLocalCalls {
    pub(crate) fn capture(response: &ModelResponse, basis: &Value) -> Result<Option<Self>, String> {
        if response.tool_calls.is_empty() {
            return Ok(None);
        }
        let remaining = 4usize
            .checked_sub(
                basis["localCallsUsed"]
                    .as_u64()
                    .ok_or("root_local_call_budget_invalid")? as usize,
            )
            .ok_or("root_local_call_limit")?;
        if response.finish_reason != "tool_calls"
            || response.tool_calls.len() > remaining
            || response.tool_calls.len() > 4
        {
            return Err("root_local_call_limit_or_response_invalid".into());
        }
        Ok(Some(Self {
            response: response.clone(),
            basis: basis.clone(),
            binding: std::sync::Arc::new(()),
        }))
    }
    pub(crate) fn execute_call(&self, index: usize) -> Result<OriginalLocalResult, String> {
        let call = self
            .response
            .tool_calls
            .get(index)
            .ok_or("root_local_tool_index_invalid")?;
        // Preserve the original per-call error priority: identity, then
        // arguments and result size, before examining any later call.
        if call.id.is_empty()
            || call.id.len() > 64
            || !call
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            || secrets::redact_text_with(&call.id, None) != call.id
            || self.response.tool_calls[..index]
                .iter()
                .any(|previous| previous.id == call.id)
            || self.basis["localHistory"]
                .as_array()
                .is_some_and(|history| {
                    history.iter().any(|step| {
                        step["calls"].as_array().is_some_and(|calls| {
                            calls.iter().any(|previous| previous["id"] == call.id)
                        })
                    })
                })
        {
            return Err("root_local_tool_id_invalid".into());
        }
        let result = execute(&call.name, &call.arguments, &self.basis)?;
        Ok(OriginalLocalResult {
            binding: self.binding.clone(),
            index,
            call: LocalResult {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
                result,
            },
        })
    }
    pub(crate) fn finalize_original(
        self,
        results: Vec<OriginalLocalResult>,
    ) -> Result<OriginalLocalProof, String> {
        let received_fact = super::super::RootModelCall::received_fact(&self.response);
        let basis = self.basis.clone();
        let step = self.finalize(results)?;
        Ok(OriginalLocalProof {
            received_fact,
            basis,
            step,
        })
    }
    pub(crate) fn finalize(self, results: Vec<OriginalLocalResult>) -> Result<LocalStep, String> {
        if results.len() != self.response.tool_calls.len() {
            return Err("root_local_result_changed".into());
        }
        let mut calls = Vec::new();
        for (index, result) in results.into_iter().enumerate() {
            let original = &self.response.tool_calls[index];
            if !std::sync::Arc::ptr_eq(&result.binding, &self.binding)
                || result.index != index
                || result.call.id != original.id
                || result.call.name != original.name
                || result.call.arguments != original.arguments
                || result.call.result != execute(&original.name, &original.arguments, &self.basis)?
            {
                return Err("root_local_result_changed".into());
            }
            calls.push(result.call);
        }
        Ok(LocalStep { calls })
    }
}
pub(crate) fn schemas() -> Vec<ToolSchema> {
    let empty = json!({"type":"object","properties":{},"additionalProperties":false});
    vec![
        ToolSchema::new("snapshot.read", "Read the captured Root snapshot. Does not refresh or execute a target.",empty.clone()),
        ToolSchema::new("evidence.read", "Read the captured, redacted evidence only. No file paths, URLs or arbitrary queries.",empty.clone()),
        ToolSchema::new("capability_budget.read", "Read the original hard ceilings and coordination capability. These ceilings are not live remaining balances or grants; Rust checks the current ledger on every admission.",empty),
        ToolSchema::new("plan.propose", "Propose the offered step or defer. This is advisory and never dispatches an assignment.",json!({"type":"object","properties":{"step":{"type":"string","maxLength":64}},"required":["step"],"additionalProperties":false})),
    ]
}
impl LocalStep {
    pub(crate) fn parse(response: &ModelResponse, basis: &Value) -> Result<Option<Self>, String> {
        let Some(original) = OriginalLocalCalls::capture(response, basis)? else {
            return Ok(None);
        };
        let results = (0..original.response.tool_calls.len())
            .map(|index| original.execute_call(index))
            .collect::<Result<Vec<_>, _>>()?;
        let proof = original.finalize_original(results)?;
        proof.verify_original(response, basis).map(Some)
    }
    pub(crate) fn verify(&self, basis: &Value) -> Result<(), String> {
        let response = ModelResponse {
            tool_calls: self
                .calls
                .iter()
                .map(|c| crate::agent_runtime::model::gateway::ToolCall {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    arguments: c.arguments.clone(),
                })
                .collect(),
            finish_reason: "tool_calls".into(),
            ..Default::default()
        };
        if Self::parse(&response, basis)?.as_ref() != Some(self) {
            return Err("root_local_result_changed".into());
        }
        Ok(())
    }
    pub(crate) fn summary(&self) -> Result<DecisionSummary, String> {
        DecisionSummary::from_json(&json!({"schemaVersion":1,
            "observed":["Requested bounded local facts or an advisory plan; no target action was executed."],
            "missing":[],"suggestions":[],"costNotes":[],"risks":[]}))
    }
    pub(crate) fn tool_names(&self) -> Vec<&str> {
        self.calls.iter().map(|call| call.name.as_str()).collect()
    }
    pub(crate) fn count(&self) -> usize {
        self.calls.len()
    }
    pub(crate) fn as_json(&self) -> Value {
        json!(self)
    }
    pub(crate) fn from_json(value: &Value, basis: &Value) -> Result<Self, String> {
        let result: Self =
            serde_json::from_value(value.clone()).map_err(|_| "root_local_result_invalid")?;
        result.verify(basis)?;
        if result.as_json() != *value {
            return Err("root_local_result_invalid".into());
        }
        Ok(result)
    }
    pub(crate) fn messages(&self) -> Vec<Value> {
        let mut result = vec![
            json!({"role":"assistant","tool_calls":self.calls.iter().map(|c|json!({
            "id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments.to_string()}})).collect::<Vec<_>>()}),
        ];
        result.extend(
            self.calls
                .iter()
                .map(|c| json!({"role":"tool","tool_call_id":c.id,"content":c.result.to_string()})),
        );
        result
    }
}
fn execute(name: &str, args: &Value, basis: &Value) -> Result<Value, String> {
    let object = args
        .as_object()
        .ok_or("root_local_tool_arguments_invalid")?;
    let result = match name {
        "snapshot.read" if object.is_empty() => {
            let mut snapshot = basis.clone();
            let object = snapshot
                .as_object_mut()
                .ok_or("root_local_snapshot_invalid")?;
            object.remove("localHistory");
            object.remove("localParents"); // Internal physical proofs are not model context.
            snapshot
        }
        "evidence.read" if object.is_empty() => {
            json!({"frozenEvidence":basis["frozenEvidence"],"changedFact":basis["changedFact"]})
        }
        "capability_budget.read" if object.is_empty() => {
            let mut value = json!({"capabilityClass":basis["capabilityClass"],
            "originalHardLimits":basis["originalHardLimits"],"deliberationContract":basis["localDeliberation"],
            "remainingBalance":"not represented by hard ceilings; current ledger admission is enforced by Rust", "targetGrant":false});
            if let Some(snapshot) = basis.get("budgetSnapshot") {
                value["budgetSnapshot"] = snapshot.clone();
            }
            value
        }
        "plan.propose" if object.len() == 1 && object.contains_key("step") => {
            let step = args["step"]
                .as_str()
                .ok_or("root_local_tool_arguments_invalid")?;
            if step != "defer" && Some(step) != basis["permittedSuggestion"].as_str() {
                return Err("root_local_proposal_not_offered".into());
            }
            json!({"step":step,"advisoryOnly":true,"dispatched":false})
        }
        _ => return Err("root_local_tool_denied".into()),
    };
    let safe = secrets::redact_json(&result);
    if safe.to_string().len() > 96 * 1024 {
        return Err("root_local_result_too_large".into());
    }
    Ok(safe)
}

#[cfg(test)]
mod original_local_error_priority_tests {
    use super::*;
    use crate::agent_runtime::model::gateway::ToolCall;
    fn response(first: ToolCall, second: ToolCall) -> ModelResponse {
        ModelResponse {
            tool_calls: vec![first, second],
            finish_reason: "tool_calls".into(),
            ..Default::default()
        }
    }
    #[test]
    fn original_local_oversized_first_result_precedes_later_invalid_identity() {
        let response = response(
            ToolCall {
                id: "first".into(),
                name: "snapshot.read".into(),
                arguments: json!({}),
            },
            ToolCall {
                id: "invalid id".into(),
                name: "evidence.read".into(),
                arguments: json!({}),
            },
        );
        let basis = json!({"localCallsUsed":0,"frozenEvidence":"x".repeat(96*1024)});
        assert_eq!(
            LocalStep::parse(&response, &basis).unwrap_err(),
            "root_local_result_too_large"
        );
    }
    #[test]
    fn original_local_first_argument_error_precedes_later_invalid_identity() {
        let response = response(
            ToolCall {
                id: "first".into(),
                name: "evidence.read".into(),
                arguments: json!([]),
            },
            ToolCall {
                id: "invalid id".into(),
                name: "snapshot.read".into(),
                arguments: json!({}),
            },
        );
        assert_eq!(
            LocalStep::parse(&response, &json!({"localCallsUsed":0})).unwrap_err(),
            "root_local_tool_arguments_invalid"
        );
    }
}
