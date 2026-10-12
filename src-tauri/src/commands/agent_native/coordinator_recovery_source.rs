// Independent historical recovery proposal source. A new Root bill belongs
// here, never in an older protection frame's deduplication basis. This proof
// owns existing invocation guards but grants no approval or executable work.
struct NativeRecoverySource {
    bounded: JsonValue,
    rows: Vec<NativeCoordinatorFrozenRows>,
    _original_web: NativeClosedProtectionResponse,
    _original_root_sdk: crate::agent_runtime::execution_owner::NativeInvocationOwner,
}
impl NativeRecoverySource {
    fn bounded_fact(&self) -> JsonValue {
        self.bounded.clone()
    }
    fn verification_rows(&self) -> &[NativeCoordinatorFrozenRows] {
        &self.rows
    }
}

fn native_recovery_source_at_on(
    snapshot: &rusqlite::Connection,
    db_path: &Path,
    original_actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    protection_receipt: &str,
) -> Result<NativeRecoverySource, String> {
    use crate::agent_runtime::multi_agent::budget::{self, root::model::tick};
    if snapshot.is_autocommit() {
        return Err("recovery_source_snapshot_required".into());
    }
    // Original audit helpers read fixed application tables. A temporary table
    // must not substitute a caller-owned financial or authorization projection.
    let shadow: bool = snapshot
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name GLOB 'agent_*'
         OR name GLOB 'sentinel_*' OR name IN ('native_web_mode_receipts',
         'native_web_dispatch_bindings','browser_auth_sessions'))",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "recovery_source_schema_unavailable")?;
    if shadow {
        return Err("recovery_source_schema_shadowed".into());
    }
    // Both probes are no-create. Keep them alive through the caller's proposal
    // capture; a missing original inode is never repaired by reading history.
    let web = native_closed_protection_response_at_on(
        snapshot,
        db_path,
        original_actor,
        protection_receipt,
    )?;
    let original_protection = web.bounded_fact();
    let root_sdk = tick::require_idle_original(snapshot, original_actor)?
        .ok_or("recovery_source_original_root_sdk_exit_missing")?;
    budget::root::RootOwner::load_original(snapshot, &original_actor.root_run_id)?
        .require_original_coordinator(snapshot, original_actor)?;
    native_recovery_require_quiescent_children(snapshot, original_actor)?;
    // This validates original owners, all original provider families, terminal
    // physical proofs and exact original paid rows, not merely their sums.
    // Preserve its fail-closed boundaries: unsupported authorization receipts,
    // exhausted elapsed facts, and unpaid semantic recovery without an original
    // explicit confirmation cannot be advertised as ready by this adapter.
    let history =
        budget::continuation::verify_settled_history_on(snapshot, &original_actor.root_run_id)?;
    let calls = native_recovery_original_root_calls(snapshot, &original_actor.root_run_id)?;
    let mut dimensions = serde_json::Map::new();
    for dimension in budget::DIMENSIONS {
        let balance = budget::balance(snapshot, &original_actor.root_run_id, None, dimension)?;
        if balance.reserved != 0 || balance.indeterminate != 0 {
            return Err("recovery_source_original_cost_unsettled".into());
        }
        let limit: Option<i64> = snapshot
            .query_row(
                "SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension=?2",
                params![original_actor.root_run_id, dimension],
                |r| r.get(0),
            )
            .map_err(|_| "recovery_source_original_limit_missing")?;
        let remaining = limit
            .map(|cap| {
                cap.checked_sub(balance.consumed)
                    .filter(|left| *left >= 0)
                    .ok_or("recovery_source_original_limit_exceeded")
            })
            .transpose()?;
        dimensions.insert(
            dimension.into(),
            json!({"hardLimit":limit,"consumed":balance.consumed,
            "reserved":0,"indeterminate":0,"remaining":remaining}),
        );
    }
    let mut rows = web.verification_rows().to_vec();
    // Freeze this independent financial snapshot, including exact empty states.
    // Root/child raw requests stay private; only physical hashes are returned.
    for sql in [
        "SELECT rowid,* FROM agent_runs WHERE id=?1 OR root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_assignments WHERE coordinator_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_assignment_attempts WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_budget_limits WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_budget_ledger WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_budget_attempts WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_budget_definitions WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_mode_definitions WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_mode_birth_seals WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_budget_clock_origins WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_elapsed_facts WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_tick_receipts WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_tick_terminal_proofs WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_tick_timeline_receipts WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_cost_reconciliation_receipts WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_specialist_calls WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_source_model_rounds WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_web_model_journal WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_model_cost_facts WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_capability_leases WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM agent_messages WHERE root_run_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM tool_invocations WHERE run_id IN(SELECT id FROM agent_runs WHERE id=?1 OR root_run_id=?1) ORDER BY rowid",
        "SELECT rowid,* FROM agent_http_request_claims WHERE run_id IN(SELECT id FROM agent_runs WHERE id=?1 OR root_run_id=?1) ORDER BY rowid",
        "SELECT rowid,* FROM agent_source_tool_receipts WHERE child_run_id IN(SELECT id FROM agent_runs WHERE root_run_id=?1 AND id<>?1) ORDER BY rowid",
        "SELECT rowid,* FROM agent_external_surface_captures WHERE child_run_id IN(SELECT id FROM agent_runs WHERE root_run_id=?1 AND id<>?1) ORDER BY rowid",
        "SELECT rowid,* FROM agent_authorization_probe_claims WHERE child_run_id IN(SELECT id FROM agent_runs WHERE root_run_id=?1 AND id<>?1) ORDER BY rowid",
        "SELECT rowid,* FROM agent_evidence_nodes WHERE root_run_id=?1 ORDER BY rowid",
    ] {
        rows.push(NativeCoordinatorFrozenRows::capture_optional(snapshot,sql,&[&original_actor.root_run_id])?);
    }
    for row in &rows {
        row.verify(snapshot)?;
    }
    if budget::continuation::verify_settled_history_on(snapshot, &original_actor.root_run_id)?
        != history
        || native_recovery_original_root_calls(snapshot, &original_actor.root_run_id)? != calls
        || native_audit_closed_protection_response_at_on(
            snapshot,
            db_path,
            original_actor,
            protection_receipt,
        )?
        .bounded
            != original_protection
    {
        return Err("recovery_source_original_fact_changed".into());
    }
    native_recovery_require_quiescent_children(snapshot, original_actor)?;
    let physical = rows
        .iter()
        .map(NativeCoordinatorFrozenRows::fact)
        .collect::<Vec<_>>();
    let bounded = json!({"schemaVersion":1,"source":"original_settled_recovery_source",
        "readiness":"ready","scanId":original_actor.scan_id,"attemptNumber":original_actor.attempt_number,
        "rootRunId":original_actor.root_run_id,"targetKey":original_actor.target_key,
        "coordinatorEpoch":original_actor.lease_epoch,"coordinatorFence":original_actor.fencing_token,
        "protectionReceiptId":protection_receipt,"originalProtection":original_protection,
        "financialHistory":history,"rootCalls":calls,"dimensions":dimensions,
        "physicalFactsHash":crate::agent_runtime::store::stable_hash(&json!(physical).to_string()),
        "grant":false,"approval":false,"executionGrant":false,"targetRequestsGranted":0});
    if bounded.to_string().len() > 64 * 1024 {
        return Err("recovery_source_bounded_fact_too_large".into());
    }
    Ok(NativeRecoverySource {
        bounded,
        rows,
        _original_web: web,
        _original_root_sdk: root_sdk,
    })
}

