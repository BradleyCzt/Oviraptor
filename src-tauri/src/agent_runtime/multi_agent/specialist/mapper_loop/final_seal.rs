//! Fresh normal close only. Historical verification never constructs a signer input.
use super::*;
use crate::agent_runtime::web_mode::{
    self, withdrawal_issuer::WithdrawalIssuer, ModeFact, VerifiedMode,
};
use rusqlite::types::Value as SqlValue;
pub(crate) const FINAL_SCHEMA: &str = include_str!("final_receipt/schema.sql");
const DOMAIN: &str = "oviraptor.original-mapper-loop-final.v1";
pub(crate) struct MapperFinalGuard {
    connection: usize,
    actor: CoordinatorLease,
    child: ScheduledChild,
    original: ModeFact,
    output: VerifiedMapperLoopOutput,
    floor: i64,
    issuer: WithdrawalIssuer,
}
pub(crate) struct FreshMapperLoopFinalReceipt {
    original: ModeFact,
    value: Value,
}
impl FreshMapperLoopFinalReceipt {
    pub(crate) fn encoded_for_original(&self, original: &ModeFact) -> Result<Vec<u8>, String> {
        if original != &self.original {
            return Err("mapper_loop_final_original_mode_changed".into());
        }
        encoded(&self.value)
    }
}
pub(crate) struct VerifiedMapperLoopFinalReceipt {
    id: String,
    message: String,
    value: Value,
}
impl VerifiedMapperLoopFinalReceipt {
    pub(crate) fn receipt_id(&self) -> &str {
        &self.id
    }
    pub(crate) fn message_id(&self) -> &str {
        &self.message
    }
    pub(crate) fn fact(&self) -> &Value {
        &self.value
    }
}
pub(crate) fn capture_final_before_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    original: &VerifiedMode,
) -> Result<MapperFinalGuard, String> {
    schema(db)?;
    web_mode::root::verify_birth(db, &actor.root_run_id, original)?;
    lease::require_executable_coordinator(db, actor)?;
    attempts::require_live_for_run(db, &child.run_id)?;
    let running:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.state='running' AND a.budget_settled_at='' AND a.finished_at='' AND r.status='running' AND r.finished_at='' AND r.cancel_requested_at='')",params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_mapper_loop_final_receipts WHERE child_run_id=?1)",[&child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !running || exists {
        return Err("mapper_loop_final_fresh_close_required".into());
    }
    let output = verified_original_output_on(db, actor, child)?;
    let floor = db
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM main.agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let issuer =
        WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, original.clone())?;
    issuer.require_actor(db, actor)?;
    Ok(MapperFinalGuard {
        connection: db as *const Connection as usize,
        actor: actor.clone(),
        child: child.clone(),
        original: original.fact().clone(),
        output,
        floor,
        issuer,
    })
}
pub(crate) fn issue_final_after_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    guard: &MapperFinalGuard,
    message_id: &str,
) -> Result<VerifiedMapperLoopFinalReceipt, String> {
    if db.is_autocommit()
        || db as *const Connection as usize != guard.connection
        || actor != &guard.actor
        || child != &guard.child
    {
        return Err("mapper_loop_final_actual_transaction_changed".into());
    }
    guard.issuer.require_actor(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    require_settled_output_on(db, actor, child, &guard.output)?;
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    let collection = json!({"payload":{"summary":guard.output.text()}});
    let delivery = delivery_on(db, actor, child, message_id, guard.floor, &collection)?;
    let value = json!({"schemaVersion":1,"purpose":"original_closed_mapper_loop_output","cause":"normal_completed_frozen_frontend_assessment","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"epoch":actor.lease_epoch,"fence":actor.fencing_token,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":grant.worker_attempt_id(),"messageId":message_id,"output":guard.output.fact(),"delivery":delivery,"executionAllowed":false});
    let fresh = FreshMapperLoopFinalReceipt {
        original: guard.original.clone(),
        value,
    };
    let signature = guard.issuer.sign_fresh_mapper_loop_final_receipt(&fresh)?;
    let text = fresh.value.to_string();
    let id = store::stable_hash(&text);
    let changed=db.execute("INSERT INTO main.agent_mapper_loop_final_receipts(receipt_id,root_run_id,assignment_id,child_run_id,worker_attempt_id,message_id,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?1,?8)",params![id,actor.root_run_id,child.assignment_id,child.run_id,grant.worker_attempt_id(),message_id,text,signature]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("mapper_loop_final_insert_unconfirmed".into());
    }
    read_final_original_on(db, actor, child)?.ok_or("mapper_loop_final_original_missing".into())
}
pub(crate) fn read_final_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Option<VerifiedMapperLoopFinalReceipt>, String> {
    if db.is_autocommit() {
        return Err("mapper_loop_snapshot_required".into());
    }
    schema(db)?;
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM main.agent_mapper_loop_final_receipts WHERE child_run_id=?1",
            [&child.run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count > 1 {
        return Err("mapper_loop_final_multiple_original_receipts".into());
    }
    let row:Option<(String,String,String,Vec<u8>)>=db.query_row("SELECT receipt_id,message_id,fact_json,signature FROM main.agent_mapper_loop_final_receipts WHERE child_run_id=?1",[&child.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, message, text, signature)) = row else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&text).map_err(|_| "mapper_loop_final_json_invalid")?;
    let canonical = value.to_string();
    if text != canonical || id != store::stable_hash(&text) {
        return Err("mapper_loop_final_hash_changed".into());
    }
    let original = crate::scanning::scope_contract::original_root_capture_mode(db, &actor.root_run_id)?;
    let issuer = WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, original)?;
    issuer.require_actor(db, actor)?;
    issuer.verify_mapper_loop_final_receipt(&encoded(&value)?, &signature)?;
    let output = verified_original_output_on(db, actor, child)?;
    require_settled_output_on(db, actor, child, &output)?;
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    let physical:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_mapper_loop_final_receipts WHERE receipt_id=?1 AND root_run_id=?2 AND assignment_id=?3 AND child_run_id=?4 AND worker_attempt_id=?5 AND message_id=?6 AND fact_hash=?1)",params![id,actor.root_run_id,child.assignment_id,child.run_id,grant.worker_attempt_id(),message],|r|r.get(0)).map_err(|e|e.to_string())?;
    let floor = value["delivery"]["eventFloor"]
        .as_i64()
        .ok_or("mapper_loop_final_event_floor_invalid")?;
    let delivery = delivery_on(
        db,
        actor,
        child,
        &message,
        floor,
        &json!({"payload":{"summary":output.text()}}),
    )?;
    let expected = json!({"schemaVersion":1,"purpose":"original_closed_mapper_loop_output","cause":"normal_completed_frozen_frontend_assessment","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"epoch":actor.lease_epoch,"fence":actor.fencing_token,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":grant.worker_attempt_id(),"messageId":message,"output":output.fact(),"delivery":delivery,"executionAllowed":false});
    if !physical || value != expected {
        return Err("mapper_loop_final_original_source_changed".into());
    }
    Ok(Some(VerifiedMapperLoopFinalReceipt { id, message, value }))
}
fn schema(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(FINAL_SCHEMA)
        .map_err(|e| e.to_string())?;
    for name in [
        "agent_mapper_loop_final_receipts",
        "mapper_loop_final_no_update",
        "mapper_loop_final_no_delete",
        "mapper_loop_final_no_replace",
    ] {
        let get = |db: &Connection| {
            db.query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get::<_, String>(0),
            )
            .map_err(|_| "mapper_loop_final_schema_missing".to_string())
        };
        if get(db)? != get(&expected)? {
            return Err("mapper_loop_final_schema_changed".into());
        }
    }
    Ok(())
}

