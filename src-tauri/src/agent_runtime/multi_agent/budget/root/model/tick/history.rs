//! Carry original physical local-step proofs into the final dispatch guard.
use super::{SavedDecision, Tick};
use rusqlite::{types::ValueRef, Connection, Params};
use serde_json::{json, Value};

impl Tick {
    pub(crate) fn local_parent_proof(
        &self,
        db: &Connection,
        saved: &SavedDecision,
        sequence: i64,
    ) -> Result<Value, String> {
        if saved.local_step.is_none() || self.published(db, saved)? != Some(sequence) {
            return Err("root_local_parent_not_published".into());
        }
        self.call.require_executable(db)?;
        let channel = self.verify_timeline(db, saved, sequence)?;
        capture(
            db,
            &self.call.owner.root,
            &self.call.owner.id,
            &self.call.id,
            self.call.round,
            sequence,
            channel.sequence,
        )
    }
    pub(super) fn verify_local_parents(&self, db: &Connection) -> Result<(), String> {
        let basis = &self.request_fact["basis"];
        let Some(parents) = basis.get("localParents") else {
            if self.call.owner.contract["root"]
                .get("localDeliberation")
                .is_some()
            {
                return Err("root_local_parent_proof_missing".into());
            }
            return Ok(());
        };
        let parents = parents
            .as_array()
            .ok_or("root_local_parent_proof_invalid")?;
        let local_round = basis["localRound"]
            .as_u64()
            .ok_or("root_local_parent_proof_invalid")?;
        let history = basis["localHistory"]
            .as_array()
            .ok_or("root_local_parent_proof_invalid")?;
        if local_round > 2
            || parents.len() != local_round as usize
            || history.len() != parents.len()
        {
            return Err("root_local_parent_proof_invalid".into());
        }
        let mut previous = 0;
        for proof in parents {
            let call = proof["callId"]
                .as_str()
                .ok_or("root_local_parent_proof_invalid")?;
            let round = proof["round"]
                .as_i64()
                .ok_or("root_local_parent_proof_invalid")?;
            let event = proof["eventSequence"]
                .as_i64()
                .ok_or("root_local_parent_proof_invalid")?;
            let channel = proof["chatSequence"]
                .as_i64()
                .ok_or("root_local_parent_proof_invalid")?;
            if round <= previous
                || round >= self.call.round
                || event <= 0
                || channel <= 0
                || proof
                    != &capture(
                        db,
                        &self.call.owner.root,
                        &self.call.owner.id,
                        call,
                        round,
                        event,
                        channel,
                    )?
            {
                return Err("root_local_original_parent_changed".into());
            }
            previous = round;
        }
        Ok(())
    }
}
fn capture(
    db: &Connection,
    root: &str,
    owner: &str,
    call: &str,
    round: i64,
    event: i64,
    chat: i64,
) -> Result<Value, String> {
    let scoped = rusqlite::params![call, root, owner, round];
    let tick=rows(db,"SELECT rowid,* FROM agent_root_tick_receipts WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND round=?4 ORDER BY phase",scoped)?;
    let model=rows(db,"SELECT rowid,* FROM agent_root_model_journal WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND round=?4 ORDER BY phase",scoped)?;
    let budget=rows(db,"SELECT rowid,* FROM agent_budget_entries WHERE source_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND assignment_id='' ORDER BY entry_id",rusqlite::params![call,root,owner])?;
    let timeline=rows(db,"SELECT rowid,* FROM agent_root_tick_timeline_receipts WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND model_event_sequence=?4 AND collaboration_sequence=?5",rusqlite::params![call,root,owner,event,chat])?;
    let event_rows = rows(
        db,
        "SELECT rowid,* FROM agent_events WHERE run_id=?1 AND sequence=?2",
        rusqlite::params![root, event],
    )?;
    let channel=rows(db,"SELECT rowid,* FROM agent_collaboration_events WHERE sequence=?1 AND entity_type='root_decision' AND event_type='root_decision'",[chat])?;
    if tick.as_array().is_none_or(|r| r.len() != 3)
        || model.as_array().is_none_or(|r| r.len() != 2)
        || budget.as_array().is_none_or(Vec::is_empty)
        || timeline.as_array().is_none_or(|r| r.len() != 1)
        || event_rows.as_array().is_none_or(|r| r.len() != 1)
        || channel.as_array().is_none_or(|r| r.len() != 1)
    {
        return Err("root_local_original_parent_missing".into());
    }
    let mut proof = json!({"version":1,"root":root,"owner":owner,"callId":call,"round":round,
        "eventSequence":event,"chatSequence":chat,"tick":tick,"model":model,
        "budget":budget,"timeline":timeline,"event":event_rows,"channel":channel});
    let protocol: i64 = db.query_row("SELECT json_extract(fact_json,'$.request.version') FROM agent_root_tick_receipts WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND round=?4 AND phase='request'",scoped,|r|r.get(0)).map_err(|e|e.to_string())?;
    if matches!(protocol, 2 | 3) {
        let terminal = rows(db,"SELECT rowid,* FROM agent_root_tick_terminal_proofs WHERE call_id=?1 AND root_run_id=?2 AND lease_attempt_id=?3 AND round=?4",scoped)?;
        if terminal.as_array().is_none_or(|r| r.len() != 1) {
            return Err("root_local_original_terminal_missing".into());
        }
        proof["terminalProof"] = terminal;
    } else if protocol != 1 {
        return Err("root_local_original_parent_invalid".into());
    }
    Ok(proof)
}
fn rows(db: &Connection, sql: &str, params: impl Params) -> Result<Value, String> {
    let mut statement = db.prepare(sql).map_err(|e| e.to_string())?;
    let count = statement.column_count();
    let mut query = statement.query(params).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    while let Some(row) = query.next().map_err(|e| e.to_string())? {
        let mut values = Vec::with_capacity(count);
        for index in 0..count {
            values.push(match row.get_ref(index).map_err(|e| e.to_string())? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => json!(n),
                ValueRef::Real(n) => json!(n),
                ValueRef::Text(t) => {
                    json!(std::str::from_utf8(t).map_err(|_| "root_local_parent_text_invalid")?)
                }
                ValueRef::Blob(_) => return Err("root_local_parent_type_invalid".into()),
            });
        }
        result.push(json!(values));
    }
    Ok(json!(result))
}
