// Pure phase-one BusinessLogic result; never a General ReAct execution grant.
pub(crate) struct VerifiedBusinessLogicOutputCollection {
    fact: JsonValue,
}
impl VerifiedBusinessLogicOutputCollection {
    pub(crate) fn fact(&self) -> &JsonValue {
        &self.fact
    }
}
pub(crate) fn native_business_logic_output_collection_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    target_dir: &Path,
) -> Result<VerifiedBusinessLogicOutputCollection, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{attempts, budget, contract_owner},
    };
    if db.is_autocommit() || child.role != AgentRole::BusinessLogic {
        return Err("business_logic_output_collection_snapshot_role_required".into());
    }
    budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    let original = original_root_capture_mode(db, &actor.root_run_id)?;
    let directory = checked_evidence_directory(
        Path::new(&original.fact().work_dir),
        target_dir,
        &actor.scan_id,
    )?;
    if directory != target_dir
        || resolve_agent_evidence_location(db, &actor.root_run_id)? != directory
    {
        return Err("business_logic_output_collection_original_directory_changed".into());
    }
    let input_source = native_business_logic_input_collection_on(db, actor, target_dir)?;
    let worker = attempts::current(db, actor, &child.assignment_id)?;
    let (task,state,revision):(String,String,i64)=db.query_row("SELECT a.task_slice_json,a.state,a.evidence_revision FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.trigger_code='business_logic_frozen_evidence_ready' AND a.contract_keys_json='[]' AND a.identity_handles_json='[]' AND a.failure_class='' AND r.cancel_requested_at='' AND ((a.state='running' AND a.finished_at='' AND a.budget_settled_at='' AND r.status='running' AND r.finished_at='' AND r.terminal_state='') OR (a.state='completed' AND a.finished_at<>'' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>''))",params![child.assignment_id,child.run_id,actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"business_logic_output_collection_normal_child_required")?;
    let completed = state == "completed";
    if revision != 1
        || worker.child_run_id != child.run_id
        || worker.lease_epoch != 1
        || !worker.failure_class.is_empty()
        || worker.state != state
        || (completed == worker.finished_at.is_empty())
    {
        return Err("business_logic_output_collection_original_worker_changed".into());
    }
    let contract: JsonValue = serde_json::from_str(&task)
        .map_err(|_| "business_logic_output_collection_original_contract_changed")?;
    if contract != *input_source.task() {
        return Err("business_logic_output_collection_original_contract_changed".into());
    }
    let key = contract_owner::schedule_contract_key(
        actor.attempt_number,
        &actor.target_key,
        "business_logic",
        "business_logic_frozen_evidence_ready",
        revision,
    );
    let owned:bool=db.query_row("SELECT count(*)=1 FROM agent_contract_owners WHERE assignment_id=?1 AND root_run_id=?2 AND contract_key=?3 AND lease_epoch=?4 AND fencing_token=?5",params![child.assignment_id,actor.root_run_id,key,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
    let caps: String = db
        .query_row(
            "SELECT capability_lease_json FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut caps: Vec<String> = serde_json::from_str(&caps)
        .map_err(|_| "business_logic_output_collection_original_capabilities_changed")?;
    caps.sort();
    let expected = vec!["evidence.read".to_string(), "mailbox.write".into()];
    let cap_count:i64=db.query_row("SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2",params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let exact_caps:i64=db.query_row("SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 AND capability IN ('evidence.read','mailbox.write') AND ((?6 AND revoked_at<>'') OR (NOT ?6 AND revoked_at=''))",params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token,completed],|r|r.get(0)).map_err(|e|e.to_string())?;
    let (lanes,scoped):(i64,i64)=db.query_row("SELECT count(*),sum(CASE WHEN scan_id=?2 AND attempt_number=?3 AND target_key=?4 AND lane='read_only_analysis' THEN 1 ELSE 0 END) FROM agent_lane_leases WHERE assignment_id=?1",params![child.assignment_id,actor.scan_id,actor.attempt_number,actor.target_key],|r|Ok((r.get(0)?,r.get::<_,Option<i64>>(1)?.unwrap_or(0)))).map_err(|e|e.to_string())?;
    if !owned
        || caps != expected
        || cap_count != 2
        || exact_caps != 2
        || (completed && lanes != 0)
        || (!completed && (lanes != 1 || scoped != 1))
    {
        return Err("business_logic_output_collection_original_resources_changed".into());
    }
    let received = crate::agent_runtime::multi_agent::specialist::received_original_for_output_on(
        db, actor, child,
    )?;
    if !received.rejection.is_empty() || received.usage.model_requests != 1 {
        return Err("business_logic_output_collection_original_model_rejected".into());
    }
    validate_business_logic_readonly_assessment(&received.text)?;
    let payload = json!({"summary":received.text,"businessLogicInput":input_source.fact(),"state":"completed","scope":"frozen_business_logic_candidates_only","targetRequests":0,"confirmedFinding":false});
    let graph_count:i64=db.query_row("SELECT count(*) FROM agent_evidence_nodes WHERE root_run_id=?1 AND created_by_run_id=?2 AND kind='request_record'",params![actor.root_run_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let run_usage: (i64, i64, i64) = db
        .query_row(
            "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1",
            [&child.run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|e| e.to_string())?;
    let expected_run_usage = if completed {
        (
            received.usage.total_tokens,
            received.usage.cached_input_tokens,
            received.usage.model_requests,
        )
    } else {
        (0, 0, 0)
    };
    if graph_count != 0 || run_usage != expected_run_usage {
        return Err("business_logic_output_collection_original_graph_or_usage_changed".into());
    }
    let (request_raw,response_raw):(String,String)=db.query_row("SELECT request_json,response_json FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND role='business_logic' AND state='received'",params![child.assignment_id,child.run_id,actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"business_logic_output_collection_original_model_missing")?;
    let request: JsonValue = serde_json::from_str(&request_raw)
        .map_err(|_| "business_logic_output_collection_original_model_request_changed")?;
    let actual_response: JsonValue = serde_json::from_str(&response_raw)
        .map_err(|_| "business_logic_output_collection_original_model_response_changed")?;
    // Request transport shape is verified by the original specialist reader;
    // its complete input must equal the independently reconstructed original input.
    let messages = request["messages"]
        .as_array()
        .ok_or("business_logic_output_collection_original_model_request_changed")?;
    if request["schemaVersion"] != 1
        || request["tools"] != json!([])
        || messages.len() != 2
        || messages[0]["role"] != "system"
        || messages[1]["role"] != "user"
        || messages[0]["content"] != BUSINESS_LOGIC_READONLY_SYSTEM
        || messages
            .iter()
            .any(|m| m.as_object().is_none_or(|o| o.len() != 2))
    {
        return Err("business_logic_output_collection_original_model_request_changed".into());
    }
    let input: JsonValue = serde_json::from_str(
        messages[1]["content"]
            .as_str()
            .ok_or("business_logic_output_collection_original_model_request_changed")?,
    )
    .map_err(|_| "business_logic_output_collection_original_model_request_changed")?;
    let expected_input = input_source.input().clone();
    if input != expected_input {
        return Err("business_logic_output_collection_original_capture_input_changed".into());
    }
    let foreign:bool=db.query_row("SELECT (SELECT count(*) FROM agent_specialist_calls WHERE assignment_id=?1 OR child_run_id=?2)<>1 OR EXISTS(SELECT 1 FROM agent_web_model_journal WHERE assignment_id=?1 OR child_run_id=?2) OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?2) OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?2) OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE (assignment_id=?1 OR lease_attempt_id=?3) AND (root_run_id<>?4 OR assignment_id<>?1 OR lease_attempt_id<>?3))",params![child.assignment_id,child.run_id,worker.id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if foreign {
        return Err("business_logic_output_collection_foreign_execution_changed".into());
    }
    let clock = budget::clock::original_worker_samples::original_worker_consumed_on(
        db, actor, child, &worker.id,
    )?;
    for (index, dimension) in budget::DIMENSIONS.iter().enumerate() {
        let b = budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            &worker.id,
            dimension,
        )?;
        let expected = match index {
            0 => received.usage.input_tokens,
            1 => received.usage.cached_input_tokens,
            2 => received.usage.output_tokens,
            3 => 1,
            4 => 0,
            9 => clock,
            _ => 0,
        };
        let initial:i64=db.query_row("SELECT COALESCE(sum(amount),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension=?4 AND kind='reserve' AND source_id='assignment:'||?2",params![actor.root_run_id,child.assignment_id,worker.id,dimension],|r|r.get(0)).map_err(|e|e.to_string())?;
        let reserve_ok = if completed {
            b.reserved == 0
        } else if index < 4 {
            b.reserved
                == initial
                    .checked_sub(expected)
                    .ok_or("budget_counter_overflow")?
        } else {
            // Original v13 concurrency is carried by execution-slot facts.
            b.reserved == 0
        };
        if b.indeterminate != 0 || b.consumed != expected || !reserve_ok {
            return Err(format!(
                "business_logic_output_collection_original_budget_changed:{dimension}"
            ));
        }
    }
    let mut physical = Vec::new();
    for (sql,keys) in [
        ("SELECT rowid,* FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",vec![child.assignment_id.as_str(),child.run_id.as_str()]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE lease_attempt_id=?1 AND dimension<>'concurrency_batches' AND kind<>'release' ORDER BY rowid",vec![worker.id.as_str()]),
        ("SELECT rowid,id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id FROM agent_assignment_attempts WHERE id=?1",vec![worker.id.as_str()]),
        ("SELECT rowid,id,child_run_id,coordinator_run_id,role,lane,target_key,dedup_key,trigger_code,task_slice_json,evidence_revision,capability_lease_json,contract_keys_json,identity_handles_json,lease_epoch,fencing_token FROM agent_assignments WHERE id=?1",vec![child.assignment_id.as_str()]),
        ("SELECT rowid,root_run_id,contract_key,assignment_id,lease_epoch,fencing_token,acquired_at FROM agent_contract_owners WHERE assignment_id=?1",vec![child.assignment_id.as_str()]),
    ] {let frozen=NativeCoordinatorFrozenRows::capture(db,sql,&keys)?;physical.push(json!({"sql":frozen.sql,"keys":frozen.keys,"hash":frozen.hash}));}
    // A readonly worker must have no target request records. Capture that
    // exact empty set so a later injected record still changes the source.
    let empty_targets = NativeCoordinatorFrozenRows::capture_optional(
        db,
        "SELECT rowid,* FROM agent_evidence_nodes WHERE root_run_id=?1 AND created_by_run_id=?2 AND kind='request_record' ORDER BY rowid",
        &[actor.root_run_id.as_str(), child.run_id.as_str()],
    )?;
    physical.push(json!({"sql":empty_targets.sql,"keys":empty_targets.keys,"hash":empty_targets.hash}));
    let mut fact = json!({"schemaVersion":1,"purpose":"original_complete_readonly_business_logic_output_collection","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":worker.id,"originalInput":input_source.fact(),"modelRequest":request,"modelResponse":actual_response,"usage":received.usage.as_json(),"payload":payload,"wallTimeMs":clock,"originalPhysicalFacts":physical,"executionAllowed":false,"recoveryImplemented":false});
    let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("business_logic_output_original_birth_missing")?;
    if mode.bootstrap_dispatch().is_some_and(|p| matches!(p["schemaVersion"].as_i64(), Some(14 | 15))) {
        let dispatch = business_logic_dispatch_receipt::verify_original_on(db, actor, child)?;
        fact["originalDispatch"] = dispatch.fact().clone();
        fact["originalDispatchReceiptId"] = dispatch.receipt_id().into();
    }
    Ok(VerifiedBusinessLogicOutputCollection { fact })
}
