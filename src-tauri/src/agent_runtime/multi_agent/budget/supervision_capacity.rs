//! Creation-only v6 request headroom. Retained capacity is neither a bill nor
//! an execution grant. Tokens and all other ceilings retain their own checks.
use rusqlite::{params, Connection, Transaction};
use serde_json::Value;
use std::collections::HashSet;

pub(crate) const ALLOCATION_RULE: &str = "retain_half_surplus_and_check_each_physical_claim";
// Reviewer + remaining Root floor + Client SDK/feedback + InputParser trigger/SDK. The budget
// assessment is paid before the first Web SDK/HTTP call, so issuance adds one.
const FOLLOWUP_REQUESTS: i64 = 6;
mod authorization_obligations;
mod business_logic_obligations;
mod capability_obligations;
mod external_surface_obligations;
mod mapper_loop_obligations;

pub(crate) fn require_mapper_loop_issue_headroom_on(
    tx: &Transaction<'_>, actor: &super::super::lease::CoordinatorLease,
) -> Result<(), String> {
    mapper_loop_obligations::require_on(tx, actor, true)
}

pub(crate) fn require_mapper_loop_reserved_headroom_on(
    tx: &Transaction<'_>, actor: &super::super::lease::CoordinatorLease,
) -> Result<(), String> {
    mapper_loop_obligations::require_on(tx, actor, false)
}

fn declared_v6(db: &Connection, root: &str) -> Result<bool, String> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE root_run_id=?1
         AND json_extract(contract_json,'$.root.bootstrapDispatch.schemaVersion') IN (6,7,8,9,10,11,12,13,14,15))",
        [root],
        |r| r.get(0),
    )
    .map_err(|e| e.to_string())
}

pub(crate) fn enabled(bootstrap: &Value) -> bool {
    matches!(bootstrap["schemaVersion"].as_i64(), Some(6..=15))
        && bootstrap["supervisionRequestAllocationRule"] == ALLOCATION_RULE
}

pub(crate) fn executor_requests(available: Option<i64>) -> Result<Option<i64>, String> {
    available
        .map(|n| {
            n.checked_sub(FOLLOWUP_REQUESTS + 1)
                .filter(|n| *n >= 2)
                .map(|n| n / 2)
                .ok_or_else(|| "multi_agent_executor_budget_unavailable".into())
        })
        .transpose()
}

/// New original v10 allocation retains one future capability assessment per
/// outstanding issued Web worker and one for this proposed new grant. Shared
/// remaining capacity already subtracts physical SDK reservations: do not
/// subtract those reservations again here.
pub(crate) fn executor_requests_on(
    db: &Connection,
    actor: &super::super::lease::CoordinatorLease,
    available: Option<i64>,
) -> Result<Option<i64>, String> {
    if !capability_obligations::enabled_on(db, &actor.root_run_id)? {
        return executor_requests(available);
    }
    let pending = capability_obligations::pending_on(db, actor)?;
    let authorization_pending = authorization_obligations::pending_on(db, actor)?;
    let surface_pending = external_surface_obligations::pending_on(db, actor)?;
    let business_pending = business_logic_obligations::pending_on(db, actor)?;
    let retained = pending
        .checked_add(authorization_pending)
        .and_then(|n| n.checked_add(surface_pending))
        .and_then(|n| n.checked_add(business_pending))
        .and_then(|n| n.checked_add(1))
        .and_then(|n| n.checked_add(FOLLOWUP_REQUESTS + 1))
        .ok_or("budget_amount_overflow")?;
    available
        .map(|n| {
            n.checked_sub(retained)
                .filter(|n| *n >= 2)
                .map(|n| n / 2)
                .ok_or_else(|| "multi_agent_executor_budget_unavailable".into())
        })
        .transpose()
}

/// The caller holds the same IMMEDIATE transaction as the physical HTTP claim.
/// A batch cannot use one successful check to authorize later requests.
pub(crate) fn require_http_headroom(
    tx: &Transaction<'_>,
    lease: &super::super::lease::CoordinatorLease,
    assignment: &str,
    run: &str,
) -> Result<(), String> {
    require_http_headroom_for_count(tx, lease, assignment, run, 1)
}

