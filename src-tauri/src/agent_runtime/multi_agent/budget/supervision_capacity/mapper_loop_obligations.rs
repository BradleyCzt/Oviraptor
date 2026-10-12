//! v15 Mapper issuance retains original follow-up obligations, not new caps.
use crate::agent_runtime::multi_agent::{budget, lease::CoordinatorLease};
use rusqlite::Transaction;

pub(super) fn require_on(
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    before_reservation: bool,
) -> Result<(), String> {
    budget::model::mapper_loop_contract::require_protocol_on(tx, actor)?;
    budget::admission::require_determinate(tx, &actor.root_run_id)?;
    let capability = super::capability_obligations::pending_on(tx, actor)?;
    let authorization = super::authorization_obligations::pending_on(tx, actor)?;
    let surface = super::external_surface_obligations::pending_on(tx, actor)?;
    let business = super::business_logic_obligations::pending_on(tx, actor)?;
    // FOLLOWUP_REQUESTS already includes the original Reviewer request. The
    // actual shared remaining has already deducted any SDK reservation.
    let retained = super::FOLLOWUP_REQUESTS
        .checked_add(capability)
        .and_then(|n| n.checked_add(authorization))
        .and_then(|n| n.checked_add(surface))
        .and_then(|n| n.checked_add(business))
        .and_then(|n| n.checked_add(if before_reservation { 2 } else { 0 }))
        .ok_or("budget_amount_overflow")?;
    budget::root::require_child_capacity(tx, &actor.root_run_id, 15_000, retained)
        .map_err(|error| format!("mapper_loop_reviewer_supervision_headroom:{error}"))
}
