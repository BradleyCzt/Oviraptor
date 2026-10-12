//! Multi Root diagnostics bind one already-committed original financial call.
//! These reads never initialize a Root, adopt C, authorize transport or store bodies.
use super::owner::{Domain, Owner};
use crate::agent_runtime::{
    multi_agent::budget::root::{model::tick::Tick, RootOwner},
    store::stable_hash,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub(super) fn load(db: &Connection, run: &str, round: i64) -> Result<Owner, String> {
    let row: (String, i64, String, String, String) = db.query_row(
        "SELECT r.scan_id,r.attempt_number,j.call_id,j.lease_attempt_id,j.request_hash
        FROM agent_root_model_journal j JOIN agent_runs r ON r.id=j.root_run_id
        JOIN agent_root_budget_attempts x ON x.id=j.lease_attempt_id AND x.root_run_id=r.id
        WHERE r.id=?1 AND j.round=?2 AND j.phase='dispatch' AND r.backend='native'
          AND r.orchestration_policy='multi' AND r.role='coordinator' AND r.assignment_id=''
          AND r.parent_run_id IS NULL AND r.root_run_id=r.id
          AND json_extract(x.contract_json,'$.root.scan')=r.scan_id
          AND json_extract(x.contract_json,'$.root.attempt')=r.attempt_number
          AND NOT EXISTS(SELECT 1 FROM agent_root_model_journal t WHERE t.call_id=j.call_id AND t.phase<>'dispatch')",
        params![run, round], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
    ).map_err(|_| "native_sdk_log_original_dispatch_missing")?;
    let mut owner = Owner {
        owner_id: String::new(),
        domain: Domain::Root,
        dispatch_key: row.2,
        scan_id: row.0,
        attempt: row.1,
        root_run_id: run.into(),
        run_id: run.into(),
        assignment_id: None,
        lease_attempt_id: row.3,
        worker_id: None,
        round,
        request_hash: row.4,
    };
    owner.seal()?;
    verify(db, &owner)?;
    Ok(owner)
}

pub(super) fn verify(db: &Connection, owner: &Owner) -> Result<(), String> {
    // The original financial owner checks the entire frozen Native Root contract
    // and its origin. It does not require current executable C for late diagnostics.
    RootOwner::load_original(db, &owner.root_run_id)?.verify(db)?;
    let (text, basis_hash, contract_text): (String, String, String) = db.query_row(
        "SELECT p.fact_json,p.basis_hash,x.contract_json FROM agent_root_tick_receipts p
        JOIN agent_root_budget_attempts x ON x.id=p.lease_attempt_id AND x.root_run_id=p.root_run_id
        JOIN agent_root_model_journal j ON j.call_id=p.call_id AND j.root_run_id=p.root_run_id
          AND j.round=p.round AND j.lease_attempt_id=p.lease_attempt_id AND j.request_hash=p.request_hash AND j.phase='dispatch'
        JOIN agent_runs r ON r.id=p.root_run_id
        WHERE p.call_id=?1 AND p.root_run_id=?2 AND p.round=?3 AND p.request_hash=?4
          AND p.lease_attempt_id=?5 AND p.phase='request' AND r.orchestration_policy='multi'
          AND r.scan_id=?6 AND r.attempt_number=?7 AND r.root_run_id=r.id AND r.parent_run_id IS NULL
          AND r.role='coordinator' AND r.assignment_id='' AND r.backend='native'",
        params![owner.dispatch_key,owner.root_run_id,owner.round,owner.request_hash,
            owner.lease_attempt_id,owner.scan_id,owner.attempt],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(|_| "native_sdk_log_original_dispatch_missing")?;
    let value: Value =
        serde_json::from_str(&text).map_err(|_| "native_sdk_log_original_binding_changed")?;
    let contract: Value = serde_json::from_str(&contract_text)
        .map_err(|_| "native_sdk_log_original_binding_changed")?;
    let request = &value["request"];
    let basis = &request["basis"];
    let canonical_value =
        serde_json::to_string(&value).map_err(|_| "native_sdk_log_original_binding_changed")?;
    let canonical_contract =
        serde_json::to_string(&contract).map_err(|_| "native_sdk_log_original_binding_changed")?;
    if canonical_value != text
        || canonical_contract != contract_text
        || value.as_object().is_none_or(|o| o.len() != 3)
        || value["version"] != 1
        || request.as_object().is_none_or(|o| o.len() != 4)
        || !matches!(request["version"].as_u64(), Some(1..=3))
        || request["owner"] != contract
        || contract["root"]["policy"] != "multi"
        || stable_hash(&request.to_string()) != owner.request_hash
        || basis.get("liveBudgetObservation") != contract["root"].get("liveBudgetObservation")
        || Tick::basis_hash(basis) != basis_hash
        || basis["root"] != owner.root_run_id
        || basis["scan"] != owner.scan_id
        || basis["attempt"] != owner.attempt
        || basis["target"] != contract["root"]["target"]
        || basis["coordinator"]["epoch"] != contract["coordinator"]["epoch"]
        || basis["coordinator"]["fence"] != contract["coordinator"]["fence"]
        || request["request"]["basisHash"] != basis_hash
        || !matches!(
            request["request"]["purpose"].as_str(),
            Some("coordinator_decision_v1" | "coordinator_changed_fact_v1")
        )
        || owner.dispatch_key
            != stable_hash(
                &json!({"owner":owner.lease_attempt_id,
            "root":owner.root_run_id,"round":owner.round,"request":owner.request_hash})
                .to_string(),
            )
    {
        return Err("native_sdk_log_original_binding_changed".into());
    }
    Ok(())
}
