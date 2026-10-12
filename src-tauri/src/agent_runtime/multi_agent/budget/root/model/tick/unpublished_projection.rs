//! Pure original unpublished assessment state; no publication or recovery authority.
use super::{super::RootModelCall, Tick};
use crate::agent_runtime::{
    model::{gateway::ModelResponse, UsageDelta},
    multi_agent::budget::root::RootOwner,
    store,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::time::Duration;
impl Tick {
    pub(super) fn original_request_for_projection(
        db: &Connection,
        root: &str,
        selected: &str,
    ) -> Result<Self, String> {
        if db.is_autocommit() {
            return Err("root_tick_projection_requires_snapshot".into());
        }
        let owner = RootOwner::load_original(db, root)?;
        if owner.contract["root"]["policy"] != "multi" {
            return Err("root_tick_requires_multi_coordinator".into());
        }
        let (id, round, request, basis, text): (String, i64, String, String, String) = db.query_row(
            "SELECT call_id,round,request_hash,basis_hash,fact_json FROM agent_root_tick_receipts
             WHERE root_run_id=?1 AND call_id=?2 AND phase='request' AND lease_attempt_id=?3",
            params![root, selected, owner.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))
            .map_err(|_| "root_tick_original_request_missing")?;
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
        Ok(tick)
    }
    pub(crate) fn project_unpublished_human(
        db: &Connection,
        root: &str,
        call: &str,
        scan: &str,
        attempt: i64,
    ) -> Result<Option<Value>, String> {
        let tick = Self::original_request_for_projection(db, root, call)?;
        let scope = &tick.call.owner.contract["root"];
        if scope["scan"] != scan || scope["attempt"] != attempt {
            return Err("root_tick_unpublished_scope_conflict".into());
        }
        let target = scope["target"].as_str().ok_or("root_tick_scope_invalid")?;
        let Some(human) = super::human_projection::from_basis(tick.basis(), target)? else {
            return Ok(None);
        };
        let mut q=db.prepare("SELECT phase,receipt_json FROM agent_root_model_journal WHERE call_id=?1 AND phase<>'dispatch' ORDER BY phase").map_err(|e|e.to_string())?;
        let rows = q
            .query_map([call], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if rows.len() > 1 {
            return Err("root_tick_original_terminal_conflict".into());
        }
        let mut reported = Value::Null;
        let state = if let Some((phase, text)) = rows.first() {
            let invoice: Value =
                serde_json::from_str(text).map_err(|_| "root_tick_json_invalid")?;
            let canonical = invoice.to_string();
            if canonical != *text {
                return Err("root_tick_json_noncanonical".into());
            }
            tick.call.verify(db, phase, &invoice)?;
            tick.verify_original_terminal_proof(db)?;
            match phase.as_str() {
                "received" => {
                    let response = invoice_response(&invoice)?;
                    let unknown = tick.cost_unknown(db, &response)?;
                    tick.cost_rows(db, Some(&response))?;
                    let saved = tick.load_decision(db)?;
                    if let Some(saved) = &saved {
                        tick.verify_paid(db, saved)?;
                    }
                    // Cold Native history without an original proof is not a
                    // verified bill and cannot be upgraded from current rows.
                    let proven = matches!(tick.request_fact["version"].as_i64(), Some(2 | 3))
                        || saved.is_some();
                    if proven
                        && response.usage_reported
                        && invoice["usage"].as_object().is_some_and(|o| {
                            o.values().all(|v| {
                                v.as_i64()
                                    .is_some_and(|n| (0..=9_007_199_254_740_991).contains(&n))
                            })
                        })
                    {
                        reported = invoice["usage"].clone();
                    }
                    if unknown || !proven {
                        "cost_unconfirmed"
                    } else {
                        "assessment_unpublished"
                    }
                }
                "uncertain" | "unsent" => {
                    if invoice.as_object().is_none_or(|o| o.len() != 1)
                        || !invoice["code"].is_string()
                    {
                        return Err("root_tick_original_terminal_conflict".into());
                    }
                    tick.verify_stopped_cost_projection(db, Some(phase))?;
                    if phase == "uncertain" {
                        "dispatch_unconfirmed"
                    } else {
                        "not_sent"
                    }
                }
                _ => return Err("root_tick_original_terminal_conflict".into()),
            }
        } else {
            tick.verify_stopped_cost_projection(db, None)?;
            "awaiting_receipt"
        };
        let published:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_tick_receipts WHERE call_id=?1 AND phase='publication')",[call],|r|r.get(0)).map_err(|e|e.to_string())?;
        if published {
            return Err("root_tick_unpublished_projection_conflict".into());
        }
        Ok(Some(
            json!({"rootRunId":root,"callId":call,"round":tick.call.round,"targetKey":target,
            "humanDirective":human,"state":state,"reportedUsage":reported,"createdAt":tick.request_row(db)?["created"]}),
        ))
    }
    pub(super) fn verify_stopped_cost_projection(
        &self,
        db: &Connection,
        phase: Option<&str>,
    ) -> Result<(), String> {
        use std::collections::BTreeMap;
        let prefix = format!("root:{}:root-model:{}:", self.call.owner.id, self.call.id);
        let mut expected = BTreeMap::new();
        for (i, dimension) in crate::agent_runtime::multi_agent::budget::DIMENSIONS[..4]
            .iter()
            .enumerate()
        {
            let n = if i == 3 { 1 } else { self.call.tokens };
            expected.insert(
                format!("{prefix}{dimension}:reserve"),
                (dimension.to_string(), "reserve".to_string(), n),
            );
            if let Some(phase) = phase {
                expected.insert(
                    format!("{prefix}{dimension}:terminal"),
                    (
                        dimension.to_string(),
                        if phase == "uncertain" {
                            "forfeit".into()
                        } else {
                            "release".into()
                        },
                        n,
                    ),
                );
            }
        }
        let mut q=db.prepare("SELECT rowid,entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id FROM agent_budget_entries WHERE source_id=?1 OR idempotency_key LIKE ?2 ORDER BY entry_id").map_err(|e|e.to_string())?;
        let rows = q
            .query_map(params![self.call.id, format!("{prefix}%")], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, i64>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (physical, id, root, assignment, owner, dimension, kind, n, key, source) =
                row.map_err(|e| e.to_string())?;
            if physical <= 0
                || uuid::Uuid::parse_str(&id).is_err()
                || root != self.call.owner.root
                || !assignment.is_empty()
                || owner != self.call.owner.id
                || source != self.call.id
                || expected.remove(&key) != Some((dimension, kind, n))
            {
                return Err("root_tick_original_cost_changed".into());
            }
        }
        if !expected.is_empty() {
            return Err("root_tick_original_cost_missing".into());
        }
        Ok(())
    }
}
pub(super) fn invoice_response(invoice: &Value) -> Result<ModelResponse, String> {
    let invalid = "root_tick_usage_invalid";
    let usage = &invoice["usage"];
    if invoice.as_object().is_none_or(|o| o.len() != 3)
        || invoice["responseHash"].as_str().is_none_or(|h| {
            h.len() != 64
                || !h
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        })
        || usage.as_object().is_none_or(|o| o.len() != 5)
    {
        return Err(invalid.into());
    }
    let n = |k: &str| usage[k].as_i64().ok_or(invalid.to_string());
    Ok(ModelResponse {
        // Never infer explicit cache reporting from a legacy numeric invoice.
        cached_usage_reported: false,
        text: String::new(),
        tool_calls: vec![],
        finish_reason: "stop".into(),
        usage_reported: invoice["usageReported"].as_bool().ok_or(invalid)?,
        usage: UsageDelta {
            input_tokens: n("inputTokens")?,
            cached_input_tokens: n("cachedInputTokens")?,
            output_tokens: n("outputTokens")?,
            total_tokens: n("totalTokens")?,
            model_requests: n("modelRequests")?,
        },
    })
}
