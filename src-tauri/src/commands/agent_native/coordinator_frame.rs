// A private captured closed-worker fact, never a live worker capability.
#[derive(Clone)]
struct NativeCoordinatorFrame {
    kind: &'static str,
    semantic: JsonValue,
    rows: Vec<NativeCoordinatorFrozenRows>,
    review_refs: Option<Vec<String>>,
    review_meta: Option<(String, i64)>,
}
impl NativeCoordinatorFrame {
    fn closed_rows(
        tx: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Vec<NativeCoordinatorFrozenRows>, String> {
        use crate::agent_runtime::multi_agent::attempts;
        if tx.is_autocommit() { return Err("root_frame_snapshot_required".into()); }
        let worker = attempts::current(tx, actor, &child.assignment_id)?;
        let closed: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
            WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND r.root_run_id=?3 AND r.parent_run_id=?3
            AND r.assignment_id=a.id AND a.role=?4 AND r.role=a.role AND a.lane=r.lane
            AND a.target_key=?5 AND r.target_url=?5 AND r.scan_id=?6 AND r.attempt_number=?7
            AND a.lease_epoch=?8 AND a.fencing_token=?9 AND a.state='completed' AND a.budget_settled_at<>''
            AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.finished_at<>'' AND r.finished_at<>''
            AND r.status='terminal' AND r.terminal_state='completed')
            AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='')
            AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
            params![child.assignment_id,child.run_id,actor.root_run_id,child.role.as_str(),actor.target_key,
                actor.scan_id,actor.attempt_number,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !closed
            || worker.child_run_id != child.run_id
            || worker.finished_at.is_empty()
            || !matches!(worker.state.as_str(), "completed" | "failed" | "expired")
        {
            return Err("root_frame_worker_not_closed".into());
        }
        let capture = |sql: &str, key: &str| NativeCoordinatorFrozenRows::capture(tx, sql, &[key]);
        let mapper_loop = child.role == crate::agent_runtime::contract::AgentRole::SpaApiMapper
            && crate::agent_runtime::web_mode::root::read(tx, &actor.root_run_id)?
                .and_then(|mode| mode.bootstrap_dispatch())
                .is_some_and(|contract| contract["schemaVersion"] == 15);
        let mut rows: Vec<NativeCoordinatorFrozenRows> = vec![
            capture(
                "SELECT rowid,* FROM agent_assignment_attempts WHERE id=?1",
                &worker.id,
            ),
            capture(
                "SELECT rowid,* FROM agent_assignments WHERE id=?1",
                &child.assignment_id,
            ),
            capture("SELECT rowid,* FROM agent_runs WHERE id=?1", &child.run_id),
            capture(
                if mapper_loop {
                    "SELECT rowid,* FROM agent_mapper_loop_journal WHERE child_run_id=?1 ORDER BY round_number,phase"
                } else {
                    "SELECT rowid,* FROM agent_specialist_calls WHERE child_run_id=?1"
                },
                &child.run_id,
            ),
            NativeCoordinatorFrozenRows::capture_optional(
                tx,
                "SELECT rowid,* FROM agent_model_cost_facts WHERE child_run_id=?1 ORDER BY rowid",
                &[&child.run_id],
            ),
            capture(
                "SELECT rowid,* FROM agent_budget_entries WHERE lease_attempt_id=?1 ORDER BY rowid",
                &worker.id,
            ),
            // Client's dedicated receipt writer publishes no synthetic snapshot.
            // Its exact empty/nonempty snapshot state is still frozen; other
            // roles retain their existing required physical snapshot proof.
            if child.role == crate::agent_runtime::contract::AgentRole::ClientSide || mapper_loop {
                NativeCoordinatorFrozenRows::capture_optional(tx,
                    "SELECT rowid,* FROM agent_snapshots WHERE run_id=?1 ORDER BY rowid", &[&child.run_id])
            } else {
                capture("SELECT rowid,* FROM agent_snapshots WHERE run_id=?1 ORDER BY rowid", &child.run_id)
            },
            if mapper_loop {
                NativeCoordinatorFrozenRows::capture_optional(tx,
                    "SELECT rowid,* FROM agent_events WHERE run_id=?1 ORDER BY sequence", &[&child.run_id])
            } else {
                capture("SELECT rowid,* FROM agent_events WHERE run_id=?1 ORDER BY sequence", &child.run_id)
            },
            capture(
                "SELECT rowid,* FROM agent_messages WHERE assignment_id=?1 ORDER BY rowid",
                &child.assignment_id,
            ),
        ]
        .into_iter()
        .collect::<Result<_, String>>()?;
        if mapper_loop {
            rows.push(capture("SELECT rowid,* FROM agent_mapper_loop_final_receipts WHERE child_run_id=?1", &child.run_id)?);
            rows.push(NativeCoordinatorFrozenRows::capture(tx,
                "SELECT rowid,* FROM agent_collaboration_events WHERE (entity_type='assignment' AND entity_id=?1) OR (entity_type='agent_run' AND entity_id=?2) ORDER BY sequence",
                &[&child.assignment_id, &child.run_id])?);
        }
        Ok(rows)
    }
    fn mapper(
        tx: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{mailbox, specialist},
        };
        if child.role != AgentRole::SpaApiMapper {
            return Err("root_frame_role_invalid".into());
        }
        let text = if crate::agent_runtime::web_mode::root::read(tx, &actor.root_run_id)?
            .and_then(|mode| mode.bootstrap_dispatch())
            .is_some_and(|contract| contract["schemaVersion"] == 15)
        {
            specialist::mapper_loop::final_seal::read_final_original_on(tx, actor, child)?
                .ok_or("root_frame_mapper_loop_original_final_missing")?;
            specialist::mapper_loop::verified_original_output_on(tx, actor, child)?.text().to_owned()
        } else {
            let receipt = specialist::received_for_local_delivery(tx, actor, child)?;
            if !receipt.rejection.is_empty() {
                return Err("root_frame_paid_output_rejected".into());
            }
            receipt.text
        };
        let id:String=tx.query_row("SELECT id FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2
            AND from_run_id=?3 AND to_run_id=?1 AND kind='evidence_summary' AND evidence_revision=1",
            params![actor.root_run_id,child.assignment_id,child.run_id],|r|r.get(0)).map_err(|_|"root_frame_output_missing")?;
        let message = mailbox::verified_completed_output(tx, actor, &id)?;
        if message.payload["summary"].as_str() != Some(&text) {
            return Err("root_frame_output_receipt_mismatch".into());
        }
        let mut semantic = json!({"messageId":id,"role":"spa_api_mapper",
            "output":native_coordinator_mapper_semantic(&text)?});
        if crate::agent_runtime::web_mode::root::read(tx, &actor.root_run_id)?
            .and_then(|mode| mode.bootstrap_dispatch())
            .is_some_and(|contract| contract["schemaVersion"] == 15)
        {
            let original = specialist::mapper_loop::final_seal::read_final_original_on(tx, actor, child)?
                .ok_or("root_frame_mapper_loop_original_final_missing")?;
            if original.message_id() != id {
                return Err("root_frame_mapper_loop_original_message_changed".into());
            }
            semantic["mapperLoopFinalReceiptId"] = original.receipt_id().into();
            semantic["mapperLoopFinalSource"] = original.fact().clone();
        }
        Ok(Self {
            kind: "mapper-output",
            semantic,
            rows: Self::closed_rows(tx, actor, child)?,
            review_refs: None,
            review_meta: None,
        })
    }
    fn reviewer(
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
        let (frozen, missing, refs) = sealed_gap_review_candidate_in_transaction(
            tx,
            &actor.root_run_id,
            candidate,
            revision,
            missing,
            &context.target_dir,
        )?;
        let (assignment,run,id):(String,String,String)=tx.query_row("SELECT q.assignment_id,q.reviewer_run_id,m.id
            FROM agent_review_requests q JOIN agent_messages m ON m.correlation_id=q.id
            AND m.assignment_id=q.assignment_id AND m.kind='review_decision' AND m.to_run_id=q.root_run_id
            WHERE q.root_run_id=?1 AND q.candidate_id=?2 AND q.candidate_revision=?3 AND q.status='insufficient_evidence'",
            params![actor.root_run_id,candidate,revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"root_frame_review_missing")?;
        let child = scheduler::ScheduledChild {
            assignment_id: assignment,
            run_id: run,
            role: AgentRole::EvidenceReviewer,
        };
        let receipt = specialist::received_for_local_delivery(tx, actor, &child)?;
        let decision = validated_review_decision(&receipt.text)?;
        if !receipt.rejection.is_empty()
            || decision.verdict != "insufficient_evidence"
            || decision.missing_evidence != missing
        {
            return Err("root_frame_review_receipt_mismatch".into());
        }
        let message = mailbox::verified_completed_output(tx, actor, &id)?;
        if message.payload["verdict"] != "insufficient_evidence"
            || message.payload["candidateId"] != candidate
            || message.payload["candidateRevision"] != revision
            || message.payload["summary"] != decision.summary
        {
            return Err("root_frame_review_envelope_mismatch".into());
        }
        let mut rows = Self::closed_rows(tx, actor, &child)?;
        rows.push(NativeCoordinatorFrozenRows::capture(tx,"SELECT rowid,* FROM agent_review_requests WHERE root_run_id=?1 AND candidate_id=?2 AND candidate_revision=CAST(?3 AS INTEGER)",&[&actor.root_run_id,candidate,&revision.to_string()])?);
        rows.push(NativeCoordinatorFrozenRows::capture(tx,"SELECT rowid,* FROM agent_review_decisions WHERE root_run_id=?1 AND candidate_id=?2 AND candidate_revision=CAST(?3 AS INTEGER)",&[&actor.root_run_id,candidate,&revision.to_string()])?);
        Ok(Self {
            kind: "reviewer-decision",
            semantic: json!({"messageId":id,"candidateId":candidate,"revision":revision,
            "candidateHash":crate::agent_runtime::store::stable_hash(&frozen.to_string()),"verdict":decision.verdict,
            "summary":decision.summary,"missingEvidence":missing,"trustedFactRefs":refs}),
            rows,
            review_refs: Some(refs),
            review_meta: Some((candidate.into(), revision)),
        })
    }
    fn fact(&self) -> JsonValue {
        json!({"kind":self.kind,"semantic":self.semantic,"physicalFacts":self.rows.iter().map(NativeCoordinatorFrozenRows::fact).collect::<Vec<_>>()})
    }
    fn verify(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        if db.is_autocommit() && self.kind == "mapper-output"
            && self.semantic.get("mapperLoopFinalReceiptId").is_some()
        {
            let snapshot = db.unchecked_transaction().map_err(|error| error.to_string())?;
            return self.verify(&snapshot, context, actor);
        }
        if let Some((candidate, revision)) = &self.review_meta {
            verify_review_snapshot(
                db,
                &actor.root_run_id,
                candidate,
                *revision,
                &context.target_dir,
            )?;
        }
        for row in &self.rows {
            row.verify(db)?;
        }
        if self.kind == "mapper-output" && self.semantic.get("mapperLoopFinalReceiptId").is_some() {
            let message = self.semantic["messageId"].as_str().ok_or("root_frame_mapper_loop_message_missing")?;
            let (assignment_id, run_id): (String, String) = db.query_row(
                "SELECT assignment_id,from_run_id FROM agent_messages WHERE id=?1 AND root_run_id=?2 AND to_run_id=?2 AND kind='evidence_summary'",
                params![message, actor.root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
            ).map_err(|_| "root_frame_mapper_loop_original_message_missing")?;
            let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
                assignment_id, run_id, role: crate::agent_runtime::contract::AgentRole::SpaApiMapper,
            };
            let original = Self::mapper(db, actor, &child)?;
            if self.fact() != original.fact() {
                return Err("root_frame_mapper_loop_original_source_changed".into());
            }
        }
        if self.kind == "human-directive" {
            self.verify_human_source(db, actor)?;
        }
        if self.kind == "budget-allocation" {
            self.verify_budget_source(db, actor)?;
        }
        if self.kind == "http-budget-consumption" {
            self.verify_http_budget_consumption_source(db, context, actor)?;
        }
        if self.kind == "budget-consumption" {
            self.verify_budget_consumption_source(db, context, actor)?;
        }
        if self.kind == "authorization-output" {
            self.verify_authorization_output_source(db, context, actor)?;
        }
        if self.kind == "business-logic-output" {
            self.verify_business_logic_output_source(db, context, actor)?;
        }
        if self.kind == "business-logic-input" {
            self.verify_business_logic_dispatch_source(db, context, actor)?;
        }
        if self.kind == "external-surface-output" {
            self.verify_external_surface_output_source(db, context, actor)?;
        }
        if self.kind == "capability-changed" {
            self.verify_capability_transition_source(db, context, actor)?;
        }
        if self.kind == "identity-session-output" {
            self.verify_identity_source(db, actor)?;
        }
        if self.kind == "client-side-output" {
            self.verify_client_source(db, context, actor)?;
        }
        if self.kind == "lease-expired" {
            if self.semantic["role"] == "web_executor" {
                self.verify_web_expiry_source(db, context, actor)?;
            } else {
                self.verify_lease_closed_source(db, context, actor)?;
            }
        }
        if self.kind == "new-evidence" {
            self.verify_new_evidence_source(db, context, actor)?;
        }
        if self.kind == "protection-changed" {
            self.verify_protection_source(db, context, actor)?;
        }
        if self.kind == "input-parser-output" {
            self.verify_input_parser_trigger_source(db, context, actor)?;
        }
        if let Some(refs) = &self.review_refs {
            if review_fact_refs(db, &actor.root_run_id, &context.target_dir)? != *refs {
                return Err("root_frame_review_facts_changed".into());
            }
        }
        Ok(())
    }
    fn step(&self) -> &'static str {
        native_coordinator_frame_step_for_kind(self.kind)
    }
}

