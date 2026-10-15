//! Explicit production adapter; no test shim or ambient thread-local observer.
use super::{Journal, Scope};
use crate::native_pipeline::{
    analyzer::AnalyzerSpec,
    process::{self, BoundedRun, ProcessError, ProcessLimits},
};
use rusqlite::Connection;
use std::path::Path;

pub(crate) fn run(
    connection: &Connection,
    spec: &AnalyzerSpec,
    key: &str,
    stage: &str,
    args: &[String],
    limits: &ProcessLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<BoundedRun, ProcessError> {
    let failure = |reason: &str| ProcessError::Launch {
        program: spec.program.display().to_string(),
        reason: reason.into(),
    };
    let path = connection
        .path()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| failure("native_process_log_database_unavailable"))?;
    // Source launch already holds NativeBranchGuard and its durable claim. Read
    // that receipt; diagnostics never create a Root or grant branch execution.
    let dispatch_claim_id: String = connection
        .query_row(
            "SELECT d.claim_id FROM native_branch_dispatches d JOIN native_scan_branches b
         ON b.scan_id=d.scan_id AND b.attempt_number=d.attempt_number AND b.branch=d.branch
         WHERE d.scan_id=?1 AND d.attempt_number=?2 AND d.branch='source'
         AND d.claim_id<>'' AND d.claimed_at<>'' AND b.status='pending'",
            rusqlite::params![spec.scan_id, spec.attempt_number],
            |row| row.get(0),
        )
        .map_err(|_| failure("native_process_log_branch_not_claimed"))?;
    let scope = Scope {
        scan_id: spec.scan_id.clone(),
        attempt: spec.attempt_number,
        branch: "source".into(),
        dispatch_claim_id,
        invocation_key: key.into(),
        execution_id: uuid::Uuid::new_v4().to_string(),
        stage: format!("{}:{stage}", spec.engine.as_str()),
    };
    let mut journal = Journal::begin(Path::new(path), scope).map_err(|error| failure(&error))?;
    let mut notify = |hint: &super::Hint| crate::scanning::log_events::notify_native_process_log(hint);
    let result = process::run_observed(
        &spec.program,
        args,
        Some(&spec.scratch_dir),
        limits,
        cancelled,
        &mut |event| journal.append(event, &mut notify).map(|_| ()),
    );
    let observed = match result {
        Ok(observed) => observed,
        Err(error) => {
            let _ = journal.finish("failed", &mut notify);
            return Err(error);
        }
    };
    let state = if observed.run.cancelled {
        "cancelled"
    } else if observed.run.timed_out {
        "timeout"
    } else if observed.log_error.is_some() {
        "gap"
    } else if observed.run.succeeded() {
        "completed"
    } else {
        "failed"
    };
    journal
        .finish(state, &mut notify)
        .map_err(|error| failure(&error))?;
    if let Some(error) = observed.log_error {
        return Err(failure(&error));
    }
    Ok(observed.run)
}
