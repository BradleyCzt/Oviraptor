// Staging a signed independent task scope is not an execution grant. Identities
// stay in their approved draft scope until the dedicated birth consumer binds
// them; generic confirm/resume/retry are explicitly closed for these drafts.
fn native_protection_recovery_draft_schema_on(db: &rusqlite::Connection) -> Result<(), String> {
    let expected = rusqlite::Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected
        .execute_batch(include_str!("native_protection_recovery_schema.sql"))
        .map_err(|e| e.to_string())?;
    let read =
        |db: &rusqlite::Connection| -> Result<Vec<(String, String, String, String)>, String> {
            let mut q = db.prepare("SELECT type,name,tbl_name,coalesce(sql,'') FROM main.sqlite_master WHERE tbl_name='native_protection_recovery_drafts' ORDER BY type,name").map_err(|e|e.to_string())?;
            let rows = q
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            Ok(rows)
        };
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name='native_protection_recovery_drafts' OR tbl_name='native_protection_recovery_drafts')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if shadow || read(db)? != read(&expected)? {
        return Err("protection_recovery_draft_schema_changed".into());
    }
    Ok(())
}

fn native_protection_recovery_draft_immutable_scope_on(
    db: &rusqlite::Connection,
    value: &JsonValue,
) -> Result<(), String> {
    let scan = value["scanId"]
        .as_str()
        .ok_or("protection_recovery_draft_corrupt")?;
    let scope: NativeProtectionRecoveryScope =
        serde_json::from_value(value["approval"]["proposal"]["newScope"].clone())
            .map_err(|_| "protection_recovery_draft_corrupt")?;
    let row: (i64, String) = db
        .query_row(
            "SELECT project_id,task_name FROM main.sentinel_scans WHERE id=?1 AND scan_type='web'",
            [scan],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "protection_recovery_draft_missing")?;
    let policy: String = db
        .query_row(
            "SELECT policy_json FROM main.sentinel_scan_contexts WHERE scan_id=?1",
            [scan],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let targets = {
        let mut q = db
            .prepare("SELECT url FROM main.sentinel_targets WHERE scan_id=?1 ORDER BY url")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([scan], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    let wrong:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.sentinel_targets WHERE scan_id=?1 AND project_id<>?2)",params![scan,scope.project_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let mode = crate::agent_runtime::web_mode::draft::read(db, scan)?;
    let mut expected_policy = build_web_investigation_policy(
        Some(&scope.scan_mode),
        Some(scope.max_budget_usd),
        scope.auth_session_ids.clone(),
        &scope.skill_ids,
        &scope.instruction,
        "agent-dialog",
        Some(&scope.closure),
    )?;
    expected_policy["orchestration"] = json!({"version":1,"mode":scope.orchestration_mode});
    let expected_policy_text =
        serde_json::to_string(&expected_policy).map_err(|e| e.to_string())?;
    if mode.0 != value["draftId"]
        || mode.1.as_str() != scope.orchestration_mode
        || expected_policy_text != policy
    {
        return Err("protection_recovery_draft_scope_changed".into());
    }
    if row != (scope.project_id, scope.task_name)
        || targets != scope.urls
        || value["policyJson"] != policy
        || wrong
        || crate::agent_runtime::web_mode::draft::read(db, scan)?.0 != value["draftId"]
    {
        return Err("protection_recovery_draft_scope_changed".into());
    }
    Ok(())
}

// History reads keep the original signed creation fact after lifecycle changes.
// Every first birth must separately require this pristine, unconsumed state.
fn native_protection_recovery_draft_pristine_scope_on(
    db: &rusqlite::Connection,
    value: &JsonValue,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    native_protection_recovery_draft_immutable_scope_on(db, value)?;
    native_protection_recovery_cancellation_schema_on(db)?;
    let scan = value["scanId"]
        .as_str()
        .ok_or("protection_recovery_draft_corrupt")?;
    let pristine: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.sentinel_scans s JOIN main.projects p ON p.id=s.project_id
         WHERE s.id=?1 AND s.status='draft' AND s.attempt_count=0 AND s.task_path='' AND p.status='active')
         AND NOT EXISTS(SELECT 1 FROM main.sentinel_targets WHERE scan_id=?1 AND status<>'queued')
         AND NOT EXISTS(SELECT 1 FROM main.sentinel_scan_attempts WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM main.agent_runs WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM main.native_branch_dispatches WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM main.sentinel_deleted_scans WHERE scan_id=?1)
         AND NOT EXISTS(SELECT 1 FROM main.native_protection_recovery_cancellations WHERE scan_id=?1 OR approval_id=?2 OR request_id=?3)",
        params![scan, value["approval"]["approvalId"].as_str(), value["approval"]["requestId"].as_str()], |r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if !pristine {
        return Err("protection_recovery_draft_not_pristine".into());
    }
    Ok(())
}

type NativeProtectionRecoveryDraftStoredRow =
    (String, String, String, String, String, String, Vec<u8>);

fn native_protection_recovery_draft_saved_on(
    db: &rusqlite::Connection,
    request: &str,
) -> Result<Option<JsonValue>, String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    native_protection_recovery_draft_schema_on(db)?;
    let row:Option<NativeProtectionRecoveryDraftStoredRow>=db.query_row(
        "SELECT approval_id,scan_id,draft_id,source_scan_id,proposal_hash,draft_json,signature FROM main.native_protection_recovery_drafts WHERE request_id=?1",
        [request],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, scan, draft, source, hash, text, signature)) = row else {
        return Ok(None);
    };
    let value: JsonValue =
        serde_json::from_str(&text).map_err(|_| "protection_recovery_draft_corrupt")?;
    let canonical = serde_json::to_string(&value).map_err(|e| e.to_string())?;
    if canonical != text
        || value["schemaVersion"] != 1
        || value["purpose"] != "explicit_protection_recovery_new_draft"
        || value["scanId"] != scan
        || value["draftId"] != draft
        || value["approval"]["approvalId"] != id
        || value["approval"]["requestId"] != request
        || value["approval"]["proposalHash"] != hash
        || value["approval"]["proposal"]["selection"]["scanId"] != source
        || scan == source
        || value["status"] != "draft_pending_dedicated_fresh_birth"
        || value["identitiesBound"] != false
        || value["executionAllowed"] != false
        || value["targetRequestsGranted"] != 0
        || value["oldScopeRenewed"] != false
        || Uuid::parse_str(&scan).is_err()
        || Uuid::parse_str(&draft).is_err()
        || native_protection_recovery_saved_on(db, request)?.as_ref() != Some(&value["approval"])
    {
        return Err("protection_recovery_draft_binding_changed".into());
    }
    recovery_approval_signature::verify_saved_draft_on(db, &value, &signature)?;
    native_protection_recovery_draft_immutable_scope_on(db, &value)?;
    native_protection_recovery_scope_budget_verify_on(db, &value)?;
    Ok(Some(value))
}

