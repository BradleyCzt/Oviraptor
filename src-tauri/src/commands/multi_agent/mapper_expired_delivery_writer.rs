//! Own the exact original DB writer and its hook; never replace a borrowed hook.
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{budget, lease::CoordinatorLease, scheduler::ScheduledChild},
};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, OpenFlags, Transaction, TransactionBehavior,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

pub(super) fn run<T>(
    original: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    work: impl FnOnce(&Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    if !original.is_autocommit() {
        return Err("mapper_loop_expired_delivery_outer_transaction_forbidden".into());
    }
    let path = original
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("mapper_loop_expired_delivery_existing_file_required")?;
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| format!("mapper_loop_expired_delivery_open:{e}"))?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    db.pragma_update(None, "foreign_keys", true)
        .map_err(|e| e.to_string())?;
    db.pragma_update(None, "synchronous", "FULL")
        .map_err(|e| e.to_string())?;
    crate::collaboration_events::install_commit_notifications(&db);
    let verified = Arc::new(AtomicBool::new(false));
    let hook = Arc::clone(&verified);
    db.authorizer(Some(move |c: AuthContext<'_>| authorize(c, hook.load(Ordering::Acquire))))
        .map_err(|e| e.to_string())?;
    let tx = Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let result: Result<T, String> = (|| {
        require_hook(&tx)?;
        // Same IMMEDIATE snapshot spans schema verification and every actual write.
        crate::collaboration_events::client_delivery_schema::verify(&tx)?;
        verify_immutable_guards(&tx)?;
        budget::model::mapper_loop_contract::require_protocol_on(&tx, actor)?;
        if child.role != AgentRole::SpaApiMapper {
            return Err("mapper_loop_expired_delivery_original_scope_invalid".into());
        }
        let grant = budget::model::original_mapper_loop_grant_on(&tx, actor, &child.assignment_id)?;
        let worker =
            crate::agent_runtime::multi_agent::attempts::current(&tx, actor, &child.assignment_id)?;
        if worker.child_run_id != child.run_id || worker.id != grant.worker_attempt_id() {
            return Err("mapper_loop_expired_delivery_original_scope_invalid".into());
        }
        // Pure historical replay is allowed; fresh proof separately requires live C.
        verified.store(true, Ordering::Release);
        let value = work(&tx)?;
        require_hook(&tx)?;
        Ok(value)
    })();
    match result {
        Ok(value) => {
            tx.commit()
                .map_err(|e| format!("mapper_loop_expired_delivery_commit:{e}"))?;
            Ok(value)
        }
        Err(error) => {
            let rollback = tx.rollback();
            let error =
                if error.contains("not authorized") || error.contains("authorization denied") {
                    format!("mapper_loop_expired_delivery_write_rejected:{error}")
                } else {
                    error
                };
            match rollback {
                Ok(()) => Err(error),
                Err(e) => Err(format!("{error};mapper_loop_expired_delivery_rollback:{e}")),
            }
        }
    }
}
fn require_hook(db: &Connection) -> Result<(), String> {
    match db.prepare("UPDATE projects SET name=name WHERE 0") {
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::AuthorizationForStatementDenied =>
        {
            Ok(())
        }
        _ => Err("mapper_loop_expired_delivery_authority_lost".into()),
    }
}
fn verify_immutable_guards(db: &Connection) -> Result<(), String> {
    let normalize = |s: &str| {
        s.trim()
            .trim_end_matches(';')
            .replace(" IF NOT EXISTS", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    for (name,suffix) in [
 ("budget_entry_no_update","BEFORE UPDATE ON agent_budget_entries BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END"),
 ("budget_entry_no_delete","BEFORE DELETE ON agent_budget_entries BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END"),
 ("budget_entry_no_replace","BEFORE INSERT ON agent_budget_entries WHEN EXISTS(SELECT 1 FROM agent_budget_entries WHERE entry_id=NEW.entry_id OR (root_run_id=NEW.root_run_id AND idempotency_key=NEW.idempotency_key)) BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END"),
 ] {
 let actual:String=db.query_row("SELECT sql FROM main.sqlite_master WHERE type='trigger' AND name=?1",[name],|r|r.get(0)).map_err(|_|"mapper_loop_expired_delivery_invoice_guard_changed")?;
 let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name=?1)",[name],|r|r.get(0)).map_err(|e|e.to_string())?;
 if shadow || normalize(&actual)!=normalize(&format!("CREATE TRIGGER {name} {suffix}")){return Err("mapper_loop_expired_delivery_invoice_guard_changed".into());}
 }
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected.execute_batch(crate::agent_runtime::multi_agent::specialist::mapper_loop::expired_seal::EXPIRED_SCHEMA).map_err(|e|e.to_string())?;
    for name in [
        "agent_mapper_loop_expired_receipts",
        "mapper_loop_expired_no_update",
        "mapper_loop_expired_no_delete",
        "mapper_loop_expired_no_replace",
    ] {
        let actual: String = db
            .query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|_| "mapper_loop_expired_delivery_receipt_guard_changed")?;
        let trusted: String = expected
            .query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name=?1 OR tbl_name='agent_mapper_loop_expired_receipts')",[name],|r|r.get(0)).map_err(|e|e.to_string())?;
        if shadow || actual != trusted {
            return Err("mapper_loop_expired_delivery_receipt_guard_changed".into());
        }
    }
    Ok(())
}
fn authorize(c: AuthContext<'_>, verified: bool) -> Authorization {
    let direct = c.database_name == Some("main") && c.accessor.is_none();
    let allow = match c.action {
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if c.database_name == Some("main") => {
            verified
                && c.accessor.is_some_and(|name| {
                    crate::collaboration_events::client_delivery_schema::EMITTERS.contains(&name)
                })
        }
        AuthAction::Insert {
            table_name:
                "agent_budget_entries" | "agent_messages" | "agent_mapper_loop_expired_receipts",
        } => direct && verified,
        AuthAction::Update {
            table_name,
            column_name,
        } if direct && verified => match table_name {
            "agent_budget_ledger" => matches!(
                column_name,
                "reserved_tokens"
                    | "reserved_requests"
                    | "spent_tokens"
                    | "spent_requests"
                    | "updated_at"
            ),
            "agent_assignments" => matches!(
                column_name,
                "reserved_tokens"
                    | "reserved_requests"
                    | "budget_settled_at"
                    | "state"
                    | "failure_class"
                    | "finished_at"
                    | "updated_at"
            ),
            "agent_runs" => matches!(
                column_name,
                "used_tokens"
                    | "used_cached_tokens"
                    | "used_requests"
                    | "updated_at"
                    | "status"
                    | "terminal_state"
                    | "terminal_code"
                    | "terminal_reason"
                    | "finished_at"
            ),
            "agent_messages" => matches!(
                column_name,
                "delivered_at" | "acknowledged_at" | "delivery_attempts"
            ),
            _ => false,
        },
        AuthAction::Delete {
            table_name: "agent_lane_leases",
        } => direct && verified,
        // Immutable guards may read and RAISE. No accessor may write even to a
        // directly allowed table, except the exact verified collaboration emitters.
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => true,
        AuthAction::Pragma { pragma_name, .. } => matches!(
            pragma_name,
            "table_info"
                | "table_xinfo"
                | "table_list"
                | "index_list"
                | "index_info"
                | "index_xinfo"
                | "foreign_key_list"
        ),
        _ => false,
    };
    if allow {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
