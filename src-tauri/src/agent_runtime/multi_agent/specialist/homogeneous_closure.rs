//! Same held SDK exit proof, with homogeneous verdict states and original attempt
//! receipts. In particular, a retry cannot substitute its new child for the old bill.
use super::*;
use crate::agent_runtime::multi_agent::budget;
pub(super) fn close(tx: &rusqlite::Transaction<'_>, call: &PendingCall) -> Result<(), String> {
    let (actor, child) = (&call.lease, &call.child);
    budget::root::RootOwner::load_original(tx, &actor.root_run_id)?
        .require_original_coordinator(tx, actor)?;
    let original =
        budget::receipts::original_owner(tx, &child.run_id, actor, &child.assignment_id)?;
    original.verify(tx)?;
    let closed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_assignment_attempts x ON x.child_run_id=r.id JOIN agent_runs root ON root.id=x.root_run_id JOIN agent_assignments a ON a.id=x.assignment_id JOIN agent_worker_reports p ON p.assignment_id=x.assignment_id JOIN agent_acceptance_verdicts v ON v.assignment_id=p.assignment_id AND v.attempt=x.lease_epoch AND v.slice_id=p.slice_id
      WHERE r.id=?1 AND a.id=?2 AND x.root_run_id=?3 AND root.status='terminal'
        AND r.status='terminal' AND r.finished_at<>'' AND x.state='completed' AND x.finished_at<>''
        AND ((r.role='deep_investigator' AND r.lane='read_only_analysis') OR (r.role='evidence_reviewer' AND r.lane='review' AND json_extract(root.plan_json,'$.homogeneousExecution.workerExecution')='model_broker_v1')) AND a.scheduling_model='homogeneous_workers'
        AND p.report_id=?4 AND v.verdict IN ('accept','retry')
        AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at='')
        AND NOT EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=r.id))",
        params![child.run_id,child.assignment_id,actor.root_run_id,
            crate::agent_runtime::multi_agent::homogeneous::store::report_id_for(&child.assignment_id,
                tx.query_row("SELECT lease_epoch FROM agent_assignment_attempts WHERE id=?1",[original.attempt_id()],|r|r.get::<_,i64>(0)).map_err(|e|e.to_string())?)],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !closed {
        let ended:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND status='terminal' AND terminal_state<>'completed')",[&actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !ended {
            return Err("homogeneous_original_worker_unsettled".into());
        }
        // The parent already holds the original SDK exit probe. Stop local
        // business work, preserving every paid/unknown fee and reservation.
        tx.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1 AND revoked_at=''",[&child.run_id]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE agent_assignment_attempts SET state='cancelled',failure_class='specialist_task_ended_reconciliation_required',finished_at=datetime('now','localtime') WHERE child_run_id=?1 AND state IN ('leased','running','paused')",[&child.run_id]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE agent_runs SET status='terminal',terminal_state='cancelled',terminal_reason='specialist_task_ended_reconciliation_required',finished_at=datetime('now','localtime') WHERE id=?1 AND status<>'terminal'",[&child.run_id]).map_err(|e|e.to_string())?;
        if let Some(a) = crate::agent_runtime::multi_agent::homogeneous::store::load_assignment(
            tx,
            &child.assignment_id,
        )
        .map_err(|e| e.detail)?
        {
            let current: String = tx
                .query_row(
                    "SELECT child_run_id FROM agent_assignments WHERE id=?1",
                    [&child.assignment_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if current == child.run_id
                && matches!(a.state.as_str(), "assigned" | "running" | "reported")
            {
                use crate::agent_runtime::multi_agent::homogeneous::store;
                store::set_assignment_state(tx, &a.assignment_id, "cancelled")
                    .map_err(|e| e.detail)?;
            }
        }
        return Ok(());
    }
    if !original.is_settled(tx)? {
        return Err("homogeneous_original_worker_unsettled".into());
    }
    let row = load(tx, call)?.ok_or("homogeneous_original_call_missing")?;
    if row.response["usageReported"] != true {
        return Err("homogeneous_original_fee_unresolved".into());
    }
    let response = receipt::verify_response(tx, call, &row)?;
    budget::receipts::verify_original_received(
        tx,
        &original,
        &format!("specialist:{}:{}", child.assignment_id, call.request_hash),
        &row.response_hash,
        &response.usage,
    )?;
    Ok(())
}
