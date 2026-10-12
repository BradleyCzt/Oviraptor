//! Actual Mapper wire and complete bounded-history admission; grants nothing.
use super::mapper_evidence_lookup::{MapperEvidenceInventory, MapperLookupObservation};
use crate::agent_runtime::{
    model::{gateway::ModelResponse, GatewayProfile, ModelRequest, ToolSchema},
    multi_agent::lease::CoordinatorLease,
    store,
};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::Path;

const SYSTEM: &str = "Analyze frozen frontend evidence only. Use evidence.lookup once if needed; when calling it, emit no text. Never access targets or claim verified vulnerabilities. Finish with JSON: summary, priorityContracts, risks. Reference only the frozen inventory.";
const DESCRIPTION: &str = "Read one original frozen frontend artifact; no network or authority.";
const OUTPUT: u64 = 512;
const ID_BYTES: usize = 64;
const ARG_BYTES: usize = 64;
const OBSERVATION_BYTES: usize = 512;

pub(crate) struct VerifiedMapperLoopRequestBounds {
    inventory: MapperEvidenceInventory,
    profile: GatewayProfile,
    configuration_binding: Value,
    first: ModelRequest,
    fact: Value,
    first_estimate: i64,
    second_estimate: i64,
}
fn configuration_binding(profile: &GatewayProfile, environment: &super::ModelRuntimeEnv) -> Value {
    json!({"model":profile.model,"endpointHash":store::stable_hash(&profile.endpoint),
        "proxyHash":store::stable_hash(profile.proxy.as_deref().unwrap_or_default()),
        "credentialHash":store::stable_hash(&environment.api_key),"deployment":environment.deployment,
        "local":profile.local,"configuredOutputCap":profile.max_output_tokens,"maxContextTokens":profile.max_context_tokens})
}
fn tool() -> ToolSchema {
    ToolSchema::new(
        "evidence.lookup",
        DESCRIPTION,
        json!({"type":"object","properties":{"evidenceId":{"type":"string","enum":["frontend-evidence"]}},"required":["evidenceId"],"additionalProperties":false}),
    )
}
fn estimate(messages: &[Value], tools: &[Value], output: u64) -> Result<i64, String> {
    let bytes = serde_json::to_vec(&json!({"messages":messages,"tools":tools}))
        .map_err(|_| "mapper_loop_wire_invalid")?
        .len();
    i64::try_from(bytes)
        .ok()
        .and_then(|n| n.checked_add(512))
        .and_then(|n| n.checked_add(i64::try_from(output).ok()?))
        .ok_or_else(|| "mapper_loop_wire_bound_overflow".into())
}
fn history(text: &str, id: &str, arguments: &str, observation: &str) -> [Value; 2] {
    [
        json!({"role":"assistant","content":text,"tool_calls":[{"id":id,"type":"function","function":{"name":"evidence.lookup","arguments":arguments}}]}),
        json!({"role":"tool","tool_call_id":id,"content":observation}),
    ]
}
impl VerifiedMapperLoopRequestBounds {
    pub(crate) fn capture_on(
        db: &Connection,
        actor: &CoordinatorLease,
        target_dir: &Path,
        inventory: MapperEvidenceInventory,
        environment: &super::ModelRuntimeEnv,
        proxy: Option<&str>,
    ) -> Result<Self, String> {
        // Re-read original bytes through the typed inventory, not caller input.
        let observed =
            inventory.lookup_on(db, actor, &json!({"evidenceId":"frontend-evidence"}))?;
        if observed.value().to_string().len() > OBSERVATION_BYTES
            || inventory.fact()["limits"]["observationMaxBytes"] != OBSERVATION_BYTES
            || super::resolve_agent_evidence_location(db, &actor.root_run_id)? != target_dir
        {
            return Err("mapper_loop_inventory_bound_invalid".into());
        }
        let mut profile = super::agent_model_profile(environment, proxy)?;
        let configuration_binding = configuration_binding(&profile, environment);
        if profile.max_output_tokens.is_some_and(|cap| cap < OUTPUT) {
            return Err("mapper_loop_output_cap_invalid".into());
        }
        // New frozen contract cap, selected before birth; never modifies an
        // existing worker, model setting or original assignment reservation.
        profile.max_output_tokens = Some(profile.max_output_tokens.unwrap_or(OUTPUT).min(OUTPUT));
        let output = profile
            .max_output_tokens
            .ok_or("mapper_loop_output_cap_missing")?;
        let messages = vec![
            json!({"role":"system","content":SYSTEM}),
            json!({"role":"user","content":json!({"target":actor.target_key,"inventory":inventory.fact(),"requiredOutput":{"summary":"string","priorityContracts":[],"risks":[]}}).to_string()}),
        ];
        let tools = vec![tool().as_function_spec()];
        let first_estimate = estimate(&messages, &tools, output)?;
        // Raw IDs/arguments expand by at most six JSON bytes per input byte.
        // Observation is already serde JSON: it contains no raw controls;
        // string re-encoding expands each original byte by at most two.
        // Backslash padding attains that second-encoding upper bound.
        let maximum = history(
            "",
            &"\0".repeat(ID_BYTES),
            &"\0".repeat(ARG_BYTES),
            &"\\".repeat(OBSERVATION_BYTES),
        );
        let mut second_messages = messages.clone();
        second_messages.extend(maximum);
        let second_estimate = estimate(&second_messages, &tools, output)?;
        let total = first_estimate
            .checked_add(second_estimate)
            .ok_or("mapper_loop_wire_bound_overflow")?;
        if total > 8000
            || (profile.max_context_tokens > 0
                && u64::try_from(second_estimate).map_err(|_| "mapper_loop_wire_bound_overflow")?
                    > profile.max_context_tokens)
        {
            return Err("mapper_loop_complete_input_bound_exceeds_grant".into());
        }
        let request = json!({"messages":messages,"tools":tools,"temperature":0.0,"model":profile.model,
            "endpointHash":store::stable_hash(&profile.endpoint),"proxyHash":store::stable_hash(profile.proxy.as_deref().unwrap_or_default()),
            "local":profile.local,"maxOutputTokens":output,"maxContextTokens":profile.max_context_tokens,
            "providerOptions":profile.bounded_wire_options()});
        let fact = json!({"schemaVersion":1,"purpose":"mapper_loop_frozen_wire_bounds",
            "scope":{"rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token},
            "inventory":inventory.fact(),"inventoryHash":inventory.hash(),"request":request,"configurationBinding":configuration_binding,
            "secondRoundMaximum":{"messages":second_messages},
            "historyLimits":{"assistantTextBytes":0,"callIdBytes":ID_BYTES,"rawArgumentBytes":ARG_BYTES,"observationBytes":OBSERVATION_BYTES},
            "maxRounds":2,"maxLookupCalls":1,"firstRoundEstimatedTokens":first_estimate,"secondRoundEstimatedTokens":second_estimate,"totalEstimatedTokens":total});
        Ok(Self {
            inventory,
            profile,
            configuration_binding,
            first: ModelRequest::new(messages, vec![tool()]),
            fact,
            first_estimate,
            second_estimate,
        })
    }
    pub(crate) fn require_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
        environment: &super::ModelRuntimeEnv,
        proxy: Option<&str>,
    ) -> Result<(), String> {
        self.require_original_on(db, actor)?;
        let current = super::agent_model_profile(environment, proxy)?;
        if configuration_binding(&current, environment) != self.configuration_binding {
            return Err("mapper_loop_original_model_configuration_changed".into());
        }
        self.require_original_on(db, actor)
    }
    // Original source verification only; never a live execution authority.
    pub(crate) fn require_original_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
    ) -> Result<(), String> {
        self.inventory.require_on(db, actor)?;
        let observed =
            self.inventory
                .lookup_on(db, actor, &json!({"evidenceId":"frontend-evidence"}))?;
        if observed.value().to_string().len() > OBSERVATION_BYTES {
            return Err("mapper_loop_inventory_bound_invalid".into());
        }
        let tools = vec![tool().as_function_spec()];
        if self.fact["configurationBinding"] != self.configuration_binding
            || self.first.tools.len() != 1
            || self.first.tools[0].as_function_spec() != tools[0]
            || self.fact["request"]["messages"] != json!(self.first.messages)
            || self.fact["request"]["tools"] != json!(tools)
            || self.fact["inventory"] != *self.inventory.fact()
            || self.fact["inventoryHash"] != self.inventory.hash()
            || estimate(&self.first.messages, &tools, OUTPUT)? != self.first_estimate
        {
            return Err("mapper_loop_original_frozen_wire_changed".into());
        }
        let maximum = history(
            "",
            &"\0".repeat(ID_BYTES),
            &"\0".repeat(ARG_BYTES),
            &"\\".repeat(OBSERVATION_BYTES),
        );
        let mut second = self.first.messages.clone();
        second.extend(maximum);
        if self.fact["secondRoundMaximum"]["messages"] != json!(second)
            || estimate(&second, &tools, OUTPUT)? != self.second_estimate
        {
            return Err("mapper_loop_original_frozen_wire_changed".into());
        }
        self.inventory.require_on(db, actor)
    }
    pub(crate) fn original_configuration_binding(&self) -> &Value {
        &self.configuration_binding
    }
    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
    pub(crate) fn profile(&self) -> &GatewayProfile {
        &self.profile
    }
    pub(crate) fn first_request(&self) -> &ModelRequest {
        &self.first
    }
    pub(crate) fn first_estimate(&self) -> i64 {
        self.first_estimate
    }
    pub(crate) fn second_estimate(&self) -> i64 {
        self.second_estimate
    }
    #[cfg(test)]
    pub(crate) fn inventory(&self) -> &MapperEvidenceInventory {
        &self.inventory
    }
    // Wire construction only. The financial caller must separately require
    // the opaque paid response and original lookup audit of this same round.
    pub(crate) fn second_request(
        &self,
        response: &ModelResponse,
        observation: &MapperLookupObservation,
    ) -> Result<ModelRequest, String> {
        let [call] = response.tool_calls.as_slice() else {
            return Err("mapper_loop_original_call_group_invalid".into());
        };
        let arguments = call.arguments.to_string();
        let encoded = observation.value().to_string();
        if !response.text.is_empty()
            || call.id.is_empty()
            || call.id.len() > ID_BYTES
            || call.name != "evidence.lookup"
            || arguments.len() > ARG_BYTES
            || encoded.len() > OBSERVATION_BYTES
        {
            return Err("mapper_loop_actual_history_exceeds_frozen_bound".into());
        }
        let mut messages = self.first.messages.clone();
        messages.extend(history(&response.text, &call.id, &arguments, &encoded));
        let tools = vec![tool().as_function_spec()];
        if estimate(
            &messages,
            &tools,
            self.profile
                .max_output_tokens
                .ok_or("mapper_loop_output_cap_missing")?,
        )? > self.second_estimate
        {
            return Err("mapper_loop_actual_wire_exceeds_frozen_bound".into());
        }
        Ok(ModelRequest::new(messages, vec![tool()]))
    }
}
