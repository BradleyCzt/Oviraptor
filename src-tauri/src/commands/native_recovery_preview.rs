// Historical preparation only; an independent approval and fresh birth are
// required before any new execution. Expired original C is not renewed here.
fn native_recovery_original_actor_on(
    snapshot: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    root: &str,
) -> Result<crate::agent_runtime::multi_agent::lease::CoordinatorLease, String> {
    use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
    let actor = snapshot
        .query_row(
            "SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,
         fencing_token,lease_expires_at FROM main.agent_coordinator_leases
         WHERE root_run_id=?1",
            [root],
            |row| {
                Ok(CoordinatorLease {
                    scan_id: row.get(0)?,
                    attempt_number: row.get(1)?,
                    target_key: row.get(2)?,
                    root_run_id: row.get(3)?,
                    lease_epoch: row.get(4)?,
                    fencing_token: row.get(5)?,
                    lease_expires_at: row.get(6)?,
                })
            },
        )
        .map_err(|_| "recovery_preview_original_coordinator_missing")?;
    if actor.scan_id != scan || actor.attempt_number != attempt || actor.root_run_id != root {
        return Err("recovery_preview_original_scope_conflict".into());
    }
    Ok(actor)
}

fn native_recovery_preview_on(
    db: &mut rusqlite::Connection,
    db_path: &Path,
    scan: &str,
    attempt: i64,
    root: &str,
    receipt: &str,
) -> Result<JsonValue, String> {
    let snapshot = db.transaction().map_err(|e| e.to_string())?;
    let actor = native_recovery_original_actor_on(&snapshot, scan, attempt, root)?;
    let source = native_recovery_history_on(&snapshot, db_path, &actor, receipt)?;
    for rows in source.verification_rows() {
        rows.verify(&snapshot)?;
    }
    let fact = source.bounded_fact();
    snapshot.commit().map_err(|e| e.to_string())?;
    // Selected and historical original invocation guards live through commit.
    drop(source);
    Ok(fact)
}

#[tauri::command]
pub fn preview_native_protection_recovery(
    state: State<AppState>,
    scan_id: String,
    attempt_number: i64,
    root_run_id: String,
    protection_receipt_id: String,
) -> Result<JsonValue, String> {
    let mut db = rusqlite::Connection::open_with_flags(
        &state.db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    native_recovery_preview_on(
        &mut db,
        &state.db_path,
        &scan_id,
        attempt_number,
        &root_run_id,
        &protection_receipt_id,
    )
}
