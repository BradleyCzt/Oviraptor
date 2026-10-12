//! Fresh deterministic semantics bound to one original paid Root response.
//! These opaque tokens authorize no execution and cannot be restored from JSON.
use super::{decision::DecisionSummary, local, Tick};
use crate::agent_runtime::model::gateway::ModelResponse;
use serde_json::{json, Value};

pub(crate) enum OriginalSemanticFailure {
    Stopped,
    LocalValidation(String),
}

enum Outcome {
    Local(local::OriginalLocalProof),
    Decision,
    Failure(OriginalSemanticFailure),
}

pub(crate) struct OriginalSemanticToken {
    binding: Value,
    outcome: Outcome,
}

impl Tick {
    fn semantic_binding(&self, response: &ModelResponse) -> Value {
        json!({"root":self.call.owner.root,"owner":self.call.owner.id,
            "call":self.call.id,"round":self.call.round,"requestHash":self.call.request,
            "basisHash":self.basis,"request":self.request_fact,"dispatch":self.dispatch_proof,
            "invoice":super::super::RootModelCall::received_fact(response)})
    }

    fn require_original_local_semantics(&self, response: &ModelResponse) -> Result<(), String> {
        let code = match self.request_fact["basis"]["phase"].as_str() {
            Some("authorization-output") => Some("root_authorization_output_local_tool_denied"),
            Some("external-surface-output") => {
                Some("root_external_surface_output_local_tool_denied")
            }
            Some("business-logic-output") => Some("root_business_logic_output_local_tool_denied"),
            Some("business-logic-input") => Some("root_business_logic_dispatch_local_tool_denied"),
            _ => None,
        };
        if let Some(code) = code.filter(|_| !response.tool_calls.is_empty()) {
            return Err(code.into());
        }
        if self.call.owner.contract["root"]["localDeliberation"].is_null() {
            return Err("root_local_history_not_authorized".into());
        }
        if response.tool_calls.is_empty() {
            return Err("root_original_local_semantic_missing_calls".into());
        }
        Ok(())
    }

    pub(crate) fn original_semantic_with_local(
        &self,
        response: &ModelResponse,
        proof: local::OriginalLocalProof,
    ) -> Result<OriginalSemanticToken, String> {
        self.require_original_local_semantics(response)?;
        proof.verify_original(response, &self.request_fact["basis"])?;
        Ok(OriginalSemanticToken {
            binding: self.semantic_binding(response),
            outcome: Outcome::Local(proof),
        })
    }

    pub(crate) fn original_semantic_failure(
        &self,
        response: &ModelResponse,
        failure: OriginalSemanticFailure,
    ) -> OriginalSemanticToken {
        OriginalSemanticToken {
            binding: self.semantic_binding(response),
            outcome: Outcome::Failure(failure),
        }
    }

    pub(crate) fn original_semantic_without_local(
        &self,
        response: &ModelResponse,
    ) -> Result<OriginalSemanticToken, String> {
        if !response.tool_calls.is_empty() {
            return Err("root_original_semantic_requires_local_proof".into());
        }
        DecisionSummary::parse_response(response)?;
        Ok(OriginalSemanticToken {
            binding: self.semantic_binding(response),
            outcome: Outcome::Decision,
        })
    }
}

impl OriginalSemanticToken {
    pub(super) fn into_semantic(
        self,
        tick: &Tick,
        response: &ModelResponse,
    ) -> Result<(DecisionSummary, Option<local::LocalStep>), String> {
        if self.binding != tick.semantic_binding(response) {
            return Err("root_original_semantic_binding_changed".into());
        }
        match self.outcome {
            Outcome::Local(proof) => {
                tick.require_original_local_semantics(response)?;
                let step = proof.verify_original(response, &tick.request_fact["basis"])?;
                Ok((step.summary()?, Some(step)))
            }
            Outcome::Decision => {
                if !response.tool_calls.is_empty() {
                    return Err("root_original_semantic_requires_local_proof".into());
                }
                DecisionSummary::parse_response(response).map(|summary| (summary, None))
            }
            Outcome::Failure(OriginalSemanticFailure::Stopped) => {
                Err("root_original_semantic_stopped".into())
            }
            Outcome::Failure(OriginalSemanticFailure::LocalValidation(code)) => Err(code),
        }
    }
}
