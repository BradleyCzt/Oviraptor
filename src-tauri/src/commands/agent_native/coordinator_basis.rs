// Captured original request basis, checked again after the last business write.
fn native_coordinator_basis_on(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<JsonValue, String> {
    let (evidence, evidence_file_hash) = native_coordinator_frozen_evidence(db, context)?;
    let mut basis = json!({"version":1,"phase":"bootstrap","root":actor.root_run_id,
        "scan":actor.scan_id,"attempt":actor.attempt_number,"target":actor.target_key,
        "planHash":context.execution_plan.hash(),
        "frozenEvidenceHash":crate::agent_runtime::store::stable_hash(&evidence.to_string()),
        "frozenEvidence":evidence,"evidenceFileHash":evidence_file_hash,
        "capabilityClass":"coordination-local-only",
        "coordinator":{"epoch":actor.lease_epoch,"fence":actor.fencing_token}});
    if let Some(mode) = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)? {
        if let Some(bootstrap) = mode.bootstrap_dispatch() {
            basis["permittedSuggestion"] = bootstrap["permittedStep"].clone();
            basis["bootstrapDispatch"] = bootstrap;
        }
        if let Some(observation) = mode.live_budget_observation() {
            basis["liveBudgetObservation"] = observation;
        }
        if let Some(protocol) = mode.lease_expiry_supervision() {
            basis["leaseExpirySupervision"] = protocol;
        }
        if let Some(protocol) = mode.new_evidence_supervision() {
            basis["newEvidenceSupervision"] = protocol;
        }
        if let Some(local) = mode.local_deliberation() {
            crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
                db,
                &actor.root_run_id,
            )?;
            crate::agent_runtime::multi_agent::budget::limits::verify_root_contract(
                db,
                &actor.root_run_id,
            )?;
            let mut statement=db.prepare("SELECT dimension,hard_limit FROM agent_budget_limits WHERE root_run_id=?1 ORDER BY dimension").map_err(|e|e.to_string())?;
            let rows = statement
                .query_map([&actor.root_run_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
                })
                .map_err(|e| e.to_string())?;
            let mut limits = serde_json::Map::new();
            for row in rows {
                let (dimension, limit) = row.map_err(|e| e.to_string())?;
                limits.insert(dimension, json!(limit));
            }
            basis["localDeliberation"] = local;
            basis["originalHardLimits"] = json!(limits);
        }
    }
    if let Some(binding) =
        crate::agent_runtime::multi_agent::budget::root::model::tick::financial_purpose::binding_on(
            db,
            &actor.root_run_id,
        )?
    {
        if [
            "localDeliberation",
            "bootstrapDispatch",
            "permittedSuggestion",
            "liveBudgetObservation",
            "newEvidenceSupervision",
            "leaseExpirySupervision",
        ]
        .iter()
        .any(|key| basis.get(*key).is_some())
        {
            return Err("financial_continuation_requires_plain_root".into());
        }
        basis["phase"] = "financial-continuation".into();
        basis["taskDescription"] = binding["taskDescription"].clone();
        basis["financialContinuation"] = binding;
    }
    if basis.to_string().len() > 256 * 1024 {
        return Err("root_tick_basis_too_large".into());
    }
    Ok(basis)
}
fn native_coordinator_verify_original_basis(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    captured: &JsonValue,
) -> Result<(), String> {
    if native_coordinator_basis_on(db, context, actor)? != *captured {
        return Err("root_tick_original_basis_changed".into());
    }
    Ok(())
}
