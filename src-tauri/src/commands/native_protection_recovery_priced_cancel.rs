// Explicit revocation of a pending priced approval. No draft or executable
// priced birth exists yet; future birth must verify this original revocation.
const PRICED_RECOVERY_CANCEL_CONSENT: &str =
    "revoke_exact_pending_priced_approval_preserve_original_history";
fn native_priced_recovery_cancellation_id(approved: &JsonValue) -> String {
    crate::agent_runtime::store::stable_hash(&format!(
        "oviraptor.priced-recovery-cancel.v1:{}",
        approved["approvalId"].as_str().unwrap_or("")
    ))
}
fn native_priced_recovery_cancellation_value(approved: &JsonValue) -> JsonValue {
    json!({"schemaVersion":1,"purpose":"cancel_unconsumed_priced_recovery_approval",
        "cancellationId":native_priced_recovery_cancellation_id(approved),
        "requestId":approved["requestId"],"approvalId":approved["approvalId"],
        "approvalHash":crate::agent_runtime::store::stable_hash(&approved.to_string()),
        "originalModeReceiptId":approved["originalModeReceiptId"],
        "reason":"user_cancelled_before_priced_fresh_birth","authorityRevoked":true,
        "executionAllowed":false,"targetRequestsGranted":0,"oldStopRetained":true,"oldCostsRetained":true})
}
fn native_priced_recovery_cancel_schema_on(db: &rusqlite::Connection) -> Result<(), String> {
    let expected = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!("native_protection_recovery_priced_schema.sql"))
        .map_err(|e| e.to_string())?;
    let read =
        |conn: &rusqlite::Connection| -> Result<Vec<(String, String, String, String)>, String> {
            let mut q=conn.prepare("SELECT type,name,tbl_name,coalesce(sql,'') FROM main.sqlite_master WHERE tbl_name='native_protection_recovery_priced_cancellations' ORDER BY type,name").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        };
    let shadow: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name='native_protection_recovery_priced_cancellations' OR tbl_name='native_protection_recovery_priced_cancellations')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if shadow || read(db)? != read(&expected)? {
        return Err("protection_recovery_priced_cancellation_schema_changed".into());
    }
    Ok(())
}
fn native_priced_recovery_cancel_saved_on(
    db: &rusqlite::Connection,
    request: &str,
) -> Result<Option<JsonValue>, String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    native_priced_recovery_cancel_schema_on(db)?;
    let row: Option<(String,String,String,Vec<u8>)>=db.query_row("SELECT cancellation_id,approval_id,cancellation_json,signature FROM main.native_protection_recovery_priced_cancellations WHERE request_id=?1",[request],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, approval_id, text, signature)) = row else {
        return Ok(None);
    };
    let approved = native_priced_recovery_saved_on(db, request)?
        .ok_or("protection_recovery_priced_cancel_approval_missing")?;
    let value: JsonValue = serde_json::from_str(&text)
        .map_err(|_| "protection_recovery_priced_cancellation_corrupt")?;
    let canonical = value.to_string();
    if value != native_priced_recovery_cancellation_value(&approved)
        || canonical != text
        || value["cancellationId"] != id
        || value["approvalId"] != approval_id
    {
        return Err("protection_recovery_priced_cancellation_corrupt".into());
    }
    recovery_approval_signature::verify_priced_cancellation_on(db, &approved, &value, &signature)?;
    Ok(Some(value))
}
impl<'a> NativePricedRecoveryWriter<'a> {
    fn install_cancellation(db: &'a rusqlite::Connection) -> Result<Self, String> {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        native_priced_recovery_cancel_schema_on(db)?;
        db.authorizer(Some(|c: AuthContext<'_>| match c.action {
            AuthAction::Insert {
                table_name: "native_protection_recovery_priced_cancellations",
            } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
            AuthAction::Pragma {
                pragma_name: "database_list",
                ..
            } => Authorization::Allow,
            AuthAction::Read { .. }
            | AuthAction::Select
            | AuthAction::Function { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. }
            | AuthAction::Recursive => Authorization::Allow,
            _ => Authorization::Deny,
        }))
        .map_err(|e| e.to_string())?;
        Ok(Self(db))
    }
}
fn native_priced_recovery_cancel_on(
    path: &Path,
    request: &str,
    approval_id: &str,
    consent: &str,
) -> Result<JsonValue, String> {
    if uuid::Uuid::parse_str(request).is_err() || consent != PRICED_RECOVERY_CANCEL_CONSENT {
        return Err("protection_recovery_priced_explicit_revocation_required".into());
    }
    let mut db = native_protection_recovery_open(path, true)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let approved = native_priced_recovery_saved_on(&tx, request)?
        .ok_or("protection_recovery_priced_cancel_approval_missing")?;
    if approved["approvalId"] != approval_id {
        return Err("protection_recovery_priced_cancel_approval_changed".into());
    }
    if let Some(saved) = native_priced_recovery_cancel_saved_on(&tx, request)? {
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(saved);
    }
    let envelope =
        recovery_approval_signature::FreshPricedRecoveryCancellation::cancel_in(&tx, &approved)?;
    let (value, signature) = envelope.sealed()?;
    let writer = NativePricedRecoveryWriter::install_cancellation(&tx)?;
    let n=tx.execute("INSERT INTO main.native_protection_recovery_priced_cancellations(cancellation_id,request_id,approval_id,cancellation_json,signature) VALUES(?1,?2,?3,?4,?5)",params![value["cancellationId"].as_str(),request,approval_id,value.to_string(),signature]).map_err(|e|e.to_string())?;
    if n != 1 || native_priced_recovery_cancel_saved_on(&tx, request)?.as_ref() != Some(&value) {
        return Err("protection_recovery_priced_cancellation_not_saved".into());
    }
    drop(writer);
    tx.commit().map_err(|e| e.to_string())?;
    Ok(value)
}
#[tauri::command]
pub fn cancel_native_priced_protection_recovery_approval(
    state: State<AppState>,
    request_id: String,
    approval_id: String,
    consent: String,
) -> Result<JsonValue, String> {
    native_priced_recovery_cancel_on(&state.db_path, &request_id, &approval_id, &consent)
}
#[tauri::command]
pub fn get_native_priced_protection_recovery_cancellation(
    state: State<AppState>,
    request_id: String,
) -> Result<Option<JsonValue>, String> {
    let db = native_protection_recovery_open(&state.db_path, false)?;
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let result = native_priced_recovery_cancel_saved_on(&tx, &request_id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
