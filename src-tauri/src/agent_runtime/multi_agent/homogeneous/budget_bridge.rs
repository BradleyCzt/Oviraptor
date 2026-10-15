//! Keep homogeneous coarse grants in the original attempt-scoped model ledger.
//! Phase 1 carriers only read the store; a nonzero or unknown model invoice is
//! never converted from the aggregate WorkerReport into invented token classes.
use super::decision::{DecisionError, DecisionPayload};
use super::{reason, store, types};
use crate::agent_runtime::multi_agent::{budget, lease::CoordinatorLease};
use crate::agent_runtime::store::UsageDelta;
use rusqlite::{params, Transaction};

fn error(detail: String) -> DecisionError {
    DecisionError::new(reason::BUDGET_EXCEEDED, detail)
}

fn configured(tx: &Transaction<'_>, root: &str) -> Result<bool, DecisionError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?1)",
        [root],
        |row| row.get(0),
    )
    .map_err(|cause| error(cause.to_string()))
}

/// Admission precedes the coarse write, so the original shared ceiling includes
/// already-paid Root calls and the current original child grants.
pub(crate) fn admit(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    payload: &DecisionPayload,
) -> Result<(), DecisionError> {
    if !configured(tx, &lease.root_run_id)? {
        return Ok(());
    }
    if let DecisionPayload::Assign(body) = payload {
        if super::model_worker::enabled(tx, &lease.scan_id, lease.attempt_number).map_err(error)? {
            let task = store::load_task_by_root_run(tx,&lease.root_run_id)?.ok_or_else(||error("homogeneous_task_missing".into()))?;
            let slice = store::load_slice(tx,&body.slice_id)?.ok_or_else(||error("homogeneous_slice_missing".into()))?;
            let _ = task;
            let minimum=crate::scanning::homogeneous_execution::slice_minimum(tx,lease,&slice).map_err(error)?;
            if body.reserved_requests<minimum.1 || body.reserved_tokens<minimum.0 {
                return Err(error("homogeneous_worker_model_reservation_insufficient".into()));
            }
        }
        budget::root::require_child_capacity(
            tx,
            &lease.root_run_id,
            body.reserved_tokens,
            body.reserved_requests,
        )
        .map_err(error)?;
    }
    Ok(())
}

/// Called inside the same transaction as the coarse assignment/verdict write.
pub(crate) fn apply(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    payload: &DecisionPayload,
) -> Result<(), DecisionError> {
    if !configured(tx, &lease.root_run_id)? {
        return Ok(());
    }
    match payload {
        DecisionPayload::Assign(body) => {
            let slice = store::load_slice(tx, &body.slice_id)?.ok_or_else(|| {
                DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "预算切片不可寻")
            })?;
            let dedup = store::dedup_key_for(
                &slice.slice_id,
                &lease.target_key,
                slice.evidence_revision_base,
                store::HOMOGENEOUS_TRIGGER_CODE,
            );
            let id = store::assignment_id_for(&lease.root_run_id, &dedup);
            let assignment = store::load_assignment(tx, &id)?.ok_or_else(|| {
                DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "原预算 assignment 不可寻")
            })?;
            if assignment.attempt != body.attempt || assignment.state != "assigned" {
                return Err(error("budget_assignment_scope_conflict".into()));
            }
            if super::model_worker::enabled(tx,&lease.scan_id,lease.attempt_number).map_err(error)? {
                tx.execute("UPDATE agent_assignments SET finished_at='',budget_settled_at='' WHERE id=?1 AND state='assigned'",[&id]).map_err(|e|error(e.to_string()))?;
            }
            budget::model::reserve(
                tx,
                lease,
                &id,
                assignment.reserved_tokens,
                assignment.reserved_requests,
            )
            .map_err(error)?;
            budget::root::require_child_capacity(tx, &lease.root_run_id, 0, 0).map_err(error)
        }
        DecisionPayload::Accept(body) => settle_read_only(tx, lease, &body.assignment_id),
        DecisionPayload::RejectRetry(body) => settle_read_only(tx, lease, &body.assignment_id),
        _ => Ok(()),
    }
}

