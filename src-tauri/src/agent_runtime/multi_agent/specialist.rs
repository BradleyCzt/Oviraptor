//! At-most-once tool-free specialist model dispatch and atomic response receipts.
//! This records provider results, not completed work or permission to execute
//! suggestions. An executing/uncertain call is never automatically retried.
use super::{
    lease::{require_executable_coordinator, validate_coordinator_lease, CoordinatorLease},
    scheduler::ScheduledChild,
};
use crate::agent_runtime::{
    contract::AgentRole,
    secrets::{redact_json, redact_text_with},
    store::{self, UsageDelta},
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;

mod dispatch;
mod lifetime;
pub(crate) mod mapper_loop;
mod readonly_closure;
mod homogeneous_closure;
pub(crate) use lifetime::{require_idle_for_root, IdleSpecialists};
mod original_received;
pub(crate) use original_received::received_original_for_output_on;
mod closed_audit;
pub(crate) use closed_audit::verify_received_original;
#[cfg(test)]
pub use dispatch::start;
#[cfg(test)]
pub use dispatch::start_authorized;
pub(crate) use dispatch::start_for_transport;
mod receipt;
mod phase_writer;
mod owned_proposal;
mod owned_unsent;
pub(crate) use owned_unsent::{not_sent_for_owned_proposal, UnsentCall};
pub(crate) use owned_proposal::received_for_owned_proposal;
#[cfg(test)]
pub use receipt::record_received;
pub use receipt::record_received_with_provenance;
use receipt::verify_received;

#[derive(Debug)]
pub struct PendingCall {
    lease: CoordinatorLease,
    child: ScheduledChild,
    request_hash: String,
}

#[derive(Debug)]
pub struct StoredResponse {
    pub text: String,
    pub usage: UsageDelta,
    pub rejection: String,
}

#[derive(Debug)]
pub enum Start {
    Dispatch(PendingCall),
    Received(StoredResponse),
}

struct CallRow {
    state: String,
    failure_code: String,
    response: Value,
    usage: Value,
    response_hash: String,
    sequence: i64,
}

struct RawCall {
    child: String,
    root: String,
    role: String,
    request: String,
    hash: String,
    state: String,
    epoch: i64,
    fence: String,
    response: String,
    usage: String,
    sequence: i64,
    response_hash: String,
    failure_code: String,
}

fn binding(
    connection: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<(), String> {
    validate_coordinator_lease(connection, lease)?;
    super::source::validate_surface_role(connection, lease, child.role)?;
    let lane = match child.role.specialist_readonly_binding_lane() {
        Some(lane) => lane,
        None => return Err("specialist_role_not_readonly".into()),
    };
    let bound: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         WHERE a.id=?1 AND a.child_run_id=?2 AND r.assignment_id=a.id \
         AND a.coordinator_run_id=?3 AND r.root_run_id=?3 AND a.target_key=?4 AND r.target_url=?4 \
         AND r.scan_id=?5 AND r.attempt_number=?6 AND a.lease_epoch=?7 AND a.fencing_token=?8 \
         AND a.role=?9 AND r.role=?9 AND a.lane=?10 AND r.lane=?10)",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.target_key,lease.scan_id,
            lease.attempt_number,lease.lease_epoch,lease.fencing_token,child.role.as_str(),lane], |r| r.get(0),
    ).map_err(|e| format!("specialist_binding:{e}"))?;
    if !bound {
        return Err("specialist_binding_invalid".into());
    }
    if super::source::is_source_role(child.role) {
        super::source::verify_assignment(connection, lease, child)?;
    }
    if child.role == AgentRole::ClientSide {
        super::client_side::assignment(connection, lease, child)?;
    }
    if child.role == AgentRole::EvidenceReviewer
        && super::source::uses_source_runtime(connection, lease, child.role)?
    {
        super::source_review_subject::verify_assignment(connection, lease, child)?;
    }
    Ok(())
}

