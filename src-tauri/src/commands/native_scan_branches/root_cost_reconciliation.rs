// This IPC inventories and confirms original bills only. It never resumes a
// worker, creates an attempt, renews authorization or accesses a target.
fn native_root_cost_reconciliation_calls_on(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> Result<Vec<(String, String)>, String> {
    // Confirmed receipts must not permanently occupy the bounded window ahead
    // of later unconfirmed bills. Presence only controls display order; every
    // selected bill/receipt is still independently verified below. Unknown
    // transport remains visible as blocked; it cannot be confirmed from an
    // absent invoice. Actionable received bills precede unknown transport.
    // Several physical phases still represent one original call; only the
    // display candidate is grouped, every receipt remains independently audited.
    let mut q = db.prepare("SELECT j.root_run_id,j.call_id FROM agent_root_model_journal j
        JOIN agent_runs r ON r.id=j.root_run_id AND r.backend='native' AND r.role='coordinator'
        WHERE r.scan_id=?1 AND r.attempt_number=?2 AND j.phase IN ('received','uncertain') AND (
          j.phase='uncertain' OR
          EXISTS(SELECT 1 FROM agent_root_cost_reconciliation_receipts c WHERE c.call_id=j.call_id)
          OR NOT EXISTS(SELECT 1 FROM agent_root_tick_terminal_proofs p WHERE p.call_id=j.call_id)
          OR (EXISTS(SELECT 1 FROM agent_budget_entries b WHERE b.source_id=j.call_id AND b.kind='forfeit')
            AND NOT EXISTS(SELECT 1 FROM agent_budget_entries b WHERE b.source_id=j.call_id AND b.kind='reconcile')))
        GROUP BY j.root_run_id,j.call_id
        ORDER BY EXISTS(SELECT 1 FROM agent_root_cost_reconciliation_receipts c WHERE c.call_id=j.call_id),
          MIN(j.phase='uncertain'),j.root_run_id,MIN(j.round),j.call_id").map_err(|e| e.to_string())?;
    let rows = q.query_map(params![scan, attempt], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    }).map_err(|e| e.to_string())?;
    let mut calls = Vec::new();
    for row in rows {
        let (root, call) = row.map_err(|e| e.to_string())?;
        // Audit original Single receipts before applying the display bound.
        // Known bills cannot fill the first hundred ahead of an unknown bill;
        // Multi, corrupt or incomplete receipts are never skipped by this gate.
        if crate::agent_runtime::multi_agent::budget::root::model::single_history::verify_settled_bill_on(
            db, &root, &call,
        ).is_ok() {
            continue;
        }
        calls.push((root, call));
        if calls.len() == 101 {
            break;
        }
    }
    Ok(calls)
}

fn native_root_cost_reconciliations_for_attempt(
    connection: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let tx = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let visible: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2)
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            params![scan, attempt],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if attempt <= 0 || !visible {
        return Err("root_reconciliation_attempt_changed".into());
    }
    let calls = native_root_cost_reconciliation_calls_on(&tx, scan, attempt)?;
    let truncated = calls.len() > 100;
    let mut items = Vec::new();
    let mut receipts = Vec::new();
    let mut blocked = Vec::new();
    for (root, call) in calls.into_iter().take(100) {
        let result =
            Tick::read_cost_reconciliation(&tx, scan, attempt, &root, &call).and_then(|receipt| {
                if let Some(receipt) = receipt {
                    receipts.push(receipt);
                } else {
                    items.push(Tick::preview_cost_reconciliation(
                        &tx, scan, attempt, &root, &call,
                    )?);
                }
                Ok(())
            });
        if let Err(reason) = result {
            // Do not expose arbitrary SQLite/provider error strings in a bill panel.
            let safe = if reason.starts_with("root_tick_")
                || reason.starts_with("root_reconciliation_")
                || reason.starts_with("budget_")
            {
                reason
            } else {
                "root_reconciliation_validation_failed".into()
            };
            blocked.push(json!({"rootRunId":root,"callId":call,"reason":safe}));
        }
    }
    Ok(
        json!({"schema":1,"executionAllowed":false,"automaticResumeAllowed":false,
        "items":items,"blocked":blocked,"receipts":receipts,"truncated":truncated}),
    )
}

#[tauri::command]
pub fn get_native_root_cost_reconciliations(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    native_root_cost_reconciliations_for_attempt(&connection, &scan_id, attempt_number)
}

#[tauri::command]
pub fn confirm_native_root_cost_reconciliation(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    root_run_id: String,
    call_id: String,
    confirmation_hash: String,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::confirm_cost_reconciliation(
        &connection,
        &scan_id,
        attempt_number,
        &root_run_id,
        &call_id,
        &confirmation_hash,
    )
}
