/// What one tool execution produced (§6.1). `model_view` is the only part that
/// may enter the conversation; `artifact_id` points at the raw bytes, and
/// `progress` carries the facts the ledger counts without any payload at all.
struct ToolExecutionResult {
    model_view: JsonValue,
    artifact_id: String,
    invocation_id: String,
}

fn agent_execute_tool_at(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    name: &str,
    arguments: &JsonValue,
    invocation_id: &str,
) -> ToolExecutionResult {
    runtime.set_invocation(invocation_id);
    agent_execute_tool_with_current_invocation(context, runtime, name, arguments)
}

/// A call that opens its own invocation, for direct (single-tool) drivers.
#[cfg(test)]
fn agent_execute_tool(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    name: &str,
    arguments: &JsonValue,
) -> ToolExecutionResult {
    let invocation_id = runtime.begin_invocation(context.attempt_number);
    runtime.set_invocation(&invocation_id);
    agent_execute_tool_with_current_invocation(context, runtime, name, arguments)
}

fn agent_execute_tool_with_current_invocation(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    name: &str,
    arguments: &JsonValue,
) -> ToolExecutionResult {
    let mut executed = agent_execute_tool_inner(context, runtime, name, arguments);
    if let Some(stop) = &runtime.target_request_stop {
        // Persist the failure at the invocation's top level even if a fan-out
        // adapter swallowed it into its partial-result array.
        if executed != *stop {
            let partial = executed;
            executed = serde_json::json!({
                "code":stop["code"], "error":stop["error"], "executionStop":stop,
                "partialResult":partial,
            });
            // Existing evidence links remain addressable by the invocation
            // ledger/UI; retaining evidence does not make the tool successful.
            for key in ["rawArtifactId", "responseDifferenceArtifactId"] {
                if let Some(reference) = executed["partialResult"][key]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                {
                    executed[key] = JsonValue::String(reference);
                }
            }
        }
    }
    ToolExecutionResult {
        // The HTTP path already projected its payload because the audit record
        // embeds the same view; re-projecting is a no-op on already-marked text.
        model_view: agent_model_view(executed, runtime),
        artifact_id: runtime.last_artifact_id.clone(),
        invocation_id: runtime.current_invocation.clone(),
    }
}

fn agent_execute_tool_inner(
    context: &AgentRunContext,
    runtime: &mut AgentToolRuntime,
    name: &str,
    arguments: &JsonValue,
) -> JsonValue {
    runtime.last_artifact_id.clear();
    if let Some(stop) = &runtime.target_request_stop {
        return stop.clone();
    }
    if let Err(rejection) = agent_validate_arguments(name, arguments) {
        runtime.note_argument_error(&rejection.fingerprint);
        return serde_json::json!({
            "error": rejection.message,
            "code": "invalid_arguments",
            "schemaErrors": rejection.errors,
            "errorFingerprint": rejection.fingerprint,
        });
    }
    runtime.clear_argument_error();
    // A stopped attempt has a distinct, auditable cancellation outcome. No
    // adapter is reachable from this branch; authorization still runs on
    // every tool invocation that could execute.
    if runtime.cancelled() {
        return agent_tool_error("任务已暂停或取消，不再向目标发请求", "cancelled");
    }
    // The model-facing tool list is not an authorization boundary. Re-read the
    // live assignment and per-tool lease before *every* call, including local
    // evidence writes. A revoked/expired child, stale coordinator or read-only
    // role must never reach a target adapter even if it retained an old context.
    if let Err(code) = agent_authorize_tool(context, name) {
        return agent_tool_error("当前 run 没有此工具的有效权限", code);
    }
    if let Err(reason) = agent_single_require_fresh(context) {
        return agent_http_claim_error(reason);
    }
    // Repository reads/evidence writes do not spend Web target requests and
    // must not depend on a fabricated Web execution plan or Web stop state.
    if agent_is_source_tool(name) {
        return agent_execute_source_tool(context, name, arguments);
    }
    // Rehydrate a denial from durable state before reaching any target adapter.
    if runtime.protected_stop.is_none() {
        let persisted = db::open(&context.db_path)
            .and_then(|db| agent_protection_stop_on(&db, context));
        match persisted {
            Ok(stop) => runtime.protected_stop = stop,
            Err(_) => return agent_tool_error("无法核验目标的持久保护状态", "tool_authorization_unavailable"),
        }
    }
    if let Some(stop) = &runtime.protected_stop {
        // Once the target is protected, further requests are not authorized.
        return serde_json::json!({"code": "protected", "stop": stop});
    }
    // Anything the model asked for that is not one of our tools is refused before
    // a payload could be echoed back.

    if runtime.target_requests >= agent_target_request_ceiling(context) {
        return agent_tool_error(
            "目标请求总数已达到本轮硬上限，请调用 finish_target 收口",
            "request_budget_exhausted",
        );
    }
    if name != "finish_target" {
        if let Err(reason) = agent_target_budget_preflight(context, runtime, name) {
            return agent_http_claim_error(reason);
        }
        if let Err(reason) = native_coordinator_budget_consumption_before_action(context) {
            return agent_http_claim_error(reason);
        }
        if let Err(reason) = native_coordinator_http_budget_consumption_before_action(context) {
            return agent_http_claim_error(reason);
        }
    }
    match name {
        "inspect_evidence" => agent_tool_inspect_evidence(context, arguments),
        "replay_http" => agent_tool_replay_http(context, runtime, arguments),
        "compare_identities" => agent_tool_compare_identities(context, runtime, arguments),
        "targeted_discovery" => agent_tool_targeted_discovery(context, runtime, arguments),
        "browser_action" => agent_tool_browser_action(context, runtime, arguments),
        "record_hypothesis_result" => {
            agent_tool_record_hypothesis_result(context, runtime, arguments)
        }
        "finish_target" => agent_tool_finish_target(context, runtime, arguments),
        other => agent_tool_error(&format!("未知工具 {other}"), "unknown_tool"),
    }
}

