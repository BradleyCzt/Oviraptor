// Preserve only an original same-attempt selection, never install new supply.
fn native_same_attempt_tool_supply_reentry(
    db_path: &Path,
    scan: &str,
    attempt: i64,
    target: &str,
    candidate: &AgentExecutionPlan,
) -> Result<AgentExecutionPlan, String> {
    let mut db = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ).map_err(|e| e.to_string())?;
    db.pragma_update(None, "query_only", true).map_err(|e| e.to_string())?;
    let snapshot = db.transaction().map_err(|e| e.to_string())?;
    let rows: Vec<(String, String)> = {
        let mut statement = snapshot.prepare(
            "SELECT id,plan_json FROM main.agent_runs WHERE scan_id=?1
             AND attempt_number=?2 AND target_url=?3 AND role='coordinator' ORDER BY id",
        ).map_err(|e| e.to_string())?;
        let rows = statement.query_map(params![scan, attempt, target], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())?;
        rows
    };
    if rows.is_empty() { return Ok(candidate.clone()); }
    if rows.len() != 1 { return Err("tool_supply_reentry_root_ambiguous".into()); }
    let (root, text) = &rows[0];
    let value: JsonValue = serde_json::from_str(text)
        .map_err(|_| "tool_supply_reentry_original_plan_invalid")?;
    if value.get("toolSupply").is_none() { return Ok(candidate.clone()); }
    let original = AgentExecutionPlan::from_json(&value)
        .ok_or("tool_supply_reentry_original_plan_invalid")?;
    let canonical = serde_json::to_string(&original.as_json()).map_err(|e| e.to_string())?;
    if canonical != *text
        || original.attempt_number != attempt || original.target_url != target
        || original.backend != AgentBackendKind::Native
    { return Err("tool_supply_reentry_original_plan_changed".into()); }
    let proof = original_root_capture_mode(&snapshot, root)?;
    if proof.fact().scan_id != scan || proof.fact().attempt_number != attempt
        || proof.fact().mode != crate::agent_runtime::web_mode::WebMode::Multi
        || !proof.fact().targets.iter().any(|url| url == target)
    { return Err("tool_supply_reentry_original_scope_changed".into()); }
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&snapshot, root)?;
    let mut inherited = candidate.clone();
    if inherited.tool_supply.is_some() && inherited.tool_supply != original.tool_supply {
        return Err("tool_supply_reentry_selection_changed".into());
    }
    inherited.tool_supply = original.tool_supply.clone();
    let inherited_text = serde_json::to_string(&inherited.as_json()).map_err(|e| e.to_string())?;
    if inherited_text != *text {
        return Err("agent_attempt_plan_frozen_conflict".into());
    }
    Ok(inherited)
}
