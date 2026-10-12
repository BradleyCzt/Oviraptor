// Original paused Web fee delivery is distinct from a successful execution.
impl NativeCoordinatorFrame {
    fn web_expiry_closed_on(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
        financial:&crate::agent_runtime::multi_agent::budget::model::expired_web_receipt::ExpiredWebFinancialProof,
    ) -> Result<Self, String> {
        financial.require_on(db, actor, child)?;
        native_web_expiry_require_current_child_on(db, actor, child)?;
        let root = native_coordinator_root_context(context, actor);
        native_coordinator_tick_authority(db, &root, actor)?;
        Self::web_expiry_original_on(db, actor, child)
    }

    // Read-only signed delivery/withdrawal source. It confers no live authority.
    fn web_expiry_original_on(
        db: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{attempts, budget},
            web_mode,
        };
        if db.is_autocommit() {
            return Err("root_web_expiry_snapshot_required".into());
        }
        if child.role != AgentRole::WebExecutor {
            return Err("root_web_expiry_role_invalid".into());
        }
        let original = original_root_capture_mode(db, &actor.root_run_id)?;
        web_mode::root::verify_birth(db, &actor.root_run_id, &original)?;
        let declaration = web_mode::root::read(db, &actor.root_run_id)?
            .ok_or("root_web_expiry_original_protocol_missing")?;
        let bootstrap = declaration.bootstrap_dispatch().unwrap_or(JsonValue::Null);
        if declaration.mode() != web_mode::WebMode::Multi
            || declaration.fact() != original.fact()
            || declaration.lease_expiry_supervision().is_none()
            || !matches!(bootstrap["schemaVersion"].as_i64(), Some(9..=15))
            || bootstrap["webLeaseExpiryRule"] != budget::model::expired_web_receipt::RULE
        {
            return Err("root_web_expiry_original_protocol_missing".into());
        }
        budget::root::RootOwner::load_original(db, &actor.root_run_id)?
            .require_original_coordinator(db, actor)?;
        let source = web_mode::web_expiry_delivery::read_closed_original_on(db, actor, child)?
            .ok_or("root_web_expiry_original_delivery_missing")?;
        let worker = attempts::current(db, actor, &child.assignment_id)?;
        if worker.child_run_id != child.run_id
            || worker.state != "expired"
            || worker.failure_class != "worker_lease_expired"
        {
            return Err("root_web_expiry_original_worker_invalid".into());
        }
        let issuer = original_withdrawal_issuer_on(db, actor)?
            .ok_or("root_web_expiry_original_issuer_missing")?;
        issuer.verify_issued(db, &worker.id)?;
        let receipt_id:String=db.query_row("SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1 AND root_run_id=?2",params![worker.id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        // Capture only this closed source; no Root SDK bill, C expiry or global
        // clock/current balance becomes a changed fact.
        let rows=vec![
            NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",&[&worker.id])?,
            NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM agent_worker_expiry_issuance_seals WHERE receipt_id=?1 AND root_run_id=?2",&[&receipt_id,&actor.root_run_id])?,
            NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM agent_root_mode_birth_seals WHERE root_run_id=?1",&[&actor.root_run_id])?,
            NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM agent_root_mode_definitions WHERE root_run_id=?1",&[&actor.root_run_id])?,
            NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM agent_assignment_attempts WHERE id=?1",&[&worker.id])?,
            NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM agent_messages WHERE id=?1",&[source.message_id()])?,
        ];
        let frame = Self {
            kind: "lease-expired",
            semantic: json!({"role":"web_executor","rootRunId":actor.root_run_id,
            "workerAttemptId":worker.id,"receiptId":receipt_id,"assignmentId":child.assignment_id,"childRunId":child.run_id,
            "deliveryReceiptId":source.receipt_id(),"messageId":source.message_id(),"closedDelivery":source.fact(),
            "dispatchGranted":false,"targetRequestsGranted":0,"executionGrant":false,"recoveryImplemented":false}),
            rows,
            review_refs: None,
            review_meta: None,
        };
        native_coordinator_require_original_lease_fact(db, actor, &frame.fact())?;
        Ok(frame)
    }

    fn verify_web_expiry_source(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.verify_web_expiry_source(&tx, context, actor);
        }
        let text = |key: &str| {
            self.semantic[key]
                .as_str()
                .ok_or("root_web_expiry_frame_invalid")
        };
        let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id: text("assignmentId")?.into(),
            run_id: text("childRunId")?.into(),
            role: crate::agent_runtime::contract::AgentRole::WebExecutor,
        };
        native_web_expiry_require_current_child_on(db, actor, &child)?;
        let actual = Self::web_expiry_original_on(db, actor, &child)?;
        if actual.fact() != self.fact() || self.review_refs.is_some() || self.review_meta.is_some()
        {
            return Err("root_web_expiry_original_fact_changed".into());
        }
        let root = native_coordinator_root_context(context, actor);
        native_coordinator_tick_authority(db, &root, actor)
    }
}

// Historical expiry receipts cannot override a subsequent explicit cancellation.
fn native_web_expiry_require_current_child_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<(), String> {
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id WHERE r.id=?1 AND a.id=?2 AND r.root_run_id=?3 AND a.coordinator_run_id=?3 AND r.status='paused' AND r.terminal_state='' AND r.finished_at='' AND r.cancel_requested_at='' AND a.state='paused' AND a.failure_class='worker_lease_expired' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0)",params![child.run_id,child.assignment_id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if valid {
        Ok(())
    } else {
        Err("root_web_expiry_current_child_stopped".into())
    }
}