fn native_recovery_original_root_calls(
    snapshot: &rusqlite::Connection,
    root: &str,
) -> Result<Vec<JsonValue>, String> {
    use crate::agent_runtime::multi_agent::budget::root::model::tick::Tick;
    let mut q = snapshot.prepare(
        "SELECT DISTINCT call_id FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY call_id",
    ).map_err(|e|e.to_string())?;
    let calls = q
        .query_map([root], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if calls.is_empty() || calls.len() > 256 {
        return Err("recovery_source_original_calls_required".into());
    }
    let mut result = Vec::new();
    for call in calls {
        let audit = Tick::audit_cost_for_continuation(snapshot, root, &call)?;
        if audit["rootRunId"] != root
            || audit["callId"] != call
            || !matches!(audit["phase"].as_str(), Some("received" | "unsent"))
        {
            return Err("recovery_source_original_call_unsettled".into());
        }
        result.push(json!({"callId":call,"phase":audit["phase"],
            "originalSemanticAvailable":audit["originalSemanticAvailable"],
            "confirmationReceiptId":audit["confirmationReceipt"]["receiptId"],
            "auditHash":crate::agent_runtime::store::stable_hash(&audit.to_string()),
            "semanticReplayAllowed":false}));
    }
    Ok(result)
}

fn native_recovery_require_quiescent_children(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    let root = actor.root_run_id.as_str();
    // A known invoice is not proof that an unfinished worker cannot send again.
    // The selected original Web has OS guards; every other original child must
    // already have a terminal worker/run and released lane/capability grant.
    let active:bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignment_attempts WHERE root_run_id=?1
           AND (state IN ('leased','running','paused') OR finished_at=''))
         OR EXISTS(SELECT 1 FROM agent_runs WHERE root_run_id=?1 AND id<>?1
           AND (status<>'terminal' OR finished_at=''))
         OR EXISTS(SELECT 1 FROM agent_capability_leases WHERE root_run_id=?1 AND revoked_at='')
         OR EXISTS(SELECT 1 FROM agent_lane_leases l JOIN agent_assignments a ON a.id=l.assignment_id
           WHERE a.coordinator_run_id=?1)
         OR EXISTS(SELECT 1 FROM tool_invocations i JOIN agent_runs r ON r.id=i.run_id
           WHERE (r.id=?1 OR r.root_run_id=?1) AND i.status='running')",
        [root],|r|r.get(0),
    ).map_err(|_|"recovery_source_original_work_unavailable")?;
    if active {
        return Err("recovery_source_original_work_not_closed".into());
    }
    let mut q = db
        .prepare(
            "SELECT x.id,x.assignment_id,x.child_run_id FROM agent_assignment_attempts x
         WHERE x.root_run_id=?1 ORDER BY x.id",
        )
        .map_err(|e| e.to_string())?;
    let workers = q
        .query_map([root], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for (worker, assignment, child) in workers {
        let owner = crate::agent_runtime::multi_agent::budget::receipts::original_owner(
            db,
            &child,
            actor,
            &assignment,
        )?;
        owner.verify(db)?;
        if owner.attempt_id() != worker {
            return Err("recovery_source_original_worker_scope_conflict".into());
        }
        for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
            let balance = crate::agent_runtime::multi_agent::budget::balance_for_attempt(
                db,
                root,
                &assignment,
                &worker,
                dimension,
            )?;
            if balance.reserved != 0 || balance.indeterminate != 0 {
                return Err("recovery_source_original_child_cost_unsettled".into());
            }
        }
    }
    Ok(())
}
