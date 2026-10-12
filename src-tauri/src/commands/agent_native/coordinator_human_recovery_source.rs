// Original pending priced approval source only. This is neither a fresh birth
// nor an execution grant, and does not claim complete ten-dimensional pricing.
// Kept as the Phase 2 recovery-audit surface; production wiring lands with the
// human-recovery lane, so the compiler-facing readers do not exist yet.
#[allow(dead_code)]
struct NativeHumanRecoverySource {
    connection: usize,
    db_path: std::path::PathBuf,
    request: String,
    selection: NativeProtectionRecoverySelection,
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    approval: JsonValue,
    history: NativeRecoveryHistory,
    fact: JsonValue,
    _target_exits: Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
}
#[allow(dead_code)]
impl NativeHumanRecoverySource {
    fn fact(&self) -> &JsonValue {
        &self.fact
    }
    fn require_on(&self, db: &rusqlite::Connection, path: &Path) -> Result<(), String> {
        if db.is_autocommit() {
            return Err("root_human_recovery_snapshot_required".into());
        }
        if self.connection != db as *const rusqlite::Connection as usize || self.db_path != path {
            return Err("root_human_recovery_original_connection_changed".into());
        }
        let approval = native_priced_recovery_saved_on(db, &self.request)?
            .ok_or("root_human_recovery_original_approval_missing")?;
        if approval != self.approval {
            return Err("root_human_recovery_original_approval_changed".into());
        }
        if native_priced_recovery_cancel_saved_on(db, &self.request)?.is_some() {
            return Err("protection_recovery_priced_approval_revoked".into());
        }
        let actor = native_recovery_original_actor_on(
            db,
            &self.selection.scan_id,
            self.selection.attempt_number,
            &self.selection.root_run_id,
        )?;
        if actor.scan_id != self.actor.scan_id
            || actor.attempt_number != self.actor.attempt_number
            || actor.target_key != self.actor.target_key
            || actor.root_run_id != self.actor.root_run_id
            || actor.lease_epoch != self.actor.lease_epoch
            || actor.fencing_token != self.actor.fencing_token
        {
            return Err("root_human_recovery_original_scope_changed".into());
        }
        self.history
            .verify_on(db, path, &self.actor, &self.selection.protection_receipt_id)?;
        if self.history.bounded_fact() != self.approval["proposal"]["originalHistory"] {
            return Err("root_human_recovery_approved_history_changed".into());
        }
        Ok(())
    }
}
#[allow(dead_code)]
fn native_human_recovery_source_on(
    db: &rusqlite::Connection,
    path: &Path,
    request: &str,
    selection: &NativeProtectionRecoverySelection,
) -> Result<NativeHumanRecoverySource, String> {
    if db.is_autocommit() {
        return Err("root_human_recovery_snapshot_required".into());
    }
    let approval = native_priced_recovery_saved_on(db, request)?
        .ok_or("root_human_recovery_original_approval_missing")?;
    if approval["proposal"]["selection"] != json!(selection) {
        return Err("root_human_recovery_original_scope_conflict".into());
    }
    if native_priced_recovery_cancel_saved_on(db, request)?.is_some() {
        return Err("protection_recovery_priced_approval_revoked".into());
    }
    let actor = native_recovery_original_actor_on(
        db,
        &selection.scan_id,
        selection.attempt_number,
        &selection.root_run_id,
    )?;
    let history = native_recovery_history_on(db, path, &actor, &selection.protection_receipt_id)?;
    if history.bounded_fact() != approval["proposal"]["originalHistory"] {
        return Err("root_human_recovery_approved_history_changed".into());
    }
    let target_exits = history.require_original_target_exits_on(db, path, &selection.scan_id)?;
    let fact = json!({"schemaVersion":1,"purpose":"original_pending_priced_human_recovery_source",
        "approval":approval,"originalHistory":history.bounded_fact(),
        "executionAllowed":false,"freshBirthImplemented":false,
        "completeTenDimensionPricingVerified":false,"targetRequestsGranted":0,
        "oldScopeRenewed":false});
    let proof = NativeHumanRecoverySource {
        connection: db as *const rusqlite::Connection as usize,
        db_path: path.to_path_buf(),
        request: request.into(),
        selection: selection.clone(),
        actor,
        approval,
        history,
        fact,
        _target_exits: target_exits,
    };
    proof.require_on(db, path)?;
    Ok(proof)
}
