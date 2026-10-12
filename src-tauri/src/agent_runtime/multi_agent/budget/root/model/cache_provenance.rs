//! Original transport metadata, never a dollar invoice or execution grant.
use super::RootModelCall;
use crate::agent_runtime::model::gateway::ModelResponse;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};
use crate::agent_runtime::web_mode::{ModeFact, VerifiedMode};

// Private construction is confined to the first received transaction.
pub(crate) struct FreshRootCacheCapture { message: Vec<u8>, original: VerifiedMode }
impl FreshRootCacheCapture {
    pub(crate) fn message_for_original(&self, fact: &ModeFact) -> Result<Vec<u8>, String> {
        if self.original.fact() != fact { return Err("root_cache_original_mode_changed".into()); }
        Ok(self.message.clone())
    }
}
fn capture_message(call: &RootModelCall, invoice: &Value, cached: bool, mode: &ModeFact) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&json!({"domain":"oviraptor.root-model-cache-capture.v1", "mode":mode,
        "rootRunId":call.owner.root,"leaseAttemptId":call.owner.id,"callId":call.id,
        "round":call.round,"requestHash":call.request,"received":invoice,"cachedUsageReported":cached})).map_err(|e|e.to_string())
}

impl RootModelCall {
    #[cfg(test)]
    pub(crate) fn verify_original_cache_for_test(&self, db: &Connection, response: &ModelResponse) -> Result<bool, String> {
        self.read_cache_provenance(db, &Self::received_fact(response))
    }

    pub(super) fn read_cache_provenance(&self, db: &Connection, invoice: &Value) -> Result<bool, String> {
        if db.is_autocommit() {
            let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.read_cache_provenance(&snapshot, invoice);
        }
        self.verify(db, "dispatch", &self.dispatch())?;
        self.verify(db, "received", invoice)?;
        let kind: Option<String> = db.query_row("SELECT type FROM main.sqlite_master
            WHERE name='agent_root_model_cache_provenance'", [], |r|r.get(0))
            .optional().map_err(|e|e.to_string())?;
        if kind.is_none() { return Ok(false); }
        if kind.as_deref() != Some("table") { return Err("root_cache_provenance_schema_conflict".into()); }
        let row = db.query_row("SELECT root_run_id,lease_attempt_id,round,request_hash,response_hash,cached_usage_reported
            FROM main.agent_root_model_cache_provenance WHERE call_id=?1", [&self.id], |r| Ok((
                r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,i64>(5)?)))
            .optional().map_err(|e|e.to_string())?;
        let Some((root,owner,round,request,response,cached)) = row else { return Ok(false); };
        verify_schema(db)?;
        if root != self.owner.root || owner != self.owner.id || round != self.round
            || request != self.request || invoice["responseHash"].as_str() != Some(response.as_str())
            || !matches!(cached,0|1) || (cached == 1 && invoice["usageReported"] != true) {
            return Err("root_cache_provenance_original_binding_conflict".into());
        }
        let kind: Option<String> = db.query_row("SELECT type FROM main.sqlite_master WHERE name='agent_root_model_cache_capture'", [], |r|r.get(0)).optional().map_err(|e|e.to_string())?;
        if kind.is_none() { return Ok(false); }
        if kind.as_deref() != Some("table") { return Err("root_cache_provenance_schema_conflict".into()); }
        let signature: Option<Vec<u8>> = db.query_row("SELECT signature FROM main.agent_root_model_cache_capture WHERE call_id=?1", [&self.id], |r|r.get(0)).optional().map_err(|e|e.to_string())?;
        let Some(signature) = signature else { return Ok(false); };
        verify_schema_source(db, include_str!("../../root_cache_capture_schema.sql"), "agent_root_model_cache_capture")?;
        let proof = crate::commands::original_root_capture_mode(db, &self.owner.root)?;
        proof.verify_root_cache(&capture_message(self, invoice, cached == 1, proof.fact())?, &signature)?;
        Ok(cached == 1)
    }
    pub(super) fn record_cache_provenance(
        &self,
        tx: &Transaction<'_>,
        response: &ModelResponse,
    ) -> Result<(), String> {
        let original = Self::received_fact(response);
        self.verify(tx, "received", &original)?;
        let usage = &response.usage;
        let valid = usage.input_tokens >= 0 && usage.output_tokens >= 0
            && usage.cached_input_tokens >= 0 && usage.cached_input_tokens <= usage.input_tokens
            && usage.input_tokens.checked_add(usage.output_tokens) == Some(usage.total_tokens)
            && usage.model_requests == 1;
        let cached = response.cached_usage_reported && response.usage_reported && valid;
        let written = tx.execute("INSERT INTO main.agent_root_model_cache_provenance
            (call_id,root_run_id,lease_attempt_id,round,request_hash,response_hash,cached_usage_reported)
            VALUES(?1,?2,?3,?4,?5,?6,?7)", params![self.id,self.owner.root,self.owner.id,
                self.round,self.request,original["responseHash"].as_str().ok_or("root_cache_response_hash_missing")?,cached])
            .map_err(|e| e.to_string())?;
        if written != 1 {
            return Err("root_cache_provenance_write_missing".into());
        }
        // Legacy/non-Web accounting has no authenticated Web mode key. Keep
        // its original numeric receipt; it cannot mint reported cache provenance.
        if crate::agent_runtime::web_mode::root::read(tx, &self.owner.root)?.is_none() {
            return self.verify(tx, "received", &original);
        }
        let original_mode = crate::commands::original_root_capture_mode(tx, &self.owner.root)?;
        let capture = FreshRootCacheCapture { message: capture_message(self, &original, cached, original_mode.fact())?, original: original_mode };
        let signature = capture.original.sign_fresh_root_cache(&capture)?;
        if tx.execute("INSERT INTO main.agent_root_model_cache_capture(call_id,signature) VALUES(?1,?2)", params![self.id,signature]).map_err(|e|e.to_string())? != 1 {
            return Err("root_cache_capture_write_missing".into());
        }
        self.verify(tx, "received", &original)
    }
}

