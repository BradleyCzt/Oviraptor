use super::Fact;
use rusqlite::{types::Value, Connection, Params};

pub(super) fn rows(db: &Connection, sql: &str, params: impl Params) -> Result<String, String> {
    let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
    let columns = q
        .column_names()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();
    let values = q
        .query_map(params, |r| {
            (0..columns.len())
                .map(|i| r.get::<_, Value>(i))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(crate::agent_runtime::store::stable_hash(&format!(
        "{columns:?}:{values:?}"
    )))
}
pub(super) fn worker(db: &Connection, id: &str) -> Result<String, String> {
    rows(
        db,
        "SELECT rowid,* FROM agent_assignment_attempts WHERE id=?1",
        [id],
    )
}
pub(super) fn caps(db: &Connection, id: &str) -> Result<String, String> {
    rows(
        db,
        "SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 ORDER BY rowid",
        [id],
    )
}
pub(super) fn context(db: &Connection, root: &str, assignment: &str) -> Result<String, String> {
    let root_row=rows(db,"SELECT rowid,id,scan_id,attempt_number,target_url,backend,role,parent_run_id,root_run_id,assignment_id,orchestration_policy,plan_hash,plan_json,created_at FROM agent_runs WHERE id=?1",[root])?;
    let control = rows(
        db,
        "SELECT rowid,* FROM agent_root_budget_attempts WHERE root_run_id=?1",
        [root],
    )?;
    let grant=rows(db,"SELECT rowid,id,coordinator_run_id,child_run_id,role,lane,target_key,dedup_key,trigger_code,task_slice_json,contract_keys_json,identity_handles_json,capability_lease_json,lease_epoch,fencing_token,created_at FROM agent_assignments WHERE id=?1",[assignment])?;
    let child = rows(db, "SELECT c.rowid,c.id,c.scan_id,c.attempt_number,c.target_url,c.backend,c.role,c.parent_run_id,c.root_run_id,c.assignment_id,c.lane,c.orchestration_policy,c.plan_hash,c.plan_json,c.created_at FROM agent_runs c JOIN agent_assignments a ON a.child_run_id=c.id WHERE a.id=?1", [assignment])?;
    Ok(crate::agent_runtime::store::stable_hash(&format!(
        "{root_row}:{control}:{grant}:{child}"
    )))
}
pub(super) fn events(db: &Connection, f: &Fact) -> Result<String, String> {
    rows(db,"SELECT rowid,* FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2 AND sequence>?3 AND sequence<=?4 AND ((entity_type='assignment' AND entity_id=?5) OR (entity_type='agent_run' AND entity_id=?6)) ORDER BY sequence",rusqlite::params![f.scan,f.attempt,f.event_floor,f.event_ceiling,f.assignment,f.child])
}
fn schema(db: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut q=db.prepare("SELECT name,sql FROM sqlite_master WHERE name IN ('agent_worker_expiry_receipts','worker_expiry_receipt_no_update','worker_expiry_receipt_no_delete','worker_expiry_receipt_no_replace','agent_worker_expiry_issuance_seals','worker_expiry_seal_no_update','worker_expiry_seal_no_delete','worker_expiry_seal_no_replace') ORDER BY name").map_err(|e|e.to_string())?;
    let rows = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}
pub(super) fn verify_schema(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!("schema.sql"))
        .map_err(|e| e.to_string())?;
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name IN ('agent_worker_expiry_receipts','worker_expiry_receipt_no_update','worker_expiry_receipt_no_delete','worker_expiry_receipt_no_replace','agent_worker_expiry_issuance_seals','worker_expiry_seal_no_update','worker_expiry_seal_no_delete','worker_expiry_seal_no_replace'))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if shadow || schema(db)? != schema(&expected)? {
        return Err("worker_expiry_receipt_schema_conflict".into());
    }
    Ok(())
}
