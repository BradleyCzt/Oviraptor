// Register every branch before starting either worker. Only this reducer may
// publish a native scan's terminal state; a successful web branch cannot cancel
// a still-running source branch or hide its failure.
fn native_web_attempt_active(db_path: &Path, scan_id: &str, attempt: i64) -> bool {
    db::open(db_path).is_ok_and(|connection| native_source_attempt_active(&connection, scan_id, attempt))
}

fn native_web_attempt_current(db_path: &Path, scan_id: &str, attempt: i64) -> bool {
    db::open(db_path).is_ok_and(|connection| connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND attempt_count=?2)
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        params![scan_id, attempt], |row| row.get::<_, bool>(0),
    ).unwrap_or(false))
}

fn native_web_attempt_progress(db_path: &Path, scan_id: &str, attempt: i64, checkpoint: &str) {
    let Ok(mut connection) = db::open(db_path) else { return; };
    let Ok(transaction) = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate) else { return; };
    if !native_source_attempt_active(&transaction, scan_id, attempt) { return; }
    if transaction.execute(
        "UPDATE sentinel_scans SET current_checkpoint=?1,updated_at=datetime('now','localtime') WHERE id=?2 AND attempt_count=?3 AND status='scanning'",
        params![checkpoint, scan_id, attempt],
    ).is_ok() {
        sync_sentinel_attempt(&transaction, scan_id);
        let _ = transaction.commit();
    }
}

fn insert_native_frontend_recon(db_path: &Path, scan_id: &str, attempt: i64, recon: &JsonValue) -> Result<i64, String> {
    let mut connection = db::open(db_path)?;
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    if !native_source_attempt_active(&transaction, scan_id, attempt) {
        return Err("native_attempt_stopped_or_replaced".into());
    }
    let count = insert_frontend_recon(&transaction, scan_id, recon)?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(count)
}

// Test convenience wrapper. Live starters register inside their publication.
#[cfg(test)]
fn register_native_branches(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64, branches: &[&str],
) -> Result<(), String> {
    let transaction = rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    register_native_branches_in(&transaction, scan_id, attempt, branches)?;
    transaction.commit().map_err(|e| e.to_string())
}

// The caller owns the transaction: Web startup includes this registration in
// the same commit as its task, targets and immutable attempt ledger.
fn register_native_branches_in(
    connection: &rusqlite::Connection, scan_id: &str, attempt: i64, branches: &[&str],
) -> Result<(), String> {
    if branches.is_empty() || branches.iter().any(|b| !matches!(*b, "source" | "web")) {
        return Err("invalid_native_branch_set".into());
    }
    if connection.is_autocommit() { return Err("native_branch_registration_requires_transaction".into()); }
    if !native_source_attempt_active(connection, scan_id, attempt) {
        return Err("native_attempt_stopped_or_replaced".into());
    }
    let existing: i64 = connection.query_row(
        "SELECT count(*) FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2",
        params![scan_id, attempt], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if existing > 0 { return Err("native_branch_set_already_frozen".into()); }
    for branch in branches {
        let changed = connection.execute(
            "INSERT INTO native_scan_branches(scan_id,attempt_number,branch) VALUES(?1,?2,?3)",
            params![scan_id, attempt, branch],
        ).map_err(|e| e.to_string())?;
        if changed != 1 { return Err("native_branch_registration_not_persisted".into()); }
        register_native_dispatch_in(connection, scan_id, attempt, branch)?;
    }
    let mut statement = connection.prepare("SELECT branch,status,checkpoint,report_json FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 ORDER BY branch").map_err(|e| e.to_string())?;
    let actual = statement.query_map(params![scan_id,attempt], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?)))
        .map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    let mut expected = branches.iter().map(|branch| (branch.to_string(), "pending".to_string(), String::new(), "{}".to_string())).collect::<Vec<_>>();
    expected.sort();
    if actual != expected { return Err("native_branch_registration_postcondition".into()); }
    let mut statement = connection.prepare("SELECT branch,claim_id,claimed_at FROM native_branch_dispatches WHERE scan_id=?1 AND attempt_number=?2 ORDER BY branch").map_err(|e| e.to_string())?;
    let dispatches = statement.query_map(params![scan_id,attempt], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))
        .map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    let expected = expected.into_iter().map(|(branch,_,_,_)| (branch,String::new(),String::new())).collect::<Vec<_>>();
    if dispatches != expected { return Err("native_dispatch_registration_postcondition".into()); }
    Ok(())
}

