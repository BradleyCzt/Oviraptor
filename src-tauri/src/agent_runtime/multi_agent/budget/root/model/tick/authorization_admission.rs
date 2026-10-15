//! Private one-shot source enum for original Authorization/ExternalSurface/BusinessLogic.
//! File path is retained for compatibility; no saved JSON or caller flag proof.
use super::{decision::DecisionSummary, Tick};
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{budget::root::RootOwner, lease::CoordinatorLease, scheduler::ScheduledChild},
    store,
};
use rusqlite::Connection;
use serde_json::{json, Value};
const AUTHORIZATION_SYSTEM: &str = "You are the Root Coordinator. Assess only the original frozen Authorization output. Return one strict DecisionSummary JSON with the offered assessment suggestion. No tools, target actions, commands, credentials, reasoning or assistant drafts. Suggestions are advisory and grant no scope or dispatch. Every list <=16; every string <=512 characters; schemaVersion=1.";
const SURFACE_SYSTEM: &str = "You are the Root Coordinator. Assess only the original frozen anonymous ExternalSurface output. Return one strict DecisionSummary JSON with the offered assessment suggestion. No tools, target actions, commands, credentials, reasoning or assistant drafts. Suggestions are advisory and grant no scope or dispatch. Every list <=16; every string <=512 characters; schemaVersion=1.";
const BUSINESS_LOGIC_SYSTEM: &str = "You are the Root Coordinator. Assess only the original frozen readonly BusinessLogic output. Return one strict DecisionSummary JSON with the offered assessment suggestion. No tools, target actions, commands, credentials, reasoning or assistant drafts. Suggestions are advisory and grant no scope or dispatch. Every list <=16; every string <=512 characters; schemaVersion=1.";
enum OneShotSource {
    BusinessLogic {
        receipt_id: String,
        birth_hash: String,
    },
    Authorization {
        receipt_id: String,
        birth_hash: String,
    },
    ExternalSurface {
        receipt_id: String,
        birth_hash: String,
    },
}
impl OneShotSource {
    fn identifiers(&self) -> (&str, &str) {
        match self {
            Self::Authorization {
                receipt_id,
                birth_hash,
            }
            | Self::ExternalSurface {
                receipt_id,
                birth_hash,
            }
            | Self::BusinessLogic {
                receipt_id,
                birth_hash,
            } => (receipt_id, birth_hash),
        }
    }
}
const INSTRUCTION: &str = "Use the permitted suggestion exactly only if useful. Empty suggestions means defer. This suggestion grants no new scope, capability, budget or target request; Rust validates the actual dispatch.";

pub(in crate::agent_runtime::multi_agent::budget::root::model) struct VerifiedOneShotAssessment {
    connection: usize,
    root: String,
    round: i64,
    request_hash: String,
    source: OneShotSource,
}
impl VerifiedOneShotAssessment {
    pub(in crate::agent_runtime::multi_agent::budget::root::model) fn require_on(
        &self,
        db: &Connection,
        root: &str,
        round: i64,
        hash: &str,
    ) -> Result<(), String> {
        if db.is_autocommit()
            || self.connection != db as *const Connection as usize
            || self.root != root
            || self.round != round
            || self.request_hash != hash
            || self.source.identifiers().0.is_empty()
            || self.source.identifiers().1.is_empty()
        {
            return Err("root_authorization_admission_binding_changed".into());
        }
        Ok(())
    }
}

