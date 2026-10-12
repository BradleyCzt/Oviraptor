// Scan-wide historical obligations, separate from selected stopped-response
// authority. Inventory comes from independent durable owners, not a UI list.
struct NativeRecoveryHistory {
    bounded: JsonValue,
    rows: Vec<NativeCoordinatorFrozenRows>,
    _selected: NativeRecoverySource,
    _historical_exit_guards: Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
}
impl NativeRecoveryHistory {
    fn bounded_fact(&self) -> JsonValue {
        self.bounded.clone()
    }
    fn verification_rows(&self) -> &[NativeCoordinatorFrozenRows] {
        &self.rows
    }
    // Authorization of new work also closes the outer pipeline, beyond Root/SDK
    // ownership. Read-only history diagnostics do not acquire these extra locks.
    // Every inventoried Root needs its original target inode; absent evidence is
    // refused even for an apparently unpaid Root rather than manufacturing it.
    fn require_original_target_exits_on(
        &self,
        db: &rusqlite::Connection,
        db_path: &Path,
        scan: &str,
    ) -> Result<Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>, String> {
        let roots = native_recovery_history_roots_on(db, scan)?;
        if json!(roots) != self.bounded["historicalRootIds"] {
            return Err("recovery_history_inventory_changed".into());
        }
        let mut targets = std::collections::BTreeSet::new();
        for root in roots {
            // Financial owner verification binds these rows to original scope;
            // Root birth verification was performed by the historical proof.
            crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(db, &root)?;
            let (attempt, target): (i64, String) = db.query_row(
                "SELECT attempt_number,target_url FROM main.agent_runs WHERE id=?1 AND scan_id=?2",
                params![root, scan], |r| Ok((r.get(0)?, r.get(1)?)),
            ).map_err(|_| "recovery_history_original_target_scope_missing")?;
            if attempt <= 0 || target.is_empty() {
                return Err("recovery_history_original_target_scope_missing".into());
            }
            targets.insert((attempt, target));
        }
        let mut guards = Vec::with_capacity(targets.len());
        for (attempt, target) in targets {
            guards.push(
                crate::agent_runtime::execution_owner::probe_native_invocation(
                    db_path, scan, attempt, "target", &target,
                )
                .map_err(|e| format!("recovery_history_target_not_idle:{e}"))?
                .ok_or("recovery_history_original_target_exit_missing")?,
            );
        }
        Ok(guards)
    }