fn finish_native_branch(
    db_path: &Path, scan_id: &str, attempt: i64, branch: &str,
    status: &str, checkpoint: &str, report: &JsonValue,
) -> Result<bool, String> {
    let mut connection = db::open(db_path)?;
    let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let changed = finish_native_branch_in(&transaction,scan_id,attempt,branch,status,checkpoint,report)?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(changed)
}

fn finish_native_branch_in(
    transaction: &rusqlite::Transaction<'_>, scan_id: &str, attempt: i64, branch: &str,
    status: &str, checkpoint: &str, report: &JsonValue,
) -> Result<bool, String> {
    if !matches!(status, "completed" | "completed_with_gaps" | "partial" | "failed") {
        return Err("invalid_native_branch_terminal".into());
    }
    if !native_source_attempt_active(transaction, scan_id, attempt) { return Ok(false); }
    let changed = transaction.execute(
        "UPDATE native_scan_branches SET status=?1,checkpoint=?2,report_json=?3,updated_at=datetime('now','localtime')
         WHERE scan_id=?4 AND attempt_number=?5 AND branch=?6 AND status='pending'",
        params![status,checkpoint,report.to_string(),scan_id,attempt,branch],
    ).map_err(|e| e.to_string())?;
    if changed == 0 { return Ok(false); }
    if branch == "source" {
        transaction.execute(
            "UPDATE sentinel_targets SET status=?1,last_attempt_number=?2
             WHERE scan_id=?3 AND url=(SELECT source_path FROM sentinel_scans WHERE id=?3)",
            params![status, attempt, scan_id],
        ).map_err(|e| e.to_string())?;
    }
    let rows: Vec<(String,String,String)> = {
        let mut statement = transaction.prepare(
            "SELECT branch,status,checkpoint FROM native_scan_branches WHERE scan_id=?1 AND attempt_number=?2 ORDER BY branch",
        ).map_err(|e| e.to_string())?;
        let rows = statement.query_map(params![scan_id,attempt], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))
            .map_err(|e| e.to_string())?.collect::<Result<_,_>>().map_err(|e| e.to_string())?;
        rows
    };
    let terminal = if rows.iter().any(|(_,s,_)| s == "pending") { "scanning" }
        else if rows.iter().all(|(_,s,_)| s == "failed") { "failed" }
        else if rows.iter().any(|(_,s,_)| matches!(s.as_str(), "failed" | "partial")) { "partial" }
        else if rows.iter().any(|(_,s,_)| s == "completed_with_gaps") { "completed_with_gaps" }
        else { "completed" };
    let mut summary = rows.iter().map(|(b,s,c)| format!("{b}: {s} · {c}")).collect::<Vec<_>>().join("；");
    let graph_notes = crate::native_pipeline::greybox::project_closed_attempt(transaction, scan_id, attempt)?;
    if !graph_notes.is_empty() {
        summary = format!("{summary}；{}", graph_notes.join("；"));
    }
    if terminal == "scanning" {
        // A sibling is still pending. This is progress, not a new activation;
        // do not re-trigger the environment-preparation activation interlock.
        transaction.execute(
            "UPDATE sentinel_scans SET current_checkpoint=?1,updated_at=datetime('now','localtime')
             WHERE id=?2 AND attempt_count=?3 AND status='scanning'",
            params![summary,scan_id,attempt],
        ).map_err(|e| e.to_string())?;
    } else {
        transaction.execute(
            "UPDATE sentinel_scans SET status=?1,current_checkpoint=?2,updated_at=datetime('now','localtime')
             WHERE id=?3 AND attempt_count=?4 AND status='scanning'",
            params![terminal,summary,scan_id,attempt],
        ).map_err(|e| e.to_string())?;
    }
    sync_sentinel_attempt(transaction, scan_id);
    Ok(true)
}

struct NativeBranchGuard {
    db_path: PathBuf,
    scan_id: String,
    attempt: i64,
    branch: &'static str,
    armed: bool,
    _invocation: Option<NativeInvocationOwner>,
}

