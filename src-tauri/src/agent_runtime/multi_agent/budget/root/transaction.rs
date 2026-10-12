//! Private Root accounting connections have no unrelated write authority.
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Transaction,
};

pub(super) fn protect<T>(
    tx: &Transaction<'_>,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    tx.authorizer(Some(authorize)).map_err(|e| e.to_string())?;
    let result = work();
    let clear = tx
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    let value = result?;
    clear?;
    Ok(value)
}

fn authorize(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Insert { table_name }
            if context.database_name == Some("main")
                && context.accessor.is_none()
                && matches!(
                    table_name,
                    "agent_root_budget_attempts"
                        | "agent_root_model_journal"
                        | "agent_root_model_cache_provenance"
                        | "agent_root_model_cache_capture"
                        | "agent_root_tick_receipts"
                        | "agent_budget_limits"
                        | "agent_budget_clock_origins"
                        | "agent_budget_entries"
                ) =>
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
    }
}
