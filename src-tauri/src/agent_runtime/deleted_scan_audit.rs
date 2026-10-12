//! Original Native deletion sources; private reconstruction is audit-only.
use crate::agent_runtime::{
    execution_owner::NativeInvocationOwner, multi_agent::budget::root::RootOwner, single_projection,
};
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
mod anchor;
mod delivery;
mod financial_schema;
mod multi;
mod rows;
mod scope;
mod source;
pub(crate) use source::OriginalSourceParents;
#[cfg(test)]
mod tests_archive_version;
#[cfg(test)]
#[path = "deleted_scan_audit/tests_mapper_diagnostic_identity.rs"]
mod tests_mapper_diagnostic_identity;
#[cfg(test)]
#[path = "deleted_scan_audit/tests_row_identity.rs"]
mod tests_row_identity;
mod writer;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    version: u8,
    scan: String,
    roots: Vec<String>,
    tables: Vec<rows::Table>,
}
pub(crate) struct Prepared {
    text: String,
    hash: String,
    bundle: Bundle,
    _owners: Vec<NativeInvocationOwner>,
    _specialists: Vec<crate::agent_runtime::multi_agent::specialist::IdleSpecialists>,
    outside: Vec<rows::Table>,
}
impl Bundle {
    fn verify(&self) -> Result<(), String> {
        if !matches!(self.version, 1 | 2) || self.scan.is_empty() || self.roots.is_empty() {
            return Err("deleted_audit_scope_invalid".into());
        }
        if self.version == 1 && self.tables.iter().any(|table| table.identity.is_some()) {
            return Err("deleted_audit_legacy_identity_invalid".into());
        }
        let db = Connection::open_in_memory().map_err(|e| e.to_string())?;
        // Private lossless reconstruction has column-only parent tables.
        // Original financial identity is checked below, never on a live DB.
        db.pragma_update(None, "foreign_keys", false)
            .map_err(|e| e.to_string())?;
        for table in &self.tables {
            if !scope::native(&table.name)
                || matches!(
                    table.name.as_str(),
                    "native_deleted_scan_audits"
                        | "native_deleted_scan_audits_v2"
                        | "native_deleted_scan_anchors"
                )
            {
                return Err("deleted_audit_table_invalid".into());
            }
            table.restore(&db)?;
        }
        db.pragma_update(None, "query_only", true)
            .map_err(|e| e.to_string())?;
        let mut q = db
            .prepare("SELECT id FROM agent_runs WHERE scan_id=?1 ORDER BY id")
            .map_err(|e| e.to_string())?;
        let roots = q
            .query_map([&self.scan], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if roots != self.roots {
            return Err("deleted_audit_root_scope_conflict".into());
        }
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        self.verify_runs(&tx)?;
        tx.rollback().map_err(|e| e.to_string())?;
        // No Connection, RootOwner or executable capability escapes this method.
        Ok(())
    }
    fn is_multi(&self) -> bool {
        self.tables
            .iter()
            .any(|t| t.name == "agent_multi_exit_receipts" && !t.rows.is_empty())
    }
    fn is_source(&self) -> bool {
        self.tables
            .iter()
            .any(|t| t.name == "source_scope_contracts" && !t.rows.is_empty())
    }
    fn retained(&self) -> impl Iterator<Item = &&'static str> {
        scope::RETAINED
            .iter()
            // Additive cache tables were absent from older signed audit bundles.
            // Preserve them when captured; never backfill or reseal old history.
            .filter(|name| {
                !matches!(
                    **name,
                    "agent_root_model_cache_provenance"
                        | "agent_frozen_tool_supply_plans"
                        | "native_tool_selection_requests"
                        | "agent_root_model_cache_capture"
                        | "native_sdk_log_mapper_owners"
                ) || self.tables.iter().any(|table| table.name == **name)
            })
            .chain(scope::MULTI_RETAINED.iter().filter(|name| {
                self.is_multi()
                    && (!matches!(
                        **name,
                        "agent_mapper_loop_journal"
                            | "agent_mapper_loop_final_receipts"
                            | "agent_mapper_loop_expired_receipts"
                    ) || self.tables.iter().any(|table| table.name == **name))
            }))
            .chain(scope::SOURCE_RETAINED.iter().filter(|_| self.is_source()))
    }
    fn verify_runs(&self, db: &Connection) -> Result<(), String> {
        let mut q=db.prepare("SELECT id,orchestration_policy,parent_run_id,root_run_id FROM agent_runs WHERE scan_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
        let runs = q
            .query_map([&self.scan], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if runs.iter().map(|r| r.0.clone()).collect::<Vec<_>>() != self.roots {
            return Err("deleted_audit_root_scope_conflict".into());
        }
        for (id, policy, parent, root) in &runs {
            if parent.is_none() { scope::verify_original_tool_supply(db, id)?; }
            match (policy.as_str(), parent) {
                ("single", None) => single_projection::deletion::verify_closed(db, id)?,
                ("multi", None) if root == id => multi::verify_closed(db, id)?,
                ("multi", Some(parent))
                    if parent == root
                        && runs.iter().any(|r| {
                            r.0 == *root && r.1 == "multi" && r.2.is_none() && r.3 == *root
                        }) => {}
                _ => return Err("deleted_audit_original_run_scope_invalid".into()),
            }
        }
        Ok(())
    }
    fn verify_retained(&self, db: &Connection) -> Result<(), String> {
        let current = scope::capture(db, &self.scan, &self.roots)?;
        for name in self.retained() {
            let t = self
                .tables
                .iter()
                .find(|t| t.name == *name)
                .ok_or("deleted_audit_retained_table_missing")?;
            // Accepted importer revisions are shared, never deleted. After the
            // Source receipt cascades, compare its captured physical rows only.
            let accepted;
            let original = if *name == "import_record_revisions" {
                accepted = rows::Table::read(db, name, &t.original_filter()?, &self.scan)?;
                &accepted
            } else {
                current
                    .iter()
                    .find(|t| t.name == *name)
                    .ok_or("deleted_audit_retained_table_missing")?
            };
            if original.canonical()? != t.canonical()? {
                return Err("deleted_audit_original_finance_conflict".into());
            }
        }
        self.verify()
    }
}
#[cfg(test)]
pub(crate) fn prepare(tx: &Transaction<'_>, scan: &str) -> Result<Prepared, String> {
    prepare_with_source_parents(tx, scan, OriginalSourceParents::default())
}
pub(crate) fn prepare_with_source_parents(
    tx: &Transaction<'_>,
    scan: &str,
    mut parents: OriginalSourceParents,
) -> Result<Prepared, String> {
    let mut q=tx.prepare("SELECT id,orchestration_policy,parent_run_id FROM agent_runs WHERE scan_id=?1 ORDER BY id").map_err(|e|e.to_string())?;
    let runs = q
        .query_map([scan], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if runs.is_empty() {
        return Err("deleted_audit_original_run_scope_invalid".into());
    }
    let mut owners = vec![];
    let mut specialists = vec![];
    for (root, policy, parent) in &runs {
        if parent.is_some() {
            continue;
        }
        scope::verify_original_tool_supply(tx, root)?;
        match policy.as_str() {
            "single" => {
                single_projection::deletion::verify_closed(tx, root)?;
                owners.extend(RootOwner::load_single(tx, root)?.require_single_deletion_idle(tx)?);
            }
            "multi" => {
                let (guards, proof) = multi::require_idle(tx, root, &mut parents)?;
                owners.extend(guards);
                specialists.push(proof);
            }
            _ => return Err("deleted_audit_original_run_scope_invalid".into()),
        }
    }
    owners.extend(parents.into_guards());
    let roots = runs.into_iter().map(|r| r.0).collect::<Vec<_>>();
    let bundle = Bundle {
        version: 2,
        scan: scan.into(),
        tables: scope::capture(tx, scan, &roots)?,
        roots,
    };
    let outside = bundle
        .tables
        .iter()
        .map(|t| rows::Table::read(tx, &t.name, &t.outside_filter()?, scan))
        .collect::<Result<Vec<_>, _>>()?;
    bundle.verify_retained(tx)?;
    let text = serde_json::to_string(&bundle).map_err(|e| e.to_string())?;
    let hash = crate::agent_runtime::store::stable_hash(&text);
    Ok(Prepared {
        text,
        hash,
        bundle,
        _owners: owners,
        _specialists: specialists,
        outside,
    })
}
impl Prepared {
    pub(crate) fn install_private_writer(&self, db: &Connection) -> Result<(), String> {
        writer::install(db, self)
    }
    fn verify_outside(&self, db: &Connection) -> Result<(), String> {
        for (t, expected) in self.bundle.tables.iter().zip(&self.outside) {
            if rows::Table::read(db, &t.name, &t.outside_filter()?, &self.bundle.scan)?
                .canonical()?
                != expected.canonical()?
            {
                return Err(format!("deleted_audit_unrelated_rows_changed:{}", t.name));
            }
        }
        Ok(())
    }
    pub(crate) fn verify_scope(&self, db: &Connection, scan: &str) -> Result<(), String> {
        if self.bundle.scan != scan {
            return Err("deleted_audit_scope_conflict".into());
        }
        self.bundle.verify_runs(db)?;
        self.bundle.verify_retained(db)
    }
    pub(crate) fn persist(&self, tx: &Transaction<'_>, stamp: &str) -> Result<(), String> {
        self.bundle.verify_retained(tx)?;
        if self.bundle.version != 2 {
            return Err("deleted_audit_new_version_invalid".into());
        }
        let conflict:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_deleted_scan_audits WHERE scan_id=?1) OR EXISTS(SELECT 1 FROM native_deleted_scan_audits_v2 WHERE scan_id=?1)",[&self.bundle.scan],|r|r.get(0)).map_err(|e|e.to_string())?;
        if conflict {
            return Err("deleted_audit_duplicate_archive".into());
        }
        let n=tx.execute("INSERT INTO native_deleted_scan_audits_v2(scan_id,version,audit_json,audit_hash,deleted_at) VALUES(?1,2,?2,?3,?4)",params![self.bundle.scan,self.text,self.hash,stamp]).map_err(|e|e.to_string())?;
        let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_deleted_scan_audits_v2 WHERE scan_id=?1 AND version=2 AND audit_json=?2 AND audit_hash=?3 AND deleted_at=?4)",params![self.bundle.scan,self.text,self.hash,stamp],|r|r.get(0)).map_err(|e|e.to_string())?;
        if n != 1 || !exact {
            return Err("deleted_audit_write_unconfirmed".into());
        }
        anchor::persist(tx, &self.bundle.scan, &self.hash, stamp)?;
        self.bundle.verify_retained(tx)
    }
    pub(crate) fn verify_deleted(&self, db: &Connection) -> Result<(), String> {
        if !verify_deleted(db, &self.bundle.scan)? {
            return Err("deleted_audit_original_sources_missing".into());
        }
        self.verify_outside(db)?;
        self.bundle.verify_retained(db)
    }
}
pub(crate) fn verify_deleted(db: &Connection, scan: &str) -> Result<bool, String> {
    // Old read-only databases may predate the additive v2 table. Reading
    // history never initializes schema or manufactures a new archive.
    let mut catalogue = db
        .prepare("SELECT type FROM main.sqlite_master WHERE name='native_deleted_scan_audits_v2'")
        .map_err(|e| e.to_string())?;
    let kinds = catalogue
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let sql=match kinds.as_slice() {
        []=>"SELECT 1,version,audit_json,audit_hash,deleted_at FROM native_deleted_scan_audits WHERE scan_id=?1",
        [kind] if kind=="table"=>"SELECT 1,version,audit_json,audit_hash,deleted_at FROM native_deleted_scan_audits WHERE scan_id=?1 UNION ALL SELECT 2,version,audit_json,audit_hash,deleted_at FROM native_deleted_scan_audits_v2 WHERE scan_id=?1",
        _=>return Err("deleted_audit_v2_schema_invalid".into()),
    };
    let mut statement = db.prepare(sql).map_err(|e| e.to_string())?;
    let saved = statement
        .query_map([scan], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if saved.len() > 1 {
        return Err("deleted_audit_duplicate_archive".into());
    }
    let saved = saved.into_iter().next();
    let Some((table_version, version, text, hash, stamp)) = saved else {
        let original:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_root_budget_attempts WHERE json_extract(contract_json,'$.root.scan')=?1)",[scan],|r|r.get(0)).map_err(|e|e.to_string())?;
        return if original {
            Err("deleted_audit_original_sources_missing".into())
        } else {
            Ok(false)
        };
    };
    let bundle: Bundle =
        serde_json::from_str(&text).map_err(|_| "deleted_audit_payload_invalid")?;
    let closed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1 AND deleted_at=?2) AND NOT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1)",params![scan,stamp],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !closed
        || version != table_version
        || i64::from(bundle.version) != version
        || !matches!(version, 1 | 2)
        || bundle.scan != scan
        || serde_json::to_string(&bundle).map_err(|e| e.to_string())? != text
        || crate::agent_runtime::store::stable_hash(&text) != hash
    {
        return Err("deleted_audit_deletion_conflict".into());
    }
    anchor::verify(db, scan, &hash, &stamp)?;
    bundle.verify_retained(db)?;
    Ok(true)
}
