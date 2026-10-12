//! Stop only original tool-free assessments with an already held SDK exit proof.
//! Provider cost, dispatch, results and mailbox are immutable during this cleanup.
use super::*;
use crate::agent_runtime::multi_agent::{attempts, attempts::audit_rows::Rows, budget};
use rusqlite::types::Value as SqlValue;
const REASON: &str = "specialist_task_ended_reconciliation_required";

pub(super) fn pause(tx: &rusqlite::Transaction<'_>, call: &PendingCall) -> Result<(), String> {
    let (scope, child) = (&call.lease, &call.child);
    let homogeneous:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id WHERE r.id=?1 AND a.id=?2 AND r.scheduling_model='homogeneous_workers' AND a.scheduling_model='homogeneous_workers')",params![child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if homogeneous { return super::homogeneous_closure::close(tx,call); }
    binding(tx, scope, child)?;
    let terminal: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND status='terminal')",
            [&scope.root_run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !terminal
        || !matches!(
            child.role,
            AgentRole::SpaApiMapper
                | AgentRole::IdentitySession
                | AgentRole::DeepInvestigator
                | AgentRole::ClientSide
        )
    {
        return Err("specialist_readonly_closure_context_invalid".into());
    }
    budget::receipts::original_owner(tx, &child.run_id, scope, &child.assignment_id)?.verify(tx)?;
    let row = load(tx, call)?.ok_or("specialist_readonly_closure_call_missing")?;
    if !matches!(row.state.as_str(), "executing" | "received" | "uncertain") {
        return Err("specialist_readonly_closure_call_state_invalid".into());
    }
    let worker = attempts::current(tx, scope, &child.assignment_id)?;
    let completed: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.state IN ('completed','failed','cancelled') AND r.status='terminal'
        AND a.finished_at<>'' AND r.finished_at<>'' AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at='')
        AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id))", params![child.assignment_id,child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if completed
        && matches!(worker.state.as_str(), "completed" | "failed" | "cancelled")
        && !worker.finished_at.is_empty()
    {
        return Ok(());
    }
    let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3 AND r.parent_run_id=?3
        AND a.state IN ('running','paused') AND r.status IN ('running','paused') AND a.budget_settled_at=''
        AND a.finished_at='' AND r.finished_at='' AND r.terminal_state='' AND a.lane='read_only_analysis' AND r.lane=a.lane
        AND a.lease_epoch=?4 AND a.fencing_token=?5)
        AND NOT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE child_run_id=?2)",
        params![child.assignment_id,child.run_id,scope.root_run_id,scope.lease_epoch,scope.fencing_token], |r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound
        || !matches!(worker.state.as_str(), "running" | "paused")
        || !worker.finished_at.is_empty()
    {
        return Err("specialist_readonly_closure_original_worker_conflict".into());
    }
    let original = frozen(tx, call)?;
    let now: String = tx
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let mut expected = runtime(tx, call)?;
    expected[0].set(&child.assignment_id, "state", "paused")?;
    expected[0].set(&child.assignment_id, "failure_class", REASON)?;
    expected[0].set(&child.assignment_id, "updated_at", &now)?;
    expected[1].set(&child.run_id, "status", "paused")?;
    expected[1].set(&child.run_id, "updated_at", &now)?;
    expected[2].set(&worker.id, "state", "paused")?;
    expected[2].set(&worker.id, "failure_class", REASON)?;
    let cap_ids = expected[3]
        .values
        .iter()
        .map(|row| match &row[expected[3].column("id")?] {
            SqlValue::Text(id) => Ok(id.clone()),
            _ => Err("specialist_readonly_closure_capability_conflict".to_string()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    for id in cap_ids {
        if expected[3].text(&id, "root_run_id")? != scope.root_run_id
            || expected[3].text(&id, "assignment_id")? != child.assignment_id
            || expected[3].text(&id, "child_run_id")? != child.run_id
            || expected[3].text(&id, "fencing_token")? != scope.fencing_token
            || expected[3].row(&id)?[expected[3].column("lease_epoch")?]
                != SqlValue::Integer(scope.lease_epoch)
        {
            return Err("specialist_readonly_closure_capability_conflict".into());
        }
        if expected[3].text(&id, "revoked_at")?.is_empty() {
            expected[3].set(&id, "revoked_at", &now)?;
        }
    }
    let lanes: i64 = tx
        .query_row(
            "SELECT count(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let a = tx.execute("UPDATE agent_assignments SET state='paused',failure_class=?4,updated_at=?5 WHERE id=?1 AND child_run_id=?2
        AND coordinator_run_id=?3 AND state IN ('running','paused') AND budget_settled_at=''",
        params![child.assignment_id,child.run_id,scope.root_run_id,REASON,now]).map_err(|e|e.to_string())?;
    let r = tx.execute("UPDATE agent_runs SET status='paused',updated_at=?3 WHERE id=?1 AND root_run_id=?2 AND status IN ('running','paused')",
        params![child.run_id,scope.root_run_id,now]).map_err(|e|e.to_string())?;
    attempts::pause(tx, scope, child, REASON)?;
    tx.execute("UPDATE agent_capability_leases SET revoked_at=?4 WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND revoked_at=''",
        params![child.assignment_id,child.run_id,scope.root_run_id,now]).map_err(|e|e.to_string())?;
    let deleted = tx.execute("DELETE FROM agent_lane_leases WHERE assignment_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_key=?4 AND lane='read_only_analysis'",
        params![child.assignment_id,scope.scan_id,scope.attempt_number,scope.target_key]).map_err(|e|e.to_string())?;
    let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='')
        OR EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)", params![child.assignment_id,child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if a != 1
        || r != 1
        || i64::try_from(deleted).map_err(|e| e.to_string())? != lanes
        || active
        || runtime(tx, call)? != expected
        || frozen(tx, call)? != original
    {
        return Err("specialist_readonly_closure_postcondition_conflict".into());
    }
    binding(tx, scope, child)?;
    load(tx, call)?.ok_or("specialist_readonly_closure_call_missing")?;
    Ok(())
}
fn runtime(db: &Connection, call: &PendingCall) -> Result<Vec<Rows>, String> {
    let child = &call.child;
    Ok(vec![
        Rows::read(db,"SELECT rowid,* FROM agent_assignments WHERE id=?1",[&child.assignment_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_runs WHERE id=?1",[&child.run_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_assignment_attempts WHERE assignment_id=?1 ORDER BY rowid",[&child.assignment_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2 ORDER BY rowid",params![child.assignment_id,child.run_id])?,
    ])
}
fn frozen(db: &Connection, call: &PendingCall) -> Result<Vec<Rows>, String> {
    let root = &call.lease.root_run_id;
    Ok(vec![
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_budget_ledger WHERE root_run_id=?1",
            [root],
        )?,
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 ORDER BY rowid",
            [root],
        )?,
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_specialist_calls WHERE root_run_id=?1 ORDER BY rowid",
            [root],
        )?,
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_model_cost_facts WHERE root_run_id=?1 ORDER BY rowid",
            [root],
        )?,
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_messages WHERE root_run_id=?1 ORDER BY rowid",
            [root],
        )?,
    ])
}

