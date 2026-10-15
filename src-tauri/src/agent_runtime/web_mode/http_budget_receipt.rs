//! Typed first HTTP-cost receipt. Saved JSON has verification only, no resealer.
use super::{root, ModeFact, VerifiedMode, WebMode};
use rusqlite::Connection;
use serde_json::Value;

pub(crate) const RULE: &str = "original_received_http_target_cost_advisory_v1";
const DOMAIN: &str = "oviraptor.http-target-budget-receipt.v1";

pub(crate) struct FreshHttpBudgetReceipt {
    original: VerifiedMode,
    value: Value,
}

impl FreshHttpBudgetReceipt {
    // Only the actual transport's private pre-send witness can enter this path.
    pub(crate) fn from_transport_on(
        db: &Connection,
        witness: &crate::scanning::http_budget_receipt::FreshHttpBudgetClaim,
        status: u16,
    ) -> Result<Self, String> {
        let (original, value) = witness.received_fact_on(db, status)?;
        require_protocol_on(
            db,
            value["rootRunId"]
                .as_str()
                .ok_or("http_budget_receipt_scope_invalid")?,
            &original,
        )?;
        Ok(Self { original, value })
    }

    pub(crate) fn message_for_original(&self, fact: &ModeFact) -> Result<Vec<u8>, String> {
        if self.original.fact() != fact {
            return Err("http_budget_receipt_original_mode_changed".into());
        }
        message(&self.value)
    }

    pub(crate) fn sealed(&self) -> Result<(Value, Vec<u8>), String> {
        Ok((
            self.value.clone(),
            self.original.sign_fresh_http_budget_receipt(self)?,
        ))
    }
}

pub(crate) fn require_protocol_on(
    db: &Connection,
    root_id: &str,
    original: &VerifiedMode,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("http_budget_receipt_snapshot_required".into());
    }
    root::verify_birth(db, root_id, original)?;
    let declaration = root::read(db, root_id)?.ok_or("http_budget_receipt_birth_missing")?;
    let bootstrap = declaration.bootstrap_dispatch().unwrap_or(Value::Null);
    if declaration.mode() != WebMode::Multi
        || !matches!(bootstrap["schemaVersion"].as_i64(), Some(8..=15))
        || bootstrap["httpBudgetConsumptionRule"] != RULE
        || declaration.fact() != original.fact()
    {
        return Err("http_budget_receipt_original_v8_protocol_required".into());
    }
    Ok(())
}

pub(crate) fn verify_saved_on(
    db: &Connection,
    original: &VerifiedMode,
    value: &Value,
    signature: &[u8],
) -> Result<(), String> {
    let root_id = value["rootRunId"]
        .as_str()
        .ok_or("http_budget_receipt_scope_invalid")?;
    require_protocol_on(db, root_id, original)?;
    // Authenticate before reading any saved physical-source description.
    original.verify_http_budget_receipt(&message(value)?, signature)?;
    if value["schemaVersion"] != 1
        || value["purpose"] != "original_received_http_target_cost"
        || value["modeReceiptId"] != original.fact().receipt_id
        || value["scanId"] != original.fact().scan_id
        || value["attemptNumber"] != original.fact().attempt_number
        || value["targetRequests"] != 1
        || value["originalFacts"]
            .as_array()
            .is_none_or(|rows| rows.is_empty())
    {
        return Err("http_budget_receipt_scope_invalid".into());
    }
    let binding: String = db
        .query_row(
            "SELECT binding_json FROM main.agent_root_mode_birth_seals WHERE root_run_id=?1",
            [root_id],
            |row| row.get(0),
        )
        .map_err(|_| "http_budget_receipt_birth_missing")?;
    if value["birthBindingHash"] != crate::agent_runtime::store::stable_hash(&binding) {
        return Err("http_budget_receipt_birth_changed".into());
    }
    Ok(())
}

fn message(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&serde_json::json!({"domain":DOMAIN,"receipt":value}))
        .map_err(|e| e.to_string())
}
