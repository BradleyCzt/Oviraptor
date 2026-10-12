// Read-only candidate configuration, never a backfilled original model grant.
fn native_single_continuation_model_contract_on(
    db: &rusqlite::Connection,
    scan: &str,
    attempt: i64,
    root: &str,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::{multi_agent::budget::continuation, store, web_mode};
    if db.is_autocommit() {
        return Err("single_continuation_model_requires_snapshot".into());
    }
    let (target, raw, plan_hash): (String, String, String) = db
        .query_row(
            "SELECT r.target_url,r.plan_json,r.plan_hash FROM agent_runs r
         JOIN sentinel_scans s ON s.id=r.scan_id JOIN projects p ON p.id=s.project_id
         WHERE r.id=?1 AND r.scan_id=?2 AND r.attempt_number=?3
           AND s.attempt_count=?3 AND s.scan_type='web' AND s.status IN ('paused','partial')
           AND p.status='active' AND r.backend='native' AND r.role='coordinator'
           AND r.orchestration_policy='single' AND r.parent_run_id IS NULL
           AND r.assignment_id='' AND (r.root_run_id=r.id OR r.root_run_id='')
           AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=s.id)",
            params![root, scan, attempt],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "single_continuation_model_scope_invalid")?;
    let value: JsonValue =
        serde_json::from_str(&raw).map_err(|_| "single_continuation_original_plan_changed")?;
    let plan =
        AgentExecutionPlan::from_json(&value).ok_or("single_continuation_original_plan_changed")?;
    let frozen = store::attempt_plan(db, scan, attempt, &target)
        .and_then(|v| AgentExecutionPlan::from_json(&v))
        .ok_or("single_continuation_original_plan_changed")?;
    if plan != frozen
        || plan.hash() != plan_hash
        || plan.target_url != target
        || plan.attempt_number != attempt
        || !matches!(plan.schema_version, 1 | 2)
    {
        return Err("single_continuation_original_plan_changed".into());
    }
    let mode = original_root_capture_mode(db, root)?;
    let declaration =
        web_mode::root::read(db, root)?.ok_or("single_continuation_original_mode_missing")?;
    if mode.fact().mode != web_mode::WebMode::Single
        || declaration.mode() != web_mode::WebMode::Single
        || mode.fact() != declaration.fact()
    {
        return Err("single_continuation_original_mode_changed".into());
    }
    declaration.validate_scope(scan, attempt, &target, &plan_hash)?;
    let _source = continuation::single::original_source_on(db, root)?;
    let requests: i64 = db.query_row(
        "SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='dispatch'",
        [root], |r| r.get(0),
    ).map_err(|e| e.to_string())?;
    if requests == 0 {
        return Err("single_continuation_original_model_request_missing".into());
    }
    let original_requests = NativeCoordinatorFrozenRows::capture(
        db,
        "SELECT rowid,* FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY rowid",
        &[root],
    )?;
    let settings = sentinel_settings(db);
    let environment = match model_runtime_env(&settings) {
        Ok(environment) => environment,
        Err(_) => return Ok(json!({
            "schemaVersion": 1,
            "purpose": "single_model_contract_readonly",
            "rootRunId": root,
            "originalPlanHash": plan_hash,
            "originalRequestPhysicalHash": original_requests.hash,
            "modelCandidates": [],
            "modelConfigurationState": "unavailable",
            "endpointBindingMeaning": "current_configured_upstream_not_original_transport",
            "originalOutputCapAuthenticated": false,
            "completeInputUpperBoundAuthenticated": false,
            "executionAllowed": false,
            "approvalBlocked": true,
            "approvalBlockReasons": ["current_model_configuration_unavailable",
                "original_single_model_contract_not_authenticated",
                "frozen_complete_model_input_upper_bound_missing"]
        })),
    };
    let mut proxies = vec![None];
    proxies.extend(
        approved_agent_proxies(&settings)
            .into_iter()
            .map(|(_, url)| Some(url)),
    );
    let mut candidates = Vec::new();
    for proxy in proxies {
        let profile = agent_model_profile(&environment, proxy.as_deref())?;
        // Exact Single transport policy in agent_native/root_model.rs. There
        // is no min(2048) or local-window clipping; the gateway rejects it.
        let output = native_single_model_output_cap(&profile);
        if output == 0
            || output > i64::MAX as u64
            || (profile.max_context_tokens != 0 && output >= profile.max_context_tokens)
        {
            return Err("single_continuation_model_output_limit_invalid".into());
        }
        let mut candidate = json!({
            "model": profile.model,
            "endpointBinding": store::stable_hash(&profile.endpoint),
            "credentialBinding": store::stable_hash(profile.bearer().unwrap_or("")),
            "proxyBinding": store::stable_hash(profile.proxy.as_deref().unwrap_or("")),
            "deployment": if profile.local { "local" } else { "cloud" },
            "maxContextTokens": profile.max_context_tokens,
            "configuredMaxOutputTokens": profile.max_output_tokens,
            "maxOutputTokens": output
        });
        candidate["modelBindingHash"] = store::stable_hash(&candidate.to_string()).into();
        candidates.push(candidate);
    }
    Ok(json!({
        "schemaVersion": 1,
        "purpose": "single_model_contract_readonly",
        "rootRunId": root,
        "originalPlanHash": plan_hash,
        "originalRequestPhysicalHash": original_requests.hash,
        "modelCandidates": candidates,
        "modelConfigurationState": "resolved",
        "endpointBindingMeaning": "current_configured_upstream_not_original_transport",
        "originalOutputCapAuthenticated": false,
        "completeInputUpperBoundAuthenticated": false,
        "executionAllowed": false,
        "approvalBlocked": true,
        "approvalBlockReasons": ["original_single_model_contract_not_authenticated",
            "frozen_complete_model_input_upper_bound_missing"]
    }))
}
