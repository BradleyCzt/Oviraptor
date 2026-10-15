//! Original successful Web closure only; no saved JSON issuer or execution grant.
use super::{root, ModeFact, VerifiedMode};
use crate::agent_runtime::{
    contract::AgentRole,
    execution_owner::NativeInvocationOwner,
    multi_agent::{
        attempts, budget,
        lease::{self, CoordinatorLease},
        scheduler::ScheduledChild,
    },
    store,
};
use rusqlite::{params, types::Value as SqlValue, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
pub(crate) const RULE: &str = "original_closed_web_capability_transition_advisory_v1";
pub(crate) const SCHEMA_SQL: &str = include_str!("capability_transition_receipt.sql");
const DOMAIN: &str = "oviraptor.original-web-capability-transition.v1";
type Rows = Vec<Vec<SqlValue>>;

pub(crate) struct CapabilityTransitionGuard {
    connection: usize,
    actor: CoordinatorLease,
    child: ScheduledChild,
    worker: String,
    mode: ModeFact,
    caps: Vec<Value>,
    fee: Value,
    floor: i64,
    _idle: Vec<NativeInvocationOwner>,
}
impl std::fmt::Debug for CapabilityTransitionGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapabilityTransitionGuard").finish_non_exhaustive()
    }
}
pub(crate) struct FreshCapabilityTransitionReceipt {
    mode: ModeFact,
    value: Value,
}
impl FreshCapabilityTransitionReceipt {
    pub(crate) fn message_for_original(&self, mode: &ModeFact) -> Result<Vec<u8>, String> {
        if &self.mode != mode {
            return Err("capability_transition_mode_changed".into());
        }
        message(&self.value)
    }
}
pub(crate) struct VerifiedCapabilityTransition {
    value: Value,
    id: String,
    message: String,
}
impl VerifiedCapabilityTransition {
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
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    proof: &VerifiedMode,
) -> Result<CapabilityTransitionGuard, String> {
    require_protocol(tx, actor, child, proof)?;
    verify_schema_on(tx)?;
    lease::validate_coordinator_lease(tx, actor)?;
    lease::require_executable_coordinator(tx, actor)?;
    let worker = attempts::current(tx, actor, &child.assignment_id)?;
    let live = attempts::require_live_for_run(tx, &child.run_id)?;
    if live.id != worker.id {
        return Err("capability_transition_live_worker_changed".into());
    }
    let ready:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.state='running' AND a.finished_at='' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='running' AND r.terminal_state='' AND r.finished_at='' AND r.cancel_requested_at='' AND EXISTS(SELECT 1 FROM agent_lane_leases l WHERE l.assignment_id=a.id AND l.scan_id=r.scan_id AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane))",params![child.assignment_id,child.run_id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !ready
        || !matches!(worker.state.as_str(), "leased" | "running")
        || worker.child_run_id != child.run_id
    {
        return Err("capability_transition_running_original_required".into());
    }
    let present:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_web_capability_transition_receipts WHERE worker_attempt_id=?1)",[&worker.id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if present {
        return Err("capability_transition_existing_source".into());
    }
    let caps = capabilities(tx, actor, child)?;
    if caps.is_empty() || caps.iter().any(|v| v["revokedAt"] != "") {
        return Err("capability_transition_live_original_required".into());
    }
    let mut idle = budget::web_lifetime::require_idle_original(tx, actor)?;
    let path = tx
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("capability_transition_database_required")?;
    let execution = crate::agent_runtime::execution_owner::probe_native_invocation(
        std::path::Path::new(path),
        &actor.scan_id,
        actor.attempt_number,
        "web-executor-original-execution",
        &child.run_id,
    )
    .map_err(|e| format!("capability_transition_execution_not_idle:{e}"))?
    .ok_or("capability_transition_original_exit_missing")?;
    idle.push(execution);
    let fee = financial_on(tx, actor, child, &worker.id)?;
    let floor = tx
        .query_row(
            "SELECT COALESCE(MAX(sequence),0) FROM main.agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(CapabilityTransitionGuard {
        connection: (&**tx as *const Connection) as usize,
        actor: actor.clone(),
        child: ScheduledChild {
            assignment_id: child.assignment_id.clone(),
            run_id: child.run_id.clone(),
            role: child.role,
        },
        worker: worker.id,
        mode: proof.fact().clone(),
        caps,
        fee,
        floor,
        _idle: idle,
    })
}

