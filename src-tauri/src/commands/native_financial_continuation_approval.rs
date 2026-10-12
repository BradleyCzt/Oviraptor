struct FinancialContinuationWriter<'a> {
    db: &'a rusqlite::Connection,
}
impl<'a> FinancialContinuationWriter<'a> {
    fn install(db: &'a rusqlite::Connection) -> Result<Self, String> {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        crate::collaboration_events::web_creation_schema::verify(db)?;
        db.authorizer(Some(|c: AuthContext<'_>| match c.action {
            AuthAction::Insert {
                table_name: "agent_collaboration_events",
            } if c.database_name == Some("main")
                && c.accessor == Some("agent_collaboration_run_insert") =>
            {
                Authorization::Allow
            }
            AuthAction::Insert { table_name }
                if c.database_name == Some("main")
                    && c.accessor.is_none()
                    && matches!(
                        table_name,
                        "agent_runs"
                            | "agent_financial_continuations"
                            | "agent_financial_root_budget_definitions"
                            | "agent_root_mode_definitions"
                            | "agent_root_mode_birth_seals"
                            | "agent_root_budget_attempts"
                            | "agent_budget_limits"
                            | "agent_budget_clock_origins"
                            | "agent_coordinator_leases"
                            | "agent_financial_startup_bindings"
                            | "native_web_mode_receipts"
                            | "sentinel_scan_attempts"
                            | "agent_evidence_locations"
                            | "app_settings"
                    ) =>
            {
                Authorization::Allow
            }
            AuthAction::Update {
                table_name: "sentinel_scans",
                column_name,
            } if c.database_name == Some("main")
                && c.accessor.is_none()
                && matches!(
                    column_name,
                    "status" | "attempt_count" | "current_checkpoint" | "task_path" | "updated_at"
                ) =>
            {
                Authorization::Allow
            }
            AuthAction::Update {
                table_name: "app_settings",
                column_name: "value",
            } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
            AuthAction::Read { .. }
            | AuthAction::Select
            | AuthAction::Function { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. }
            | AuthAction::Recursive => Authorization::Allow,
            _ => Authorization::Deny,
        }))
        .map_err(|e| e.to_string())?;
        Ok(Self { db })
    }
}
impl Drop for FinancialContinuationWriter<'_> {
    fn drop(&mut self) {
        use rusqlite::hooks::{AuthContext, Authorization};
        let _ = self
            .db
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}

fn financial_continuation_issuance_receipt_in(
    db: &rusqlite::Connection,
    fact: &JsonValue,
) -> Result<JsonValue, String> {
    financial_continuation_status_in(db, fact)
}

