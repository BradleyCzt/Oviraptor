//! Original paid dispatch binding. Missing historical bindings are never issued.
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{attempts, budget, lease::CoordinatorLease, scheduler::ScheduledChild},
    store,
    web_mode::{withdrawal_issuer::WithdrawalIssuer, ModeFact},
};
use rusqlite::{params, types::Value as SqlValue, Connection};
use serde_json::{json, Value};
use std::path::Path;
pub(crate) const SCHEMA_SQL: &str = include_str!("business_logic_dispatch_receipt.sql");
const DOMAIN: &str = "oviraptor.original-business-logic-dispatch.v1";
pub(crate) struct FreshBusinessLogicDispatchReceipt {
    original: ModeFact,
    value: Value,
}
impl FreshBusinessLogicDispatchReceipt {
    pub(crate) fn encoded_for_original(&self, original: &ModeFact) -> Result<Vec<u8>, String> {
        if original != &self.original {
            return Err("business_logic_dispatch_original_mode_changed".into());
        }
        encoded(&self.value)
    }
}
pub(crate) struct VerifiedBusinessLogicDispatchReceipt {
    value: Value,
    id: String,
}
impl VerifiedBusinessLogicDispatchReceipt {
    pub(crate) fn fact(&self) -> &Value {
        &self.value
    }
    pub(crate) fn receipt_id(&self) -> &str {
        &self.id
    }
}

pub(super) fn issue_new_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    guard: &super::FreshBusinessLogicDispatchMint,
) -> Result<VerifiedBusinessLogicDispatchReceipt, String> {
    if db.is_autocommit()
        || guard.connection != db as *const Connection as usize
        || guard.root != actor.root_run_id
        || guard.assignment != child.assignment_id
        || guard.child != child.run_id
    {
        return Err("business_logic_dispatch_fresh_mint_required".into());
    }
    verify_schema_on(db)?;
    let original = super::original_root_capture_mode(db, &actor.root_run_id)?;
    let value = original_fact_on(db, actor, child, &guard.directory, guard.event)?;
    if value["originalInput"] != guard.input || value["workerAttemptId"] != guard.worker {
        return Err("business_logic_dispatch_fresh_scope_changed".into());
    }
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_business_logic_dispatch_receipts WHERE assignment_id=?1 OR child_run_id=?2)",params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if exists {
        return Err("business_logic_dispatch_existing_binding".into());
    }
    let fresh = FreshBusinessLogicDispatchReceipt {
        original: original.fact().clone(),
        value,
    };
    let issuer = WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, original)?;
    issuer.require_actor(db, actor)?;
    let tag = issuer.sign_fresh_business_logic_dispatch_receipt(&fresh)?;
    let raw = fresh.value.to_string();
    let id = store::stable_hash(&raw);
    db.execute("INSERT INTO main.agent_business_logic_dispatch_receipts(receipt_id,root_run_id,assignment_id,child_run_id,worker_attempt_id,event_sequence,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?1,?8)",params![id,actor.root_run_id,child.assignment_id,child.run_id,guard.worker,guard.event,raw,tag]).map_err(|e|format!("business_logic_dispatch_insert:{e}"))?;
    verify_original_on(db, actor, child)
}

pub(crate) fn verify_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<VerifiedBusinessLogicDispatchReceipt, String> {
    if db.is_autocommit() || child.role != AgentRole::BusinessLogic {
        return Err("business_logic_dispatch_snapshot_role_required".into());
    }
    verify_schema_on(db)?;
    let (id,root,assignment,run,worker,event,raw,hash,tag):(String,String,String,String,String,i64,String,String,Vec<u8>)=db.query_row("SELECT receipt_id,root_run_id,assignment_id,child_run_id,worker_attempt_id,event_sequence,fact_json,fact_hash,signature FROM main.agent_business_logic_dispatch_receipts WHERE assignment_id=?1",[&child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).map_err(|_|"business_logic_dispatch_original_binding_missing")?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|_| "business_logic_dispatch_original_binding_changed")?;
    let original = super::original_root_capture_mode(db, &actor.root_run_id)?;
    let issuer = WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, original)?;
    issuer.require_actor(db, actor)?;
    issuer.verify_business_logic_dispatch_receipt(&encoded(&value)?, &tag)?;
    let canonical = value.to_string();
    if raw != canonical
        || id != hash
        || hash != store::stable_hash(&raw)
        || root != actor.root_run_id
        || assignment != child.assignment_id
        || run != child.run_id
        || value["workerAttemptId"] != worker
        || value["dispatchEventSequence"] != event
    {
        return Err("business_logic_dispatch_original_binding_changed".into());
    }
    let directory = value["targetDirectory"]
        .as_str()
        .ok_or("business_logic_dispatch_original_directory_missing")?;
    if value != original_fact_on(db, actor, child, Path::new(directory), event)? {
        return Err("business_logic_dispatch_original_source_changed".into());
    }
    Ok(VerifiedBusinessLogicDispatchReceipt { value, id })
}

