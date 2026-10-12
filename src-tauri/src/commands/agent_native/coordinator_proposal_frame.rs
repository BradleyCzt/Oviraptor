impl NativeCoordinatorFrame {
    fn investigator(
        tx: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        candidate: &str,
        revision: i64,
        missing: &JsonValue,
    ) -> Result<Self, String> {
    if tx.is_autocommit() { return Err("gap_original_snapshot_required".into()); }
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{mailbox, scheduler, specialist},
        };
        let (_, missing, refs) = sealed_gap_review_candidate_in_transaction(
            tx,
            &actor.root_run_id,
            candidate,
            revision,
            missing,
            &context.target_dir,
        )?;
        let (assignment,run,task):(String,String,String)=tx.query_row("SELECT id,child_run_id,task_slice_json
            FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'
            AND trigger_code='reviewer_insufficient_evidence' AND evidence_revision=?2 AND state='completed'",
            params![actor.root_run_id,revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"root_proposal_frame_missing")?;
        let task: JsonValue =
            serde_json::from_str(&task).map_err(|_| "root_proposal_frame_task_invalid")?;
        if task["candidateId"] != candidate
            || task["revision"] != revision
            || !completed_gap_round_valid(
                tx,
                &actor.root_run_id,
                &assignment,
                &run,
                candidate,
                revision,
                &missing,
                &context.target_dir,
            )?
        {
            return Err("root_proposal_frame_not_closed".into());
        }
        let child = scheduler::ScheduledChild {
            assignment_id: assignment,
            run_id: run,
            role: AgentRole::DeepInvestigator,
        };
        let receipt = specialist::received_for_local_delivery(tx, actor, &child)?;
        if !receipt.rejection.is_empty() {
            return Err("root_proposal_frame_paid_output_rejected".into());
        }
        let raw: JsonValue = serde_json::from_str(&receipt.text)
            .map_err(|_| "root_proposal_frame_schema_invalid")?;
        let proposal = parse_gap_proposal(&raw, &refs, &missing)?;
        let (message, assessment): (String, String) = tx
            .query_row(
                "SELECT p.id,a.payload_json
            FROM agent_messages p JOIN agent_messages a ON a.assignment_id=p.assignment_id
            AND a.root_run_id=p.root_run_id AND a.kind='proposal_assessed'
            WHERE p.assignment_id=?1 AND p.root_run_id=?2 AND p.kind='gap_proposed'",
                params![child.assignment_id, actor.root_run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| "root_proposal_frame_mailbox_missing")?;
        let message = mailbox::verified_completed_output(tx, actor, &message)?;
        let assessment: JsonValue = serde_json::from_str(&assessment)
            .map_err(|_| "root_proposal_frame_assessment_invalid")?;
        let expected_assessment = crate::agent_runtime::secrets::redact_json(&json!({
            "candidateId":candidate,"evidenceRevision":revision,
            "summary":"当前缺口需要新的已校验合同或证据 revision，暂不派发目标请求",
            "decision":"deferred_requires_new_evidence_revision",
            "reasonCode":gap_assessment_reason(&proposal,3),
            "newAttemptRequired":proposal["nextStep"]=="request_new_contract",
            "targetRequestsGranted":0,"schemaVersion":3
        }));
        mailbox::verified_completed_gap_assessment(tx,actor,&child,&expected_assessment)?;
        if message.payload["schemaVersion"] != 3
            || message.payload["proposal"] != crate::agent_runtime::secrets::redact_json(&proposal)
            || assessment["schemaVersion"] != 3
            || assessment["decision"] != "deferred_requires_new_evidence_revision"
            || assessment["reasonCode"] != gap_assessment_reason(&proposal, 3)
            || assessment["targetRequestsGranted"] != 0
        {
            return Err("root_proposal_frame_mailbox_conflict".into());
        }
        let mut rows = Self::closed_rows(tx, actor, &child)?;
        rows.push(NativeCoordinatorFrozenRows::capture(tx,"SELECT rowid,* FROM agent_review_requests WHERE root_run_id=?1 AND candidate_id=?2 AND candidate_revision=CAST(?3 AS INTEGER)",&[&actor.root_run_id,candidate,&revision.to_string()])?);
        rows.push(NativeCoordinatorFrozenRows::capture(tx,"SELECT rowid,* FROM agent_review_decisions WHERE root_run_id=?1 AND candidate_id=?2 AND candidate_revision=CAST(?3 AS INTEGER)",&[&actor.root_run_id,candidate,&revision.to_string()])?);
        Ok(Self {
            kind: "investigator-proposal",
            semantic: json!({"messageId":message.id,
            "candidateId":candidate,"revision":revision,"proposal":crate::agent_runtime::secrets::redact_json(&proposal),"existingRustAssessment":assessment,
            "trustedFactRefs":refs,"newTargetEvidenceProven":false,"targetRequestsGranted":0}),
            rows,
            review_refs: Some(refs),
            review_meta: Some((candidate.into(), revision)),
        })
    }
}