fn verify_schema(db: &Connection) -> Result<(), String> {
    verify_schema_source(db, include_str!("../../root_cache_provenance_schema.sql"), "agent_root_model_cache_provenance")
}
fn verify_schema_source(db: &Connection, source: &str, table_name: &str) -> Result<(), String> {
    let mut pieces = source.split("CREATE TRIGGER IF NOT EXISTS ");
    let table = pieces.next().ok_or("root_cache_provenance_schema_conflict")?;
    let start = table.find("CREATE TABLE").ok_or("root_cache_provenance_schema_conflict")?;
    let mut objects = vec![(table_name.to_string(),"table",table[start..].replace("IF NOT EXISTS ",""))];
    for trigger in pieces {
        let name = trigger.split_whitespace().next().ok_or("root_cache_provenance_schema_conflict")?;
        objects.push((name.to_string(),"trigger",format!("CREATE TRIGGER {trigger}")));
    }
    let normalize = |sql: &str| sql.trim().trim_end_matches(';').chars().filter(|c| !c.is_ascii_whitespace()).collect::<String>();
    for (name,kind,expected) in objects {
        let original: String = db.query_row("SELECT sql FROM main.sqlite_master WHERE name=?1 AND type=?2",params![name,kind],|r|r.get(0))
            .map_err(|_| "root_cache_provenance_schema_conflict")?;
        if normalize(&original) != normalize(&expected) { return Err("root_cache_provenance_schema_conflict".into()); }
    }
    let count: i64 = db.query_row("SELECT count(*) FROM main.sqlite_master WHERE type='trigger' AND tbl_name=?1",[table_name],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count != 3 { return Err("root_cache_provenance_schema_conflict".into()); }
    Ok(())
}
