// Read the persisted collaboration timeline in the caller's scan snapshot.
fn native_status_timeline(
    transaction: &rusqlite::Connection,
    scan_id: &str,
    attempt: i64,
    window: &TimelineWindow,
    closure_handoff: &JsonValue,
    administrative_closure: Option<&JsonValue>,
) -> Result<(Vec<JsonValue>, i64), String> {
    verify_request_review_timeline(transaction, scan_id, attempt)?;
    let plaintext = crate::agent_runtime::multi_agent::homogeneous::material_policy::plaintext(transaction, scan_id, attempt)?;
    let root_decisions = native_root_decision_timeline(transaction, scan_id, attempt)?;
    let after_sequence = window.after;
    let before_sequence = window.before;
    let mut timeline = root_decisions;
    let mut guidance=transaction.prepare("SELECT d.directive_id,d.updated_at,d.state,d.user_feedback_zh,d.raw_text_redacted,d.reason_zh,d.suggestion_zh,t.root_run_id,t.target_key,d.source_user_directive_id FROM agent_human_directive_gated d JOIN agent_homogeneous_tasks t ON t.task_id=d.parent_task_id WHERE t.scan_id=?1 AND t.attempt_number=?2").map_err(|e|e.to_string())?;
    timeline.extend(guidance.query_map(params![scan_id,attempt],|r| {
        let state:String=r.get(2)?;
        let feedback:String=r.get(3)?;
        let summary=if state=="awaiting_coordinator" {"用户引导已确认，等待主管处理。".into()} else if state=="applied" {"Worker 已读取主管批准的引导约束。".into()} else if matches!(state.as_str(),"relayed"|"rewritten") {format!("{feedback}；已投递，等待 Worker 读取。已发送的模型上下文保持原样。")} else {format!("{feedback} {} {}",r.get::<_,String>(5)?,r.get::<_,String>(6)?)};
        let mut item=json!({"id":r.get::<_,String>(0)?,"timestamp":r.get::<_,String>(1)?,"eventType":"homogeneous_guidance","fromRole":"coordinator","fromRunId":r.get::<_,String>(7)?,"toRole":"operator","toRunId":"","messageKind":"homogeneous_guidance","correlationId":"","assignmentId":"","evidenceRevision":0,"threadKey":"team","deliveryState":"persisted","ackState":"n/a","status":state,"summary":crate::agent_runtime::secrets::redact_text_with(&summary,None),"targetKey":r.get::<_,String>(8)?});
        let source:String=r.get(9)?;
        if !source.is_empty() {
            let bound:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM agent_user_directives WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 AND root_run_id=?4 AND source_draft_id<>'' AND confirmed_revision>0 AND confirmation_at<>'')",params![source,scan_id,attempt,r.get::<_,String>(7)?],|r|r.get(0))?;
            if !bound {return Err(rusqlite::Error::InvalidQuery);}
            item["sourceUserDirectiveId"]=json!(source);
        }
        Ok(item)
    }).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?);
    let mut reports = transaction.prepare("SELECT p.report_id,p.created_at,p.worker_id,p.slice_id,p.assignment_id,p.terminal_hint,p.output_json,p.consumed_tokens,p.consumed_requests,a.child_run_id,t.root_run_id,t.target_key FROM agent_worker_reports p JOIN agent_assignments a ON a.id=p.assignment_id JOIN agent_homogeneous_tasks t ON t.root_run_id=a.coordinator_run_id WHERE t.scan_id=?1 AND t.attempt_number=?2 AND (?3=0 OR EXISTS(SELECT 1 FROM agent_collaboration_events e WHERE e.event_type='worker_report' AND e.entity_id=p.report_id AND e.sequence>?3 AND (?4 IS NULL OR e.sequence<?4)))").map_err(|e|e.to_string())?;
    timeline.extend(reports.query_map(params![scan_id,attempt,after_sequence,before_sequence],|r| {
        let output:JsonValue=serde_json::from_str(&r.get::<_,String>(6)?).map_err(|e|rusqlite::Error::FromSqlConversionFailure(6,rusqlite::types::Type::Text,Box::new(e)))?;
        Ok(json!({"id":r.get::<_,String>(0)?,"timestamp":r.get::<_,String>(1)?,"eventType":"worker_report",
            "fromRole":"homogeneous_worker","fromRunId":r.get::<_,String>(9)?,"toRole":"coordinator","toRunId":r.get::<_,String>(10)?,
            "workerId":r.get::<_,String>(2)?,"sliceId":r.get::<_,String>(3)?,"assignmentId":r.get::<_,String>(4)?,
            "messageKind":"homogeneous_worker_report","correlationId":"","evidenceRevision":0,"threadKey":"team","deliveryState":"persisted","ackState":"n/a",
            "status":r.get::<_,String>(5)?,"summary":crate::agent_runtime::secrets::redact_text_with(output["summary_zh"].as_str().unwrap_or(""),None),
            "targetKey":r.get::<_,String>(11)?,"modelRequests":r.get::<_,i64>(8)?,"totalTokens":r.get::<_,i64>(7)?}))
    }).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?);
    // Retry changes the assignment's current carrier. Historical reports retain
    // the original attempt sender rather than borrowing the replacement run.
    for item in timeline.iter_mut().filter(|item| item["eventType"]=="worker_report") {
        let mut attempts=transaction.prepare("SELECT child_run_id,lease_epoch FROM agent_assignment_attempts WHERE assignment_id=?1").map_err(|e|e.to_string())?;
        let rows=attempts.query_map([item["assignmentId"].as_str().unwrap_or("")],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).map_err(|e|e.to_string())?;
        for row in rows {let (run,attempt)=row.map_err(|e|e.to_string())?;
            if crate::agent_runtime::multi_agent::homogeneous::store::report_id_for(item["assignmentId"].as_str().unwrap_or(""),attempt)==item["id"].as_str().unwrap_or("") {item["fromRunId"]=json!(run);break;}
        }
    }
    let mut decisions=transaction.prepare("SELECT d.audit_id,d.created_at,d.action,d.stop_reason,d.zero_intent_json,d.produced_intent,d.trigger_reason_code,t.root_run_id,t.target_key FROM agent_homogeneous_decision_audit d JOIN agent_homogeneous_tasks t ON t.task_id=d.parent_task_id WHERE t.scan_id=?1 AND t.attempt_number=?2 AND (?3=0 OR EXISTS(SELECT 1 FROM agent_collaboration_events e WHERE e.event_type='homogeneous_decision' AND e.entity_id=CAST(d.audit_id AS TEXT) AND e.sequence>?3 AND (?4 IS NULL OR e.sequence<?4)))").map_err(|e|e.to_string())?;
    timeline.extend(decisions.query_map(params![scan_id,attempt,after_sequence,before_sequence],|r|{
        let action:String=r.get(2)?; let stop:String=r.get(3)?;
        let zero:JsonValue=serde_json::from_str(&r.get::<_,String>(4)?).unwrap_or(JsonValue::Null);
        let summary=match action.as_str(){"decompose"=>"主管按目标分解了任务切片。".into(),"assign"=>"主管选定空闲 Worker 并分派切片。".into(),"accept"=>"主管核验汇报并接受切片。".into(),"reject_retry"=>"主管驳回汇报，切片等待重新分派。".into(),"stop"=>format!("主管收尾：{stop}。"),"relay_directive"=>"主管已处理用户引导并转发约束。".into(),"reject_directive"=>"主管拒绝了用户引导，原因见指令回执。".into(),_=>String::new()};
        Ok(json!({"id":r.get::<_,i64>(0)?.to_string(),"timestamp":r.get::<_,String>(1)?,"eventType":"homogeneous_decision",
            "fromRole":"coordinator","fromRunId":r.get::<_,String>(7)?,"toRole":"operator","toRunId":"","messageKind":action,"correlationId":"","assignmentId":"","evidenceRevision":0,
            "threadKey":"team","deliveryState":"persisted","ackState":"n/a","status":action,"summary":summary,"targetKey":r.get::<_,String>(8)?,
            "zeroIntent":zero,"producedIntent":r.get::<_,bool>(5)?,"triggerReasonCode":r.get::<_,String>(6)?}))
    }).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?);
    if let Some(receipt) = closure_handoff["successor"].as_object() {
        timeline.push(json!({"id":receipt["requestId"],"timestamp":receipt["createdAt"],
            "eventType":"closure_handoff","fromRole":"operator","fromRunId":"","toRole":"coordinator","toRunId":"",
            "messageKind":"closure_handoff","correlationId":"","assignmentId":"","evidenceRevision":0,
            "threadKey":"team","deliveryState":"persisted","ackState":"n/a","status":"draft_created",
            "summary":format!("已创建独立交接草稿 {}；本交接没有启动扫描，不结清原任务结果，也不重放旧请求。",receipt["scanId"].as_str().unwrap_or(""))}));
    }
    if let Some(receipt) = administrative_closure {
        timeline.push(json!({"id":receipt["closureId"],"timestamp":receipt["closedAt"],
            "eventType":"administrative_closure","fromRole":"operator","fromRunId":"","toRole":"coordinator","toRunId":"",
            "messageKind":"administrative_closure","correlationId":"","assignmentId":"","evidenceRevision":0,
            "threadKey":"team","deliveryState":"persisted","ackState":"n/a","status":"closed_unsettled",
            "summary":ADMINISTRATIVE_CLOSURE_CHECKPOINT}));
    }
    let mut request_review_query = transaction
        .prepare(
            "SELECT id,created_at,target_url,disposition FROM agent_request_reviews review
         WHERE scan_id=?1 AND attempt_number=?2 AND (?3 IS NULL OR EXISTS (
           SELECT 1 FROM agent_collaboration_events ev WHERE ev.scan_id=?1 AND ev.attempt_number=?2
             AND ev.event_type='request_review' AND ev.entity_id=review.id AND ev.sequence>?3
             AND (?4 IS NULL OR ev.sequence<?4))) ORDER BY rowid",
        )
        .map_err(|e| e.to_string())?;
    timeline.extend(request_review_query.query_map(params![scan_id,attempt,after_sequence,before_sequence], |row|Ok(json!({
        "id":row.get::<_,String>(0)?,"timestamp":row.get::<_,String>(1)?,"targetKey":row.get::<_,String>(2)?,
        "eventType":"request_review","fromRole":"operator","fromRunId":"","toRole":"coordinator","toRunId":"",
        "messageKind":"operator_attestation","correlationId":"","assignmentId":"","evidenceRevision":0,
        "threadKey":"team","deliveryState":"persisted","ackState":"n/a","status":row.get::<_,String>(3)?,
        "summary":"本地操作者已保存请求核对。仅为人工声明，不改变预算、机器回执或执行权限；详见目标请求核对记录。"
    }))).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?);
    let mut draft_query = transaction.prepare(
        "SELECT id,created_at,status,text_redacted,recipient_role,root_run_id,target_key,intent,coordinator_decision,\
         validation_result,reason_codes_json,confirmation_required,revision,draft_hash,side_effect_class,required_approvals_json,thread_key \
         FROM agent_directive_drafts WHERE scan_id=?1 AND attempt_number=?2 \
         AND (?3 IS NULL OR EXISTS (SELECT 1 FROM agent_collaboration_events ev \
           WHERE ev.scan_id=?1 AND ev.attempt_number=?2 AND ev.event_type='directive_draft' \
             AND ev.entity_id=agent_directive_drafts.id AND ev.sequence>?3
             AND (?4 IS NULL OR ev.sequence<?4))) ORDER BY created_at,id",
    ).map_err(|e| e.to_string())?;
    timeline.extend(draft_query.query_map(params![scan_id, attempt, after_sequence, before_sequence], |row| Ok(json!({
        "id": row.get::<_,String>(0)?, "timestamp": row.get::<_,String>(1)?, "eventType":"directive_draft",
        "fromRole":"operator", "fromRunId":"", "toRole":row.get::<_,String>(4)?, "toRunId":row.get::<_,String>(5)?,
        "messageKind":"human_directive_draft", "correlationId":"", "assignmentId":"", "evidenceRevision":0,
        "threadKey":row.get::<_,String>(16)?,
        "deliveryState":"persisted", "ackState":row.get::<_,String>(2)?, "status":row.get::<_,String>(2)?,
        "summary":row.get::<_,String>(3)?, "targetKey":row.get::<_,String>(6)?, "intent":row.get::<_,String>(7)?,
        "decision":row.get::<_,String>(8)?, "validationResult":row.get::<_,String>(9)?,
        "reasonCodes":serde_json::from_str::<JsonValue>(&row.get::<_,String>(10)?).unwrap_or_else(|_| json!([])),
        "confirmationRequired":row.get::<_,i64>(11)? != 0, "revision":row.get::<_,i64>(12)?,
        "draftHash":row.get::<_,String>(13)?, "sideEffectClass":row.get::<_,String>(14)?,
        "requiredApprovals":serde_json::from_str::<JsonValue>(&row.get::<_,String>(15)?).unwrap_or_else(|_| json!([])),
    }))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?);

    for item in timeline
        .iter_mut()
        .filter(|item| item["eventType"] == "directive_draft")
    {
        item["humanReview"] =
            match crate::agent_runtime::multi_agent::directive::human_review::receipt_for(
                transaction,
                item["id"].as_str().unwrap_or_default(),
            ) {
                Ok(receipt) => receipt.unwrap_or(JsonValue::Null),
                Err(error) if error == "directive_human_review_receipt_unverified" => {
                    item["deliveryState"] = json!("receipt_unverified");
                    if let Some(codes) = item["reasonCodes"].as_array_mut() {
                        codes.push(json!("human_review_receipt_unverified"));
                    }
                    JsonValue::Null
                }
                Err(error) => return Err(error),
            };
    }

    let mut boundary_query = transaction.prepare(
        "SELECT candidate.id,candidate.created_at,candidate.summary_redacted,candidate.target_key,\
                candidate.status,candidate.source_kind,COALESCE(draft.thread_key,'team') \
         FROM agent_host_boundary_candidates candidate \
         LEFT JOIN agent_directive_drafts draft ON draft.id=candidate.source_draft_id \
         WHERE candidate.scan_id=?1 AND candidate.attempt_number=?2 \
           AND (?3 IS NULL OR EXISTS (SELECT 1 FROM agent_collaboration_events ev \
             WHERE ev.scan_id=?1 AND ev.attempt_number=?2 AND ev.event_type='host_boundary_candidate' \
               AND ev.entity_id=candidate.id AND ev.sequence>?3 AND (?4 IS NULL OR ev.sequence<?4))) \
         ORDER BY candidate.created_at,candidate.id",
    ).map_err(|e| e.to_string())?;
    timeline.extend(boundary_query.query_map(params![scan_id, attempt, after_sequence, before_sequence], |row| Ok(json!({
        "id":row.get::<_,String>(0)?, "timestamp":row.get::<_,String>(1)?,
        "eventType":"host_boundary_candidate", "fromRole":"coordinator", "fromRunId":"",
        "toRole":"operator", "toRunId":"", "messageKind":"host_boundary_recorded_only",
        "correlationId":"", "assignmentId":"", "evidenceRevision":0,
        "threadKey":row.get::<_,String>(6)?, "deliveryState":"persisted", "ackState":"n/a",
        "status":row.get::<_,String>(4)?, "summary":row.get::<_,String>(2)?,
        "targetKey":row.get::<_,String>(3)?, "sourceKind":row.get::<_,String>(5)?,
        "intent":"host_boundary_candidate", "decision":"not_authorized",
        "reasonCodes":["host_boundary_not_supported"], "confirmationRequired":false,
    }))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?);

    let mut directive_query = transaction.prepare(
        "SELECT id,created_at,status,text_redacted,recipient_role,root_run_id,target_key,thread_key,\
         json_extract(payload_json,'$.modelDelivery.state'),rejection_code,\
         (SELECT receipt_json FROM agent_directive_queue_actions a WHERE a.directive_id=agent_user_directives.id), \
         (SELECT json_object('state',p.state,'assignmentId',p.assignment_id,'childRunId',p.child_run_id,\
           'summary',json_extract(p.response_json,'$.summary'),'errorCode',p.error_code,\
           'advisoryOnly',json('true'),'coverageVerified',json('false')) \
          FROM agent_directive_proposals p WHERE p.directive_id=agent_user_directives.id), \
         json_extract(payload_json,'$.taskClosure'),json_extract(payload_json,'$.localReconciliation'),json_extract(payload_json,'$.sourceGuidance') \
         FROM agent_user_directives \
         WHERE scan_id=?1 AND attempt_number=?2 \
         AND (?3 IS NULL OR EXISTS (SELECT 1 FROM agent_collaboration_events ev \
           WHERE ev.scan_id=?1 AND ev.attempt_number=?2 AND ev.event_type='user_directive' \
             AND ev.entity_id=agent_user_directives.id AND ev.sequence>?3
             AND (?4 IS NULL OR ev.sequence<?4))) ORDER BY created_at,id",
    ).map_err(|e| e.to_string())?;
    let mut directive_events=directive_query.query_map(params![scan_id, attempt, after_sequence, before_sequence], |row| Ok(json!({
        "id": row.get::<_,String>(0)?, "timestamp": row.get::<_,String>(1)?, "eventType":"user_directive",
        "fromRole":"operator", "fromRunId":"", "toRole":row.get::<_,String>(4)?, "toRunId":row.get::<_,String>(5)?,
        "messageKind":"human_directive", "correlationId":"", "assignmentId":"", "evidenceRevision":0,
        "threadKey":row.get::<_,String>(7)?,
        "deliveryState":row.get::<_,Option<String>>(8)?.unwrap_or_else(|| "persisted".into()),
        "ackState":row.get::<_,String>(2)?, "status":row.get::<_,String>(2)?,
        "reasonCodes":if row.get::<_,String>(9)?.is_empty() { Vec::<String>::new() } else { vec![row.get::<_,String>(9)?] },
        "queueAction":row.get::<_,Option<String>>(10)?.and_then(|value| serde_json::from_str::<JsonValue>(&value).ok()),
        "proposalAction":row.get::<_,Option<String>>(11)?.and_then(|value| serde_json::from_str::<JsonValue>(&value).ok()),
        "taskClosure":row.get::<_,Option<String>>(12)?.and_then(|value| serde_json::from_str::<JsonValue>(&value).ok()),
        "localReconciliation":row.get::<_,Option<String>>(13)?.and_then(|value| serde_json::from_str::<JsonValue>(&value).ok()),
        "sourceGuidance":row.get::<_,Option<String>>(14)?.and_then(|value| serde_json::from_str::<JsonValue>(&value).ok()),
        "summary":row.get::<_,String>(3)?, "targetKey":row.get::<_,String>(6)?,
    }))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    for item in &mut directive_events {
        match crate::agent_runtime::multi_agent::directive::ordered_execution::project(transaction, item["id"].as_str().unwrap_or_default()) {
            Ok(value) => item["orderedAssessmentExecution"] = value.unwrap_or(JsonValue::Null),
            Err(_) => { item["orderedAssessmentExecution"] = JsonValue::Null; item["deliveryState"] = json!("receipt_unverified");
                item["reasonCodes"].as_array_mut().expect("directive reason array").push(json!("ordered_receipt_unverified")); }
        }
        match crate::agent_runtime::multi_agent::directive::proposals::project_receipt(
            transaction,
            item["id"].as_str().unwrap_or_default(),
        ) {
            Ok(receipt) => item["proposalAction"] = receipt.unwrap_or(JsonValue::Null),
            Err(_) => {
                item["proposalAction"] = JsonValue::Null;
                item["localReconciliation"] = JsonValue::Null;
                item["deliveryState"] = json!("receipt_unverified");
                item["reasonCodes"]
                    .as_array_mut()
                    .expect("directive reason array")
                    .push(json!("proposal_receipt_unverified"));
            }
        }
        match crate::agent_runtime::multi_agent::directive::queue_actions::project_receipt(
            transaction,
            item["id"].as_str().unwrap_or_default(),
        ) {
            Ok(receipt) => item["queueAction"] = receipt.unwrap_or(JsonValue::Null),
            Err(_) => {
                item["queueAction"] = JsonValue::Null;
                item["deliveryState"] = json!("receipt_unverified");
                item["reasonCodes"]
                    .as_array_mut()
                    .expect("directive reason array")
                    .push(json!("queue_action_receipt_unverified"));
            }
        }
        match crate::agent_runtime::multi_agent::directive::source_guidance::project_delivery(
            transaction,
            item["id"].as_str().unwrap_or_default(),
        ) {
            Ok(Some(receipt)) => item["sourceGuidance"] = receipt,
            Ok(None) => {}
            Err(_) => {
                item["sourceGuidance"] = JsonValue::Null;
                item["deliveryState"] = json!("receipt_unverified");
                item["reasonCodes"]
                    .as_array_mut()
                    .expect("directive reason array")
                    .push(json!("source_guidance_receipt_unverified"));
            }
        }
    }
    timeline.extend(directive_events);

    let mut message_query = transaction.prepare(
        "SELECT m.id,m.created_at,m.from_agent,m.from_run_id,m.to_agent,m.to_run_id,m.kind,m.correlation_id,m.assignment_id,\
         m.evidence_revision,m.delivered_at,m.acknowledged_at,m.payload_json FROM agent_messages m JOIN agent_runs r ON r.id=m.root_run_id \
         WHERE r.scan_id=?1 AND r.attempt_number=?2 \
         AND (?3 IS NULL OR EXISTS (SELECT 1 FROM agent_collaboration_events ev \
           WHERE ev.scan_id=?1 AND ev.attempt_number=?2 AND ev.event_type='mailbox_message' \
             AND ev.entity_id=m.id AND ev.sequence>?3 AND (?4 IS NULL OR ev.sequence<?4))) ORDER BY m.created_at,m.id",
    ).map_err(|e| e.to_string())?;
    let mut message_events = message_query.query_map(params![scan_id, attempt, after_sequence, before_sequence], |row| {
        let payload: String = row.get(12)?;
        let value = serde_json::from_str::<JsonValue>(&payload).unwrap_or(JsonValue::Null);
        let summary = value.get("summary").or_else(|| value.get("verdict")).or_else(|| value.get("text"))
            .and_then(JsonValue::as_str).unwrap_or("typed mailbox message");
        let summary = if plaintext { summary.to_string() } else { crate::agent_runtime::secrets::redact_text_with(summary, None) };
        let delivered: String = row.get(10)?;
        let acknowledged: String = row.get(11)?;
        let kind: String = row.get(6)?;
        let summary = if kind == "homogeneous_guidance" {
            let text = format!("主管引导：{}。{}",value["constraint"].as_str().unwrap_or(""),if acknowledged.is_empty(){"已投递，等待 Worker 读取。"}else{"Worker 已读取约束。"});
            if plaintext { text } else { crate::agent_runtime::secrets::redact_text_with(&text, None) }
        } else {summary};
        let mut event = json!({
            "id":row.get::<_,String>(0)?, "timestamp":row.get::<_,String>(1)?, "eventType":"mailbox_message",
            "fromRole":row.get::<_,String>(2)?, "fromRunId":row.get::<_,String>(3)?, "toRole":row.get::<_,String>(4)?,
            "toRunId":row.get::<_,String>(5)?, "messageKind":kind, "correlationId":row.get::<_,String>(7)?,
            "assignmentId":row.get::<_,String>(8)?, "evidenceRevision":row.get::<_,i64>(9)?,
            "deliveryState":if delivered.is_empty(){"pending"}else{"delivered"},
            "ackState":if acknowledged.is_empty(){"pending"}else{"acknowledged"}, "status":"persisted", "summary":summary,
        });
        if let Some((key, detail)) = gap_timeline_projection(&kind, &value) {
            event[key] = detail;
        }
        Ok(event)
    }).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    for event in &mut message_events {
        if event["messageKind"] != "human_assessment_result" {
            continue;
        }
        let directive_id = event["correlationId"].as_str().unwrap_or_default();
        let proof = crate::agent_runtime::multi_agent::directive::proposals::project_receipt(
            transaction,
            directive_id,
        );
        let canonical: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_directive_proposals WHERE directive_id=?1 AND result_message_id=?2 AND state IN ('completed','failed'))",
            params![directive_id,event["id"].as_str().unwrap_or_default()],|r|r.get(0),
        ).map_err(|e|e.to_string())?;
        if !matches!(proof, Ok(Some(_))) || !canonical {
            event["summary"] =
                json!("评估结果消息未通过交付校验；请核对原始记录，未将其作为有效评估。");
            event["deliveryState"] = json!("receipt_unverified");
            event["reasonCodes"] = json!(["proposal_receipt_unverified"]);
        }
    }
    timeline.extend(message_events);

    let mut run_query = transaction.prepare(
        "SELECT id,created_at,role,status,COALESCE(parent_run_id,''),assignment_id,terminal_reason,terminal_code FROM agent_runs \
         WHERE scan_id=?1 AND attempt_number=?2 \
         AND (?3 IS NULL OR EXISTS (SELECT 1 FROM agent_collaboration_events ev \
           WHERE ev.scan_id=?1 AND ev.attempt_number=?2 AND ev.event_type='agent_run' \
             AND ev.entity_id=agent_runs.id AND ev.sequence>?3 AND (?4 IS NULL OR ev.sequence<?4))) ORDER BY created_at,id",
    ).map_err(|e| e.to_string())?;
    timeline.extend(run_query.query_map(params![scan_id, attempt, after_sequence, before_sequence], |row| {
        let reason: String = row.get(6)?;
        let code: String = row.get(7)?;
        Ok(json!({
            "id":row.get::<_,String>(0)?, "timestamp":row.get::<_,String>(1)?, "eventType":"agent_run",
            "fromRole":row.get::<_,String>(2)?, "fromRunId":row.get::<_,String>(0)?, "toRole":"coordinator",
            "toRunId":row.get::<_,String>(4)?, "messageKind":"run_state", "correlationId":"",
            "assignmentId":row.get::<_,String>(5)?, "evidenceRevision":0, "deliveryState":"local", "ackState":"n/a",
            "status":row.get::<_,String>(3)?, "summary":if reason.is_empty(){code}else{reason},
        }))
    }).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?);

    let mut assignment_query = transaction.prepare(
        "SELECT a.id,a.created_at,a.role,a.child_run_id,a.coordinator_run_id,a.trigger_code,a.evidence_revision,a.state \
         FROM agent_assignments a JOIN agent_runs r ON r.id=a.coordinator_run_id WHERE r.scan_id=?1 AND r.attempt_number=?2 \
         AND (?3 IS NULL OR EXISTS (SELECT 1 FROM agent_collaboration_events ev \
           WHERE ev.scan_id=?1 AND ev.attempt_number=?2 AND ev.event_type='assignment' \
             AND ev.entity_id=a.id AND ev.sequence>?3 AND (?4 IS NULL OR ev.sequence<?4))) ORDER BY a.created_at,a.id",
    ).map_err(|e| e.to_string())?;
    timeline.extend(assignment_query.query_map(params![scan_id, attempt, after_sequence, before_sequence], |row| Ok(json!({
        "id":row.get::<_,String>(0)?, "timestamp":row.get::<_,String>(1)?, "eventType":"assignment",
        "fromRole":"coordinator", "fromRunId":row.get::<_,String>(4)?, "toRole":row.get::<_,String>(2)?,
        "toRunId":row.get::<_,String>(3)?, "messageKind":"task_assignment", "correlationId":row.get::<_,String>(5)?,
        "assignmentId":row.get::<_,String>(0)?, "evidenceRevision":row.get::<_,i64>(6)?, "deliveryState":"leased",
        "ackState":"n/a", "status":row.get::<_,String>(7)?, "summary":row.get::<_,String>(5)?,
    }))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?);

    let mut review_query = transaction.prepare(
        "SELECT q.id,q.created_at,q.reviewer_run_id,q.root_run_id,q.assignment_id,q.candidate_id,q.candidate_revision,q.status,\
         COALESCE(d.reason_codes_json,'[]') FROM agent_review_requests q JOIN agent_runs r ON r.id=q.root_run_id \
         LEFT JOIN agent_review_decisions d ON d.id=q.decision_id WHERE r.scan_id=?1 AND r.attempt_number=?2 \
         AND (?3 IS NULL OR EXISTS (SELECT 1 FROM agent_collaboration_events ev \
           WHERE ev.scan_id=?1 AND ev.attempt_number=?2 AND ev.event_type='review_gate' \
             AND ev.entity_id=q.id AND ev.sequence>?3 AND (?4 IS NULL OR ev.sequence<?4))) ORDER BY q.created_at,q.id",
    ).map_err(|e| e.to_string())?;
    timeline.extend(review_query.query_map(params![scan_id, attempt, after_sequence, before_sequence], |row| Ok(json!({
        "id":row.get::<_,String>(0)?, "timestamp":row.get::<_,String>(1)?, "eventType":"review_gate",
        "fromRole":"evidence_reviewer", "fromRunId":row.get::<_,String>(2)?, "toRole":"coordinator",
        "toRunId":row.get::<_,String>(3)?, "messageKind":"review_decision", "correlationId":row.get::<_,String>(5)?,
        "assignmentId":row.get::<_,String>(4)?, "evidenceRevision":row.get::<_,i64>(6)?, "deliveryState":"persisted",
        "ackState":"n/a", "status":row.get::<_,String>(7)?, "summary":row.get::<_,String>(8)?,
    }))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?);
    let mut sequence_by_entity = HashMap::<(String, String), i64>::new();
    let mut sequence_query = transaction.prepare(
        "SELECT ev.event_type,ev.entity_id,MAX(ev.sequence) FROM agent_collaboration_events ev \
         WHERE ev.scan_id=?1 AND ev.attempt_number=?2 AND ev.sequence>?3
           AND (?4 IS NULL OR ev.sequence<?4)
           AND NOT EXISTS(SELECT 1 FROM agent_collaboration_events newer
             WHERE newer.scan_id=ev.scan_id AND newer.attempt_number=ev.attempt_number
               AND newer.event_type=ev.event_type AND newer.entity_id=ev.entity_id
               AND newer.sequence>ev.sequence) \
         GROUP BY ev.event_type,ev.entity_id",
    ).map_err(|error| error.to_string())?;
    for row in sequence_query
        .query_map(
            params![scan_id, attempt, after_sequence, before_sequence],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
    {
        let (event_type, entity_id, sequence) = row.map_err(|error| error.to_string())?;
        sequence_by_entity.insert((event_type, entity_id), sequence);
    }
    for item in &mut timeline {
        let run=item["fromRunId"].as_str().unwrap_or_default();
        let assignment=item["assignmentId"].as_str().unwrap_or_default();
        let homogeneous:Option<(String,String,String,String)>=transaction.query_row(
            "SELECT homogeneous_worker_id,slice_id,json_extract(task_slice_json,'$.scope_kind'),json_extract(task_slice_json,'$.scope_value') FROM agent_assignments WHERE scheduling_model='homogeneous_workers' AND (id=?1 OR child_run_id=?2) ORDER BY created_at DESC LIMIT 1",
            params![assignment,run],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(|e|e.to_string())?;
        if let Some((worker,slice,kind,value))=homogeneous {
            if item.get("workerId").is_none() {item["workerId"]=json!(worker);} if item.get("sliceId").is_none() {item["sliceId"]=json!(slice);}
            if item["fromRole"]=="deep_investigator" {item["fromRole"]=json!("homogeneous_worker");}
            if item["toRole"]=="deep_investigator" {item["toRole"]=json!("homogeneous_worker");}
            if item["messageKind"]=="homogeneous_slice_dispatch" || item["eventType"]=="assignment" {
                item["summary"]=json!(format!("主管分派只读切片：{kind} = {value}。"));
            }
        }
        let event_type = item
            .get("eventType")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        let entity_id = item
            .get("id")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        let sequence = sequence_by_entity
            .get(&(event_type.to_string(), entity_id.to_string()))
            .copied()
            .unwrap_or_default();
        if let Some(object) = item.as_object_mut() {
            object.insert("sequence".into(), json!(sequence));
        }
    }
    timeline.retain(|item| {
        let sequence = item
            .get("sequence")
            .and_then(JsonValue::as_i64)
            .unwrap_or_default();
        sequence > after_sequence && before_sequence.is_none_or(|before| sequence < before)
    });
    timeline.sort_by(|left, right| {
        left.get("timestamp")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .cmp(
                right
                    .get("timestamp")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default(),
            )
            .then_with(|| {
                left.get("sequence")
                    .and_then(JsonValue::as_i64)
                    .unwrap_or_default()
                    .cmp(
                        &right
                            .get("sequence")
                            .and_then(JsonValue::as_i64)
                            .unwrap_or_default(),
                    )
            })
            .then_with(|| {
                left.get("id")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .cmp(
                        right
                            .get("id")
                            .and_then(JsonValue::as_str)
                            .unwrap_or_default(),
                    )
            })
    });
    Ok((timeline, window.watermark))
}
