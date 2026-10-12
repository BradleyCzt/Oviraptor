// A dedicated human choice, independent from chat directives and old fees.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeProtectionRecoverySelection {
    pub scan_id: String,
    pub attempt_number: i64,
    pub root_run_id: String,
    pub protection_receipt_id: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NativeProtectionRecoveryScope {
    pub project_id: i64,
    pub task_name: String,
    pub urls: Vec<String>,
    pub allowed_methods: Vec<String>,
    pub auth_session_ids: Vec<String>,
    pub auth_session_scope_id: String,
    pub model: String,
    pub proxy_binding_hash: Option<String>,
    pub orchestration_mode: String,
    pub scan_mode: String,
    pub closure: String,
    pub skill_ids: Vec<i64>,
    pub instruction: String,
    pub max_budget_usd: f64,
    pub limits: [Option<i64>; 10],
    pub deadline_utc: String,
}

impl NativeProtectionRecoveryScope {
    fn validate_on(&self, db: &rusqlite::Connection) -> Result<JsonValue, String> {
        if self.project_id <= 0
            || self.task_name.trim() != self.task_name
            || self.task_name.is_empty()
            || self.task_name.chars().count() > 120
            || self.task_name.chars().any(char::is_control)
            || self.instruction.len() > 32 * 1024
            || self.urls.is_empty()
            || self.urls.len() > 200
            || self.urls.windows(2).any(|v| v[0] >= v[1])
            || self.allowed_methods.is_empty()
            || self.allowed_methods.len() > 7
            || self.allowed_methods.windows(2).any(|v| v[0] >= v[1])
            || self.allowed_methods.iter().any(|v| {
                !matches!(
                    v.as_str(),
                    "GET" | "HEAD" | "OPTIONS" | "POST" | "PUT" | "PATCH" | "DELETE"
                )
            })
            || self.auth_session_ids.len() > 5
            || self
                .auth_session_ids
                .iter()
                .any(|v| uuid::Uuid::parse_str(v).is_err())
            || self.auth_session_ids.windows(2).any(|v| v[0] >= v[1])
            || (!self.auth_session_scope_id.is_empty()
                && uuid::Uuid::parse_str(&self.auth_session_scope_id).is_err())
            || !matches!(self.orchestration_mode.as_str(), "single" | "multi")
            || !matches!(self.scan_mode.as_str(), "quick" | "standard" | "deep")
            || !matches!(self.closure.as_str(), "breadth" | "proof")
            || self.skill_ids.len() > 64
            || self.skill_ids.iter().any(|v| *v <= 0)
            || self.skill_ids.windows(2).any(|v| v[0] >= v[1])
            || !self.max_budget_usd.is_finite()
            || self.max_budget_usd <= 0.0
            || self.max_budget_usd > 10_000.0
            || self.limits.iter().any(|v| v.is_some_and(|n| n < 0))
            || self.limits[3..].iter().any(Option::is_none)
            || self.limits[3].is_none_or(|n| n == 0)
            || self.limits[4].is_none_or(|n| n == 0)
            || self.limits[8].is_none_or(|n| n > 128)
            || self.limits[9].is_none_or(|n| n == 0)
        {
            return Err("protection_recovery_new_scope_invalid".into());
        }
        // The three denied dimensions still have no real append consumer.
        // An approval cannot pretend to enable their unavailable writers.
        if self.limits[5..8] != [Some(0); 3] {
            return Err("protection_recovery_dimension_writer_unavailable".into());
        }
        let deadline = chrono::DateTime::parse_from_rfc3339(&self.deadline_utc)
            .map_err(|_| "protection_recovery_deadline_invalid")?;
        if !self.deadline_utc.ends_with('Z') || deadline <= chrono::Utc::now() {
            return Err("protection_recovery_deadline_expired".into());
        }
        crate::auth_session::validate_draft_sessions_for_task(
            db,
            &self.auth_session_ids,
            self.project_id,
            &self.auth_session_scope_id,
        )?;
        let documents = crate::auth_session::distinct_session_documents_for_scan(
            db,
            &self.auth_session_ids,
            self.project_id,
        )?;
        for literal in &self.urls {
            let url =
                reqwest::Url::parse(literal).map_err(|_| "protection_recovery_url_invalid")?;
            if literal.trim() != literal
                || literal.len() > 4096
                || literal.chars().any(char::is_control)
                || !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err("protection_recovery_url_invalid".into());
            }
            let fused: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM main.sentinel_fuse_zone WHERE project_id=?1
                 AND normalized_url=lower(rtrim(trim(?2),'/')))",
                    params![self.project_id, literal],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if fused {
                return Err("protection_recovery_target_fused".into());
            }
            for document in &documents {
                if !document["scopeHosts"].as_array().is_some_and(|hosts| {
                    hosts.iter().any(|host| {
                        host.as_str()
                            .is_some_and(|h| h.eq_ignore_ascii_case(url.host_str().unwrap_or("")))
                    })
                }) {
                    return Err("protection_recovery_identity_target_conflict".into());
                }
            }
        }
        for id in &self.skill_ids {
            let enabled: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM main.agent_skills WHERE id=?1 AND enabled=1)",
                    [id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !enabled {
                return Err("protection_recovery_skill_unavailable".into());
            }
        }
        let settings = sentinel_settings(db);
        // Store a reference, never a credential-bearing proxy URL in the task
        // choice or public approval. Only the existing approved pool resolves it.
        let proxy = self
            .proxy_binding_hash
            .as_ref()
            .map(|binding| {
                if binding.len() != 64 || !binding.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err("protection_recovery_proxy_binding_invalid".to_string());
                }
                approved_agent_proxies(&settings)
                    .into_iter()
                    .find(|(_, url)| crate::agent_runtime::store::stable_hash(url) == *binding)
                    .map(|(_, url)| url)
                    .ok_or_else(|| "protection_recovery_proxy_not_approved".to_string())
            })
            .transpose()?;
        let environment = model_runtime_env(&settings)?;
        let profile = agent_model_profile(&environment, proxy.as_deref())?;
        if profile.model != self.model {
            return Err("protection_recovery_selected_model_changed".into());
        }
        use crate::agent_runtime::store::stable_hash;
        Ok(
            json!({"model":profile.model,"endpointBinding":stable_hash(&profile.endpoint),
            "deployment":if profile.local {"local"}else {"cloud"},
            "credentialBinding":stable_hash(profile.bearer().unwrap_or("")),
            "proxyBinding":stable_hash(profile.proxy.as_deref().unwrap_or("")),
            "maxContextTokens":profile.max_context_tokens,"maxOutputTokens":profile.max_output_tokens,
            "temperature":crate::agent_runtime::model::gateway::ModelRequest::default().temperature}),
        )
    }
}

