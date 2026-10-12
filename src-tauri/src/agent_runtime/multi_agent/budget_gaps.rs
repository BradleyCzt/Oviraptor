//! §5.4 Loop7: read-only gap report between the current single-row budget
//! summary and the Master-required append-only multi-dimensional ledger.
//!
//! Nothing here moves money. This is a snapshot of missing dimensions, active
//! assignment reservations, and tool invocations whose outcome is unknown.
//! The status projection surfaces these as diagnostics, never as admission gates.

use rusqlite::{Connection, OptionalExtension};

/// Dimensions the Master §5.4 BudgetVector requires. Seven dimensions have
/// live append writers; browser_actions / controlled_writes / upload_bytes
/// remain missing until a contract writer lands.
pub const REQUIRED_DIMENSIONS: &[&str] = &[
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

/// Append-only dimensions already written live: model `DIMENSIONS[..4]`,
/// broker `target_requests`, admission `wall_time_ms`, and lane
/// `concurrency_batches`. Browser/controlled-write/upload stay denied by
/// contract and remain reported as missing until a live writer exists.
pub const TRACKED_DIMENSIONS: &[&str] = &[
    "model_input_tokens",
    "model_cached_tokens",
    "model_output_tokens",
    "model_requests",
    "target_requests",
    "concurrency_batches",
    "wall_time_ms",
];

/// Re-export: must stay identical to `budget::diagnostics` denied coverage.
#[cfg(test)]
pub use super::budget::DENIED_BY_CONTRACT_DIMENSIONS;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetLedgerGaps {
    pub ledger_exists: bool,
    pub reserved_tokens: i64,
    pub spent_tokens: i64,
    pub reserved_requests: i64,
    pub spent_requests: i64,
    /// Sum of assignment reservations that have not been settled, including
    /// terminal assignments whose cleanup has not yet run.
    pub unsettled_assignment_tokens: i64,
    pub unsettled_assignment_requests: i64,
    /// Ledger reservation minus the sum of unsettled assignments; meaningful
    /// only when ledger_exists is true. Negative means under-reservation.
    pub reservation_token_delta: Option<i64>,
    pub reservation_request_delta: Option<i64>,
    /// Started but never finished: unknown tool outcome, not a cost estimate.
    pub indeterminate_invocations: i64,
    /// Web dispatches without a received/unsent receipt under the same binding.
    /// Includes uncertain transport; estimated received costs stay in the vector.
    pub unclosed_web_model_calls: i64,
    /// Invocations with an empty contract_key: unattributable to any owner.
    pub unkeyed_invocations: i64,
    pub coordinator_lease_found: bool,
    pub fencing_matches_coordinator: bool,
    pub missing_dimensions: Vec<String>,
    pub append_journal: Option<Vec<super::budget::diagnostics::DimensionDiagnostic>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supervision_request_capacity: Option<super::budget::supervision_capacity::SupervisionRequestDiagnostic>,
}

#[cfg(test)]
pub fn budget_ledger_gaps(
    connection: &Connection,
    root_run_id: &str,
) -> Result<BudgetLedgerGaps, String> {
    // One deferred read transaction prevents a concurrent settle from making
    // the ledger and assignment sums appear inconsistent across two SELECTs.
    let snapshot =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Deferred)
            .map_err(|error| format!("无法开始预算盘点快照：{error}"))?;
    let gaps = budget_ledger_gaps_in_snapshot(&snapshot, root_run_id)?;
    snapshot
        .commit()
        .map_err(|error| format!("无法结束预算盘点快照：{error}"))?;
    Ok(gaps)
}

