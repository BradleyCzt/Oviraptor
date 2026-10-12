//! Same-transaction original aggregate Authorization output. No JSON issuer.
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{
        attempts,
        lease::{self, CoordinatorLease},
        scheduler::ScheduledChild,
    },
    store,
    web_mode::{self, withdrawal_issuer::WithdrawalIssuer, ModeFact, VerifiedMode},
};
use rusqlite::{params, types::Value as SqlValue, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
pub(crate) const RULE: &str = "original_closed_three_side_authorization_output_advisory_v1";
pub(crate) const CAPACITY_RULE: &str =
    "retain_one_original_authorization_output_assessment_per_worker_v1";
pub(crate) const SCHEMA_SQL: &str = include_str!("coordinator_authorization_output_receipt.sql");
const DOMAIN: &str = "oviraptor.original-authorization-output.v1";

pub(crate) struct AuthorizationOutputGuard {
    connection: usize,
    actor: CoordinatorLease,
    assignment: String,
    child: String,
    worker: String,
    original: ModeFact,
    target_dir: PathBuf,
    collection: super::VerifiedAuthorizationOutputCollection,
    event_floor: i64,
    issuer: WithdrawalIssuer,
}
impl std::fmt::Debug for AuthorizationOutputGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorizationOutputGuard")
            .finish_non_exhaustive()
    }
}
pub(crate) struct FreshAuthorizationOutputReceipt {
    original: ModeFact,
    value: Value,
}
impl FreshAuthorizationOutputReceipt {
    pub(crate) fn encoded_for_original(&self, original: &ModeFact) -> Result<Vec<u8>, String> {
        if &self.original != original {
            return Err("authorization_output_original_mode_changed".into());
        }
        encoded(&self.value)
    }
}
pub(crate) struct VerifiedAuthorizationOutputReceipt {
    value: Value,
    id: String,
    message: String,
}
impl VerifiedAuthorizationOutputReceipt {
    pub(crate) fn fact(&self) -> &Value {
        &self.value
    }
    pub(crate) fn receipt_id(&self) -> &str {
        &self.id
    }
    pub(crate) fn message_id(&self) -> &str {
        &self.message
    }
}

pub(crate) fn capture_before_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    target_dir: &Path,
    original: &VerifiedMode,
) -> Result<AuthorizationOutputGuard, String> {
    require_protocol_on(db, actor, child, original)?;
    verify_schema_on(db)?;
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    let worker = attempts::require_live_for_run(db, &child.run_id)?;
    let running:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.state='running' AND a.finished_at='' AND r.status='running' AND r.terminal_state='' AND r.finished_at='' AND r.cancel_requested_at='')",params![child.assignment_id,child.run_id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !running
        || worker.assignment_id != child.assignment_id
        || worker.child_run_id != child.run_id
        || worker.state != "running"
    {
        return Err("authorization_output_actual_running_source_required".into());
    }
    let existing:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_authorization_output_receipts WHERE worker_attempt_id=?1)",[&worker.id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if existing {
        return Err("authorization_output_existing_source".into());
    }
    let collection =
        super::native_authorization_output_collection_on(db, actor, child, target_dir)?;
    let event_floor = db
        .query_row(
            "SELECT coalesce(max(sequence),0) FROM main.agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let issuer =
        WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, original.clone())?;
    issuer.require_actor(db, actor)?;
    Ok(AuthorizationOutputGuard {
        connection: db as *const Connection as usize,
        actor: actor.clone(),
        assignment: child.assignment_id.clone(),
        child: child.run_id.clone(),
        worker: worker.id,
        original: original.fact().clone(),
        target_dir: target_dir.into(),
        collection,
        event_floor,
        issuer,
    })
}

