// Signed normal Web closure, not an expiry event or a live worker capability.
impl NativeCoordinatorFrame {
    fn capability_transition_closed_on(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Self, String> {
        let root = native_coordinator_root_context(context, actor);
        native_coordinator_tick_authority(db, &root, actor)?;
        let frame = Self::capability_transition_original_on(db, actor, child)?;
        frame.verify_capability_transition_source(db, &root, actor)?;
        Ok(frame)
    }

    // Pure original source; does not require or confer fresh execution authority.
    fn capability_transition_original_on(
        db: &rusqlite::Connection,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{contract::AgentRole, multi_agent::budget, web_mode};
        if db.is_autocommit() {
            return Err("root_capability_transition_snapshot_required".into());
        }
        if child.role != AgentRole::WebExecutor {
            return Err("root_capability_transition_role_invalid".into());
        }
        let original = original_root_capture_mode(db, &actor.root_run_id)?;
        web_mode::root::verify_birth(db, &actor.root_run_id, &original)?;
        let mode = web_mode::root::read(db, &actor.root_run_id)?
            .ok_or("root_capability_transition_original_protocol_missing")?;
        let bootstrap = mode.bootstrap_dispatch().unwrap_or(JsonValue::Null);
        if mode.mode() != web_mode::WebMode::Multi
            || mode.fact() != original.fact()
            || !matches!(bootstrap["schemaVersion"].as_i64(), Some(10..=15))
            || bootstrap["capabilityTransitionRule"]
                != "original_closed_web_capability_transition_advisory_v1"
        {
            return Err("root_capability_transition_original_protocol_missing".into());
        }
        budget::root::RootOwner::load_original(db, &actor.root_run_id)?
            .require_original_coordinator(db, actor)?;
        // Reader re-verifies its private seal, normal closed Web financial
        // proof, exact frozen plan/cap set and original message/terminal rows.
        // No rows are signed or adopted by this Root advisory consumer.
        let source = web_mode::capability_transition_receipt::read_original_on(db, actor, child)?
            .ok_or("root_capability_transition_original_receipt_missing")?;
        Ok(Self {
            kind: "capability-changed",
            semantic: json!({"role":"web_executor","rootRunId":actor.root_run_id,
                "scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,
                "assignmentId":child.assignment_id,"childRunId":child.run_id,
                "transitionMessageId":source.message_id(),
                "transitionReceiptId":source.receipt_id(),"originalClosedTransition":source.fact(),
                "advisoryOnly":true,"executionGrant":false,"dispatchGranted":false,
                "targetRequestsGranted":0,"additionalScopeGranted":false}),
            // The typed reader includes original physical hashes and validates
            // every source row on each read. Exclude mutable Root fees/TTL.
            rows: Vec::new(),
            review_refs: None,
            review_meta: None,
        })
    }

    fn verify_capability_transition_source(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.verify_capability_transition_source(&tx, context, actor);
        }
        let text = |key: &str| self.semantic[key].as_str()
            .ok_or("root_capability_transition_frame_invalid");
        let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id: text("assignmentId")?.into(),
            run_id: text("childRunId")?.into(),
            role: crate::agent_runtime::contract::AgentRole::WebExecutor,
        };
        let original = Self::capability_transition_original_on(db, actor, &child)?;
        if self.fact() != original.fact() || self.review_refs.is_some() || self.review_meta.is_some() {
            return Err("root_capability_transition_original_fact_changed".into());
        }
        let root = native_coordinator_root_context(context, actor);
        native_coordinator_tick_authority(db, &root, actor)
    }
}