pub(crate) fn issue_after_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    proof: &VerifiedMode,
    guard: &CapabilityTransitionGuard,
    message_id: &str,
) -> Result<VerifiedCapabilityTransition, String> {
    require_protocol(db, actor, child, proof)?;
    verify_schema_on(db)?;
    if db as *const Connection as usize != guard.connection
        || guard.actor != *actor
        || guard.child.assignment_id != child.assignment_id
        || guard.child.run_id != child.run_id
        || guard.mode != *proof.fact()
    {
        return Err("capability_transition_original_guard_changed".into());
    }
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    require_closed(db, actor, child, &guard.worker)?;
    require_closed_slot(db, actor, child, &guard.worker)?;
    let caps = capabilities(db, actor, child)?;
    if caps.len() != guard.caps.len() {
        return Err("capability_transition_set_changed".into());
    }
    for (before, after) in guard.caps.iter().zip(&caps) {
        let mut normalized = after.clone();
        normalized["revokedAt"] = json!("");
        if &normalized != before || after["revokedAt"].as_str().is_none_or(str::is_empty) {
            return Err("capability_transition_revocation_changed".into());
        }
    }
    if financial_on(db, actor, child, &guard.worker)? != guard.fee {
        return Err("capability_transition_original_fees_changed".into());
    }
    let delivery = delivery_on(db, actor, child, message_id, guard.floor)?;
    let value = json!({"schemaVersion":1,"purpose":"original_closed_web_capability_transition","cause":"normal_completed_execution_result","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":guard.worker,"messageId":message_id,"beforeCapabilities":guard.caps,"afterCapabilities":caps,"financial":guard.fee,"delivery":delivery,"executionAllowed":false,"recoveryImplemented":false});
    let fresh = FreshCapabilityTransitionReceipt {
        mode: guard.mode.clone(),
        value,
    };
    let signature = proof.sign_fresh_capability_transition_receipt(&fresh)?;
    let canonical = fresh.value.to_string();
    let id = store::stable_hash(&canonical);
    let count=db.execute("INSERT INTO main.agent_web_capability_transition_receipts(receipt_id,root_run_id,worker_attempt_id,assignment_id,child_run_id,message_id,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?1,?8)",params![id,actor.root_run_id,guard.worker,child.assignment_id,child.run_id,message_id,canonical,signature]).map_err(|e|e.to_string())?;
    if count != 1 {
        return Err("capability_transition_insert_unconfirmed".into());
    }
    read_original_on(db, actor, child)?.ok_or("capability_transition_original_seal_missing".into())
}

