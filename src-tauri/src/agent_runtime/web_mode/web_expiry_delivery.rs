//! Original financial-only expiry mailbox delivery. No worker resurrection.
use super::{root, withdrawal_issuer::WithdrawalIssuer, ModeFact};
use crate::agent_runtime::{
    multi_agent::{
        budget::model::expired_web_receipt::ExpiredWebFinancialProof,
        lease::{self, CoordinatorLease},
        mailbox,
        scheduler::ScheduledChild,
    },
    store,
};
use rusqlite::{params, types::Value as SqlValue, Connection, OptionalExtension};
use serde_json::{json, Value};

pub(crate) const SCHEMA_SQL: &str = include_str!("web_expiry_delivery.sql");
const DOMAIN: &str = "oviraptor.original-web-expiry-delivery.v1";

pub(crate) struct FreshWebExpiryDelivery {
    mode: ModeFact,
    value: Value,
}
impl FreshWebExpiryDelivery {
    pub(crate) fn message_for_original(&self, fact: &ModeFact) -> Result<Vec<u8>, String> {
        if &self.mode != fact {
            return Err("web_expiry_delivery_original_mode_changed".into());
        }
        message(&self.value)
    }
}

pub(crate) struct VerifiedWebExpiryDelivery {
    value: Value,
    receipt_id: String,
}
impl VerifiedWebExpiryDelivery {
    pub(crate) fn fact(&self) -> &Value {
        &self.value
    }
    pub(crate) fn receipt_id(&self) -> &str {
        &self.receipt_id
    }
    pub(crate) fn message_id(&self) -> &str {
        self.value["messageId"].as_str().unwrap_or("")
    }
}

// Must run in the original private transaction while the financial proof owns
// the original exit guards. No constructor accepts arbitrary payload/UsageDelta.
pub(crate) fn issue_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    issuer: &WithdrawalIssuer,
    financial: &ExpiredWebFinancialProof,
) -> Result<VerifiedWebExpiryDelivery, String> {
    if db.is_autocommit() {
        return Err("web_expiry_delivery_snapshot_required".into());
    }
    let sp = format!("web_expiry_delivery_{}", uuid::Uuid::new_v4().simple());
    db.execute_batch(&format!("SAVEPOINT {sp}"))
        .map_err(|e| e.to_string())?;
    match issue_inner_on(db, actor, child, issuer, financial) {
        Ok(saved) => {
            if let Err(error) = db.execute_batch(&format!("RELEASE {sp}")) {
                db.execute_batch(&format!("ROLLBACK TO {sp}; RELEASE {sp}"))
                    .map_err(|e| format!("{error};web_expiry_delivery_rollback:{e}"))?;
                return Err(error.to_string());
            }
            Ok(saved)
        }
        Err(error) => {
            db.execute_batch(&format!("ROLLBACK TO {sp}; RELEASE {sp}"))
                .map_err(|e| format!("{error};web_expiry_delivery_rollback:{e}"))?;
            Err(error)
        }
    }
}

