//! Local explicit user confirmation. One transaction writes exact costs,
//! original terminal proof and immutable confirmation. Never SDK or target I/O.
use super::Tick;
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Value};
mod preview;
mod receipts;

impl Tick {
    /// Historical financial evidence only. Unlike the user confirmation
    /// command, this read cannot confirm a bill or authorize a current attempt.
    pub(crate) fn read_original_cost_reconciliation(
        snapshot: &Connection,
        root: &str,
        call: &str,
    ) -> Result<Option<Value>, String> {
        let tick = Self::original_request_for_projection(snapshot, root, call)?;
        if tick.request_fact["version"] != 3 {
            return Err("root_tick_reconcile_requires_v3".into());
        }
        receipts::read(snapshot, &tick)
    }
    pub(crate) fn preview_cost_reconciliation(
        snapshot: &Connection,
        scan: &str,
        attempt: i64,
        root: &str,
        call: &str,
    ) -> Result<Value, String> {
        let tick = preview::scoped_tick(snapshot, scan, attempt, root, call)?;
        preview::preview(snapshot, &tick)
    }
    pub(crate) fn read_cost_reconciliation(
        snapshot: &Connection,
        scan: &str,
        attempt: i64,
        root: &str,
        call: &str,
    ) -> Result<Option<Value>, String> {
        let tick = preview::scoped_tick(snapshot, scan, attempt, root, call)?;
        receipts::read(snapshot, &tick)
    }
    pub(crate) fn confirm_cost_reconciliation(
        db: &Connection,
        scan: &str,
        attempt: i64,
        root: &str,
        call: &str,
        hash: &str,
    ) -> Result<Value, String> {
        if !db.is_autocommit() {
            return Err("root_tick_reconcile_requires_own_transaction".into());
        }
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return Err("root_reconciliation_confirmation_invalid".into());
        }
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        let result = protect(&tx, || {
            let tick = preview::scoped_tick(&tx, scan, attempt, root, call)?;
            if let Some(receipt) = receipts::read(&tx, &tick)? {
                if receipt["confirmationHash"] != hash {
                    return Err("root_reconciliation_confirmation_changed".into());
                }
                return Ok(receipt);
            }
            let preview = preview::preview(&tx, &tick)?;
            if preview["confirmationHash"] != hash {
                return Err("root_reconciliation_confirmation_changed".into());
            }
            match preview["operation"].as_str() {
                Some("settle_received") => {
                    let (original, capture) = Self::settle_unsettled_received_in(&tx, root, call)?;
                    original.persist_terminal_capture_in(&tx, capture)?;
                }
                Some("reconcile_forfeit") => Self::reconcile_forfeited_unknown_in(&tx, root, call)?,
                _ => return Err("root_reconciliation_operation_invalid".into()),
            }
            let receipt = receipts::public(&preview);
            let fact = json!({"preview":preview,"receipt":receipt,"costProof":receipts::costs(&tx, &tick)?});
            let text = fact.to_string();
            if text.len() > 1024 * 1024 {
                return Err("root_reconciliation_receipt_too_large".into());
            }
            let n = tx.execute("INSERT INTO agent_root_cost_reconciliation_receipts
                (receipt_id,scan_id,attempt_number,root_run_id,call_id,lease_attempt_id,invoice_hash,fact_json)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                params![hash,scan,attempt,root,call,tick.call.owner.id,preview["invoiceHash"].as_str(),text])
                .map_err(|e| e.to_string())?;
            if n != 1 || receipts::read(&tx, &tick)? != Some(receipt.clone()) {
                return Err("root_reconciliation_insert_unconfirmed".into());
            }
            Ok(receipt)
        })?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(result)
    }
}

fn protect<T>(
    tx: &rusqlite::Transaction<'_>,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    tx.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Insert { table_name }
            if c.database_name == Some("main")
                && c.accessor.is_none()
                && matches!(
                    table_name,
                    "agent_budget_entries"
                        | "agent_root_tick_terminal_proofs"
                        | "agent_root_cost_reconciliation_receipts"
                ) =>
        {
            Authorization::Allow
        }
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }))
    .map_err(|e| e.to_string())?;
    let result = work();
    let clear = tx
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let value = result?;
    clear?;
    Ok(value)
}
