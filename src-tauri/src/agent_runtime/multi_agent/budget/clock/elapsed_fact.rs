//! Exceptional final elapsed observations, never an executable allowance.
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ElapsedFact {
    pub(crate) id: String,
    pub(crate) root: String,
    pub(crate) owner_kind: String,
    pub(crate) owner_id: String,
    pub(crate) assignment: String,
    pub(crate) scan: String,
    pub(crate) attempt: i64,
    pub(crate) target: String,
    pub(crate) epoch: i64,
    pub(crate) fence: String,
    pub(crate) origin: String,
    pub(crate) cutoff: String,
    pub(crate) hard: i64,
    pub(crate) elapsed: i64,
    pub(crate) journaled: i64,
    pub(crate) unsettled: i64,
    pub(crate) exhausted: bool,
    pub(crate) binding: String,
    pub(crate) wall_hash: String,
}

pub(crate) fn read(db: &Connection, root: &str) -> Result<Option<ElapsedFact>, String> {
    db.query_row("SELECT fact_id,root_run_id,owner_kind,owner_id,assignment_id,scan_id,attempt_number,
        target_key,coordinator_epoch,coordinator_fence,origin,cutoff,hard_limit_ms,elapsed_ms,journaled_ms,
        unsettled_ms,exhausted,binding_hash,wall_hash FROM agent_root_elapsed_facts WHERE root_run_id=?1",[root],|r|Ok(ElapsedFact {
        id:r.get(0)?,root:r.get(1)?,owner_kind:r.get(2)?,owner_id:r.get(3)?,assignment:r.get(4)?,scan:r.get(5)?,attempt:r.get(6)?,
        target:r.get(7)?,epoch:r.get(8)?,fence:r.get(9)?,origin:r.get(10)?,cutoff:r.get(11)?,hard:r.get(12)?,elapsed:r.get(13)?,
        journaled:r.get(14)?,unsettled:r.get(15)?,exhausted:r.get(16)?,binding:r.get(17)?,wall_hash:r.get(18)?,
    })).optional().map_err(|e|e.to_string())
}

mod owner;
mod writer;