fn issue_inner_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    issuer: &WithdrawalIssuer,
    financial: &ExpiredWebFinancialProof,
) -> Result<VerifiedWebExpiryDelivery, String> {
    require_original_on(db, actor, child, issuer, financial)?;
    verify_schema_on(db)?;
    if let Some(saved) = read_original_on(db, actor, child, issuer, financial)? {
        return Ok(saved);
    }
    let correlation = format!("web-expiry:{}", financial.worker_attempt_id());
    let revision: i64 = db.query_row("SELECT evidence_revision FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3",params![child.assignment_id,child.run_id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let dedup = format!(
        "{}:execution_expiry:{correlation}:{revision}",
        child.assignment_id
    );
    let present: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM main.agent_messages WHERE run_id=?1 AND dedup_key=?2)",
            params![actor.root_run_id, dedup],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if present {
        return Err("web_expiry_delivery_missing_original_seal".into());
    }
    let message_id = uuid::Uuid::new_v4().to_string();
    let payload = json!({"schemaVersion":1,"kind":"execution_expiry",
        "financial":financial.fact(),"executionAllowed":false,"completed":false,
        "workerState":"expired","assignmentState":"paused","recoveryImplemented":false});
    let canonical_payload = payload.to_string();
    let floor: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM main.agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let guard = WriteGuard::install(db)?;
    let inserted = db.execute("INSERT INTO main.agent_messages(id,run_id,from_agent,to_agent,kind,correlation_id,dedup_key,payload_json,root_run_id,from_run_id,to_run_id,assignment_id,evidence_revision) VALUES(?1,?2,'web_executor','coordinator','execution_expiry',?3,?4,?5,?2,?6,?2,?7,?8)",params![message_id,actor.root_run_id,correlation,dedup,canonical_payload,child.run_id,child.assignment_id,revision]).map_err(|e|e.to_string())?;
    if inserted != 1 {
        return Err("web_expiry_delivery_insert_unconfirmed".into());
    }
    // Existing Root-only local consumption validates exact route/payload and
    // increments delivery once. Its receiver is Root, never the expired worker.
    mailbox::consume_coordinator_message(
        db,
        actor,
        &actor.root_run_id,
        &message_id,
        "execution_expiry",
        &payload,
    )?;
    require_original_on(db, actor, child, issuer, financial)?;
    let (raw, created, delivered, acknowledged, attempts): (String,String,String,String,i64) = db.query_row("SELECT payload_json,created_at,delivered_at,acknowledged_at,delivery_attempts FROM main.agent_messages WHERE id=?1 AND run_id=?2 AND root_run_id=?2 AND from_run_id=?3 AND to_run_id=?2 AND assignment_id=?4 AND from_agent='web_executor' AND to_agent='coordinator' AND kind='execution_expiry' AND correlation_id=?5 AND dedup_key=?6 AND evidence_revision=?7 AND artifact_refs_json='[]'",params![message_id,actor.root_run_id,child.run_id,child.assignment_id,correlation,dedup,revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|e.to_string())?;
    if raw != canonical_payload
        || created.is_empty()
        || delivered.is_empty()
        || acknowledged.is_empty()
        || attempts != 1
    {
        return Err("web_expiry_delivery_ack_changed".into());
    }
    let events = event_rows(db, &message_id, floor)?;
    let total: i64 = db
        .query_row(
            "SELECT count(*) FROM main.agent_collaboration_events WHERE sequence>?1",
            [floor],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if events.len() != 2 || total != 2 {
        return Err("web_expiry_delivery_outbox_changed".into());
    }
    for (index, row) in events.iter().enumerate() {
        let expected = json!({"kind":"execution_expiry","deliveredAt":if index==0 {""} else {&delivered},"acknowledgedAt":if index==0 {""} else {&acknowledged}}).to_string();
        if row.1 != actor.scan_id
            || row.2 != actor.attempt_number
            || row.3 != "mailbox_message"
            || row.4 != message_id
            || row.5 != "mailbox_message"
            || serde_json::from_str::<Value>(&row.6).map_err(|e| e.to_string())?
                != serde_json::from_str::<Value>(&expected).map_err(|e| e.to_string())?
        {
            return Err("web_expiry_delivery_outbox_changed".into());
        }
    }
    let value = json!({"schemaVersion":1,"purpose":"original_paused_web_execution_expiry_delivery",
        "rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,
        "targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,
        "assignmentId":child.assignment_id,"childRunId":child.run_id,
        "workerAttemptId":financial.worker_attempt_id(),"expiryReceiptId":financial.expiry_receipt_id(),
        "financial":financial.fact(),"messageId":message_id,"correlationId":correlation,"dedupKey":dedup,
        "evidenceRevision":revision,"eventSequences":events.iter().map(|row|row.0).collect::<Vec<_>>(),
        "physicalDeliveryHash":delivery_hash(db,&message_id,&events)?,"executionAllowed":false,"completed":false});
    let mode = root::read(db, &actor.root_run_id)?
        .ok_or("web_expiry_delivery_birth_missing")?
        .fact()
        .clone();
    let fresh = FreshWebExpiryDelivery { mode, value };
    let tag = issuer.sign_fresh_web_expiry_delivery(&fresh)?;
    let text = fresh.value.to_string();
    let hash = store::stable_hash(&text);
    let n = db.execute("INSERT INTO main.agent_web_expiry_deliveries(receipt_id,root_run_id,worker_attempt_id,assignment_id,child_run_id,expiry_receipt_id,message_id,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?1,?9)",params![hash,actor.root_run_id,financial.worker_attempt_id(),child.assignment_id,child.run_id,financial.expiry_receipt_id(),message_id,text,tag]).map_err(|e|e.to_string())?;
    if n != 1 {
        return Err("web_expiry_delivery_seal_insert_unconfirmed".into());
    }
    let saved = read_original_on(db, actor, child, issuer, financial)?
        .ok_or("web_expiry_delivery_seal_missing")?;
    drop(guard);
    Ok(saved)
}

pub(crate) fn read_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    issuer: &WithdrawalIssuer,
    financial: &ExpiredWebFinancialProof,
) -> Result<Option<VerifiedWebExpiryDelivery>, String> {
    require_original_on(db, actor, child, issuer, financial)?;
    let saved = read_closed_original_on(db, actor, child)?;
    if saved
        .as_ref()
        .is_some_and(|r| r.value["financial"] != financial.fact())
    {
        return Err("web_expiry_delivery_original_financial_changed".into());
    }
    Ok(saved)
}

// Original source-only verification. No expiry sweep, settlement, writes,
// ownership-lock reprobes, mailbox delivery or new financial proof issuance.
pub(crate) fn read_closed_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Option<VerifiedWebExpiryDelivery>, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{attempts, budget},
    };
    if db.is_autocommit() {
        return Err("web_expiry_delivery_snapshot_required".into());
    }
    if child.role != AgentRole::WebExecutor {
        return Err("web_expiry_delivery_role_invalid".into());
    }
    verify_schema_on(db)?;
    let original = crate::scanning::scope_contract::original_root_capture_mode(db, &actor.root_run_id)?;
    let issuer = WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, original)?;
    issuer.require_actor(db, actor)?;
    budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    let worker = attempts::current(db, actor, &child.assignment_id)?;
    if worker.child_run_id != child.run_id
        || worker.state != "expired"
        || worker.failure_class != "worker_lease_expired"
    {
        return Err("web_expiry_delivery_original_worker_changed".into());
    }
    issuer.verify_issued(db, &worker.id)?;
    type Row = (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        Vec<u8>,
    );
    let stored:Option<Row>=db.query_row("SELECT receipt_id,root_run_id,worker_attempt_id,assignment_id,child_run_id,expiry_receipt_id,message_id,fact_json,fact_hash,signature FROM main.agent_web_expiry_deliveries WHERE worker_attempt_id=?1",[&worker.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, root, stored_worker, assignment, run, expiry, mid, text, hash, tag)) = stored
    else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let canonical_fact = value.to_string();
    issuer.verify_web_expiry_delivery(&message(&value)?, &tag)?;
    let expiry_id:String=db.query_row("SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1 AND root_run_id=?2",params![worker.id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if text != canonical_fact
        || id != hash
        || hash != store::stable_hash(&text)
        || tag.len() != 32
        || root != actor.root_run_id
        || stored_worker != worker.id
        || assignment != child.assignment_id
        || run != child.run_id
        || expiry != expiry_id
        || value["schemaVersion"] != 1
        || value["purpose"] != "original_paused_web_execution_expiry_delivery"
        || value["rootRunId"] != root
        || value["scanId"] != actor.scan_id
        || value["attemptNumber"] != actor.attempt_number
        || value["targetKey"] != actor.target_key
        || value["coordinatorEpoch"] != actor.lease_epoch
        || value["coordinatorFence"] != actor.fencing_token
        || value["assignmentId"] != assignment
        || value["childRunId"] != run
        || value["workerAttemptId"] != stored_worker
        || value["expiryReceiptId"] != expiry
        || value["financial"]["expiryReceiptId"] != expiry
        || value["messageId"] != mid
        || value["executionAllowed"] != false
        || value["completed"] != false
    {
        return Err("web_expiry_delivery_original_binding_changed".into());
    }
    verify_financial_on(db, actor, child, &worker.id, &value["financial"])?;
    let sequences: Vec<i64> =
        serde_json::from_value(value["eventSequences"].clone()).map_err(|e| e.to_string())?;
    if sequences.len() != 2 || sequences[0] >= sequences[1] {
        return Err("web_expiry_delivery_outbox_changed".into());
    }
    let events = event_rows(db, &mid, 0)?;
    if events.iter().map(|r| r.0).collect::<Vec<_>>() != sequences
        || value["physicalDeliveryHash"] != delivery_hash(db, &mid, &events)?
    {
        return Err("web_expiry_delivery_original_fact_changed".into());
    }
    Ok(Some(VerifiedWebExpiryDelivery {
        value,
        receipt_id: id,
    }))
}

fn verify_financial_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
    fact: &Value,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::budget;
    let mode = root::read(db, &actor.root_run_id)?.ok_or("web_expiry_delivery_birth_missing")?;
    let protocol = mode.bootstrap_dispatch().unwrap_or(Value::Null);
    if !matches!(protocol["schemaVersion"].as_i64(), Some(9..=15))
        || protocol["webLeaseExpiryRule"] != budget::model::expired_web_receipt::RULE
    {
        return Err("web_expiry_delivery_original_protocol_changed".into());
    }
    let usage =
        budget::web_lifetime::child_usage_original(db, actor, &child.assignment_id, &child.run_id)?;
    if fact["schemaVersion"] != 1
        || fact["purpose"] != "original_expired_web_financial_closure"
        || fact["rootRunId"] != actor.root_run_id
        || fact["scanId"] != actor.scan_id
        || fact["attemptNumber"] != actor.attempt_number
        || fact["targetKey"] != actor.target_key
        || fact["coordinatorEpoch"] != actor.lease_epoch
        || fact["coordinatorFence"] != actor.fencing_token
        || fact["assignmentId"] != child.assignment_id
        || fact["childRunId"] != child.run_id
        || fact["workerAttemptId"] != worker
        || fact["usage"] != usage.as_json()
        || fact["executionAllowed"] != false
        || fact["recoveryImplemented"] != false
    {
        return Err("web_expiry_delivery_original_financial_changed".into());
    }
    let paused:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id AND r.assignment_id=a.id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.lease_epoch=?4 AND a.fencing_token=?5 AND a.state='paused' AND a.failure_class='worker_lease_expired' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.finished_at='' AND r.status='paused' AND r.terminal_state='' AND r.finished_at='' AND r.used_tokens=?6 AND r.used_cached_tokens=?7 AND r.used_requests=?8 AND r.role=a.role AND r.lane=a.lane AND EXISTS(SELECT 1 FROM agent_lane_leases l WHERE l.assignment_id=a.id AND l.scan_id=r.scan_id AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane) AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=a.id OR child_run_id=r.id) AND revoked_at=''))",params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token,usage.total_tokens,usage.cached_input_tokens,usage.model_requests],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !paused {
        return Err("web_expiry_delivery_paused_source_required".into());
    }
    let unknown:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1 AND (response_status=0 OR received_at='')) OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1 AND status='running')",[&child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if unknown {
        return Err("budget_indeterminate_requires_reconciliation".into());
    }
    let prefix = format!("http:{}:", child.run_id);
    let sources=db.prepare("SELECT 'http:'||run_id||':'||invocation_id||':'||request_index||':'||request_hash FROM agent_http_request_claims WHERE run_id=?2 UNION SELECT source_id FROM main.agent_http_budget_receipts WHERE root_run_id=?1 AND run_id=?2 UNION SELECT source_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?3 AND lease_attempt_id=?4 AND dimension='target_requests' AND substr(source_id,1,length(?5))=?5 ORDER BY 1").map_err(|e|e.to_string())?.query_map(params![actor.root_run_id,child.run_id,child.assignment_id,worker,prefix],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut receipts = Vec::new();
    for source in sources {
        let receipt = super::http_budget_storage::read_original_on(db, actor, &source)?
            .ok_or("web_expiry_delivery_original_http_missing")?;
        if receipt.fact()["claim"]["runId"] != child.run_id {
            return Err("web_expiry_delivery_original_http_changed".into());
        }
        receipts.push(receipt.receipt_id().to_string());
    }
    if fact["httpReceiptIds"] != json!(receipts) {
        return Err("web_expiry_delivery_original_http_changed".into());
    }
    for (index, dimension) in budget::DIMENSIONS.iter().enumerate() {
        let balance = budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            worker,
            dimension,
        )?;
        let expected = match index {
            0 => usage.input_tokens,
            1 => usage.cached_input_tokens,
            2 => usage.output_tokens,
            3 => usage.model_requests,
            4 => i64::try_from(receipts.len()).map_err(|e| e.to_string())?,
            9 => budget::clock::original_worker_samples::original_worker_consumed_on(
                db, actor, child, worker,
            )?,
            _ => 0,
        };
        if balance.reserved != 0 || balance.indeterminate != 0 || balance.consumed != expected {
            return Err("web_expiry_delivery_financial_unresolved".into());
        }
    }
    let mut rows = Vec::new();
    for (sql,keys) in [
        ("SELECT rowid,* FROM agent_root_mode_birth_seals WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_root_mode_definitions WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_assignment_attempts WHERE id=?1",vec![worker]),
        ("SELECT rowid,* FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",vec![worker]),
        ("SELECT rowid,* FROM agent_worker_expiry_issuance_seals WHERE root_run_id=?1 AND receipt_id IN (SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?2)",vec![actor.root_run_id.as_str(),worker]),
        ("SELECT rowid,* FROM agent_web_model_journal WHERE child_run_id=?1 ORDER BY rowid",vec![child.run_id.as_str()]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 ORDER BY rowid",vec![actor.root_run_id.as_str(),child.assignment_id.as_str(),worker]),
        ("SELECT * FROM main.agent_http_budget_receipts WHERE root_run_id=?1 AND run_id=?2 ORDER BY receipt_id",vec![actor.root_run_id.as_str(),child.run_id.as_str()]),
    ]{
        let mut q=db.prepare(sql).map_err(|e|e.to_string())?;let count=q.column_count();
        rows.push(q.query_map(rusqlite::params_from_iter(keys),|r|(0..count).map(|i|r.get::<_,SqlValue>(i)).collect::<rusqlite::Result<Vec<_>>>()).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?);
    }
    if fact["physicalFactsHash"] != store::stable_hash(&format!("{rows:?}")) {
        return Err("web_expiry_delivery_original_financial_changed".into());
    }
    Ok(())
}

