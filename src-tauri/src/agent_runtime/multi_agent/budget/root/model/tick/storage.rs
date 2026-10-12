//! Canonical append-only receipts; no retained assistant text.
use super::{SavedDecision, Tick};
use crate::agent_runtime::model::gateway::ModelResponse;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
impl Tick {
    pub(super) fn request_row(&self, db: &Connection) -> Result<Value, String> {
        db.query_row("SELECT rowid,created_at,fact_json FROM agent_root_tick_receipts
            WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND phase='request'",
            params![self.call.id,self.call.owner.root,self.call.owner.id],|r|Ok(json!({
                "rowid":r.get::<_,i64>(0)?,"created":r.get::<_,String>(1)?,"fact":r.get::<_,String>(2)?})))
            .map_err(|e|e.to_string())
    }
    pub(super) fn read(&self, db: &Connection, phase: &str) -> Result<Option<Value>, String> {
        let text: Option<String> = db
            .query_row(
                "SELECT fact_json FROM agent_root_tick_receipts
            WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND round=?4
            AND request_hash=?5 AND basis_hash=?6 AND phase=?7",
                params![
                    self.call.id,
                    self.call.owner.root,
                    self.call.owner.id,
                    self.call.round,
                    self.call.request,
                    self.basis,
                    phase
                ],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        text.map(|text| {
            let value: Value = serde_json::from_str(&text).map_err(|_| "root_tick_json_invalid")?;
            let canonical = value.to_string();
            if canonical != text {
                return Err("root_tick_json_noncanonical".into());
            }
            Ok(value)
        })
        .transpose()
    }
    pub(super) fn insert(
        &self,
        tx: &Transaction<'_>,
        phase: &str,
        value: &Value,
    ) -> Result<(), String> {
        let text = value.to_string();
        if text.len() > 1024 * 1024 {
            return Err("root_tick_receipt_too_large".into());
        }
        let changed=tx.execute("INSERT INTO agent_root_tick_receipts(call_id,root_run_id,
            lease_attempt_id,round,request_hash,basis_hash,phase,fact_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![self.call.id,self.call.owner.root,self.call.owner.id,self.call.round,
                self.call.request,self.basis,phase,text]).map_err(|e|e.to_string())?;
        if changed != 1 || self.read(tx, phase)? != Some(value.clone()) {
            return Err("root_tick_insert_unconfirmed".into());
        }
        Ok(())
    }
}
impl SavedDecision {
    pub(super) fn fact(&self) -> Value {
        let mut value = json!({"version":1,"summary":self.summary.as_json(),"invoice":self.invoice,
            "costProof":self.cost_proof,"requestProof":self.request_proof,"unknown":self.unknown});
        if let Some(step) = &self.local_step {
            value["localStep"] = step.as_json();
        }
        value
    }
    pub(super) fn financial_response(
        &self,
        db: &Connection,
        call: &super::super::RootModelCall,
    ) -> Result<ModelResponse, String> {
        use crate::agent_runtime::model::UsageDelta;
        let u = &self.invoice["usage"];
        let n = |key: &str| u[key].as_i64().ok_or("root_tick_usage_invalid".to_string());
        if self.invoice.as_object().is_none_or(|o| o.len() != 3)
            || self.invoice["responseHash"]
                .as_str()
                .is_none_or(|s| s.len() != 64)
            || u.as_object().is_none_or(|o| o.len() != 5)
        {
            return Err("root_tick_usage_invalid".into());
        }
        Ok(ModelResponse {
            cached_usage_reported: call.read_cache_provenance(db, &self.invoice)?,
            text: String::new(),
            tool_calls: vec![],
            finish_reason: "stop".into(),
            usage_reported: self.invoice["usageReported"]
                .as_bool()
                .ok_or("root_tick_usage_invalid")?,
            usage: UsageDelta {
                input_tokens: n("inputTokens")?,
                cached_input_tokens: n("cachedInputTokens")?,
                output_tokens: n("outputTokens")?,
                total_tokens: n("totalTokens")?,
                model_requests: n("modelRequests")?,
            },
        })
    }
}
