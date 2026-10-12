// Only the original private startup reader can construct this source.
pub(crate) struct OriginalToolSelectionPolicySource {
    mode: VerifiedMode,
    canonical_policy: String,
}
impl OriginalToolSelectionPolicySource {
    pub(crate) fn mode(&self) -> &VerifiedMode { &self.mode }
    pub(crate) fn canonical_policy(&self) -> &str { &self.canonical_policy }
}
pub(crate) fn capture_original_tool_selection_policy_on(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
) -> Result<OriginalToolSelectionPolicySource, String> {
    if db.is_autocommit() {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return capture_original_tool_selection_policy_on(&snapshot, scan, attempt);
    }
    let mode = private_web_mode_on(db, scan, attempt)?;
    if mode.fact().mode != WebMode::Multi {
        return Err("tool_selection_original_multi_required".into());
    }
    let directory = WebBindingDirectory::open(Path::new(&mode.fact().work_dir))?;
    let task = directory.read("task.json", WEB_DISPATCH_BINDING_LIMIT, false, false)?
        .ok_or("web_mode_task_missing")?;
    let document: JsonValue = serde_json::from_slice(&task)
        .map_err(|_| "web_mode_task_invalid")?;
    let canonical_policy = serde_json::to_string(&document["effectiveWebPolicy"])
        .map_err(|e| e.to_string())?;
    if format!("{:x}", Sha256::digest(&task)) != mode.fact().task_hash
        || crate::agent_runtime::store::stable_hash(&canonical_policy) != mode.fact().policy_hash
        || document["scanId"] != scan
        || mode_task_targets(&document)? != mode.fact().targets
    {
        return Err("tool_selection_original_signed_policy_changed".into());
    }
    directory.check_identity()?;
    let confirmed = private_web_mode_on(db, scan, attempt)?;
    if confirmed.fact() != mode.fact() {
        return Err("tool_selection_original_signed_policy_changed".into());
    }
    Ok(OriginalToolSelectionPolicySource { mode, canonical_policy })
}

fn native_tool_selection_for_original_startup(
    db_path: &Path, scan: &str, attempt: i64, target: &str,
    candidate: &AgentExecutionPlan,
    configuration: Option<&crate::tool_supply::ToolAdministratorConfiguration>,
) -> Result<AgentExecutionPlan, String> {
    let mut db = rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| e.to_string())?;
    db.pragma_update(None, "query_only", true).map_err(|e| e.to_string())?;
    let snapshot = db.transaction().map_err(|e| e.to_string())?;
    let proof = private_web_mode_on(&snapshot, scan, attempt)?;
    if proof.fact().mode == WebMode::Single {
        if candidate.tool_supply.as_ref().is_some_and(|v| !v.approval_ids().is_empty()) {
            return Err("tool_selection_original_multi_required".into());
        }
        return Ok(candidate.clone());
    }
    let source = capture_original_tool_selection_policy_on(&snapshot, scan, attempt)?;
    let policy: JsonValue = serde_json::from_str(source.canonical_policy()).map_err(|e| e.to_string())?;
    let Some(pending) = policy.get("toolSupplySelection") else {
        if candidate.tool_supply.as_ref().is_some_and(|v| !v.approval_ids().is_empty()) {
            return Err("tool_selection_original_signed_request_missing".into());
        }
        return Ok(candidate.clone());
    };
    let configuration = configuration.ok_or("tool_selection_app_configuration_missing")?;
    let request_id = pending["prepared"]["request"]["requestId"].as_str()
        .ok_or("tool_selection_original_pending_changed")?;
    let original_source = crate::tool_supply::selection::OriginalSignedToolSelectionPolicy::capture_original_on(
        &snapshot, scan, attempt,
    )?;
    let verified = crate::tool_supply::selection::require_current_signed_on(
        &snapshot, &original_source, configuration, request_id,
    )?;
    if !proof.fact().targets.iter().any(|url| url == target)
        || candidate.attempt_number != attempt || candidate.target_url != target
        || candidate.tool_supply.as_ref().is_some_and(|selection| selection != verified.selection())
    { return Err("tool_selection_original_scope_changed".into()); }
    let mut plan = candidate.clone();
    plan.tool_supply = Some(verified.selection().clone());
    Ok(plan)
}
