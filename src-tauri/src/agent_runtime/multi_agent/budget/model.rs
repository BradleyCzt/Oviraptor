//! Model accounting at existing scheduler and receipt settlement boundaries.
//! Token reservations cover each possible token class until receipt breakdown
//! is known. The existing aggregate cap additionally bounds their shared sum.
use super::{append, scope, BudgetVector, Kind, DIMENSIONS};
use crate::agent_runtime::{multi_agent::lease::CoordinatorLease, store::UsageDelta};
use rusqlite::{params, Transaction};

pub(crate) fn reserve(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    tokens: i64,
    requests: i64,
) -> Result<(), String> {
    super::continuation::migration::require_plain_root_boundary(tx, &lease.root_run_id)?;
    let vector = BudgetVector {
        model_input_tokens: tokens,
        model_cached_tokens: tokens,
        model_output_tokens: tokens,
        model_requests: requests,
        ..BudgetVector::default()
    };
    for (dimension, amount) in DIMENSIONS[..4].iter().zip([
        vector.model_input_tokens,
        vector.model_cached_tokens,
        vector.model_output_tokens,
        vector.model_requests,
    ]) {
        if amount > 0 {
            append(
                tx,
                lease,
                assignment,
                dimension,
                Kind::Reserve,
                amount,
                &format!("reserve:{assignment}:{dimension}"),
                &format!("assignment:{assignment}"),
            )?;
        }
    }
    Ok(())
}

type BudgetWrite = fn(
    &Transaction<'_>,
    &CoordinatorLease,
    &str,
    &str,
    Kind,
    i64,
    &str,
    &str,
) -> Result<(), String>;

pub(crate) fn settle(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    usage: &UsageDelta,
) -> Result<(), String> {
    settle_with(tx, lease, assignment, usage, append, super::Authority::Live)
}

include!("model/original_web_receipt.rs");

pub(crate) mod expired_web_receipt;

pub(crate) fn settle_terminal_receipt(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    usage: &UsageDelta,
) -> Result<(), String> {
    settle_with(
        tx,
        lease,
        assignment,
        usage,
        super::append_terminal_receipt,
        super::Authority::TerminalReceipt,
    )
}