/// U58: single table for wired frame → permitted step (shared with inventory tests).
fn native_coordinator_frame_step_for_kind(kind: &str) -> &'static str {
    // Unknown kinds must not silently advertise dispatch:deep_investigator.
    // reviewer-decision keeps that step on purpose (independent investigator path).
    // U61: reserved role-output kinds map to their future Root dispatch steps but
    // stay off the wired supervision inventory until producers exist.
    match kind {
        "human-directive" => "assess:human_directive",
        "new-evidence" => "assess:new_evidence",
        "lease-expired" => "assess:lease_expired",
        "protection-changed" => "assess:protection_changed",
        "budget-allocation" => "assess:budget_allocation",
        "budget-consumption" => "assess:budget_consumption",
        "http-budget-consumption" => "assess:http_budget_consumption",
        "capability-changed" => "assess:capability_changed",
        "mapper-output" => "dispatch:web_executor",
        "investigator-proposal" => "assess:gap_proposal",
        "client-side-output" => "assess:client_side_configuration",
        "identity-session-output" => "assess:identity_session_metadata",
        "reviewer-decision" => "dispatch:deep_investigator",
        "input-parser-output" => "dispatch:input_parser",
        "authorization-output" => "assess:authorization_output",
        "external-surface-output" => "assess:external_surface_output",
        "business-logic-output" => "assess:business_logic_output",
        "business-logic-input" => "dispatch:business_logic",
        // DESIGN §5.4：同质 Worker 主管决策帧。只新增映射行，不改旧 kind 的逻辑，
        // 也不把新 kind 塞进旧 wired 清单（那里有 18 条的存量盘点断言）。
        "homogeneous-decision" => "assess:homogeneous_decision",
        _ => "assess:unmapped_frame_kind",
    }
}