pub(super) fn verify_on(
    db: &Connection,
    root: &str,
    round: i64,
    fact: &Value,
) -> Result<Option<VerifiedOneShotAssessment>, String> {
    let basis = &fact["basis"];
    let (phase, role, step, system) = match basis["phase"].as_str() {
        Some("authorization-output") => (
            "authorization-output",
            AgentRole::Authorization,
            "assess:authorization_output",
            AUTHORIZATION_SYSTEM,
        ),
        Some("external-surface-output") => (
            "external-surface-output",
            AgentRole::ExternalSurface,
            "assess:external_surface_output",
            SURFACE_SYSTEM,
        ),
        Some("business-logic-output") => (
            "business-logic-output",
            AgentRole::BusinessLogic,
            "assess:business_logic_output",
            BUSINESS_LOGIC_SYSTEM,
        ),
        _ => return Ok(None),
    };
    let invalid = || "root_authorization_admission_invalid".to_string();
    if db.is_autocommit() || round < 1 || fact["version"] != 3 {
        return Err("root_authorization_admission_snapshot_version_changed".into());
    }
    let owner = RootOwner::load_original(db, root)?;
    if fact["owner"] != owner.contract || owner.contract["root"]["policy"] != "multi" {
        return Err("root_authorization_admission_owner_changed".into());
    }
    let actor = db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[root],|r|Ok(CoordinatorLease{scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).map_err(|_|invalid())?;
    owner.require_original_coordinator(db, &actor)?;
    let semantic = &basis["changedFact"]["semantic"];
    let child = ScheduledChild {
        assignment_id: semantic["assignmentId"]
            .as_str()
            .ok_or_else(invalid)?
            .into(),
        run_id: semantic["childRunId"].as_str().ok_or_else(invalid)?.into(),
        role,
    };
    // Each role uses its own original private typed source verifier; a
    // foreign role seal cannot satisfy the other phase's one-shot proof.
    let (source_id, message_id, source_fact) = match role {
        AgentRole::Authorization => {
            let source = crate::scanning::authorization_output_receipt::read_original_on(
                db, &actor, &child,
            )?
            .ok_or("root_authorization_admission_source_missing")?;
            (
                source.receipt_id().to_string(),
                source.message_id().to_string(),
                source.fact().clone(),
            )
        }
        AgentRole::ExternalSurface => {
            let source = crate::scanning::external_surface_output_receipt::read_original_on(
                db, &actor, &child,
            )?
            .ok_or("root_external_surface_admission_source_missing")?;
            (
                source.receipt_id().to_string(),
                source.message_id().to_string(),
                source.fact().clone(),
            )
        }
        AgentRole::BusinessLogic => {
            // The private reader verifies the exact original v13 birth and all
            // three rules, frozen input, known SDK fees, zero target and closure.
            let source = crate::scanning::business_logic_output_receipt::read_original_on(
                db, &actor, &child,
            )?
            .ok_or("root_business_logic_admission_source_missing")?;
            (
                source.receipt_id().to_string(),
                source.message_id().to_string(),
                source.fact().clone(),
            )
        }
        _ => return Err(invalid()),
    };
    let expected = json!({"kind":phase,"physicalFacts":[],"semantic":{
        "role":role.as_str(),"rootRunId":actor.root_run_id,"scanId":actor.scan_id,
        "attemptNumber":actor.attempt_number,"targetKey":actor.target_key,
        "assignmentId":child.assignment_id,"childRunId":child.run_id,
        "outputMessageId":message_id,"outputReceiptId":source_id,
        "originalClosedOutput":source_fact,"advisoryOnly":true,"executionGrant":false,
        "dispatchGranted":false,"targetRequestsGranted":0,"additionalScopeGranted":false}});
    let request = &fact["request"];
    if basis["changedFact"] != expected {
        return Err("root_authorization_admission_signed_frame_changed".into());
    }
    if basis["localRound"] != 0
        || basis["localCallsUsed"] != 0
        || basis["localParents"] != json!([])
        || basis["localHistory"] != json!([])
        || basis["localDeliberation"] != owner.contract["root"]["localDeliberation"]
    {
        return Err("root_authorization_admission_local_history_changed".into());
    }
    if basis["changedFact"] != expected
        || basis["root"] != root
        || basis["scan"] != actor.scan_id
        || basis["attempt"] != actor.attempt_number
        || basis["target"] != actor.target_key
        || basis["coordinator"] != json!({"epoch":actor.lease_epoch,"fence":actor.fencing_token})
        || basis["permittedSuggestion"] != step
        || basis["localRound"] != 0
        || basis["localCallsUsed"] != 0
        || basis["localParents"] != json!([])
        || basis["localHistory"] != json!([])
        || basis["localDeliberation"] != owner.contract["root"]["localDeliberation"]
        || request["purpose"] != "coordinator_changed_fact_v1"
        || request["root"] != root
        || request["basisHash"] != Tick::basis_hash(basis)
        || request["tools"] != json!([])
        || basis.get("financialContinuation").is_some()
        || request.get("financialContinuation").is_some()
    {
        return Err("root_authorization_admission_request_scope_changed".into());
    }
    let messages = request["messages"].as_array().ok_or_else(invalid)?;
    if messages.len() != 2
        || messages[0]["role"] != "system"
        || messages[0]["content"] != system
        || messages[1]["role"] != "user"
        || messages
            .iter()
            .any(|m| m.as_object().is_none_or(|o| o.len() != 2))
    {
        return Err("root_authorization_admission_wire_messages_changed".into());
    }
    let input: Value = serde_json::from_str(messages[1]["content"].as_str().ok_or_else(invalid)?)
        .map_err(|_| invalid())?;
    // coordinator_tick's first local round adds proof bookkeeping to the
    // captured basis after request_for_frame has frozen the user message.
    // tick_once copies only budgetSnapshot into that original wire message.
    // These four internal fields must be exact zero/empty above, and must not
    // be invented in the provider input when reconstructing the real caller.
    let mut wire_basis = basis.clone();
    let object = wire_basis.as_object_mut().ok_or_else(invalid)?;
    for key in [
        "localRound",
        "localCallsUsed",
        "localParents",
        "localHistory",
    ] {
        object.remove(key);
    }
    if input["basis"] != wire_basis
        || input.as_object().is_none_or(|o| o.len() != 4)
        || input["instruction"] != INSTRUCTION
        || input["permittedSuggestion"] != step
        || input["requiredOutput"] != DecisionSummary::schema()
    {
        return Err("root_authorization_admission_wire_input_changed".into());
    }
    let birth: String = db
        .query_row(
            "SELECT binding_json FROM agent_root_mode_birth_seals WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|_| invalid())?;
    Ok(Some(VerifiedOneShotAssessment {
        connection: db as *const Connection as usize,
        root: root.into(),
        round,
        request_hash: store::stable_hash(&fact.to_string()),
        source: match role {
            AgentRole::Authorization => OneShotSource::Authorization {
                receipt_id: source_id,
                birth_hash: store::stable_hash(&birth),
            },
            AgentRole::ExternalSurface => OneShotSource::ExternalSurface {
                receipt_id: source_id,
                birth_hash: store::stable_hash(&birth),
            },
            AgentRole::BusinessLogic => OneShotSource::BusinessLogic {
                receipt_id: source_id,
                birth_hash: store::stable_hash(&birth),
            },
            _ => return Err(invalid()),
        },
    }))
}
