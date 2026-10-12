//! Original Single financial evidence. This capability grants no new work.
use rusqlite::Connection;
use serde_json::Value;

pub(crate) struct VerifiedSingleFinancialSource {
    history: Value,
    remaining: Value,
}
impl VerifiedSingleFinancialSource {
    pub(crate) fn history(&self) -> Value {
        self.history.clone()
    }
    pub(crate) fn remaining(&self) -> Value {
        self.remaining.clone()
    }
}

/// Caller retains original worker exit guards throughout its snapshot. This
/// reader never creates an invocation inode, Coordinator lease or receipt.
pub(crate) fn original_source_on(
    db: &Connection,
    root: &str,
) -> Result<VerifiedSingleFinancialSource, String> {
    if db.is_autocommit() {
        return Err("continuation_financial_snapshot_required".into());
    }
    let (history, remaining) = super::single_remaining::capture_original_on(db, root)?;
    Ok(VerifiedSingleFinancialSource { history, remaining })
}
