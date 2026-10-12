//! Restricted task-external writer. It owns its connection and cannot replace
//! a caller's authorizer; no transaction or caller-provided callback escapes.
use super::approval::{self, ToolAdministratorConfiguration, VerifiedToolApproval};
use crate::db::EnvironmentPreparationLease;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection, OpenFlags, Transaction, TransactionBehavior,
};
use std::time::Duration;

#[cfg(test)]
pub(super) fn approve(
    original: &Connection,
    preparation: &EnvironmentPreparationLease,
    configuration: &ToolAdministratorConfiguration,
    decision: &[u8],
    manifest: &[u8],
    package: &[u8],
) -> Result<VerifiedToolApproval, String> {
    run(original, preparation, |tx| {
        approval::approve_on(tx, preparation, configuration, decision, manifest, package)
    })
}

/// Publish inert bytes before committing the original approval. A cache fact
/// alone grants nothing; failure rolls back the decision and retains the fence.
pub(super) fn approve_with_package(
    original: &Connection,
    preparation: &EnvironmentPreparationLease,
    configuration: &ToolAdministratorConfiguration,
    decision: &[u8],
    manifest: &[u8],
    package: &[u8],
) -> Result<VerifiedToolApproval, String> {
    run(original, preparation, |tx| {
        let candidate = approval::preflight_approve(configuration, decision, manifest, package)?;
        let original_approval = approval::read_original_on(tx, configuration, candidate.id())?;
        if let Some(original) = &original_approval {
            approval::require_same_original_request_on(tx, original.id(), decision, manifest)?;
        }
        let published = super::approved_package_publish::publish_on(
            tx, preparation, configuration, decision, manifest, package,
        )?;
        let approved = if let Some(original) = original_approval {
            original
        } else {
            approval::approve_on(tx, preparation, configuration, decision, manifest, package)?
        };
        if published.approval_id() != approved.id() {
            return Err("tool_package_publish_original_changed".into());
        }
        super::approved_package_source::ApprovedPackageSource::open_on(
            tx, configuration, approved.id(),
        )?;
        Ok(approved)
    })
}

pub(super) fn revoke(
    original: &Connection,
    preparation: &EnvironmentPreparationLease,
    configuration: &ToolAdministratorConfiguration,
    decision: &[u8],
) -> Result<(), String> {
    run(original, preparation, |tx| {
        approval::revoke_on(tx, preparation, configuration, decision)
    })
}

fn run<T>(
    original: &Connection,
    preparation: &EnvironmentPreparationLease,
    work: impl FnOnce(&Transaction<'_>) -> Result<T, String>,
) -> Result<T, String> {
    if !original.is_autocommit() {
        return Err("tool_administrator_outer_transaction_forbidden".into());
    }
    require_original_entity(original)?;
    let path = original
        .path()
        .filter(|path| !path.is_empty())
        .ok_or("tool_administrator_existing_database_required")?;
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| format!("tool_administrator_existing_database_open:{e}"))?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    db.pragma_update(None, "foreign_keys", true)
        .map_err(|e| e.to_string())?;
    db.pragma_update(None, "synchronous", "FULL")
        .map_err(|e| e.to_string())?;
    db.authorizer(Some(|context: AuthContext<'_>| authorize(context)))
        .map_err(|e| e.to_string())?;
    let tx = Transaction::new_unchecked(&db, TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let result: Result<T, String> = (|| {
        require_hook(&tx)?;
        require_original_entity(original)?;
        preparation.require_tool_supply_on(&tx)?;
        // approve/revoke verify exact compiled schema and reject all additional
        // decision-table triggers before attempting their sole INSERT.
        let value = work(&tx)?;
        preparation.require_tool_supply_on(&tx)?;
        require_hook(&tx)?;
        require_original_entity(original)?;
        Ok(value)
    })();
    match result {
        Ok(value) => {
            tx.commit()
                .map_err(|e| format!("tool_administrator_commit:{e}"))?;
            Ok(value)
        }
        Err(error) => {
            let rollback = tx.rollback();
            let error =
                if error.contains("not authorized") || error.contains("authorization denied") {
                    format!("tool_administrator_write_rejected:{error}")
                } else {
                    error
                };
            match rollback {
                Ok(()) => Err(error),
                Err(e) => Err(format!("{error};tool_administrator_rollback:{e}")),
            }
        }
    }
}
fn require_original_entity(db: &Connection) -> Result<(), String> {
    let mut moved: std::os::raw::c_int = 0;
    let result = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            db.handle(), c"main".as_ptr(), rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
            (&mut moved as *mut std::os::raw::c_int).cast(),
        )
    };
    if result != rusqlite::ffi::SQLITE_OK || moved != 0 {
        return Err("tool_administrator_original_database_changed".into());
    }
    Ok(())
}
fn require_hook(db: &Connection) -> Result<(), String> {
    match db.prepare("UPDATE main.environment_preparation_lease SET owner=owner WHERE 0") {
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::AuthorizationForStatementDenied =>
        {
            Ok(())
        }
        _ => Err("tool_administrator_writer_authority_lost".into()),
    }
}
fn authorize(context: AuthContext<'_>) -> Authorization {
    let allow = match context.action {
        AuthAction::Insert {
            table_name: "tool_administrator_decisions",
        } => context.database_name == Some("main") && context.accessor.is_none(),
        // The three exact compiled immutable triggers may SELECT and RAISE;
        // no trigger, including those names, receives any write authority.
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => true,
        _ => false,
    };
    if allow {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
