// Original sealed target cost is a stable fact; current authority is separate.
impl NativeCoordinatorFrame {
    fn http_budget_consumption_on(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
        source: &str,
    ) -> Result<Self, String> {
        crate::agent_runtime::multi_agent::scheduler::verify_original_scheduled_child(
            db, actor, child,
        )?;
        context
            .supervision
            .as_ref()
            .ok_or("worker_supervisor_stopped")?
            .check_run(
                db,
                &context.db_path,
                &actor.scan_id,
                actor.attempt_number,
                &actor.target_key,
                &child.run_id,
            )?;
        Self::http_budget_consumption_original_on(db, context, actor, child, source)
    }

    // Used by completed replay only after the original executor terminal proof.
    fn http_budget_consumption_original_on(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
        source: &str,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{attempts, budget},
            web_mode::http_budget_storage,
        };
        if db.is_autocommit() {
            return Err("root_http_budget_snapshot_required".into());
        }
        if child.role != AgentRole::WebExecutor {
            return Err("root_http_budget_role_invalid".into());
        }
        if !native_http_budget_protocol_enabled_on(db, &actor.root_run_id)? {
            return Err("root_http_budget_original_protocol_missing".into());
        }
        budget::root::RootOwner::load_original(db, &actor.root_run_id)?
            .require_original_coordinator(db, actor)?;
        let receipt = http_budget_storage::read_original_on(db, actor, source)?
            .ok_or("root_http_budget_original_receipt_missing")?;
        let fact = receipt.fact();
        if fact["claim"]["runId"] != child.run_id {
            return Err("root_http_budget_original_scope_conflict".into());
        }
        let worker = attempts::current(db, actor, &child.assignment_id)?;
        if worker.child_run_id != child.run_id {
            return Err("root_http_budget_original_worker_conflict".into());
        }
        // Storage re-verifies the private seal, original headers and the exact
        // three original target cost rows. Never sign/adopt a saved claim here.
        let (tool,identity,received):(String,String,String)=db.query_row(
            "SELECT tool_name,identity_handle,received_at FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=?3 AND ordinal=?4 AND run_id=?5 AND invocation_id=?6 AND request_index=?7 AND request_hash=?8 AND attempt_number=?9 AND response_status=?10 AND received_at<>''",
            params![actor.scan_id,actor.target_key,fact["claim"]["budgetAttempt"].as_i64(),fact["claim"]["ordinal"].as_i64(),child.run_id,fact["claim"]["invocationId"].as_str(),fact["claim"]["requestIndex"].as_i64(),fact["claim"]["requestHash"].as_str(),actor.attempt_number,fact["status"].as_i64()],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).map_err(|_|"root_http_budget_original_claim_missing")?;
        let rows=vec![NativeCoordinatorFrozenRows::capture(db,
            "SELECT * FROM main.agent_http_budget_receipts WHERE root_run_id=?1 AND source_id=?2",
            &[&actor.root_run_id,source])?];
        let frame = Self {
            kind: "http-budget-consumption",
            semantic: json!({"receiptId":receipt.receipt_id(),"sourceId":source,
                "rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,
                "targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,
                "assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":worker.id,
                "claim":fact["claim"],"status":fact["status"],"receivedAt":received,"tool":tool,"identityHandle":identity,
                "targetRequests":1,"protocol":"original_received_http_target_cost_advisory_v1",
                "trigger":"capability_budget_state_changed","advisoryOnly":true,"executionGrant":false,
                "additionalScopeGranted":false,"targetRequestsGranted":0}),
            rows,
            review_refs: None,
            review_meta: None,
        };
        native_http_budget_require_original_fact_on(db, actor, &frame.fact())?;
        let root = native_coordinator_root_context(context, actor);
        native_coordinator_tick_authority(db, &root, actor)?;
        Ok(frame)
    }

    fn verify_http_budget_consumption_source(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.verify_http_budget_consumption_source(&tx, context, actor);
        }
        let text = |key: &str| {
            self.semantic[key]
                .as_str()
                .ok_or("root_http_budget_frame_invalid")
        };
        let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id: text("assignmentId")?.into(),
            run_id: text("childRunId")?.into(),
            role: crate::agent_runtime::contract::AgentRole::WebExecutor,
        };
        let actual =
            Self::http_budget_consumption_on(db, context, actor, &child, text("sourceId")?)?;
        if actual.fact() != self.fact() || self.review_refs.is_some() || self.review_meta.is_some()
        {
            return Err("root_http_budget_original_fact_changed".into());
        }
        Ok(())
    }
}

fn native_http_budget_protocol_enabled_on(
    db: &rusqlite::Connection,
    root: &str,
) -> Result<bool, String> {
    if db.is_autocommit() {
        return Err("root_http_budget_snapshot_required".into());
    }
    let original = original_root_capture_mode(db, root)?;
    let declaration = crate::agent_runtime::web_mode::root::read(db, root)?
        .ok_or("root_http_budget_original_protocol_missing")?;
    if original.fact() != declaration.fact()
        || declaration.mode() != crate::agent_runtime::web_mode::WebMode::Multi
    {
        return Err("root_http_budget_original_protocol_missing".into());
    }
    crate::agent_runtime::web_mode::root::verify_birth(db, root, &original)?;
    // A verified original Native birth may predate bootstrap protocols. It
    // declares no HTTP advisory; physical HTTP admission remains independent.
    let Some(bootstrap) = declaration.bootstrap_dispatch() else {
        return Ok(false);
    };
    if bootstrap.get("httpBudgetConsumptionRule").is_none()
        && bootstrap["schemaVersion"]
            .as_i64()
            .is_some_and(|v| (1..=7).contains(&v))
    {
        return Ok(false);
    }
    crate::agent_runtime::web_mode::http_budget_receipt::require_protocol_on(db, root, &original)?;
    Ok(true)
}

fn native_http_budget_require_original_fact_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    current: &JsonValue,
) -> Result<(), String> {
    let rows=db.prepare("SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'").map_err(|e|e.to_string())?
        .query_map([&actor.root_run_id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    for raw in rows {
        let saved: JsonValue =
            serde_json::from_str(&raw).map_err(|_| "root_http_budget_original_request_invalid")?;
        let canonical = saved.to_string();
        if canonical != raw {
            return Err("root_http_budget_original_request_invalid".into());
        }
        let original = &saved["request"]["basis"]["changedFact"];
        if saved["request"]["basis"]["phase"] == "http-budget-consumption"
            && original["semantic"]["sourceId"] == current["semantic"]["sourceId"]
            && original != current
        {
            return Err("root_http_budget_original_fact_changed".into());
        }
    }
    Ok(())
}
