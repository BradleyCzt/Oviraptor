// Pure original Reviewer source. The caller has already verified the signed
// withdrawal, original birth, paid invoice, ten balances and closed child.
fn native_lease_reviewer_closed_frame(
    db: &rusqlite::Connection,
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    worker: &crate::agent_runtime::multi_agent::attempts::AssignmentAttempt,
    invoice: JsonValue,
) -> Result<NativeCoordinatorFrame, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{mailbox, specialist},
    };
    if db.is_autocommit()
        || child.role != AgentRole::EvidenceReviewer
        || worker.state != "expired"
        || worker.child_run_id != child.run_id
    {
        return Err("root_lease_reviewer_original_snapshot_required".into());
    }
    if !review_attempt_active(db, actor)? {
        return Err("root_lease_reviewer_attempt_not_active".into());
    }
    let (task, assignment_revision): (String, i64) = db.query_row(
        "SELECT task_slice_json,evidence_revision FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND role='evidence_reviewer' AND lane='review' AND trigger_code='candidate_ready' AND state='completed' AND lease_epoch=?4 AND fencing_token=?5",
        params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token],
        |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|_| "root_lease_reviewer_task_missing")?;
    let task: JsonValue =
        serde_json::from_str(&task).map_err(|_| "root_lease_reviewer_task_invalid")?;
    let candidate = task["candidateId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("root_lease_reviewer_task_invalid")?;
    let revision = task["candidateRevision"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or("root_lease_reviewer_task_invalid")?;
    if task != json!({"candidateId":candidate,"candidateRevision":revision})
        || assignment_revision != revision
    {
        return Err("root_lease_reviewer_task_invalid".into());
    }
    for (dimension, amount) in [
        ("model_input_tokens", 8000),
        ("model_cached_tokens", 8000),
        ("model_output_tokens", 8000),
        ("model_requests", 1),
    ] {
        let logical = format!("reserve:{}:{dimension}", child.assignment_id);
        let key = if worker.lease_epoch == 1 {
            logical
        } else {
            format!("worker:{}:{logical}", worker.id)
        };
        let count: i64 = db.query_row(
            "SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension=?4 AND kind='reserve' AND amount=?5 AND idempotency_key=?6 AND source_id=?7",
            params![actor.root_run_id,child.assignment_id,worker.id,dimension,amount,key,format!("assignment:{}",child.assignment_id)], |r| r.get(0),
        ).map_err(|e|e.to_string())?;
        if count != 1 {
            return Err("root_lease_reviewer_original_grant_conflict".into());
        }
    }
    let mut query = db.prepare(
        "SELECT q.id,q.candidate_json,q.evidence_revision,q.manifest_hash,d.verdict,d.reason_codes_json,d.missing_evidence_json,d.confidence FROM agent_review_requests q JOIN agent_review_decisions d ON d.id=q.decision_id AND d.root_run_id=q.root_run_id AND d.candidate_id=q.candidate_id AND d.candidate_revision=q.candidate_revision AND d.reviewer_run_id=q.reviewer_run_id AND d.verdict=q.status WHERE q.root_run_id=?1 AND q.assignment_id=?2 AND q.reviewer_run_id=?3 AND q.candidate_id=?4 AND q.candidate_revision=?5 AND q.lease_epoch=?6 AND q.fencing_token=?7 AND q.finished_at<>''",
    ).map_err(|e|e.to_string())?;
    let originals = query
        .query_map(
            params![
                actor.root_run_id,
                child.assignment_id,
                child.run_id,
                candidate,
                revision,
                actor.lease_epoch,
                actor.fencing_token
            ],
            |r| {
                Ok(NativeLeaseReviewerOriginal {
                    request: r.get(0)?,
                    candidate: r.get(1)?,
                    evidence_revision: r.get(2)?,
                    manifest: r.get(3)?,
                    verdict: r.get(4)?,
                    reasons: r.get(5)?,
                    missing: r.get(6)?,
                    confidence: r.get(7)?,
                })
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if originals.len() != 1 {
        return Err("root_lease_reviewer_original_decision_missing_or_conflicting".into());
    }
    let original = &originals[0];
    verify_review_snapshot(
        db,
        &actor.root_run_id,
        candidate,
        revision,
        &context.target_dir,
    )?;
    let frozen: JsonValue = serde_json::from_str(&original.candidate)
        .map_err(|_| "root_lease_reviewer_candidate_invalid")?;
    let receipt = specialist::received_for_local_delivery(db, actor, child)?;
    let decision = validated_review_decision(&receipt.text)?;
    let reasons: JsonValue = serde_json::from_str(&original.reasons)
        .map_err(|_| "root_lease_reviewer_decision_invalid")?;
    let missing: JsonValue = serde_json::from_str(&original.missing)
        .map_err(|_| "root_lease_reviewer_decision_invalid")?;
    if !receipt.rejection.is_empty()
        || receipt.usage.as_json() != invoice
        || decision.verdict != original.verdict
        || decision.reason_codes != reasons
        || decision.missing_evidence != missing
        || !original.confidence.is_finite()
        || (decision.confidence - original.confidence).abs() > f64::EPSILON
    {
        return Err("root_lease_reviewer_original_decision_conflict".into());
    }
    let request: String = db.query_row("SELECT request_json FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND state='received'",params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let request: JsonValue =
        serde_json::from_str(&request).map_err(|_| "root_lease_reviewer_request_invalid")?;
    let model_candidate = request
        .pointer("/messages/1/content")
        .and_then(JsonValue::as_str)
        .and_then(|s| serde_json::from_str::<JsonValue>(s).ok());
    let system = if frozen.get("gapFollowup").is_some() {
        GAP_FOLLOWUP_REVIEW_PROMPT
    } else {
        "你是独立 Evidence Reviewer。你没有网络、浏览器、上传或目标访问能力，只能审查冻结 CandidateBundle。输出严格 JSON：{\"verdict\":\"confirmed|rejected|insufficient_evidence\",\"reasonCodes\":[\"...\"],\"missingEvidence\":[\"...\"],\"confidence\":0.0,\"summary\":\"...\"}。没有可验证工具证据时不得 confirmed。"
    };
    if request["schemaVersion"] != 1
        || request["tools"] != json!([])
        || request["messages"].as_array().map(Vec::len) != Some(2)
        || request["messages"][0] != json!({"role":"system","content":system})
        || request["messages"][1]["role"] != "user"
        || model_candidate != Some(crate::agent_runtime::secrets::redact_json(&frozen))
        || request.get("humanDirectiveDispatch").is_some()
    {
        return Err("root_lease_reviewer_original_input_conflict".into());
    }
    validate_gap_assessment(&frozen, &decision)?;
    if frozen.get("gapFollowup").is_some() {
        verified_gap_review_receipt(db, &original.request)?;
    }
    let mut query = db
        .prepare("SELECT id FROM agent_messages WHERE assignment_id=?1 ORDER BY rowid")
        .map_err(|e| e.to_string())?;
    let ids = query
        .query_map([&child.assignment_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if ids.len() != 1 {
        return Err("root_lease_reviewer_original_ack_missing_or_conflicting".into());
    }
    let message_id = &ids[0];
    let valid: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_messages WHERE id=?1 AND run_id=?2 AND root_run_id=?2 AND assignment_id=?3 AND from_run_id=?4 AND to_run_id=?2 AND from_agent='evidence_reviewer' AND to_agent='coordinator' AND kind='review_decision' AND correlation_id=?5 AND evidence_revision=?6 AND dedup_key=?7 AND artifact_refs_json='[]' AND delivery_attempts=1 AND delivered_at<>'' AND acknowledged_at<>'')",
        params![message_id,actor.root_run_id,child.assignment_id,child.run_id,original.request,revision,format!("{}:review_decision:{}:{revision}",child.assignment_id,original.request)],|r|r.get(0),
    ).map_err(|e|e.to_string())?;
    let message = mailbox::verified_completed_output(db, actor, message_id)?;
    let expected = crate::agent_runtime::secrets::redact_json(
        &json!({"verdict":decision.verdict,"summary":decision.summary,"candidateId":candidate,"candidateRevision":revision}),
    );
    if !valid || message.payload != expected {
        return Err("root_lease_reviewer_original_ack_conflict".into());
    }
    native_lease_reviewer_events(
        db,
        actor,
        child,
        &original.request,
        message_id,
        candidate,
        &decision.verdict,
    )?;
    let mut rows = NativeCoordinatorFrame::closed_rows(db, actor, child)?;
    native_lease_reviewer_findings(
        db,
        actor,
        child,
        revision,
        &decision.verdict,
        &frozen,
        &mut rows,
    )?;
    let mut original_authorization = Vec::new();
    for finding in frozen["findingCandidates"].as_array().ok_or("root_lease_reviewer_candidate_invalid")? {
        if finding["record"]["kind"] == "authorization_cross_object_read"
            || finding["recordKey"].as_str().is_some_and(|key|key.starts_with("authorization-control:")) {
            let source=native_authorization_original_finding_on(db,actor,&context.target_dir,finding)?;
            rows.extend(source.rows);
            original_authorization.push(source.bounded_facts);
        }
    }
    let receipt_id: String = db.query_row("SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1 AND root_run_id=?2",params![worker.id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    for (sql,keys) in [
        ("SELECT rowid,* FROM agent_review_requests WHERE assignment_id=?1 ORDER BY rowid",vec![child.assignment_id.as_str()]),
        ("SELECT rowid,* FROM agent_review_decisions WHERE reviewer_run_id=?1 ORDER BY rowid",vec![child.run_id.as_str()]),
        ("SELECT rowid,* FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",vec![worker.id.as_str()]),
        ("SELECT rowid,* FROM agent_worker_expiry_issuance_seals WHERE receipt_id=?1 AND root_run_id=?2",vec![receipt_id.as_str(),actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_root_mode_birth_seals WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_root_mode_definitions WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 ORDER BY rowid",vec![child.assignment_id.as_str()]),
        ("SELECT rowid,* FROM agent_collaboration_events WHERE entity_id IN (?1,?2,?3,?4) ORDER BY sequence",vec![child.assignment_id.as_str(),child.run_id.as_str(),original.request.as_str(),message_id.as_str()]),
        ("SELECT name,type,tbl_name,sql FROM sqlite_master WHERE tbl_name IN ('agent_worker_expiry_receipts','agent_worker_expiry_issuance_seals','agent_root_mode_birth_seals','agent_root_mode_definitions','agent_review_requests','agent_review_decisions','agent_finding_candidates','sentinel_findings','agent_messages','agent_collaboration_events') ORDER BY name",vec![]),
    ] {rows.push(NativeCoordinatorFrozenRows::capture(db,sql,&keys)?);}
    for sql in [
        "SELECT rowid,* FROM agent_evidence_revisions WHERE root_run_id=?1 ORDER BY revision",
        "SELECT rowid,* FROM agent_evidence_nodes WHERE root_run_id=?1 ORDER BY id",
        "SELECT rowid,* FROM agent_evidence_edges WHERE root_run_id=?1 ORDER BY id",
        "SELECT rowid,* FROM agent_gap_review_receipts WHERE request_id=?1 ORDER BY rowid",
    ] {
        let key = if sql.contains("request_id") {
            original.request.as_str()
        } else {
            actor.root_run_id.as_str()
        };
        rows.push(NativeCoordinatorFrozenRows::capture_optional(
            db,
            sql,
            &[key],
        )?);
    }
    rows.push(NativeCoordinatorFrozenRows::capture_optional(
        db,
        "SELECT rowid,* FROM agent_gap_followups WHERE scan_id=?1 ORDER BY rowid",
        &[&actor.scan_id],
    )?);
    let mut frame = NativeCoordinatorFrame {
        kind: "lease-expired",
        semantic: json!({
            "rootRunId":actor.root_run_id,"role":"evidence_reviewer","workerAttemptId":worker.id,
            "receiptId":receipt_id,"assignmentId":child.assignment_id,"childRunId":child.run_id,
            "messageId":message_id,"originalInvoice":invoice,"candidateId":candidate,"candidateRevision":revision,
            "evidenceRevision":original.evidence_revision,"manifestHash":original.manifest,"originalDecision":{
                "verdict":decision.verdict,"reasonCodes":reasons,"missingEvidence":missing,"confidence":decision.confidence,"summary":decision.summary,"gapAssessment":decision.gap_assessment},
            "savedOutput":expected,"dispatchGranted":false,"targetRequestsGranted":0,
        }),
        rows,
        review_refs: None,
        review_meta: None,
    };
    if !original_authorization.is_empty() {
        frame.semantic["originalAuthorizationFindings"] = json!(original_authorization);
    }
    Ok(frame)
}

struct NativeLeaseReviewerOriginal {
    request: String,
    candidate: String,
    evidence_revision: i64,
    manifest: String,
    verdict: String,
    reasons: String,
    missing: String,
    confidence: f64,
}

fn native_lease_reviewer_events(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    request: &str,
    message: &str,
    candidate: &str,
    verdict: &str,
) -> Result<(), String> {
    crate::collaboration_events::client_delivery_schema::verify(db)?;
    let (events,created,consumed): (i64,i64,i64) = db.query_row(
        "SELECT count(*),sum(e.scan_id=?2 AND e.attempt_number=?3 AND e.entity_type='mailbox_message' AND e.event_type=e.entity_type AND e.payload_json=json_object('kind',m.kind,'deliveredAt','','acknowledgedAt','') AND e.created_at=m.created_at),sum(e.scan_id=?2 AND e.attempt_number=?3 AND e.entity_type='mailbox_message' AND e.event_type=e.entity_type AND e.payload_json=json_object('kind',m.kind,'deliveredAt',m.delivered_at,'acknowledgedAt',m.acknowledged_at) AND e.created_at>=m.acknowledged_at) FROM agent_collaboration_events e JOIN agent_messages m ON m.id=e.entity_id WHERE e.entity_id=?1",
        params![message,actor.scan_id,actor.attempt_number], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
    ).map_err(|e|e.to_string())?;
    if (events, created, consumed) != (2, 1, 1) {
        return Err("root_lease_reviewer_original_message_events_conflict".into());
    }
    let (events,kinds): (i64,i64) = db.query_row(
        "SELECT count(*),count(DISTINCT entity_type) FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2 AND entity_type=event_type AND ((entity_type='assignment' AND entity_id=?3 AND payload_json=json_object('role','evidence_reviewer','state','completed')) OR (entity_type='agent_run' AND entity_id=?4 AND payload_json=json_object('role','evidence_reviewer','status','terminal','terminalState','completed')) OR (entity_type='review_gate' AND entity_id=?5 AND payload_json=json_object('status',?6,'candidateId',?7)))",
        params![actor.scan_id,actor.attempt_number,child.assignment_id,child.run_id,request,verdict,candidate],|r|Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|e|e.to_string())?;
    if (events, kinds) != (3, 3) {
        return Err("root_lease_reviewer_original_closure_events_conflict".into());
    }
    Ok(())
}

fn native_lease_reviewer_findings(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    revision: i64,
    verdict: &str,
    frozen: &JsonValue,
    rows: &mut Vec<NativeCoordinatorFrozenRows>,
) -> Result<(), String> {
    let findings = frozen["findingCandidates"]
        .as_array()
        .ok_or("root_lease_reviewer_candidate_invalid")?;
    let status = match verdict {
        "confirmed" => "published",
        "rejected" => "rejected",
        "insufficient_evidence" => "insufficient_evidence",
        _ => return Err("root_lease_reviewer_decision_invalid".into()),
    };
    let count: i64 = db.query_row("SELECT count(*) FROM agent_finding_candidates WHERE root_run_id=?1 AND candidate_revision=?2 AND reviewer_run_id=?3",params![actor.root_run_id,revision,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count != findings.len() as i64 {
        return Err("root_lease_reviewer_original_findings_conflict".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for finding in findings {
        let id = finding["id"]
            .as_str()
            .ok_or("root_lease_reviewer_candidate_invalid")?;
        if !seen.insert(id) {
            return Err("root_lease_reviewer_candidate_invalid".into());
        }
        let (stored,published): (JsonValue,bool) = db.query_row(
            "SELECT c.stage,c.kind,c.record_key,c.title,c.severity,c.record_json,((?5='published' AND c.published_at<>'' AND (SELECT count(*) FROM sentinel_findings f WHERE f.scan_id=c.scan_id AND f.target_url=c.target_url AND f.stage=c.stage AND f.kind=c.kind AND f.record_key=c.record_key AND f.title=c.title AND f.severity=c.severity AND f.record_json=c.record_json)=1) OR (?5<>'published' AND c.published_at='')) FROM agent_finding_candidates c WHERE c.id=?1 AND c.root_run_id=?2 AND c.candidate_revision=?3 AND c.reviewer_run_id=?4 AND c.status=?5 AND c.scan_id=?6 AND c.target_url=?7",
            params![id,actor.root_run_id,revision,child.run_id,status,actor.scan_id,actor.target_key], |r| {
                let raw: String = r.get(5)?;
                let record: JsonValue = serde_json::from_str(&raw).map_err(|e|rusqlite::Error::FromSqlConversionFailure(5,rusqlite::types::Type::Text,Box::new(e)))?;
                Ok((json!({"id":id,"stage":r.get::<_,String>(0)?,"kind":r.get::<_,String>(1)?,"recordKey":r.get::<_,String>(2)?,"title":r.get::<_,String>(3)?,"severity":r.get::<_,String>(4)?,"record":record}),r.get(6)?))
            },
        ).map_err(|_| "root_lease_reviewer_original_findings_conflict")?;
        if stored != *finding || !published {
            return Err("root_lease_reviewer_original_findings_conflict".into());
        }
        rows.push(NativeCoordinatorFrozenRows::capture(
            db,
            "SELECT rowid,* FROM agent_finding_candidates WHERE id=?1",
            &[id],
        )?);
        rows.push(NativeCoordinatorFrozenRows::capture_optional(db,"SELECT f.rowid,f.* FROM sentinel_findings f JOIN agent_finding_candidates c ON f.scan_id=c.scan_id AND f.target_url=c.target_url AND f.stage=c.stage AND f.kind=c.kind AND f.record_key=c.record_key WHERE c.id=?1 ORDER BY f.rowid",&[id])?);
    }
    let revision = revision.to_string();
    rows.push(NativeCoordinatorFrozenRows::capture_optional(db,"SELECT rowid,* FROM agent_finding_candidates WHERE root_run_id=?1 AND candidate_revision=CAST(?2 AS INTEGER) AND reviewer_run_id=?3 ORDER BY id",&[&actor.root_run_id,&revision,&child.run_id])?);
    Ok(())
}