fn native_financial_continuation_approve_on(
    db_path: &Path,
    app_data: &Path,
    scan: &str,
    attempt: i64,
    root: &str,
    receipt: &str,
    proposal_hash: &str,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::{
        multi_agent::{budget, lease},
        store, web_mode,
    };
    let _control = claim_scan_control(db_path, scan)?;
    let db = db::open(db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    use rusqlite::OptionalExtension;
    let issued: Option<String> = tx
        .query_row(
            "SELECT new_root_run_id FROM agent_financial_continuations WHERE source_root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(issued) = issued {
        let fact = verify_financial_continuation_birth_on(&tx, &issued)?;
        if fact["scanId"] != scan
            || fact["sourceAttemptNumber"] != attempt
            || fact["sourceReceiptId"] != receipt
            || fact["proposalHash"] != proposal_hash
        {
            return Err("financial_continuation_approval_conflict".into());
        }
        return financial_continuation_issuance_receipt_in(&tx, &fact);
    }
    let _quiescence = claim_scan_quiescence_in(&tx, db_path, scan)?;
    let actor = tx.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at
        FROM agent_coordinator_leases WHERE root_run_id=?1",[root],|r|Ok(lease::CoordinatorLease {
            scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,
            fencing_token:r.get(5)?,lease_expires_at:r.get(6)?})).map_err(|e|e.to_string())?;
    let _sdk = budget::root::model::tick::require_idle_original(&tx, &actor)?;
    let _parent = crate::agent_runtime::execution_owner::probe_native_invocation(
        db_path,
        scan,
        attempt,
        "parent-supervisor",
        root,
    )?
    .ok_or("financial_continuation_original_parent_exit_missing")?;
    let approval = FinancialContinuationAuthorization::approve_in(
        &tx,
        scan,
        attempt,
        root,
        receipt,
        proposal_hash,
    )?;
    let mut files = FinancialContinuationFiles::allocate_in(&tx, app_data, &approval)?;
    // This new caller owns its scope before birth, never after a paid call.
    // The dedicated worker reclaims the same inode before its actual SDK.
    let _new_target = claim_native_invocation(
        db_path,
        scan,
        approval.plan.attempt_number,
        "target",
        &approval.plan.target_url,
    )?;
    let floor: i64 = tx
        .query_row(
            "SELECT coalesce(max(sequence),0) FROM agent_collaboration_events",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let floor_text = floor.to_string();
    let old_events = NativeCoordinatorFrozenRows::capture_optional(&tx,
        "SELECT rowid,* FROM agent_collaboration_events WHERE sequence<=CAST(?1 AS INTEGER) ORDER BY rowid",&[&floor_text])?;
    let old_birth_seals = NativeCoordinatorFrozenRows::capture_optional(&tx,
        "SELECT rowid,* FROM agent_root_mode_birth_seals WHERE root_run_id<>?1 ORDER BY rowid", &[approval.new_root_id()])?;
    let writer = FinancialContinuationWriter::install(&tx)?;
    let work = files.attempt.path.to_str().ok_or("web_mode_path_invalid")?;
    let task = files.attempt.path.join("task.json");
    let count = tx.execute("UPDATE sentinel_scans SET status='scanning',attempt_count=?1,current_checkpoint='financial_continuation_issued',
        task_path=?2,updated_at=datetime('now','localtime') WHERE id=?3 AND attempt_count=?4 AND status=?5",
        params![approval.plan.attempt_number,task.to_str(),scan,attempt,approval.proposal["previousStatus"].as_str()]).map_err(|e|e.to_string())?;
    if count != 1 {
        return Err("financial_continuation_current_scope_invalid".into());
    }
    let backend = json!({"scanId":scan,"attemptNumber":approval.plan.attempt_number,"requiresNode":false,"requiresBrowser":false,
        "targets":[{"url":approval.plan.target_url,"backend":"native","selectionReason":"explicit_financial_continuation"}]});
    tx.execute("INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode,status,stage,checkpoint,work_dir,
        llm_requests_start,input_tokens_start,output_tokens_start,cached_tokens_start,total_tokens_start,backend_plan_json)
        SELECT id,?2,'resume','scanning','agent_execution','financial_continuation_issued',?3,
        llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,?4 FROM sentinel_scans WHERE id=?1",
        params![scan,approval.plan.attempt_number,work,backend.to_string()]).map_err(|e|e.to_string())?;
    tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![format!("agent-current-attempt:{scan}"),approval.plan.attempt_number.to_string()]).map_err(|e|e.to_string())?;
    let mode = files.publish_mode_in(&tx, &approval)?;
    let declaration = web_mode::root::NewRootModeDeclaration::from_verified(
        &mode,
        &approval.plan.target_url,
        &approval.plan.hash(),
    )?;
    let fact = store::insert_financial_continuation_root_in(&tx, &approval, |fresh| {
        budget::continuation::migration::freeze_new(fresh, &approval)?;
        let fact = approval.publish_birth_in(&tx, &files)?;
        web_mode::root::freeze_new(fresh, &declaration)?;
        let expiry: String = tx
            .query_row(
                "SELECT min(datetime('now','localtime','+600 seconds'),?1)",
                [fact["originalDeadline"].as_str()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let fence = Uuid::new_v4().to_string();
        tx.execute("INSERT INTO agent_coordinator_leases(scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at,heartbeat_at)
            VALUES(?1,?2,?3,?4,1,?5,?6,datetime('now','localtime'))",params![scan,approval.plan.attempt_number,approval.plan.target_url,
                fresh.id(),fence,expiry]).map_err(|e|e.to_string())?;
        budget::root::RootOwner::initialize_new_native_root(fresh)?;
        tx.execute("INSERT INTO agent_evidence_locations(root_run_id,scan_id,attempt_number,target_url,attempt_dir,target_dir)
            VALUES(?1,?2,?3,?4,?5,?6)",params![fresh.id(),scan,approval.plan.attempt_number,approval.plan.target_url,
                work,files.target.to_str()]).map_err(|e|e.to_string())?;
        Ok(fact)
    })?;
    native_financial_continuation_require_authority_in(&tx, &fact)?;
    native_financial_continuation_require_admission_in(&tx, db_path, scan, approval.new_root_id())?;
    approval.source.verify_on(&tx)?;
    files.verify()?;
    crate::collaboration_events::web_creation_schema::verify(&tx)?;
    old_events.verify(&tx)?;
    old_birth_seals.verify(&tx)?;
    web_mode::root::verify_birth(&tx, approval.new_root_id(), &mode)?;
    let event: (String, i64, String, String, String, String) = tx
        .query_row(
            "SELECT scan_id,attempt_number,entity_type,entity_id,event_type,payload_json
        FROM agent_collaboration_events WHERE sequence>?1",
            [floor],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM agent_collaboration_events WHERE sequence>?1",
            [floor],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count != 1
        || event.0 != scan
        || event.1 != approval.plan.attempt_number
        || event.2 != "agent_run"
        || event.3 != approval.new_root_id()
        || event.4 != "agent_run"
        || serde_json::from_str::<JsonValue>(&event.5)
            .map_err(|_| "financial_continuation_creation_event_invalid")?
            != json!({"role":"coordinator","status":"prepared","terminalState":""})
    {
        return Err("financial_continuation_creation_event_invalid".into());
    }
    let result = financial_continuation_issuance_receipt_in(&tx, &fact)?;
    drop(writer);
    // Unknown commit outcomes preserve only this new attempt for inspection.
    files.attempt.preserve = true;
    tx.commit()
        .map_err(|e| format!("financial_continuation_commit_unknown:{e}"))?;
    Ok(result)
}

#[tauri::command]
pub fn approve_native_root_financial_continuation(
    state: State<'_, AppState>,
    scan_id: String,
    attempt_number: i64,
    root_run_id: String,
    receipt_id: String,
    proposal_hash: String,
) -> Result<JsonValue, String> {
    let issued = native_financial_continuation_approve_on(
        &state.db_path,
        &state.app_data_dir,
        &scan_id,
        attempt_number,
        &root_run_id,
        &receipt_id,
        &proposal_hash,
    )?;
    let root = issued["newRootRunId"]
        .as_str()
        .ok_or("financial_continuation_birth_invalid")?
        .to_string();
    let path = state.db_path.clone();
    if issued["workerStartAllowed"] == true {
        std::thread::spawn(move || {
            let _ = native_financial_continuation_work(&path, &scan_id, &root);
        });
    }
    Ok(issued)
}
