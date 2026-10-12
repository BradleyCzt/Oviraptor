// Pure original Mapper history. This is neither a live session nor a Root outcome.
struct OriginalMapperPreparedProjection {
    fact: JsonValue,
}
impl OriginalMapperPreparedProjection {
    fn fact(&self) -> &JsonValue {
        &self.fact
    }
}

fn native_mapper_prepared_history_read_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Option<OriginalMapperPreparedProjection>, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{budget, scheduler::ScheduledChild, specialist},
    };
    if db.is_autocommit() {
        return Err("mapper_prepared_history_snapshot_required".into());
    }
    budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("mapper_prepared_history_original_mode_missing")?;
    if mode.bootstrap_dispatch().is_none_or(|c| {
        c["schemaVersion"] != 15
            || c["mapperLoopRule"] != "original_frozen_mapper_two_round_one_lookup_v1"
    }) {
        return Err("mapper_prepared_history_original_protocol_missing".into());
    }
    // The grant/final readers below reverify signed original birth, not current configuration.
    let rows=db.prepare("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND (role='spa_api_mapper' OR json_type(task_slice_json,'$.mapperLoop') IS NOT NULL) ORDER BY id").map_err(|e|e.to_string())?
        .query_map([&actor.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 {
        return Err("mapper_prepared_history_original_mapper_conflict".into());
    }
    let child = ScheduledChild {
        assignment_id: rows[0].0.clone(),
        run_id: rows[0].1.clone(),
        role: AgentRole::SpaApiMapper,
    };
    if crate::agent_runtime::multi_agent::attempts::current(db, actor, &child.assignment_id)?.state == "expired" {
        return native_mapper_expired_history_read_on(db, actor, &child);
    }
    let Some(receipt) =
        specialist::mapper_loop::final_seal::read_final_original_on(db, actor, &child)?
    else {
        return Ok(None);
    };
    let output = specialist::mapper_loop::verified_original_output_on(db, actor, &child)?;
    // Reconstruct the exact original frame using the signed final delivery.
    // Generic mailbox::verified_completed_output requires live C and cannot be used here.
    let frame = NativeCoordinatorFrame {
        kind: "mapper-output",
        semantic: json!({"messageId":receipt.message_id(),"role":"spa_api_mapper","output":native_coordinator_mapper_semantic(output.text())?,"mapperLoopFinalReceiptId":receipt.receipt_id(),"mapperLoopFinalSource":receipt.fact()}),
        rows: NativeCoordinatorFrame::closed_rows(db, actor, &child)?,
        review_refs: None,
        review_meta: None,
    };
    let receipts=db.prepare("SELECT q.fact_json,p.fact_json FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.root_run_id=q.root_run_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request'").map_err(|e|e.to_string())?
        .query_map([&actor.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut publication = None;
    for (request, saved) in receipts {
        let request: JsonValue = serde_json::from_str(&request).map_err(|e| e.to_string())?;
        if request["request"]["basis"]["phase"] != "mapper-output"
            || request["request"]["basis"]["changedFact"] != frame.fact()
        {
            continue;
        }
        let saved: JsonValue = serde_json::from_str(&saved).map_err(|e| e.to_string())?;
        let sequence = saved["eventSequence"]
            .as_i64()
            .ok_or("mapper_prepared_history_publication_invalid")?;
        let original =
            budget::root::model::tick::Tick::project_publication(db, &actor.root_run_id, sequence)?;
        if original.record.get("localTools").is_some()
            || !original.record["summary"]["suggestions"]
                .as_array()
                .is_some_and(|s| s.iter().any(|v| v == "dispatch:web_executor"))
        {
            return Err("mapper_prepared_history_publication_invalid".into());
        }
        if publication.replace(sequence).is_some() {
            return Err("mapper_prepared_history_publication_conflict".into());
        }
    }
    let ids=db.prepare("SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND (role='web_executor' OR trigger_code='mapper_handoff_ready') UNION SELECT assignment_id FROM agent_runs WHERE root_run_id=?1 AND role='web_executor'").map_err(|e|e.to_string())?.query_map([&actor.root_run_id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut handoff = None;
    if !ids.is_empty() {
        if ids.len() != 1 {
            return Err("mapper_prepared_history_executor_inventory_conflict".into());
        }
        let (run,revision,task_raw,caps_raw):(String,i64,String,String)=db.query_row("SELECT child_run_id,evidence_revision,task_slice_json,capability_lease_json FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND role='web_executor' AND lane='target_touching' AND trigger_code='mapper_handoff_ready'",params![ids[0],actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"mapper_prepared_history_executor_original_missing")?;
        let task: JsonValue = serde_json::from_str(&task_raw).map_err(|e| e.to_string())?;
        let task_canonical = task.to_string();
        let caps: Vec<String> = serde_json::from_str(&caps_raw).map_err(|e| e.to_string())?;
        let event = publication.ok_or("mapper_prepared_history_executor_paid_handoff_missing")?;
        let paid =
            budget::root::model::tick::Tick::project_publication(db, &actor.root_run_id, event)?;
        if task_canonical != task_raw
            || revision <= 0
            || task["target"] != actor.target_key
            || task["objective"]
                != "在授权范围内执行冻结计划中的验证合同，所有目标访问都必须经过工具 Broker"
            || task["mapperAssignmentId"] != child.assignment_id
            || task["rootDecision"]["eventSequence"] != event
            || task["rootDecision"]["summaryHash"]
                != crate::agent_runtime::store::stable_hash(&paid.record["summary"].to_string())
            || task["rootDecision"]["frameHash"]
                != crate::agent_runtime::store::stable_hash(&frame.fact().to_string())
            || task["rootDecision"]["step"] != "dispatch:web_executor"
        {
            return Err("mapper_prepared_history_executor_paid_handoff_changed".into());
        }
        let executor = ScheduledChild {
            assignment_id: ids[0].clone(),
            run_id: run,
            role: AgentRole::WebExecutor,
        };
        let grant =
            budget::model::original_web_grant_inventory_on(db, actor, &executor.assignment_id)?;
        if task["rootDecision"]["rustPolicy"]["costBasis"]["reservedModelTokens"] != grant.tokens()
            || task["rootDecision"]["rustPolicy"]["costBasis"]["reservedModelRequests"]
                != grant.requests()
        {
            return Err("mapper_prepared_history_executor_original_budget_changed".into());
        }
        let mut original_caps=db.prepare("SELECT capability FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 ORDER BY capability").map_err(|e|e.to_string())?.query_map(params![executor.assignment_id,executor.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
        let mut expected_caps = caps.clone();
        expected_caps.sort();
        if original_caps != expected_caps {
            return Err("mapper_prepared_history_executor_original_capabilities_changed".into());
        }
        original_caps.dedup();
        if original_caps.len() != caps.len() {
            return Err("mapper_prepared_history_executor_original_capabilities_changed".into());
        }
        let payload_raw:String=db.query_row("SELECT payload_json FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2 AND kind='execution_assignment'",params![actor.root_run_id,executor.assignment_id],|r|r.get(0)).map_err(|_|"mapper_prepared_history_executor_original_message_missing")?;
        let payload: JsonValue = serde_json::from_str(&payload_raw).map_err(|e| e.to_string())?;
        let surface = if payload["publicSurface"].is_null() {
            JsonValue::Null
        } else {
            let surface_rows=db.prepare("SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND (role='external_surface' OR trigger_code='anonymous_entry_contract')").map_err(|e|e.to_string())?.query_map([&actor.root_run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
            if surface_rows.is_empty() {
                let absent = json!({"state":"not_scheduled","reason":"insufficient_budget_for_surface_executor_and_review","targetRequests":0});
                if payload["publicSurface"] != absent {
                    return Err("mapper_prepared_history_external_surface_source_required".into());
                }
                absent
            } else {
                if surface_rows.len() != 1 {
                    return Err("mapper_prepared_history_external_surface_source_conflict".into());
                }
                let surface_child = ScheduledChild {
                    assignment_id: surface_rows[0].0.clone(),
                    run_id: surface_rows[0].1.clone(),
                    role: AgentRole::ExternalSurface,
                };
                let source =
                    external_surface_output_receipt::read_original_on(db, actor, &surface_child)?
                        .ok_or("mapper_prepared_history_external_surface_source_required")?;
                source.fact()["collection"]["payload"].clone()
            }
        };
        let expected = json!({"target":actor.target_key,"mapperSummary":{"summary":output.text()},"capabilities":caps,"publicSurface":surface});
        let delivery =
            crate::agent_runtime::multi_agent::mailbox::read_original_execution_assignment_on(
                db, actor, &executor, revision, &expected,
            )?;
        handoff = Some(
            json!({"assignmentId":executor.assignment_id,"childRunId":executor.run_id,"revision":revision,"workerAttemptId":grant.worker_attempt_id(),"originalModelTokens":grant.tokens(),"originalModelRequests":grant.requests(),"delivery":delivery.fact(),"task":task}),
        );
    }
    let terminal:bool=db.query_row("SELECT status='terminal' FROM agent_runs WHERE id=?1 AND role='coordinator' AND scan_id=?2 AND attempt_number=?3 AND target_url=?4",params![actor.root_run_id,actor.scan_id,actor.attempt_number,actor.target_key],|r|r.get(0)).map_err(|e|e.to_string())?;
    Ok(Some(OriginalMapperPreparedProjection {
        fact: json!({"schemaVersion":1,"purpose":"original_mapper_prepared_history_readonly","stage":if handoff.is_some() {"OriginalHandoffIssued"} else if terminal {"RootTerminalHistory"} else {"MapperClosed"},"rootTerminal":terminal,"originalHandoff":handoff,"rootRunId":actor.root_run_id,"assignmentId":child.assignment_id,"childRunId":child.run_id,"mapperFinalReceiptId":receipt.receipt_id(),"mapperFinalSource":receipt.fact(),"payload":{"summary":output.text()},"usage":output.usage().as_json(),"mapperPublicationSequence":publication,"executionAllowed":false,"rootFinalOutcomeVerified":false}),
    }))
}

// Original expired delivery and its advisory publication are historical facts.
// Neither fact restores an Executor grant or asserts a final Root outcome.
fn native_mapper_expired_history_read_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<Option<OriginalMapperPreparedProjection>, String> {
    use crate::agent_runtime::multi_agent::{budget, specialist::mapper_loop};
    let Some(seal) = mapper_loop::expired_seal::read_original_on(db, actor, child)? else {
        return Ok(None);
    };
    let closed = mapper_loop::expired_saved::read_closed_original_on(db, actor, child)?;
    let frame = NativeCoordinatorFrame::lease_closed_mapper_loop_original_on(db, actor, child)?;
    let mut query = db.prepare("SELECT q.fact_json,p.fact_json FROM agent_root_tick_receipts q JOIN agent_root_tick_receipts p ON p.call_id=q.call_id AND p.root_run_id=q.root_run_id AND p.phase='publication' WHERE q.root_run_id=?1 AND q.phase='request'").map_err(|e|e.to_string())?;
    let receipts = query.query_map([&actor.root_run_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))
        .map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let mut publication = None;
    for (request, saved) in receipts {
        let request: JsonValue = serde_json::from_str(&request).map_err(|e|e.to_string())?;
        if request["request"]["basis"]["phase"] != "lease-expired"
            || request["request"]["basis"]["changedFact"] != frame.fact() { continue; }
        let saved: JsonValue = serde_json::from_str(&saved).map_err(|e|e.to_string())?;
        let sequence = saved["eventSequence"].as_i64().ok_or("mapper_expired_history_publication_invalid")?;
        let original = budget::root::model::tick::Tick::project_publication(db, &actor.root_run_id, sequence)?;
        if original.record.get("localTools").is_some()
            || !original.record["summary"]["suggestions"].as_array()
                .is_some_and(|items| !items.is_empty() && items.iter().all(|item| item == "assess:lease_expired"))
            || publication.replace(sequence).is_some()
        { return Err("mapper_expired_history_publication_invalid".into()); }
    }
    let terminal: bool = db.query_row("SELECT status='terminal' FROM agent_runs WHERE id=?1", [&actor.root_run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    Ok(Some(OriginalMapperPreparedProjection {
        fact: serde_json::json!({"stage":"original expired mapper delivered","assignmentId":child.assignment_id,
            "childRunId":child.run_id,"originalExpiredOutput":seal.fact(),"usage":closed.output().usage().as_json(),
            "supervisionEventSequence":publication,"rootTerminal":terminal,
            "rootFinalOutcomeVerified":false,"executionAllowed":false}),
    }))
}