pub(crate) fn issue_after_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    guard: &AuthorizationOutputGuard,
    message_id: &str,
) -> Result<VerifiedAuthorizationOutputReceipt, String> {
    if db.is_autocommit()
        || db as *const Connection as usize != guard.connection
        || actor != &guard.actor
        || child.assignment_id != guard.assignment
        || child.run_id != guard.child
        || child.role != AgentRole::Authorization
    {
        return Err("authorization_output_actual_transaction_guard_changed".into());
    }
    let original = super::original_root_capture_mode(db, &actor.root_run_id)?;
    require_protocol_on(db, actor, child, &original)?;
    verify_schema_on(db)?;
    guard.issuer.require_actor(db, actor)?;
    if original.fact() != &guard.original {
        return Err("authorization_output_original_mode_changed".into());
    }
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    let collection =
        super::native_authorization_output_collection_on(db, actor, child, &guard.target_dir)?;
    if collection.fact() != guard.collection.fact() {
        return Err("authorization_output_original_collection_changed".into());
    }
    require_completed_on(db, actor, child, &guard.worker)?;
    let delivery = delivery_on(
        db,
        actor,
        child,
        message_id,
        guard.event_floor,
        collection.fact(),
    )?;
    let directory = guard
        .target_dir
        .to_str()
        .filter(|s| !s.is_empty())
        .ok_or("authorization_output_directory_invalid")?;
    let value = json!({"schemaVersion":1,"purpose":"original_closed_authorization_output","cause":"normal_completed_authorization_result","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":guard.worker,"targetDirectory":directory,"messageId":message_id,"collection":collection.fact(),"delivery":delivery,"executionAllowed":false,"recoveryImplemented":false});
    let fresh = FreshAuthorizationOutputReceipt {
        original: guard.original.clone(),
        value,
    };
    let signature = guard
        .issuer
        .sign_fresh_authorization_output_receipt(&fresh)?;
    let canonical = fresh.value.to_string();
    let id = store::stable_hash(&canonical);
    let count=db.execute("INSERT INTO main.agent_authorization_output_receipts(receipt_id,root_run_id,assignment_id,child_run_id,worker_attempt_id,message_id,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?1,?8)",params![id,actor.root_run_id,child.assignment_id,child.run_id,guard.worker,message_id,canonical,signature]).map_err(|e|e.to_string())?;
    if count != 1 {
        return Err("authorization_output_insert_unconfirmed".into());
    }
    read_original_on(db, actor, child)?
        .ok_or("authorization_output_original_receipt_missing".into())
}

pub(crate) fn read_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Option<VerifiedAuthorizationOutputReceipt>, String> {
    let original = super::original_root_capture_mode(db, &actor.root_run_id)?;
    require_protocol_on(db, actor, child, &original)?;
    verify_schema_on(db)?;
    let worker = attempts::current(db, actor, &child.assignment_id)?;
    type Row = (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Vec<u8>,
    );
    let row:Option<Row>=db.query_row("SELECT receipt_id,root_run_id,assignment_id,child_run_id,message_id,fact_json,fact_hash,signature FROM main.agent_authorization_output_receipts WHERE worker_attempt_id=?1",[&worker.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, root, assignment, run, mid, raw, hash, tag)) = row else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let issuer = WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, original)?;
    issuer.require_actor(db, actor)?;
    issuer.verify_authorization_output_receipt(&encoded(&value)?, &tag)?;
    let canonical = value.to_string();
    if raw != canonical
        || id != hash
        || hash != store::stable_hash(&raw)
        || root != actor.root_run_id
        || assignment != child.assignment_id
        || run != child.run_id
        || value["schemaVersion"] != 1
        || value["purpose"] != "original_closed_authorization_output"
        || value["cause"] != "normal_completed_authorization_result"
        || value["rootRunId"] != root
        || value["scanId"] != actor.scan_id
        || value["attemptNumber"] != actor.attempt_number
        || value["targetKey"] != actor.target_key
        || value["coordinatorEpoch"] != actor.lease_epoch
        || value["coordinatorFence"] != actor.fencing_token
        || value["assignmentId"] != assignment
        || value["childRunId"] != run
        || value["workerAttemptId"] != worker.id
        || value["messageId"] != mid
        || value["executionAllowed"] != false
        || value["recoveryImplemented"] != false
    {
        return Err("authorization_output_original_binding_changed".into());
    }
    require_completed_on(db, actor, child, &worker.id)?;
    // Only after authenticating its signed bytes may the historical reader open
    // the original directory, whose scope the collection rechecks independently.
    let directory = value["targetDirectory"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("authorization_output_directory_invalid")?;
    let collection =
        super::native_authorization_output_collection_on(db, actor, child, Path::new(directory))?;
    if collection.fact() != &value["collection"] {
        return Err("authorization_output_original_collection_changed".into());
    }
    let floor = value["delivery"]["eventFloor"]
        .as_i64()
        .filter(|n| *n >= 0)
        .ok_or("authorization_output_original_delivery_changed")?;
    if delivery_on(db, actor, child, &mid, floor, collection.fact())? != value["delivery"] {
        return Err("authorization_output_original_delivery_changed".into());
    }
    Ok(Some(VerifiedAuthorizationOutputReceipt {
        value,
        id,
        message: mid,
    }))
}