pub(crate) fn record_if_exceptional(
    db: &Connection,
    c: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Option<ElapsedFact>, String> {
    writer::run(db, |tx| record_on(tx, c, true))
}

/// Pure financial proof of an already committed fact. Never creates a fact
/// inside the publication writer or changes any executable permission.
pub(crate) fn verify_existing(
    db: &Connection,
    c: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Option<ElapsedFact>, String> {
    let Some(saved) = read(db, &c.root_run_id)? else {
        return Ok(None);
    };
    let actual = record_on(db, c, false)?;
    if actual.as_ref() != Some(&saved) {
        return Err("budget_elapsed_fact_original_conflict".into());
    }
    Ok(actual)
}

fn record_on(
    db: &Connection,
    c: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    create_fact: bool,
) -> Result<Option<ElapsedFact>, String> {
    record_at(db, c, create_fact, None)
}

// Only the private original-financial writer supplies an observed cutoff.
pub(super) fn record_at(
    db: &Connection,
    c: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    create_fact: bool,
    observed_cutoff: Option<&str>,
) -> Result<Option<ElapsedFact>, String> {
    use rusqlite::params;
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_budget_limits WHERE root_run_id=?1",
            [&c.root_run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count == 0 {
        return Ok(None);
    }
    if count != 10 {
        return Err("budget_dimension_contract_incomplete".into());
    }
    let (status, finished): (String, String) = db
        .query_row(
            "SELECT status,finished_at FROM agent_runs
        WHERE id=?1 AND root_run_id=id AND scan_id=?2 AND attempt_number=?3 AND target_url=?4
        AND backend='native' AND role='coordinator' AND orchestration_policy='multi'
        AND assignment_id='' AND parent_run_id IS NULL",
            params![c.root_run_id, c.scan_id, c.attempt_number, c.target_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "budget_root_binding_conflict")?;
    if !matches!(
        status.as_str(),
        "prepared" | "running" | "paused" | "terminal"
    ) {
        return Err("budget_root_binding_conflict".into());
    }
    let existing = read(db, &c.root_run_id)?;
    let cutoff = if let Some(f) = &existing {
        f.cutoff.clone()
    } else if status == "terminal" {
        if finished.is_empty() {
            return Err("budget_history_requires_reconciliation".into());
        }
        finished.clone()
    } else {
        if !finished.is_empty() {
            return Err("budget_root_binding_conflict".into());
        }
        match observed_cutoff {
            Some(cutoff) => cutoff.to_string(),
            None => db.query_row(
                "SELECT strftime('%Y-%m-%d %H:%M:%f','now','localtime')",
                [], |r| r.get(0),
            ).map_err(|e| e.to_string())?,
        }
    };
    let (origin,current,elapsed,hard,not_future):(String,String,Option<i64>,i64,bool)=db.query_row("SELECT o.started_at,r.created_at,
        CAST((julianday(?2)-julianday(o.started_at))*86400000 AS INTEGER),l.hard_limit,
        COALESCE(julianday(?2)<=julianday('now','localtime'),0)
        FROM agent_budget_clock_origins o JOIN agent_runs r ON r.id=o.root_run_id
        JOIN agent_budget_limits l ON l.root_run_id=r.id AND l.dimension='wall_time_ms' WHERE r.id=?1",
        params![c.root_run_id,cutoff],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|"budget_clock_origin_missing")?;
    if origin != current {
        return Err("budget_clock_origin_conflict".into());
    }
    let elapsed = elapsed
        .filter(|v| *v >= 0 && not_future)
        .ok_or("budget_clock_origin_invalid")?;
    let live:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE root_run_id=?1 AND scan_id=?2
        AND attempt_number=?3 AND target_key=?4 AND lease_epoch=?5 AND fencing_token=?6
        AND CASE WHEN ?8 THEN julianday(lease_expires_at)>julianday(?7) ELSE lease_expires_at>datetime('now','localtime') END)",params![c.root_run_id,c.scan_id,c.attempt_number,c.target_key,c.lease_epoch,c.fencing_token,cutoff,observed_cutoff.is_some()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if existing.is_none() && live && elapsed < hard {
        return Ok(None);
    }
    let root_control: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1)",
            [&c.root_run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    // Legacy workers need separate immutable-original-attempt evidence.
    // Do not fill a missing Root control or fabricate a financial sender.
    if !root_control {
        return Ok(None);
    }
    // This producer proves the real original hard contract; a manually altered
    // scalar is not evidence of original elapsed exceeding its original hard.
    super::super::limits::verify_root_contract(db, &c.root_run_id)?;
    let original = owner::original(db, c)?;
    let binding = owner::binding(db, c, &original)?;
    let balance = super::super::balance(db, &c.root_run_id, None, "wall_time_ms")?;
    if balance.reserved != 0 || balance.indeterminate != 0 || balance.consumed > elapsed {
        return Err("budget_clock_unsettled_sample".into());
    }
    if status == "terminal" && finished != cutoff {
        return Err("budget_elapsed_fact_cutoff_conflict".into());
    }
    let wall_hash = super::final_receipt::wall(db, &c.root_run_id)?;
    if let Some(saved) = existing {
        if uuid::Uuid::parse_str(&saved.id).is_err()
            || saved.root != c.root_run_id
            || saved.scan != c.scan_id
            || saved.attempt != c.attempt_number
            || saved.target != c.target_key
            || saved.epoch != c.lease_epoch
            || saved.fence != c.fencing_token
            || (
                saved.owner_kind.clone(),
                saved.owner_id.clone(),
                saved.assignment.clone(),
            ) != original
            || saved.origin != origin
            || saved.hard != hard
            || saved.elapsed != elapsed
            || saved.journaled != balance.consumed
            || saved.unsettled != elapsed - balance.consumed
            || saved.exhausted != (elapsed >= hard)
            || saved.binding != binding
            || saved.wall_hash != wall_hash
        {
            return Err("budget_elapsed_fact_original_conflict".into());
        }
        return Ok(Some(saved));
    }
    if !create_fact {
        return Err("budget_elapsed_fact_original_conflict".into());
    }
    let fact = ElapsedFact {
        id: uuid::Uuid::new_v4().to_string(),
        root: c.root_run_id.clone(),
        owner_kind: original.0,
        owner_id: original.1,
        assignment: original.2,
        scan: c.scan_id.clone(),
        attempt: c.attempt_number,
        target: c.target_key.clone(),
        epoch: c.lease_epoch,
        fence: c.fencing_token.clone(),
        origin,
        cutoff,
        hard,
        elapsed,
        journaled: balance.consumed,
        unsettled: elapsed - balance.consumed,
        exhausted: elapsed >= hard,
        binding,
        wall_hash,
    };
    let changed=db.execute("INSERT INTO agent_root_elapsed_facts(fact_id,root_run_id,owner_kind,owner_id,assignment_id,
        scan_id,attempt_number,target_key,coordinator_epoch,coordinator_fence,origin,cutoff,hard_limit_ms,elapsed_ms,journaled_ms,
        unsettled_ms,exhausted,binding_hash,wall_hash) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
        params![fact.id,fact.root,fact.owner_kind,fact.owner_id,fact.assignment,fact.scan,fact.attempt,fact.target,fact.epoch,
            fact.fence,fact.origin,fact.cutoff,fact.hard,fact.elapsed,fact.journaled,fact.unsettled,fact.exhausted,fact.binding,fact.wall_hash])
        .map_err(|e|e.to_string())?;
    if changed != 1
        || read(db, &c.root_run_id)?.as_ref() != Some(&fact)
        || owner::binding(db, c, &owner::original(db, c)?)? != fact.binding
        || super::final_receipt::wall(db, &c.root_run_id)? != fact.wall_hash
    {
        return Err("budget_elapsed_fact_persistence_conflict".into());
    }
    super::super::limits::verify_root_contract(db, &c.root_run_id)?;
    Ok(Some(fact))
}
