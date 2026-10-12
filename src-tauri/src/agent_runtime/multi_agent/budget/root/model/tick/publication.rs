//! Advisory local publication only; Rust retains every dispatch decision.
use super::{SavedDecision, Tick};
use crate::agent_runtime::{contract::AgentEventKind, store};
use rusqlite::{params, Connection, Transaction};
use serde_json::{json, Value};

impl Tick {
    pub(crate) fn published(
        &self,
        db: &Connection,
        saved: &SavedDecision,
    ) -> Result<Option<i64>, String> {
        self.verify_paid(db, saved)?;
        self.verify_original_terminal_proof(db)?;
        self.verify_local_parents(db)?;
        let Some(value) = self.read(db, "publication")? else {
            return Ok(None);
        };
        let sequence = value["eventSequence"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or("root_tick_publication_invalid")?;
        // Preserve the original physical event, including its timestamp. An
        // old receipt without this proof cannot adopt the row now on disk.
        let mut expected = self.publication(saved, sequence);
        expected["eventProof"] = value["eventProof"].clone();
        let original_event = self.event_row(db, sequence)?;
        let payload = crate::agent_runtime::secrets::redact_json(&expected["event"]);
        let event: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_events WHERE run_id=?1
            AND sequence=?2 AND event_type='model_round_completed' AND payload_json=?3
            AND artifact_refs_json='[]')",
                params![self.call.owner.root, sequence, payload.to_string()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if value != expected || !event || value["eventProof"] != original_event {
            return Err("root_tick_publication_changed".into());
        }
        self.verify_timeline(db, saved, sequence)?;
        Ok(Some(sequence))
    }
    pub(crate) fn publish(
        &self,
        tx: &Transaction<'_>,
        saved: &SavedDecision,
    ) -> Result<i64, String> {
        self.verify_paid(tx, saved)?;
        if let Some(sequence) = self.published(tx, saved)? {
            return Ok(sequence);
        }
        if saved.unknown {
            return Err("budget_indeterminate_requires_reconciliation".into());
        }
        self.call.require_next_work(tx)?;
        protect(tx, || {
            let sequence = store::append_event(
                tx,
                &self.call.owner.root,
                AgentEventKind::ModelRoundCompleted,
                &self.publication(saved, 0)["event"],
                &[],
            )?;
            let mut fact = self.publication(saved, sequence);
            fact["eventProof"] = self.event_row(tx, sequence)?;
            self.insert(tx, "publication", &fact)?;
            self.append_timeline(tx, saved, sequence)?;
            self.verify_paid(tx, saved)?;
            if self.published(tx, saved)? != Some(sequence) {
                return Err("root_tick_publication_unconfirmed".into());
            }
            self.call.require_next_work(tx)?;
            Ok(sequence)
        })
    }
    fn event_row(&self, db: &Connection, sequence: i64) -> Result<Value, String> {
        db.query_row(
            "SELECT rowid,id,run_id,sequence,event_type,payload_json,artifact_refs_json,created_at
            FROM agent_events WHERE run_id=?1 AND sequence=?2",
            params![self.call.owner.root, sequence],
            |r| {
                Ok(json!({"rowid":r.get::<_,i64>(0)?,"id":r.get::<_,i64>(1)?,
                "root":r.get::<_,String>(2)?,"sequence":r.get::<_,i64>(3)?,
                "type":r.get::<_,String>(4)?,"payload":r.get::<_,String>(5)?,
                "refs":r.get::<_,String>(6)?,"created":r.get::<_,String>(7)?}))
            },
        )
        .map_err(|_| "root_tick_original_publication_event_missing".into())
    }
    fn publication(&self, saved: &SavedDecision, sequence: i64) -> Value {
        let mut value = json!({"version":1,"eventSequence":sequence,"decisionHash":store::stable_hash(&saved.summary.as_json().to_string()),
            "responseHash":saved.invoice["responseHash"],"event":{"rootTickVersion":1,
                "rootControl":self.call.owner.id,"callId":self.call.id,"turns":self.call.round,
                "requestHash":self.call.request,"basisHash":self.basis,
                "responseHash":saved.invoice["responseHash"],"usage":saved.invoice["usage"],
                "modelRequests":1,"totalTokens":saved.invoice["usage"]["totalTokens"],
                "toolCalls":[],"advisoryOnly":true,"decisionSummary":saved.summary.as_json()}});
        if let Some(step) = &saved.local_step {
            value["event"]["localStep"] = step.as_json();
        }
        value
    }
}
fn protect<T>(tx: &Transaction<'_>, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    tx.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Insert { table_name }
            if c.database_name == Some("main")
                && c.accessor.is_none()
                && matches!(
                    table_name,
                    "agent_root_tick_receipts"
                        | "agent_events"
                        | "agent_collaboration_events"
                        | "agent_root_tick_timeline_receipts"
                ) =>
        {
            Authorization::Allow
        }
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }))
    .map_err(|e| e.to_string())?;
    let result = work();
    let clear = tx
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let value = result?;
    clear?;
    Ok(value)
}
