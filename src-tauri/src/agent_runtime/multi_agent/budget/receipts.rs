//! A verified provider receipt charges its call before tools or child closure.
//! Unknown breakdowns retain estimates. A response is never execution authority.
use super::{Kind, DIMENSIONS};
use crate::agent_runtime::{multi_agent::lease::CoordinatorLease, store::UsageDelta};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub(crate) fn verify(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
    call_source: &str,
    receipt_hash: &str,
    usage: &UsageDelta,
) -> Result<(), String> {
    let expected_attempt = super::super::attempts::current(db, lease, assignment)?.id;
    verify_for_attempt(
        db,
        lease,
        assignment,
        call_source,
        receipt_hash,
        usage,
        &expected_attempt,
    )
}

fn verify_for_attempt(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
    call_source: &str,
    receipt_hash: &str,
    usage: &UsageDelta,
    expected_attempt: &str,
) -> Result<(), String> {
    let source = format!("{call_source}:receipt:{receipt_hash}");
    let prefix = super::scope::stored_key(
        db,
        &lease.root_run_id,
        assignment,
        expected_attempt,
        &format!("received:{call_source}:"),
    )?;
    let unknown: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1
        AND assignment_id=?2 AND substr(idempotency_key,1,length(?3))=?3 AND lease_attempt_id=?4 AND kind='forfeit')",
            params![lease.root_run_id, assignment, prefix, expected_attempt],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    for (index, (dimension, amount)) in DIMENSIONS[..4]
        .iter()
        .zip([
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.output_tokens,
            usage.model_requests,
        ])
        .enumerate()
    {
        let row: Option<(String, String, String, String, i64)> = db
            .query_row(
                "SELECT assignment_id,lease_attempt_id,source_id,kind,amount
            FROM agent_budget_entries WHERE root_run_id=?1 AND idempotency_key=?2 AND dimension=?3",
                params![lease.root_run_id, format!("{prefix}{dimension}"), dimension],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((owner, attempt, saved_source, kind, charged)) = row {
            if owner != assignment
                || attempt != expected_attempt
                || saved_source != source
                || (index == 3 || !unknown) && (kind != "consume" || charged != amount)
                || index < 3 && unknown && (kind != "forfeit" || charged <= 0)
            {
                return Err("budget_receipt_journal_conflict".into());
            }
        } else if index == 3 || !unknown && amount > 0 {
            return Err("budget_receipt_journal_missing".into());
        }
    }
    Ok(())
}

pub(crate) fn record(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    call_source: &str,
    receipt_hash: &str,
    usage: &UsageDelta,
    estimate: Option<i64>,
) -> Result<(), String> {
    let owner = ReceiptOwner::current(tx, lease, assignment)?;
    record_with_provenance(
        tx,
        lease,
        assignment,
        call_source,
        receipt_hash,
        usage,
        estimate,
        true,
        &owner,
    )
}

pub(crate) fn record_estimated(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    call_source: &str,
    receipt_hash: &str,
    usage: &UsageDelta,
    estimate: Option<i64>,
) -> Result<(), String> {
    let owner = ReceiptOwner::current(tx, lease, assignment)?;
    record_with_provenance(
        tx,
        lease,
        assignment,
        call_source,
        receipt_hash,
        usage,
        estimate,
        false,
        &owner,
    )
}

use write::record_with_provenance;
mod owner;
mod write;
use owner::ReceiptOwner;

pub(crate) struct OriginalReceiptOwner(ReceiptOwner);

impl OriginalReceiptOwner {
    pub(crate) fn is_settled(&self, db: &Connection) -> Result<bool, String> {
        self.verify(db)?;
        for dimension in &DIMENSIONS[..4] {
            let balance=self.0.balance(db,dimension)?;
            if balance.reserved != 0 || balance.indeterminate != 0 { return Ok(false); }
        }
        Ok(true)
    }
    pub(crate) fn attempt_id(&self) -> &str {
        &self.0.attempt
    }

    pub(crate) fn verify(&self, db: &Connection) -> Result<(), String> {
        self.0.verify(db)
    }

    pub(crate) fn is_unresolved(&self, db: &Connection) -> Result<bool, String> {
        self.verify(db)?;
        for dimension in &DIMENSIONS[..4] {
            if self.0.balance(db, dimension)?.indeterminate > 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

pub(crate) fn verify_original_received(
    db: &Connection, owner: &OriginalReceiptOwner, call_source: &str,
    receipt_hash: &str, usage: &UsageDelta,
) -> Result<(), String> {
    owner.verify(db)?;
    verify_for_attempt(db,&owner.0.lease,&owner.0.assignment,call_source,receipt_hash,usage,owner.attempt_id())
}

pub(crate) fn original_owner(
    db: &Connection,
    run: &str,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<OriginalReceiptOwner, String> {
    Ok(OriginalReceiptOwner(ReceiptOwner::original(
        db, run, lease, assignment,
    )?))
}

/// The caller verifies the durable original provider dispatch and receipt in
/// this same transaction before calling and again after the last cost write.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_original(
    tx: &Transaction<'_>,
    owner: &OriginalReceiptOwner,
    call_source: &str,
    receipt_hash: &str,
    usage: &UsageDelta,
    reported: bool,
) -> Result<(), String> {
    record_original_estimated(tx, owner, call_source, receipt_hash, usage, None, reported)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_original_estimated(
    tx: &Transaction<'_>,
    owner: &OriginalReceiptOwner,
    call_source: &str,
    receipt_hash: &str,
    usage: &UsageDelta,
    estimate: Option<i64>,
    reported: bool,
) -> Result<(), String> {
    owner.verify(tx)?;
    record_with_provenance(
        tx,
        &owner.0.lease,
        &owner.0.assignment,
        call_source,
        receipt_hash,
        usage,
        estimate,
        reported,
        &owner.0,
    )
}

pub(crate) fn forfeit_original_call(
    tx: &Transaction<'_>,
    owner: &OriginalReceiptOwner,
    source: &str,
) -> Result<(), String> {
    forfeit_original_call_estimated(tx, owner, source, None)
}

pub(crate) fn forfeit_original_call_estimated(
    tx: &Transaction<'_>,
    owner: &OriginalReceiptOwner,
    source: &str,
    estimate: Option<i64>,
) -> Result<(), String> {
    owner.verify(tx)?;
    for (index, dimension) in DIMENSIONS[..4].iter().enumerate() {
        let reserved = owner.0.balance(tx, dimension)?.reserved;
        let amount = if index == 3 {
            reserved.min(1)
        } else {
            reserved.min(estimate.unwrap_or(reserved))
        };
        if amount > 0 {
            owner.0.append(
                tx,
                dimension,
                Kind::Forfeit,
                amount,
                &format!("unknown:{source}:{dimension}"),
                source,
            )?;
        }
    }
    owner.verify(tx)
}
