//! Exact original journal balances; no aggregate alone proves a paid call.
use super::super::{admission, balance, clock, root, DIMENSIONS};
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::{params, Connection};

pub(super) struct Amounts {
    pub remaining: [Option<i64>; 10],
    pub tokens: Option<i64>,
    pub requests: Option<i64>,
    pub deadline: String,
}

pub(super) fn settled(
    db: &Connection,
    root: &str,
    actor: Option<&CoordinatorLease>,
    require_live_deadline: bool,
) -> Result<Amounts, String> {
    // These native actions have no production physical-receipt consumer yet.
    // Balanced reserve/release rows alone cannot certify original action history.
    // Execution lane occupancy is separate from race-batch accounting.
    let unsupported: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.agent_budget_entries WHERE root_run_id=?1
         AND dimension IN ('browser_actions','controlled_writes','upload_bytes','concurrency_batches'))",
        [root], |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    if unsupported {
        return Err("continuation_unsupported_original_action_proof".into());
    }
    admission::require_financially_determinate(db, root)?;
    let elapsed = match actor {
        Some(actor) => clock::elapsed_fact::verify_existing(db, actor)?,
        None => {
            if clock::elapsed_fact::read(db, root)?.is_some() {
                return Err("continuation_original_elapsed_scope_invalid".into());
            }
            None
        }
    };
    if let Some(fact) = elapsed {
        if fact.unsettled != 0 || fact.exhausted {
            return Err("continuation_elapsed_cost_unsettled".into());
        }
    }
    let mut remaining = [None; 10];
    for (index, dimension) in DIMENSIONS.iter().enumerate() {
        let b = balance(db, root, None, dimension)?;
        if b.reserved != 0 || b.indeterminate != 0 {
            return Err("continuation_original_cost_unsettled".into());
        }
        let cap: Option<i64> = db
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![root, dimension],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        remaining[index] = cap
            .map(|limit| {
                limit
                    .checked_sub(b.consumed)
                    .filter(|v| *v >= 0)
                    .ok_or_else(|| "budget_hard_limit_exceeded".to_string())
            })
            .transpose()?;
    }
    let (tokens, requests) = root::remaining_child_capacity(db, root)?;
    // The wall journal balance above is stable financial data. This real-time
    // check separately limits executable time to the original absolute clock.
    if require_live_deadline {
        clock::remaining(db, root)?;
    }
    let deadline: String = db.query_row(
        "SELECT strftime('%Y-%m-%d %H:%M:%f',o.started_at,printf('+%f seconds',l.hard_limit/1000.0))
         FROM agent_budget_clock_origins o JOIN agent_budget_limits l ON l.root_run_id=o.root_run_id
         AND l.dimension='wall_time_ms' WHERE o.root_run_id=?1",
        [root],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    Ok(Amounts {
        remaining,
        tokens,
        requests,
        deadline,
    })
}
