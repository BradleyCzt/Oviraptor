//! Native source closure; protected business/asset/CAS rows are never copied.
use super::rows::{quote, Table};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
pub(super) const RETAINED: &[&str] = &[
    "agent_frozen_tool_supply_plans",
    "native_tool_selection_requests",
    "agent_budget_entries",
    "agent_root_budget_attempts",
    "agent_root_model_journal",
    "agent_root_model_cache_provenance",
    "agent_root_model_cache_capture",
    "agent_budget_limits",
    "agent_budget_clock_origins",
    "agent_single_exit_receipts",
    "agent_single_projection_receipts",
    "agent_root_budget_definitions",
    "agent_root_mode_definitions",
    "native_web_mode_receipts",
    "native_web_mode_drafts",
    "native_web_mode_actions",
    "native_sdk_log_owners",
    "native_sdk_log_rows",
    "native_sdk_log_gaps",
    "native_sdk_log_mapper_owners",
];
pub(super) const MULTI_RETAINED: &[&str] = &[
    "agent_root_tick_receipts",
    "agent_root_tick_timeline_receipts",
    "agent_multi_exit_receipts",
    "agent_web_model_journal",
    "agent_model_cost_facts",
    "agent_assignment_attempts",
    "agent_assignment_replacements",
    "agent_root_elapsed_facts",
    "agent_mapper_loop_journal",
    "agent_mapper_loop_final_receipts",
    "agent_mapper_loop_expired_receipts",
];
// Finite Source material names; no generic source/import/asset/CAS wildcard.
pub(super) const SOURCE_RETAINED: &[&str] = &[
    "source_snapshots",
    "analyzer_runs",
    "analyzer_container_receipts",
    "import_record_revisions",
];
const SOURCE_ARCHIVED: &[&str] = &[
    "source_scope_contracts",
    "source_runtime_contracts",
    "source_analysis_views",
    "source_analysis_results",
    "source_ci_policies",
];
pub(super) fn native(name: &str) -> bool {
    name.starts_with("agent_")
        || name.starts_with("native_")
        || name.starts_with("sentinel_")
        || matches!(name, "tool_invocations" | "browser_auth_sessions")
        || SOURCE_RETAINED.contains(&name)
        || SOURCE_ARCHIVED.contains(&name)
}

