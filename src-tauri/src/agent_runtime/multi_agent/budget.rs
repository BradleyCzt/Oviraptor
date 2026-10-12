//! Fenced append-only budget transitions. All writes require the caller's
//! transaction, so admission, assignment, lane and accounting commit together.
use super::lease::{validate_coordinator_lease, CoordinatorLease};
use rusqlite::{params, Transaction};

pub(crate) mod admission;
pub(crate) mod clock;
pub(crate) mod continuation;
pub(crate) mod diagnostics;
mod entries;
mod execution_slots;
pub(crate) use entries::balance_for_attempt;
mod historical;
pub(crate) mod limits;
pub(crate) mod model;
pub(crate) mod model_facts;
pub(crate) mod receipts;
pub(crate) mod root;
pub(crate) mod root_definition;
mod scope;
pub(crate) mod supervision_capacity;
pub(crate) mod target;
pub(crate) mod web_lifetime;

// SQL aliases d/t denote original dispatch/terminal receipt. A call id alone
// cannot close an obligation restored under a different worker or request.
pub(crate) const WEB_RECEIPT_BINDING: &str = "t.root_run_id=d.root_run_id
    AND t.assignment_id=d.assignment_id AND t.child_run_id=d.child_run_id
    AND t.lease_epoch=d.lease_epoch AND t.fencing_token=d.fencing_token
    AND t.round=d.round AND t.request_hash=d.request_hash";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Authority {
    Live,
    ClockSample,
    TerminalReceipt,
    BoundReceipt,
}

pub const DIMENSIONS: [&str; 10] = [
    "model_input_tokens",
    "model_cached_tokens",
    "model_output_tokens",
    "model_requests",
    "target_requests",
    "browser_actions",
    "controlled_writes",
    "upload_bytes",
    "concurrency_batches",
    "wall_time_ms",
];

/// No live append writer yet; diagnostics coverage stays `denied_by_contract`.
pub const DENIED_BY_CONTRACT_DIMENSIONS: &[&str] = &[
    "browser_actions",
    "controlled_writes",
    "upload_bytes",
];

