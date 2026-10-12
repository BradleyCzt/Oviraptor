// Phase one is frozen-evidence candidate analysis; General ReAct is still pending.
fn validate_business_logic_readonly_assessment(text: &str) -> Result<JsonValue, String> {
    let value: JsonValue =
        serde_json::from_str(text).map_err(|_| "business_logic_output_invalid")?;
    let object = value.as_object().ok_or("business_logic_output_invalid")?;
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort();
    if keys
        != [
            "confirmedFindings",
            "evidenceGaps",
            "flowCandidates",
            "schemaVersion",
            "summary",
        ]
        || value["schemaVersion"].as_i64() != Some(1)
        || value["confirmedFindings"] != false
    {
        return Err("business_logic_output_schema_invalid".into());
    }
    let string = |v: &JsonValue| {
        v.as_str()
            .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= 512)
    };
    if !string(&value["summary"]) {
        return Err("business_logic_output_summary_invalid".into());
    }
    let gaps = value["evidenceGaps"]
        .as_array()
        .ok_or("business_logic_output_gaps_invalid")?;
    let candidates = value["flowCandidates"]
        .as_array()
        .ok_or("business_logic_output_candidates_invalid")?;
    if gaps.len() > 16 || candidates.len() > 16 || gaps.iter().any(|v| !string(v)) {
        return Err("business_logic_output_bounds_invalid".into());
    }
    for candidate in candidates {
        let c = candidate
            .as_object()
            .ok_or("business_logic_output_candidate_invalid")?;
        let mut names = c.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort();
        let refs = candidate["evidenceRefs"]
            .as_array()
            .ok_or("business_logic_output_refs_invalid")?;
        if names != ["evidenceRefs", "hypothesis", "name"]
            || !string(&candidate["name"])
            || !string(&candidate["hypothesis"])
            || refs.is_empty()
            || refs.len() > 16
            || refs.iter().any(|r| r.as_str() != Some("frontend-evidence"))
        {
            return Err("business_logic_output_candidate_invalid".into());
        }
    }
    if crate::agent_runtime::secrets::redact_json(&value) != value {
        return Err("business_logic_output_secret_denied".into());
    }
    Ok(value)
}