fn settle_with(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    usage: &UsageDelta,
    write: BudgetWrite,
    authority: super::Authority,
) -> Result<(), String> {
    super::validate_budget_owner(tx, lease, authority)?;
    super::validate_budget_assignment(tx, lease, assignment, Kind::Consume)?;
    let worker = super::super::attempts::current(tx, lease, assignment)?;
    let unresolved:bool=tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM agent_web_model_journal d
        WHERE d.root_run_id=?1 AND d.assignment_id=?2 AND d.child_run_id=?3 AND d.phase='dispatch'
        AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal t WHERE t.call_id=d.call_id AND t.phase IN ('received','unsent') AND {}))",super::WEB_RECEIPT_BINDING),
        params![lease.root_run_id,assignment,worker.child_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if unresolved {
        return Err("budget_indeterminate_requires_reconciliation".into());
    }
    let source = format!(
        "child:{assignment}:usage:{}",
        crate::agent_runtime::store::stable_hash(&usage.as_json().to_string())
    );
    let prefix = scope::stored_key(
        tx,
        &lease.root_run_id,
        assignment,
        &worker.id,
        &format!("settle:{assignment}:"),
    )?;
    let conflict:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND substr(idempotency_key,1,length(?3))=?3 AND source_id<>?4 AND lease_attempt_id=?5)",
        params![lease.root_run_id,assignment,prefix,source,worker.id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if conflict {
        return Err("budget_entry_replay_conflict".into());
    }
    let values = [
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.model_requests,
    ];
    if values.iter().any(|n| *n < 0)
        || usage.cached_input_tokens > usage.input_tokens
        || usage.total_tokens < 0
    {
        return Err("budget_usage_invalid".into());
    }
    let complete = usage.input_tokens.checked_add(usage.output_tokens) == Some(usage.total_tokens);
    let mut statement=tx.prepare("SELECT dimension,kind,amount,idempotency_key FROM agent_budget_entries
        WHERE root_run_id=?1 AND assignment_id=?2 AND substr(idempotency_key,1,length(?3))=?3 AND lease_attempt_id=?4 ORDER BY rowid").map_err(|e|e.to_string())?;
    let previous = statement
        .query_map(
            params![lease.root_run_id, assignment, prefix, worker.id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if !previous.is_empty() {
        for (dimension, amount) in DIMENSIONS[..4].iter().zip(values) {
            let own = scope::current_balance(tx, lease, assignment, dimension)?;
            let exact = if dimension == &"model_requests" || complete {
                own.consumed == amount && own.indeterminate == 0
            } else {
                own.indeterminate > 0 && own.consumed <= amount
            };
            if own.reserved != 0 || !exact {
                return Err("budget_entry_replay_conflict".into());
            }
        }
        for (dimension, kind, amount, key) in previous {
            write(
                tx,
                lease,
                assignment,
                &dimension,
                Kind::parse(&kind)?,
                amount,
                &key,
                &source,
            )?;
        }
        return Ok(());
    }
    for (dimension, amount) in DIMENSIONS[..4].iter().zip(values) {
        let own = scope::current_balance(tx, lease, assignment, dimension)?;
        let mut reserved = own.reserved;
        let remaining = amount
            .checked_sub(own.consumed)
            .filter(|n| *n >= 0)
            .ok_or("budget_usage_below_recorded_cost")?;
        let incomplete = dimension != &"model_requests" && !complete;
        if !incomplete && own.indeterminate > 0 {
            return Err("budget_indeterminate_requires_reconciliation".into());
        }
        let held = reserved
            .checked_add(own.indeterminate)
            .and_then(|n| n.checked_add(own.consumed))
            .ok_or("budget_amount_overflow")?;
        let required = if incomplete {
            usage
                .total_tokens
                .checked_sub(held)
                .unwrap_or(0)
                .max(0)
                .checked_add(reserved)
                .ok_or("budget_amount_overflow")?
        } else {
            remaining
        };
        if required > reserved {
            let limit:Option<i64>=tx.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![lease.root_run_id,dimension],|r|r.get(0)).map_err(|e|e.to_string())?;
            if limit.is_some() {
                return Err("budget_usage_exceeds_reservation".into());
            }
            write(
                tx,
                lease,
                assignment,
                dimension,
                Kind::Reserve,
                required - reserved,
                &format!("{prefix}{dimension}:unlimited"),
                &source,
            )?;
            reserved = required;
        }
        if incomplete {
            if reserved > 0 {
                write(
                    tx,
                    lease,
                    assignment,
                    dimension,
                    Kind::Forfeit,
                    reserved,
                    &format!("{prefix}{dimension}:unknown"),
                    &source,
                )?;
            }
            continue;
        }
        if remaining > reserved {
            return Err("budget_usage_exceeds_reservation".into());
        }
        if remaining > 0 {
            write(
                tx,
                lease,
                assignment,
                dimension,
                Kind::Consume,
                remaining,
                &format!("{prefix}{dimension}:consume"),
                &source,
            )?;
        }
        if reserved > remaining {
            write(
                tx,
                lease,
                assignment,
                dimension,
                Kind::Release,
                reserved - remaining,
                &format!("{prefix}{dimension}:release"),
                &source,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn release_unsent(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<(), String> {
    let worker = super::super::attempts::current(tx, lease, assignment)?;
    let mapper_journal_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE type='table' AND name='agent_mapper_loop_journal')",
        [], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    let mapper_dispatch = if mapper_journal_exists {
        tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.agent_mapper_loop_journal WHERE assignment_id=?1 AND child_run_id=?2 AND phase='dispatch')",
            params![assignment, worker.child_run_id], |row| row.get(0),
        ).map_err(|error| error.to_string())?
    } else { false };
    // An already settled zero-reservation worker may finish without a refund.
    // Any remaining Mapper reservation belongs to its dedicated closure.
    if mapper_dispatch {
        for dimension in &DIMENSIONS[..4] {
            if scope::current_balance(tx, lease, assignment, dimension)?.reserved > 0 {
                return Err("mapper_loop_original_reconciliation_required".into());
            }
        }
    }
    let sent:bool=tx.query_row(&format!("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND NOT(state='executing' AND failure_code='model_cancelled_before_transport' AND finished_at<>'' AND event_sequence=0 AND response_hash='')) OR EXISTS(SELECT 1 FROM agent_source_model_rounds c
          LEFT JOIN agent_assignment_attempts x ON x.child_run_id=c.child_run_id AND x.assignment_id=c.assignment_id AND x.root_run_id=c.root_run_id
          WHERE c.assignment_id=?1 AND c.child_run_id=?2 AND NOT EXISTS(SELECT 1 FROM agent_model_cost_facts f
            WHERE {unsent}))
        OR EXISTS(SELECT 1 FROM agent_web_model_journal d WHERE d.assignment_id=?1 AND d.child_run_id=?2 AND d.phase='dispatch'
          AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal t WHERE t.call_id=d.call_id AND t.phase='unsent' AND {}))",super::WEB_RECEIPT_BINDING,unsent=super::model_facts::SOURCE_UNSENT_BINDING),
        params![assignment,worker.child_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    for dimension in &DIMENSIONS[..4] {
        let own = scope::current_balance(tx, lease, assignment, dimension)?;
        if own.indeterminate > 0 {
            return Err("budget_indeterminate_requires_reconciliation".into());
        }
        let reserved = own.reserved;
        if reserved > 0 {
            if sent {
                return Err("budget_indeterminate_requires_reconciliation".into());
            }
            append(
                tx,
                lease,
                assignment,
                dimension,
                Kind::Release,
                reserved,
                &format!("finish:{assignment}:{dimension}"),
                &format!("unsent:{assignment}"),
            )?;
        }
    }
    Ok(())
}

/// The caller verifies and marks the actual durable call uncertain in this
/// same transaction. A tool-free call holds the remaining estimate; source
/// rounds hold their per-round estimate. One model request was attempted.
pub(crate) fn forfeit_call(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
    source: &str,
    estimated_tokens: Option<i64>,
) -> Result<(), String> {
    for (index, dimension) in DIMENSIONS[..4].iter().enumerate() {
        let reserved = scope::current_balance(tx, lease, assignment, dimension)?.reserved;
        let maximum = if index == 3 {
            1
        } else {
            estimated_tokens.unwrap_or(reserved)
        };
        let amount = reserved.min(maximum);
        if amount > 0 {
            super::append_unknown_cost(tx, lease, assignment, dimension, amount, source)?;
        }
    }
    Ok(())
}

mod mapper_allocation;
pub(crate) use mapper_allocation::original_mapper_grant;
pub(crate) mod mapper_loop_contract;
pub(crate) use mapper_allocation::original_mapper_loop_grant_on;

mod web_allocation;
pub(crate) use web_allocation::{original_web_grant, original_web_grant_inventory_on};