/// U102: DENIED dims have ledger columns + hard-reject append, but no live writer.
#[cfg(test)]
pub fn denied_budget_dimension_writer_ready(dimension: &str) -> bool {
    if !DENIED_BY_CONTRACT_DIMENSIONS.contains(&dimension) {
        return false;
    }
    false
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BudgetVector {
    pub model_input_tokens: i64,
    pub model_cached_tokens: i64,
    pub model_output_tokens: i64,
    pub model_requests: i64,
    pub target_requests: i64,
    pub browser_actions: i64,
    pub controlled_writes: i64,
    pub upload_bytes: i64,
    pub concurrency_batches: i64,
    pub wall_time_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Reserve,
    Consume,
    Release,
    Forfeit,
    Reconcile,
}
impl Kind {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "reserve" => Ok(Self::Reserve),
            "consume" => Ok(Self::Consume),
            "release" => Ok(Self::Release),
            "forfeit" => Ok(Self::Forfeit),
            "reconcile" => Ok(Self::Reconcile),
            _ => Err("budget_kind_invalid".into()),
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Reserve => "reserve",
            Self::Consume => "consume",
            Self::Release => "release",
            Self::Forfeit => "forfeit",
            Self::Reconcile => "reconcile",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Balance {
    pub reserved: i64,
    pub consumed: i64,
    pub indeterminate: i64,
}

pub fn balance(
    db: &rusqlite::Connection,
    root: &str,
    assignment: Option<&str>,
    dimension: &str,
) -> Result<Balance, String> {
    // A misspelled or unsupported name is not evidence of an unspent budget.
    // Configured zero-use dimensions still read normally; caller ownership
    // and snapshot proofs remain separate requirements.
    if !DIMENSIONS.contains(&dimension) {
        return Err("budget_dimension_unknown".into());
    }
    let mut statement=db.prepare("SELECT kind,amount FROM agent_budget_entries WHERE root_run_id=?1 AND (?2 IS NULL OR assignment_id=?2) AND dimension=?3 ORDER BY rowid").map_err(|e|e.to_string())?;
    let rows = statement
        .query_map(params![root, assignment, dimension], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut b = Balance::default();
    for row in rows {
        let (kind, amount) = row.map_err(|e| e.to_string())?;
        apply(&mut b, &kind, amount)?;
    }
    Ok(b)
}

fn add(value: i64, amount: i64) -> Result<i64, String> {
    value
        .checked_add(amount)
        .ok_or("budget_amount_overflow".into())
}
fn subtract(value: i64, amount: i64) -> Result<i64, String> {
    value
        .checked_sub(amount)
        .filter(|v| *v >= 0)
        .ok_or("budget_transition_exceeds_balance".into())
}
fn apply(b: &mut Balance, kind: &str, amount: i64) -> Result<(), String> {
    match Kind::parse(kind)? {
        Kind::Reserve => b.reserved = add(b.reserved, amount)?,
        Kind::Consume => {
            b.reserved = subtract(b.reserved, amount)?;
            b.consumed = add(b.consumed, amount)?;
        }
        Kind::Release => b.reserved = subtract(b.reserved, amount)?,
        Kind::Forfeit => {
            b.reserved = subtract(b.reserved, amount)?;
            b.indeterminate = add(b.indeterminate, amount)?;
        }
        Kind::Reconcile => {
            b.indeterminate = subtract(b.indeterminate, amount)?;
            b.consumed = add(b.consumed, amount)?;
        }
    }
    Ok(())
}

/// A reconciliation conservatively consumes a previously indeterminate charge.
/// It never refunds sent work. The receipt-verifying caller owns that authority.
#[allow(clippy::too_many_arguments)]
pub fn append(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
) -> Result<(), String> {
    append_inner(
        tx,
        lease,
        assignment,
        dimension,
        kind,
        amount,
        key,
        source,
        Authority::Live,
    )
}

/// Only a separately verified saved receipt may use an expired, unchanged
/// terminal root owner. This never grants capability or changes run status.
#[allow(clippy::too_many_arguments)]
pub(crate) fn append_terminal_receipt(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
) -> Result<(), String> {
    append_inner(
        tx,
        lease,
        assignment,
        dimension,
        kind,
        amount,
        key,
        source,
        Authority::TerminalReceipt,
    )
}

/// A verified failed dispatch can record its unknown debt after expiry. This
/// moves existing reservations only and grants no execution or refund.
pub(crate) fn append_unknown_cost(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    amount: i64,
    source: &str,
) -> Result<(), String> {
    append_inner(
        tx,
        lease,
        assignment,
        dimension,
        Kind::Forfeit,
        amount,
        &format!("unknown:{source}:{dimension}"),
        source,
        Authority::BoundReceipt,
    )
}

// The caller has verified a durable model receipt in this same transaction.
// Recording its cost after expiry never authorizes another dispatch or refund.
#[allow(clippy::too_many_arguments)]
fn append_received_cost(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
) -> Result<(), String> {
    validate_received_transition(tx, lease, dimension, kind)?;
    append_inner(
        tx,
        lease,
        assignment,
        dimension,
        kind,
        amount,
        key,
        source,
        Authority::BoundReceipt,
    )
}

fn validate_received_transition(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    dimension: &str,
    kind: Kind,
) -> Result<(), String> {
    if kind == Kind::Reserve {
        let limit: Option<i64> = tx
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![lease.root_run_id, dimension],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !DIMENSIONS[..3].contains(&dimension) || limit.is_some() {
            return Err("budget_receipt_transition_invalid".into());
        }
    } else if !matches!(kind, Kind::Consume | Kind::Forfeit) {
        return Err("budget_receipt_transition_invalid".into());
    }
    Ok(())
}

fn validate_budget_owner(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    authority: Authority,
) -> Result<(), String> {
    if matches!(authority, Authority::Live | Authority::ClockSample) {
        validate_coordinator_lease(tx, lease)?;
    }
    let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_coordinator_leases c ON c.root_run_id=r.id
        WHERE r.id=?1 AND r.backend='native' AND r.role='coordinator' AND r.status<>'legacy_backend_removed'
        AND r.scan_id=?2 AND r.attempt_number=?3 AND r.target_url=?4
        AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url
        AND c.lease_epoch=?5 AND c.fencing_token=?6 AND (?7=0 OR r.status='terminal'))",
        params![lease.root_run_id,lease.scan_id,lease.attempt_number,lease.target_key,lease.lease_epoch,lease.fencing_token,authority==Authority::TerminalReceipt],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {
        return Err("budget_root_authority_conflict".into());
    }
    Ok(())
}

fn validate_budget_assignment(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    kind: Kind,
) -> Result<(), String> {
    let bound:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND target_key=?3
        AND ((lease_epoch=?4 AND fencing_token=?5) OR (?6='reserve' AND state='prepared' AND lease_epoch=0 AND fencing_token='')))",
        params![assignment,lease.root_run_id,lease.target_key,lease.lease_epoch,lease.fencing_token,kind.as_str()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound {
        return Err("budget_assignment_scope_conflict".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_inner(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    dimension: &str,
    kind: Kind,
    amount: i64,
    key: &str,
    source: &str,
    authority: Authority,
) -> Result<(), String> {
    validate_budget_owner(tx, lease, authority)?;
    if !DIMENSIONS.contains(&dimension)
        || amount <= 0
        || key.trim().is_empty()
        || source.trim().is_empty()
    {
        return Err("budget_entry_invalid".into());
    }
    if DENIED_BY_CONTRACT_DIMENSIONS.contains(&dimension) {
        return Err("budget_dimension_denied_by_contract".into());
    }
    validate_budget_assignment(tx, lease, assignment, kind)?;
    if authority == Authority::Live && kind == Kind::Reserve {
        super::attempts::require_reservation(tx, lease, assignment)?;
    }
    let attempt = super::attempts::current(tx, lease, assignment)?.id;
    let write = if matches!(
        authority,
        Authority::BoundReceipt | Authority::TerminalReceipt
    ) && kind == Kind::Reserve
    {
        entries::persist_known_cost
    } else {
        entries::persist
    };
    write(
        tx,
        &lease.root_run_id,
        assignment,
        attempt.clone(),
        dimension,
        kind,
        amount,
        key,
        source,
    )?;
    validate_budget_owner(tx, lease, authority)?;
    validate_budget_assignment(tx, lease, assignment, kind)?;
    if super::attempts::current(tx, lease, assignment)?.id != attempt {
        return Err("budget_attempt_scope_conflict".into());
    }
    if authority == Authority::Live && kind == Kind::Reserve {
        super::attempts::require_reservation(tx, lease, assignment)?;
    }
    Ok(())
}
