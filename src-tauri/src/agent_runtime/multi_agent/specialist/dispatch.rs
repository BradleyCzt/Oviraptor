use super::*;
use crate::agent_runtime::multi_agent::{attempts, budget, source, source_review_subject};

/// The caller must use this exact frozen request when Dispatch is returned.
/// A received result is a local replay; it never regrants a revoked capability.
#[cfg(test)]
pub fn start(
    connection: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    request: &Value,
) -> Result<Start, String> {
    start_authorized(connection, lease, child, request, |_| Ok(()))
}

/// The commands layer can bind a published runtime under the SAME write lock
/// as the durable dispatch claim. Recheck after INSERT-trigger side effects.
#[cfg(test)]
pub fn start_authorized(
    connection: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
    request: &Value,
    authorize: impl Fn(&Connection) -> Result<(), String>,
) -> Result<Start, String> {
    start_inner(connection, lease, child, request, authorize, false).map(|(start, _)| start)
}

// Only the actual transport receives an execution owner; PendingCall remains financial.
pub(crate) fn start_for_transport(
    connection: &Connection, lease: &CoordinatorLease, child: &ScheduledChild,
    request: &Value, authorize: impl Fn(&Connection) -> Result<(), String>,
) -> Result<(Start, Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>), String> {
    start_inner(connection, lease, child, request, authorize, true)
}

fn start_inner(
    connection: &Connection, lease: &CoordinatorLease, child: &ScheduledChild,
    request: &Value, authorize: impl Fn(&Connection) -> Result<(), String>, transport: bool,
) -> Result<(Start, Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>), String> {
    let plaintext = super::super::homogeneous::material_policy::for_root(connection, &lease.root_run_id)?;
    let request_value = if plaintext { request.clone() } else { redact_json(request) };
    let request = request_value.to_string();
    if request.len() > 6_000_000 {
        return Err("specialist_request_too_large".into());
    }
    let call = PendingCall {
        lease: lease.clone(),
        child: child.clone(),
        request_hash: store::stable_hash(&request),
    };
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    binding(&tx, lease, child)?;
    require_executable_coordinator(&tx, lease)?;
    authorize(&tx)?;
    if plaintext != super::super::homogeneous::material_policy::for_root(&tx, &lease.root_run_id)? {
        return Err("homogeneous_material_policy_changed".into());
    }
    if source::is_source_role(child.role) {
        source::validate_request(&tx, lease, child, &request_value)?;
    }
    if child.role == AgentRole::EvidenceReviewer
        && source::uses_source_runtime(&tx, lease, child.role)?
    {
        source_review_subject::validate_request(&tx, lease, child, &request_value)?;
    }
    if let Some(row) = load(&tx, &call)? {
        return if row.state == "received" {
            let receipt = verify_received(&tx, &call, &row)?;
            let owner = if transport { Some(super::lifetime::transport_owner(&tx, &call, &request_value, true)?) } else { None };
            Ok((Start::Received(receipt), owner))
        } else if row.state == "executing" && row.failure_code == "model_cancelled_before_transport"
        {
            Err("specialist_model_not_sent_new_assignment_required".into())
        } else {
            Err("specialist_call_outcome_unknown_requires_reconciliation".into())
        };
    }
    attempts::require_live_for_run(&tx, &child.run_id)?;
    // Do not treat an older build's call as new merely because no journal exists.
    let dispatchable: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         WHERE a.id=?1 AND a.state='running' AND r.status='running' AND a.budget_settled_at='' AND a.reserved_requests>=1) \
         AND EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 \
           AND capability='evidence.read' AND lease_epoch=?4 AND fencing_token=?5 AND revoked_at='' AND lease_expires_at>datetime('now','localtime')) \
         AND NOT EXISTS(SELECT 1 FROM agent_events WHERE run_id=?2 AND event_type='model_round_completed') \
         AND NOT EXISTS(SELECT 1 FROM agent_snapshots WHERE run_id=?2)",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.lease_epoch,lease.fencing_token], |r|r.get(0),
    ).map_err(|e|format!("specialist_dispatch_guard:{e}"))?;
    if !dispatchable {
        return Err("specialist_dispatch_state_or_history_conflict".into());
    }
    budget::admission::require_determinate(&tx, &lease.root_run_id)?;
    let owner = if transport { Some(super::lifetime::transport_owner(&tx, &call, &request_value, false)?) } else { None };
    budget::clock::sample(&tx, lease, &child.assignment_id)?;
    let inserted = tx.execute(
        "INSERT INTO agent_specialist_calls(assignment_id,child_run_id,root_run_id,role,lease_epoch,fencing_token,request_json,request_hash,state) \
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'executing')",
        params![child.assignment_id,child.run_id,lease.root_run_id,child.role.as_str(),lease.lease_epoch,lease.fencing_token,request,call.request_hash],
    ).map_err(|e|format!("specialist_dispatch_persist:{e}"))?;
    if inserted != 1 || load(&tx, &call)?.is_none_or(|row| row.state != "executing") {
        return Err("specialist_dispatch_persist_missing".into());
    }
    authorize(&tx)?;
    if plaintext != super::super::homogeneous::material_policy::for_root(&tx, &lease.root_run_id)? {
        return Err("homogeneous_material_policy_changed".into());
    }
    attempts::require_live_for_run(&tx, &child.run_id)?;
    binding(&tx, lease, child)?;
    require_executable_coordinator(&tx, lease)?;
    budget::admission::require_determinate(&tx, &lease.root_run_id)?;
    if source::is_source_role(child.role) {
        source::validate_request(&tx, lease, child, &request_value)?;
    }
    if child.role == AgentRole::EvidenceReviewer
        && source::uses_source_runtime(&tx, lease, child.role)?
    {
        source_review_subject::validate_request(&tx, lease, child, &request_value)?;
    }
    tx.commit()
        .map_err(|e| format!("specialist_dispatch_commit:{e}"))?;
    Ok((Start::Dispatch(call), owner))
}