fn require_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    issuer: &WithdrawalIssuer,
    financial: &ExpiredWebFinancialProof,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("web_expiry_delivery_snapshot_required".into());
    }
    financial.require_on(db, actor, child)?;
    issuer.require_actor(db, actor)?;
    issuer.verify_issued(db, financial.worker_attempt_id())?;
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    lease::require_active_attempt(db, &actor.scan_id, actor.attempt_number)?;
    let paused: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.state='paused' AND a.failure_class='worker_lease_expired' AND a.finished_at='' AND r.status='paused' AND r.terminal_state='' AND r.finished_at='' AND r.cancel_requested_at='')",params![child.assignment_id,child.run_id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !paused {
        return Err("web_expiry_delivery_paused_source_required".into());
    }
    Ok(())
}

type EventRow = (i64, String, i64, String, String, String, String);
fn event_rows(db: &Connection, id: &str, floor: i64) -> Result<Vec<EventRow>, String> {
    let mut q=db.prepare("SELECT sequence,scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM main.agent_collaboration_events WHERE entity_id=?1 AND sequence>?2 ORDER BY sequence").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![id, floor], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}
fn delivery_hash(db: &Connection, id: &str, events: &[EventRow]) -> Result<String, String> {
    let mut q = db
        .prepare("SELECT rowid,* FROM main.agent_messages WHERE id=?1")
        .map_err(|e| e.to_string())?;
    let count = q.column_count();
    let row = q
        .query_row([id], |r| {
            (0..count)
                .map(|i| r.get::<_, SqlValue>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?;
    Ok(store::stable_hash(&format!("{row:?}:{events:?}")))
}
fn message(value: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({"domain":DOMAIN,"delivery":value})).map_err(|e| e.to_string())
}

pub(crate) fn verify_schema_on(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(SCHEMA_SQL)
        .map_err(|e| e.to_string())?;
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE tbl_name='agent_web_expiry_deliveries' OR name LIKE 'web_expiry_delivery_%')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if shadow || schema_rows(db)? != schema_rows(&expected)? {
        return Err("web_expiry_delivery_schema_conflict".into());
    }
    Ok(())
}
fn schema_rows(db: &Connection) -> Result<Vec<(String, String, Option<String>)>, String> {
    let mut q=db.prepare("SELECT type,name,sql FROM main.sqlite_master WHERE tbl_name='agent_web_expiry_deliveries' OR name IN ('web_expiry_delivery_no_update','web_expiry_delivery_no_delete','web_expiry_delivery_no_replace') ORDER BY type,name").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}
struct WriteGuard<'a>(&'a Connection);
impl<'a> WriteGuard<'a> {
    fn install(db: &'a Connection) -> Result<Self, String> {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        db.authorizer(Some(|a: AuthContext<'_>| {
            let allow = match a.action {
                AuthAction::Insert { table_name } => {
                    a.database_name == Some("main")
                        && match table_name {
                            "agent_messages" | "agent_web_expiry_deliveries" => {
                                a.accessor.is_none()
                            }
                            "agent_collaboration_events" => matches!(
                                a.accessor,
                                Some(
                                    "agent_collaboration_message_insert"
                                        | "agent_collaboration_message_update"
                                )
                            ),
                            _ => false,
                        }
                }
                AuthAction::Update {
                    table_name,
                    column_name,
                } => {
                    a.database_name == Some("main")
                        && a.accessor.is_none()
                        && table_name == "agent_messages"
                        && matches!(
                            column_name,
                            "delivered_at" | "acknowledged_at" | "delivery_attempts"
                        )
                }
                AuthAction::Read { .. }
                | AuthAction::Select
                | AuthAction::Function { .. }
                | AuthAction::Transaction { .. }
                | AuthAction::Savepoint { .. }
                | AuthAction::Recursive => true,
                _ => false,
            };
            if allow {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }))
        .map_err(|e| e.to_string())?;
        Ok(Self(db))
    }
}
impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
    }
}