pub(crate) fn read_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Option<VerifiedCapabilityTransition>, String> {
    if db.is_autocommit() {
        return Err("capability_transition_snapshot_required".into());
    }
    verify_schema_on(db)?;
    let proof = crate::scanning::scope_contract::original_root_capture_mode(db, &actor.root_run_id)?;
    require_protocol(db, actor, child, &proof)?;
    let worker = attempts::current(db, actor, &child.assignment_id)?;
    type Row = (
        String,
        String,
        String,
        Vec<u8>,
        String,
        String,
        String,
        String,
    );
    let row:Option<Row>=db.query_row("SELECT receipt_id,fact_json,fact_hash,signature,root_run_id,assignment_id,child_run_id,message_id FROM main.agent_web_capability_transition_receipts WHERE worker_attempt_id=?1",[&worker.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, text, hash, tag, root, assignment, run, mid)) = row else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    proof.verify_capability_transition_receipt(&message(&value)?, &tag)?;
    let canonical = value.to_string();
    if text != canonical
        || id != hash
        || hash != store::stable_hash(&text)
        || root != actor.root_run_id
        || assignment != child.assignment_id
        || run != child.run_id
        || value["schemaVersion"] != 1
        || value["purpose"] != "original_closed_web_capability_transition"
        || value["cause"] != "normal_completed_execution_result"
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
        return Err("capability_transition_original_binding_changed".into());
    }
    require_closed(db, actor, child, &worker.id)?;
    require_closed_slot(db, actor, child, &worker.id)?;
    let after = capabilities(db, actor, child)?;
    let before: Vec<Value> =
        serde_json::from_value(value["beforeCapabilities"].clone()).map_err(|e| e.to_string())?;
    if after.is_empty() || json!(after) != value["afterCapabilities"] || before.len() != after.len()
    {
        return Err("capability_transition_original_capabilities_changed".into());
    }
    for (b, a) in before.iter().zip(&after) {
        let mut normal = a.clone();
        normal["revokedAt"] = json!("");
        if b != &normal || a["revokedAt"].as_str().is_none_or(str::is_empty) {
            return Err("capability_transition_original_capabilities_changed".into());
        }
    }
    if financial_on(db, actor, child, &worker.id)? != value["financial"]
        || delivery_on(
            db,
            actor,
            child,
            &mid,
            value["delivery"]["eventFloor"]
                .as_i64()
                .ok_or("capability_transition_original_outbox_changed")?,
        )? != value["delivery"]
    {
        return Err("capability_transition_original_fact_changed".into());
    }
    Ok(Some(VerifiedCapabilityTransition {
        value,
        id,
        message: mid,
    }))
}
fn require_protocol(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    proof: &VerifiedMode,
) -> Result<(), String> {
    if db.is_autocommit() || child.role != AgentRole::WebExecutor {
        return Err("capability_transition_snapshot_role_invalid".into());
    }
    root::verify_birth(db, &actor.root_run_id, proof)?;
    let mode = root::read(db, &actor.root_run_id)?.ok_or("capability_transition_birth_missing")?;
    let p = mode.bootstrap_dispatch().unwrap_or(Value::Null);
    if mode.fact() != proof.fact()
        || !matches!(p["schemaVersion"].as_i64(), Some(10..=15))
        || p["capabilityTransitionRule"] != RULE
    {
        return Err("capability_transition_original_protocol_missing".into());
    }
    budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)
}
fn require_closed_slot(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
) -> Result<(), String> {
    let slot = budget::balance_for_attempt(
        db,
        &actor.root_run_id,
        &child.assignment_id,
        worker,
        "concurrency_batches",
    )?;
    if slot.reserved != 0 || slot.indeterminate != 0 || slot.consumed != 0 {
        return Err("capability_transition_original_slot_unresolved".into());
    }
    Ok(())
}
fn require_closed(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
) -> Result<(), String> {
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=r.id WHERE a.id=?1 AND r.id=?2 AND x.id=?3 AND a.coordinator_run_id=?4 AND a.lease_epoch=?5 AND a.fencing_token=?6 AND x.coordinator_epoch=?5 AND x.coordinator_fencing_token=?6 AND a.state='completed' AND a.failure_class='' AND a.finished_at<>'' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>'' AND r.cancel_requested_at='' AND x.state='completed' AND x.finished_at<>'' AND x.failure_class='' AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id) AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at=''))",params![child.assignment_id,child.run_id,worker,actor.root_run_id,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
    if valid {
        Ok(())
    } else {
        Err("capability_transition_original_completed_required".into())
    }
}
fn capabilities(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<Vec<Value>, String> {
    let mut q=db.prepare("SELECT id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at,revoked_at,created_at FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2 ORDER BY id").map_err(|e|e.to_string())?;
    let rows=q.query_map(params![child.assignment_id,child.run_id],|r|Ok(json!({"id":r.get::<_,String>(0)?,"rootRunId":r.get::<_,String>(1)?,"assignmentId":r.get::<_,String>(2)?,"childRunId":r.get::<_,String>(3)?,"capability":r.get::<_,String>(4)?,"epoch":r.get::<_,i64>(5)?,"fence":r.get::<_,String>(6)?,"expiresAt":r.get::<_,String>(7)?,"revokedAt":r.get::<_,String>(8)?,"createdAt":r.get::<_,String>(9)?}))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let declared: String = db
        .query_row(
            "SELECT capability_lease_json FROM agent_assignments WHERE id=?1 AND child_run_id=?2",
            params![child.assignment_id, child.run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let declared: Vec<String> = serde_json::from_str(&declared).map_err(|e| e.to_string())?;
    let expected: std::collections::BTreeSet<_> = declared.iter().map(String::as_str).collect();
    let actual: std::collections::BTreeSet<_> = rows
        .iter()
        .filter_map(|v| v["capability"].as_str())
        .collect();
    if expected != actual || declared.len() != expected.len() || rows.len() != actual.len() {
        return Err("capability_transition_declared_set_changed".into());
    }
    if rows.iter().any(|v| {
        v["rootRunId"] != actor.root_run_id
            || v["assignmentId"] != child.assignment_id
            || v["childRunId"] != child.run_id
            || v["epoch"] != actor.lease_epoch
            || v["fence"] != actor.fencing_token
    }) {
        return Err("capability_transition_foreign_capability".into());
    }
    Ok(rows)
}
fn financial_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
) -> Result<Value, String> {
    let unknown:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1 AND (received_at='' OR response_status=0)) OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1 AND status='running')",[&child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if unknown {
        return Err("budget_indeterminate_requires_reconciliation".into());
    }
    let usage =
        budget::web_lifetime::child_usage_original(db, actor, &child.assignment_id, &child.run_id)?;
    let prefix = format!("http:{}:", child.run_id);
    let sources=db.prepare("SELECT 'http:'||run_id||':'||invocation_id||':'||request_index||':'||request_hash FROM agent_http_request_claims WHERE run_id=?2 UNION SELECT source_id FROM main.agent_http_budget_receipts WHERE root_run_id=?1 AND run_id=?2 UNION SELECT source_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?3 AND lease_attempt_id=?4 AND dimension='target_requests' AND substr(source_id,1,length(?5))=?5 ORDER BY 1").map_err(|e|e.to_string())?.query_map(params![actor.root_run_id,child.run_id,child.assignment_id,worker,prefix],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut http = vec![];
    for source in sources {
        let r = super::http_budget_storage::read_original_on(db, actor, &source)?
            .ok_or("capability_transition_original_http_missing")?;
        if r.fact()["claim"]["runId"] != child.run_id {
            return Err("capability_transition_original_http_scope_changed".into());
        }
        http.push(r.receipt_id().to_owned());
    }
    let clock = budget::clock::original_worker_samples::original_worker_consumed_on(
        db, actor, child, worker,
    )?;
    for (i, dim) in budget::DIMENSIONS.iter().enumerate() {
        if i == 8 {
            let slot = budget::balance_for_attempt(
                db,
                &actor.root_run_id,
                &child.assignment_id,
                worker,
                dim,
            )?;
            if slot.indeterminate != 0 || slot.consumed != 0 || !matches!(slot.reserved, 0 | 1) {
                return Err("capability_transition_original_slot_unresolved".into());
            }
            continue;
        }
        let b =
            budget::balance_for_attempt(db, &actor.root_run_id, &child.assignment_id, worker, dim)?;
        let expected = match i {
            0 => usage.input_tokens,
            1 => usage.cached_input_tokens,
            2 => usage.output_tokens,
            3 => usage.model_requests,
            4 => i64::try_from(http.len()).map_err(|_| "budget_amount_overflow")?,
            9 => clock,
            _ => 0,
        };
        if b.reserved != 0 || b.indeterminate != 0 || b.consumed != expected {
            return Err(format!(
                "capability_transition_original_fees_unresolved:{dim}"
            ));
        }
    }
    let mut rows = vec![];
    for (sql,keys) in [("SELECT rowid,* FROM agent_web_model_journal WHERE child_run_id=?1 ORDER BY rowid",vec![child.run_id.as_str()]),("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension<>'concurrency_batches' ORDER BY rowid",vec![actor.root_run_id.as_str(),child.assignment_id.as_str(),worker]),("SELECT * FROM main.agent_http_budget_receipts WHERE root_run_id=?1 AND run_id=?2 ORDER BY receipt_id",vec![actor.root_run_id.as_str(),child.run_id.as_str()])]{rows.push(read_rows(db,sql,&keys)?);}
    let stored:(i64,i64,i64)=db.query_row("SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1 AND root_run_id=?2",params![child.run_id,actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    if stored
        != (
            usage.total_tokens,
            usage.cached_input_tokens,
            usage.model_requests,
        )
    {
        return Err("capability_transition_usage_projection_changed".into());
    }
    Ok(
        json!({"usage":usage.as_json(),"httpReceiptIds":http,"wallTimeMs":clock,"physicalFactsHash":store::stable_hash(&format!("{rows:?}"))}),
    )
}
fn delivery_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    id: &str,
    floor: i64,
) -> Result<Value, String> {
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_messages m JOIN agent_assignments a ON a.id=m.assignment_id WHERE m.id=?1 AND m.run_id=?2 AND m.root_run_id=?2 AND m.from_run_id=?3 AND m.to_run_id=?2 AND m.assignment_id=?4 AND m.from_agent='web_executor' AND m.to_agent='coordinator' AND m.kind='execution_result' AND m.correlation_id='executor-result:'||a.id AND m.evidence_revision=a.evidence_revision AND m.artifact_refs_json='[]' AND m.created_at<>'' AND m.delivered_at<>'' AND m.acknowledged_at<>'' AND m.delivery_attempts=1)",params![id,actor.root_run_id,child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {
        return Err("capability_transition_original_delivery_missing".into());
    }
    let (delivered, acknowledged): (String, String) = db
        .query_row(
            "SELECT delivered_at,acknowledged_at FROM main.agent_messages WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let message = read_rows(
        db,
        "SELECT rowid,* FROM main.agent_messages WHERE id=?1",
        &[id],
    )?;
    let events=read_rows(db,"SELECT sequence,scan_id,attempt_number,entity_type,entity_id,event_type,payload_json FROM main.agent_collaboration_events WHERE sequence>CAST(?1 AS INTEGER) AND entity_id=?2 ORDER BY sequence",&[&floor.to_string(),id])?;
    if events.len() != 2 {
        return Err("capability_transition_original_outbox_changed".into());
    }
    for (index, event) in events.iter().enumerate() {
        let SqlValue::Text(raw) = &event[6] else {
            return Err("capability_transition_original_outbox_changed".into());
        };
        let actual: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        let expected = json!({"kind":"execution_result","deliveredAt":if index==0 {""} else {&delivered},"acknowledgedAt":if index==0 {""} else {&acknowledged}});
        if actual != expected {
            return Err("capability_transition_original_outbox_changed".into());
        }
        if event[1] != SqlValue::Text(actor.scan_id.clone())
            || event[2] != SqlValue::Integer(actor.attempt_number)
            || event[3] != SqlValue::Text("mailbox_message".into())
            || event[5] != SqlValue::Text("mailbox_message".into())
        {
            return Err("capability_transition_original_outbox_changed".into());
        }
    }
    Ok(
        json!({"eventFloor":floor,"physicalHash":store::stable_hash(&format!("{message:?}:{events:?}"))}),
    )
}
fn read_rows(db: &Connection, sql: &str, keys: &[&str]) -> Result<Rows, String> {
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
fn message(v: &Value) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({"domain":DOMAIN,"transition":v})).map_err(|e| e.to_string())
}
pub(crate) fn verify_schema_on(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(SCHEMA_SQL)
        .map_err(|e| e.to_string())?;
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE tbl_name='agent_web_capability_transition_receipts' OR name LIKE 'capability_transition_%')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    fn inventory(db: &Connection) -> Result<Rows, String> {
        read_rows(db,"SELECT type,name,sql FROM main.sqlite_master WHERE tbl_name='agent_web_capability_transition_receipts' ORDER BY type,name",&[])
    }
    if shadow || inventory(db)? != inventory(&expected)? {
        Err("capability_transition_schema_conflict".into())
    } else {
        Ok(())
    }
}
