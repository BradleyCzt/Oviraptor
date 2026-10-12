//! Finite withdrawal authority on the supervisor's own private connection.
//! No budget, new grant, target action or arbitrary trigger writes are allowed.
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection,
};

// A token can only be borrowed inside this finite private writer. It is not
// Clone/Deserialize and cannot be reconstructed from a restored expired row.
pub(in crate::agent_runtime::multi_agent) struct ExpiryWriteAuthority {
    connection: usize,
}
impl ExpiryWriteAuthority {
    pub(in crate::agent_runtime::multi_agent) fn require_snapshot(
        &self,
        db: &Connection,
    ) -> Result<(), String> {
        if db.is_autocommit() || self.connection != db as *const Connection as usize {
            return Err("worker_expiry_private_writer_required".into());
        }
        Ok(())
    }
}

pub(super) fn write<T>(
    db: &mut Connection,
    work: impl FnOnce(&mut Connection, &ExpiryWriteAuthority) -> Result<T, String>,
) -> Result<T, String> {
    if !db.is_autocommit() {
        return Err("worker_supervisor_private_transaction_required".into());
    }
    crate::collaboration_events::closure_schema::verify(db)?;
    db.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
    let authority = ExpiryWriteAuthority {
        connection: db as *const Connection as usize,
    };
    let result = work(db, &authority);
    // This connection belongs to WorkerSupervisor, never to an IPC caller.
    // Clear only our finite authority before the next Coordinator heartbeat.
    db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string())?;
    result
}

fn authorize(c: AuthContext<'_>) -> Authorization {
    let main = c.database_name == Some("main") && c.accessor.is_none();
    let allowed = match c.action {
        AuthAction::Insert {
            table_name: "agent_collaboration_events",
        } if c.database_name == Some("main") => matches!(
            c.accessor,
            Some("agent_collaboration_run_update" | "agent_collaboration_assignment_update")
        ),
        AuthAction::Insert {
            table_name: "agent_worker_expiry_receipts" | "agent_worker_expiry_issuance_seals",
        } if main => true,
        AuthAction::Update {
            table_name,
            column_name,
        } if main => match table_name {
            "agent_assignment_attempts" => {
                matches!(column_name, "state" | "finished_at" | "failure_class")
            }
            "agent_assignments" => matches!(column_name, "state" | "failure_class" | "updated_at"),
            "agent_runs" => matches!(column_name, "status" | "updated_at"),
            "agent_capability_leases" => column_name == "revoked_at",
            _ => false,
        },
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