fn original_fact_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    directory: &Path,
    event: i64,
) -> Result<Value, String> {
    budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    let frame = super::NativeCoordinatorFrame::business_logic_input_on(db, actor, directory)?;
    let input = super::native_business_logic_input_collection_on(db, actor, directory)?;
    let worker = attempts::current(db, actor, &child.assignment_id)?;
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3 AND r.assignment_id=a.id AND a.role='business_logic' AND r.role=a.role AND a.lane='read_only_analysis' AND r.lane=a.lane AND r.scan_id=?4 AND r.attempt_number=?5 AND r.target_url=?6 AND a.target_key=?6 AND a.trigger_code='business_logic_frozen_evidence_ready' AND a.evidence_revision=1 AND a.contract_keys_json='[]' AND a.identity_handles_json='[]' AND a.lease_epoch=?7 AND a.fencing_token=?8 AND a.task_slice_json=?9)",params![child.assignment_id,child.run_id,actor.root_run_id,actor.scan_id,actor.attempt_number,actor.target_key,actor.lease_epoch,actor.fencing_token,input.task().to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact || worker.child_run_id != child.run_id || worker.lease_epoch != 1 {
        return Err("business_logic_dispatch_original_assignment_changed".into());
    }
    let (request_raw,publication_raw):(String,String)=db.query_row("SELECT q.fact_json,p.fact_json FROM agent_root_tick_receipts p JOIN agent_root_tick_receipts q ON q.call_id=p.call_id AND q.root_run_id=p.root_run_id AND q.phase='request' WHERE p.root_run_id=?1 AND p.phase='publication' AND json_extract(p.fact_json,'$.eventSequence')=?2",params![actor.root_run_id,event],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"business_logic_dispatch_original_publication_missing")?;
    let request: Value = serde_json::from_str(&request_raw)
        .map_err(|_| "business_logic_dispatch_original_request_invalid")?;
    let projection =
        budget::root::model::tick::Tick::project_publication(db, &actor.root_run_id, event)?;
    let suggestions = projection.record["summary"]["suggestions"]
        .as_array()
        .ok_or("business_logic_dispatch_original_decision_invalid")?;
    if projection.record.get("localTools").is_some()
        || suggestions.is_empty()
        || suggestions
            .iter()
            .any(|s| s.as_str() != Some("dispatch:business_logic"))
        || request["request"]["basis"]["phase"] != "business-logic-input"
        || request["request"]["basis"]["permittedSuggestion"] != "dispatch:business_logic"
        || request["request"]["basis"]["changedFact"] != frame.fact()
    {
        return Err("business_logic_dispatch_original_decision_invalid".into());
    }
    let mut physical = Vec::new();
    for (sql,keys) in [
 ("SELECT rowid,id,child_run_id,coordinator_run_id,role,lane,target_key,dedup_key,trigger_code,task_slice_json,evidence_revision,capability_lease_json,contract_keys_json,identity_handles_json,lease_epoch,fencing_token FROM agent_assignments WHERE id=?1",vec![child.assignment_id.as_str()]),
 ("SELECT rowid,id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id FROM agent_assignment_attempts WHERE id=?1",vec![worker.id.as_str()]),
 ("SELECT rowid,root_run_id,contract_key,assignment_id,lease_epoch,fencing_token,acquired_at FROM agent_contract_owners WHERE assignment_id=?1",vec![child.assignment_id.as_str()]),
 ] {let rows=super::NativeCoordinatorFrozenRows::capture(db,sql,&keys)?;physical.push(json!({"sql":rows.sql,"keys":rows.keys,"hash":rows.hash}));}
    Ok(
        json!({"schemaVersion":1,"purpose":"original_paid_business_logic_dispatch","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":worker.id,"targetDirectory":directory.to_str().ok_or("business_logic_dispatch_original_directory_invalid")?,"dispatchEventSequence":event,"originalInput":input.fact(),"originalFrame":frame.fact(),"originalPaidRequest":request,"originalPaidPublication":serde_json::from_str::<Value>(&publication_raw).map_err(|_|"business_logic_dispatch_original_publication_invalid")?,"originalPhysicalFacts":physical,"executionAllowed":false}),
    )
}
fn encoded(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({"domain":DOMAIN,"dispatch":value})).map_err(|e| e.to_string())
}
fn rows(db: &Connection) -> Result<Vec<Vec<SqlValue>>, String> {
    let mut q=db.prepare("SELECT type,name,sql FROM main.sqlite_master WHERE tbl_name='agent_business_logic_dispatch_receipts' ORDER BY type,name").map_err(|e|e.to_string())?;
    let result = q
        .query_map([], |r| Ok(vec![r.get(0)?, r.get(1)?, r.get(2)?]))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string());
    result
}
pub(crate) fn verify_schema_on(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(SCHEMA_SQL)
        .map_err(|e| e.to_string())?;
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE tbl_name='agent_business_logic_dispatch_receipts' OR name LIKE 'business_logic_dispatch_%')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if shadow || rows(db)? != rows(&expected)? {
        Err("business_logic_dispatch_schema_conflict".into())
    } else {
        Ok(())
    }
}