fn delivery_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    id: &str,
    floor: i64,
    collection: &Value,
) -> Result<Value, String> {
    crate::collaboration_events::client_delivery_schema::verify(db)?;
    let payload = crate::agent_runtime::secrets::redact_json(&collection["payload"]);
    let canonical = payload.to_string();
    let correlation = format!("mapper:{}", child.assignment_id);
    let dedup = format!("{}:evidence_summary:{correlation}:1", child.assignment_id);
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_messages WHERE id=?1 AND run_id=?2 AND root_run_id=?2 AND from_run_id=?3 AND to_run_id=?2 AND assignment_id=?4 AND from_agent='spa_api_mapper' AND to_agent='coordinator' AND kind='evidence_summary' AND correlation_id=?5 AND dedup_key=?6 AND evidence_revision=1 AND artifact_refs_json='[]' AND payload_json=?7 AND created_at<>'' AND delivered_at<>'' AND acknowledged_at<>'' AND delivery_attempts=1)",params![id,actor.root_run_id,child.run_id,child.assignment_id,correlation,dedup,canonical],|r|r.get(0)).map_err(|e|e.to_string())?;
    let deliveries:i64=db.query_row("SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2 AND kind='evidence_summary'",params![actor.root_run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact || deliveries != 1 {
        return Err("mapper_loop_final_original_delivery_changed".into());
    }
    // The generic consumed-mailbox reader rechecks live attempt/C authority.
    // This signed historical source instead verifies its exact original route,
    // canonical payload, completed worker and immutable ACK/outbox below.
    let events=rows(db,"SELECT sequence,scan_id,attempt_number,entity_type,entity_id,event_type,payload_json,created_at FROM main.agent_collaboration_events WHERE entity_id=?1 ORDER BY sequence",&[id])?;
    let (created, delivered, ack): (String, String, String) = db
        .query_row(
            "SELECT created_at,delivered_at,acknowledged_at FROM agent_messages WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    if events.len() != 2 {
        return Err("mapper_loop_final_original_outbox_changed".into());
    }
    for (index, event) in events.iter().enumerate() {
        let SqlValue::Integer(sequence) = &event[0] else {
            return Err("mapper_loop_final_original_outbox_changed".into());
        };
        let SqlValue::Text(raw) = &event[6] else {
            return Err("mapper_loop_final_original_outbox_changed".into());
        };
        let SqlValue::Text(time) = &event[7] else {
            return Err("mapper_loop_final_original_outbox_changed".into());
        };
        let actual: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        let expected = json!({"kind":"evidence_summary","deliveredAt":if index==0 {""}else{&delivered},"acknowledgedAt":if index==0 {""}else{&ack}});
        if *sequence <= floor
            || event[1] != SqlValue::Text(actor.scan_id.clone())
            || event[2] != SqlValue::Integer(actor.attempt_number)
            || event[3] != SqlValue::Text("mailbox_message".into())
            || event[4] != SqlValue::Text(id.into())
            || event[5] != SqlValue::Text("mailbox_message".into())
            || actual != expected
            || (index == 0 && time != &created)
            || (index == 1 && time < &ack)
        {
            return Err("mapper_loop_final_original_outbox_changed".into());
        }
    }
    let closures=rows(db,"SELECT sequence,scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM main.agent_collaboration_events WHERE sequence>CAST(?1 AS INTEGER) AND ((entity_type='assignment' AND entity_id=?2 AND json_extract(payload_json,'$.state')='completed') OR (entity_type='agent_run' AND entity_id=?3 AND json_extract(payload_json,'$.terminalState')='completed')) ORDER BY sequence",&[&floor.to_string(),&child.assignment_id,&child.run_id])?;
    if closures.len() != 2
        || closures
            .iter()
            .filter(|event| event[3] == SqlValue::Text("assignment".into()))
            .count()
            != 1
        || closures
            .iter()
            .filter(|event| event[3] == SqlValue::Text("agent_run".into()))
            .count()
            != 1
    {
        return Err("mapper_loop_final_original_closure_events_changed".into());
    }
    for event in &closures {
        let SqlValue::Text(raw) = &event[6] else {
            return Err("mapper_loop_final_original_closure_events_changed".into());
        };
        let value: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        let expected = if event[3] == SqlValue::Text("assignment".into()) {
            json!({"role":"spa_api_mapper","state":"completed"})
        } else {
            json!({"role":"spa_api_mapper","status":"terminal","terminalState":"completed"})
        };
        if event[1] != SqlValue::Text(actor.scan_id.clone())
            || event[2] != SqlValue::Integer(actor.attempt_number)
            || event[3] != event[5]
            || value != expected
        {
            return Err("mapper_loop_final_original_closure_events_changed".into());
        }
    }
    let message = rows(
        db,
        "SELECT rowid,* FROM main.agent_messages WHERE id=?1",
        &[id],
    )?;
    Ok(
        json!({"eventFloor":floor,"messageId":id,"physicalFactsHash":store::stable_hash(&format!("{message:?}:{events:?}:{closures:?}"))}),
    )
}
fn encoded(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({"domain":DOMAIN,"output":value})).map_err(|e| e.to_string())
}
fn rows(db: &Connection, sql: &str, keys: &[&str]) -> Result<Vec<Vec<SqlValue>>, String> {
    let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
    let n = q.column_count();
    let r = q
        .query_map(rusqlite::params_from_iter(keys.iter()), |r| {
            (0..n)
                .map(|i| r.get::<_, SqlValue>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(r)
}