pub(crate) fn native_protection_recovery_draft_action_on(
    db: &rusqlite::Connection,
    scan: &str,
) -> Result<(), String> {
    let request: String = db
        .query_row(
            "SELECT request_id FROM main.native_protection_recovery_drafts WHERE scan_id=?1",
            [scan],
            |r| r.get(0),
        )
        .map_err(|_| "protection_recovery_draft_missing")?;
    native_protection_recovery_draft_saved_on(db, &request)?
        .ok_or("protection_recovery_draft_missing")?;
    Ok(())
}

struct NativeProtectionRecoveryDraftWriter<'a>(&'a rusqlite::Connection);
impl<'a> NativeProtectionRecoveryDraftWriter<'a> {
    fn install(db: &'a rusqlite::Connection) -> Result<Self, String> {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        native_protection_recovery_draft_schema_on(db)?;
        native_protection_recovery_scope_budget_schema_on(db)?;
        db.authorizer(Some(|c: AuthContext<'_>| match c.action {
            AuthAction::Insert { table_name }
                if c.database_name == Some("main")
                    && c.accessor.is_none()
                    && matches!(
                        table_name,
                        "sentinel_scans"
                            | "sentinel_targets"
                            | "sentinel_scan_contexts"
                            | "native_protection_recovery_drafts"
                            | "native_protection_recovery_scope_budgets"
                    ) =>
            {
                Authorization::Allow
            }
            AuthAction::Pragma {
                pragma_name: "database_list",
                ..
            } => Authorization::Allow,
            AuthAction::Read { .. }
            | AuthAction::Select
            | AuthAction::Function { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. }
            | AuthAction::Recursive => Authorization::Allow,
            _ => Authorization::Deny,
        }))
        .map_err(|e| e.to_string())?;
        Ok(Self(db))
    }
}
impl Drop for NativeProtectionRecoveryDraftWriter<'_> {
    fn drop(&mut self) {
        use rusqlite::hooks::{AuthContext, Authorization};
        let _ = self
            .0
            .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    }
}