fn require_protocol_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    original: &VerifiedMode,
) -> Result<(), String> {
    if db.is_autocommit() || child.role != AgentRole::Authorization {
        return Err("authorization_output_snapshot_role_invalid".into());
    }
    web_mode::root::verify_birth(db, &actor.root_run_id, original)?;
    let mode = web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("authorization_output_birth_missing")?;
    let protocol = mode.bootstrap_dispatch().unwrap_or(Value::Null);
    if mode.fact() != original.fact()
        || !matches!(protocol["schemaVersion"].as_i64(), Some(11..=15))
        || protocol["authorizationOutputRule"] != RULE
        || protocol["authorizationOutputRequestCapacityRule"] != CAPACITY_RULE
    {
        return Err("authorization_output_original_protocol_missing".into());
    }
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        db,
        &actor.root_run_id,
    )?
    .require_original_coordinator(db, actor)
}
fn require_completed_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
) -> Result<(), String> {
    let closed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_assignment_attempts x ON x.child_run_id=r.id AND x.assignment_id=a.id WHERE a.id=?1 AND r.id=?2 AND x.id=?3 AND a.coordinator_run_id=?4 AND a.state='completed' AND a.finished_at<>'' AND a.failure_class='' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>'' AND r.cancel_requested_at='' AND x.state='completed' AND x.finished_at<>'' AND x.failure_class='' AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=a.id OR child_run_id=r.id) AND revoked_at='') AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id))",params![child.assignment_id,child.run_id,worker,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if closed {
        Ok(())
    } else {
        Err("authorization_output_normal_completed_required".into())
    }
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
    let verified = collection["verified"]
        .as_i64()
        .filter(|n| *n >= 0)
        .ok_or("authorization_output_collection_invalid")?;
    let count = collection["controlCount"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("authorization_output_collection_invalid")?;
    let requests = collection["requestCount"]
        .as_i64()
        .ok_or("authorization_output_collection_invalid")?;
    if verified > count || count.checked_mul(3) != Some(requests) {
        return Err("authorization_output_collection_invalid".into());
    }
    let payload = crate::agent_runtime::secrets::redact_json(
        &json!({"summary":format!("authorization controls verified {verified}/{count}"),"verified":verified,"requestCount":requests}),
    );
    let canonical = payload.to_string();
    let correlation = format!("authorization:{}", child.assignment_id);
    let dedup = format!(
        "{}:authorization_result:{correlation}:1",
        child.assignment_id
    );
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_messages WHERE id=?1 AND run_id=?2 AND root_run_id=?2 AND from_run_id=?3 AND to_run_id=?2 AND assignment_id=?4 AND from_agent='authorization' AND to_agent='coordinator' AND kind='authorization_result' AND correlation_id=?5 AND dedup_key=?6 AND evidence_revision=1 AND artifact_refs_json='[]' AND payload_json=?7 AND created_at<>'' AND delivered_at<>'' AND acknowledged_at<>'' AND delivery_attempts=1)",params![id,actor.root_run_id,child.run_id,child.assignment_id,correlation,dedup,canonical],|r|r.get(0)).map_err(|e|e.to_string())?;
    let deliveries:i64=db.query_row("SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2 AND kind='authorization_result'",params![actor.root_run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact || deliveries != 1 {
        return Err("authorization_output_original_delivery_changed".into());
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
        return Err("authorization_output_original_outbox_changed".into());
    }
    for (index, event) in events.iter().enumerate() {
        let SqlValue::Integer(sequence) = &event[0] else {
            return Err("authorization_output_original_outbox_changed".into());
        };
        let SqlValue::Text(raw) = &event[6] else {
            return Err("authorization_output_original_outbox_changed".into());
        };
        let SqlValue::Text(time) = &event[7] else {
            return Err("authorization_output_original_outbox_changed".into());
        };
        let actual: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        let expected = json!({"kind":"authorization_result","deliveredAt":if index==0 {""}else{&delivered},"acknowledgedAt":if index==0 {""}else{&ack}});
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
            return Err("authorization_output_original_outbox_changed".into());
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
        return Err("authorization_output_original_closure_events_changed".into());
    }
    for event in &closures {
        let SqlValue::Text(raw) = &event[6] else {
            return Err("authorization_output_original_closure_events_changed".into());
        };
        let value: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        let expected = if event[3] == SqlValue::Text("assignment".into()) {
            json!({"role":"authorization","state":"completed"})
        } else {
            json!({"role":"authorization","status":"terminal","terminalState":"completed"})
        };
        if event[1] != SqlValue::Text(actor.scan_id.clone())
            || event[2] != SqlValue::Integer(actor.attempt_number)
            || event[3] != event[5]
            || value != expected
        {
            return Err("authorization_output_original_closure_events_changed".into());
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
pub(crate) fn verify_schema_on(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(SCHEMA_SQL)
        .map_err(|e| e.to_string())?;
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE tbl_name='agent_authorization_output_receipts' OR name LIKE 'authorization_output_%')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    fn inventory(db: &Connection) -> Result<Vec<Vec<SqlValue>>, String> {
        rows(db,"SELECT type,name,sql FROM main.sqlite_master WHERE tbl_name='agent_authorization_output_receipts' ORDER BY type,name",&[])
    }
    if shadow || inventory(db)? != inventory(&expected)? {
        Err("authorization_output_schema_conflict".into())
    } else {
        Ok(())
    }
}
