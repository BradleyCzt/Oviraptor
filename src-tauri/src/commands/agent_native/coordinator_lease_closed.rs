// Original signed expiry plus paid, saved local delivery. This reader grants
// neither a worker lease nor SDK permission; paid admission is separate.
impl NativeCoordinatorFrame {
    fn lease_closed_mapper(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{contract::AgentRole, multi_agent::{attempts, budget, mailbox, specialist}};
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return Self::lease_closed_mapper(&tx, context, actor, child);
        }
        if !matches!(child.role,AgentRole::SpaApiMapper | AgentRole::InputParser | AgentRole::ClientSide | AgentRole::IdentitySession | AgentRole::DeepInvestigator | AgentRole::EvidenceReviewer) {
            return Err("root_lease_closed_role_unimplemented".into());
        }
        let root = native_coordinator_root_context(context, actor);
        native_coordinator_tick_authority(db, &root, actor)?;
        native_coordinator_frozen_evidence(db, &root)?;
        let declaration = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
            .ok_or("root_lease_closed_birth_missing")?;
        if declaration.lease_expiry_supervision().is_none() {
            return Err("root_lease_closed_original_protocol_missing".into());
        }
        if child.role == AgentRole::SpaApiMapper
            && declaration.bootstrap_dispatch().is_some_and(|contract| contract["schemaVersion"] == 15)
        {
            return Self::lease_closed_mapper_loop_original_on(db, actor, child);
        }
        let issuer = original_withdrawal_issuer_on(db, actor)?
            .ok_or("root_lease_closed_issuer_missing")?;
        let worker = attempts::current(db, actor, &child.assignment_id)?;
        if worker.state != "expired" || worker.child_run_id != child.run_id {
            return Err("root_lease_closed_original_expiry_required".into());
        }
        issuer.verify_issued(db, &worker.id)?;
        attempts::verify_expired_closed_audit(db, actor, child)?;
        specialist::verify_received_original(db, actor, &child.assignment_id)?;
        let receipt = specialist::received_for_local_delivery(db, actor, child)?;
        let reported: bool = db.query_row("SELECT json_extract(response_json,'$.usageReported') IS 1 FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2 AND state='received'",
            params![child.assignment_id,child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
        if !reported || receipt.usage.input_tokens.checked_add(receipt.usage.output_tokens) != Some(receipt.usage.total_tokens)
            || !receipt.rejection.is_empty() || receipt.usage.model_requests != 1 {
            return Err("root_lease_closed_original_invoice_invalid".into());
        }
        let invoice = [receipt.usage.input_tokens, receipt.usage.cached_input_tokens,
            receipt.usage.output_tokens, receipt.usage.model_requests];
        for (i, dimension) in budget::DIMENSIONS.iter().enumerate() {
            let balance = budget::balance_for_attempt(db, &actor.root_run_id,
                &child.assignment_id, &worker.id, dimension)?;
            if balance.reserved != 0 || balance.indeterminate != 0
                || i < 4 && (invoice[i] < 0 || balance.consumed != invoice[i])
                || (4..8).contains(&i) && balance.consumed != 0 {
                return Err("root_lease_closed_original_budget_unresolved".into());
            }
        }
        let (tokens, cached, requests): (i64, i64, i64) = db.query_row(
            "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1 AND cancel_requested_at=''", [&child.run_id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
        if tokens != receipt.usage.total_tokens || cached != receipt.usage.cached_input_tokens
            || requests != receipt.usage.model_requests {
            return Err("root_lease_closed_original_usage_conflict".into());
        }
        if child.role==AgentRole::EvidenceReviewer {
            return native_lease_reviewer_closed_frame(db,context,actor,child,&worker,receipt.usage.as_json());
        }
        if child.role==AgentRole::DeepInvestigator {
            return native_lease_deep_closed_frame(db,context,actor,child,&worker,receipt.usage.as_json());
        }
        let is_input = child.role == AgentRole::InputParser;
        let is_client = child.role == AgentRole::ClientSide;
        let is_identity = child.role == AgentRole::IdentitySession;
        let role = child.role.as_str();
        let kind = if is_input {"input_assessment"} else if is_identity {"identity_assessment"} else {"evidence_summary"};
        let correlation = format!("{}:{}",if is_input {"input-parser"} else if is_client {"client-side"} else if is_identity {"identity"} else {"mapper"},child.assignment_id);
        let dedup = format!("{}:{}:{}:1",child.assignment_id,kind,correlation);
        if is_input && !input_parser_completed_original_replay(db,actor,child,&context.target_dir)? {
            return Err("root_lease_closed_input_original_delivery_required".into());
        }
        let mut q = db.prepare("SELECT id FROM agent_messages WHERE root_run_id=?1 AND assignment_id=?2 AND from_run_id=?3 AND to_run_id=?1 AND from_agent=?6 AND to_agent='coordinator' AND kind=?7 AND run_id=?1 AND correlation_id=?4 AND evidence_revision=1 AND dedup_key=?5 AND artifact_refs_json='[]'").map_err(|e|e.to_string())?;
        let ids = q.query_map(params![actor.root_run_id, child.assignment_id, child.run_id,
            correlation,dedup,role,kind], |r| r.get::<_,String>(0))
            .map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
        if ids.len() != 1 { return Err("root_lease_closed_original_ack_missing_or_conflicting".into()); }
        let message = mailbox::verified_completed_output(db, actor, &ids[0])?;
        let expected = crate::agent_runtime::secrets::redact_json(&if is_input {
            json!({"summary":receipt.text,"inputParserTask":input_parser_build_surface(db,actor,child,&context.target_dir)?})
        } else if is_client {
            native_lease_client_closed_payload(db,context,actor,child,&worker)?
        } else if is_identity {
            native_lease_identity_closed_payload(db,actor,child,&worker)?
        } else { json!({"summary":receipt.text}) });
        if message.payload != expected {
            return Err("root_lease_closed_original_payload_conflict".into());
        }
        crate::collaboration_events::client_delivery_schema::verify(db)?;
        let (events, kinds): (i64, i64) = db.query_row("SELECT count(*),count(DISTINCT entity_type) FROM agent_collaboration_events
            WHERE scan_id=?1 AND attempt_number=?2 AND entity_type=event_type AND (
             (entity_type='assignment' AND entity_id=?3 AND payload_json=json_object('role',?6,'state','completed'))
             OR (entity_type='agent_run' AND entity_id=?4 AND payload_json=json_object('role',?6,'status','terminal','terminalState','completed'))
             OR (entity_type='mailbox_message' AND entity_id=?5 AND payload_json=(SELECT
               json_object('kind',?7,'deliveredAt',delivered_at,'acknowledgedAt',acknowledged_at) FROM agent_messages WHERE id=?5)))",
            params![actor.scan_id,actor.attempt_number,child.assignment_id,child.run_id,ids[0],role,kind],
            |r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
        if events != 3 || kinds != 3 { return Err("root_lease_closed_original_delivery_events_invalid".into()); }
        let request: String = db.query_row("SELECT request_json FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2", params![child.assignment_id,child.run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
        let request: JsonValue = serde_json::from_str(&request).map_err(|_|"root_lease_closed_request_invalid")?;
        if request.get("humanDirectiveDispatch").is_some() {
            return Err("root_lease_closed_ordered_role_unimplemented".into());
        }
        let receipt_id: String = db.query_row("SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1 AND root_run_id=?2", params![worker.id,actor.root_run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
        let mut rows = Self::closed_rows(db, actor, child)?;
        for (sql, keys) in [
            ("SELECT rowid,* FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1", vec![worker.id.as_str()]),
            ("SELECT rowid,* FROM agent_worker_expiry_issuance_seals WHERE receipt_id=?1 AND root_run_id=?2", vec![receipt_id.as_str(), actor.root_run_id.as_str()]),
            ("SELECT rowid,* FROM agent_root_mode_birth_seals WHERE root_run_id=?1", vec![actor.root_run_id.as_str()]),
            ("SELECT rowid,* FROM agent_root_mode_definitions WHERE root_run_id=?1", vec![actor.root_run_id.as_str()]),
            ("SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 ORDER BY rowid", vec![child.assignment_id.as_str()]),
            ("SELECT rowid,* FROM agent_collaboration_events WHERE entity_id IN (?1,?2,?3) ORDER BY sequence", vec![child.assignment_id.as_str(), child.run_id.as_str(), ids[0].as_str()]),
        ] {
            rows.push(NativeCoordinatorFrozenRows::capture(db, sql, &keys)?);
        }
        if is_input {
            let (slice,_) = input_parser_assignment_slice(db,actor,child)?;
            for binding in slice["frame"]["factBindings"].as_array().ok_or("root_lease_closed_input_bindings_invalid")? {
                let id = binding["evidenceRef"].as_str().ok_or("root_lease_closed_input_bindings_invalid")?;
                rows.push(NativeCoordinatorFrozenRows::capture(db,NATIVE_INPUT_PARSER_FRAME_FACT_SQL,&[id,actor.root_run_id.as_str()])?);
            }
        }
        if is_client {
            let task = crate::agent_runtime::multi_agent::client_side::assignment(db,actor,child)?;
            for observation in &task.observations {
                rows.push(NativeCoordinatorFrozenRows::capture(db,NATIVE_INPUT_PARSER_FRAME_FACT_SQL,&[observation.id.as_str(),actor.root_run_id.as_str()])?);
            }
        }
        if is_identity {
            native_coordinator_identity_message_events(db,actor,&ids[0])?;
            rows.push(NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM sentinel_scan_contexts WHERE scan_id=?1",&[actor.scan_id.as_str()])?);
            rows.push(NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM browser_auth_sessions WHERE owner_scan_id=?1 AND id IN (SELECT value FROM sentinel_scan_contexts c,json_each(c.policy_json,'$.authSessionIds') WHERE c.scan_id=?1) ORDER BY rowid",&[actor.scan_id.as_str()])?);
        }
        let mut semantic = json!({"rootRunId":actor.root_run_id,
            "workerAttemptId":worker.id,"receiptId":receipt_id,"assignmentId":child.assignment_id,
            "childRunId":child.run_id,"messageId":ids[0],"originalInvoice":receipt.usage.as_json(),
            "savedOutput":expected,"dispatchGranted":false,"targetRequestsGranted":0});
        // Preserve the published Mapper fact format byte for byte.
        if is_input || is_client || is_identity {semantic["role"] = role.into();}
        Ok(Self {kind:"lease-expired",semantic,rows,review_refs:None,review_meta:None})
    }

    fn verify_lease_closed_source(&self, db: &rusqlite::Connection, context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease) -> Result<(), String> {
        let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id:self.semantic["assignmentId"].as_str().ok_or("root_lease_closed_binding_invalid")?.into(),
            run_id:self.semantic["childRunId"].as_str().ok_or("root_lease_closed_binding_invalid")?.into(),
            role:match self.semantic.get("role").and_then(JsonValue::as_str) {
                None => crate::agent_runtime::contract::AgentRole::SpaApiMapper,
                Some("input_parser") => crate::agent_runtime::contract::AgentRole::InputParser,
                Some("spa_api_mapper") => crate::agent_runtime::contract::AgentRole::SpaApiMapper,
                Some("client_side") => crate::agent_runtime::contract::AgentRole::ClientSide,
                Some("identity_session") => crate::agent_runtime::contract::AgentRole::IdentitySession,
                Some("deep_investigator") => crate::agent_runtime::contract::AgentRole::DeepInvestigator,
                Some("evidence_reviewer") => crate::agent_runtime::contract::AgentRole::EvidenceReviewer,
                _ => return Err("root_lease_closed_role_unimplemented".into()),
            },
        };
        let current = Self::lease_closed_mapper(db, context, actor, &child)?;
        if self.fact() != current.fact() || self.review_refs.is_some() || self.review_meta.is_some() {
            return Err("root_lease_closed_original_fact_changed".into());
        }
        Ok(())
    }
}

// Hold this original inode once across all paid-frame reads and publication.
// Rebuilding a frame never reprobes an inode already held by its outer hook.
fn native_coordinator_lease_original_sdk_exit(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<crate::agent_runtime::execution_owner::NativeInvocationOwner, String> {
    if child.role == crate::agent_runtime::contract::AgentRole::SpaApiMapper {
        let db = db::open(&context.db_path)?;
        if crate::agent_runtime::web_mode::root::read(&db, &actor.root_run_id)?
            .and_then(|mode| mode.bootstrap_dispatch())
            .is_some_and(|contract| contract["schemaVersion"] == 15)
        {
            return native_coordinator_mapper_expired_sdk_exit(context, actor, child);
        }
    }
    crate::agent_runtime::execution_owner::probe_native_invocation(&context.db_path,
        &actor.scan_id, actor.attempt_number, "specialist-sdk", &child.run_id)
        .map_err(|e| format!("root_lease_closed_sdk_not_idle:{e}"))?
        .ok_or_else(|| "root_lease_closed_original_sdk_exit_missing".into())
}
