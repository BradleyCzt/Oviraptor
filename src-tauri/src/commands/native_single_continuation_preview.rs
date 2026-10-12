// Production read-only Single provenance preview; no approval or new execution.
fn native_single_continuation_preview_in(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    root: &str,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::{multi_agent::budget::continuation, web_mode};
    if db.is_autocommit() {
        return Err("single_continuation_requires_snapshot".into());
    }
    let (status, target, plan_hash): (String, String, String) = db
        .query_row(
            "SELECT s.status,r.target_url,r.plan_hash FROM sentinel_scans s
         JOIN projects p ON p.id=s.project_id JOIN agent_runs r ON r.scan_id=s.id
         WHERE s.id=?1 AND s.attempt_count=?2 AND s.scan_type='web'
           AND s.status IN ('paused','partial') AND p.status='active'
           AND r.id=?3 AND r.attempt_number=?2 AND r.backend='native'
           AND r.role='coordinator' AND r.orchestration_policy='single'
           AND (r.root_run_id=r.id OR r.root_run_id='')
           AND r.parent_run_id IS NULL AND r.assignment_id=''
           AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=s.id)",
            params![scan, attempt, root],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "single_continuation_current_scope_invalid")?;
    let original = original_root_capture_mode(db, root)?;
    let declaration =
        web_mode::root::read(db, root)?.ok_or("single_continuation_original_mode_missing")?;
    if original.fact().mode != web_mode::WebMode::Single
        || declaration.mode() != web_mode::WebMode::Single
        || declaration.fact() != original.fact()
    {
        return Err("single_continuation_original_mode_changed".into());
    }
    declaration.validate_scope(scan, attempt, &target, &plan_hash)?;
    let source = continuation::single::original_source_on(db, root)?;
    let remaining = source.remaining();
    let model_contract = native_single_continuation_model_contract_on(db, scan, attempt, root)?;
    // Do not substitute today's configurable output cap or a rough input
    // estimate for an authenticated original Single model contract.
    Ok(json!({
        "schemaVersion": 1,
        "purpose": "single_resume_proposal_readonly",
        "scanId": scan,
        "attemptNumber": attempt,
        "rootRunId": root,
        "targetKey": target,
        "previousStatus": status,
        "mode": "single",
        "originalModeReceiptId": original.fact().receipt_id,
        "originalPlanHash": plan_hash,
        "sourceHistory": source.history(),
        "originalDeadline": remaining["originalDeadline"],
        "remaining": remaining,
        "modelContract": model_contract,
        "executionAllowed": false,
        "approvalImplemented": false,
        "birthImplemented": false,
        "approvalBlocked": true,
        "approvalBlockReasons": [
            "frozen_single_model_output_cap_missing",
            "frozen_complete_model_input_upper_bound_missing"
        ]
    }))
}

#[tauri::command]
pub fn preview_native_single_continuation(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    root_run_id: String,
) -> Result<JsonValue, String> {
    let db = native_protection_recovery_open(&state.db_path, false)?;
    let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
    native_single_continuation_preview_in(&snapshot, &scan_id, attempt_number, &root_run_id)
}
