//! Installed only on the deletion command's private connection, through COMMIT.
use super::{scope, Prepared};
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    Connection,
};
pub(super) fn install(db: &Connection, proof: &Prepared) -> Result<(), String> {
    let protected = scope::Catalogue::read(db, &proof.bundle.roots)?
        .protected_cascades(db, &proof.bundle.scan)?;
    let multi = proof.bundle.is_multi();
    let source = proof.bundle.is_source();
    let archive_table = match proof.bundle.version {
        1 => "native_deleted_scan_audits",
        2 => "native_deleted_scan_audits_v2",
        _ => return Err("deleted_audit_scope_invalid".into()),
    };
    db.authorizer(Some(move |c: AuthContext<'_>| {
        let direct = c.database_name == Some("main") && c.accessor.is_none();
        let allowed = match c.action {
            AuthAction::Insert { table_name } if direct => {
                table_name == archive_table
                    || matches!(
                        table_name,
                        "native_deleted_scan_anchors" | "sentinel_deleted_scans"
                    )
            }
            AuthAction::Update {
                table_name,
                column_name,
            } if direct => {
                (table_name == "sentinel_scans" && column_name == "previous_scan_id")
                    || (table_name == "agent_dialog_selections" && column_name == "scan_id")
                    || protected.contains(table_name)
            }
            AuthAction::Delete { table_name } if direct => {
                (scope::native(table_name)
                    && !scope::RETAINED.contains(&table_name)
                    && !(multi && scope::MULTI_RETAINED.contains(&table_name))
                    && table_name != "import_record_revisions"
                    && !(source && scope::SOURCE_RETAINED.contains(&table_name))
                    && !matches!(
                        table_name,
                        "native_deleted_scan_audits"
                            | "native_deleted_scan_audits_v2"
                            | "native_deleted_scan_anchors"
                            | "sentinel_deleted_scans"
                    ))
                    || protected.contains(table_name)
            }
            AuthAction::Pragma {
                pragma_name: "foreign_key_list" | "table_info" | "table_list",
                ..
            } => true,
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
    }))
    .map_err(|e| e.to_string())
}
