// A failed dedicated assessment may only stop its new attempt for review.
// No triggers may publish collateral business, asset or financial changes.
struct FinancialContinuationFailureWriter<'a> {
    db: &'a rusqlite::Connection,
}
impl<'a> FinancialContinuationFailureWriter<'a> {
    fn install(db: &'a rusqlite::Connection) -> Result<Self, String> {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        db.authorizer(Some(|c: AuthContext<'_>| match c.action {
            AuthAction::Update {
                table_name,
                column_name,
            } if c.database_name == Some("main")
                && c.accessor.is_none()
                && match table_name {
                    "sentinel_scans" => {
                        matches!(column_name, "status" | "current_checkpoint" | "updated_at")
                    }
                    "sentinel_scan_attempts" => matches!(
                        column_name,
                        "status" | "finished_at" | "stop_reason" | "updated_at"
                    ),
                    _ => false,
                } =>
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
        .map_err(|e| e.to_string())?;
        Ok(Self { db })
    }
}
impl Drop for FinancialContinuationFailureWriter<'_> {
    fn drop(&mut self) {
        use rusqlite::hooks::{AuthContext, Authorization};
        let _ = self
            .db
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}

fn native_financial_continuation_stop_attempt(
    db: &rusqlite::Connection,
    scan: &str,
    root: &str,
) -> Result<(), String> {
    let writer = FinancialContinuationFailureWriter::install(db)?;
    let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let birth = verify_financial_continuation_birth_on(&tx, root)?;
    if birth["scanId"] != scan {
        return Err("financial_continuation_publication_scope_changed".into());
    }
    let attempt = birth["newAttemptNumber"]
        .as_i64()
        .ok_or("financial_continuation_birth_invalid")?;
    let status: String = tx
        .query_row(
            "SELECT status FROM sentinel_scans WHERE id=?1 AND attempt_count=?2
        AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
            params![scan, attempt],
            |r| r.get(0),
        )
        .map_err(|_| "financial_continuation_current_scope_invalid")?;
    if status == "scanning" {
        let checkpoint = "financial_assessment_requires_review";
        let changed = tx.execute("UPDATE sentinel_scan_attempts SET status='partial',finished_at=datetime('now','localtime'),
            stop_reason=?3,updated_at=datetime('now','localtime') WHERE scan_id=?1 AND attempt_number=?2 AND status='scanning'",
            params![scan,attempt,checkpoint]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("financial_continuation_attempt_stop_conflict".into());
        }
        let changed = tx.execute("UPDATE sentinel_scans SET status='partial',current_checkpoint=?3,
            updated_at=datetime('now','localtime') WHERE id=?1 AND attempt_count=?2 AND status='scanning'",
            params![scan,attempt,checkpoint]).map_err(|e|e.to_string())?;
        if changed != 1 {
            return Err("financial_continuation_scan_stop_conflict".into());
        }
    } else if !matches!(status.as_str(), "partial" | "paused") {
        return Err("financial_continuation_current_scope_invalid".into());
    }
    let result = tx.commit().map_err(|e| e.to_string());
    drop(writer);
    result
}
