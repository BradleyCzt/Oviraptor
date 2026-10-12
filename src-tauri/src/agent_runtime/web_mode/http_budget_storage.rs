//! Exact immutable HTTP-cost storage; readers never create tables or receipts.
//! Initialization must install SCHEMA_SQL through the normal schema path only.
//! The fresh producer runs in its private headers writer's transaction, after
//! prior writer authorizer guards are dropped; do not call on shared readers.
use super::http_budget_receipt::FreshHttpBudgetReceipt;
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

pub(crate) const SCHEMA_SQL: &str = include_str!("http_budget_receipt.sql");
const TABLE: &str = "agent_http_budget_receipts";

pub(crate) struct VerifiedHttpBudgetReceipt {
    value: Value,
    receipt_id: String,
}
impl VerifiedHttpBudgetReceipt {
    pub(crate) fn fact(&self) -> &Value {
        &self.value
    }
    pub(crate) fn receipt_id(&self) -> &str {
        &self.receipt_id
    }
}

pub(crate) fn insert_fresh_on(
    db: &Connection,
    actor: &CoordinatorLease,
    fresh: &FreshHttpBudgetReceipt,
) -> Result<VerifiedHttpBudgetReceipt, String> {
    require_snapshot(db)?;
    verify_schema_on(db)?;
    let (value, signature) = fresh.sealed()?;
    crate::commands::http_budget_receipt::verify_saved_on(db, actor, &value, &signature)?;
    let source = text(&value, "sourceId")?;
    let raw = value.to_string();
    let hash = crate::agent_runtime::store::stable_hash(&raw);
    if let Some(saved) = read_original_on(db, actor, source)? {
        if saved.value != value || saved.receipt_id != hash {
            return Err("http_budget_receipt_existing_fact_changed".into());
        }
        // Same actual fresh fact may reenter locally, but never INSERT OR REPLACE.
        return Ok(saved);
    }
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    db.authorizer(Some(|a: AuthContext<'_>| match a.action {
        AuthAction::Insert { table_name }
            if a.database_name == Some("main") && table_name == TABLE && a.accessor.is_none() =>
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
    .map_err(|_| "http_budget_receipt_write_guard_unavailable")?;
    let guard = ReceiptWriteGuard(db);
    let inserted = db
        .execute(
            "INSERT INTO main.agent_http_budget_receipts(receipt_id,root_run_id,source_id,
         scan_id,attempt_number,target_url,run_id,response_status,fact_json,fact_hash,signature)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?1,?10)",
            params![
                hash,
                actor.root_run_id,
                source,
                actor.scan_id,
                actor.attempt_number,
                actor.target_key,
                value["claim"]["runId"]
                    .as_str()
                    .ok_or("http_budget_receipt_claim_invalid")?,
                value["status"]
                    .as_i64()
                    .ok_or("http_budget_receipt_status_invalid")?,
                raw,
                signature
            ],
        )
        .map_err(|_| "http_budget_receipt_insert_failed")?;
    if inserted != 1 {
        return Err("http_budget_receipt_insert_unconfirmed".into());
    }
    // Validate after all INSERT triggers, while the restrictive writer guard lives.
    let saved =
        read_original_on(db, actor, source)?.ok_or("http_budget_receipt_insert_unconfirmed")?;
    if saved.value != value || saved.receipt_id != hash {
        return Err("http_budget_receipt_insert_unconfirmed".into());
    }
    drop(guard);
    Ok(saved)
}

// Read-only, snapshot-bound, exact source query. Missing stays missing.
pub(crate) fn read_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    source: &str,
) -> Result<Option<VerifiedHttpBudgetReceipt>, String> {
    require_snapshot(db)?;
    verify_schema_on(db)?;
    type StoredRow = (
        String,
        String,
        String,
        String,
        i64,
        String,
        String,
        i64,
        String,
        String,
        Vec<u8>,
    );
    let row: Option<StoredRow> = db
        .query_row(
            "SELECT receipt_id,root_run_id,source_id,scan_id,attempt_number,target_url,
         run_id,response_status,fact_json,fact_hash,signature FROM main.agent_http_budget_receipts
         WHERE root_run_id=?1 AND source_id=?2",
            params![actor.root_run_id, source],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                ))
            },
        )
        .optional()
        .map_err(|_| "http_budget_receipt_read_failed")?;
    let Some((id, root, stored_source, scan, attempt, target, run, status, raw, hash, signature)) =
        row
    else {
        return Ok(None);
    };
    let value: Value =
        serde_json::from_str(&raw).map_err(|_| "http_budget_receipt_json_invalid")?;
    let canonical = value.to_string();
    if canonical != raw
        || hash != crate::agent_runtime::store::stable_hash(&raw)
        || id != hash
        || signature.len() != 32
        || root != actor.root_run_id
        || stored_source != source
        || scan != actor.scan_id
        || attempt != actor.attempt_number
        || target != actor.target_key
        || value["rootRunId"] != root
        || value["sourceId"] != stored_source
        || value["scanId"] != scan
        || value["attemptNumber"] != attempt
        || value["targetUrl"] != target
        || value["claim"]["runId"] != run
        || value["status"] != status
    {
        return Err("http_budget_receipt_stored_binding_changed".into());
    }
    crate::commands::http_budget_receipt::verify_saved_on(db, actor, &value, &signature)?;
    Ok(Some(VerifiedHttpBudgetReceipt {
        value,
        receipt_id: id,
    }))
}

// Match the full table/index/trigger inventory, including unexpected triggers.
// Only the disposable in-memory reference schema is created by this verifier.
pub(crate) fn verify_schema_on(db: &Connection) -> Result<(), String> {
    let expected =
        Connection::open_in_memory().map_err(|_| "http_budget_receipt_schema_unavailable")?;
    expected
        .execute_batch(SCHEMA_SQL)
        .map_err(|_| "http_budget_receipt_schema_unavailable")?;
    let shadow: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE tbl_name=?1 OR name IN
         ('agent_http_budget_receipts','http_budget_receipt_no_update',
          'http_budget_receipt_no_delete','http_budget_receipt_no_replace'))",
            [TABLE],
            |r| r.get(0),
        )
        .map_err(|_| "http_budget_receipt_schema_unavailable")?;
    if shadow || schema_rows(db)? != schema_rows(&expected)? {
        return Err("http_budget_receipt_schema_conflict".into());
    }
    Ok(())
}

type SchemaRow = (String, String, String, Option<String>);
fn schema_rows(db: &Connection) -> Result<Vec<SchemaRow>, String> {
    let mut query = db
        .prepare(
            "SELECT type,name,tbl_name,sql FROM main.sqlite_master WHERE tbl_name=?1 OR name IN
         ('agent_http_budget_receipts','http_budget_receipt_no_update',
          'http_budget_receipt_no_delete','http_budget_receipt_no_replace') ORDER BY type,name",
        )
        .map_err(|_| "http_budget_receipt_schema_unavailable")?;
    let rows = query
        .query_map([TABLE], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .map_err(|_| "http_budget_receipt_schema_unavailable")?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| "http_budget_receipt_schema_unavailable")?;
    Ok(rows)
}
fn require_snapshot(db: &Connection) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("http_budget_receipt_snapshot_required".into());
    }
    Ok(())
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "http_budget_receipt_fact_invalid".into())
}
struct ReceiptWriteGuard<'a>(&'a Connection);
impl Drop for ReceiptWriteGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
    }
}
