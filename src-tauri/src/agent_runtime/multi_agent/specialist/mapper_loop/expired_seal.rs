//! Dedicated original expired Mapper output closure. No normal-final signing.
use super::expired_saved::{read_closed_original_on, OriginalExpiredMapperOutput};
use super::*;
use crate::agent_runtime::web_mode::{withdrawal_issuer::WithdrawalIssuer, ModeFact};
use rusqlite::types::Value as SqlValue;
pub(crate) const EXPIRED_SCHEMA: &str = include_str!("expired_receipt/schema.sql");
const DOMAIN: &str = "oviraptor.original-expired-mapper-loop-output.v1";
pub(crate) struct ExpiredMapperSealGuard<'proof, 'tx, 'conn> {
    transaction: &'tx Transaction<'conn>,
    financial: &'proof OriginalExpiredMapperOutput<'tx, 'conn>,
    actor: CoordinatorLease,
    child: ScheduledChild,
    original: ModeFact,
    issuer: WithdrawalIssuer,
    floor: i64,
}
pub(crate) struct FreshExpiredMapperReceipt<'proof, 'tx, 'conn> {
    transaction: &'tx Transaction<'conn>,
    financial: &'proof OriginalExpiredMapperOutput<'tx, 'conn>,
    actor: CoordinatorLease,
    child: ScheduledChild,
    original: ModeFact,
    value: Value,
}
impl FreshExpiredMapperReceipt<'_, '_, '_> {
    pub(crate) fn encoded_for_original(&self, original: &ModeFact) -> Result<Vec<u8>, String> {
        if original != &self.original {
            return Err("mapper_loop_expired_original_birth_changed".into());
        }
        self.financial
            .require_closed_original_on(self.transaction, &self.actor, &self.child)?;
        let message = self.value["messageId"]
            .as_str()
            .ok_or("mapper_loop_expired_message_missing")?;
        let floor = self.value["delivery"]["eventFloor"]
            .as_i64()
            .ok_or("mapper_loop_expired_event_floor_invalid")?;
        let actual = closed_value_on(
            self.transaction,
            &self.actor,
            &self.child,
            self.financial.fact(),
            self.financial.output().text(),
            message,
            floor,
        )?;
        if actual != self.value {
            return Err("mapper_loop_expired_fresh_source_changed".into());
        }
        encoded(&self.value)
    }
}
pub(crate) struct VerifiedExpiredMapperReceipt {
    id: String,
    message: String,
    worker: String,
    value: Value,
}
impl VerifiedExpiredMapperReceipt {
    pub(crate) fn receipt_id(&self) -> &str {
        &self.id
    }
    pub(crate) fn message_id(&self) -> &str {
        &self.message
    }
    pub(crate) fn fact(&self) -> &Value {
        &self.value
    }
    pub(crate) fn worker_attempt_id(&self) -> &str {
        &self.worker
    }
}
pub(crate) fn capture_before_on<'proof, 'tx, 'conn>(
    tx: &'tx Transaction<'conn>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    financial: &'proof OriginalExpiredMapperOutput<'tx, 'conn>,
) -> Result<ExpiredMapperSealGuard<'proof, 'tx, 'conn>, String> {
    financial.require_before_on(tx, actor, child)?;
    schema(tx)?;
    let prior:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_mapper_loop_expired_receipts WHERE child_run_id=?1 OR assignment_id=?2) OR EXISTS(SELECT 1 FROM agent_mapper_loop_final_receipts WHERE child_run_id=?1 OR assignment_id=?2) OR EXISTS(SELECT 1 FROM agent_messages WHERE assignment_id=?2 AND kind='evidence_summary')",params![child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if prior {
        return Err("mapper_loop_expired_fresh_delivery_required".into());
    }
    let original = crate::scanning::scope_contract::original_root_capture_mode(tx, &actor.root_run_id)?;
    let issuer =
        WithdrawalIssuer::from_verified_birth_on(tx, &actor.root_run_id, original.clone())?;
    let floor = tx
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(ExpiredMapperSealGuard {
        transaction: tx,
        financial,
        actor: actor.clone(),
        child: child.clone(),
        original: original.fact().clone(),
        issuer,
        floor,
    })
}
pub(crate) fn issue_after_on(
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    guard: &ExpiredMapperSealGuard<'_, '_, '_>,
    message: &str,
) -> Result<VerifiedExpiredMapperReceipt, String> {
    if !std::ptr::eq(tx, guard.transaction) || actor != &guard.actor || child != &guard.child {
        return Err("mapper_loop_expired_guard_binding_changed".into());
    }
    guard
        .financial
        .require_closed_original_on(tx, actor, child)?;
    schema(tx)?;
    let value = closed_value_on(
        tx,
        actor,
        child,
        guard.financial.fact(),
        guard.financial.output().text(),
        message,
        guard.floor,
    )?;
    let fresh = FreshExpiredMapperReceipt {
        transaction: guard.transaction,
        financial: guard.financial,
        actor: actor.clone(),
        child: child.clone(),
        original: guard.original.clone(),
        value,
    };
    let signature = guard
        .issuer
        .sign_fresh_mapper_loop_expired_receipt(&fresh)?;
    let text = fresh.value.to_string();
    let id = store::stable_hash(&text);
    let n=tx.execute("INSERT INTO agent_mapper_loop_expired_receipts(receipt_id,root_run_id,assignment_id,child_run_id,worker_attempt_id,message_id,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?1,?8)",params![id,actor.root_run_id,child.assignment_id,child.run_id,guard.financial.worker_attempt_id(),message,text,signature]).map_err(|e|e.to_string())?;
    if n != 1 {
        return Err("mapper_loop_expired_insert_unconfirmed".into());
    }
    read_original_on(tx, actor, child)?.ok_or("mapper_loop_expired_original_receipt_missing".into())
}
pub(crate) fn read_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Option<VerifiedExpiredMapperReceipt>, String> {
    budget::model::mapper_loop_contract::require_protocol_on(db, actor)?;
    if child.role != AgentRole::SpaApiMapper {
        return Err("mapper_loop_expired_role_invalid".into());
    }
    schema(db)?;
    let rows=db.prepare("SELECT receipt_id,message_id,fact_json,signature FROM agent_mapper_loop_expired_receipts WHERE child_run_id=?1 OR assignment_id=?2").map_err(|e|e.to_string())?.query_map(params![child.run_id,child.assignment_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Vec<u8>>(3)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 {
        return Err("mapper_loop_expired_multiple_original_receipts".into());
    }
    let (id, message, raw, signature) = &rows[0];
    let value: Value =
        serde_json::from_str(raw).map_err(|_| "mapper_loop_expired_original_json_invalid")?;
    let canonical = value.to_string();
    if canonical != *raw || store::stable_hash(raw) != *id {
        return Err("mapper_loop_expired_original_hash_changed".into());
    }
    let original = crate::scanning::scope_contract::original_root_capture_mode(db, &actor.root_run_id)?;
    let issuer = WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, original)?;
    issuer.verify_mapper_loop_expired_receipt(&encoded(&value)?, signature)?;
    let financial = read_closed_original_on(db, actor, child)?;
    let physical:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_mapper_loop_expired_receipts WHERE receipt_id=?1 AND root_run_id=?2 AND assignment_id=?3 AND child_run_id=?4 AND worker_attempt_id=?5 AND message_id=?6 AND fact_hash=?1)",params![id,actor.root_run_id,child.assignment_id,child.run_id,financial.fact()["workerAttemptId"].as_str().ok_or("mapper_loop_expired_worker_missing")?,message],|r|r.get(0)).map_err(|e|e.to_string())?;
    let floor = value["delivery"]["eventFloor"]
        .as_i64()
        .ok_or("mapper_loop_expired_event_floor_invalid")?;
    let actual = closed_value_on(
        db,
        actor,
        child,
        financial.fact(),
        financial.output().text(),
        message,
        floor,
    )?;
    if !physical || actual != value {
        return Err("mapper_loop_expired_original_source_changed".into());
    }
    Ok(Some(VerifiedExpiredMapperReceipt {
        id: id.clone(),
        message: message.clone(),
        worker: financial.fact()["workerAttemptId"]
            .as_str()
            .ok_or("mapper_loop_expired_worker_missing")?
            .to_owned(),
        value,
    }))
}
fn closed_value_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    financial: &Value,
    text: &str,
    message: &str,
    floor: i64,
) -> Result<Value, String> {
    if floor < 0 {
        return Err("mapper_loop_expired_event_floor_invalid".into());
    }
    let delivery = delivery_on(
        db,
        actor,
        child,
        message,
        floor,
        &json!({"payload":{"summary":text}}),
    )?;
    Ok(
        json!({"schemaVersion":1,"purpose":"original_closed_expired_mapper_output","cause":"original_signed_expired_worker_saved_output_delivery","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"epoch":actor.lease_epoch,"fence":actor.fencing_token,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":financial["workerAttemptId"],"messageId":message,"financial":financial,"delivery":delivery,"executionAllowed":false}),
    )
}
fn encoded(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({"domain":DOMAIN,"output":value})).map_err(|e| e.to_string())
}
fn schema(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(EXPIRED_SCHEMA)
        .map_err(|e| e.to_string())?;
    for name in [
        "agent_mapper_loop_expired_receipts",
        "mapper_loop_expired_no_update",
        "mapper_loop_expired_no_delete",
        "mapper_loop_expired_no_replace",
    ] {
        let get = |db: &Connection| {
            db.query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get::<_, String>(0),
            )
            .map_err(|_| "mapper_loop_expired_schema_missing".to_string())
        };
        let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name=?1 OR tbl_name='agent_mapper_loop_expired_receipts')",[name],|r|r.get(0)).map_err(|e|e.to_string())?;
        if shadow || get(db)? != get(&expected)? {
            return Err("mapper_loop_expired_schema_changed".into());
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
        return Err("mapper_loop_expired_original_delivery_changed".into());
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
        return Err("mapper_loop_expired_original_outbox_changed".into());
    }
    for (index, event) in events.iter().enumerate() {
        let SqlValue::Integer(sequence) = &event[0] else {
            return Err("mapper_loop_expired_original_outbox_changed".into());
        };
        let SqlValue::Text(raw) = &event[6] else {
            return Err("mapper_loop_expired_original_outbox_changed".into());
        };
        let SqlValue::Text(time) = &event[7] else {
            return Err("mapper_loop_expired_original_outbox_changed".into());
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
            return Err("mapper_loop_expired_original_outbox_changed".into());
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
        return Err("mapper_loop_expired_original_closure_events_changed".into());
    }
    for event in &closures {
        let SqlValue::Text(raw) = &event[6] else {
            return Err("mapper_loop_expired_original_closure_events_changed".into());
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
            return Err("mapper_loop_expired_original_closure_events_changed".into());
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
