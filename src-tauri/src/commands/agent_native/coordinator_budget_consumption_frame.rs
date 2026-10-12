// A settled original Web call is a stable fact; current balances are not its identity.
impl NativeCoordinatorFrame {
    fn budget_consumption_on(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
        call_id: &str,
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
        Self::budget_consumption_original_on(db, context, actor, child, call_id)
    }

    // Historical source identity does not confer live worker authority.
    fn budget_consumption_original_on(
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
        call_id: &str,
    ) -> Result<Self, String> {
        use crate::agent_runtime::{
            contract::AgentRole,
            multi_agent::{attempts, budget},
            store,
        };
        if db.is_autocommit() {
            return Err("root_budget_consumption_snapshot_required".into());
        }
        native_budget_consumption_protocol_on(db, actor)?;
        if child.role != AgentRole::WebExecutor {
            return Err("root_budget_consumption_role_invalid".into());
        }
        let worker = attempts::current(db, actor, &child.assignment_id)?;
        let owner =
            budget::receipts::original_owner(db, &child.run_id, actor, &child.assignment_id)?;
        if owner.attempt_id() != worker.id || worker.child_run_id != child.run_id {
            return Err("root_budget_consumption_worker_conflict".into());
        }
        owner.verify(db)?;
        // This source is also reverified during its own legitimate Root SDK.
        // Fresh whole-Root financial admission belongs to the hook/Tick gate;
        // counting that Root's open dispatch here would reject its own bill.
        if owner.is_unresolved(db)? {
            return Err("root_budget_consumption_unsettled".into());
        }
        let (round, request_hash, raw): (i64, String, String) = db
            .query_row(
                "SELECT r.round,r.request_hash,r.receipt_json FROM agent_web_model_journal r
             JOIN agent_web_model_journal d ON d.call_id=r.call_id AND d.root_run_id=r.root_run_id
              AND d.assignment_id=r.assignment_id AND d.child_run_id=r.child_run_id
              AND d.lease_epoch=r.lease_epoch AND d.fencing_token=r.fencing_token
              AND d.round=r.round AND d.request_hash=r.request_hash AND d.phase='dispatch'
             WHERE r.call_id=?1 AND r.root_run_id=?2 AND r.assignment_id=?3
              AND r.child_run_id=?4 AND r.lease_epoch=?5 AND r.fencing_token=?6
              AND r.phase='received' AND d.receipt_json='{}'
              AND (SELECT count(*) FROM agent_web_model_journal x WHERE x.call_id=r.call_id)=2",
                params![
                    call_id,
                    actor.root_run_id,
                    child.assignment_id,
                    child.run_id,
                    actor.lease_epoch,
                    actor.fencing_token
                ],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(|_| "root_budget_consumption_original_receipt_required")?;
        let expected_call = store::stable_hash(
            &json!({"root":actor.root_run_id,
            "assignment":child.assignment_id,"child":child.run_id,"epoch":actor.lease_epoch,
            "fence":actor.fencing_token,"round":round,"requestHash":request_hash})
            .to_string(),
        );
        if round < 1 || expected_call != call_id {
            return Err("root_budget_consumption_call_binding_conflict".into());
        }
        let receipt: JsonValue = serde_json::from_str(&raw)
            .map_err(|_| "root_budget_consumption_original_usage_invalid")?;
        let canonical_receipt = receipt.to_string();
        if canonical_receipt != raw
            || receipt["usageReported"] != true
            || receipt.as_object().map(|v| v.len()) != Some(3)
        {
            return Err("root_budget_consumption_original_usage_invalid".into());
        }
        let number = |key: &str| {
            receipt["usage"][key]
                .as_i64()
                .filter(|v| *v >= 0)
                .ok_or("root_budget_consumption_original_usage_invalid")
        };
        let usage = store::UsageDelta {
            input_tokens: number("inputTokens")?,
            cached_input_tokens: number("cachedInputTokens")?,
            output_tokens: number("outputTokens")?,
            total_tokens: number("totalTokens")?,
            model_requests: number("modelRequests")?,
        };
        let response_hash = receipt["responseHash"]
            .as_str()
            .filter(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .ok_or("root_budget_consumption_original_usage_invalid")?;
        if usage.cached_input_tokens > usage.input_tokens
            || usage.input_tokens.checked_add(usage.output_tokens) != Some(usage.total_tokens)
            || usage.model_requests != 1
            || receipt["usage"] != usage.as_json()
        {
            return Err("root_budget_consumption_original_usage_invalid".into());
        }
        let source = format!(
            "web-model:{}:{}:{}",
            child.assignment_id, round, request_hash
        );
        budget::receipts::verify(
            db,
            actor,
            &child.assignment_id,
            &source,
            response_hash,
            &usage,
        )?;
        // The receipt verifier proves each canonical charge. Also reject an
        // extra consumed row under another key for this same original bill.
        let expected_charges = [
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.output_tokens,
            usage.model_requests,
        ]
        .iter()
        .filter(|amount| **amount > 0)
        .count() as i64;
        let actual_charges: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1
             AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 AND kind='consume'",
                params![
                    actor.root_run_id,
                    child.assignment_id,
                    owner.attempt_id(),
                    format!("{source}:receipt:{response_hash}")
                ],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if actual_charges != expected_charges {
            return Err("root_budget_consumption_original_charge_conflict".into());
        }
        let (tokens, requests) =
            native_budget_consumption_original_grant_on(db, actor, child, owner.attempt_id())?;
        let worker_id = owner.attempt_id();
        let rows = vec![
            NativeCoordinatorFrozenRows::capture(db,
                "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND kind='reserve' AND source_id=?4 ORDER BY rowid",
                &[&actor.root_run_id,&child.assignment_id,worker_id,&format!("assignment:{}",child.assignment_id)],
            )?,
            NativeCoordinatorFrozenRows::capture(
                db,
                "SELECT rowid,* FROM agent_web_model_journal WHERE call_id=?1 ORDER BY rowid",
                &[call_id],
            )?,
            NativeCoordinatorFrozenRows::capture(
                db,
                "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2
                 AND lease_attempt_id=?3 AND source_id=?4 ORDER BY rowid",
                &[
                    &actor.root_run_id,
                    &child.assignment_id,
                    worker_id,
                    &format!("{source}:receipt:{response_hash}"),
                ],
            )?,
            NativeCoordinatorFrozenRows::capture(
                db,
                "SELECT rowid,id,root_run_id,assignment_id,child_run_id,coordinator_epoch,
                 coordinator_fencing_token,lease_epoch,fencing_token,worker_id
                 FROM agent_assignment_attempts WHERE id=?1",
                &[worker_id],
            )?,
        ];
        let frame = Self {
            kind: "budget-consumption",
            semantic: json!({"callId":call_id,"rootRunId":actor.root_run_id,
                "assignmentId":child.assignment_id,"childRunId":child.run_id,
                "workerAttemptId":worker_id,"round":round,"requestHash":request_hash,
                "responseHash":response_hash,"usage":usage.as_json(),
                "originalTokens":tokens,"originalRequests":requests,
                "trigger":"capability_budget_state_changed","advisoryOnly":true,
                "executionGrant":false,"additionalScopeGranted":false,"targetRequestsGranted":0}),
            rows,
            review_refs: None,
            review_meta: None,
        };
        native_budget_consumption_require_original_fact(db, actor, &frame.fact())?;
        // Current execution authority is checked separately, never frozen into
        // the fact so lease renewals or another paid Root tick cannot rekey it.
        let root = native_coordinator_root_context(context, actor);
        native_coordinator_tick_authority(db, &root, actor)?;
        Ok(frame)
    }

    fn verify_budget_consumption_source(
        &self,
        db: &rusqlite::Connection,
        context: &AgentRunContext,
        actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.verify_budget_consumption_source(&tx, context, actor);
        }
        let text = |key: &str| {
            self.semantic[key]
                .as_str()
                .ok_or("root_budget_consumption_frame_invalid")
        };
        let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
            assignment_id: text("assignmentId")?.into(),
            run_id: text("childRunId")?.into(),
            role: crate::agent_runtime::contract::AgentRole::WebExecutor,
        };
        let actual = Self::budget_consumption_on(db, context, actor, &child, text("callId")?)?;
        if self.fact() != actual.fact() || self.review_refs.is_some() || self.review_meta.is_some()
        {
            return Err("root_budget_consumption_original_fact_changed".into());
        }
        Ok(())
    }
}

