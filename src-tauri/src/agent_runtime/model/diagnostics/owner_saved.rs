//! Original SDK dispatch binding survives cancellation; it never grants I/O.
use super::owner::{Domain, Owner};
use crate::agent_runtime::store::stable_hash;
use rusqlite::{params, Connection};
use serde_json::json;
pub(super) fn verify(db: &Connection, owner: &Owner) -> Result<(), String> {
    let mut canonical = owner.clone();
    canonical.seal()?;
    if canonical != *owner
        || (owner.domain == Domain::Root
            && (owner.run_id != owner.root_run_id
                || owner.assignment_id.is_some()
                || owner.worker_id.is_some()))
        || (owner.domain != Domain::Root
            && (owner.assignment_id.is_none()
                || owner.worker_id.is_none()
                || owner.run_id == owner.root_run_id))
        || (owner.domain == Domain::Specialist && owner.round != 1)
    {
        return Err("native_sdk_log_owner_unverified".into());
    }
    if owner.domain != Domain::Root
        && owner.domain != Domain::MapperLoop
        && owner.dispatch_key
            != stable_hash(
                &json!([
                    owner.domain,
                    owner.assignment_id,
                    owner.run_id,
                    owner.round,
                    owner.request_hash
                ])
                .to_string(),
            )
    {
        return Err("native_sdk_log_owner_unverified".into());
    }
    if owner.domain == Domain::Root {
        crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
            db,
            &owner.root_run_id,
        )?;
    }
    let valid:bool=if owner.domain==Domain::MapperLoop {
        super::owner_mapper_loop::verify(db,owner,false)?;
        Ok(true)
    } else if owner.domain==Domain::Root {
  db.query_row("SELECT COUNT(*)=1 FROM agent_root_model_journal j JOIN agent_runs r ON r.id=j.root_run_id JOIN agent_root_budget_attempts x ON x.id=j.lease_attempt_id AND x.root_run_id=r.id
   WHERE j.call_id=?1 AND j.root_run_id=?2 AND j.round=?3 AND j.request_hash=?4 AND j.lease_attempt_id=?5 AND j.phase='dispatch'
   AND r.scan_id=?6 AND r.attempt_number=?7 AND r.backend='native' AND r.role='coordinator' AND r.orchestration_policy IN ('single','multi')
   AND json_extract(x.contract_json,'$.root.policy')=r.orchestration_policy
   AND r.assignment_id='' AND r.parent_run_id IS NULL AND json_extract(x.contract_json,'$.root.scan')=r.scan_id AND json_extract(x.contract_json,'$.root.attempt')=r.attempt_number",
   params![owner.dispatch_key,owner.root_run_id,owner.round,owner.request_hash,owner.lease_attempt_id,owner.scan_id,owner.attempt],|r|r.get(0))
 }else {
  let table=if owner.domain==Domain::SourceRound {"agent_source_model_rounds"}else{"agent_specialist_calls"};
  let round=if owner.domain==Domain::SourceRound {"c.round_number=?6"}else{"1=?6"};
  db.query_row(&format!("SELECT COUNT(*)=1 FROM {table} c JOIN agent_assignment_attempts x ON x.id=?5
   JOIN agent_runs r ON r.id=c.child_run_id WHERE c.assignment_id=?1 AND c.child_run_id=?2 AND c.root_run_id=?3 AND c.request_hash=?4 AND {round}
   AND x.worker_id=?7 AND x.assignment_id=c.assignment_id AND x.child_run_id=c.child_run_id AND x.root_run_id=c.root_run_id
   AND x.coordinator_epoch=c.lease_epoch AND x.coordinator_fencing_token=c.fencing_token
   AND r.scan_id=?8 AND r.attempt_number=?9 AND r.assignment_id=c.assignment_id AND r.root_run_id=c.root_run_id AND r.role=c.role AND r.backend='native'"),
   params![owner.assignment_id,owner.run_id,owner.root_run_id,owner.request_hash,owner.lease_attempt_id,owner.round,owner.worker_id,owner.scan_id,owner.attempt],|r|r.get(0))
 }.map_err(|_|"native_sdk_log_owner_unverified")?;
    if !valid {
        return Err("native_sdk_log_owner_unverified".into());
    }
    if owner.domain == Domain::Root {
        let multi: bool = db
            .query_row(
                "SELECT orchestration_policy='multi' FROM agent_runs WHERE id=?1",
                [&owner.root_run_id],
                |r| r.get(0),
            )
            .map_err(|_| "native_sdk_log_owner_unverified")?;
        if multi {
            super::owner_coordinator::verify(db, owner)?;
        }
    }
    let mut q=db.prepare("SELECT DISTINCT cost_phase FROM native_sdk_log_rows WHERE owner_id=?1 AND cost_phase<>''").map_err(|_|"native_sdk_log_cost_unverified")?;
    let costs = q
        .query_map([&owner.owner_id], |r| r.get::<_, String>(0))
        .map_err(|_| "native_sdk_log_cost_unverified")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "native_sdk_log_cost_unverified")?;
    if costs.len() > 1
        || costs
            .first()
            .is_some_and(|cost| owner.cost_phase(db).ok().flatten().as_ref() != Some(cost))
    {
        return Err("native_sdk_log_cost_unverified".into());
    }
    Ok(())
}
