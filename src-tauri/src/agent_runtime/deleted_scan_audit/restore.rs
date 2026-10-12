//! Trusted private reconstruction; never executes archived DDL.
use super::{quote, RowIdentity, Table};
use rusqlite::Connection;


impl Table {
    pub fn restore(&self, db: &Connection) -> Result<(), String> {
        self.validate_identity()?;
        let trusted = match self.name.as_str() {
            "agent_frozen_tool_supply_plans" => Some(include_str!("../../tool_supply/frozen_plan/schema.sql")),
            "native_tool_selection_requests" => Some(include_str!("../../tool_supply/selection/schema.sql")),
            "agent_multi_exit_receipts" => Some(include_str!(
                "../multi_agent/budget/clock/finalization/exit_receipt/schema.sql"
            )),
            "agent_root_model_cache_provenance" => Some(include_str!(
                "../multi_agent/budget/root_cache_provenance_schema.sql"
            )),
            "agent_root_model_cache_capture" => Some(include_str!(
                "../multi_agent/budget/root_cache_capture_schema.sql"
            )),
            "agent_http_budget_receipts" => {
                Some(include_str!("../web_mode/http_budget_receipt.sql"))
            }
            "agent_web_expiry_deliveries" => {
                Some(include_str!("../web_mode/web_expiry_delivery.sql"))
            }
            "agent_web_capability_transition_receipts" => Some(include_str!(
                "../web_mode/capability_transition_receipt.sql"
            )),
            "agent_authorization_output_receipts" => Some(include_str!(
                "../../commands/agent_native/coordinator_authorization_output_receipt.sql"
            )),
            "agent_public_surface_response_receipts" => Some(include_str!(
                "../../commands/agent_native/external_surface_response_receipt.sql"
            )),
            "agent_external_surface_output_receipts" => Some(include_str!(
                "../../commands/agent_native/external_surface_output_receipt.sql"
            )),
            "agent_business_logic_output_receipts" => Some(include_str!(
                "../../commands/agent_native/business_logic_output_receipt.sql"
            )),
            "agent_business_logic_dispatch_receipts" => Some(include_str!(
                "../../commands/agent_native/business_logic_dispatch_receipt.sql"
            )),
            "agent_mapper_loop_journal" => Some(include_str!(
                "../multi_agent/specialist/mapper_loop/schema.sql"
            )),
            "agent_mapper_loop_final_receipts" => Some(include_str!(
                "../multi_agent/specialist/mapper_loop/final_receipt/schema.sql"
            )),
            "agent_mapper_loop_expired_receipts" => Some(include_str!(
                "../multi_agent/specialist/mapper_loop/expired_receipt/schema.sql"
            )),
            "native_sdk_log_mapper_owners" => Some(
                include_str!("../model/diagnostics/schema.sql")
                    .split_once("-- Additive logical Mapper ownership; original owner family/cursors stay unchanged.")
                    .ok_or("deleted_audit_mapper_diagnostic_schema_missing")?.1,
            ),
            _ => None,
        };
        if let Some(schema) = trusted {
            db.execute_batch(schema).map_err(|e| e.to_string())?;
            let (identity, columns, _) = Self::original_layout(db, &self.name)?;
            if identity != self.identity || columns != self.columns {
                return Err("deleted_audit_original_schema_identity_changed".into());
            }
        } else {
            // Legacy rowid tables retain their original column-only arena.
            // An unknown archived PK is never a source of trusted schema.
            if !matches!(self.effective_identity(), RowIdentity::Rowid) || self.identity.is_some() {
                return Err("deleted_audit_primary_key_schema_unknown".into());
            }
            db.execute_batch(&format!(
                "CREATE TABLE {}({})",
                quote(&self.name),
                self.columns[1..]
                    .iter()
                    .map(|c| quote(c))
                    .collect::<Vec<_>>()
                    .join(",")
            ))
            .map_err(|e| e.to_string())?;
        }
        let sql = format!(
            "INSERT INTO {}({}) VALUES({})",
            quote(&self.name),
            self.columns
                .iter()
                .map(|c| quote(c))
                .collect::<Vec<_>>()
                .join(","),
            vec!["?"; self.columns.len()].join(",")
        );
        for row in &self.rows {
            db.execute(
                &sql,
                rusqlite::params_from_iter(row.iter().map(|c| c.value())),
            )
            .map_err(|e| e.to_string())?;
        }
        if self.name == "agent_budget_entries" {
            // The historical rowid arena keeps its original column layout.
            // Original Mapper allocations also require these exact immutable
            // invoice guards. These are compiled source DDL, never bundle SQL.
            db.execute_batch(
                "CREATE TRIGGER budget_entry_no_update BEFORE UPDATE ON agent_budget_entries BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END;
                 CREATE TRIGGER budget_entry_no_delete BEFORE DELETE ON agent_budget_entries BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END;
                 CREATE TRIGGER budget_entry_no_replace BEFORE INSERT ON agent_budget_entries WHEN EXISTS(SELECT 1 FROM agent_budget_entries WHERE entry_id=NEW.entry_id OR (root_run_id=NEW.root_run_id AND idempotency_key=NEW.idempotency_key)) BEGIN SELECT RAISE(ABORT,'budget_entry_immutable'); END;",
            ).map_err(|e| e.to_string())?;
        }
        let emitters: &[&str] = match self.name.as_str() {
            "agent_runs" => &["agent_collaboration_run_update"],
            "agent_assignments" => &["agent_collaboration_assignment_update"],
            "agent_messages" => &[
                "agent_collaboration_message_insert",
                "agent_collaboration_message_update",
            ],
            _ => &[],
        };
        // Install only after inserting the original rows: replay must never
        // emit new collaboration events. Extract exact compiled emitter DDL
        // from the same source checked by client_delivery_schema::verify.
        let source = include_str!("../../collaboration_events.rs");
        for name in emitters {
            let start = source
                .find(&format!("CREATE TRIGGER IF NOT EXISTS {name}\n"))
                .ok_or("deleted_audit_original_delivery_schema_missing")?;
            let tail = &source[start..];
            let end = tail
                .find("\nEND;")
                .ok_or("deleted_audit_original_delivery_schema_missing")?
                + 5;
            db.execute_batch(&tail[..end]).map_err(|e| e.to_string())?;
        }
        // Preserve ordered keys and every value, including original physical
        // rowids. Rehashed archives cannot replace the trusted PK definition.
        let restored = Self::read(db, &self.name, "?1 IS NOT NULL", "original-private-arena")?;
        if restored.canonical()? != self.canonical()? {
            return Err("deleted_audit_original_rows_changed".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tool_supply_schema_tests {
    use super::*;
    #[test]
    fn deleted_audit_tool_supply_private_restore_preserves_compiled_schema() {
        for (name, schema) in [
            ("agent_frozen_tool_supply_plans", include_str!("../../tool_supply/frozen_plan/schema.sql")),
            ("native_tool_selection_requests", include_str!("../../tool_supply/selection/schema.sql")),
        ] {
            let source = Connection::open_in_memory().unwrap();
            source.execute_batch(schema).unwrap();
            let original = Table::read(&source, name, "?1 IS NOT NULL", "fixture").unwrap();
            let arena = Connection::open_in_memory().unwrap();
            original.restore(&arena).unwrap();
            assert_eq!(
                serde_json::to_string(&Table::read(&arena, name, "?1 IS NOT NULL", "fixture").unwrap()).unwrap(),
                serde_json::to_string(&original).unwrap(),
            );
            let ddl = |db: &Connection| {
                let mut statement = db.prepare("SELECT type,name,sql FROM main.sqlite_master WHERE tbl_name=?1 ORDER BY type,name").unwrap();
                statement.query_map([name], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))
                    .unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap()
            };
            assert_eq!(ddl(&arena), ddl(&source));
        }
    }
    #[test]
    fn deleted_audit_tool_supply_retains_captured_tables_without_old_bundle_backfill() {
        for version in [1, 2] {
            let mut bundle = super::super::super::Bundle {
                version, scan: "fixture".into(), roots: vec![], tables: vec![],
            };
            let original = serde_json::to_string(&bundle).unwrap();
            for (name, schema) in [
                ("agent_frozen_tool_supply_plans", include_str!("../../tool_supply/frozen_plan/schema.sql")),
                ("native_tool_selection_requests", include_str!("../../tool_supply/selection/schema.sql")),
            ] {
                assert!(!bundle.retained().any(|retained| *retained == name));
                assert_eq!(serde_json::to_string(&bundle).unwrap(), original);
                let source = Connection::open_in_memory().unwrap();
                source.execute_batch(schema).unwrap();
                bundle.tables.push(Table::read(&source, name, "?1 IS NOT NULL", "fixture").unwrap());
                assert!(bundle.retained().any(|retained| *retained == name));
                bundle.tables.clear();
            }
        }
    }
}

