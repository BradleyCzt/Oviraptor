//! Original delivered assignment history, never recipient authority.
use super::*;
use crate::agent_runtime::multi_agent::{budget::root::RootOwner, scheduler::ScheduledChild};
use serde_json::json;
pub(crate) struct OriginalAssignmentDelivery {
    fact: JsonValue,
}
impl OriginalAssignmentDelivery {
    pub(crate) fn fact(&self) -> &JsonValue {
        &self.fact
    }
}
pub(crate) fn read_original_execution_assignment_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    revision: i64,
    expected: &JsonValue,
) -> Result<OriginalAssignmentDelivery, String> {
    if db.is_autocommit()
        || revision <= 0
        || child.role != crate::agent_runtime::contract::AgentRole::WebExecutor
    {
        return Err("mailbox_assignment_history_snapshot_or_scope_invalid".into());
    }
    RootOwner::load_original(db, &actor.root_run_id)?.require_original_coordinator(db, actor)?;
    crate::collaboration_events::client_delivery_schema::verify(db)?;
    let worker = super::super::attempts::current(db, actor, &child.assignment_id)?;
    if worker.child_run_id != child.run_id || worker.lease_epoch != 1 {
        return Err("mailbox_assignment_history_worker_conflict".into());
    }
    let route:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs c ON c.id=a.child_run_id WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.child_run_id=?3 AND a.role='web_executor' AND a.lane='target_touching' AND a.target_key=?4 AND a.evidence_revision=?5 AND a.lease_epoch=?6 AND a.fencing_token=?7 AND c.assignment_id=a.id AND c.root_run_id=?2 AND c.parent_run_id=?2 AND c.role=a.role AND c.lane=a.lane AND c.backend='native' AND c.orchestration_policy='multi' AND c.scan_id=?8 AND c.attempt_number=?9 AND c.target_url=?4)",params![child.assignment_id,actor.root_run_id,child.run_id,actor.target_key,revision,actor.lease_epoch,actor.fencing_token,actor.scan_id,actor.attempt_number],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !route {
        return Err("mailbox_assignment_history_route_conflict".into());
    }
    let correlation = format!("executor:{}", child.assignment_id);
    let dedup = format!(
        "{}:execution_assignment:{correlation}:{revision}",
        child.assignment_id
    );
    let canonical = redact_json(expected).to_string();
    let rows=db.prepare("SELECT id,created_at,delivered_at,acknowledged_at FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2 AND kind='execution_assignment'").map_err(|e|e.to_string())?.query_map(params![actor.root_run_id,child.assignment_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    if rows.len() != 1 {
        return Err("mailbox_assignment_history_original_missing".into());
    }
    let (id, created, delivered, ack) = &rows[0];
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_messages WHERE id=?1 AND run_id=?2 AND root_run_id=?2 AND from_run_id=?2 AND to_run_id=?3 AND assignment_id=?4 AND from_agent='coordinator' AND to_agent='web_executor' AND correlation_id=?5 AND dedup_key=?6 AND evidence_revision=?7 AND artifact_refs_json='[]' AND payload_json=?8 AND created_at<>'' AND delivered_at<>'' AND acknowledged_at<>'' AND delivery_attempts=1)",params![id,actor.root_run_id,child.run_id,child.assignment_id,correlation,dedup,revision,canonical],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact {
        return Err("mailbox_assignment_history_original_conflict".into());
    }
    let events=db.prepare("SELECT sequence,scan_id,attempt_number,entity_type,event_type,payload_json,created_at FROM agent_collaboration_events WHERE entity_id=?1 ORDER BY sequence").map_err(|e|e.to_string())?.query_map([id],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    // Execution assignment uses send INSERT, deliver_expected UPDATE, then
    // acknowledge UPDATE in a separate transaction: precisely three events.
    // Readonly output's atomic delivery/ACK has a different two-event contract.
    if events.len() != 3 {
        return Err("mailbox_assignment_history_outbox_changed".into());
    }
    for (i, e) in events.iter().enumerate() {
        let value: JsonValue = serde_json::from_str(&e.5).map_err(|e| e.to_string())?;
        let expected = json!({"kind":"execution_assignment","deliveredAt":if i==0 {""}else{delivered},"acknowledgedAt":if i<2 {""}else{ack}});
        if e.0 <= 0
            || e.1 != actor.scan_id
            || e.2 != actor.attempt_number
            || e.3 != "mailbox_message"
            || e.4 != "mailbox_message"
            || value != expected
            || (i == 0 && &e.6 != created)
            || (i == 1 && &e.6 < delivered)
            || (i == 2 && &e.6 < ack)
        {
            return Err("mailbox_assignment_history_outbox_changed".into());
        }
    }
    Ok(OriginalAssignmentDelivery {
        fact: json!({"messageId":id,"payload":redact_json(expected),"eventSequences":events.iter().map(|e|e.0).collect::<Vec<_>>(),"physicalDeliveryHash":crate::agent_runtime::store::stable_hash(&format!("{rows:?}:{events:?}")),"executionAllowed":false}),
    })
}
