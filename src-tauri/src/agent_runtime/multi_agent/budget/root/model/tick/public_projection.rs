//! Pure paid-publication reader; original frozen identity is not live authority.
use super::{super::RootModelCall, Tick};
use crate::agent_runtime::{multi_agent::budget::root::RootOwner, store};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::time::Duration;

pub(crate) struct DecisionProjection {
    pub(crate) root: String,
    pub(crate) scan: String,
    pub(crate) attempt: i64,
    pub(crate) target: String,
    pub(crate) thread: String,
    pub(crate) entity: String,
    pub(crate) sequence: i64,
    pub(crate) created: String,
    pub(crate) record: Value,
}
impl Tick {
    pub(crate) fn original_publication_for_continuation(
        db: &Connection,
        root: &str,
        event: i64,
    ) -> Result<(Tick, super::SavedDecision), String> {
        if db.is_autocommit() {
            return Err("root_tick_projection_requires_snapshot".into());
        }
        Self::project_publication(db, root, event)?;
        let call: String = db
            .query_row(
                "SELECT call_id FROM agent_root_tick_receipts
            WHERE root_run_id=?1 AND phase='publication' AND round=1
            AND json_extract(fact_json,'$.eventSequence')=?2",
                params![root, event],
                |r| r.get(0),
            )
            .map_err(|_| "financial_continuation_publication_unverified")?;
        let tick = Self::original_request_for_projection(db, root, &call)?;
        if tick.request_fact["request"]["purpose"] != super::financial_purpose::PURPOSE {
            return Err("financial_continuation_publication_scope_changed".into());
        }
        let saved = tick
            .load_decision(db)?
            .ok_or("root_tick_decision_missing")?;
        if saved.unknown || saved.local_step.is_some() || tick.published(db, &saved)? != Some(event)
        {
            return Err("financial_continuation_publication_unverified".into());
        }
        Ok((tick, saved))
    }
    pub(crate) fn project_publication(
        db: &Connection,
        root: &str,
        model_event: i64,
    ) -> Result<DecisionProjection, String> {
        if db.is_autocommit() || model_event <= 0 {
            return Err("root_tick_projection_requires_snapshot".into());
        }
        let owner = RootOwner::load_original(db, root)?;
        if owner.contract["root"]["policy"] != "multi" {
            return Err("root_tick_requires_multi_coordinator".into());
        }
        let (id, round, request, basis, text): (String, i64, String, String, String) = db
            .query_row(
                "SELECT p.call_id,p.round,p.request_hash,p.basis_hash,q.fact_json
            FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q
              ON q.call_id=p.call_id AND q.phase='request' AND q.root_run_id=p.root_run_id
              AND q.round=p.round AND q.lease_attempt_id=p.lease_attempt_id
              AND q.request_hash=p.request_hash AND q.basis_hash=p.basis_hash
            WHERE p.root_run_id=?1 AND p.lease_attempt_id=?2 AND p.phase='publication'
              AND json_extract(p.fact_json,'$.eventSequence')=?3",
                params![root, owner.id, model_event],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .map_err(|_| "root_tick_original_publication_missing")?;
        let value: Value = serde_json::from_str(&text).map_err(|_| "root_tick_json_invalid")?;
        let fact = &value["request"];
        let scope = &owner.contract["root"];
        let frozen = &fact["basis"];
        let canonical = value.to_string();
        if canonical != text
            || value.as_object().is_none_or(|o| o.len() != 3)
            || value["version"] != 1
            || fact.as_object().is_none_or(|o| o.len() != 4)
            || !matches!(fact["version"].as_i64(), Some(1..=3))
            || fact["owner"] != owner.contract
            || store::stable_hash(&fact.to_string()) != request
            || Self::basis_hash(frozen) != basis
            || frozen["root"] != root
            || frozen["scan"] != scope["scan"]
            || frozen["attempt"] != scope["attempt"]
            || frozen["target"] != scope["target"]
            || frozen["planHash"] != scope["planHash"]
            || frozen["coordinator"]["epoch"] != owner.contract["coordinator"]["epoch"]
            || frozen["coordinator"]["fence"] != owner.contract["coordinator"]["fence"]
            || fact["request"]["root"] != root
            || fact["request"]["basisHash"] != basis
            || !matches!(
                fact["request"]["purpose"].as_str(),
                Some(
                    "coordinator_decision_v1"
                        | "coordinator_changed_fact_v1"
                        | "coordinator_financial_continuation_v1"
                )
            )
            || round <= 0
            || id
                != store::stable_hash(
                    &json!({"owner":owner.id,
                "root":root,"round":round,"request":request})
                    .to_string(),
                )
        {
            return Err("root_tick_original_input_conflict".into());
        }
        let tokens: i64 = db
            .query_row(
                "SELECT json_extract(receipt_json,'$.reservedTokens')
            FROM agent_root_model_journal WHERE call_id=?1 AND root_run_id=?2
            AND lease_attempt_id=?3 AND round=?4 AND request_hash=?5 AND phase='dispatch'",
                params![id, root, owner.id, round, request],
                |r| r.get(0),
            )
            .map_err(|_| "root_tick_original_dispatch_missing")?;
        if tokens <= 0 {
            return Err("root_tick_original_dispatch_conflict".into());
        }
        let tick = Self {
            call: RootModelCall {
                owner,
                id,
                round,
                request,
                tokens,
                remaining: Duration::ZERO,
            },
            basis,
            request_fact: fact.clone(),
            dispatch_proof: value["dispatchProof"].clone(),
        };
        tick.verify_request(db)?;
        let saved = tick
            .load_decision(db)?
            .ok_or("root_tick_decision_missing")?;
        if saved.unknown || tick.published(db, &saved)? != Some(model_event) {
            return Err("root_tick_original_publication_unverified".into());
        }
        let channel = tick.verify_timeline(db, &saved, model_event)?;
        let scope = &tick.call.owner.contract["root"];
        let mut record = json!({"schemaVersion":1,"advisoryOnly":true,"summary":saved.summary.as_json(),
            "usage":saved.invoice["usage"],"callId":tick.call.id,"round":tick.call.round,
            "modelEventSequence":model_event});
        if let Some(step) = &saved.local_step {
            record["localTools"] = json!(step.tool_names());
        }
        let target = scope["target"].as_str().ok_or("root_tick_scope_invalid")?;
        let human = super::human_projection::from_basis(tick.basis(), target)?;
        let thread = human
            .as_ref()
            .and_then(|c| c["threadKey"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("coordinator:{root}"));
        if let Some(context) = human {
            record["humanDirective"] = context;
        }
        Ok(DecisionProjection {
            root: root.into(),
            thread,
            scan: scope["scan"]
                .as_str()
                .ok_or("root_tick_scope_invalid")?
                .into(),
            attempt: scope["attempt"].as_i64().ok_or("root_tick_scope_invalid")?,
            target: scope["target"]
                .as_str()
                .ok_or("root_tick_scope_invalid")?
                .into(),
            entity: channel.entity,
            sequence: channel.sequence,
            created: channel.created,
            record,
        })
    }
}