/// Historical provenance only. Never consult mutable approval or issue a grant.
pub(super) fn verify_original_tool_supply(db: &Connection, root: &str) -> Result<(), String> {
    let (scan, attempt, text): (String, i64, String) = db.query_row(
        "SELECT scan_id,attempt_number,plan_json FROM agent_runs WHERE id=?1",
        [root], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(|e| e.to_string())?;
    let plan: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let selection = crate::tool_supply::selection::FrozenToolSelection::from_plan_value(plan.get("toolSupply"))?;
    let Some(selection) = selection else { return Ok(()); };
    let sealed = crate::tool_supply::frozen_plan::read_original_seal_on(db, root)?
        .ok_or("deleted_audit_original_tool_supply_missing")?;
    let policy = crate::commands::capture_original_tool_selection_policy_on(db, &scan, attempt)?;
    let policy: serde_json::Value = serde_json::from_str(policy.canonical_policy()).map_err(|e| e.to_string())?;
    let request = policy["toolSupplySelection"]["prepared"]["request"]["requestId"]
        .as_str().ok_or("deleted_audit_original_tool_selection_missing")?;
    let source = crate::tool_supply::selection::OriginalSignedToolSelectionPolicy::capture_original_on(db, &scan, attempt)?;
    let original = crate::tool_supply::selection::verify_signed_request_on(db, &source, request)?;
    let prepared = &original.fact()["snapshot"]["prepared"];
    if original.selection() != &selection
        || sealed.historical_fact()["tools"] != prepared["inventory"]
        || sealed.historical_fact()["configurationDigest"] != prepared["configurationDigest"]
    { return Err("deleted_audit_original_tool_supply_changed".into()); }
    Ok(())
}
type Foreign = BTreeMap<i64, Vec<(String, String, String, String)>>;
struct Meta {
    primary: Vec<String>,
    columns: Vec<String>,
    foreign: Foreign,
}
pub(super) struct Catalogue {
    tables: BTreeMap<String, Meta>,
    roots: String,
}
impl Catalogue {
    pub(super) fn read(db: &Connection, roots: &[String]) -> Result<Self, String> {
        let mut q=db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").map_err(|e|e.to_string())?;
        let names = q
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        let mut tables = BTreeMap::new();
        for name in names {
            let columns = db
                .prepare(&format!("SELECT * FROM {} LIMIT 0", quote(&name)))
                .map_err(|e| e.to_string())?
                .column_names()
                .iter()
                .map(|c| (*c).to_owned())
                .collect();
            let mut info = db
                .prepare(&format!("PRAGMA table_info({})", quote(&name)))
                .map_err(|e| e.to_string())?;
            let mut primary = info
                .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            primary.retain(|(ordinal, _)| *ordinal > 0);
            primary.sort_by_key(|(ordinal, _)| *ordinal);
            let primary = primary.into_iter().map(|(_, name)| name).collect();
            let mut foreign: Foreign = BTreeMap::new();
            let mut q = db
                .prepare(&format!("PRAGMA foreign_key_list({})", quote(&name)))
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                        r.get::<_, String>(6)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (id, parent, from, to, action) = row.map_err(|e| e.to_string())?;
                foreign
                    .entry(id)
                    .or_default()
                    .push((parent, from, to, action));
            }
            tables.insert(
                name,
                Meta {
                    primary,
                    columns,
                    foreign,
                },
            );
        }
        Ok(Self {
            tables,
            roots: roots
                .iter()
                .map(|r| format!("'{}'", r.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(","),
        })
    }
    fn predicate(&self, name: &str, seen: &mut BTreeSet<String>) -> Result<String, String> {
        if !seen.insert(name.into()) {
            return Ok("0".into());
        }
        if name == "import_record_revisions" {
            return Ok("id IN(SELECT json_extract(j.value,'$.record.revisionId') FROM source_analysis_results s,
                json_each(s.receipt_json,'$.records') j WHERE s.scan_id=?1)".into());
        }
        let meta = self
            .tables
            .get(name)
            .ok_or("deleted_audit_schema_scope_missing")?;
        let mut terms = vec![];
        if name == "sentinel_scans" {
            terms.push("id=?1".into());
        }
        for c in &meta.columns {
            if matches!(c.as_str(), "scan_id" | "owner_scan_id") {
                terms.push(format!("{}=?1", quote(c)));
            }
            if matches!(
                c.as_str(),
                "root_run_id" | "run_id" | "coordinator_run_id" | "child_run_id" | "parent_run_id"
            ) {
                terms.push(format!("{} IN({})", quote(c), self.roots));
            }
        }
        // The capture seal is keyed by the original call, without a synthetic
        // foreign key to a journal which has multiple phases per call.
        if name == "agent_root_model_cache_capture" {
            terms.push(format!(
                "call_id IN(SELECT call_id FROM agent_root_model_journal WHERE root_run_id IN({}))",
                self.roots
            ));
        }
        for edge in meta.foreign.values() {
            let parent = &edge[0].0;
            if !native(parent)
                || matches!(
                    parent.as_str(),
                    "native_deleted_scan_audits"
                        | "native_deleted_scan_audits_v2"
                        | "native_deleted_scan_anchors"
                )
            {
                continue;
            }
            let filter = self.predicate(parent, seen)?;
            let from = edge
                .iter()
                .map(|e| quote(&e.1))
                .collect::<Vec<_>>()
                .join(",");
            let keys = if edge.iter().all(|e| e.2.is_empty()) {
                let primary = &self
                    .tables
                    .get(parent)
                    .ok_or("deleted_audit_schema_scope_missing")?
                    .primary;
                if primary.len() != edge.len() {
                    return Err("deleted_audit_foreign_key_unresolved".into());
                }
                primary.clone()
            } else {
                if edge.iter().any(|e| e.2.is_empty()) {
                    return Err("deleted_audit_foreign_key_unresolved".into());
                }
                edge.iter().map(|e| e.2.clone()).collect()
            };
            let to = keys
                .iter()
                .map(|key| quote(key))
                .collect::<Vec<_>>()
                .join(",");
            terms.push(format!(
                "({from}) IN(SELECT {to} FROM {} WHERE {filter})",
                quote(parent)
            ));
        }
        seen.remove(name);
        Ok(if terms.is_empty() {
            "0".into()
        } else {
            format!("COALESCE(({}),0)", terms.join(" OR "))
        })
    }
    pub(super) fn filter(&self, name: &str) -> Result<String, String> {
        Ok(format!(
            "({}) AND ?1 IS NOT NULL",
            self.predicate(name, &mut BTreeSet::new())?
        ))
    }
    pub(super) fn capture(&self, db: &Connection, scan: &str) -> Result<Vec<Table>, String> {
        let mut tables = vec![];
        for (name, meta) in self.tables.iter().filter(|(n, _)| {
            native(n)
                && !matches!(
                    n.as_str(),
                    "native_deleted_scan_audits"
                        | "native_deleted_scan_audits_v2"
                        | "native_deleted_scan_anchors"
                )
        }) {
            let filter = self.filter(name)?;
            for c in meta
                .columns
                .iter()
                .filter(|c| matches!(c.as_str(), "scan_id" | "owner_scan_id"))
            {
                let conflict:bool=db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM {} WHERE {filter} AND {} IS NOT NULL AND {}<>'' AND {}<>?1)",quote(name),quote(c),quote(c),quote(c)),[scan],|r|r.get(0)).map_err(|e|e.to_string())?;
                if conflict {
                    return Err("deleted_audit_foreign_scan_source".into());
                }
            }
            tables.push(Table::read(db, name, &filter, scan)?);
        }
        Ok(tables)
    }
    pub(super) fn protected_cascades(
        &self,
        db: &Connection,
        scan: &str,
    ) -> Result<BTreeSet<String>, String> {
        let mut deleting = BTreeSet::from([
            "sentinel_scans".into(),
            "sentinel_validations".into(),
            "sentinel_opportunities".into(),
            "sentinel_findings".into(),
            "sentinel_checkpoints".into(),
            "sentinel_targets".into(),
            "sentinel_processes".into(),
            "browser_auth_sessions".into(),
        ]);
        loop {
            let before = deleting.len();
            for (name, meta) in &self.tables {
                if meta.foreign.values().any(|e| {
                    deleting.contains(&e[0].0)
                        && matches!(e[0].3.as_str(), "CASCADE" | "SET NULL" | "SET DEFAULT")
                }) {
                    deleting.insert(name.clone());
                }
            }
            if before == deleting.len() {
                break;
            }
        }
        let mut empty = BTreeSet::new();
        for name in deleting.iter().filter(|n| !native(n)) {
            let affected: bool = db
                .query_row(
                    &format!(
                        "SELECT EXISTS(SELECT 1 FROM {} WHERE {})",
                        quote(name),
                        self.filter(name)?
                    ),
                    [scan],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if affected {
                return Err("deleted_audit_protected_record_requires_preservation".into());
            }
            empty.insert(name.clone());
        }
        Ok(empty)
    }
}
pub(super) fn capture(db: &Connection, scan: &str, roots: &[String]) -> Result<Vec<Table>, String> {
    Catalogue::read(db, roots)?.capture(db, scan)
}
