fn native_protection_recovery_open(
    path: &Path,
    write: bool,
) -> Result<rusqlite::Connection, String> {
    let flag = if write {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let db = rusqlite::Connection::open_with_flags(
        path,
        flag | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    if write {
        db.pragma_update(None, "synchronous", "FULL")
            .map_err(|e| e.to_string())?;
    }
    Ok(db)
}

fn native_protection_recovery_schema_on(db: &rusqlite::Connection) -> Result<(), String> {
    let expected = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!("native_protection_recovery_schema.sql"))
        .map_err(|e| e.to_string())?;
    let sql = "SELECT type,name,tbl_name,coalesce(sql,'') FROM main.sqlite_master
        WHERE tbl_name='native_protection_recovery_approvals' ORDER BY type,name";
    let read =
        |db: &rusqlite::Connection| -> Result<Vec<(String, String, String, String)>, String> {
            let mut q = db.prepare(sql).map_err(|e| e.to_string())?;
            let rows = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        };
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master
        WHERE name='native_protection_recovery_approvals' OR tbl_name='native_protection_recovery_approvals')",[],|r|r.get(0))
        .map_err(|e|e.to_string())?;
    if shadow || read(db)? != read(&expected)? {
        return Err("protection_recovery_approval_schema_changed".into());
    }
    Ok(())
}

