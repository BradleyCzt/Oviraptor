//! First-worker Web allocation proof; historical inventory confers no live authority.
use crate::agent_runtime::multi_agent::{attempts, lease::CoordinatorLease};
use rusqlite::{params, Connection};
pub(crate) struct OriginalWebGrantInventory {
    tokens: i64,
    requests: i64,
    worker_id: String,
}
impl OriginalWebGrantInventory {
    pub(crate) fn tokens(&self) -> i64 {
        self.tokens
    }
    pub(crate) fn requests(&self) -> i64 {
        self.requests
    }
    pub(crate) fn worker_attempt_id(&self) -> &str {
        &self.worker_id
    }
}
pub(crate) fn original_web_grant(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<(i64, i64), String> {
    let grant = read_original_inventory(db, lease, assignment)?;
    let projection:(i64,i64,String,String)=db.query_row("SELECT reserved_tokens,reserved_requests,state,budget_settled_at FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND role='web_executor'",params![assignment,lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"executor_allocation_original_grant_conflict")?;
    if projection.0 != grant.tokens
        || projection.1 != grant.requests
        || !matches!(
            projection.2.as_str(),
            "leased" | "running" | "waiting_review"
        )
        || !projection.3.is_empty()
    {
        return Err("executor_allocation_original_grant_conflict".into());
    }
    Ok((grant.tokens, grant.requests))
}
pub(crate) fn original_web_grant_inventory_on(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<OriginalWebGrantInventory, String> {
    if db.is_autocommit() {
        return Err("executor_allocation_history_snapshot_required".into());
    }
    let original = crate::scanning::scope_contract::original_root_capture_mode(db, &lease.root_run_id)?;
    crate::agent_runtime::web_mode::root::verify_birth(db, &lease.root_run_id, &original)?;
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs c ON c.id=a.child_run_id JOIN agent_runs r ON r.id=a.coordinator_run_id WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.role='web_executor' AND a.lane='target_touching' AND a.target_key=?3 AND a.lease_epoch=?4 AND a.fencing_token=?5 AND c.assignment_id=a.id AND c.root_run_id=r.id AND c.parent_run_id=r.id AND c.role=a.role AND c.lane=a.lane AND c.backend='native' AND c.orchestration_policy='multi' AND c.scan_id=?6 AND c.attempt_number=?7 AND c.target_url=?3 AND r.role='coordinator' AND r.backend='native' AND r.orchestration_policy='multi' AND r.scan_id=?6 AND r.attempt_number=?7 AND r.target_url=?3)",params![assignment,lease.root_run_id,lease.target_key,lease.lease_epoch,lease.fencing_token,lease.scan_id,lease.attempt_number],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact {
        return Err("executor_allocation_original_scope_conflict".into());
    }
    let grant = read_original_inventory(db, lease, assignment)?;
    let source = format!("assignment:{assignment}");
    let foreign:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE (assignment_id=?1 OR lease_attempt_id=?2) AND kind='reserve' AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') AND (root_run_id<>?3 OR assignment_id<>?1 OR lease_attempt_id<>?2 OR source_id<>?4))",params![assignment,grant.worker_id,lease.root_run_id,source],|r|r.get(0)).map_err(|e|e.to_string())?;
    if foreign {
        return Err("executor_allocation_original_invoice_scope_conflict".into());
    }
    Ok(grant)
}
fn read_original_inventory(
    db: &Connection,
    lease: &CoordinatorLease,
    assignment: &str,
) -> Result<OriginalWebGrantInventory, String> {
    super::mapper_allocation::require_original_invoice_guards(db)?;
    super::super::root::RootOwner::load_original(db, &lease.root_run_id)?
        .require_original_coordinator(db, lease)?;
    let worker = attempts::current(db, lease, assignment)?;
    if worker.lease_epoch != 1 {
        return Err("executor_allocation_original_worker_conflict".into());
    }
    let hard: (i64, i64) = db
        .query_row(
            "SELECT hard_token_budget,hard_request_budget FROM agent_runs WHERE id=?1",
            [&lease.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let source = format!("assignment:{assignment}");
    let mut q=db.prepare("SELECT dimension,amount,idempotency_key FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND kind='reserve' AND source_id=?4 AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') ORDER BY dimension").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(
            params![lease.root_run_id, assignment, worker.id, source],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let amount = |dimension: &str, limit: i64| -> Result<i64, String> {
        let found = rows.iter().find(|r| r.0 == dimension);
        match (limit, found) {
            (0, None) => Ok(0),
            (n, Some(row)) if n > 0 && row.1 > 0 && row.1 <= n => Ok(row.1),
            _ => Err("executor_allocation_original_grant_conflict".into()),
        }
    };
    let tokens = amount("model_input_tokens", hard.0)?;
    let requests = amount("model_requests", hard.1)?;
    let count = usize::from(tokens > 0) * 3 + usize::from(requests > 0);
    if rows.len() != count
        || rows.iter().any(|(dimension, n, key)| {
            *n != if dimension == "model_requests" {
                requests
            } else {
                tokens
            } || key != &format!("reserve:{assignment}:{dimension}")
        })
    {
        return Err("executor_allocation_original_grant_conflict".into());
    }
    Ok(OriginalWebGrantInventory {
        tokens,
        requests,
        worker_id: worker.id,
    })
}
