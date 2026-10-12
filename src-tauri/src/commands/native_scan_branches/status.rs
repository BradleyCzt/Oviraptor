fn native_scan_status_after(
    connection: &rusqlite::Connection,
    scan_id: &str,
    after_sequence: Option<i64>,
) -> Result<JsonValue, String> {
    native_scan_status_for_attempt(connection, scan_id, after_sequence, None)
}

fn native_scan_status_for_attempt(
    connection: &rusqlite::Connection,
    scan_id: &str,
    after_sequence: Option<i64>,
    expected_attempt_number: Option<i64>,
) -> Result<JsonValue, String> {
    if expected_attempt_number.is_some_and(|attempt| attempt <= 0) {
        return Err("无效的任务轮次".into());
    }
    // One read snapshot: scan attempt and its branch rows cannot come from
    // different retries when the UI polls during a restart.
    let transaction = connection.unchecked_transaction().map_err(|e| e.to_string())?;
    let (attempt, status): (i64, String) = transaction.query_row(
        "SELECT attempt_count,status FROM sentinel_scans WHERE id=?1
         AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",
        [scan_id], |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => "任务不存在".to_string(),
        other => format!("无法读取任务状态：{other}"),
    })?;
    // A cursor is scoped to its attempt. A retry may already have started by
    // the time a polling IPC reads this snapshot; in that case send a full
    // projection for the new attempt, never a delta based on the old cursor.
    let after_sequence = after_sequence.filter(|_| {
        expected_attempt_number.is_none_or(|expected| expected == attempt)
    });
    // A later attempt must not silently hide an unsettled saved response from
    // the operator. This is only a debt indicator, not a verified receipt or
    // permission to settle against the current attempt's lease/budget.
    let historical_pending_receipts: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_user_directives d \
         JOIN agent_directive_proposals p ON p.directive_id=d.id \
         WHERE d.scan_id=?1 AND d.attempt_number<?2 AND d.status='deferred' \
           AND d.rejection_code='directive_task_ended_receipt_pending' \
           AND p.state='received' AND p.result_message_id=''",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    // Identity only. A row being listed is not proof that its frozen response
    // can be settled; the explicit historical command revalidates every link
    // under an IMMEDIATE transaction before writing to the original ledger.
    let mut historical_query = transaction.prepare(
        "SELECT d.id,d.attempt_number,d.created_at FROM agent_user_directives d \
         JOIN agent_directive_proposals p ON p.directive_id=d.id \
         WHERE d.scan_id=?1 AND d.attempt_number<?2 AND d.status='deferred' \
           AND d.rejection_code='directive_task_ended_receipt_pending' \
           AND p.state='received' AND p.result_message_id='' \
         ORDER BY d.attempt_number DESC,d.created_at DESC,d.id DESC LIMIT 50",
    ).map_err(|e| e.to_string())?;
    let historical_receipts = historical_query.query_map(params![scan_id, attempt], |row| Ok(json!({
        "directiveId":row.get::<_, String>(0)?,
        "attemptNumber":row.get::<_, i64>(1)?,
        "createdAt":row.get::<_, String>(2)?,
        "verified":false,
    }))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    let mut statement = transaction.prepare(
        "SELECT b.branch,b.status,b.checkpoint,b.report_json,b.updated_at,
         CASE WHEN d.scan_id IS NULL THEN 'legacy_unknown'
              WHEN d.claim_id='' AND d.claimed_at='' THEN 'never_claimed'
              WHEN d.claim_id<>'' AND d.claimed_at<>'' THEN 'claimed'
              ELSE 'invalid_receipt' END,d.claimed_at
         FROM native_scan_branches b LEFT JOIN native_branch_dispatches d
         ON d.scan_id=b.scan_id AND d.attempt_number=b.attempt_number AND d.branch=b.branch
         WHERE b.scan_id=?1 AND b.attempt_number=?2 ORDER BY b.branch",
    ).map_err(|e| e.to_string())?;
    let manual_web_recovery = web_recovery_available_in(&transaction,scan_id,attempt);
    let manual_web_closure = web_closure_available_in(&transaction,scan_id,attempt);
    let administrative_closure = verified_administrative_closure(&transaction,scan_id)?;
    let closure_handoff = closure_handoff_relations(&transaction,scan_id)?;
    let manual_administrative_closure = administrative_closure_available(&transaction,scan_id,attempt)?;
    let branch_rows = statement.query_map(params![scan_id,attempt], |r| Ok((
        r.get::<_,String>(0)?, r.get::<_,String>(1)?, r.get::<_,String>(2)?,
        r.get::<_,String>(3)?, r.get::<_,String>(4)?, r.get::<_,String>(5)?,
        r.get::<_,Option<String>>(6)?,
    ))).map_err(|e| e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
    let branches: Vec<JsonValue> = branch_rows.into_iter().map(|(branch,status,checkpoint,raw,updated_at,dispatch_state,claimed_at)| {
        let report = serde_json::from_str::<JsonValue>(&raw)
            .map_err(|error| format!("无法读取分支报告 {branch}：{error}"))?;
        Ok(json!({"branch":branch,"status":status,"checkpoint":checkpoint,"report":report,
            "updatedAt":updated_at,
            "dispatch":{"state":dispatch_state,"claimedAt":claimed_at,
                "automaticReplayAllowed":false,
                "manualRecoveryAvailable":manual_web_recovery && branch == "web",
                "manualClosureAvailable":manual_web_closure && branch == "web"}}))
    }).collect::<Result<_, String>>()?;
    let mut receipts = transaction.prepare(
        "SELECT receipt_id,purpose,container_name,cleanup_status,detail FROM analyzer_container_receipts
         WHERE scan_id=?1 AND cleanup_status<>'confirmed' ORDER BY created_at LIMIT 50",
    ).map_err(|e| e.to_string())?;
    let unresolved: Vec<JsonValue> = receipts.query_map([scan_id], |r| Ok(json!({
        "receiptId":r.get::<_,String>(0)?,"purpose":r.get::<_,String>(1)?,
        "containerName":r.get::<_,String>(2)?,"status":r.get::<_,String>(3)?,"detail":r.get::<_,String>(4)?
    }))).map_err(|e| e.to_string())?.collect::<Result<_,_>>().map_err(|e| e.to_string())?;
    // Web scheduling metadata shares the plan table, but is not a source
    // analyzer freeze. Decode source plans only for source task types.
    let scan_type:String=transaction.query_row("SELECT scan_type FROM sentinel_scans WHERE id=?1",[scan_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let source_plan = if scan_type=="web" {None} else {crate::native_pipeline::NativeSourcePlan::load(&transaction, scan_id, attempt)?};
    let source_gaps = source_plan.map(|plan| plan.gaps).unwrap_or_default();
    let mut targets_query = transaction.prepare(
        "SELECT url,status,substr(routing_reason,1,500) FROM sentinel_targets WHERE scan_id=?1 ORDER BY id",
    ).map_err(|e| e.to_string())?;
    let targets: Vec<JsonValue> = targets_query.query_map([scan_id], |row| Ok(json!({
        "url": row.get::<_, String>(0)?,
        "status": row.get::<_, String>(1)?,
        "detail": row.get::<_, String>(2)?,
    }))).map_err(|e| e.to_string())?.collect::<Result<_,_>>().map_err(|e| e.to_string())?;
    let mut stop_diagnostic = native_stop_diagnostic(&status, &branches, &targets, &source_gaps, &unresolved);
    if administrative_closure.is_some() {
        stop_diagnostic["code"] = json!("administratively_closed_unsettled");
        stop_diagnostic["nextAction"] = json!("create_independent_task");
        stop_diagnostic["automaticResumeAllowed"] = json!(false);
    }
    let (mut requests, mut tokens): (i64, i64) = transaction.query_row(
        "SELECT llm_requests,total_tokens FROM sentinel_scans WHERE id=?1",
        [scan_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|error| format!("无法读取任务模型用量：{error}"))?;
    if crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(&transaction,scan_id,attempt)? {
        // Cumulative scan counters do not receive these independent tool-free
        // invoices. Display the original ledger without inventing or recharging.
        let mut roots=transaction.prepare("SELECT root_run_id FROM agent_homogeneous_tasks WHERE scan_id=?1 ORDER BY attempt_number,root_run_id").map_err(|e|e.to_string())?;
        let ids=roots.query_map([scan_id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
        requests=0;tokens=0;
        for root in ids {
            use crate::agent_runtime::multi_agent::budget;
            budget::root::RootOwner::load_original(&transaction,&root)?;
            requests=requests.checked_add(budget::balance(&transaction,&root,None,"model_requests")?.consumed).ok_or("budget_amount_overflow")?;
            for dim in ["model_input_tokens","model_output_tokens"] {tokens=tokens.checked_add(budget::balance(&transaction,&root,None,dim)?.consumed).ok_or("budget_amount_overflow")?;}
        }
    }
    let coordinator_lease_valid: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_coordinator_leases WHERE scan_id=?1 AND attempt_number=?2 \
         AND lease_expires_at>datetime('now','localtime'))",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|error| format!("无法读取 Coordinator 租约状态：{error}"))?;
    let child_runs_started: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND role<>'coordinator' \
         AND (started_at<>'' OR status IN ('running','terminal')))",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|error| format!("无法读取子智能体运行状态：{error}"))?;
    let assignments_running: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.coordinator_run_id \
         WHERE r.scan_id=?1 AND r.attempt_number=?2 AND a.state IN ('leased','running','waiting_review'))",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|error| format!("无法读取 assignment 运行状态：{error}"))?;
    let mailbox_consumer_active: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_messages m JOIN agent_runs r ON r.id=m.root_run_id \
         WHERE r.scan_id=?1 AND r.attempt_number=?2 AND m.delivered_at<>'' AND m.acknowledged_at<>'')",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|error| format!("无法读取协作消息消费状态：{error}"))?;
    let independent_reviewer_run_id: String = transaction.query_row(
        "SELECT id FROM agent_runs WHERE scan_id=?1 AND attempt_number=?2 AND role='evidence_reviewer' \
         ORDER BY created_at DESC,id DESC LIMIT 1",
        params![scan_id, attempt], |row| row.get(0),
    ).optional().map_err(|error| format!("无法读取独立 Reviewer：{error}"))?.unwrap_or_default();
    let finding_candidate_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM agent_finding_candidates fc JOIN agent_runs c ON c.id=fc.root_run_id \
         WHERE c.scan_id=?1 AND c.attempt_number=?2 AND c.role='coordinator'",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|error| format!("无法读取漏洞候选数量：{error}"))?;
    // A legitimate rejection closes the review work without publishing a
    // vulnerability. Every candidate must be settled by the latest frozen
    // request, a completed Reviewer child and an acknowledged decision.
    let review_gate_satisfied: bool = finding_candidate_count > 0 && transaction.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM agent_finding_candidates fc
           JOIN agent_runs c ON c.id=fc.root_run_id
           WHERE c.scan_id=?1 AND c.attempt_number=?2 AND c.role='coordinator'
             AND NOT EXISTS(SELECT 1 FROM agent_review_requests q
               JOIN agent_review_decisions d ON d.id=q.decision_id
               JOIN agent_runs v ON v.id=q.reviewer_run_id
               JOIN agent_assignments a ON a.id=q.assignment_id
               JOIN agent_messages m ON m.root_run_id=q.root_run_id
                 AND m.from_run_id=v.id AND m.to_run_id=c.id
                 AND m.assignment_id=a.id AND m.correlation_id=q.id
                 AND m.evidence_revision=q.candidate_revision AND m.kind='review_decision'
                 AND m.delivered_at<>'' AND m.acknowledged_at<>''
               WHERE q.root_run_id=c.id AND q.candidate_revision=fc.candidate_revision
                 AND q.reviewer_run_id=fc.reviewer_run_id
                 AND q.status IN ('confirmed','rejected') AND q.status=d.verdict
                 AND d.root_run_id=c.id AND d.reviewer_run_id=v.id
                 AND d.candidate_id=q.candidate_id AND d.candidate_revision=q.candidate_revision
                 AND v.root_run_id=c.id AND v.parent_run_id=c.id
                 AND v.role='evidence_reviewer' AND v.lane='review'
                 AND v.status='terminal' AND v.terminal_state='completed'
                 AND v.assignment_id=a.id AND a.coordinator_run_id=c.id
                 AND a.child_run_id=v.id AND a.state='completed'
                 AND ((q.status='confirmed' AND fc.status='published')
                   OR (q.status='rejected' AND fc.status='rejected'))
                 AND EXISTS(SELECT 1 FROM json_each(CASE WHEN json_valid(q.candidate_json)
                   THEN q.candidate_json ELSE '{}' END,'$.findingCandidates') item
                   WHERE json_extract(item.value,'$.id')=fc.id)
                 AND NOT EXISTS(SELECT 1 FROM agent_review_requests newer
                   WHERE newer.root_run_id=c.id AND newer.candidate_id=q.candidate_id
                     AND newer.candidate_revision>q.candidate_revision)))",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|error| format!("无法核验 Reviewer 审查状态：{error}"))?;
    let review_status = if finding_candidate_count == 0 {
        "not_applicable".to_string()
    } else if review_gate_satisfied {
        let rejected: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_finding_candidates fc JOIN agent_runs c ON c.id=fc.root_run_id
             WHERE c.scan_id=?1 AND c.attempt_number=?2 AND fc.status='rejected')",
            params![scan_id, attempt], |row| row.get(0),
        ).map_err(|error| format!("无法读取 Reviewer 拒绝状态：{error}"))?;
        if rejected { "rejected" } else { "confirmed" }.to_string()
    } else {
        transaction.query_row(
            "SELECT q.status FROM agent_review_requests q JOIN agent_runs c ON c.id=q.root_run_id
             WHERE c.scan_id=?1 AND c.attempt_number=?2 ORDER BY q.candidate_revision DESC,q.created_at DESC LIMIT 1",
            params![scan_id, attempt], |row| row.get::<_, String>(0),
        ).optional().map_err(|error| format!("无法读取 Reviewer 最新审查状态：{error}"))?
            .unwrap_or_else(|| "pending".to_string())
    };
    let scheduler_active: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.coordinator_run_id \
         WHERE r.scan_id=?1 AND r.attempt_number=?2)",
        params![scan_id, attempt], |row| row.get(0),
    ).map_err(|error| format!("无法读取调度 assignment 状态：{error}"))?;
    // Readiness is a provenance check, not a count of loosely related rows. Every
    // fact below must belong to the same Coordinator root and target. This prevents
    // a hand-written child row, mailbox message or stale review from making the UI
    // advertise a working multi-agent runtime.
    let orchestration_integrity_ready: bool = transaction.query_row(
        "SELECT EXISTS(
           SELECT 1 FROM agent_runs c
           JOIN agent_coordinator_leases l ON l.root_run_id=c.id AND l.scan_id=c.scan_id
             AND l.attempt_number=c.attempt_number AND l.target_key=c.target_url
           WHERE c.scan_id=?1 AND c.attempt_number=?2 AND c.role='coordinator'
             AND c.root_run_id=c.id AND c.orchestration_policy='multi'
             AND c.lane='read_only_analysis' AND l.lease_expires_at>datetime('now','localtime')
             AND NOT EXISTS(SELECT 1 FROM tool_invocations ti WHERE ti.run_id=c.id)
             AND NOT EXISTS(
               SELECT 1 FROM json_each(CASE WHEN json_valid(c.capability_lease_json)
                 THEN c.capability_lease_json ELSE '[]' END)
               WHERE value NOT IN ('coordination.control','evidence.read','mailbox.read','mailbox.write')
             )
             AND (SELECT COUNT(DISTINCT r.id) FROM agent_runs r
                  JOIN agent_assignments a ON a.child_run_id=r.id AND a.coordinator_run_id=c.id
                  WHERE r.parent_run_id=c.id AND r.root_run_id=c.id
                    AND r.assignment_id=a.id AND r.orchestration_policy='multi'
                    AND r.role IN ('spa_api_mapper','web_executor','evidence_reviewer'))=
                    CASE WHEN EXISTS(SELECT 1 FROM agent_finding_candidates fc WHERE fc.root_run_id=c.id)
                      THEN 3 ELSE 2 END
             AND EXISTS(SELECT 1 FROM agent_runs m WHERE m.root_run_id=c.id
                  AND m.parent_run_id=c.id AND m.role='spa_api_mapper' AND m.lane='read_only_analysis'
                  AND EXISTS(SELECT 1 FROM agent_events e WHERE e.run_id=m.id AND e.event_type='model_round_completed')
                  AND EXISTS(SELECT 1 FROM agent_snapshots s WHERE s.run_id=m.id)
                  AND NOT EXISTS(SELECT 1 FROM tool_invocations ti WHERE ti.run_id=m.id))
             AND EXISTS(SELECT 1 FROM agent_runs x WHERE x.root_run_id=c.id
                  AND x.parent_run_id=c.id AND x.role='web_executor' AND x.lane='target_touching'
                  AND NOT EXISTS(SELECT 1 FROM agent_runs old WHERE old.scan_id=c.scan_id
                    AND old.attempt_number=c.attempt_number AND old.target_url=c.target_url
                    AND old.role='deep_investigator' AND old.lane='target_touching')
                  AND NOT EXISTS(SELECT 1 FROM agent_assignments old
                    JOIN agent_runs owner ON owner.id=old.coordinator_run_id
                    WHERE owner.scan_id=c.scan_id AND owner.attempt_number=c.attempt_number
                      AND owner.target_url=c.target_url AND old.role='deep_investigator'
                      AND old.lane='target_touching')
                  AND EXISTS(SELECT 1 FROM agent_events e WHERE e.run_id=x.id AND e.event_type='model_round_completed')
                  AND EXISTS(SELECT 1 FROM agent_snapshots s WHERE s.run_id=x.id)
                  AND EXISTS(SELECT 1 FROM tool_invocations ti WHERE ti.run_id=x.id)
                  AND EXISTS(SELECT 1 FROM agent_messages ma WHERE ma.root_run_id=c.id
                    AND ma.to_run_id=x.id AND ma.assignment_id=x.assignment_id
                    AND ma.kind='execution_assignment' AND ma.delivered_at<>'' AND ma.acknowledged_at<>'')
                  AND EXISTS(SELECT 1 FROM agent_messages mr WHERE mr.root_run_id=c.id
                    AND mr.from_run_id=x.id AND mr.assignment_id=x.assignment_id
                    AND mr.kind='execution_result' AND mr.delivered_at<>'' AND mr.acknowledged_at<>''))
             AND ((NOT EXISTS(SELECT 1 FROM agent_finding_candidates fc WHERE fc.root_run_id=c.id)
                  AND NOT EXISTS(SELECT 1 FROM agent_review_requests q WHERE q.root_run_id=c.id)
                  AND NOT EXISTS(SELECT 1 FROM agent_runs v WHERE v.root_run_id=c.id AND v.role='evidence_reviewer')
                  AND c.status='terminal')
               OR (?3 AND EXISTS(SELECT 1 FROM agent_finding_candidates fc WHERE fc.root_run_id=c.id)
                 AND EXISTS(SELECT 1 FROM agent_runs v WHERE v.root_run_id=c.id
                  AND v.parent_run_id=c.id AND v.role='evidence_reviewer' AND v.lane='review'
                  AND v.status='terminal' AND v.terminal_state='completed'
                  AND EXISTS(SELECT 1 FROM agent_assignments a WHERE a.id=v.assignment_id
                    AND a.coordinator_run_id=c.id AND a.child_run_id=v.id AND a.state='completed')
                  AND EXISTS(SELECT 1 FROM agent_events e WHERE e.run_id=v.id AND e.event_type='model_round_completed')
                  AND EXISTS(SELECT 1 FROM agent_snapshots s WHERE s.run_id=v.id)
                  AND NOT EXISTS(SELECT 1 FROM tool_invocations ti WHERE ti.run_id=v.id)
                  AND NOT EXISTS(SELECT 1 FROM json_each(CASE WHEN json_valid(v.capability_lease_json)
                    THEN v.capability_lease_json ELSE '[]' END)
                    WHERE value NOT IN ('evidence.read','review.write'))
                  AND EXISTS(SELECT 1 FROM agent_review_requests q
                    JOIN agent_review_decisions d ON d.id=q.decision_id
                    WHERE q.root_run_id=c.id AND q.reviewer_run_id=v.id
                      AND q.status IN ('confirmed','rejected') AND d.verdict=q.status
                      AND d.root_run_id=c.id AND d.reviewer_run_id=v.id
                      AND d.candidate_id=q.candidate_id AND d.candidate_revision=q.candidate_revision
                      AND NOT EXISTS(SELECT 1 FROM agent_review_requests newer
                        WHERE newer.root_run_id=q.root_run_id AND newer.candidate_id=q.candidate_id
                          AND newer.candidate_revision>q.candidate_revision)
                  AND EXISTS(SELECT 1 FROM agent_messages md WHERE md.root_run_id=c.id
                    AND md.from_run_id=v.id AND md.to_run_id=c.id
                    AND md.assignment_id=v.assignment_id AND md.correlation_id=q.id
                    AND md.evidence_revision=q.candidate_revision
                    AND md.kind='review_decision' AND md.delivered_at<>'' AND md.acknowledged_at<>'')))))
             AND EXISTS(SELECT 1 FROM agent_messages ms WHERE ms.root_run_id=c.id
                  AND ms.kind='evidence_summary' AND ms.delivered_at<>'' AND ms.acknowledged_at<>'')
             AND NOT EXISTS(SELECT 1 FROM agent_finding_candidates fc
                  WHERE fc.root_run_id=c.id AND fc.status='published'
                    AND NOT EXISTS(SELECT 1 FROM agent_review_decisions d
                      WHERE d.root_run_id=c.id AND d.reviewer_run_id=fc.reviewer_run_id
                        AND d.candidate_revision=fc.candidate_revision AND d.verdict='confirmed'))
         )",
        params![scan_id, attempt, review_gate_satisfied], |row| row.get(0),
    ).map_err(|error| format!("无法核验多智能体链路：{error}"))?;
    let multi_agent_ready = orchestration_integrity_ready;

    let timeline_window = TimelineWindow::latest(&transaction, scan_id, attempt, after_sequence)?;
    let (timeline, latest_sequence) = native_status_timeline(
        &transaction, scan_id, attempt, &timeline_window, &closure_handoff,
        administrative_closure.as_ref(),
    )?;
    let directive_drafts =
        crate::agent_runtime::multi_agent::directive::list_open_drafts(
            &transaction,
            scan_id,
            attempt,
        )?;
    let value = json!({
        "scanId":scan_id,"attemptNumber":attempt,"status":status,"branches":branches,
        "historicalPendingReceipts":historical_pending_receipts,
        "historicalReceiptItems":historical_receipts,
        "administrativeClosure":administrative_closure,"manualAdministrativeClosureAvailable":manual_administrative_closure,
        "closureHandoff":closure_handoff,
        "targets":targets,"llmRequests":requests,"totalTokens":tokens,
        "sourceGaps":source_gaps,"unresolvedContainers":unresolved,"stopDiagnostic":stop_diagnostic,
        "orchestration":"native_pipeline_branches",
        "schedulerActive": scheduler_active && administrative_closure.is_none(),
        "coordinatorLeaseValid": coordinator_lease_valid && administrative_closure.is_none(),
        "childRunsStarted": child_runs_started,
        "assignmentsRunning": assignments_running && administrative_closure.is_none(),
        "mailboxConsumerActive": mailbox_consumer_active && administrative_closure.is_none(),
        "independentReviewerRunId": independent_reviewer_run_id,
        "findingCandidateCount": finding_candidate_count,
        "reviewGateSatisfied": review_gate_satisfied,
        "reviewStatus": review_status,
        "orchestrationIntegrityReady": orchestration_integrity_ready,
        "multiAgentReady": multi_agent_ready,
        "latestSequence": latest_sequence,
        "isIncremental": after_sequence.is_some(),
        "timelineBeforeSequence": timeline_window.first_sequence,
        "hasEarlierTimeline": timeline_window.has_earlier,
        "timelineHasMore": timeline_window.has_newer,
        "directiveDrafts": directive_drafts,
        "directiveRecipients": directive_recipient_roots(&transaction, scan_id, attempt, "team")?
            .into_iter().map(|(root, target)| json!({"threadKey":format!("coordinator:{root}"),"rootRunId":root,"targetKey":target})).collect::<Vec<_>>(),
        "followup": gap_followup_relations(&transaction, scan_id)?,
        "humanAssessmentObligations": native_human_assessment_obligations(&transaction,scan_id,attempt)?,
        "timeline": timeline,
    });
    Ok(if crate::agent_runtime::multi_agent::homogeneous::material_policy::plaintext(&transaction, scan_id, attempt)? {
        value
    } else { crate::agent_runtime::secrets::redact_json(&value) })
}