/// The caller already owns a consistent read snapshot and has checked scan
/// visibility and attempt scope before passing a root id from that snapshot.
pub(crate) fn budget_ledger_gaps_in_snapshot(
    snapshot: &Connection,
    root_run_id: &str,
) -> Result<BudgetLedgerGaps, String> {
    let ledger: Option<(i64, i64, i64, i64, i64, String)> = snapshot
        .query_row(
            "SELECT reserved_tokens,spent_tokens,reserved_requests,spent_requests,lease_epoch,fencing_token \
             FROM agent_budget_ledger WHERE root_run_id=?1",
            [root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .optional()
        .map_err(|error| format!("无法读取预算账本：{error}"))?;
    let scope: (String, i64, String) = snapshot
        .query_row(
            "SELECT scan_id,attempt_number,target_url FROM agent_runs \
             WHERE id=?1 AND (root_run_id='' OR root_run_id=?1)",
            [root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| format!("无法定位 root 运行：{error}"))?;
    let (assignment_tokens, assignment_requests): (i64, i64) = snapshot
        .query_row(
            "SELECT COALESCE(SUM(reserved_tokens),0),COALESCE(SUM(reserved_requests),0) \
             FROM agent_assignments WHERE coordinator_run_id=?1 AND budget_settled_at='' \
             AND scheduling_model='legacy_role_pipeline'",
            [root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("无法盘点未结算 assignment：{error}"))?;
    let indeterminate: i64 = snapshot
        .query_row(
            "SELECT COUNT(*) FROM tool_invocations WHERE run_id IN \
             (SELECT id FROM agent_runs WHERE id=?1 OR root_run_id=?1) AND finished_at=''",
            [root_run_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法盘点未定调用：{error}"))?;
    let unkeyed: i64 = snapshot
        .query_row(
            "SELECT COUNT(*) FROM tool_invocations WHERE run_id IN \
             (SELECT id FROM agent_runs WHERE id=?1 OR root_run_id=?1) AND contract_key=''",
            [root_run_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法盘点无合同调用：{error}"))?;
    let lease: Option<(i64, String)> = snapshot
        .query_row(
            "SELECT lease_epoch,fencing_token FROM agent_coordinator_leases \
             WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4",
            rusqlite::params![scope.0, scope.1, scope.2, root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("无法读取 Coordinator 租约：{error}"))?;
    let missing = REQUIRED_DIMENSIONS
        .iter()
        .filter(|dimension| !TRACKED_DIMENSIONS.contains(dimension))
        .map(|dimension| dimension.to_string())
        .collect();
    let token_delta = ledger
        .as_ref()
        .map(|row| {
            row.0
                .checked_sub(assignment_tokens)
                .ok_or("预算 token 差额溢出")
        })
        .transpose()?;
    let request_delta = ledger
        .as_ref()
        .map(|row| {
            row.2
                .checked_sub(assignment_requests)
                .ok_or("预算 request 差额溢出")
        })
        .transpose()?;
    let append_journal = super::budget::diagnostics::read(snapshot, root_run_id)?;
    // Summary-only legacy roots have no append contract to interpret as v6.
    let supervision_request_capacity = if append_journal.is_some() {
        super::budget::supervision_capacity::diagnostic(snapshot, root_run_id)?
    } else {
        None
    };
    let gaps = BudgetLedgerGaps {
        ledger_exists: ledger.is_some(),
        reserved_tokens: ledger.as_ref().map(|ledger| ledger.0).unwrap_or(0),
        spent_tokens: ledger.as_ref().map(|ledger| ledger.1).unwrap_or(0),
        reserved_requests: ledger.as_ref().map(|ledger| ledger.2).unwrap_or(0),
        spent_requests: ledger.as_ref().map(|ledger| ledger.3).unwrap_or(0),
        unsettled_assignment_tokens: assignment_tokens,
        unsettled_assignment_requests: assignment_requests,
        reservation_token_delta: token_delta,
        reservation_request_delta: request_delta,
        indeterminate_invocations: indeterminate,
        unclosed_web_model_calls: super::budget::diagnostics::unclosed_web_calls(
            snapshot,
            root_run_id,
        )?,
        unkeyed_invocations: unkeyed,
        coordinator_lease_found: lease.is_some(),
        fencing_matches_coordinator: match (&ledger, &lease) {
            (Some(ledger), Some(lease)) => ledger.4 == lease.0 && ledger.5 == lease.1,
            _ => false,
        },
        missing_dimensions: missing,
        append_journal,
        supervision_request_capacity,
    };
    Ok(gaps)
}

#[cfg(test)]
#[path = "budget_gaps_tests.rs"]
mod tests;
