//! Read-only, original Single balances. No remaining amount grants execution.
use super::{accounting, verify_settled_history_on, RootOwner, DIMENSIONS};
use rusqlite::Connection;
use serde_json::{json, Value};

/// Read all ten dimensions from the original signed owner's snapshot. An
/// expired deadline remains auditable; a future execution approval must check
/// its liveness separately and cannot interpret this result as renewed time.
#[cfg(test)]
pub(crate) fn remaining_on(db: &Connection, root: &str) -> Result<Value, String> {
    read_on(db, root, false).map(|(_, remaining)| remaining)
}

/// Capture the exact existing history shape and balances in one verified
/// snapshot pass for the sibling's typed original-source constructor.
pub(super) fn capture_original_on(db: &Connection, root: &str) -> Result<(Value, Value), String> {
    read_on(db, root, false)
}

/// Approval-side read additionally checks the original absolute deadline.
/// This still issues no approval, lease, reservation or execution capability.
/// Test-only until the dedicated Single approval consumer is implemented.
#[cfg(test)]
pub(crate) fn remaining_for_new_work_on(db: &Connection, root: &str) -> Result<Value, String> {
    read_on(db, root, true).map(|(_, remaining)| remaining)
}

fn read_on(
    db: &Connection,
    root: &str,
    require_live_deadline: bool,
) -> Result<(Value, Value), String> {
    if db.is_autocommit() {
        return Err("continuation_financial_snapshot_required".into());
    }
    let owner = RootOwner::load_single(db, root)?;
    let exit = owner.read_single_exit(db)?;
    if exit.unsettled_ms != 0 || exit.exhausted {
        return Err("recovery_history_single_interval_unsettled".into());
    }
    // Totals alone cannot establish paid-call provenance. Verify the original
    // dispatch, received invoice and physical target evidence before balances.
    let history = verify_settled_history_on(db, root)?;
    // The shared implementation rejects missing limits, unresolved reserves,
    // indeterminate cost and checked-subtraction overflow/over-limit values.
    // None retains its original unlimited meaning; it is never changed to zero.
    let amounts = accounting::settled(db, root, None, require_live_deadline)?;
    if owner.read_single_exit(db)? != exit {
        return Err("recovery_history_original_finance_changed".into());
    }
    let remaining = json!({
        "version": 1,
        "proofKind": "settled_original_single_remaining",
        "rootRunId": root,
        "ownerId": owner.id,
        "dimensions": DIMENSIONS,
        "remaining": amounts.remaining,
        "sharedRemainingTokens": amounts.tokens,
        "sharedRemainingRequests": amounts.requests,
        "originalDeadline": amounts.deadline,
        "executionAllowed": false
    });
    Ok((history, remaining))
}