    // The guards are already held. Recheck without probing our own inodes.
    fn verify_on(
        &self,
        db: &rusqlite::Connection,
        db_path: &Path,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        receipt: &str,
    ) -> Result<(), String> {
        for row in &self.rows {
            row.verify(db)?;
        }
        let roots = native_recovery_history_roots_on(db, &actor.scan_id)?;
        if json!(roots) != self.bounded["historicalRootIds"]
            || native_audit_closed_protection_response_at_on(db, db_path, actor, receipt)?.bounded
                != self.bounded["selectedSource"]["originalProtection"]
        {
            return Err("recovery_history_selected_source_changed".into());
        }
        let proofs = self.bounded["historicalProofs"]
            .as_array()
            .ok_or("recovery_history_original_finance_changed")?;
        if roots.len() != proofs.len() {
            return Err("recovery_history_inventory_changed".into());
        }
        for (root, original) in roots.iter().zip(proofs) {
            let attempt: i64 = db
                .query_row(
                    "SELECT attempt_number FROM main.agent_runs WHERE id=?1",
                    [root],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            native_recovery_history_verify_birth_on(db, &actor.scan_id, attempt, root)?;
            if crate::agent_runtime::multi_agent::budget::continuation::verify_settled_history_on(
                db, root,
            )? != *original
            {
                return Err("recovery_history_original_finance_changed".into());
            }
        }
        Ok(())
    }
}

fn native_recovery_history_roots_on(
    db: &rusqlite::Connection,
    scan: &str,
) -> Result<Vec<String>, String> {
    if db.is_autocommit() {
        return Err("recovery_history_snapshot_required".into());
    }
    let orphan:bool=db.query_row(
        "SELECT EXISTS(SELECT 1 FROM (
            SELECT run_id child FROM main.agent_http_request_claims WHERE scan_id=?1
            UNION SELECT child_run_id child FROM main.agent_external_surface_captures WHERE scan_id=?1
            UNION SELECT child_run_id child FROM main.agent_authorization_probe_claims WHERE scan_id=?1
        ) c LEFT JOIN main.agent_runs r ON r.id=c.child
        WHERE r.id IS NULL OR (r.role<>'coordinator' AND (r.root_run_id='' OR r.parent_run_id IS NULL)))",
        [scan],|r|r.get(0)).map_err(|e|format!("recovery_history_target_inventory_unavailable:{e}"))?;
    if orphan {
        return Err("recovery_history_original_target_owner_missing".into());
    }
    let mut query = db
        .prepare(
            "SELECT id FROM main.agent_runs WHERE scan_id=?1 AND backend='native'
          AND (role='coordinator' OR parent_run_id IS NULL OR assignment_id='')
         UNION SELECT root_run_id FROM main.agent_runs WHERE scan_id=?1 AND root_run_id<>''
         UNION SELECT CASE WHEN r.role='coordinator' THEN r.id ELSE r.root_run_id END
           FROM (SELECT run_id child FROM main.agent_http_request_claims WHERE scan_id=?1
             UNION SELECT child_run_id child FROM main.agent_external_surface_captures WHERE scan_id=?1
             UNION SELECT child_run_id child FROM main.agent_authorization_probe_claims WHERE scan_id=?1) c
           JOIN main.agent_runs r ON r.id=c.child
         UNION SELECT root_run_id FROM main.agent_coordinator_leases WHERE scan_id=?1
         UNION SELECT root_run_id FROM main.agent_root_budget_attempts
           WHERE json_extract(contract_json,'$.root.scan')=?1
         UNION SELECT root_run_id FROM main.agent_root_budget_definitions
           WHERE json_extract(definition_json,'$.scanId')=?1
         UNION SELECT root_run_id FROM main.agent_financial_root_budget_definitions
           WHERE json_extract(definition_json,'$.declaration.scanId')=?1
         UNION SELECT json_extract(definition_json,'$.sourceRootRunId') FROM main.agent_financial_root_budget_definitions
           WHERE json_extract(definition_json,'$.declaration.scanId')=?1
         UNION SELECT root_run_id FROM main.agent_root_mode_definitions
           WHERE json_extract(definition_json,'$.fact.scanId')=?1
         UNION SELECT root_run_id FROM main.agent_root_mode_birth_seals
           WHERE json_extract(binding_json,'$.definition.fact.scanId')=?1
         UNION SELECT root_run_id FROM main.agent_root_elapsed_facts WHERE scan_id=?1
         UNION SELECT root_run_id FROM main.agent_protection_response_receipts WHERE scan_id=?1
         UNION SELECT source_root_run_id FROM main.agent_financial_continuations
           WHERE scan_id=?1 OR json_extract(fact_json,'$.scanId')=?1
         UNION SELECT new_root_run_id FROM main.agent_financial_continuations
           WHERE scan_id=?1 OR json_extract(fact_json,'$.scanId')=?1
         UNION SELECT root_run_id FROM main.agent_protection_response_receipts
           WHERE json_extract(fact_json,'$.scanId')=?1
         UNION SELECT json_extract(fact_json,'$.rootRunId') FROM main.agent_protection_response_receipts
           WHERE json_extract(fact_json,'$.scanId')=?1
         UNION SELECT json_extract(binding_json,'$.rootRunId') FROM main.agent_root_mode_birth_seals
           WHERE json_extract(binding_json,'$.definition.fact.scanId')=?1
         UNION SELECT json_extract(contract_json,'$.root.declaredRoot') FROM main.agent_root_budget_attempts
           WHERE json_extract(contract_json,'$.root.scan')=?1 AND json_extract(contract_json,'$.root.declaredRoot')<>''
         UNION SELECT json_extract(fact_json,'$.sourceRootRunId') FROM main.agent_financial_continuations
           WHERE json_extract(fact_json,'$.scanId')=?1
         UNION SELECT json_extract(fact_json,'$.newRootRunId') FROM main.agent_financial_continuations
           WHERE json_extract(fact_json,'$.scanId')=?1
         UNION SELECT root_run_id FROM main.agent_root_tick_receipts
           WHERE phase='request' AND json_extract(fact_json,'$.request.owner.root.scan')=?1
         UNION SELECT root_run_id FROM main.agent_root_cost_reconciliation_receipts
           WHERE scan_id=?1 OR json_extract(fact_json,'$.preview.scanId')=?1 OR json_extract(fact_json,'$.receipt.scanId')=?1
         UNION SELECT json_extract(fact_json,'$.preview.rootRunId') FROM main.agent_root_cost_reconciliation_receipts
           WHERE json_extract(fact_json,'$.preview.scanId')=?1
         ORDER BY 1",
        )
        .map_err(|e| format!("recovery_history_inventory_unavailable:{e}"))?;
    let roots = query
        .query_map([scan], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if roots.is_empty() || roots.len() > 256 || roots.iter().any(|r| r.is_empty()) {
        return Err("recovery_history_inventory_invalid".into());
    }
    Ok(roots)
}

