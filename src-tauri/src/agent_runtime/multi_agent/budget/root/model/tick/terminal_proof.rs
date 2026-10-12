//! Capture at original billing time; later append cannot roll back the bill.
use super::{unpublished_projection::invoice_response, Tick};
use crate::agent_runtime::model::gateway::ModelResponse;
use rusqlite::{params, Connection, Transaction};
use serde_json::{json, Value};

pub(crate) struct TerminalCapture(Result<Value, String>);
impl Tick {
    pub(crate) fn capture_terminal_proof(
        &self,
        db: &Connection,
        phase: &str,
        response: Option<&ModelResponse>,
    ) -> TerminalCapture {
        TerminalCapture(self.terminal_fact(db, phase, response))
    }
    fn terminal_fact(
        &self,
        db: &Connection,
        phase: &str,
        response: Option<&ModelResponse>,
    ) -> Result<Value, String> {
        self.terminal_fact_version(db, phase, response, 2)
    }
    fn terminal_fact_version(
        &self,
        db: &Connection,
        phase: &str,
        response: Option<&ModelResponse>,
        version: u64,
    ) -> Result<Value, String> {
        if !matches!(version, 1 | 2) {
            return Err("root_tick_terminal_proof_invalid".into());
        }
        self.verify_request(db)?;
        if !matches!(
            (phase, response),
            ("received", Some(_)) | ("uncertain" | "unsent", None)
        ) {
            return Err("root_tick_terminal_proof_phase_invalid".into());
        }
        if let Some(response) = response {
            self.cost_rows(db, Some(response))?;
        } else {
            self.verify_stopped_cost_projection(db, Some(phase))?;
        }
        let journal: Value=db.query_row("SELECT rowid,root_run_id,lease_attempt_id,round,request_hash,receipt_json,created_at FROM agent_root_model_journal WHERE call_id=?1 AND phase=?2",params![self.call.id,phase],|r|Ok(json!({"rowid":r.get::<_,i64>(0)?,"root":r.get::<_,String>(1)?,"owner":r.get::<_,String>(2)?,"round":r.get::<_,i64>(3)?,"request":r.get::<_,String>(4)?,"receipt":r.get::<_,String>(5)?,"created":r.get::<_,String>(6)?}))).map_err(|e|e.to_string())?;
        let invoice: Value = serde_json::from_str(
            journal["receipt"]
                .as_str()
                .ok_or("root_tick_terminal_proof_invalid")?,
        )
        .map_err(|_| "root_tick_terminal_proof_invalid")?;
        let canonical = invoice.to_string();
        if journal["receipt"] != canonical
            || journal["rowid"].as_i64().is_none_or(|n| n <= 0)
            || journal["root"] != self.call.owner.root
            || journal["owner"] != self.call.owner.id
            || journal["round"] != self.call.round
            || journal["request"] != self.call.request
        {
            return Err("root_tick_terminal_proof_invalid".into());
        }
        self.call.verify(db, phase, &invoice)?;
        let prefix = format!("root:{}:root-model:{}:%", self.call.owner.id, self.call.id);
        let mut q=db.prepare("SELECT rowid,entry_id,root_run_id,assignment_id,lease_attempt_id,dimension,kind,amount,idempotency_key,source_id,created_at FROM agent_budget_entries WHERE (source_id=?1 OR idempotency_key LIKE ?2) AND kind<>'reconcile' ORDER BY entry_id").map_err(|e|e.to_string())?;
        let costs=q.query_map(params![self.call.id,prefix],|r|Ok(json!({"rowid":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"root":r.get::<_,String>(2)?,"assignment":r.get::<_,String>(3)?,"owner":r.get::<_,String>(4)?,"dimension":r.get::<_,String>(5)?,"kind":r.get::<_,String>(6)?,"amount":r.get::<_,i64>(7)?,"key":r.get::<_,String>(8)?,"source":r.get::<_,String>(9)?,"created":r.get::<_,String>(10)?}))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        let original_request = self.request_row(db)?;
        // The full canonical request remains in its immutable original row.
        // Bind its physical identity and bytes without recursively embedding
        // escaped copies in every later local model round.
        let request_proof = if version == 1 {
            original_request
        } else {
            let text = original_request["fact"]
                .as_str()
                .ok_or("root_tick_terminal_proof_invalid")?;
            json!({"rowid":original_request["rowid"],"created":original_request["created"],
                "factHash":crate::agent_runtime::store::stable_hash(text)})
        };
        Ok(
            json!({"version":version,"phase":phase,"requestProof":request_proof,"journalProof":journal,"costProof":costs}),
        )
    }
    fn verify_terminal_fact(&self, db: &Connection, fact: &Value) -> Result<(), String> {
        let phase = fact["phase"]
            .as_str()
            .ok_or("root_tick_terminal_proof_invalid")?;
        let invoice: Value = serde_json::from_str(
            fact["journalProof"]["receipt"]
                .as_str()
                .ok_or("root_tick_terminal_proof_invalid")?,
        )
        .map_err(|_| "root_tick_terminal_proof_invalid")?;
        let response = if phase == "received" {
            Some(invoice_response(&invoice)?)
        } else {
            None
        };
        let version = fact["version"]
            .as_u64()
            .ok_or("root_tick_terminal_proof_invalid")?;
        if self.terminal_fact_version(db, phase, response.as_ref(), version)? != *fact {
            return Err("root_tick_terminal_proof_changed".into());
        }
        Ok(())
    }
    // Caller must already have committed the original financial transaction.
    // Only this captured original value can be appended; never rebuild a proof
    // from current rows on a read, restart or historical reconciliation.
    pub(crate) fn persist_terminal_capture(
        &self,
        db: &Connection,
        capture: TerminalCapture,
    ) -> Result<(), String> {
        if !db.is_autocommit() {
            return Err("root_tick_terminal_proof_requires_committed_bill".into());
        }
        let fact = capture.0?;
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        protect(&tx, || self.persist_terminal_fact_in(&tx, fact))?;
        tx.commit().map_err(|e| e.to_string())
    }
    // Explicit reconciliation can atomically append its original settlement
    // capture with the user's confirmation receipt. No historical backfill.
    pub(super) fn persist_terminal_capture_in(
        &self,
        tx: &Transaction<'_>,
        capture: TerminalCapture,
    ) -> Result<(), String> {
        self.persist_terminal_fact_in(tx, capture.0?)
    }
    fn persist_terminal_fact_in(&self, tx: &Transaction<'_>, fact: Value) -> Result<(), String> {
        let text = fact.to_string();
        if text.len() > 1024 * 1024 {
            return Err("root_tick_terminal_proof_too_large".into());
        }
        self.verify_terminal_fact(tx, &fact)?;
        let n=tx.execute("INSERT INTO agent_root_tick_terminal_proofs(call_id,root_run_id,lease_attempt_id,round,request_hash,basis_hash,phase,fact_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![self.call.id,self.call.owner.root,self.call.owner.id,self.call.round,self.call.request,self.basis,fact["phase"].as_str().unwrap(),text]).map_err(|e|e.to_string())?;
        if n != 1 {
            return Err("root_tick_terminal_proof_insert_unconfirmed".into());
        }
        self.verify_terminal_fact(tx, &fact)?;
        self.verify_original_terminal_proof(tx)
    }
    pub(crate) fn verify_original_terminal_proof(&self, db: &Connection) -> Result<(), String> {
        if self.request_fact["version"] == 1 {
            return Ok(());
        }
        if !matches!(self.request_fact["version"].as_i64(), Some(2 | 3)) {
            return Err("root_tick_original_input_conflict".into());
        }
        let rows:Vec<(String,String)>=db.prepare("SELECT phase,fact_json FROM agent_root_tick_terminal_proofs WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND round=?4 AND request_hash=?5 AND basis_hash=?6 ORDER BY phase").map_err(|e|e.to_string())?.query_map(params![self.call.id,self.call.owner.root,self.call.owner.id,self.call.round,self.call.request,self.basis],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        if rows.len() != 1 {
            return Err("root_tick_terminal_proof_missing".into());
        }
        let (phase, text) = &rows[0];
        let fact: Value =
            serde_json::from_str(text).map_err(|_| "root_tick_terminal_proof_invalid")?;
        let canonical = fact.to_string();
        if canonical != *text || fact["phase"] != *phase {
            return Err("root_tick_terminal_proof_invalid".into());
        }
        self.verify_terminal_fact(db, &fact)
    }
}
fn protect<T>(tx: &Transaction<'_>, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    tx.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Insert { table_name }
            if c.database_name == Some("main")
                && c.accessor.is_none()
                && table_name == "agent_root_tick_terminal_proofs" =>
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