fn agent_require_frozen_web_plan(
    connection: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<(), &'static str> {
    if !matches!(context.execution_plan.schema_version, 1 | 2) {
        return Err("tool_execution_surface_denied");
    }
    // The frozen attempt plan, rather than a chat directive or an in-memory
    // capability advertisement, owns the execution boundary. Historical V1
    // plans had only Web tools; V2 must explicitly say web_only. A missing,
    // malformed, changed or future surface cannot reach any adapter.
    let stored = crate::agent_runtime::store::attempt_plan(
        connection,
        &context.scan_id,
        context.attempt_number,
        &context.target_url,
    )
    .ok_or("tool_execution_plan_unavailable")?;
    let frozen = AgentExecutionPlan::from_json(&stored).ok_or("tool_execution_surface_denied")?;
    if frozen != context.execution_plan
        || frozen.attempt_number != context.attempt_number
        || frozen.target_url != context.target_url
    {
        return Err("tool_execution_surface_denied");
    }
    Ok(())
}

fn agent_authorize_tool(context: &AgentRunContext, name: &str) -> Result<(), &'static str> {
    let connection = db::open(&context.db_path).map_err(|_| "tool_authorization_unavailable")?;
    agent_authorize_tool_on(&connection, context, name)
}

fn agent_authorize_tool_on(
    connection: &rusqlite::Connection,
    context: &AgentRunContext,
    name: &str,
) -> Result<(), &'static str> {
    if let Some(ticket) = &context.supervision {
        let run = context.run.as_ref().ok_or("tool_run_not_found")?;
        ticket
            .check_run(
                connection,
                &context.db_path,
                &context.scan_id,
                context.attempt_number,
                &context.target_url,
                &run.run_id,
            )
            .map_err(|_| "worker_supervision_denied")?;
    }
    if context.target_url.starts_with("source:") {
        return agent_native_source_tool_authority(connection, context, name).map(|_| ());
    }
    // Historical source-view unit fixtures are not production Web grants.
    #[cfg(not(test))]
    if agent_is_source_tool(name) {
        return Err("source_tool_requires_native_source_assignment");
    }
    let Some(run) = &context.run else {
        // Directly constructed test contexts have no ledger. A production tool
        // call without one is unaccountable and must fail closed.
        #[cfg(test)]
        {
            return Ok(());
        }
        #[cfg(not(test))]
        {
            return Err("tool_run_not_found");
        }
    };
    if run.db_path != context.db_path {
        return Err("tool_authorization_unavailable");
    }
    agent_require_frozen_web_plan(connection, context)?;
    let identity: Option<(String, String)> = connection
        .query_row(
            "SELECT orchestration_policy,role FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4",
            params![run.run_id, context.scan_id, context.attempt_number, context.target_url],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| "tool_authorization_unavailable")?;
    let Some((policy, role)) = identity else {
        return Err("tool_run_not_found");
    };
    // A still-valid run or capability lease must not outlive its scan attempt.
    // Check before the single-coordinator fast path as well as multi-agent
    // dispatch; the per-request send boundary calls this function again.
    crate::agent_runtime::multi_agent::lease::require_active_attempt(
        connection,
        &context.scan_id,
        context.attempt_number,
    )
    .map_err(|_| "agent_attempt_not_active")?;
    if policy != "multi" {
        return if policy == "single" && role == "coordinator" {
            crate::agent_runtime::multi_agent::lease::require_open_coordinator(
                connection,
                &context.scan_id,
                context.attempt_number,
                &context.target_url,
                &run.run_id,
            )
            .map_err(|_| "coordinator_not_executable")?;
            agent_require_bound_identities(connection, context)
        } else {
            Err("tool_policy_or_role_denied")
        };
    }
    // Only the WebExecutor currently dispatches through this model
    // loop. Adding another role requires its own explicit broker policy, not a
    // broad 'any child' rule. The lane and capability are checked in storage.
    crate::agent_runtime::multi_agent::attempts::require_live_for_run(connection, &run.run_id)
        .map_err(|_| "tool_capability_or_fencing_denied")?;
    let authorized: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs r \
             JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id \
             JOIN agent_coordinator_leases c ON c.root_run_id=a.coordinator_run_id \
               AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url \
             JOIN agent_runs root ON root.id=c.root_run_id AND root.root_run_id=root.id AND root.role='coordinator' \
               AND root.scan_id=c.scan_id AND root.attempt_number=c.attempt_number AND root.target_url=c.target_key \
               AND root.status IN ('prepared','running') AND root.cancel_requested_at='' AND r.root_run_id=root.id \
             JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id \
               AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane \
             JOIN agent_capability_leases p ON p.assignment_id=a.id AND p.child_run_id=r.id \
               AND p.root_run_id=a.coordinator_run_id AND p.capability=?2 \
             WHERE r.id=?1 AND r.status='running' AND r.cancel_requested_at='' AND r.role='web_executor' AND r.lane='target_touching' \
               AND a.role=r.role AND a.lane=r.lane AND a.target_key=r.target_url AND a.state='running' \
               AND a.lease_epoch=c.lease_epoch AND a.fencing_token=c.fencing_token \
               AND a.lease_expires_at>datetime('now','localtime') \
               AND p.lease_epoch=c.lease_epoch AND p.fencing_token=c.fencing_token \
               AND p.revoked_at='' AND p.lease_expires_at>datetime('now','localtime') \
               AND c.lease_expires_at>datetime('now','localtime'))",
            params![run.run_id, name],
            |row| row.get(0),
        )
        .map_err(|_| "tool_authorization_unavailable")?;
    if !authorized {
        return Err("tool_capability_or_fencing_denied");
    }
    agent_require_bound_identities(connection, context)
}

