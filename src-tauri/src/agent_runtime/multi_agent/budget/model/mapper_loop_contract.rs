//! Frozen v15 wire bounds. This input is not a dispatch or financial grant.
use crate::agent_runtime::{multi_agent::lease::CoordinatorLease, store, web_mode};
use rusqlite::Connection;
use serde_json::{json, Value};

pub(crate) const RULE: &str = "original_frozen_mapper_two_round_one_lookup_v1";
pub(crate) const SYSTEM: &str = "Analyze frozen frontend evidence only. Use evidence.lookup once if needed; when calling it, emit no text. Never access targets or claim verified vulnerabilities. Finish with JSON: summary, priorityContracts, risks. Reference only the frozen inventory.";
pub(crate) const DESCRIPTION: &str =
    "Read one original frozen frontend artifact; no network or authority.";

pub(crate) struct FrozenMapperLoopInput {
    fact: Value,
    tokens: i64,
}

impl FrozenMapperLoopInput {
    pub(crate) fn from_frozen_wire_on(
        db: &Connection,
        actor: &CoordinatorLease,
        fact: &Value,
    ) -> Result<Self, String> {
        require_protocol_on(db, actor)?;
        let tokens = verify_wire(actor, fact)?;
        Ok(Self {
            fact: fact.clone(),
            tokens,
        })
    }

    pub(crate) fn require_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
    ) -> Result<(), String> {
        require_protocol_on(db, actor)?;
        if verify_wire(actor, &self.fact)? != self.tokens {
            return Err("mapper_loop_frozen_wire_changed".into());
        }
        Ok(())
    }

    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
    pub(crate) fn hash(&self) -> String {
        store::stable_hash(&self.fact.to_string())
    }
    pub(crate) fn reserved_tokens(&self) -> i64 {
        self.tokens
    }
}

pub(crate) fn require_protocol_on(db: &Connection, actor: &CoordinatorLease) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("mapper_loop_snapshot_required".into());
    }
    super::super::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    let mode = web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("mapper_loop_original_protocol_missing")?;
    let proof = crate::commands::original_root_capture_mode(db, &actor.root_run_id)?;
    web_mode::root::verify_birth(db, &actor.root_run_id, &proof)?;
    let protocol = mode
        .bootstrap_dispatch()
        .ok_or("mapper_loop_original_protocol_missing")?;
    if mode.mode() != web_mode::WebMode::Multi
        || mode.fact() != proof.fact()
        || protocol["schemaVersion"] != 15
        || protocol["mapperLoopRule"] != RULE
        || protocol["reservedModelRequests"] != 2
        || protocol["reservedModelTokens"] != 8_000
    {
        return Err("mapper_loop_original_protocol_missing".into());
    }
    Ok(())
}

fn keys(value: &Value, expected: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|v| v.len() == expected.len() && expected.iter().all(|k| v.contains_key(*k)))
}
fn hash(value: &Value) -> bool {
    value.as_str().is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}