struct NativeProtectionRecoveryProposal {
    fact: JsonValue,
    source: NativeRecoveryHistory,
    rows: Vec<NativeCoordinatorFrozenRows>,
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    _original_target_exit_guards: Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
}
impl NativeProtectionRecoveryProposal {
    fn verify_on(
        &self,
        db: &rusqlite::Connection,
        db_path: &Path,
        selected: &NativeProtectionRecoverySelection,
        scope: &NativeProtectionRecoveryScope,
    ) -> Result<(), String> {
        for row in &self.rows {
            row.verify(db)?;
        }
        self.source
            .verify_on(db, db_path, &self.actor, &selected.protection_receipt_id)?;
        if scope.validate_on(db)? != self.fact["modelBinding"] {
            return Err("protection_recovery_selected_model_changed".into());
        }
        Ok(())
    }
}

fn native_protection_recovery_proposal_in(
    db: &rusqlite::Connection,
    db_path: &Path,
    selected: &NativeProtectionRecoverySelection,
    scope: &NativeProtectionRecoveryScope,
) -> Result<NativeProtectionRecoveryProposal, String> {
    if db.is_autocommit() {
        return Err("protection_recovery_snapshot_required".into());
    }
    let model = scope.validate_on(db)?;
    let project: i64 = db.query_row(
        "SELECT s.project_id FROM main.sentinel_scans s JOIN main.projects p ON p.id=s.project_id
         WHERE s.id=?1 AND s.scan_type='web' AND s.status IN ('paused','partial','failed','completed','cancelled')
          AND p.status='active' AND NOT EXISTS(SELECT 1 FROM main.sentinel_deleted_scans WHERE scan_id=s.id)",
        [&selected.scan_id], |r| r.get(0)
    ).map_err(|_| "protection_recovery_source_task_not_closed")?;
    if project != scope.project_id {
        return Err("protection_recovery_project_conflict".into());
    }
    let actor = native_recovery_original_actor_on(
        db,
        &selected.scan_id,
        selected.attempt_number,
        &selected.root_run_id,
    )?;
    let source = native_recovery_history_on(db, db_path, &actor, &selected.protection_receipt_id)?;
    let original_target_exit_guards =
        source.require_original_target_exits_on(db, db_path, &selected.scan_id)?;
    let mut rows = Vec::new();
    for sql in [
        "SELECT s.rowid,s.*,p.rowid,p.* FROM main.sentinel_scans s JOIN main.projects p ON p.id=s.project_id WHERE s.id=?1",
        "SELECT rowid,* FROM main.sentinel_deleted_scans WHERE scan_id=?1 ORDER BY rowid",
        "SELECT rowid,* FROM main.sentinel_targets WHERE scan_id=?1 ORDER BY rowid",
    ] { rows.push(NativeCoordinatorFrozenRows::capture_optional(db,sql,&[&selected.scan_id])?); }
    rows.push(NativeCoordinatorFrozenRows::capture(
        db,
        "SELECT rowid,* FROM main.config_profiles ORDER BY is_default DESC,id LIMIT 1",
        &[],
    )?);
    let project_text = project.to_string();
    rows.push(NativeCoordinatorFrozenRows::capture_optional(db,
        "SELECT rowid,* FROM main.sentinel_fuse_zone WHERE project_id=CAST(?1 AS INTEGER) ORDER BY rowid", &[&project_text])?);
    for id in &scope.auth_session_ids {
        rows.push(NativeCoordinatorFrozenRows::capture(
            db,
            "SELECT rowid,* FROM main.browser_auth_sessions WHERE id=?1",
            &[id],
        )?);
    }
    for id in &scope.skill_ids {
        rows.push(NativeCoordinatorFrozenRows::capture(
            db,
            "SELECT rowid,* FROM main.agent_skills WHERE id=CAST(?1 AS INTEGER)",
            &[&id.to_string()],
        )?);
    }
    let mut fact = json!({"schemaVersion":1,"purpose":"explicit_new_target_work_after_original_protection_stop",
        "selection":selected,"newScope":scope,"budgetDimensionNames":crate::agent_runtime::multi_agent::budget::DIMENSIONS,
        "originalHistory":source.bounded_fact(),
        "modelBinding":model,"choicePhysicalHash":crate::agent_runtime::store::stable_hash(
            &json!(rows.iter().map(NativeCoordinatorFrozenRows::fact).collect::<Vec<_>>()).to_string()),
        "originalTargetExecutorsVerified":true,
        "oldStopRetained":true,"oldCostsRetained":true,"oldScopeRenewed":false,
        "unknownOriginalWorkReplayAllowed":false,"approval":false,"executionAllowed":false,
        "freshBirthRequired":true,"targetRequestsGranted":0});
    fact["proposalHash"] = crate::agent_runtime::store::stable_hash(&fact.to_string()).into();
    if fact.to_string().len() > 192 * 1024 {
        return Err("protection_recovery_proposal_too_large".into());
    }
    let proof = NativeProtectionRecoveryProposal {
        fact,
        source,
        rows,
        actor,
        _original_target_exit_guards: original_target_exit_guards,
    };
    proof.verify_on(db, db_path, selected, scope)?;
    Ok(proof)
}

fn native_protection_recovery_preview_on(
    db: &mut rusqlite::Connection,
    db_path: &Path,
    selected: &NativeProtectionRecoverySelection,
    scope: &NativeProtectionRecoveryScope,
) -> Result<JsonValue, String> {
    let snapshot = db.transaction().map_err(|e| e.to_string())?;
    let proof = native_protection_recovery_proposal_in(&snapshot, db_path, selected, scope)?;
    proof.verify_on(&snapshot, db_path, selected, scope)?;
    let value = proof.fact.clone();
    snapshot.commit().map_err(|e| e.to_string())?;
    drop(proof);
    Ok(value)
}

#[tauri::command]
pub fn preview_native_protection_recovery_proposal(
    state: State<AppState>,
    selection: NativeProtectionRecoverySelection,
    new_scope: NativeProtectionRecoveryScope,
) -> Result<JsonValue, String> {
    let mut db = native_protection_recovery_open(&state.db_path, false)?;
    native_protection_recovery_preview_on(&mut db, &state.db_path, &selection, &new_scope)
}