fn settle_read_only(
    tx: &Transaction<'_>,
    lease: &CoordinatorLease,
    assignment_id: &str,
) -> Result<(), DecisionError> {
    let assignment = store::load_assignment(tx, assignment_id)?.ok_or_else(|| {
        DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "原预算 assignment 不可寻")
    })?;
    let task=store::load_task_by_root_run(tx,&lease.root_run_id)?.ok_or_else(||error("homogeneous_task_missing".into()))?;
    let execution=super::execution_policy::read_original_on(tx,&lease.root_run_id).map_err(error)?.is_some();
    let tools = if execution {super::gate::tool_allowlist_for_task(tx,&task,&assignment.lane)?} else {super::gate::tool_allowlist_for_lane("read_only_analysis")};
    if (!execution && assignment.lane != "read_only_analysis")
        || assignment.coordinator_run_id != lease.root_run_id
        || assignment
            .capability_lease
            .iter()
            .any(|tool| !tools.contains(tool))
    {
        return Err(error("budget_homogeneous_carrier_not_read_only".into()));
    }
    let zero: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
             WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.scheduling_model='homogeneous_workers'
               AND r.backend='native' AND r.role='deep_investigator' AND r.lane='read_only_analysis'
               AND r.used_tokens=0 AND r.used_cached_tokens=0 AND r.used_requests=0
               AND NOT EXISTS(SELECT 1 FROM agent_worker_reports p
                 WHERE p.assignment_id=a.id
                   AND (p.consumed_tokens<>0 OR p.consumed_requests<>0))
               AND NOT EXISTS(SELECT 1 FROM agent_model_cost_facts f WHERE f.child_run_id=r.id)
               AND NOT EXISTS(SELECT 1 FROM agent_specialist_calls s WHERE s.child_run_id=r.id)
               AND NOT EXISTS(SELECT 1 FROM agent_source_model_rounds m WHERE m.child_run_id=r.id)
               AND NOT EXISTS(SELECT 1 FROM agent_web_model_journal w WHERE w.child_run_id=r.id))",
            params![assignment_id, lease.root_run_id],
            |row| row.get(0),
        )
        .map_err(|cause| error(cause.to_string()))?;
    if !zero && assignment.lane!="target_touching" {
        if !super::model_worker::enabled(tx,&lease.scan_id,lease.attempt_number).map_err(error)? {
            return Err(error("budget_homogeneous_usage_requires_original_receipt".into()));
        }
        let run: String = tx.query_row("SELECT child_run_id FROM agent_assignments WHERE id=?1",[assignment_id],|r|r.get(0)).map_err(|e|error(e.to_string()))?;
        let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id:assignment_id.into(),run_id:run,role:if assignment.lane=="review"{crate::agent_runtime::contract::AgentRole::EvidenceReviewer}else{crate::agent_runtime::contract::AgentRole::DeepInvestigator}};
        let original = crate::agent_runtime::multi_agent::specialist::received_for_local_delivery(tx,lease,&child).map_err(error)?;
        if original.rejection=="model_usage_requires_reconciliation" { return Err(error(original.rejection)); }
        let report = store::load_report(tx,&store::report_id_for(assignment_id,assignment.attempt))?
            .ok_or_else(||error("homogeneous_worker_report_missing".into()))?;
        if report.consumed_tokens!=original.usage.total_tokens || report.consumed_requests!=original.usage.model_requests {
            return Err(error("homogeneous_worker_receipt_usage_mismatch".into()));
        }
        budget::model::settle(tx,lease,assignment_id,&original.usage).map_err(error)?;
        tx.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1 AND revoked_at=''",[&child.run_id]).map_err(|e|error(e.to_string()))?;
        tx.execute("UPDATE agent_assignments SET finished_at=datetime('now','localtime'),budget_settled_at=datetime('now','localtime') WHERE id=?1 AND state IN ('accepted','rejected_retry')",[assignment_id]).map_err(|e|error(e.to_string()))?;
        return budget::root::require_child_capacity(tx,&lease.root_run_id,0,0).map_err(error);
    }
    // This is an unused original grant, not a fabricated original model bill.
    budget::model::settle(tx, lease, assignment_id, &UsageDelta::default()).map_err(error)?;
    if execution {
        tx.execute("UPDATE agent_assignments SET finished_at=datetime('now','localtime'),budget_settled_at=datetime('now','localtime') WHERE id=?1 AND state IN ('accepted','rejected_retry')",[assignment_id]).map_err(|e|error(e.to_string()))?;
    }
    budget::root::require_child_capacity(tx, &lease.root_run_id, 0, 0).map_err(error)
}

/// Expiry shares the transaction that cancels the carrier and releases coarse
/// reservations. The existing original lease/attempt validators remain intact.
pub(crate) fn reclaim(
    tx: &Transaction<'_>,
    task: &types::HomogeneousTask,
    assignment: &types::HomogeneousAssignment,
) -> Result<(), DecisionError> {
    if !configured(tx, &task.root_run_id)? {
        return Ok(());
    }
    let lease = tx
        .query_row(
            "SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
             FROM agent_coordinator_leases WHERE root_run_id=?1",
            [&task.root_run_id],
            |row| {
                Ok(CoordinatorLease {
                    scan_id: row.get(0)?,
                    attempt_number: row.get(1)?,
                    target_key: row.get(2)?,
                    root_run_id: row.get(3)?,
                    lease_epoch: row.get(4)?,
                    fencing_token: row.get(5)?,
                    lease_expires_at: row.get(6)?,
                })
            },
        )
        .map_err(|cause| error(cause.to_string()))?;
    settle_read_only(tx, &lease, &assignment.assignment_id)
}