fn load(connection: &Connection, call: &PendingCall) -> Result<Option<CallRow>, String> {
    let raw = connection.query_row(
        "SELECT child_run_id,root_run_id,role,request_json,request_hash,state,lease_epoch,fencing_token,response_json,usage_json,event_sequence,response_hash,failure_code \
         FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2", params![call.child.assignment_id,call.child.run_id],
        |r| Ok(RawCall {child:r.get(0)?,root:r.get(1)?,role:r.get(2)?,request:r.get(3)?,hash:r.get(4)?,
            state:r.get(5)?,epoch:r.get(6)?,fence:r.get(7)?,response:r.get(8)?,usage:r.get(9)?,sequence:r.get(10)?,response_hash:r.get(11)?,failure_code:r.get(12)?}),
    ).optional().map_err(|e| format!("specialist_call_load:{e}"))?;
    let Some(RawCall {
        child,
        root,
        role,
        request,
        hash,
        state,
        epoch,
        fence,
        response,
        usage,
        sequence,
        response_hash,
        failure_code,
    }) = raw
    else {
        return Ok(None);
    };
    if child != call.child.run_id
        || root != call.lease.root_run_id
        || role != call.child.role.as_str()
        || hash != call.request_hash
        || hash != store::stable_hash(&request)
        || epoch != call.lease.lease_epoch
        || fence != call.lease.fencing_token
    {
        return Err("specialist_call_binding_or_request_changed".into());
    }
    Ok(Some(CallRow {
        state,
        failure_code,
        response: serde_json::from_str(&response)
            .map_err(|_| "specialist_response_json_invalid")?,
        usage: serde_json::from_str(&usage).map_err(|_| "specialist_usage_json_invalid")?,
        response_hash,
        sequence,
    }))
}

/// Local-only receipt verification under the caller's write transaction. This
/// never creates a dispatch, renews a lease or restores a child's capabilities.
/// Unlike `start`, no reconstructed model request is needed for bookkeeping.
pub(crate) fn received_for_reconciliation(
    tx: &rusqlite::Transaction<'_>,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<StoredResponse, String> {
    received_for_local_delivery(tx, lease, child)
}

