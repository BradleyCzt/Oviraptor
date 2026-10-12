//! Append-only physical proof of an explicitly confirmed exact settlement.
use super::super::Tick;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub(super) fn costs(db: &Connection, tick: &Tick) -> Result<Value, String> {
    let mut q = db
        .prepare(
            "SELECT rowid,entry_id,dimension,kind,amount,idempotency_key,created_at
        FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=''
        AND lease_attempt_id=?2 AND source_id=?3 ORDER BY entry_id",
        )
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map(
            params![tick.call.owner.root, tick.call.owner.id, tick.call.id],
            |r| {
                Ok(
                    json!({"rowid":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,
        "dimension":r.get::<_,String>(2)?,"kind":r.get::<_,String>(3)?,"amount":r.get::<_,i64>(4)?,
        "key":r.get::<_,String>(5)?,"created":r.get::<_,String>(6)?}),
                )
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(json!(rows))
}

pub(super) fn public(preview: &Value) -> Value {
    let mut receipt = preview.clone();
    receipt["receiptId"] = preview["confirmationHash"].clone();
    receipt["actualCostConfirmed"] = json!(true);
    receipt["executionAllowed"] = json!(false);
    receipt["automaticResumeAllowed"] = json!(false);
    receipt
}

pub(super) fn read(db: &Connection, tick: &Tick) -> Result<Option<Value>, String> {
    let row: Option<(String,String,i64,String,String)> = db
        .query_row(
            "SELECT receipt_id,scan_id,attempt_number,invoice_hash,fact_json FROM agent_root_cost_reconciliation_receipts
         WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3",
            params![tick.call.id, tick.call.owner.root, tick.call.owner.id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    row.map(|(id, scan, attempt, invoice_hash, raw)| {
        let fact: Value =
            serde_json::from_str(&raw).map_err(|_| "root_reconciliation_receipt_invalid")?;
        let canonical = fact.to_string();
        if canonical != raw || fact.as_object().is_none_or(|o| o.len() != 3) {
            return Err("root_reconciliation_receipt_invalid".into());
        }
        let preview = super::preview::binding(
            db,
            tick,
            fact["preview"]["operation"]
                .as_str()
                .ok_or("root_reconciliation_receipt_invalid")?,
        )?;
        tick.verify_original_terminal_proof(db)?;
        if preview["confirmationHash"] != id
            || preview["scanId"] != scan
            || preview["attemptNumber"] != attempt
            || preview["invoiceHash"] != invoice_hash
        {
            return Err("root_reconciliation_receipt_changed".into());
        }
        if fact["preview"] != preview
            || fact["receipt"] != public(&preview)
            || fact["costProof"] != costs(db, tick)?
        {
            return Err("root_reconciliation_receipt_changed".into());
        }
        verify_actual_costs(db, tick, &preview)?;
        Ok(fact["receipt"].clone())
    })
    .transpose()
}

fn verify_actual_costs(db: &Connection, tick: &Tick, preview: &Value) -> Result<(), String> {
    for (dimension, key) in [
        ("model_input_tokens", "inputTokens"),
        ("model_cached_tokens", "cachedInputTokens"),
        ("model_output_tokens", "outputTokens"),
        ("model_requests", "modelRequests"),
    ] {
        let (paid,debt,held): (i64,i64,i64) = db.query_row(
            "SELECT COALESCE(SUM(CASE WHEN kind IN ('consume','reconcile') THEN amount ELSE 0 END),0),
             COALESCE(SUM(CASE kind WHEN 'forfeit' THEN amount WHEN 'reconcile' THEN -amount ELSE 0 END),0),
             COALESCE(SUM(CASE kind WHEN 'reserve' THEN amount WHEN 'consume' THEN -amount
               WHEN 'release' THEN -amount WHEN 'forfeit' THEN -amount ELSE 0 END),0)
             FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=''
             AND lease_attempt_id=?2 AND source_id=?3 AND dimension=?4",
            params![tick.call.owner.root,tick.call.owner.id,tick.call.id,dimension],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).map_err(|e| e.to_string())?;
        if preview["usage"][key] != paid || debt != 0 || held != 0 {
            return Err("root_reconciliation_actual_cost_unproven".into());
        }
    }
    Ok(())
}
