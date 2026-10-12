//! Read-only original financial facts. This type cannot issue a Root, budget,
//! attempt, execution capability or user approval.
use super::{root::model::tick::Tick, root::RootOwner, DIMENSIONS};
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

mod accounting;
mod proofs;
mod targets;
pub(crate) mod single;
pub(crate) mod single_remaining;
pub(crate) mod migration;

#[cfg(test)]
pub(crate) fn verify_original_targets_on(db: &Connection, root: &str) -> Result<(), String> {
    targets::verify(db, root)
}

pub(crate) struct VerifiedFinancialSource {
    root: String,
    receipt: String,
    facts: Value,
}

impl VerifiedFinancialSource {
    pub(crate) fn as_json(&self) -> Value {
        self.facts.clone()
    }

    /// Repeat the original physical audit in the caller's snapshot. Elapsed
    /// wall time is checked against the original deadline, never hashed as a
    /// changing TTL and never interpreted as a renewed timeout.
    pub(crate) fn verify_on(&self, db: &Connection) -> Result<(), String> {
        let actual = settled_source_on(db, &self.root, &self.receipt)?;
        if actual.facts != self.facts {
            return Err("continuation_original_finance_changed".into());
        }
        Ok(())
    }
}

pub(crate) fn settled_source_on(
    db: &Connection,
    root: &str,
    required_receipt: &str,
) -> Result<VerifiedFinancialSource, String> {
    settled_source_with_deadline_on(db, root, required_receipt, true)
}

/// Historical provenance remains readable after expiry. This cannot grant a
/// new execution: the private approval constructor uses the live reader above.
pub(crate) fn settled_source_history_on(
    db: &Connection,
    root: &str,
    required_receipt: &str,
) -> Result<VerifiedFinancialSource, String> {
    settled_source_with_deadline_on(db, root, required_receipt, false)
}

fn settled_source_with_deadline_on(
    db: &Connection,
    root: &str,
    required_receipt: &str,
    require_live_deadline: bool,
) -> Result<VerifiedFinancialSource, String> {
    if db.is_autocommit() {
        return Err("continuation_financial_snapshot_required".into());
    }
    if required_receipt.len() != 64
        || !required_receipt
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("continuation_original_receipt_required".into());
    }
    let owner = RootOwner::load_original(db, root)?;
    let text: String = db
        .query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let contract: Value =
        serde_json::from_str(&text).map_err(|_| "budget_root_original_owner_conflict")?;
    let c = &contract["coordinator"];
    let actor = CoordinatorLease {
        scan_id: c["scan"]
            .as_str()
            .ok_or("continuation_original_coordinator_required")?
            .into(),
        attempt_number: c["attempt"]
            .as_i64()
            .ok_or("continuation_original_coordinator_required")?,
        target_key: c["target"]
            .as_str()
            .ok_or("continuation_original_coordinator_required")?
            .into(),
        root_run_id: root.into(),
        lease_epoch: c["epoch"]
            .as_i64()
            .ok_or("continuation_original_coordinator_required")?,
        fencing_token: c["fence"]
            .as_str()
            .ok_or("continuation_original_coordinator_required")?
            .into(),
        lease_expires_at: String::new(),
    };
    owner.require_original_coordinator(db, &actor)?;
    let call: String = db
        .query_row(
            "SELECT call_id FROM agent_root_cost_reconciliation_receipts
         WHERE receipt_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3
         AND scan_id=?4 AND attempt_number=?5",
            params![
                required_receipt,
                root,
                owner.id,
                actor.scan_id,
                actor.attempt_number
            ],
            |r| r.get(0),
        )
        .map_err(|_| "continuation_original_receipt_required")?;
    let receipt = Tick::read_original_cost_reconciliation(db, root, &call)?
        .ok_or("continuation_original_receipt_required")?;
    if receipt["receiptId"] != required_receipt || receipt["actualCostConfirmed"] != true {
        return Err("continuation_original_receipt_changed".into());
    }
    let amounts = accounting::settled(db, root, Some(&actor), require_live_deadline)?;
    proofs::verify_calls(db, root, Some(&actor))?;
    let physical = proofs::physical_hashes(db, root, false)?;
    let facts = json!({
        "version":1,"proofKind":"settled_original_finance",
        "rootRunId":root,"ownerId":owner.id,"scanId":actor.scan_id,
        "attemptNumber":actor.attempt_number,"targetKey":actor.target_key,
        "coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,
        "receiptId":required_receipt,"dimensions":DIMENSIONS,"remaining":amounts.remaining,
        "sharedRemainingTokens":amounts.tokens,"sharedRemainingRequests":amounts.requests,
        "originalDeadline":amounts.deadline,"rootPhysicalHash":physical[0],
        "ownerPhysicalHash":physical[1],"financialPhysicalHash":physical[2],
        "executionAllowed":false,
    });
    Ok(VerifiedFinancialSource {
        root: root.into(),
        receipt: required_receipt.into(),
        facts,
    })
}

/// Audit historical obligations without requiring an older Root's deadline to
/// remain live. This proof cannot be used as the selected continuation source.
pub(crate) fn verify_settled_history_on(db: &Connection, root: &str) -> Result<Value, String> {
    if db.is_autocommit() {
        return Err("continuation_financial_snapshot_required".into());
    }
    let owner = RootOwner::load_original(db, root)?;
    let raw: String = db
        .query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let contract: Value =
        serde_json::from_str(&raw).map_err(|_| "budget_root_original_owner_conflict")?;
    let c = &contract["coordinator"];
    let actor = if c.is_null() {
        None
    } else {
        Some(CoordinatorLease {
            scan_id: c["scan"]
                .as_str()
                .ok_or("budget_root_scope_invalid")?
                .into(),
            attempt_number: c["attempt"].as_i64().ok_or("budget_root_scope_invalid")?,
            target_key: c["target"]
                .as_str()
                .ok_or("budget_root_scope_invalid")?
                .into(),
            root_run_id: root.into(),
            lease_epoch: c["epoch"].as_i64().ok_or("budget_root_scope_invalid")?,
            fencing_token: c["fence"]
                .as_str()
                .ok_or("budget_root_scope_invalid")?
                .into(),
            lease_expires_at: String::new(),
        })
    };
    if let Some(actor) = &actor {
        owner.require_original_coordinator(db, actor)?;
    }
    accounting::settled(db, root, actor.as_ref(), false)?;
    proofs::verify_calls(db, root, actor.as_ref())?;
    let physical = proofs::physical_hashes(db, root, actor.is_none())?;
    Ok(
        json!({"version":1,"proofKind":"settled_original_history","rootRunId":root,
        "rootPhysicalHash":physical[0],"ownerPhysicalHash":physical[1],
        "financialPhysicalHash":physical[2],"executionAllowed":false}),
    )
}
