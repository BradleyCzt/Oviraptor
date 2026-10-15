//! One private authority for the complete financial publication transaction.
//! Built-in emitter SQL is verified; all other trigger writes are denied.
use crate::agent_runtime::multi_agent::lease::{self, CoordinatorLease};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection,
};

pub(crate) fn closure_write<T>(
    db: &Connection,
    actor: &CoordinatorLease,
    source: bool,
    work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    write(db, actor, source, false, work)
}

// Only the original Source pause consumer uses this finite publication path.
// Root closure and the exact business pause share one private transaction.
pub(crate) fn source_pause_write<T>(
    db: &Connection,
    actor: &CoordinatorLease,
    work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    write(db, actor, false, true, work)
}

/// Publish the terminal result of an explicitly issued financial assessment.
/// The same finite status columns apply; no targets or execution grants arise.
pub(crate) fn financial_assessment_write<T>(
    db: &Connection,
    actor: &CoordinatorLease,
    work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    write(db, actor, false, true, |tx| {
        let birth = crate::scanning::financial_continuation::verify_financial_continuation_birth_on(tx, &actor.root_run_id)?;
        if birth["scanId"] != actor.scan_id || birth["newAttemptNumber"] != actor.attempt_number {
            return Err("financial_continuation_closure_scope_invalid".into());
        }
        work(tx)
    })
}

fn write<T>(
    db: &Connection,
    actor: &CoordinatorLease,
    source: bool,
    pause: bool,
    work: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    if !db.is_autocommit() {
        return Err("budget_closure_writer_requires_private_transaction".into());
    }
    let path = db.path().filter(|path| !path.is_empty())
        .ok_or("budget_closure_database_missing")?;
    let caller = db;
    crate::collaboration_events::closure_schema::verify(caller)?;
    let private = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| e.to_string())?;
    private.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e|e.to_string())?;
    private.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;").map_err(|e|e.to_string())?;
    let db = &private;
    db.authorizer(Some(move |c: AuthContext<'_>| authorize(c, source, pause)))
        .map_err(|e| e.to_string())?;
    (|| {
        let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        // Both emitter validation and all writes hold the same SQLite lock.
        lease::validate_coordinator_lease(&tx, actor)?;
        crate::collaboration_events::closure_schema::verify(&tx)?;
        let events = super::closure_events::Events::capture(&tx)?;
        let value = work(&tx)?;
        events.verify(&tx, actor, source)?;
        crate::collaboration_events::closure_schema::verify(&tx)?;
        crate::collaboration_events::closure_schema::verify(caller)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(value)
    })()
}

fn authorize(c: AuthContext<'_>, source: bool, pause: bool) -> Authorization {
    let main = c.database_name == Some("main") && c.accessor.is_none();
    let allowed = match c.action {
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if c.database_name == Some("main") => c.accessor.is_some_and(|name| {
            crate::collaboration_events::closure_schema::EMITTERS.contains(&name)
        }),
        AuthAction::Insert { table_name } if main => {
            matches!(table_name, "agent_budget_entries" | "agent_multi_exit_receipts")
                || (source && table_name == "agent_events")
        }
        AuthAction::Update {
            table_name,
            column_name,
        } if main => match table_name {
            "sentinel_scans" | "sentinel_scan_attempts" | "native_scan_branches" | "sentinel_targets" => {
                pause && super::pause_authority::allows(table_name, column_name)
            }
            "agent_runs" => matches!(
                column_name,
                "status"
                    | "terminal_state"
                    | "terminal_code"
                    | "terminal_reason"
                    | "finished_at"
                    | "updated_at"
            ),
            "agent_assignments" => matches!(
                column_name,
                "state"
                    | "failure_class"
                    | "reserved_tokens"
                    | "reserved_requests"
                    | "finished_at"
                    | "updated_at"
            ),
            "agent_assignment_attempts" => {
                matches!(column_name, "state" | "finished_at" | "failure_class")
            }
            "agent_capability_leases" => column_name == "revoked_at",
            "agent_budget_ledger" => matches!(
                column_name,
                "reserved_tokens" | "reserved_requests" | "updated_at"
            ),
            "agent_directive_proposals" => {
                matches!(column_name, "state" | "error_code" | "updated_at")
            }
            "agent_user_directives" => matches!(
                column_name,
                "status" | "rejection_code" | "payload_json" | "finished_at" | "updated_at"
            ),
            "sentinel_scan_contexts" => {
                source && matches!(column_name, "gate_status" | "gate_reason")
            }
            _ => false,
        },
        AuthAction::Delete {
            table_name: "agent_lane_leases",
        } if main => true,
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => true,
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