/// Pure preflight for a single request or the indivisible two-sided identity
/// comparison. Every physical claim still repeats the one-request guard.
pub(crate) fn require_http_headroom_for_count(
    tx: &Transaction<'_>,
    lease: &super::super::lease::CoordinatorLease,
    assignment: &str,
    run: &str,
    request_count: i64,
) -> Result<(), String> {
    if !matches!(request_count, 1 | 2) {
        return Err("root_supervision_request_count_invalid".into());
    }
    let Some(mode) = crate::agent_runtime::web_mode::root::read(tx, &lease.root_run_id)? else {
        // Legacy journal-only roots did not declare this protocol. A v6
        // original financial owner cannot downgrade by losing its sidecar.
        let declared = declared_v6(tx, &lease.root_run_id)?;
        return if declared {
            Err("web_mode_root_missing".into())
        } else {
            Ok(())
        };
    };
    if !enabled(&mode.bootstrap_dispatch().unwrap_or(Value::Null)) {
        if declared_v6(tx, &lease.root_run_id)? {
            return Err("root_supervision_request_contract_changed".into());
        }
        return Ok(());
    }
    let role: String = tx.query_row(
        "SELECT role FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND child_run_id=?3",
        params![assignment, lease.root_run_id, run], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if role != "web_executor" {
        return Ok(());
    }
    super::admission::require_determinate(tx, &lease.root_run_id)?;
    super::root::RootOwner::load_original(tx, &lease.root_run_id)?
        .require_original_coordinator(tx, lease)?;
    // Shared across workers: a second lane must count the first lane's physical
    // obligations too. Only an independently verified paid publication removes
    // an obligation; settlement or a graph row alone does not remove it.
    let pending = pending_obligations(
        tx,
        &lease.root_run_id,
        &lease.scan_id,
        lease.attempt_number,
        &lease.target_key,
    )?;
    let capability_pending = capability_obligations::pending_on(tx, lease)?;
    let authorization_pending = authorization_obligations::pending_on(tx, lease)?;
    let surface_pending = external_surface_obligations::pending_on(tx, lease)?;
    let business_pending = business_logic_obligations::pending_on(tx, lease)?;
    let required = http_required_for_count(pending, request_count)?
        .checked_add(capability_pending)
        .and_then(|n| n.checked_add(authorization_pending))
        .and_then(|n| n.checked_add(surface_pending))
        .and_then(|n| n.checked_add(business_pending))
        .ok_or("budget_amount_overflow")?;
    let (_, remaining) = super::root::remaining_child_capacity(tx, &lease.root_run_id)?;
    if remaining.is_some_and(|n| n < required) {
        return Err("root_supervision_request_capacity_exhausted".into());
    }
    Ok(())
}

/// Called inside the original new Authorization scheduling transaction, only
/// for a fresh assignment. It retains capacity, never model usage or a grant.
pub(crate) fn require_authorization_issue_headroom_on(
    tx: &Transaction<'_>,
    actor: &super::super::lease::CoordinatorLease,
) -> Result<(), String> {
    require_authorization_headroom_on(tx, actor, true)
}

/// Each dedicated three-side physical target claim repeats this same shared
/// headroom gate. The issued worker is already in inventory: no extra +1.
pub(crate) fn require_authorization_probe_headroom_on(
    tx: &Transaction<'_>,
    actor: &super::super::lease::CoordinatorLease,
) -> Result<(), String> {
    require_authorization_headroom_on(tx, actor, false)
}

fn require_authorization_headroom_on(
    tx: &Transaction<'_>,
    actor: &super::super::lease::CoordinatorLease,
    new_assignment: bool,
) -> Result<(), String> {
    if !authorization_obligations::enabled_on(tx, &actor.root_run_id)? {
        return Ok(());
    }
    super::admission::require_determinate(tx, &actor.root_run_id)?;
    super::root::RootOwner::load_original(tx, &actor.root_run_id)?
        .require_original_coordinator(tx, actor)?;
    let capability = capability_obligations::pending_on(tx, actor)?;
    let authorization = authorization_obligations::pending_on(tx, actor)?;
    let surface = external_surface_obligations::pending_on(tx, actor)?;
    let business = business_logic_obligations::pending_on(tx, actor)?;
    let required = FOLLOWUP_REQUESTS
        .checked_add(capability)
        .and_then(|n| n.checked_add(authorization))
        .and_then(|n| n.checked_add(surface))
        .and_then(|n| n.checked_add(business))
        .and_then(|n| n.checked_add(i64::from(new_assignment)))
        .ok_or("budget_amount_overflow")?;
    let (_, remaining) = super::root::remaining_child_capacity(tx, &actor.root_run_id)?;
    if remaining.is_some_and(|n| n < required) {
        return Err("root_supervision_request_capacity_exhausted".into());
    }
    Ok(())
}

/// Fresh Surface issuance retains its one future Root output and the one
/// actual specialist SDK reservation that the scheduler will create next.
pub(crate) fn require_external_surface_issue_headroom_on(
    tx: &Transaction<'_>,
    actor: &super::super::lease::CoordinatorLease,
    proposed_sdk_requests: i64,
) -> Result<(), String> {
    if !external_surface_obligations::enabled_on(tx, &actor.root_run_id)? {
        return Ok(());
    }
    if proposed_sdk_requests != 1 {
        return Err("root_supervision_surface_issue_requests_invalid".into());
    }
    require_external_surface_headroom_on(tx, actor, true, None)
}

/// The worker's SDK reservation is already deducted from shared remaining;
/// issued output inventory already contains its one final assessment.
pub(crate) fn require_external_surface_probe_headroom_on(
    tx: &Transaction<'_>,
    actor: &super::super::lease::CoordinatorLease,
) -> Result<(), String> {
    require_external_surface_headroom_on(tx, actor, false, None)
}

/// Same original writer, own unsent target claim only. All other unknown costs
/// retain the ordinary refusal; this creates neither a receipt nor a refund.
pub(crate) fn require_external_surface_presend_headroom_on(
    tx: &Transaction<'_>,
    proof: &crate::scanning::external_surface_response_receipt::ClaimedPublicSurfaceWitness,
) -> Result<(), String> {
    let actor = proof.verify_unsent_on(tx)?;
    require_external_surface_headroom_on(tx, actor, false, Some(proof))
}

fn require_external_surface_headroom_on(
    tx: &Transaction<'_>,
    actor: &super::super::lease::CoordinatorLease,
    new_assignment: bool,
    fresh: Option<&crate::scanning::external_surface_response_receipt::ClaimedPublicSurfaceWitness>,
) -> Result<(), String> {
    if !external_surface_obligations::enabled_on(tx, &actor.root_run_id)? {
        return Ok(());
    }
    if let Some(proof) = fresh {
        super::admission::require_determinate_for_fresh_public_surface_on(tx, proof)?;
    } else {
        super::admission::require_determinate(tx, &actor.root_run_id)?;
    }
    super::root::RootOwner::load_original(tx, &actor.root_run_id)?
        .require_original_coordinator(tx, actor)?;
    let capability = capability_obligations::pending_on(tx, actor)?;
    let authorization = authorization_obligations::pending_on(tx, actor)?;
    let surface = external_surface_obligations::pending_on(tx, actor)?;
    let business = business_logic_obligations::pending_on(tx, actor)?;
    let physical = pending_obligations(
        tx,
        &actor.root_run_id,
        &actor.scan_id,
        actor.attempt_number,
        &actor.target_key,
    )?;
    let required = FOLLOWUP_REQUESTS
        .checked_add(capability)
        .and_then(|n| n.checked_add(authorization))
        .and_then(|n| n.checked_add(surface))
        .and_then(|n| n.checked_add(business))
        .and_then(|n| n.checked_add(physical))
        .and_then(|n| n.checked_add(if new_assignment { 2 } else { 0 }))
        .ok_or("budget_amount_overflow")?;
    let (_, remaining) = super::root::remaining_child_capacity(tx, &actor.root_run_id)?;
    if remaining.is_some_and(|n| n < required) {
        return Err("root_supervision_request_capacity_exhausted".into());
    }
    Ok(())
}

/// A fresh phase-one BusinessLogic worker costs one actual SDK reservation
/// plus one future Root assessment. Existing SDK reserves are already subtracted.
pub(crate) fn require_business_logic_issue_headroom_on(
    tx: &Connection,
    actor: &super::super::lease::CoordinatorLease,
    proposed_sdk_requests: i64,
) -> Result<(), String> {
    if !business_logic_obligations::enabled_on(tx, &actor.root_run_id)? {
        return Ok(());
    }
    if proposed_sdk_requests != 1 {
        return Err("root_supervision_business_logic_issue_requests_invalid".into());
    }
    super::super::lease::validate_coordinator_lease(tx, actor)?;
    super::super::lease::require_executable_coordinator(tx, actor)?;
    super::admission::require_determinate(tx, &actor.root_run_id)?;
    super::clock::remaining(tx, &actor.root_run_id)?;
    super::root::RootOwner::load_original(tx, &actor.root_run_id)?
        .require_original_coordinator(tx, actor)?;
    let capability = capability_obligations::pending_on(tx, actor)?;
    let authorization = authorization_obligations::pending_on(tx, actor)?;
    let surface = external_surface_obligations::pending_on(tx, actor)?;
    let business = business_logic_obligations::pending_on(tx, actor)?;
    let physical = pending_obligations(
        tx,
        &actor.root_run_id,
        &actor.scan_id,
        actor.attempt_number,
        &actor.target_key,
    )?;
    let required = FOLLOWUP_REQUESTS
        .checked_add(capability)
        .and_then(|n| n.checked_add(authorization))
        .and_then(|n| n.checked_add(surface))
        .and_then(|n| n.checked_add(business))
        .and_then(|n| n.checked_add(physical))
        .and_then(|n| n.checked_add(2))
        .ok_or("budget_amount_overflow")?;
    let (_, remaining) = super::root::remaining_child_capacity(tx, &actor.root_run_id)?;
    if remaining.is_some_and(|n| n < required) {
        return Err("root_supervision_request_capacity_exhausted".into());
    }
    Ok(())
}

fn next_http_required(pending: i64) -> Result<i64, String> {
    http_required_for_count(pending, 1)
}

fn http_required_for_count(pending: i64, request_count: i64) -> Result<i64, String> {
    pending
        .checked_add(request_count)
        .and_then(|n| n.checked_add(FOLLOWUP_REQUESTS))
        .ok_or_else(|| "budget_amount_overflow".into())
}

// The guard and the read-only report use exactly the same original publication
// verification. A graph node or a settled HTTP receipt cannot erase this count.
fn pending_obligations(
    db: &Connection,
    root: &str,
    scan: &str,
    attempt: i64,
    target: &str,
) -> Result<i64, String> {
    let mut query = db.prepare(
        "SELECT c.run_id,c.invocation_id,c.request_index,c.request_hash,c.budget_attempt,c.ordinal
         FROM agent_http_request_claims c JOIN agent_runs r ON r.id=c.run_id
         WHERE r.root_run_id=?1 AND r.role='web_executor' AND c.scan_id=?2 AND c.attempt_number=?3 AND c.target_url=?4",
    ).map_err(|e| e.to_string())?;
    let mut pending = query
        .query_map(params![root, scan, attempt, target], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<HashSet<_>>>()
        .map_err(|e| e.to_string())?;
    let mut publications = db
        .prepare(
            "SELECT json_extract(p.fact_json,'$.eventSequence'),q.fact_json
         FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q
          ON q.call_id=p.call_id AND q.root_run_id=p.root_run_id AND q.phase='request'
         WHERE p.root_run_id=?1 AND p.phase='publication'
          AND json_extract(q.fact_json,'$.request.basis.phase')='new-evidence'",
        )
        .map_err(|e| e.to_string())?;
    for row in publications
        .query_map([root], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
    {
        let (event, raw) = row.map_err(|e| e.to_string())?;
        let publication = super::root::model::tick::Tick::project_publication(db, root, event)?;
        // A paid local tool step is still an intermediate deliberation. Verify
        // its original receipt, but retain the obligation until a final decision.
        if publication.record.get("localTools").is_some() {
            continue;
        }
        let fact: Value = serde_json::from_str(&raw).map_err(|_| "root_tick_json_invalid")?;
        let binding =
            &fact["request"]["basis"]["changedFact"]["semantic"]["binding"]["payload"]["httpClaim"];
        let key = (
            binding["runId"]
                .as_str()
                .ok_or("root_new_evidence_http_claim_missing")?
                .to_owned(),
            binding["invocationId"]
                .as_str()
                .ok_or("root_new_evidence_http_claim_missing")?
                .to_owned(),
            binding["requestIndex"]
                .as_i64()
                .ok_or("root_new_evidence_http_claim_missing")?,
            binding["requestHash"]
                .as_str()
                .ok_or("root_new_evidence_http_claim_missing")?
                .to_owned(),
            binding["budgetAttempt"]
                .as_i64()
                .ok_or("root_new_evidence_http_claim_missing")?,
            binding["ordinal"]
                .as_i64()
                .ok_or("root_new_evidence_http_claim_missing")?,
        );
        pending.remove(&key);
    }
    i64::try_from(pending.len()).map_err(|_| "budget_amount_overflow".into())
}

/// Numeric capacity is not an execution grant, a financial reservation, or a
/// guarantee about future provider charges. No physical identifiers are exposed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisionRequestDiagnostic {
    schema_version: u8,
    authoritative: bool,
    execution_allowed: bool,
    pending_physical_requests: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_capability_transitions: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_authorization_outputs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_external_surface_outputs: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_business_logic_outputs: Option<i64>,
    minimum_followup_model_requests: i64,
    next_http_required_model_requests: i64,
    remaining_shared_model_requests: Option<i64>,
    remaining_shared_model_tokens: Option<i64>,
    next_http_request_balance_sufficient: bool,
    fresh_work_blocked: Option<&'static str>,
    future_token_capacity_guaranteed: bool,
    usd_capacity_coverage: &'static str,
}

pub(crate) fn diagnostic(
    db: &Connection,
    root: &str,
) -> Result<Option<SupervisionRequestDiagnostic>, String> {
    // All reads must describe one committed snapshot, including publications,
    // outstanding obligations and the shared balances.
    if db.is_autocommit() {
        return Err("supervision_capacity_snapshot_required".into());
    }
    let Some(mode) = crate::agent_runtime::web_mode::root::read(db, root)? else {
        return if declared_v6(db, root)? {
            Err("web_mode_root_missing".into())
        } else {
            Ok(None)
        };
    };
    if !enabled(&mode.bootstrap_dispatch().unwrap_or(Value::Null)) {
        if declared_v6(db, root)? {
            return Err("root_supervision_request_contract_changed".into());
        }
        return Ok(None);
    }
    super::root::RootOwner::load_original(db, root)?;
    let (scan, attempt, target): (String, i64, String) = db
        .query_row(
            "SELECT scan_id,attempt_number,target_url FROM agent_runs WHERE id=?1",
            [root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let pending = pending_obligations(db, root, &scan, attempt, &target)?;
    let capability_pending = if capability_obligations::enabled_on(db, root)? {
        let actor = db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[root],|r|Ok(super::super::lease::CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).map_err(|e|e.to_string())?;
        Some(capability_obligations::pending_on(db, &actor)?)
    } else {
        None
    };
    let authorization_pending = if authorization_obligations::enabled_on(db, root)? {
        let actor = db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[root],|r|Ok(super::super::lease::CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).map_err(|e|e.to_string())?;
        Some(authorization_obligations::pending_on(db, &actor)?)
    } else {
        None
    };
    let surface_pending = if external_surface_obligations::enabled_on(db, root)? {
        let actor = db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[root],|r|Ok(super::super::lease::CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).map_err(|e|e.to_string())?;
        Some(external_surface_obligations::pending_on(db, &actor)?)
    } else {
        None
    };
    let business_pending = if business_logic_obligations::enabled_on(db, root)? {
        let actor = db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",[root],|r|Ok(super::super::lease::CoordinatorLease {scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).map_err(|e|e.to_string())?;
        Some(business_logic_obligations::pending_on(db, &actor)?)
    } else {
        None
    };
    let required = next_http_required(pending)?
        .checked_add(capability_pending.unwrap_or(0))
        .and_then(|n| n.checked_add(authorization_pending.unwrap_or(0)))
        .and_then(|n| n.checked_add(surface_pending.unwrap_or(0)))
        .and_then(|n| n.checked_add(business_pending.unwrap_or(0)))
        .ok_or("budget_amount_overflow")?;
    let (tokens, requests) = super::root::remaining_child_capacity(db, root)?;
    // Final elapsed facts are not the only deadline boundary: an original
    // clock can expire before its next sample. This read appends no time charge.
    let readiness = super::admission::require_determinate(db, root)
        .and_then(|()| super::clock::remaining(db, root).map(|_| ()));
    let blocked = match readiness {
        Ok(()) => None,
        Err(code) => match code.as_str() {
            "budget_indeterminate_requires_reconciliation" => Some("cost_reconciliation_required"),
            "budget_wall_time_exhausted" => Some("wall_time_exhausted"),
            "budget_elapsed_finalized_no_new_work" => Some("elapsed_finalized"),
            _ => return Err(code),
        },
    };
    Ok(Some(SupervisionRequestDiagnostic {
        schema_version: 1,
        authoritative: false,
        execution_allowed: false,
        pending_physical_requests: pending,
        pending_capability_transitions: capability_pending,
        pending_authorization_outputs: authorization_pending,
        pending_external_surface_outputs: surface_pending,
        pending_business_logic_outputs: business_pending,
        minimum_followup_model_requests: FOLLOWUP_REQUESTS,
        next_http_required_model_requests: required,
        remaining_shared_model_requests: requests,
        remaining_shared_model_tokens: tokens,
        next_http_request_balance_sufficient: requests.is_none_or(|n| n >= required),
        fresh_work_blocked: blocked,
        future_token_capacity_guaranteed: false,
        usd_capacity_coverage: "not_enforced_by_this_ledger",
    }))
}
