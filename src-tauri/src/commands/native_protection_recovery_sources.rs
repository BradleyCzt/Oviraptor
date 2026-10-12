// Source selection only: authenticated historical receipts are not readiness,
// approval or execution. Every proposal independently rechecks full history.
fn native_protection_recovery_sources_on(
    db: &rusqlite::Connection,
    scan: &str,
) -> Result<JsonValue, String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    let shadow: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name GLOB 'agent_*' OR name GLOB 'sentinel_*' OR name GLOB 'native_*')",
        [], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if shadow {
        return Err("protection_recovery_sources_schema_shadowed".into());
    }
    crate::agent_runtime::target_protection::verify_schema_on(db)?;
    let (project, task): (i64, String) = db
        .query_row(
            "SELECT project_id,task_name FROM main.sentinel_scans WHERE id=?1 AND scan_type='web'",
            [scan],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "protection_recovery_sources_scan_missing")?;
    let mut query = db.prepare(
        "SELECT attempt_number,root_run_id,receipt_id FROM main.agent_protection_response_receipts WHERE scan_id=?1 ORDER BY attempt_number,root_run_id,receipt_id LIMIT 257",
    ).map_err(|e| e.to_string())?;
    let candidates = query
        .query_map([scan], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if candidates.len() > 256 {
        return Err("protection_recovery_sources_inventory_too_large".into());
    }
    let mut items = Vec::new();
    let mut proofs = Vec::new();
    for (attempt, root, receipt) in candidates {
        let actor = native_recovery_original_actor_on(db, scan, attempt, &root)?;
        let proof = native_original_protection_response_on(db, &actor, &receipt)?;
        let fact = proof.bounded_fact();
        items.push(json!({"selection": NativeProtectionRecoverySelection {
            scan_id: scan.into(), attempt_number: attempt, root_run_id: root,
            protection_receipt_id: receipt,
        }, "targetUrl":actor.target_key,"responseStatus":fact["status"],
            "originalFactHash":fact["originalFactHash"],"sourceVerified":true,
            "readinessAssessed":false,"executionAllowed":false}));
        proofs.push(proof);
    }
    for proof in proofs {
        for rows in proof.verification_rows() {
            rows.verify(db)?;
        }
    }
    Ok(
        json!({"schemaVersion":1,"purpose":"select_original_signed_protection_source",
        "scanId":scan,"projectId":project,"taskName":task,"items":items,
        "approval":false,"readinessAssessed":false,"executionAllowed":false,"targetRequestsGranted":0}),
    )
}

#[tauri::command]
pub fn discover_native_protection_recovery_sources(
    state: State<AppState>,
    scan_id: String,
) -> Result<JsonValue, String> {
    let db = native_protection_recovery_open(&state.db_path, false)?;
    let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let result = native_protection_recovery_sources_on(&snapshot, &scan_id)?;
    snapshot.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
