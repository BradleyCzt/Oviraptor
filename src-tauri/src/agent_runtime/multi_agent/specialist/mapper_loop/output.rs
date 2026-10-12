use super::*;

pub(crate) struct VerifiedMapperLoopOutput {
    text: String,
    usage: UsageDelta,
    fact: Value,
}
impl VerifiedMapperLoopOutput {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }
    pub(crate) fn usage(&self) -> &UsageDelta {
        &self.usage
    }
    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
}
pub(crate) fn verified_output_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    bounds: &VerifiedMapperLoopRequestBounds,
) -> Result<VerifiedMapperLoopOutput, String> {
    if db.is_autocommit() {
        return Err("mapper_loop_snapshot_required".into());
    }
    bounds.require_original_on(db, actor)?;
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    if bounds.fact() != grant.input().fact() {
        return Err("mapper_loop_original_input_changed".into());
    }
    verified_original_output_on(db, actor, child)
}
pub(crate) fn verified_original_output_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<VerifiedMapperLoopOutput, String> {
    if db.is_autocommit() {
        return Err("mapper_loop_snapshot_required".into());
    }
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    lookup::original_inventory_on(db, actor, child)?;
    let mut query=db.prepare("SELECT call_id FROM main.agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='dispatch' ORDER BY round_number").map_err(|e|e.to_string())?;
    let ids = query
        .query_map([&child.run_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if ids.is_empty() || ids.len() > 2 {
        return Err("mapper_loop_output_rounds_invalid".into());
    }
    let rounds = ids
        .iter()
        .map(|id| read_received_on(db, actor, child, id))
        .collect::<Result<Vec<_>, _>>()?;
    let first = &rounds[0];
    if first.round != 1 || !first.rejection().is_empty() {
        return Err("mapper_loop_output_original_rejected".into());
    }
    let final_round = rounds.last().ok_or("mapper_loop_output_missing")?;
    if rounds.len() == 1 {
        let audit:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_mapper_loop_journal WHERE child_run_id=?1 AND phase='lookup')",[&child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !first.response.tool_calls.is_empty() || audit {
            return Err("mapper_loop_output_continuation_missing".into());
        }
    } else if final_round.round != 2 || first.response.tool_calls.len() != 1 {
        return Err("mapper_loop_output_chain_invalid".into());
    }
    if !final_round.rejection().is_empty() || !final_round.response.tool_calls.is_empty() {
        return Err("mapper_loop_output_original_rejected".into());
    }
    let value: Value = serde_json::from_str(&final_round.response.text)
        .map_err(|_| "mapper_loop_output_schema_invalid")?;
    let object = value
        .as_object()
        .ok_or("mapper_loop_output_schema_invalid")?;
    if object.len() != 3
        || !object.contains_key("summary")
        || !object.contains_key("priorityContracts")
        || !object.contains_key("risks")
        || object["summary"]
            .as_str()
            .is_none_or(|s| s.is_empty() || s.len() > 512)
    {
        return Err("mapper_loop_output_schema_invalid".into());
    }
    for key in ["priorityContracts", "risks"] {
        let list = object[key]
            .as_array()
            .ok_or("mapper_loop_output_schema_invalid")?;
        if list.len() > 16
            || list
                .iter()
                .any(|v| v.as_str().is_none_or(|s| s.len() > 512))
        {
            return Err("mapper_loop_output_schema_invalid".into());
        }
    }
    let mut usage = UsageDelta {
        input_tokens: 0,
        cached_input_tokens: 0,
        output_tokens: 0,
        total_tokens: 0,
        model_requests: 0,
    };
    for round in &rounds {
        for (sum, n) in [
            (&mut usage.input_tokens, round.response.usage.input_tokens),
            (
                &mut usage.cached_input_tokens,
                round.response.usage.cached_input_tokens,
            ),
            (&mut usage.output_tokens, round.response.usage.output_tokens),
            (&mut usage.total_tokens, round.response.usage.total_tokens),
            (
                &mut usage.model_requests,
                round.response.usage.model_requests,
            ),
        ] {
            *sum = sum
                .checked_add(n)
                .ok_or("mapper_loop_output_usage_overflow")?;
        }
    }
    if usage.model_requests
        != i64::try_from(rounds.len()).map_err(|_| "mapper_loop_output_rounds_invalid")?
    {
        return Err("mapper_loop_output_usage_invalid".into());
    }
    for (dimension, expected) in [
        ("model_input_tokens", usage.input_tokens),
        ("model_cached_tokens", usage.cached_input_tokens),
        ("model_output_tokens", usage.output_tokens),
        ("model_requests", usage.model_requests),
        ("target_requests", 0),
    ] {
        let balance = budget::balance(
            db,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )?;
        if balance.consumed != expected || balance.indeterminate != 0 {
            return Err(format!("mapper_loop_output_fee_unresolved:{dimension}"));
        }
    }
    let foreign:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE child_run_id=?1) OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)",[&child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if foreign {
        return Err("mapper_loop_output_foreign_source".into());
    }
    let fact = json!({"schemaVersion":1,"purpose":"original_frozen_mapper_loop_output","inputHash":grant.input().hash(),"workerAttemptId":grant.worker_attempt_id(),"rounds":rounds.iter().map(|r|json!({"callId":r.id,"requestHash":r.request_hash,"received":r.fact})).collect::<Vec<_>>(),"assessment":value,"usage":usage.as_json(),"targetRequests":0});
    Ok(VerifiedMapperLoopOutput {
        text: value.to_string(),
        usage,
        fact,
    })
}

pub(crate) fn require_settled_output_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    output: &VerifiedMapperLoopOutput,
) -> Result<(), String> {
    let original = verified_original_output_on(db, actor, child)?;
    if original.fact != output.fact
        || original.text != output.text
        || original.usage != output.usage
    {
        return Err("mapper_loop_output_original_changed".into());
    }
    let projection:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.root_run_id=?3 AND a.finished_at<>'' AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>'' AND r.used_tokens=?4 AND r.used_cached_tokens=?5 AND r.used_requests=?6)",params![child.assignment_id,child.run_id,actor.root_run_id,output.usage.total_tokens,output.usage.cached_input_tokens,output.usage.model_requests],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !projection {
        return Err("mapper_loop_output_settlement_projection_changed".into());
    }
    let closed:bool=db.query_row("SELECT NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?1 AND revoked_at='') AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?2) AND EXISTS(SELECT 1 FROM agent_assignment_attempts WHERE assignment_id=?2 AND child_run_id=?1 AND state='completed' AND finished_at<>'')",params![child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !closed {
        return Err("mapper_loop_output_authority_not_closed".into());
    }
    for dimension in &budget::DIMENSIONS[..4] {
        let balance = budget::balance(
            db,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )?;
        if balance.reserved != 0 || balance.indeterminate != 0 {
            return Err("mapper_loop_output_settlement_unresolved".into());
        }
    }
    Ok(())
}
