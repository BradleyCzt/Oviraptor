//! One elapsed clock per root, shared by all children. Each admitted boundary
//! appends only the delta since the previous sample; overlapping work cannot
//! multiply elapsed time. A restart or child retry cannot reset the origin.
use super::{append_inner, balance, Authority, Kind};
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::{params, OptionalExtension, Transaction};

/// Read the original shared deadline for transport. This grants no authority,
/// changes no clock origin, and never grants a new full timeout to a child.
pub(crate) fn remaining(
    db: &rusqlite::Connection,
    root: &str,
) -> Result<std::time::Duration, String> {
    let (origin,current,elapsed,limit):(String,String,Option<i64>,i64)=db.query_row(
        "SELECT o.started_at,r.created_at,CAST((julianday('now','localtime')-julianday(o.started_at))*86400000 AS INTEGER),l.hard_limit
        FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id
        JOIN agent_budget_limits l ON l.root_run_id=r.id AND l.dimension='wall_time_ms' WHERE r.id=?1",
        [root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"budget_clock_origin_missing")?;
    if origin != current {
        return Err("budget_clock_origin_conflict".into());
    }
    let previous = balance(db, root, None, "wall_time_ms")?;
    if previous.reserved != 0 || previous.indeterminate != 0 {
        return Err("budget_clock_unsettled_sample".into());
    }
    available(elapsed, limit, previous.consumed)
}

fn available(
    elapsed: Option<i64>,
    limit: i64,
    consumed: i64,
) -> Result<std::time::Duration, String> {
    let elapsed = elapsed
        .filter(|v| *v >= 0)
        .ok_or("budget_clock_origin_invalid")?;
    if elapsed < consumed {
        return Err("budget_clock_moved_backwards".into());
    }
    let remaining = limit
        .checked_sub(elapsed)
        .filter(|v| *v > 0)
        .ok_or("budget_wall_time_exhausted")?;
    Ok(std::time::Duration::from_millis(remaining as u64))
}

/// Before the first grant, observe the original declared Root clock without
/// creating limits/origin or backfilling any historical execution record.
pub(crate) fn supervision_remaining(
    db: &rusqlite::Connection,
    root: &str,
) -> Result<std::time::Duration, String> {
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count == 10 {
        return remaining(db, root);
    }
    if count != 0 {
        return Err("budget_dimension_contract_incomplete".into());
    }
    let pristine: bool = db.query_row("SELECT
        EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND used_tokens=0 AND used_cached_tokens=0 AND used_requests=0
          AND reserved_tokens=0 AND reserved_requests=0)
        AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE root_run_id=?1 AND id<>?1)
        AND NOT EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_budget_clock_origins WHERE root_run_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_budget_ledger WHERE root_run_id=?1
          AND (spent_tokens<>0 OR spent_requests<>0 OR reserved_tokens<>0 OR reserved_requests<>0))",
        [root], |r|r.get(0)).map_err(|e|e.to_string())?;
    if !pristine {
        return Err("budget_history_requires_reconciliation".into());
    }
    let (text, elapsed): (String, Option<i64>) = db
        .query_row(
            "SELECT plan_json,
        CAST((julianday('now','localtime')-julianday(created_at))*86400000 AS INTEGER)
        FROM agent_runs WHERE id=?1 AND backend='native' AND role='coordinator'",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let plan = serde_json::from_str(&text).map_err(|_| "budget_frozen_plan_invalid")?;
    available(elapsed, super::limits::frozen_wall_time(&plan)?, 0)
}

/// An absolute original deadline avoids rounding a fresh TTL beyond the Root.
pub(crate) fn renewal_expiry(db: &rusqlite::Connection, root: &str) -> Result<String, String> {
    supervision_remaining(db, root)?;
    let (origin, text, frozen): (String, String, Option<i64>) = db.query_row("SELECT r.created_at,r.plan_json,l.hard_limit
        FROM agent_runs r LEFT JOIN agent_budget_limits l ON l.root_run_id=r.id AND l.dimension='wall_time_ms'
        WHERE r.id=?1", [root], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    let limit = match frozen {
        Some(limit) => limit,
        None => super::limits::frozen_wall_time(
            &serde_json::from_str(&text).map_err(|_| "budget_frozen_plan_invalid")?,
        )?,
    };
    db.query_row("SELECT min(datetime('now','+600 seconds','localtime'),datetime(?1,printf('+%f seconds',?2/1000.0)))",
        params![origin,limit], |r|r.get(0)).map_err(|e|format!("budget_renewal_deadline:{e}"))
}