fn native_protection_recovery_create_draft_on(
    db_path: &Path,
    request: &str,
) -> Result<JsonValue, String> {
    if Uuid::parse_str(request).is_err() {
        return Err("protection_recovery_request_invalid".into());
    }
    // Discover only the original source needed for its lifecycle mutex; verify
    // the signed approval again in the subsequent writer transaction.
    let read = native_protection_recovery_open(db_path, false)?;
    let snapshot = read.unchecked_transaction().map_err(|e| e.to_string())?;
    let approved = native_protection_recovery_saved_on(&snapshot, request)?
        .ok_or("protection_recovery_approval_missing")?;
    let selected: NativeProtectionRecoverySelection =
        serde_json::from_value(approved["proposal"]["selection"].clone())
            .map_err(|_| "protection_recovery_approval_corrupt")?;
    snapshot.commit().map_err(|e| e.to_string())?;
    drop(read);
    let _source_control = claim_scan_control(db_path, &selected.scan_id)?;
    let mut db = native_protection_recovery_open(db_path, true)?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if native_protection_recovery_saved_on(&tx, request)?.as_ref() != Some(&approved) {
        return Err("protection_recovery_draft_approval_changed".into());
    }
    native_protection_recovery_cancellation_schema_on(&tx)?;
    let revoked: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.native_protection_recovery_cancellations WHERE approval_id=?1 OR request_id=?2)",
        params![approved["approvalId"].as_str(), request], |r|r.get(0),
    ).map_err(|e|e.to_string())?;
    if revoked {
        return Err("protection_recovery_draft_cancelled".into());
    }
    if let Some(saved) = native_protection_recovery_draft_saved_on(&tx, request)? {
        if native_protection_recovery_cancellation_saved_on(
            &tx,
            saved["scanId"]
                .as_str()
                .ok_or("protection_recovery_draft_corrupt")?,
        )?
        .is_some()
        {
            return Err("protection_recovery_draft_cancelled".into());
        }
        crate::agent_runtime::web_mode::action::read_hash(
            &tx,
            saved["scanId"]
                .as_str()
                .ok_or("protection_recovery_draft_corrupt")?,
        )?
        .ok_or("web_mode_independent_action_missing")?;
        tx.commit().map_err(|e| e.to_string())?;
        return Ok(saved);
    }
    let scope: NativeProtectionRecoveryScope =
        serde_json::from_value(approved["proposal"]["newScope"].clone())
            .map_err(|_| "protection_recovery_approval_corrupt")?;
    let proof = native_protection_recovery_proposal_in(&tx, db_path, &selected, &scope)?;
    if proof.fact != approved["proposal"] {
        return Err("protection_recovery_proposal_changed".into());
    }
    let window = crate::agent_runtime::web_mode::draft::DraftCreationWindow::begin(&tx)?;
    let scan = Uuid::new_v4().to_string();
    let project_name: String = tx
        .query_row(
            "SELECT name FROM main.projects WHERE id=?1 AND status='active'",
            [scope.project_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let policy = build_web_investigation_policy(
        Some(&scope.scan_mode),
        Some(scope.max_budget_usd),
        scope.auth_session_ids.clone(),
        &scope.skill_ids,
        &scope.instruction,
        "agent-dialog",
        Some(&scope.closure),
    )?;
    let writer = NativeProtectionRecoveryDraftWriter::install(&tx)?;
    tx.execute("INSERT INTO main.sentinel_scans(id,project_id,project_name,status,current_checkpoint,task_path,scan_type,task_name) VALUES(?1,?2,?3,'draft','待专属恢复出生：批准范围已冻结','','web',?4)",params![scan,scope.project_id,project_name,scope.task_name]).map_err(|e|e.to_string())?;
    tx.execute("INSERT INTO main.sentinel_scan_contexts(scan_id,environment,policy_json) VALUES(?1,'internal',?2)",params![scan,policy.to_string()]).map_err(|e|e.to_string())?;
    for url in &scope.urls {
        tx.execute("INSERT INTO main.sentinel_targets(project_id,scan_id,company,url,status) VALUES(?1,?2,?3,?4,'queued')",params![scope.project_id,scan,project_name,url]).map_err(|e|e.to_string())?;
    }
    drop(writer);
    let mode = match scope.orchestration_mode.as_str() {
        "multi" => crate::agent_runtime::web_mode::WebMode::Multi,
        "single" => crate::agent_runtime::web_mode::WebMode::Single,
        _ => return Err("protection_recovery_mode_invalid".into()),
    };
    let fresh = crate::agent_runtime::web_mode::draft::register_new(window, &scan, mode)?;
    let staged = recovery_approval_signature::FreshProtectionRecoveryDraft::create_in(
        &fresh, db_path, &proof, &approved,
    )?;
    let (value, signature) = staged.sealed()?;
    let writer = NativeProtectionRecoveryDraftWriter::install(&tx)?;
    native_protection_recovery_scope_budget_insert_on(&tx, &value)?;
    tx.execute("INSERT INTO main.native_protection_recovery_drafts(approval_id,request_id,scan_id,draft_id,source_scan_id,proposal_hash,draft_json,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![approved["approvalId"].as_str(),request,scan,fresh.draft_id(),selected.scan_id,approved["proposalHash"].as_str(),value.to_string(),signature]).map_err(|e|e.to_string())?;
    if native_protection_recovery_draft_saved_on(&tx, request)?.as_ref() != Some(&value) {
        return Err("protection_recovery_draft_not_saved".into());
    }
    drop(writer);
    crate::agent_runtime::web_mode::action::freeze_new(
        fresh,
        &crate::agent_runtime::web_mode::action::NewWebAction {
            schema_version: 1,
            kind: "protection_recovery".into(),
            request_id: request.into(),
            source_scan_id: selected.scan_id.clone(),
            source_hash: approved["proposalHash"]
                .as_str()
                .ok_or("protection_recovery_approval_corrupt")?
                .into(),
            request_hash: approved["approvalId"]
                .as_str()
                .ok_or("protection_recovery_approval_corrupt")?
                .into(),
            targets: scope.urls.clone(),
        },
    )?;
    proof.verify_on(&tx, db_path, &selected, &scope)?;
    if native_protection_recovery_draft_saved_on(&tx, request)?.as_ref() != Some(&value) {
        return Err("protection_recovery_draft_not_saved".into());
    }
    native_protection_recovery_draft_pristine_scope_on(&tx, &value)?;
    tx.commit().map_err(|e| e.to_string())?;
    drop(proof);
    Ok(value)
}

#[tauri::command]
pub fn create_native_protection_recovery_draft(
    state: State<AppState>,
    request_id: String,
) -> Result<JsonValue, String> {
    native_protection_recovery_create_draft_on(&state.db_path, &request_id)
}
#[tauri::command]
pub fn get_native_protection_recovery_draft(
    state: State<AppState>,
    request_id: String,
) -> Result<Option<JsonValue>, String> {
    let db = native_protection_recovery_open(&state.db_path, false)?;
    let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
    let saved = native_protection_recovery_draft_saved_on(&snapshot, &request_id)?;
    if let Some(value) = &saved {
        crate::agent_runtime::web_mode::action::read_hash(
            &snapshot,
            value["scanId"]
                .as_str()
                .ok_or("protection_recovery_draft_corrupt")?,
        )?
        .ok_or("web_mode_independent_action_missing")?;
    }
    snapshot.commit().map_err(|e| e.to_string())?;
    Ok(saved)
}