fn native_budget_consumption_protocol_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<(), String> {
    let declaration = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("root_budget_consumption_original_protocol_missing")?;
    if declaration.mode() != crate::agent_runtime::web_mode::WebMode::Multi
        || declaration.bootstrap_dispatch().unwrap_or(JsonValue::Null)["budgetConsumptionRule"]
            != "original_settled_web_call_advisory_v1"
    {
        return Err("root_budget_consumption_original_protocol_missing".into());
    }
    let proof = original_root_capture_mode(db, &actor.root_run_id)?;
    crate::agent_runtime::web_mode::root::verify_birth(db, &actor.root_run_id, &proof)
}

fn native_budget_consumption_require_original_fact(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    current: &JsonValue,
) -> Result<(), String> {
    let mut q=db.prepare("SELECT fact_json FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='request'")
        .map_err(|e|e.to_string())?;
    let rows = q
        .query_map([&actor.root_run_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let raw = row.map_err(|e| e.to_string())?;
        let fact: JsonValue = serde_json::from_str(&raw)
            .map_err(|_| "root_budget_consumption_original_request_invalid")?;
        let canonical_fact = fact.to_string();
        if canonical_fact != raw {
            return Err("root_budget_consumption_original_request_invalid".into());
        }
        let original = &fact["request"]["basis"]["changedFact"];
        if fact["request"]["basis"]["phase"] == "budget-consumption"
            && original["semantic"]["callId"] == current["semantic"]["callId"]
            && original != current
        {
            return Err("root_budget_consumption_original_fact_changed".into());
        }
    }
    Ok(())
}

// Original reserve rows remain immutable after settlement; never reconstruct a grant
// from the mutable zeroed assignment projection or mutate that projection.
fn native_budget_consumption_original_grant_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    worker: &str,
) -> Result<(i64, i64), String> {
    use crate::agent_runtime::multi_agent::budget;
    let projection:(i64,i64,String,String)=db.query_row(
        "SELECT reserved_tokens,reserved_requests,state,budget_settled_at FROM agent_assignments WHERE id=?1 AND coordinator_run_id=?2 AND role='web_executor'",
        params![child.assignment_id,actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
    ).map_err(|_|"executor_allocation_original_grant_conflict")?;
    if projection.3.is_empty() {
        return budget::model::original_web_grant(db, actor, &child.assignment_id);
    }
    budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    let original =
        crate::agent_runtime::multi_agent::attempts::current(db, actor, &child.assignment_id)?;
    if original.id != worker
        || original.lease_epoch != 1
        || original.child_run_id != child.run_id
        || projection.0 != 0
        || projection.1 != 0
        || !matches!(
            projection.2.as_str(),
            "running" | "waiting_review" | "completed"
        )
    {
        return Err("executor_allocation_original_grant_conflict".into());
    }
    let hard: (i64, i64) = db
        .query_row(
            "SELECT hard_token_budget,hard_request_budget FROM agent_runs WHERE id=?1",
            [&actor.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    let rows=db.prepare("SELECT dimension,amount,idempotency_key FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND kind='reserve' AND source_id=?4 AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests') ORDER BY dimension").map_err(|e|e.to_string())?
        .query_map(params![actor.root_run_id,child.assignment_id,worker,format!("assignment:{}",child.assignment_id)],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    let amount = |dimension: &str, limit: i64| -> Result<i64, String> {
        match (limit, rows.iter().find(|r| r.0 == dimension)) {
            (0, None) => Ok(0),
            (n, Some(r)) if n > 0 && r.1 > 0 && r.1 <= n => Ok(r.1),
            _ => Err("executor_allocation_original_grant_conflict".into()),
        }
    };
    let tokens = amount("model_input_tokens", hard.0)?;
    let requests = amount("model_requests", hard.1)?;
    if rows.len() != usize::from(tokens > 0) * 3 + usize::from(requests > 0)
        || rows.iter().any(|(dim, n, key)| {
            *n != if dim == "model_requests" {
                requests
            } else {
                tokens
            } || key != &format!("reserve:{}:{dim}", child.assignment_id)
        })
    {
        return Err("executor_allocation_original_grant_conflict".into());
    }
    // Compare the settlement projection against the original physical lifetime.
    let usage =
        budget::web_lifetime::child_usage_original(db, actor, &child.assignment_id, &child.run_id)?;
    let actual:(i64,i64,i64)=db.query_row("SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1 AND root_run_id=?2 AND assignment_id=?3",params![child.run_id,actor.root_run_id,child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    if actual
        != (
            usage.total_tokens,
            usage.cached_input_tokens,
            usage.model_requests,
        )
    {
        return Err("root_budget_consumption_settlement_conflict".into());
    }
    let owner = budget::receipts::original_owner(db, &child.run_id, actor, &child.assignment_id)?;
    owner.verify(db)?;
    for (dimension, expected) in budget::DIMENSIONS[..4].iter().zip([
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.output_tokens,
        usage.model_requests,
    ]) {
        let balance = budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            owner.attempt_id(),
            dimension,
        )?;
        if balance.reserved != 0 || balance.indeterminate != 0 || balance.consumed != expected {
            return Err("root_budget_consumption_settlement_conflict".into());
        }
    }
    Ok((tokens, requests))
}
