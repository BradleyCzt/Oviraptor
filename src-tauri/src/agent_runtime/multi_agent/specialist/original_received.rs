//! Pure original Surface or BusinessLogic SDK receipt; never execution authority.
use super::*;

pub(crate) fn received_original_for_output_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<StoredResponse, String> {
    if db.is_autocommit()
        || !matches!(
            child.role,
            AgentRole::ExternalSurface | AgentRole::BusinessLogic
        )
    {
        return Err("specialist_output_original_context_invalid".into());
    }
    super::super::budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    let bound:bool=db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
         WHERE a.id=?1 AND a.child_run_id=?2 AND r.assignment_id=a.id
         AND a.coordinator_run_id=?3 AND r.root_run_id=?3 AND r.parent_run_id=?3
         AND r.scan_id=?4 AND r.attempt_number=?5 AND r.target_url=?6 AND a.target_key=?6
         AND a.lease_epoch=?7 AND a.fencing_token=?8
         AND a.role=?9 AND r.role=a.role
         AND a.lane=?10 AND r.lane=a.lane
         AND r.backend='native' AND r.orchestration_policy='multi'
         AND ((a.state='running' AND r.status='running' AND r.terminal_state='' AND a.budget_settled_at='')
           OR (a.state='completed' AND r.status='terminal' AND r.terminal_state='completed'
             AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0
             AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=a.id)
             AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=r.id AND revoked_at=''))))",
        params![child.assignment_id,child.run_id,actor.root_run_id,actor.scan_id,
            actor.attempt_number,actor.target_key,actor.lease_epoch,actor.fencing_token,child.role.as_str(),if child.role==AgentRole::ExternalSurface {"target_touching"}else{"read_only_analysis"}],
        |r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound {
        return Err("specialist_output_original_scope_or_state_invalid".into());
    }
    let worker = super::super::attempts::current(db, actor, &child.assignment_id)?;
    if worker.child_run_id != child.run_id
        || worker.lease_epoch != 1
        || !matches!(worker.state.as_str(), "running" | "completed")
        || !worker.failure_class.is_empty()
        || (worker.state == "running" && !worker.finished_at.is_empty())
        || (worker.state == "completed" && worker.finished_at.is_empty())
    {
        return Err("specialist_output_original_worker_invalid".into());
    }
    let original = super::super::budget::receipts::original_owner(
        db,
        &child.run_id,
        actor,
        &child.assignment_id,
    )?;
    original.verify(db)?;
    if original.attempt_id() != worker.id || original.is_unresolved(db)? {
        return Err("specialist_output_original_fee_unresolved".into());
    }
    let request_hash:String=db.query_row(
        "SELECT request_hash FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND state='received'",
        params![child.assignment_id,child.run_id],|r|r.get(0)).optional().map_err(|e|e.to_string())?
        .ok_or("specialist_output_original_received_missing")?;
    let (request,task):(String,String)=db.query_row(
        "SELECT c.request_json,a.task_slice_json FROM agent_specialist_calls c JOIN agent_assignments a ON a.id=c.assignment_id WHERE c.assignment_id=?1 AND c.child_run_id=?2",
        params![child.assignment_id,child.run_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let frozen: Value =
        serde_json::from_str(&request).map_err(|_| "specialist_output_original_request_invalid")?;
    let task: Value =
        serde_json::from_str(&task).map_err(|_| "specialist_output_original_contract_invalid")?;
    let messages = frozen["messages"]
        .as_array()
        .ok_or("specialist_output_original_request_invalid")?;
    if frozen["schemaVersion"] != 1
        || frozen["tools"] != serde_json::json!([])
        || messages.len() != 2
        || messages[0]["role"] != "system"
        || messages[1]["role"] != "user"
    {
        return Err("specialist_output_original_request_invalid".into());
    }
    let input: Value = serde_json::from_str(
        messages[1]["content"]
            .as_str()
            .ok_or("specialist_output_original_input_invalid")?,
    )
    .map_err(|_| "specialist_output_original_input_invalid")?;
    if child.role == AgentRole::ExternalSurface {
        if input["contract"] != task || !input["observedCapture"].is_object() {
            return Err("specialist_output_original_contract_binding_invalid".into());
        }
    } else {
        let directory = crate::commands::resolve_agent_evidence_location(db, &actor.root_run_id)?;
        let collection =
            crate::commands::native_business_logic_input_collection_on(db, actor, &directory)?;
        let (trigger, revision): (String, i64) = db
            .query_row(
                "SELECT trigger_code,evidence_revision FROM agent_assignments WHERE id=?1",
                [&child.assignment_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        if task != *collection.task()
            || input != *collection.input()
            || messages[0]["content"] != crate::commands::BUSINESS_LOGIC_READONLY_SYSTEM
            || trigger != "business_logic_frozen_evidence_ready"
            || revision != 1
        {
            return Err("specialist_business_original_contract_binding_invalid".into());
        }
        let key = super::super::contract_owner::schedule_contract_key(
            actor.attempt_number,
            &actor.target_key,
            child.role.as_str(),
            &trigger,
            revision,
        );
        super::super::contract_owner::verify_held_owner(
            db,
            &actor.root_run_id,
            &key,
            &child.assignment_id,
            actor.lease_epoch,
            &actor.fencing_token,
        )?;
    }
    let call = PendingCall {
        lease: actor.clone(),
        child: child.clone(),
        request_hash,
    };
    let row = load(db, &call)?.ok_or("specialist_output_original_received_missing")?;
    if !row.failure_code.is_empty() || row.response["usageReported"] != true {
        return Err("specialist_output_original_fee_unresolved".into());
    }
    let saved = verify_received(db, &call, &row)?;
    if !saved.rejection.is_empty() {
        return Err("specialist_output_original_response_rejected".into());
    }
    if child.role == AgentRole::BusinessLogic {
        require_business_logic_resources_on(db, actor, child, &worker.id, &saved)?;
    }
    Ok(saved)
}

fn require_business_logic_resources_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
    saved: &StoredResponse,
) -> Result<(), String> {
    let raw: String = db
        .query_row(
            "SELECT capability_lease_json FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut caps: Vec<String> =
        serde_json::from_str(&raw).map_err(|_| "specialist_business_original_caps_invalid")?;
    caps.sort();
    let state: String = db
        .query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let completed = state == "completed";
    let (count,exact):(i64,i64)=db.query_row("SELECT count(*),sum(CASE WHEN assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 AND capability IN ('evidence.read','mailbox.write') AND ((?6 AND revoked_at<>'') OR (NOT ?6 AND revoked_at='')) THEN 1 ELSE 0 END) FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2",params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token,completed],|r|Ok((r.get(0)?,r.get::<_,Option<i64>>(1)?.unwrap_or(0)))).map_err(|e|e.to_string())?;
    if caps != vec!["evidence.read".to_string(), "mailbox.write".into()]
        || count != 2
        || exact != 2
        || saved.usage.model_requests != 1
    {
        return Err("specialist_business_original_caps_invalid".into());
    }
    let foreign:bool=db.query_row("SELECT (SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1 OR child_run_id=?2)<>1 OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?2) OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?2) OR EXISTS(SELECT 1 FROM agent_external_surface_captures WHERE assignment_id=?1 OR child_run_id=?2) OR EXISTS(SELECT 1 FROM agent_web_model_journal WHERE assignment_id=?1 OR child_run_id=?2)",params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if foreign {
        return Err("specialist_business_original_foreign_execution".into());
    }
    let clock = super::super::budget::clock::original_worker_samples::original_worker_consumed_on(
        db, actor, child, worker,
    )?;
    let mut original_tokens = None;
    for (index, dimension) in super::super::budget::DIMENSIONS.iter().enumerate() {
        let balance = super::super::budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            worker,
            dimension,
        )?;
        let expected = match index {
            0 => saved.usage.input_tokens,
            1 => saved.usage.cached_input_tokens,
            2 => saved.usage.output_tokens,
            3 => 1,
            9 => clock,
            _ => 0,
        };
        let reserve_ok = if index < 4 {
            // The original grant reserves every token class, including a
            // provider's explicitly reported zero cached usage. Zero charges
            // legitimately have no consume row; original verify_received above
            // authenticates the reported breakdown and exact receipt rows.
            let (count,initial):(i64,i64)=db.query_row("SELECT count(*),COALESCE(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension=?4 AND kind='reserve' AND source_id='assignment:'||?2",params![actor.root_run_id,child.assignment_id,worker,dimension],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
            if count != 1
                || initial <= 0
                || (index == 3 && initial != 1)
                || (index < 3 && original_tokens.is_some_and(|n| n != initial))
            {
                return Err(format!(
                    "specialist_business_original_grant_invalid:{dimension}"
                ));
            }
            if index < 3 {
                original_tokens = Some(initial);
            }
            if completed {
                balance.reserved == 0
            } else {
                balance.reserved
                    == initial
                        .checked_sub(expected)
                        .ok_or("budget_counter_overflow")?
            }
        } else {
            // v13 concurrency is carried by original execution-slot facts;
            // target/browser/write/upload also retain no budget reservation.
            balance.reserved == 0
        };
        if balance.indeterminate != 0 || balance.consumed != expected || !reserve_ok {
            return Err(format!(
                "specialist_business_original_fee_invalid:{dimension}"
            ));
        }
    }
    Ok(())
}