struct NativeProtectionRecoveryApprovalWriter<'a>(&'a rusqlite::Connection);
impl<'a> NativeProtectionRecoveryApprovalWriter<'a> {
    fn install(db: &'a rusqlite::Connection) -> Result<Self, String> {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        native_protection_recovery_schema_on(db)?;
        db.authorizer(Some(|c: AuthContext<'_>| match c.action {
            AuthAction::Insert {
                table_name: "native_protection_recovery_approvals",
            } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
            // Physical-path verification uses only this metadata pragma.
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
impl Drop for NativeProtectionRecoveryApprovalWriter<'_> {
    fn drop(&mut self) {
        use rusqlite::hooks::{AuthContext, Authorization};
        let _ = self
            .0
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}

type NativeRecoveryApprovalStoredRow = (
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    Vec<u8>,
);

fn native_protection_recovery_saved_on(
    db: &rusqlite::Connection,
    request: &str,
) -> Result<Option<JsonValue>, String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    native_protection_recovery_schema_on(db)?;
    let row:Option<NativeRecoveryApprovalStoredRow> = db.query_row(
        "SELECT approval_id,request_id,source_scan_id,source_attempt_number,source_root_run_id,
         protection_receipt_id,proposal_hash,approval_json,signature FROM main.native_protection_recovery_approvals WHERE request_id=?1",
        [request],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))
    ).optional().map_err(|e|e.to_string())?;
    let Some((id, request, scan, attempt, root, receipt, hash, text, signature)) = row else {
        return Ok(None);
    };
    let fact: JsonValue =
        serde_json::from_str(&text).map_err(|_| "protection_recovery_approval_corrupt")?;
    let mut proposal = fact["proposal"].clone();
    let original_hash = proposal
        .as_object_mut()
        .ok_or("protection_recovery_approval_corrupt")?
        .remove("proposalHash")
        .ok_or("protection_recovery_approval_corrupt")?;
    let selection = &proposal["selection"];
    // Compare canonical JSON bytes, not Value's string-value PartialEq.
    let canonical =
        serde_json::to_string(&fact).map_err(|_| "protection_recovery_approval_corrupt")?;
    if canonical != text
        || fact["schemaVersion"] != 1
        || fact["purpose"] != "explicit_new_target_work_approval"
        || fact["approvalId"] != id
        || fact["requestId"] != request
        || fact["consent"] != "approve_exact_new_scope_preserve_old_stop_and_costs"
        || fact["status"] != "approved_pending_fresh_birth"
        || fact["executionAllowed"] != false
        || fact["targetRequestsGranted"] != 0
        || fact["oldScopeRenewed"] != false
        || selection["scanId"] != scan
        || selection["attemptNumber"] != attempt
        || selection["rootRunId"] != root
        || selection["protectionReceiptId"] != receipt
        || original_hash != hash
        || fact["proposalHash"] != hash
        || crate::agent_runtime::store::stable_hash(&proposal.to_string()) != hash
        || native_protection_recovery_approval_id(&hash) != id
        || uuid::Uuid::parse_str(&request).is_err()
    {
        return Err("protection_recovery_approval_binding_changed".into());
    }
    recovery_approval_signature::verify_saved_on(db, &fact, &signature)?;
    Ok(Some(fact))
}
fn native_protection_recovery_approval_id(hash: &str) -> String {
    crate::agent_runtime::store::stable_hash(&format!(
        "oviraptor.explicit-protection-recovery-approval.v1:{hash}"
    ))
}

fn native_protection_recovery_approve_on(
    db_path: &Path,
    selection: &NativeProtectionRecoverySelection,
    scope: &NativeProtectionRecoveryScope,
    request: &str,
    proposal_hash: &str,
    consent: &str,
) -> Result<JsonValue, String> {
    if uuid::Uuid::parse_str(request).is_err()
        || proposal_hash.len() != 64
        || !proposal_hash.bytes().all(|b| b.is_ascii_hexdigit())
        || consent != "approve_exact_new_scope_preserve_old_stop_and_costs"
    {
        return Err("protection_recovery_explicit_approval_required".into());
    }
    let _approval_queue = crate::agent_runtime::execution_owner::queue_recovery_approval(db_path)?;
    let mut db = native_protection_recovery_open(db_path, true)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_protection_recovery_schema_on(&tx)?;
    if let Some(saved) = native_protection_recovery_saved_on(&tx, request)? {
        if saved["proposalHash"] != proposal_hash
            || saved["proposal"]["selection"] != json!(selection)
            || saved["proposal"]["newScope"] != json!(scope)
        {
            return Err("protection_recovery_approval_conflict".into());
        }
        // A response-loss query returns the original approval only. It does
        // not claim that a now-expired or changed choice can execute today.
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(saved);
    }
    let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM main.native_protection_recovery_approvals
        WHERE source_scan_id=?1 AND source_root_run_id=?2 AND protection_receipt_id=?3 OR proposal_hash=?4)",
        params![selection.scan_id,selection.root_run_id,selection.protection_receipt_id,proposal_hash],|r|r.get(0))
        .map_err(|e|e.to_string())?;
    if duplicate {
        return Err("protection_recovery_approval_conflict".into());
    }
    let proof = native_protection_recovery_proposal_in(&tx, db_path, selection, scope)?;
    if proof.fact["proposalHash"] != proposal_hash {
        return Err("protection_recovery_proposal_changed".into());
    }
    let approved = recovery_approval_signature::FreshProtectionRecoveryApproval::approve_in(
        &tx, db_path, &proof, selection, scope, request, consent,
    )?;
    let (value, signature) = approved.sealed()?;
    let id = native_protection_recovery_approval_id(proposal_hash);
    let writer = NativeProtectionRecoveryApprovalWriter::install(&tx)?;
    let n=tx.execute("INSERT INTO main.native_protection_recovery_approvals(
        approval_id,request_id,source_scan_id,source_attempt_number,source_root_run_id,
        protection_receipt_id,proposal_hash,approval_json,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![id,request,selection.scan_id,selection.attempt_number,selection.root_run_id,
            selection.protection_receipt_id,proposal_hash,value.to_string(),signature]).map_err(|e|e.to_string())?;
    if n != 1 || native_protection_recovery_saved_on(&tx, request)?.as_ref() != Some(&value) {
        return Err("protection_recovery_approval_not_saved".into());
    }
    proof.verify_on(&tx, db_path, selection, scope)?;
    drop(writer);
    tx.commit().map_err(|e| e.to_string())?;
    drop(proof);
    Ok(value)
}

#[tauri::command]
pub fn approve_native_protection_recovery(
    state: State<AppState>,
    selection: NativeProtectionRecoverySelection,
    new_scope: NativeProtectionRecoveryScope,
    request_id: String,
    proposal_hash: String,
    consent: String,
) -> Result<JsonValue, String> {
    native_protection_recovery_approve_on(
        &state.db_path,
        &selection,
        &new_scope,
        &request_id,
        &proposal_hash,
        &consent,
    )
}

#[tauri::command]
pub fn get_native_protection_recovery_approval(
    state: State<AppState>,
    request_id: String,
) -> Result<Option<JsonValue>, String> {
    let db = native_protection_recovery_open(&state.db_path, false)?;
    let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let result = native_protection_recovery_saved_on(&tx, &request_id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