fn multi_agent_business_logic_readonly(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<JsonValue, String> {
    let (payload, child) = multi_agent_business_logic_readonly_output_on(context, lease)?;
    // Closure has committed. A failed Root advisory must never clean up or
    // rewrite the already completed child, its fee, ACK or original source.
    native_coordinator_business_logic_output_feedback(context, lease, &child)?;
    Ok(payload)
}

// A bounded first-slice receipt endpoint: genuine child SDK/closure only.
// This is not evidence of a paid Root output assessment.
#[cfg(test)]
fn multi_agent_business_logic_readonly_output_only(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<JsonValue, String> {
    multi_agent_business_logic_readonly_output_on(context, lease).map(|(payload, _)| payload)
}

fn multi_agent_business_logic_readonly_output_on(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<
    (
        JsonValue,
        crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ),
    String,
> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{lease::require_executable_coordinator, scheduler},
    };
    let db = db::open(&context.db_path)?;
    let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
    if context.scan_id != lease.scan_id
        || context.attempt_number != lease.attempt_number
        || context.target_url != lease.target_key
    {
        return Err("business_logic_context_scope_invalid".into());
    }
    // Inventory is discovery only; mutable completed state is never proof.
    // Include original sealed rows so hiding/changing an assignment cannot
    // silently create a second worker or a replacement receipt.
    let inventory: Vec<(String,String,String,String,String,bool)> = snapshot.prepare(
        "SELECT a.id,a.child_run_id,a.role,r.role,a.trigger_code,
         (a.state='completed' OR EXISTS(SELECT 1 FROM agent_business_logic_output_receipts o WHERE o.assignment_id=a.id OR o.child_run_id=a.child_run_id))
         FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         WHERE (a.coordinator_run_id=?1 OR r.root_run_id=?1) AND (a.role='business_logic' OR r.role='business_logic')"
    ).map_err(|e|e.to_string())?.query_map([&lease.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))
        .map_err(|e|e.to_string())?.collect::<rusqlite::Result<_>>().map_err(|e|e.to_string())?;
    let orphaned: bool = snapshot.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_business_logic_output_receipts o WHERE o.root_run_id=?1 AND NOT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=o.assignment_id AND a.child_run_id=o.child_run_id AND a.coordinator_run_id=?1 AND r.root_run_id=?1 AND a.role='business_logic' AND r.role='business_logic'))",
        [&lease.root_run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if orphaned || inventory.len() > 1 {
        return Err("business_logic_original_worker_conflict".into());
    }
    if let Some((assignment_id, run_id, role, run_role, trigger, closed)) =
        inventory.into_iter().next()
    {
        if role != "business_logic"
            || run_role != "business_logic"
            || trigger != "business_logic_frozen_evidence_ready"
        {
            return Err("business_logic_original_worker_scope_changed".into());
        }
        if closed {
            let child = scheduler::ScheduledChild {
                assignment_id,
                run_id,
                role: AgentRole::BusinessLogic,
            };
            let receipt =
                business_logic_output_receipt::read_original_on(&snapshot, lease, &child)?
                    .ok_or("business_logic_original_output_receipt_missing")?;
            let payload = receipt.fact()["collection"]["payload"].clone();
            snapshot.rollback().map_err(|e| e.to_string())?;
            return Ok((payload, child));
        }
    }
    let mode = crate::agent_runtime::web_mode::root::read(&snapshot, &lease.root_run_id)?
        .ok_or("business_logic_original_birth_missing")?;
    if mode.bootstrap_dispatch().is_some_and(|p| p["schemaVersion"] == 14) {
        return Err("business_logic_paid_dispatch_required".into());
    }
    let source = native_business_logic_input_collection_on(&snapshot, lease, &context.target_dir)?;
    let (evidence, hash) = native_coordinator_frozen_evidence(&snapshot, context)?;
    if source.input()["frozenEvidence"] != evidence
        || source.fact()["originalEvidenceSha256"] != hash
    {
        return Err("business_logic_context_evidence_changed".into());
    }
    snapshot.rollback().map_err(|e| e.to_string())?;
    // Actual scheduling uses original hard limits and the shared Reviewer floor.
    let child = scheduler::prepare_readonly_child_checked(
        &db,
        lease,
        AgentRole::BusinessLogic,
        "business_logic_frozen_evidence_ready",
        source.task(),
        2000,
        |tx| {
            crate::agent_runtime::multi_agent::budget::supervision_capacity::require_business_logic_issue_headroom_on(tx, lease, 1)?;
            let current =
                native_business_logic_input_collection_on(tx, lease, &context.target_dir)?;
            if current.fact() != source.fact() {
                return Err("business_logic_original_input_changed".into());
            }
            agent_require_frozen_web_plan(tx, context).map_err(str::to_string)?;
            require_executable_coordinator(tx, lease)
        },
    )?;
    let delivered = multi_agent_business_logic_execute_prepared(context, lease, &child)?;
    Ok((delivered, child))
}

// Execute the original already-minted child; never schedule a second worker.
fn multi_agent_business_logic_execute_prepared(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<JsonValue, String> {
    let db = db::open(&context.db_path)?;
    let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
    if child.role != crate::agent_runtime::contract::AgentRole::BusinessLogic
        || context.scan_id != lease.scan_id
        || context.attempt_number != lease.attempt_number
        || context.target_url != lease.target_key
    { return Err("business_logic_prepared_context_scope_invalid".into()); }
    let source = native_business_logic_input_collection_on(&snapshot, lease, &context.target_dir)?;
    snapshot.rollback().map_err(|e| e.to_string())?;
    let mut isolated = context.clone();
    isolated.run = Some(AgentRunLedger {
        db_path: context.db_path.clone(),
        run_id: child.run_id.clone(),
    });
    isolated.identities = vec![AgentIdentity::anonymous()];
    isolated.browser = None;
    // Use the actual transport in tests as well: never a deterministic role double.
    let (text, usage) = multi_agent_child_round_transport(
        &isolated,
        lease,
        child,
        BUSINESS_LOGIC_READONLY_SYSTEM,
        source.input().clone(),
    )?;
    validate_business_logic_readonly_assessment(&text)?;
    let payload = json!({"summary":text,"businessLogicInput":source.fact(),"state":"completed","scope":"frozen_business_logic_candidates_only","targetRequests":0,"confirmedFinding":false});
    // Protected completion commits the original output receipt in the same writer.
    let delivered = deliver_readonly_assessment(
        &db,
        lease,
        child,
        &usage,
        &payload,
        Some(&context.target_dir),
    )?;
    Ok(delivered)
}
