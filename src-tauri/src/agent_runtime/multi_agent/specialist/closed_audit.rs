//! Pure original paid specialist proof; never a dispatch or live lease.
use super::*;
pub(crate) fn verify_received_original(
    db: &Connection,
    actor: &CoordinatorLease,
    assignment: &str,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("specialist_audit_snapshot_required".into());
    }
    let (child,role,hash):(String,String,String)=db.query_row(
        "SELECT c.child_run_id,c.role,c.request_hash FROM agent_specialist_calls c
        JOIN agent_assignments a ON a.id=c.assignment_id AND a.child_run_id=c.child_run_id
        JOIN agent_runs r ON r.id=c.child_run_id AND r.assignment_id=a.id
        WHERE c.assignment_id=?1 AND c.root_run_id=?2 AND a.coordinator_run_id=?2
        AND r.root_run_id=?2 AND r.parent_run_id=?2 AND r.scan_id=?3 AND r.attempt_number=?4 AND r.target_url=?5
        AND a.target_key=r.target_url AND a.role=c.role AND r.role=c.role AND a.lane=r.lane
        AND r.backend='native' AND r.orchestration_policy='multi'
        AND c.lease_epoch=?6 AND c.fencing_token=?7 AND a.lease_epoch=?6 AND a.fencing_token=?7
        AND c.state='received' AND a.state='completed' AND a.budget_settled_at<>''
        AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='terminal' AND r.terminal_state='completed'
        AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id)
        AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at='')",
        params![assignment,actor.root_run_id,actor.scan_id,actor.attempt_number,actor.target_key,actor.lease_epoch,actor.fencing_token],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"specialist_original_closed_receipt_required")?;
    let role = AgentRole::try_parse(&role)
        .filter(|v| v.as_str() == role)
        .ok_or("specialist_audit_role_invalid")?;
    if !matches!(
        role,
        AgentRole::InputParser
            | AgentRole::SpaApiMapper
            | AgentRole::IdentitySession
            | AgentRole::DeepInvestigator
            | AgentRole::ExternalSurface
            | AgentRole::BusinessLogic
            | AgentRole::ClientSide
            | AgentRole::EvidenceReviewer
    ) {
        return Err("specialist_audit_role_unsupported".into());
    }
    super::super::budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    super::super::budget::receipts::original_owner(db, &child, actor, assignment)?.verify(db)?;
    let call = PendingCall {
        lease: actor.clone(),
        child: ScheduledChild {
            assignment_id: assignment.into(),
            run_id: child,
            role,
        },
        request_hash: hash,
    };
    if role == AgentRole::BusinessLogic {
        received_original_for_output_on(db, actor, &call.child)?;
        return Ok(());
    }
    let row = load(db, &call)?.ok_or("specialist_original_closed_receipt_required")?;
    verify_received(db, &call, &row)?;
    Ok(())
}
