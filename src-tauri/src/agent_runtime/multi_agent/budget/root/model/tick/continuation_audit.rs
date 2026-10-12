//! Pure original paid-call proof for financial continuation. No live grant,
//! read-side repair, old semantic replay or transport. Legacy proof stays legacy.
use super::{unpublished_projection::invoice_response, Tick};
use rusqlite::{params, types::ValueRef, Connection, Params};
use serde_json::{json, Value};

impl Tick {
    pub(crate) fn audit_cost_for_continuation(
        snapshot: &Connection,
        root: &str,
        call: &str,
    ) -> Result<Value, String> {
        let tick = Self::original_request_for_projection(snapshot, root, call)?;
        if tick.request_fact["version"] != 3 {
            return Err("continuation_original_v3_proof_required".into());
        }
        let terminals: Vec<(String, String)> = snapshot
            .prepare(
                "SELECT phase,receipt_json FROM agent_root_model_journal
             WHERE call_id=?1 AND phase<>'dispatch' ORDER BY phase",
            )
            .map_err(|e| e.to_string())?
            .query_map([call], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        if terminals.len() != 1 {
            return Err("continuation_original_call_unsettled".into());
        }
        let (phase, raw) = &terminals[0];
        let invoice: Value = serde_json::from_str(raw).map_err(|_| "root_tick_json_invalid")?;
        let canonical = invoice.to_string();
        if canonical != *raw {
            return Err("root_tick_json_noncanonical".into());
        }
        tick.call.verify(snapshot, phase, &invoice)?;
        tick.verify_original_terminal_proof(snapshot)?;
        let mut receipt = Value::Null;
        let mut original_semantic_available = false;
        match phase.as_str() {
            "received" => {
                let response = invoice_response(&invoice)?;
                // Validate the exact original settlement write set, even when
                // a later explicit receipt reconciled the original forfeits.
                tick.cost_rows(snapshot, Some(&response))?;
                let confirmed = Self::read_original_cost_reconciliation(snapshot, root, call)?;
                if tick.cost_unknown(snapshot, &response)? && confirmed.is_none() {
                    return Err("continuation_original_cost_unconfirmed".into());
                }
                if let Some(confirmed) = confirmed {
                    receipt = confirmed;
                }
                if let Some(saved) = tick.load_decision(snapshot)? {
                    tick.verify_paid(snapshot, &saved)?;
                    let sequence = tick.published(snapshot, &saved)?;
                    if let Some(sequence) = sequence {
                        Self::project_publication(snapshot, root, sequence)?;
                        original_semantic_available = true;
                    }
                } else if tick.read(snapshot, "publication")?.is_some() {
                    return Err("continuation_original_publication_unproven".into());
                }
            }
            "unsent" => {
                tick.verify_stopped_cost_projection(snapshot, Some("unsent"))?;
            }
            _ => return Err("continuation_original_call_unsettled".into()),
        }
        let scope = params![call, root, tick.call.owner.id];
        Ok(
            json!({"version":1,"rootRunId":root,"callId":call,"phase":phase,
            "originalSemanticAvailable":original_semantic_available,"semanticReplayAllowed":false,
            "confirmationReceipt":receipt,
            "requestRows":rows(snapshot,"SELECT rowid,* FROM agent_root_tick_receipts
                WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 ORDER BY phase",scope)?,
            "modelRows":rows(snapshot,"SELECT rowid,* FROM agent_root_model_journal
                WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 ORDER BY phase",scope)?,
            "terminalRows":rows(snapshot,"SELECT rowid,* FROM agent_root_tick_terminal_proofs
                WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3",scope)?,
            "costRows":rows(snapshot,"SELECT rowid,* FROM agent_budget_entries WHERE source_id=?1
                AND root_run_id=?2 AND lease_attempt_id=?3 AND assignment_id='' ORDER BY entry_id",scope)?,
            "confirmationRows":rows(snapshot,"SELECT rowid,* FROM agent_root_cost_reconciliation_receipts
                WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3",scope)?}),
        )
    }
}

fn rows(db: &Connection, sql: &str, parameters: impl Params) -> Result<Value, String> {
    let mut statement = db.prepare(sql).map_err(|e| e.to_string())?;
    let columns = statement.column_count();
    let mut query = statement.query(parameters).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    while let Some(row) = query.next().map_err(|e| e.to_string())? {
        let mut values = Vec::new();
        for index in 0..columns {
            values.push(match row.get_ref(index).map_err(|e| e.to_string())? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => json!(n),
                ValueRef::Real(n) => json!(n),
                ValueRef::Text(text) => {
                    json!(std::str::from_utf8(text)
                        .map_err(|_| "continuation_original_text_invalid")?)
                }
                ValueRef::Blob(_) => return Err("continuation_original_type_invalid".into()),
            });
        }
        result.push(json!(values));
    }
    Ok(json!(result))
}