fn agent_require_bound_identities(
    connection: &rusqlite::Connection,
    context: &AgentRunContext,
) -> Result<(), &'static str> {
    // Preparation is not a permanent credential lease. Revalidate the entire
    // identity set for Single and Multi before every brokered invocation and
    // at the request send boundary, including anonymous/authenticated handles.
    let (_, session_ids) = crate::auth_session::validated_scan_identities(
        connection,
        &context.scan_id,
        &context.target_url,
    )
    .map_err(|_| "tool_identity_binding_denied")?;
    let mut bound = context
        .identities
        .iter()
        .map(|identity| {
            (
                identity.key.clone(),
                identity.session_id.clone(),
                identity.anonymous,
            )
        })
        .collect::<Vec<_>>();
    bound.sort();
    let expected = if session_ids.is_empty() {
        vec![("anonymous".to_string(), None, true)]
    } else {
        let mut expected = session_ids
            .into_iter()
            .map(|id| (id.clone(), Some(id), false))
            .collect::<Vec<_>>();
        // The fixed anonymous control carries no credential. It may accompany
        // the entire validated account set; it cannot replace any account or
        // authorize arbitrary aliases, duplicated handles or partial bindings.
        let control = ("anonymous".to_string(), None, true);
        if bound.contains(&control) {
            expected.push(control);
            expected.sort();
        }
        expected
    };
    if bound != expected {
        return Err("tool_identity_binding_denied");
    }
    Ok(())
}

/// Requests against the target are bounded by the plan, not by model turns.
fn agent_target_request_ceiling(context: &AgentRunContext) -> usize {
    let plan = &context.execution_plan;
    let ceiling = plan.hard_model_requests.max(1).saturating_mul(4).min(400) as usize;
    // Dedicated public capture is not part of the generic tool runtime's
    // in-memory count. Claimed/unknown requests still consume its shared cap.
    let claimed = db::open(&context.db_path).and_then(|connection| {
        let tx = connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let attempts = crate::agent_runtime::target_requests::budget_attempts(
            &tx,
            &context.scan_id,
            context.attempt_number,
        )?;
        let usage = crate::agent_runtime::target_requests::external_usage(
            &tx,
            &context.scan_id,
            &attempts,
            &context.target_url,
        )?;
        let authorization = crate::agent_runtime::target_requests::authorization_usage(
            &tx,
            &context.scan_id,
            &attempts,
            &context.target_url,
        )?;
        usize::try_from(
            usage.received + usage.unresolved + authorization.received + authorization.unresolved,
        )
        .map_err(|e| e.to_string())
    });
    match claimed {
        Ok(count) => ceiling.saturating_sub(count),
        Err(_) => 0,
    }
}
