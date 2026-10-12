// Target-touching work and its usage have already happened. This transaction
// publishes only the known result; it must never replay model or target I/O.
#[allow(clippy::too_many_arguments)]
fn complete_target_child_delivery(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    kind: &str,
    correlation_id: &str,
    payload: &JsonValue,
    success: bool,
    summary: &str,
) -> Result<Option<crate::agent_runtime::web_mode::capability_transition_receipt::CapabilityTransitionGuard>, String> {
    complete_target_child_delivery_on(connection, lease, child, TargetChildDeliveryMessage {
        kind, correlation_id, payload, success, summary,
    }, None)
}
struct TargetChildDeliveryMessage<'a> {
    kind: &'a str,
    correlation_id: &'a str,
    payload: &'a JsonValue,
    success: bool,
    summary: &'a str,
}
fn complete_target_child_delivery_on(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    delivery: TargetChildDeliveryMessage<'_>,
    authorization_target_dir: Option<&std::path::Path>,
) -> Result<Option<crate::agent_runtime::web_mode::capability_transition_receipt::CapabilityTransitionGuard>, String> {
    let TargetChildDeliveryMessage { kind, correlation_id, payload, success, summary } = delivery;
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{lease::{require_executable_coordinator, validate_coordinator_lease}, mailbox, scheduler},
        secrets::redact_json,
    };
    if !matches!((child.role,kind),
        (AgentRole::WebExecutor,"execution_result") | (AgentRole::Authorization,"authorization_result")) {
        return Err("target_child_delivery_role_mismatch".into());
    }
    let tx = rusqlite::Transaction::new_unchecked(connection,rusqlite::TransactionBehavior::Immediate)
        .map_err(|error|format!("target_child_delivery_lock:{error}"))?;
    validate_coordinator_lease(&tx,lease)?;
    require_executable_coordinator(&tx,lease)?;
    let revision: i64=tx.query_row(
        "SELECT evidence_revision FROM agent_assignments WHERE id=?1 AND child_run_id=?2 \
         AND coordinator_run_id=?3 AND target_key=?4 AND role=?5 AND state='running' \
         AND lease_epoch=?6 AND fencing_token=?7 AND budget_settled_at<>'' \
         AND reserved_tokens=0 AND reserved_requests=0",
        params![child.assignment_id,child.run_id,lease.root_run_id,lease.target_key,child.role.as_str(),lease.lease_epoch,lease.fencing_token],
        |row|row.get(0),
    ).map_err(|error|format!("target_child_delivery_binding_or_usage:{error}"))?;
    let event_floor: i64=tx.query_row("SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",[],|row|row.get(0))
        .map_err(|error|format!("target_child_delivery_event_cursor:{error}"))?;
    let capability_mode = if success && child.role == AgentRole::WebExecutor {
        let declaration = crate::agent_runtime::web_mode::root::read(&tx, &lease.root_run_id)?;
        if declaration.is_some_and(|mode| mode.bootstrap_dispatch().is_some_and(|b|
            b["capabilityTransitionRule"] == crate::agent_runtime::web_mode::capability_transition_receipt::RULE)) {
            Some(original_root_capture_mode(&tx, &lease.root_run_id)?)
        } else { None }
    } else { None };
    let authorization_guard = if success && child.role == AgentRole::Authorization
        && crate::agent_runtime::web_mode::root::read(&tx, &lease.root_run_id)?
            .is_some_and(|mode| mode.bootstrap_dispatch().is_some_and(|b|
                b["authorizationOutputRule"] == authorization_output_receipt::RULE)) {
        let mode = original_root_capture_mode(&tx, &lease.root_run_id)?;
        Some(authorization_output_receipt::capture_before_on(&tx, lease, child,
            authorization_target_dir.ok_or("authorization_output_original_directory_required")?, &mode)?)
    } else { None };
    let capability_guard = capability_mode.as_ref().map(|mode|
        crate::agent_runtime::web_mode::capability_transition_receipt::capture_before_on(
            &tx, lease, child, mode,
        )).transpose()?;
    let payload=redact_json(payload);
    let message=mailbox::send(&tx,lease,&child.run_id,&lease.root_run_id,child.role.as_str(),
        AgentRole::Coordinator.as_str(),kind,correlation_id,&child.assignment_id,revision,&payload)?;
    // Unlike deliver_expected, this helper participates in the caller's transaction.
    consume_proposal_mailbox_in_transaction(&tx,lease,&lease.root_run_id,&message,kind,&payload)?;
    scheduler::finish_child_in_transaction(&tx,lease,child,success,summary)?;
    // Check after the final resource write: later triggers must not silently
    // change the message's route, contents, or acknowledgement.
    let complete: bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_messages m JOIN agent_assignments a ON a.id=m.assignment_id \
         WHERE m.id=?1 AND m.run_id=?2 AND m.root_run_id=?2 AND m.from_run_id=?3 AND m.to_run_id=?2 \
         AND m.from_agent=?4 AND m.to_agent='coordinator' AND m.assignment_id=?5 \
         AND m.kind=?6 AND m.correlation_id=?7 AND m.evidence_revision=?8 AND m.payload_json=?9 \
         AND m.delivered_at<>'' AND m.acknowledged_at<>'' AND m.delivery_attempts=1 \
         AND a.evidence_revision=?8 AND a.budget_settled_at<>'')",
        params![message,lease.root_run_id,child.run_id,child.role.as_str(),child.assignment_id,
            kind,correlation_id,revision,payload.to_string()],|row|row.get(0),
    ).map_err(|error|format!("target_child_delivery_postcondition:{error}"))?;
    if !complete { return Err("target_child_delivery_postcondition_failed".into()); }
    let state=if success { "completed" } else { "failed" };
    let visible: i64=tx.query_row(
        "SELECT COUNT(DISTINCT event_type) FROM agent_collaboration_events e \
         WHERE sequence>?1 AND scan_id=?2 AND attempt_number=?3 AND entity_type=event_type AND ( \
           (event_type='assignment' AND entity_id=?4 AND json_extract(payload_json,'$.state')=?7) \
           OR (event_type='agent_run' AND entity_id=?5 AND json_extract(payload_json,'$.status')='terminal' \
             AND json_extract(payload_json,'$.terminalState')=?7) \
           OR (event_type='mailbox_message' AND entity_id=?6 AND json_extract(payload_json,'$.kind')=?8 \
             AND EXISTS(SELECT 1 FROM agent_messages m WHERE m.id=e.entity_id \
               AND json_extract(e.payload_json,'$.deliveredAt')=m.delivered_at \
               AND json_extract(e.payload_json,'$.acknowledgedAt')=m.acknowledged_at)))",
        params![event_floor,lease.scan_id,lease.attempt_number,child.assignment_id,child.run_id,message,state,kind],
        |row|row.get(0),
    ).map_err(|error|format!("target_child_delivery_event_postcondition:{error}"))?;
    if visible!=3 { return Err("target_child_delivery_event_postcondition_failed".into()); }
    validate_coordinator_lease(&tx,lease)?;
    require_executable_coordinator(&tx,lease)?;
    if let (Some(mode), Some(guard)) = (&capability_mode, &capability_guard) {
        crate::agent_runtime::web_mode::capability_transition_receipt::issue_after_on(
            &tx, lease, child, mode, guard, &message,
        )?;
    }
    if let Some(guard) = &authorization_guard {
        authorization_output_receipt::issue_after_on(&tx, lease, child, guard, &message)?;
    }
    tx.commit().map_err(|error|format!("target_child_delivery_commit:{error}"))?;
    Ok(capability_guard)
}
