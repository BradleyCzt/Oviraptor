// Pure original Gap source. The shared lease reader has already checked the
// private withdrawal, paid invoice, ten balances and closed worker binding.
fn native_lease_deep_closed_frame(
    db:&rusqlite::Connection, context:&AgentRunContext,
    actor:&crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child:&crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    worker:&crate::agent_runtime::multi_agent::attempts::AssignmentAttempt,
    invoice:JsonValue,
) -> Result<NativeCoordinatorFrame,String> {
    use crate::agent_runtime::multi_agent::mailbox;
    let task:String=db.query_row("SELECT task_slice_json FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND role='deep_investigator' AND lane='read_only_analysis' AND trigger_code='reviewer_insufficient_evidence' AND state='completed'",params![child.assignment_id,child.run_id,actor.root_run_id],|row|row.get(0)).map_err(|_|"root_lease_deep_task_missing")?;
    let mut task:JsonValue=serde_json::from_str(&task).map_err(|_|"root_lease_deep_task_invalid")?;
    let decision=task.as_object_mut().and_then(|task|task.remove("rootDecision")).ok_or("root_lease_deep_original_dispatch_missing")?;
    let candidate=task["candidateId"].as_str().ok_or("root_lease_deep_task_invalid")?;
    let revision=task["revision"].as_i64().filter(|n|*n>0).ok_or("root_lease_deep_task_invalid")?;
    let missing=&task["missingEvidence"];
    if task!=json!({"candidateId":candidate,"revision":revision,"missingEvidence":missing}) {return Err("root_lease_deep_task_invalid".into());}
    gap_receipt_root_decision_valid(db,&actor.root_run_id,&decision,candidate,revision)?;
    let (candidate_json,missing,refs)=sealed_gap_review_candidate_in_transaction(db,&actor.root_run_id,candidate,revision,missing,&context.target_dir)?;
    let request:String=db.query_row("SELECT request_json FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",params![child.assignment_id,child.run_id],|row|row.get(0)).map_err(|e|e.to_string())?;
    let request:JsonValue=serde_json::from_str(&request).map_err(|_|"root_lease_deep_request_invalid")?;
    let input=request.pointer("/messages/1/content").and_then(JsonValue::as_str).and_then(|text|serde_json::from_str::<JsonValue>(text).ok());
    let expected_input=crate::agent_runtime::secrets::redact_json(&json!({"candidateId":candidate,"evidenceRevision":revision,"reviewerMissingEvidence":missing,"trustedFactRefs":refs,"frozenCandidate":candidate_json}));
    if request["schemaVersion"]!=1 || request["tools"]!=json!([]) || request["messages"].as_array().map(Vec::len)!=Some(2) || request["messages"][0]["role"]!="system" || request["messages"][1]["role"]!="user" || input!=Some(expected_input) || request.get("humanDirectiveDispatch").is_some() {return Err("root_lease_deep_original_input_conflict".into());}
    for (dimension,amount) in [("model_input_tokens",4000),("model_cached_tokens",4000),("model_output_tokens",4000),("model_requests",1)] {
        let logical=format!("reserve:{}:{dimension}",child.assignment_id);
        let key=if worker.lease_epoch==1 {logical} else {format!("worker:{}:{logical}",worker.id)};
        let count:i64=db.query_row("SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension=?4 AND kind='reserve' AND amount=?5 AND idempotency_key=?6 AND source_id=?7",params![actor.root_run_id,child.assignment_id,worker.id,dimension,amount,key,format!("assignment:{}",child.assignment_id)],|row|row.get(0)).map_err(|e|e.to_string())?;
        if count!=1 {return Err("root_lease_deep_original_grant_conflict".into());}
    }
    let proposal=NativeCoordinatorFrame::investigator(db,context,actor,candidate,revision,&missing)?;
    let reviewer=NativeCoordinatorFrame::reviewer(db,context,actor,candidate,revision,&missing)?;
    if crate::agent_runtime::store::stable_hash(&reviewer.fact().to_string())!=decision["frameHash"] {return Err("root_lease_deep_original_reviewer_changed".into());}
    let correlation=format!("review-gap:{candidate}:{revision}");
    let mut query=db.prepare("SELECT id,kind FROM agent_messages WHERE assignment_id=?1 AND root_run_id=?2 ORDER BY rowid").map_err(|e|e.to_string())?;
    let messages=query.query_map(params![child.assignment_id,actor.root_run_id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    if messages.len()!=2 {return Err("root_lease_deep_original_messages_conflict".into());}
    crate::collaboration_events::client_delivery_schema::verify(db)?;
    for (id,kind) in &messages {
        let (from,to,from_role,to_role)=match kind.as_str() {
            "gap_proposed"=>(child.run_id.as_str(),actor.root_run_id.as_str(),"deep_investigator","coordinator"),
            "proposal_assessed"=>(actor.root_run_id.as_str(),child.run_id.as_str(),"coordinator","deep_investigator"),
            _=>return Err("root_lease_deep_original_messages_conflict".into()),
        };
        let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_messages WHERE id=?1 AND run_id=?2 AND from_run_id=?3 AND to_run_id=?4 AND from_agent=?5 AND to_agent=?6 AND correlation_id=?7 AND evidence_revision=?8 AND dedup_key=?9 AND artifact_refs_json='[]' AND delivery_attempts=1 AND delivered_at<>'' AND acknowledged_at<>'')",params![id,actor.root_run_id,from,to,from_role,to_role,correlation,revision,format!("{}:{kind}:{correlation}:{revision}",child.assignment_id)],|row|row.get(0)).map_err(|e|e.to_string())?;
        if !valid {return Err("root_lease_deep_original_messages_conflict".into());}
        if kind=="gap_proposed" {
            let message=mailbox::verified_completed_output(db,actor,id)?;
            let parsed=&proposal.semantic["proposal"];
            let expected=crate::agent_runtime::secrets::redact_json(&json!({"candidateId":candidate,"evidenceRevision":revision,"summary":parsed["summary"],"nextStep":parsed["nextStep"],"missingEvidence":missing,"schemaVersion":3,"proposal":parsed}));
            if message.payload!=expected {return Err("root_lease_deep_original_proposal_conflict".into());}
        }
        let (events,created,consumed):(i64,i64,i64)=db.query_row("SELECT count(*),sum(e.scan_id=?2 AND e.attempt_number=?3 AND e.entity_type='mailbox_message' AND e.event_type=e.entity_type AND e.payload_json=json_object('kind',m.kind,'deliveredAt','','acknowledgedAt','') AND e.created_at=m.created_at),sum(e.scan_id=?2 AND e.attempt_number=?3 AND e.entity_type='mailbox_message' AND e.event_type=e.entity_type AND e.payload_json=json_object('kind',m.kind,'deliveredAt',m.delivered_at,'acknowledgedAt',m.acknowledged_at) AND e.created_at>=m.acknowledged_at) FROM agent_collaboration_events e JOIN agent_messages m ON m.id=e.entity_id WHERE e.entity_id=?1",params![id,actor.scan_id,actor.attempt_number],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(|e|e.to_string())?;
        if (events,created,consumed)!=(2,1,1) {return Err("root_lease_deep_original_message_events_conflict".into());}
    }
    let (closures,entities):(i64,i64)=db.query_row("SELECT count(*),count(DISTINCT entity_type) FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2 AND event_type=entity_type AND ((entity_type='assignment' AND entity_id=?3 AND payload_json=json_object('role','deep_investigator','state','completed')) OR (entity_type='agent_run' AND entity_id=?4 AND payload_json=json_object('role','deep_investigator','status','terminal','terminalState','completed')))",params![actor.scan_id,actor.attempt_number,child.assignment_id,child.run_id],|row|Ok((row.get(0)?,row.get(1)?))).map_err(|e|e.to_string())?;
    if (closures,entities)!=(2,2) {return Err("root_lease_deep_original_closure_events_conflict".into());}
    let receipt_id:String=db.query_row("SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1 AND root_run_id=?2",params![worker.id,actor.root_run_id],|row|row.get(0)).map_err(|e|e.to_string())?;
    let original_proposal=proposal.fact();
    let mut rows=proposal.rows;
    rows.extend(reviewer.rows);
    for (sql,keys) in [
        ("SELECT rowid,* FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",vec![worker.id.as_str()]),
        ("SELECT rowid,* FROM agent_worker_expiry_issuance_seals WHERE receipt_id=?1 AND root_run_id=?2",vec![receipt_id.as_str(),actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_root_mode_birth_seals WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_root_mode_definitions WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 ORDER BY rowid",vec![child.assignment_id.as_str()]),
        ("SELECT rowid,* FROM agent_collaboration_events WHERE entity_id IN (?1,?2) ORDER BY sequence",vec![child.assignment_id.as_str(),child.run_id.as_str()]),
    ] { rows.push(NativeCoordinatorFrozenRows::capture(db,sql,&keys)?); }
    for (id,_) in &messages {rows.push(NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM agent_collaboration_events WHERE entity_id=?1 ORDER BY sequence",&[id.as_str()])?);}
    Ok(NativeCoordinatorFrame {kind:"lease-expired",semantic:json!({"rootRunId":actor.root_run_id,"role":"deep_investigator","workerAttemptId":worker.id,"receiptId":receipt_id,"assignmentId":child.assignment_id,"childRunId":child.run_id,"originalInvoice":invoice,"originalProposal":original_proposal,"dispatchGranted":false,"targetRequestsGranted":0}),rows,review_refs:None,review_meta:None})
}
