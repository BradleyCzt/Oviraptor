//! Pure historical shared-clock accounting. Never samples or renews a lease.
use crate::agent_runtime::{
    multi_agent::{budget, lease::CoordinatorLease, scheduler::ScheduledChild},
    store,
};
use rusqlite::{params, Connection};

#[derive(Clone)]
struct Entry {
    rowid: i64,
    assignment: String,
    worker: String,
    kind: String,
    amount: i64,
    key: String,
    source: String,
}

pub(crate) fn original_worker_consumed_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
) -> Result<i64, String> {
    if db.is_autocommit() {
        return Err("budget_original_clock_snapshot_required".into());
    }
    budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    let bound:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts x JOIN agent_assignments a ON a.id=x.assignment_id JOIN agent_runs r ON r.id=x.child_run_id WHERE x.id=?1 AND x.root_run_id=?2 AND x.assignment_id=?3 AND x.child_run_id=?4 AND x.coordinator_epoch=?5 AND x.coordinator_fencing_token=?6 AND a.coordinator_run_id=x.root_run_id AND a.child_run_id=r.id AND a.role=?7 AND r.role=a.role AND r.root_run_id=x.root_run_id AND r.scan_id=?8 AND r.attempt_number=?9 AND r.target_url=?10)",params![worker,actor.root_run_id,child.assignment_id,child.run_id,actor.lease_epoch,actor.fencing_token,child.role.as_str(),actor.scan_id,actor.attempt_number,actor.target_key],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound {
        return Err("budget_original_clock_worker_scope_changed".into());
    }
    let (origin,limit):(String,i64)=db.query_row("SELECT o.started_at,l.hard_limit FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id AND r.created_at=o.started_at JOIN agent_budget_limits l ON l.root_run_id=r.id AND l.dimension='wall_time_ms' WHERE r.id=?1",[&actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"budget_original_clock_origin_changed")?;
    let foreign:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE dimension='wall_time_ms' AND lease_attempt_id=?1 AND (root_run_id<>?2 OR assignment_id<>?3))",params![worker,actor.root_run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if foreign {
        return Err("budget_original_clock_foreign_source".into());
    }
    let floor:i64=db.query_row("SELECT COALESCE(MAX(rowid),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='wall_time_ms'",params![actor.root_run_id,child.assignment_id,worker],|r|r.get(0)).map_err(|e|e.to_string())?;
    if floor == 0 {
        return Ok(0);
    }
    // Only predecessors needed to prove this worker's deltas. Subsequent paid
    // Root samples must never change the child's stable financial fact.
    let mut q=db.prepare("SELECT rowid,assignment_id,lease_attempt_id,kind,amount,idempotency_key,source_id FROM agent_budget_entries WHERE root_run_id=?1 AND dimension='wall_time_ms' AND rowid<=?2 ORDER BY rowid").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![actor.root_run_id, floor], |r| {
            Ok(Entry {
                rowid: r.get(0)?,
                assignment: r.get(1)?,
                worker: r.get(2)?,
                kind: r.get(3)?,
                amount: r.get(4)?,
                key: r.get(5)?,
                source: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let (pairs, remainder) = rows.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err("budget_original_clock_unclosed_source".into());
    }
    let origin_hash = store::stable_hash(&origin);
    let mut global = 0i64;
    let mut own = 0i64;
    for pair in pairs {
        let a = &pair[0];
        let b = &pair[1];
        if a.rowid >= b.rowid
            || a.assignment != b.assignment
            || a.worker != b.worker
            || a.source != b.source
            || a.kind != "reserve"
            || b.kind != "consume"
            || a.amount <= 0
            || a.amount != b.amount
        {
            return Err("budget_original_clock_pair_changed".into());
        }
        let elapsed = if a.assignment.is_empty() {
            let prefix = "root-clock:";
            let suffix = format!(":{origin_hash}");
            a.source
                .strip_prefix(prefix)
                .and_then(|s| s.strip_suffix(&suffix))
        } else {
            let prefix = format!("clock:{origin_hash}:");
            a.source.strip_prefix(&prefix)
        }
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or("budget_original_clock_source_changed")?;
        if a.source
            != if a.assignment.is_empty() {
                format!("root-clock:{elapsed}:{origin_hash}")
            } else {
                format!("clock:{origin_hash}:{elapsed}")
            }
            || elapsed <= global
            || elapsed < 0
            || (!a.assignment.is_empty() && elapsed >= limit)
            || elapsed.checked_sub(global) != Some(a.amount)
        {
            return Err("budget_original_clock_delta_changed".into());
        }
        if !a.assignment.is_empty() {
            let scope:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts x JOIN agent_assignments a ON a.id=x.assignment_id JOIN agent_runs r ON r.id=x.child_run_id WHERE x.id=?1 AND x.assignment_id=?2 AND x.root_run_id=?3 AND x.coordinator_epoch=?4 AND x.coordinator_fencing_token=?5 AND a.coordinator_run_id=x.root_run_id AND a.child_run_id=r.id AND r.root_run_id=x.root_run_id AND r.scan_id=?6 AND r.attempt_number=?7 AND r.target_url=?8)",params![a.worker,a.assignment,actor.root_run_id,actor.lease_epoch,actor.fencing_token,actor.scan_id,actor.attempt_number,actor.target_key],|r|r.get(0)).map_err(|e|e.to_string())?;
            if !scope {
                return Err("budget_original_clock_predecessor_scope_changed".into());
            }
        }
        let reserve = budget::scope::stored_key(
            db,
            &actor.root_run_id,
            &a.assignment,
            &a.worker,
            &format!("{}:reserve", a.source),
        )?;
        let consume = budget::scope::stored_key(
            db,
            &actor.root_run_id,
            &a.assignment,
            &a.worker,
            &format!("{}:consume", a.source),
        )?;
        if a.key != reserve || b.key != consume {
            return Err("budget_original_clock_namespace_changed".into());
        }
        global = elapsed;
        if a.assignment == child.assignment_id && a.worker == worker {
            own = own.checked_add(a.amount).ok_or("budget_amount_overflow")?;
        }
    }
    Ok(own)
}
