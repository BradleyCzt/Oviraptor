fn native_protection_recovery_cancellation_schema_on(
    db: &rusqlite::Connection,
) -> Result<(), String> {
    let expected = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!("native_protection_recovery_schema.sql"))
        .map_err(|e| e.to_string())?;
    let read =
        |db: &rusqlite::Connection| -> Result<Vec<(String, String, String, String)>, String> {
            let mut q=db.prepare("SELECT type,name,tbl_name,coalesce(sql,'') FROM main.sqlite_master WHERE tbl_name='native_protection_recovery_cancellations' ORDER BY type,name").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        };
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name='native_protection_recovery_cancellations' OR tbl_name='native_protection_recovery_cancellations')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if shadow || read(db)? != read(&expected)? {
        return Err("protection_recovery_cancellation_schema_changed".into());
    }
    Ok(())
}
type NativeProtectionRecoveryCancellationStoredRow =
    (String, String, String, String, String, Vec<u8>);
fn native_protection_recovery_cancellation_saved_on(
    db: &rusqlite::Connection,
    scan: &str,
) -> Result<Option<JsonValue>, String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    native_protection_recovery_cancellation_schema_on(db)?;
    let row:Option<NativeProtectionRecoveryCancellationStoredRow>=db.query_row("SELECT cancellation_id,approval_id,request_id,draft_hash,cancellation_json,signature FROM main.native_protection_recovery_cancellations WHERE scan_id=?1",[scan],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, approval, request, hash, text, signature)) = row else {
        return Ok(None);
    };
    let value: JsonValue =
        serde_json::from_str(&text).map_err(|_| "protection_recovery_cancellation_corrupt")?;
    let canonical = serde_json::to_string(&value).map_err(|e| e.to_string())?;
    let draft = native_protection_recovery_draft_saved_on(db, &request)?
        .ok_or("protection_recovery_draft_missing")?;
    if canonical != text
        || value["schemaVersion"] != 1
        || value["purpose"] != "cancel_unconsumed_protection_recovery_draft"
        || value["scanId"] != scan
        || value["cancellationId"] != id
        || Uuid::parse_str(&id).is_err()
        || value["approvalId"] != approval
        || value["requestId"] != request
        || value["draftHash"] != hash
        || value["draftId"] != draft["draftId"]
        || draft["scanId"] != scan
        || draft["approval"]["approvalId"] != approval
        || crate::agent_runtime::store::stable_hash(&draft.to_string()) != hash
        || value["reason"] != "user_cancelled_before_fresh_birth"
        || value["authorityRevoked"] != true
        || value["executionAllowed"] != false
        || value["targetRequestsGranted"] != 0
        || value["oldStopRetained"] != true
        || value["oldCostsRetained"] != true
    {
        return Err("protection_recovery_cancellation_binding_changed".into());
    }
    recovery_approval_signature::verify_saved_cancellation_on(db, &draft, &value, &signature)?;
    Ok(Some(value))
}
fn native_protection_recovery_cancelled_projection_on(
    db: &rusqlite::Connection,
    scan: &str,
) -> Result<(), String> {
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.sentinel_scans WHERE id=?1 AND status='cancelled' AND attempt_count=0 AND task_path='') AND NOT EXISTS(SELECT 1 FROM main.sentinel_targets WHERE scan_id=?1 AND status<>'cancelled') AND NOT EXISTS(SELECT 1 FROM main.sentinel_scan_attempts WHERE scan_id=?1) AND NOT EXISTS(SELECT 1 FROM main.agent_runs WHERE scan_id=?1) AND NOT EXISTS(SELECT 1 FROM main.native_branch_dispatches WHERE scan_id=?1)",[scan],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {
        return Err("protection_recovery_cancellation_projection_changed".into());
    }
    Ok(())
}
struct NativeProtectionRecoveryCancellationWriter<'a>(&'a rusqlite::Connection);
impl<'a> NativeProtectionRecoveryCancellationWriter<'a> {
    fn install(db: &'a rusqlite::Connection) -> Result<Self, String> {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        native_protection_recovery_cancellation_schema_on(db)?;
        db.authorizer(Some(|c: AuthContext<'_>| match c.action {
            AuthAction::Insert {
                table_name: "native_protection_recovery_cancellations",
            } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
            AuthAction::Update {
                table_name: "sentinel_scans",
                column_name,
            } if c.database_name == Some("main")
                && c.accessor.is_none()
                && matches!(column_name, "status" | "current_checkpoint" | "updated_at") =>
            {
                Authorization::Allow
            }
            AuthAction::Update {
                table_name: "sentinel_targets",
                column_name,
            } if c.database_name == Some("main")
                && c.accessor.is_none()
                && matches!(column_name, "status" | "updated_at") =>
            {
                Authorization::Allow
            }
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
impl Drop for NativeProtectionRecoveryCancellationWriter<'_> {
    fn drop(&mut self) {
        use rusqlite::hooks::{AuthContext, Authorization};
        let _ = self
            .0
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}
// Caller owns the new scan's lifecycle mutex and its Immediate transaction.
fn native_protection_recovery_cancel_draft_in(
    db: &rusqlite::Connection,
    scan: &str,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    native_protection_recovery_cancellation_schema_on(db)?;
    if native_protection_recovery_cancellation_saved_on(db, scan)?.is_some() {
        native_protection_recovery_cancelled_projection_on(db, scan)?;
        return Ok(());
    }
    let request: String = db
        .query_row(
            "SELECT request_id FROM main.native_protection_recovery_drafts WHERE scan_id=?1",
            [scan],
            |r| r.get(0),
        )
        .map_err(|_| "protection_recovery_draft_missing")?;
    let draft = native_protection_recovery_draft_saved_on(db, &request)?
        .ok_or("protection_recovery_draft_missing")?;
    native_protection_recovery_draft_pristine_scope_on(db, &draft)?;
    let fresh =
        recovery_approval_signature::FreshProtectionRecoveryCancellation::cancel_in(db, &draft)?;
    let (value, signature) = fresh.sealed()?;
    let writer = NativeProtectionRecoveryCancellationWriter::install(db)?;
    let n=db.execute("INSERT INTO main.native_protection_recovery_cancellations(scan_id,cancellation_id,approval_id,request_id,draft_hash,cancellation_json,signature) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![scan,value["cancellationId"].as_str(),value["approvalId"].as_str(),request,value["draftHash"].as_str(),value.to_string(),signature]).map_err(|e|e.to_string())?;
    if n!=1 || db.execute("UPDATE main.sentinel_scans SET status='cancelled',current_checkpoint='已取消：恢复批准已撤销；原停机与费用保留',updated_at=datetime('now','localtime') WHERE id=?1 AND status='draft' AND attempt_count=0 AND task_path=''",[scan]).map_err(|e|e.to_string())?!=1 {
        return Err("protection_recovery_cancel_state_changed".into());
    }
    let n=db.execute("UPDATE main.sentinel_targets SET status='cancelled',updated_at=datetime('now','localtime') WHERE scan_id=?1 AND status='queued'",[scan]).map_err(|e|e.to_string())?;
    if n != draft["approval"]["proposal"]["newScope"]["urls"]
        .as_array()
        .ok_or("protection_recovery_draft_corrupt")?
        .len()
        || native_protection_recovery_cancellation_saved_on(db, scan)?.as_ref() != Some(&value)
        || native_protection_recovery_draft_saved_on(db, &request)?.as_ref() != Some(&draft)
    {
        return Err("protection_recovery_cancel_not_saved".into());
    }
    native_protection_recovery_cancelled_projection_on(db, scan)?;
    drop(writer);
    Ok(())
}
// A client can recover a lost cancellation ACK using the UUID it persisted
// before approval; finding this record never grants or renews authority.
fn native_protection_recovery_cancellation_by_request_on(
    db: &rusqlite::Connection,
    request: &str,
) -> Result<Option<JsonValue>, String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    if Uuid::parse_str(request).is_err() {
        return Err("protection_recovery_request_invalid".into());
    }
    native_protection_recovery_cancellation_schema_on(db)?;
    let scan: Option<String> = db
        .query_row(
            "SELECT scan_id FROM main.native_protection_recovery_cancellations WHERE request_id=?1",
            [request],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(scan) = scan else {
        return Ok(None);
    };
    let value = native_protection_recovery_cancellation_saved_on(db, &scan)?
        .ok_or("protection_recovery_cancellation_missing")?;
    if value["requestId"] != request {
        return Err("protection_recovery_cancellation_binding_changed".into());
    }
    Ok(Some(value))
}

#[tauri::command]
pub fn get_native_protection_recovery_cancellation_by_request(
    state: State<AppState>,
    request_id: String,
) -> Result<Option<JsonValue>, String> {
    let db = native_protection_recovery_open(&state.db_path, false)?;
    let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let value = native_protection_recovery_cancellation_by_request_on(&snapshot, &request_id)?;
    snapshot.commit().map_err(|e| e.to_string())?;
    Ok(value)
}

#[tauri::command]
pub fn get_native_protection_recovery_cancellation(
    state: State<AppState>,
    scan_id: String,
) -> Result<Option<JsonValue>, String> {
    let db = native_protection_recovery_open(&state.db_path, false)?;
    let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let value = native_protection_recovery_cancellation_saved_on(&snapshot, &scan_id)?;
    snapshot.commit().map_err(|e| e.to_string())?;
    Ok(value)
}