pub(crate) fn received_for_local_delivery(
    db: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<StoredResponse, String> {
    if db.is_autocommit() {
        return Err("specialist_local_delivery_transaction_required".into());
    }
    binding(db, lease, child)?;
    require_executable_coordinator(db, lease)?;
    let request_hash: String = db.query_row(
        "SELECT request_hash FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND state='received'",
        params![child.assignment_id,child.run_id], |r| r.get(0),
    ).map_err(|_| "specialist_received_receipt_required")?;
    let call = PendingCall {
        lease: lease.clone(),
        child: child.clone(),
        request_hash,
    };
    let row = load(db, &call)?.ok_or("specialist_received_receipt_required")?;
    verify_received(db, &call, &row)
}

/// Receipt-only source closure audit. The caller verifies the completed
/// assignment/child and frozen task contract in the same transaction. Unlike
/// reconciliation, this also works after the root terminal write so its
/// postconditions can be checked without resurrecting execution authority.
pub(crate) fn source_received_for_completion(
    connection: &Connection,
    lease: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<StoredResponse, String> {
    if connection.is_autocommit()
        || !(super::source::is_source_role(child.role)
            || (child.role == AgentRole::EvidenceReviewer
                && super::source::uses_source_runtime(connection, lease, child.role)?))
    {
        return Err("source_completion_receipt_context_invalid".into());
    }
    let request_hash = connection
        .query_row(
            "SELECT request_hash FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",
            params![child.assignment_id,child.run_id],
            |r| r.get(0),
        )
        .map_err(|_| "source_completion_assessment_receipt_missing")?;
    let call = PendingCall {
        lease: lease.clone(),
        child: child.clone(),
        request_hash,
    };
    let row = load(connection, &call)?.ok_or("source_completion_assessment_receipt_missing")?;
    verify_received(connection, &call, &row)
}

/// Historical, read-only audit of an already completed specialist. This does
/// not validate/renew a live lease and MUST NOT be used to authorize dispatch.
pub(crate) fn received_for_audit(
    connection: &Connection,
    assignment: &str,
) -> Result<StoredResponse, String> {
    let (lease, child, request_hash) = connection.query_row(
        "SELECT r.scan_id,r.attempt_number,r.target_url,r.root_run_id,c.lease_epoch,c.fencing_token, \
         r.id,c.request_hash FROM agent_specialist_calls c JOIN agent_assignments a ON a.id=c.assignment_id \
         JOIN agent_runs r ON r.id=c.child_run_id AND r.assignment_id=a.id \
         JOIN agent_runs root ON root.id=c.root_run_id AND root.role='coordinator' \
         WHERE c.assignment_id=?1 AND c.state='received' AND c.role='evidence_reviewer' \
         AND a.coordinator_run_id=root.id AND a.child_run_id=r.id AND r.root_run_id=root.id \
         AND r.scan_id=root.scan_id AND r.attempt_number=root.attempt_number AND r.target_url=root.target_url \
         AND a.target_key=r.target_url AND a.role=c.role AND r.role=c.role AND a.lane='review' AND r.lane='review' \
         AND a.lease_epoch=c.lease_epoch AND a.fencing_token=c.fencing_token \
         AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 \
         AND r.status='terminal' AND r.terminal_state='completed' \
         AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id) \
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at='')",
        [assignment], |r| Ok((CoordinatorLease { scan_id:r.get(0)?,attempt_number:r.get(1)?,
            target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:String::new() },
            ScheduledChild {assignment_id:assignment.into(),run_id:r.get(6)?,role:AgentRole::EvidenceReviewer},r.get(7)?)),
    ).map_err(|_| "specialist_completed_receipt_required")?;
    let call = PendingCall {
        lease,
        child,
        request_hash,
    };
    let row = load(connection, &call)?.ok_or("specialist_completed_receipt_required")?;
    verify_received(connection, &call, &row)
}

pub fn record_uncertain(
    connection: &Connection,
    call: &PendingCall,
    code: &str,
) -> Result<(), String> {
    match phase_writer::run(connection, |db| record_uncertain_on(db, call, code)) {
        Ok(()) => Ok(()),
        Err(primary) => match receipt::record_failed_uncertain(connection, call) {
            Ok(()) => Err(primary),
            Err(persist) => Err(format!("{primary};specialist_cost_after_uncertain_failure:{persist}")),
        },
    }
}

fn record_uncertain_on(
    connection: &Connection,
    call: &PendingCall,
    code: &str,
) -> Result<(), String> {
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if super::attempts::require_unexpired_response_worker(&tx, &call.lease, &call.child).is_err() {
        super::budget::model_facts::specialist(
            &tx,
            &call.lease,
            &call.child,
            &call.request_hash,
            super::budget::model_facts::Fact::Uncertain,
        )?;
        return tx.commit().map_err(|e| e.to_string());
    }
    binding(&tx, &call.lease, &call.child)?;
    if load(&tx, call)?.is_none_or(|row| row.state != "executing" || !row.failure_code.is_empty()) {
        return Err("specialist_uncertain_state_conflict".into());
    }
    let changed = tx.execute("UPDATE agent_specialist_calls SET state='uncertain',failure_code=?1,finished_at=datetime('now','localtime') WHERE assignment_id=?2 AND child_run_id=?3 AND state='executing' AND failure_code=''",
        params![redact_text_with(code,None),call.child.assignment_id,call.child.run_id]).map_err(|e|format!("specialist_uncertain_persist:{e}"))?;
    if changed != 1 || load(&tx, call)?.is_none_or(|row| row.state != "uncertain") {
        return Err("specialist_uncertain_persist_missing".into());
    }
    super::budget::model::forfeit_call(
        &tx,
        &call.lease,
        &call.child.assignment_id,
        &format!(
            "specialist:{}:{}",
            call.child.assignment_id, call.request_hash
        ),
        None,
    )?;
    tx.commit().map_err(|e| e.to_string())
}

/// This is called only for the gateway's typed before-transport outcome. The
/// original claim remains as an immutable-by-contract tombstone: the same
/// assignment must not silently issue a second model request. A revoked lease
/// may still record this local no-send fact, but never settle its own budget.
pub fn record_not_sent(
    connection: &Connection,
    call: &PendingCall,
    code: &str,
) -> Result<(), String> {
    if code != "user_cancelled" {
        return Err("specialist_no_send_reason_invalid".into());
    }
    phase_writer::run(connection, |db| record_not_sent_on(db, call))
}

fn record_not_sent_on(connection: &Connection, call: &PendingCall) -> Result<(), String> {
    let tx = rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if super::attempts::require_unexpired_response_worker(&tx, &call.lease, &call.child).is_err() {
        super::budget::model_facts::specialist(
            &tx,
            &call.lease,
            &call.child,
            &call.request_hash,
            super::budget::model_facts::Fact::Unsent,
        )?;
        return tx.commit().map_err(|e| e.to_string());
    }
    if load(&tx, call)?.is_none_or(|row| {
        row.state != "executing"
            || !row.failure_code.is_empty()
            || row.sequence != 0
            || !row.response_hash.is_empty()
    }) {
        return Err("specialist_no_send_state_conflict".into());
    }
    let changed = tx.execute(
        "UPDATE agent_specialist_calls SET failure_code='model_cancelled_before_transport',finished_at=datetime('now','localtime') \
         WHERE assignment_id=?1 AND child_run_id=?2 AND state='executing' AND failure_code='' AND event_sequence=0 AND response_hash=''",
        params![call.child.assignment_id,call.child.run_id],
    ).map_err(|e| format!("specialist_no_send_persist:{e}"))?;
    if changed != 1
        || load(&tx, call)?.is_none_or(|row| row.failure_code != "model_cancelled_before_transport")
    {
        return Err("specialist_no_send_persist_missing".into());
    }
    tx.commit()
        .map_err(|e| format!("specialist_no_send_commit:{e}"))
}
