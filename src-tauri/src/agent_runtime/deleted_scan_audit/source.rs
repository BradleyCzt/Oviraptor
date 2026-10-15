//! Closed Source has its own material/role/receipt proof, never a Web mode.
use crate::agent_runtime::{
    execution_owner::{self, NativeInvocationOwner},
    multi_agent::{budget, lease::CoordinatorLease, source_rounds, specialist},
};
use rusqlite::{params, Connection};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
// Fields are private: the sole insertion probes the actual original Source
// inode. A caller cannot substitute a boolean or a path-only exit attestation.
#[derive(Default)]
pub(crate) struct OriginalSourceParents(BTreeMap<(PathBuf, String, i64), NativeInvocationOwner>);
impl OriginalSourceParents {
    pub(crate) fn hold_existing(
        &mut self,
        path: &Path,
        scan: &str,
        attempt: i64,
    ) -> Result<(), String> {
        let key = (
            path.canonicalize().map_err(|e| e.to_string())?,
            scan.into(),
            attempt,
        );
        if self.0.contains_key(&key) {
            return Err("deleted_audit_source_parent_already_joined".into());
        }
        let owner = execution_owner::probe_native_invocation(
            path,
            scan,
            attempt,
            "source-model",
            "source",
        )?
        .ok_or("scan_quiescence_original_source_exit_missing")?;
        self.0.insert(key, owner);
        Ok(())
    }
    fn take(
        &mut self,
        path: &Path,
        scan: &str,
        attempt: i64,
    ) -> Result<Option<NativeInvocationOwner>, String> {
        Ok(self.0.remove(&(
            path.canonicalize().map_err(|e| e.to_string())?,
            scan.into(),
            attempt,
        )))
    }
    pub(super) fn into_guards(self) -> Vec<NativeInvocationOwner> {
        self.0.into_values().collect()
    }
}
pub(super) fn verify_closed(db: &Connection, actor: &CoordinatorLease) -> Result<(), String> {
    budget::clock::FinalClock::verify_original_exit(db, &actor.root_run_id)?;
    budget::admission::require_settled_for_completion(db, &actor.root_run_id)?;
    let incompatible:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM agent_root_model_journal WHERE root_run_id=?1)
        OR EXISTS(SELECT 1 FROM analyzer_container_receipts WHERE scan_id=?2 AND attempt_number=?3 AND cleanup_status<>'confirmed')",
        params![actor.root_run_id,actor.scan_id,actor.attempt_number],|r|r.get(0)).map_err(|e|e.to_string())?;
    if incompatible {
        return Err("deleted_audit_source_obligations_not_closed".into());
    }
    if crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?.is_some() {
        return Err("deleted_audit_source_web_mode_conflict".into());
    }
    crate::scanning::source_deletion::verify_closed_source_for_deletion(db, actor)
}
pub(super) fn require_idle(
    db: &Connection,
    actor: &CoordinatorLease,
    parents: &mut OriginalSourceParents,
) -> Result<(Vec<NativeInvocationOwner>, specialist::IdleSpecialists), String> {
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("deleted_audit_database_missing")?;
    let path = Path::new(path);
    let parent = match parents.take(path, &actor.scan_id, actor.attempt_number)? {
        Some(owner) => owner,
        None => execution_owner::probe_native_invocation(
            path,
            &actor.scan_id,
            actor.attempt_number,
            "source-model",
            "source",
        )?
        .ok_or("deleted_audit_original_source_exit_missing")?,
    };
    let mut guards = vec![parent];
    guards.extend(source_rounds::require_idle_for_root(db, actor)?);
    let specialists = specialist::require_idle_for_root(db, actor)?;
    Ok((guards, specialists))
}