/// Original multi-round exit proof, kept separate from the legacy single SDK
/// reader. This stops local work without settling, refunding or delivering it.
pub(super) fn pause_mapper_loop(
    tx: &rusqlite::Transaction<'_>,
    scope: &CoordinatorLease,
    child: &ScheduledChild,
    proof: &super::mapper_loop::VerifiedMapperIdle,
) -> Result<(), String> {
    proof.require_on(tx, scope, child)?;
    let terminal: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND status='terminal')",
            [&scope.root_run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !terminal || child.role != AgentRole::SpaApiMapper {
        return Err("mapper_loop_readonly_closure_context_invalid".into());
    }
    let worker = attempts::current(tx, scope, &child.assignment_id)?;
    let completed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.state IN ('completed','failed','cancelled') AND r.status='terminal'
          AND a.finished_at<>'' AND r.finished_at<>'' AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at='')
          AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id))",params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if completed
        && matches!(worker.state.as_str(), "completed" | "failed" | "cancelled")
        && !worker.finished_at.is_empty()
    {
        return Ok(());
    }
    let bound:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3 AND r.parent_run_id=?3
          AND a.state IN ('running','paused') AND r.status IN ('running','paused') AND a.budget_settled_at=''
          AND a.finished_at='' AND r.finished_at='' AND r.terminal_state='' AND a.lane='read_only_analysis' AND r.lane=a.lane
          AND a.lease_epoch=?4 AND a.fencing_token=?5)
        AND NOT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE child_run_id=?2)
        AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE child_run_id=?2)",params![child.assignment_id,child.run_id,scope.root_run_id,scope.lease_epoch,scope.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound
        || !matches!(worker.state.as_str(), "running" | "paused")
        || !worker.finished_at.is_empty()
    {
        return Err("mapper_loop_readonly_closure_worker_conflict".into());
    }
    let original = mapper_frozen(tx, scope)?;
    let mut expected = mapper_runtime(tx, child)?;
    let now: String = tx
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    expected[0].set(&child.assignment_id, "state", "paused")?;
    expected[0].set(&child.assignment_id, "failure_class", REASON)?;
    expected[0].set(&child.assignment_id, "updated_at", &now)?;
    expected[1].set(&child.run_id, "status", "paused")?;
    expected[1].set(&child.run_id, "updated_at", &now)?;
    expected[2].set(&worker.id, "state", "paused")?;
    expected[2].set(&worker.id, "failure_class", REASON)?;
    let ids = expected[3]
        .values
        .iter()
        .map(|row| match &row[expected[3].column("id")?] {
            SqlValue::Text(id) => Ok(id.clone()),
            _ => Err("mapper_loop_readonly_closure_capability_conflict".to_string()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    for id in ids {
        if expected[3].text(&id, "root_run_id")? != scope.root_run_id
            || expected[3].text(&id, "assignment_id")? != child.assignment_id
            || expected[3].text(&id, "child_run_id")? != child.run_id
            || expected[3].text(&id, "fencing_token")? != scope.fencing_token
            || expected[3].row(&id)?[expected[3].column("lease_epoch")?]
                != SqlValue::Integer(scope.lease_epoch)
        {
            return Err("mapper_loop_readonly_closure_capability_conflict".into());
        }
        if expected[3].text(&id, "revoked_at")?.is_empty() {
            expected[3].set(&id, "revoked_at", &now)?;
        }
    }
    let lanes: i64 = tx
        .query_row(
            "SELECT count(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let a=tx.execute("UPDATE agent_assignments SET state='paused',failure_class=?4,updated_at=?5 WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND state IN ('running','paused') AND budget_settled_at=''",params![child.assignment_id,child.run_id,scope.root_run_id,REASON,now]).map_err(|e|e.to_string())?;
    let r=tx.execute("UPDATE agent_runs SET status='paused',updated_at=?3 WHERE id=?1 AND root_run_id=?2 AND status IN ('running','paused')",params![child.run_id,scope.root_run_id,now]).map_err(|e|e.to_string())?;
    attempts::pause(tx, scope, child, REASON)?;
    tx.execute("UPDATE agent_capability_leases SET revoked_at=?4 WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND revoked_at=''",params![child.assignment_id,child.run_id,scope.root_run_id,now]).map_err(|e|e.to_string())?;
    let deleted=tx.execute("DELETE FROM agent_lane_leases WHERE assignment_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_key=?4 AND lane='read_only_analysis'",params![child.assignment_id,scope.scan_id,scope.attempt_number,scope.target_key]).map_err(|e|e.to_string())?;
    let active:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='') OR EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if a != 1
        || r != 1
        || i64::try_from(deleted).map_err(|e| e.to_string())? != lanes
        || active
        || mapper_runtime(tx, child)? != expected
        || mapper_frozen(tx, scope)? != original
    {
        return Err("mapper_loop_readonly_closure_postcondition_conflict".into());
    }
    proof.require_on(tx, scope, child)?;
    Ok(())
}

fn mapper_runtime(db: &Connection, child: &ScheduledChild) -> Result<Vec<Rows>, String> {
    Ok(vec![
        Rows::read(db,"SELECT rowid,* FROM agent_assignments WHERE id=?1",[&child.assignment_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_runs WHERE id=?1",[&child.run_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_assignment_attempts WHERE assignment_id=?1 ORDER BY rowid",[&child.assignment_id])?,
        Rows::read(db,"SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2 ORDER BY rowid",params![child.assignment_id,child.run_id])?,
    ])
}
fn mapper_frozen(db: &Connection, scope: &CoordinatorLease) -> Result<Vec<Rows>, String> {
    Ok(vec![
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_budget_ledger WHERE root_run_id=?1",
            [&scope.root_run_id],
        )?,
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 ORDER BY rowid",
            [&scope.root_run_id],
        )?,
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_mapper_loop_journal WHERE root_run_id=?1 ORDER BY rowid",
            [&scope.root_run_id],
        )?,
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_model_cost_facts WHERE root_run_id=?1 ORDER BY rowid",
            [&scope.root_run_id],
        )?,
        Rows::read(
            db,
            "SELECT rowid,* FROM agent_messages WHERE root_run_id=?1 ORDER BY rowid",
            [&scope.root_run_id],
        )?,
    ])
}