fn native_recovery_history_on(
    db: &rusqlite::Connection,
    db_path: &Path,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    receipt: &str,
) -> Result<NativeRecoveryHistory, String> {
    use crate::agent_runtime::multi_agent::budget;
    let selected = native_recovery_source_at_on(db, db_path, actor, receipt)?;
    let roots = native_recovery_history_roots_on(db, &actor.scan_id)?;
    if !roots.contains(&actor.root_run_id) {
        return Err("recovery_history_selected_root_missing".into());
    }
    let mut rows = selected.verification_rows().to_vec();
    let mut proofs = Vec::new();
    let mut guards = Vec::new();
    for root in &roots {
        let scope: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM main.agent_runs WHERE id=?1 AND scan_id=?2
              AND attempt_number>0 AND backend='native' AND role='coordinator'
              AND parent_run_id IS NULL AND assignment_id=''
              AND (root_run_id=id OR (root_run_id='' AND orchestration_policy='single')))",
                params![root, actor.scan_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !scope {
            return Err("recovery_history_original_scope_missing".into());
        }
        let (attempt, policy): (i64, String) = db
            .query_row(
                "SELECT attempt_number,orchestration_policy FROM main.agent_runs WHERE id=?1",
                [root],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        native_recovery_history_verify_birth_on(db, &actor.scan_id, attempt, root)?;
        if policy == "multi" {
            // A paid SDK may have returned while its parent service still owns
            // orchestration. Probe that original inode too; never create it.
            let parent = crate::agent_runtime::execution_owner::probe_native_invocation(
                db_path,
                &actor.scan_id,
                attempt,
                "parent-supervisor",
                root,
            )
            .map_err(|e| format!("recovery_history_parent_not_idle:{e}"))?
            .ok_or("recovery_history_original_parent_exit_missing")?;
            guards.push(parent);
        }
        let mut single_source = None;
        if root != &actor.root_run_id {
            let pending:bool=db.query_row(
                "SELECT EXISTS(SELECT 1 FROM main.agent_assignment_attempts WHERE root_run_id=?1
                    AND (state IN ('leased','running','paused') OR finished_at=''))
                 OR EXISTS(SELECT 1 FROM main.agent_runs WHERE root_run_id=?1 AND id<>?1
                    AND (status<>'terminal' OR finished_at=''))
                 OR EXISTS(SELECT 1 FROM main.agent_capability_leases WHERE root_run_id=?1 AND revoked_at='')
                 OR EXISTS(SELECT 1 FROM main.agent_lane_leases l JOIN main.agent_assignments a ON a.id=l.assignment_id WHERE a.coordinator_run_id=?1)
                 OR EXISTS(SELECT 1 FROM main.tool_invocations WHERE run_id IN
                    (SELECT id FROM main.agent_runs WHERE id=?1 OR root_run_id=?1) AND status='running')",
                [root],|r|r.get(0)).map_err(|e|e.to_string())?;
            if pending {
                return Err("recovery_history_original_work_not_closed".into());
            }
            let root_calls:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_root_model_journal WHERE root_run_id=?1)",
                [root],|r|r.get(0)).map_err(|e|e.to_string())?;
            if policy == "single" {
                let owner = budget::root::RootOwner::load_original(db, root)?;
                guards.extend(owner.require_single_deletion_idle(db)?);
                single_source = Some(budget::continuation::single::original_source_on(db, root)?);
            } else if policy == "multi" && root_calls {
                let old = db.query_row(
                    "SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
                     FROM main.agent_coordinator_leases WHERE root_run_id=?1",[root],
                    |r|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {
                        scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,
                        lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
                    })).map_err(|_|"recovery_history_original_coordinator_missing")?;
                budget::root::RootOwner::load_original(db, root)?
                    .require_original_coordinator(db, &old)?;
                let guard = budget::root::model::tick::require_idle_original(db, &old)?
                    .ok_or("recovery_history_original_sdk_exit_missing")?;
                guards.push(guard);
                native_recovery_require_quiescent_children(db, &old)?;
                native_recovery_original_root_calls(db, root)?;
            }
        }
        proofs.push(match single_source {
            Some(source) => source.history(),
            None => budget::continuation::verify_settled_history_on(db, root)?,
        });
        for sql in [
            "SELECT rowid,* FROM main.agent_runs WHERE id=?1 OR root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_root_budget_attempts WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_root_budget_definitions WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_root_mode_definitions WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_root_mode_birth_seals WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_root_model_journal WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_coordinator_leases WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_budget_entries WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_root_cost_reconciliation_receipts WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_financial_root_budget_definitions WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_financial_continuations WHERE source_root_run_id=?1 OR new_root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_single_exit_receipts WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_root_tick_receipts WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_budget_limits WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_assignment_attempts WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_assignments WHERE coordinator_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_capability_leases WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT l.rowid,l.* FROM main.agent_lane_leases l JOIN main.agent_assignments a ON a.id=l.assignment_id WHERE a.coordinator_run_id=?1 ORDER BY l.rowid",
            "SELECT rowid,* FROM main.agent_messages WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_root_tick_terminal_proofs WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_specialist_calls WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_source_model_rounds WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid",
            "SELECT rowid,* FROM main.agent_model_cost_facts WHERE root_run_id=?1 ORDER BY rowid",
        ] { rows.push(NativeCoordinatorFrozenRows::capture_optional(db,sql,&[root])?); }
    }
    for row in &rows {
        row.verify(db)?;
    }
    if native_recovery_history_roots_on(db, &actor.scan_id)? != roots {
        return Err("recovery_history_inventory_changed".into());
    }
    // Recheck external original files after the complete inventory audit;
    // SQLite's snapshot alone cannot freeze private task/response bytes.
    if native_audit_closed_protection_response_at_on(db, db_path, actor, receipt)?.bounded
        != selected.bounded_fact()["originalProtection"]
    {
        return Err("recovery_history_selected_source_changed".into());
    }
    for (root, original) in roots.iter().zip(&proofs) {
        let attempt: i64 = db
            .query_row(
                "SELECT attempt_number FROM main.agent_runs WHERE id=?1",
                [root],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        native_recovery_history_verify_birth_on(db, &actor.scan_id, attempt, root)?;
        if budget::continuation::verify_settled_history_on(db, root)? != *original {
            return Err("recovery_history_original_finance_changed".into());
        }
    }
    let bounded = json!({"schemaVersion":1,"source":"scan_original_settled_recovery_history",
        "readiness":"ready","scanId":actor.scan_id,"selectedSource":selected.bounded_fact(),
        "historicalRootIds":roots,"historicalRootsChecked":proofs.len(),"historicalProofs":proofs,
        "historyPhysicalHash":crate::agent_runtime::store::stable_hash(&json!(rows.iter().map(NativeCoordinatorFrozenRows::fact).collect::<Vec<_>>()).to_string()),
        "approval":false,"grant":false,"executionGrant":false,"targetRequestsGranted":0});
    if bounded.to_string().len() > 128 * 1024 {
        return Err("recovery_history_bounded_fact_too_large".into());
    }
    Ok(NativeRecoveryHistory {
        bounded,
        rows,
        _selected: selected,
        _historical_exit_guards: guards,
    })
}

fn native_recovery_history_verify_birth_on(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    root: &str,
) -> Result<(), String> {
    let signed: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM main.agent_root_mode_birth_seals WHERE root_run_id=?1
            OR json_extract(binding_json,'$.rootRunId')=?1)",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mode = crate::agent_runtime::web_mode::root::read(db, root)?;
    if signed && mode.is_none() {
        return Err("recovery_history_original_mode_missing".into());
    }
    if let Some(mode) = mode {
        let financial:bool=db.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.agent_financial_continuations WHERE new_root_run_id=?1
                OR json_extract(fact_json,'$.newRootRunId')=?1)",
            [root],|r|r.get(0)).map_err(|e|e.to_string())?;
        let birth = if financial {
            Some(verify_financial_continuation_birth_on(db, root)?)
        } else {
            None
        };
        let original = private_historical_web_mode_for_mode_in(
            db,
            scan,
            attempt,
            root,
            birth.as_ref(),
            mode.mode(),
        )?;
        crate::agent_runtime::web_mode::root::verify_birth(db, root, &original)?;
    }
    Ok(())
}