fn verify_wire(actor: &CoordinatorLease, fact: &Value) -> Result<i64, String> {
    let invalid = "mapper_loop_frozen_wire_invalid";
    if !keys(
        fact,
        &[
            "schemaVersion",
            "purpose",
            "scope",
            "inventory",
            "inventoryHash",
            "request",
            "configurationBinding",
            "secondRoundMaximum",
            "historyLimits",
            "maxRounds",
            "maxLookupCalls",
            "firstRoundEstimatedTokens",
            "secondRoundEstimatedTokens",
            "totalEstimatedTokens",
        ],
    ) || fact["schemaVersion"] != 1
        || fact["purpose"] != "mapper_loop_frozen_wire_bounds"
        || fact["maxRounds"] != 2
        || fact["maxLookupCalls"] != 1
        || fact["scope"]
            != json!({"rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,
            "targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token})
    {
        return Err(invalid.into());
    }
    let inventory = &fact["inventory"];
    if !keys(
        inventory,
        &[
            "schemaVersion",
            "rootRunId",
            "scanId",
            "attemptNumber",
            "targetKey",
            "coordinatorEpoch",
            "coordinatorFence",
            "evidenceRevision",
            "limits",
            "entries",
        ],
    ) || inventory["schemaVersion"] != 1
        || inventory["rootRunId"] != actor.root_run_id
        || inventory["scanId"] != actor.scan_id
        || inventory["attemptNumber"] != actor.attempt_number
        || inventory["targetKey"] != actor.target_key
        || inventory["coordinatorEpoch"] != actor.lease_epoch
        || inventory["coordinatorFence"] != actor.fencing_token
        || inventory["evidenceRevision"].as_i64().is_none_or(|n| n < 1)
        || !keys(
            &inventory["limits"],
            &["sourceMaxBytes", "observationMaxBytes"],
        )
        || inventory["limits"]["sourceMaxBytes"]
            .as_u64()
            .is_none_or(|n| n == 0 || n > 256 * 1024)
        || inventory["limits"]["observationMaxBytes"] != 512
        || fact["inventoryHash"] != store::stable_hash(&inventory.to_string())
    {
        return Err(invalid.into());
    }
    let entries = inventory["entries"].as_array().ok_or(invalid)?;
    if entries.len() != 1
        || !keys(
            &entries[0],
            &["evidenceId", "artifact", "sha256", "sizeBytes", "mediaType"],
        )
        || entries[0]["evidenceId"] != "frontend-evidence"
        || entries[0]["artifact"] != "frontend-evidence.json"
        || entries[0]["mediaType"] != "application/json"
        || !hash(&entries[0]["sha256"])
        || entries[0]["sizeBytes"].as_u64().is_none_or(|n| {
            n == 0 || n > inventory["limits"]["sourceMaxBytes"].as_u64().unwrap_or(0)
        })
    {
        return Err(invalid.into());
    }
    if fact["historyLimits"]
        != json!({"assistantTextBytes":0,"callIdBytes":64,"rawArgumentBytes":64,"observationBytes":512})
    {
        return Err(invalid.into());
    }
    let request = &fact["request"];
    if !keys(
        request,
        &[
            "messages",
            "tools",
            "temperature",
            "model",
            "endpointHash",
            "proxyHash",
            "local",
            "maxOutputTokens",
            "maxContextTokens",
        ],
    ) || request["temperature"].as_f64() != Some(0.0)
        || request["maxOutputTokens"] != 512
        || request["model"].as_str().is_none_or(str::is_empty)
        || !hash(&request["endpointHash"])
        || !hash(&request["proxyHash"])
        || !request["local"].is_boolean()
        || request["maxContextTokens"].as_u64().is_none()
    {
        return Err(invalid.into());
    }
    // Original model identity is frozen alongside the actual request. This
    // reader compares saved facts only; a journal's claimed current profile
    // cannot replace this original binding or confer live authorization.
    let binding = &fact["configurationBinding"];
    if !keys(
        binding,
        &[
            "model",
            "endpointHash",
            "proxyHash",
            "credentialHash",
            "deployment",
            "local",
            "configuredOutputCap",
            "maxContextTokens",
        ],
    ) || binding["model"] != request["model"]
        || binding["endpointHash"] != request["endpointHash"]
        || binding["proxyHash"] != request["proxyHash"]
        || !hash(&binding["credentialHash"])
        || binding["deployment"].as_str().is_none_or(str::is_empty)
        || binding["local"] != request["local"]
        || binding["local"].as_bool() != Some(binding["deployment"] == "local")
        || binding["maxContextTokens"] != request["maxContextTokens"]
        || (!binding["configuredOutputCap"].is_null()
            && binding["configuredOutputCap"]
                .as_u64()
                .is_none_or(|n| n < 512))
    {
        return Err("mapper_loop_original_configuration_binding_invalid".into());
    }
    let messages = request["messages"].as_array().ok_or(invalid)?;
    if messages.len() != 2
        || messages
            .iter()
            .any(|m| !keys(m, &["role", "content"]) || m["content"].as_str().is_none())
        || messages[0]["role"] != "system"
        || messages[1]["role"] != "user"
    {
        return Err(invalid.into());
    }
    let expected_messages = json!([
        {"role":"system","content":SYSTEM},
        {"role":"user","content":json!({"target":actor.target_key,"inventory":inventory,"requiredOutput":{"summary":"string","priorityContracts":[],"risks":[]}}).to_string()}
    ]);
    if request["messages"] != expected_messages {
        return Err(invalid.into());
    }
    // The request factory owns the exact SDK function specification; this
    // reader refuses any extra tool or any non-readonly argument surface.
    let tools = request["tools"].as_array().ok_or(invalid)?;
    if tools.len() != 1 || !keys(&tools[0], &["type", "function"]) || tools[0]["type"] != "function"
    {
        return Err(invalid.into());
    }
    let function = &tools[0]["function"];
    if !keys(function, &["name", "description", "parameters"])
        || function["name"] != "evidence.lookup"
        || function["description"] != DESCRIPTION
        || function["parameters"]
            != json!({"type":"object","properties":{"evidenceId":{"type":"string","enum":["frontend-evidence"]}},"required":["evidenceId"],"additionalProperties":false})
    {
        return Err(invalid.into());
    }
    let mut maximum = messages.clone();
    let id = "\0".repeat(64);
    maximum.push(json!({"role":"assistant","content":"","tool_calls":[{"id":id,"type":"function","function":{"name":"evidence.lookup","arguments":"\0".repeat(64)}}]}));
    // Observation content is serde_json::Value::to_string(), never arbitrary
    // text: its valid JSON encoding has no raw controls, so outer encoding
    // expands each byte by at most two, attained by a backslash.
    maximum.push(json!({"role":"tool","tool_call_id":id,"content":"\\".repeat(512)}));
    if fact["secondRoundMaximum"] != json!({"messages":maximum}) {
        return Err(invalid.into());
    }
    let estimate = |messages: &Value| -> Result<i64, String> {
        let bytes = serde_json::to_vec(&json!({"messages":messages,"tools":tools}))
            .map_err(|e| e.to_string())?
            .len();
        i64::try_from(bytes)
            .ok()
            .and_then(|n| n.checked_add(512))
            .and_then(|n| n.checked_add(512))
            .ok_or_else(|| "mapper_loop_input_bound_overflow".into())
    };
    let first = estimate(&request["messages"])?;
    let second = estimate(&fact["secondRoundMaximum"]["messages"])?;
    let tokens = first
        .checked_add(second)
        .ok_or("mapper_loop_input_bound_overflow")?;
    if fact["firstRoundEstimatedTokens"] != first
        || fact["secondRoundEstimatedTokens"] != second
        || fact["totalEstimatedTokens"] != tokens
    {
        return Err("mapper_loop_frozen_wire_changed".into());
    }
    let context = request["maxContextTokens"].as_u64().ok_or(invalid)?;
    let first_u64 = u64::try_from(first).map_err(|_| "mapper_loop_input_bound_overflow")?;
    let second_u64 = u64::try_from(second).map_err(|_| "mapper_loop_input_bound_overflow")?;
    if tokens > 8_000 || context > 0 && (first_u64 > context || second_u64 > context) {
        return Err("mapper_loop_input_bound_exceeds_original_budget".into());
    }
    Ok(tokens)
}