impl NativeBranchGuard {
    /// Must precede recon, model-policy writes and construction of the failure
    /// guard. A duplicate launch returns without owning any cleanup obligation.
    fn claim(db_path: &Path, scan_id: &str, attempt: i64, branch: &'static str) -> Result<Self, String> {
        Self::claim_with_preflight(db_path,scan_id,attempt,branch, |connection| {
            let bound: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM native_web_dispatch_bindings WHERE scan_id=?1 AND attempt_number=?2 AND branch=?3)",
                params![scan_id,attempt,branch], |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            if bound { Err("web_binding_preflight_required".into()) } else { Ok(()) }
        })
    }

    fn claim_with_preflight(
        db_path: &Path, scan_id: &str, attempt: i64, branch: &'static str,
        mut preflight: impl FnMut(&rusqlite::Connection) -> Result<(),String>,
    ) -> Result<Self, String> {
        if !matches!(branch, "web" | "source") { return Err("invalid_native_branch".into()); }
        let invocation = claim_native_invocation(db_path, scan_id, attempt, "branch", branch)?;
        let mut connection = db::open(db_path)?;
        // FULL makes the admission receipt durable before target/model/browser
        // effects. db::open's NORMAL is appropriate for other progress writes,
        // but losing this commit could otherwise turn a restart into replay.
        connection.pragma_update(None, "synchronous", "FULL").map_err(|error| error.to_string())?;
        let transaction = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        preflight(&transaction)?;
        claim_native_dispatch_in(&transaction, scan_id, attempt, branch)?;
        // A claim trigger must not alter the configuration after its admission
        // check. Neither a guard nor any worker effect exists until commit.
        preflight(&transaction)?;
        transaction.commit().map_err(|error| format!("native_dispatch_commit_unconfirmed:{error}"))?;
        Ok(Self { db_path: db_path.to_path_buf(), scan_id: scan_id.into(), attempt, branch,
            armed: true, _invocation: Some(invocation) })
    }

    fn disarm(&mut self) { self.armed = false; }
}

impl Drop for NativeBranchGuard {
    fn drop(&mut self) {
        // Successful completion is already immutable; pause/new attempts reject
        // this write. Source needs original facts; unproved exit remains pending
        // for recovery rather than overwriting paid work as a raw failure.
        if self.armed {
            if self.branch == "source" {
                // No raw fallback may erase an original paid or unclosed Root.
                let _ = finish_native_source_branch(&self.db_path, &self.scan_id, self.attempt,
                    &json!({"error":"branch_worker_exited"}));
            } else {
                let _ = finish_native_branch(&self.db_path, &self.scan_id, self.attempt,
                    self.branch, "failed", "分支工作线程意外退出；结果不完整", &json!({"error":"branch_worker_exited"}));
            }
        }
        drop(self._invocation.take());
        if sentinel_scan_pause_requested(&self.db_path,&self.scan_id) {
            let _ = finish_sentinel_pause(&self.db_path,&self.scan_id,self.attempt);
        }
    }
}

#[tauri::command]
pub fn get_native_scan_status(
    state: State<'_, AppState>,
    scan_id: String,
    after_sequence: Option<i64>,
    expected_attempt_number: Option<i64>,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    native_scan_status_for_attempt(&connection, &scan_id, after_sequence, expected_attempt_number)
}
#[tauri::command]
pub fn get_native_scan_timeline_page(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    before_sequence: i64,
) -> Result<JsonValue, String> {
    let connection = db::open(&state.db_path)?;
    native_scan_timeline_page(&connection, &scan_id, attempt_number, before_sequence)
}
// Keep branch lifecycle, read-only history/status and operator directives in
// their own business files. Includes preserve the existing command scope.
include!("native_scan_branches/history.rs");
include!("native_scan_branches/status_helpers.rs");
include!("native_scan_branches/status_timeline_window.rs");
include!("native_scan_branches/status_root_decisions.rs");
include!("native_scan_branches/status_human_assessments.rs");
include!("native_scan_branches/status_timeline.rs");
include!("native_scan_branches/status_timeline_page.rs");
include!("native_scan_branches/status_budget.rs");
include!("native_scan_branches/root_cost_reconciliation.rs");
include!("native_financial_continuation_preview.rs");
include!("native_recovery_preview.rs");
include!("native_financial_continuation_authorization.rs");
include!("native_financial_continuation_files.rs");
include!("native_financial_continuation_failure_writer.rs");
include!("native_financial_continuation_worker.rs");
#[cfg(test)]
include!("native_financial_continuation_received_crash.rs");
include!("native_financial_continuation_admission.rs");
include!("native_financial_continuation_finish.rs");
include!("native_financial_continuation_approval.rs");
include!("native_financial_continuation_status.rs");
include!("native_financial_continuation_discovery.rs");
include!("native_scan_branches/status.rs");
include!("native_scan_branches/directives.rs");
