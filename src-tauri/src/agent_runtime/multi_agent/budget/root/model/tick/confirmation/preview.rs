//! Pure preview binds the original physical invoice and request, never today's
//! mutable balance alone. It does not issue execution or transport authority.
use super::super::{unpublished_projection::invoice_response, Tick};
use crate::agent_runtime::store;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub(super) fn scoped_tick(
    db: &Connection,
    scan: &str,
    attempt: i64,
    root: &str,
    call: &str,
) -> Result<Tick, String> {
    let current: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2)
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            params![scan, attempt],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if attempt <= 0 || !current {
        return Err("root_reconciliation_attempt_changed".into());
    }
    let tick = Tick::original_request_for_projection(db, root, call)?;
    if tick.request_fact["version"] != 3 {
        return Err("root_tick_reconcile_requires_v3".into());
    }
    let scope = &tick.call.owner.contract["root"];
    if scope["scan"] != scan || scope["attempt"] != attempt {
        return Err("root_reconciliation_scope_conflict".into());
    }
    Ok(tick)
}

pub(super) fn binding(db: &Connection, tick: &Tick, operation: &str) -> Result<Value, String> {
    if !matches!(operation, "settle_received" | "reconcile_forfeit") {
        return Err("root_reconciliation_operation_invalid".into());
    }
    let physical: Value = db
        .query_row(
            "SELECT rowid,receipt_json,created_at FROM agent_root_model_journal
         WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND round=?4
         AND request_hash=?5 AND phase='received'",
            params![
                tick.call.id,
                tick.call.owner.root,
                tick.call.owner.id,
                tick.call.round,
                tick.call.request
            ],
            |r| {
                Ok(
                    json!({"rowid":r.get::<_,i64>(0)?,"invoice":r.get::<_,String>(1)?,
            "created":r.get::<_,String>(2)?}),
                )
            },
        )
        .map_err(|_| "root_tick_reconcile_received_missing")?;
    let raw = physical["invoice"]
        .as_str()
        .ok_or("root_tick_json_invalid")?;
    let invoice: Value = serde_json::from_str(raw).map_err(|_| "root_tick_json_invalid")?;
    let canonical = invoice.to_string();
    if canonical != raw {
        return Err("root_tick_json_noncanonical".into());
    }
    tick.call.verify(db, "received", &invoice)?;
    let response = invoice_response(&invoice)?;
    let u = &response.usage;
    if !response.usage_reported
        || u.model_requests != 1
        || u.cached_input_tokens > u.input_tokens
        || [
            u.input_tokens,
            u.cached_input_tokens,
            u.output_tokens,
            u.total_tokens,
        ]
        .iter()
        .any(|n| !(0..=9_007_199_254_740_991).contains(n))
        || u.input_tokens.checked_add(u.output_tokens) != Some(u.total_tokens)
    {
        return Err("root_tick_reconcile_actual_cost_unverified".into());
    }
    let scope = &tick.call.owner.contract["root"];
    let mut result = json!({"version":1,"scanId":scope["scan"],"attemptNumber":scope["attempt"],
        "rootRunId":tick.call.owner.root,"callId":tick.call.id,"leaseAttemptId":tick.call.owner.id,
        "round":tick.call.round,"requestHash":tick.call.request,"basisHash":tick.basis,
        "invoiceHash":store::stable_hash(raw),"usage":invoice["usage"],"operation":operation});
    let hash = store::stable_hash(
        &json!({"preview":result,"invoiceProof":physical,
        "requestProof":tick.request_row(db)?})
        .to_string(),
    );
    result["confirmationHash"] = json!(hash);
    Ok(result)
}

pub(super) fn preview(db: &Connection, tick: &Tick) -> Result<Value, String> {
    let terminal: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_tick_terminal_proofs WHERE call_id=?1)",
            [&tick.call.id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let operation = if terminal {
        tick.verify_original_terminal_proof(db)?;
        "reconcile_forfeit"
    } else {
        let progressed: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1
             AND lease_attempt_id=?2 AND source_id=?3 AND kind<>'reserve')",
                params![tick.call.owner.root, tick.call.owner.id, tick.call.id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if progressed {
            return Err("root_tick_reconcile_cost_already_progressed".into());
        }
        "settle_received"
    };
    let result = binding(db, tick, operation)?;
    let raw: String = db.query_row(
        "SELECT receipt_json FROM agent_root_model_journal WHERE call_id=?1 AND phase='received'",
        [&tick.call.id], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    let response =
        invoice_response(&serde_json::from_str(&raw).map_err(|_| "root_tick_json_invalid")?)?;
    if !tick.call.known_cost_before_settlement(db, &response)? {
        return Err("root_tick_reconcile_actual_cost_unverified".into());
    }
    if operation == "reconcile_forfeit" {
        for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS[..3].iter() {
            let actual = result["usage"][match *dimension {
                "model_input_tokens" => "inputTokens",
                "model_cached_tokens" => "cachedInputTokens",
                _ => "outputTokens",
            }]
            .as_i64()
            .ok_or("root_tick_usage_invalid")?;
            let amount: i64 = db.query_row(
                "SELECT amount FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=''
                 AND lease_attempt_id=?2 AND source_id=?3 AND dimension=?4 AND kind='forfeit'",
                params![tick.call.owner.root,tick.call.owner.id,tick.call.id,dimension], |r| r.get(0),
            ).map_err(|_| "root_tick_reconcile_forfeit_missing")?;
            if amount <= 0 || amount != actual {
                return Err("root_tick_reconcile_actual_cost_mismatch".into());
            }
        }
    }
    Ok(result)
}