mod closure_events;
mod closure_writer;
mod final_receipt;
mod finalization;
mod pause_authority;
pub(crate) use closure_writer::{closure_write, financial_assessment_write, source_pause_write};
pub(crate) use finalization::FinalClock;

// Preserve the existing unique closed-worker financial contract test. All
// production finalization uses the captured cutoff and terminal transaction.
#[cfg(test)]
pub(crate) fn finish_sample(tx: &Transaction<'_>, lease: &CoordinatorLease) -> Result<(), String> {
    FinalClock::capture(tx, lease)?.sample(tx)
}

pub(crate) fn freeze(tx: &Transaction<'_>, root: &str) -> Result<(), String> {
    let origin: String = tx
        .query_row(
            "SELECT created_at FROM agent_runs WHERE id=?1 AND backend='native'
        AND role='coordinator' AND julianday(created_at)<=julianday('now','localtime')",
            [root],
            |r| r.get(0),
        )
        .map_err(|_| "budget_clock_origin_invalid")?;
    let inserted = tx
        .execute(
            "INSERT INTO agent_budget_clock_origins(root_run_id,started_at) VALUES(?1,?2)",
            params![root, origin],
        )
        .map_err(|e| e.to_string())?;
    let actual: Option<String> = tx
        .query_row(
            "SELECT started_at FROM agent_budget_clock_origins WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if inserted != 1 || actual.as_deref() != Some(&origin) {
        return Err("budget_clock_origin_persistence_conflict".into());
    }
    Ok(())
}

pub(crate) fn sample(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<(), String> {
    let (origin,current,elapsed):(String,String,Option<i64>)=tx.query_row(
        "SELECT o.started_at,r.created_at,CAST((julianday('now','localtime')-julianday(o.started_at))*86400000 AS INTEGER)
        FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id WHERE o.root_run_id=?1",
        [&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"budget_clock_origin_missing")?;
    if origin != current {
        return Err("budget_clock_origin_conflict".into());
    }
    let elapsed = elapsed
        .filter(|v| *v >= 0)
        .ok_or("budget_clock_origin_invalid")?;
    let previous = balance(tx, &lease.root_run_id, None, "wall_time_ms")?;
    if previous.reserved != 0 || previous.indeterminate != 0 {
        return Err("budget_clock_unsettled_sample".into());
    }
    let delta = elapsed
        .checked_sub(previous.consumed)
        .filter(|v| *v >= 0)
        .ok_or("budget_clock_moved_backwards")?;
    let limit:i64=tx.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='wall_time_ms'",
        [&lease.root_run_id],|r|r.get(0)).map_err(|_|"budget_clock_limit_missing")?;
    if elapsed >= limit {
        return Err("budget_wall_time_exhausted".into());
    }
    if delta == 0 {
        return Ok(());
    }
    let source = format!(
        "clock:{}:{elapsed}",
        crate::agent_runtime::store::stable_hash(&origin)
    );
    // Elapsed root time is an already incurred charge, even after its last
    // child closes. This private path never admits work or extends a grant.
    append_inner(
        tx,
        lease,
        assignment,
        "wall_time_ms",
        Kind::Reserve,
        delta,
        &format!("{source}:reserve"),
        &source,
        Authority::ClockSample,
    )?;
    append_inner(
        tx,
        lease,
        assignment,
        "wall_time_ms",
        Kind::Consume,
        delta,
        &format!("{source}:consume"),
        &source,
        Authority::ClockSample,
    )?;
    let unchanged:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_budget_clock_origins o ON o.root_run_id=r.id
        WHERE r.id=?1 AND r.created_at=?2 AND o.started_at=?2)",params![lease.root_run_id,origin],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !unchanged {
        return Err("budget_clock_origin_conflict".into());
    }
    Ok(())
}

pub(crate) mod elapsed_fact;

pub(crate) mod observation;

pub(crate) mod original_worker_samples;
