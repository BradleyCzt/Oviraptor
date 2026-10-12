/// Copy the working ledger back into the checkpoint. These are the counters a
/// resumed attempt must not get a second time.
fn native_sync(
    context: &AgentRunContext,
    state: &mut NativeAgentState,
    runtime: &AgentToolRuntime,
    queue: &[String],
) {
    state.pending_queue = queue.to_vec();
    // §9.1: only families with a covered ledger entry are recorded as covered;
    // having merely sent a request for a family is "worked", not "covered".
    let mut ledger_covered: Vec<String> = runtime
        .coverage
        .iter()
        .filter(|entry| entry.result == "covered" && !entry.request_record_ids.is_empty())
        .map(|entry| entry.family.clone())
        .collect();
    ledger_covered.sort();
    ledger_covered.dedup();
    state.covered_families = ledger_covered;
    state.observed_requests = runtime.requests.clone();
    state.discovery_rounds = runtime.discovery_rounds as i64;
    state.target_requests = runtime.target_requests as i64;
    state.confirmed_findings = runtime.confirmed_findings;
    state.verdict_keys = sorted_copy(&runtime.verdict_keys);
    state.coverage_evidence = runtime.coverage.clone();
    state.contract_attempts = runtime
        .contract_attempts
        .iter()
        .map(|(key, attempts)| (key.clone(), *attempts))
        .collect();
    state.contract_attempts.sort();
    state.contract_outcomes = runtime
        .contract_outcomes
        .iter()
        .map(|(key, outcome)| (key.clone(), outcome.clone()))
        .collect();
    state.contract_outcomes.sort();
    let completed = state.completed_contracts();
    state.completed_contract_keys = completed;
    let mut exhausted: Vec<String> = runtime
        .contract_attempts
        .iter()
        .filter(|(key, attempts)| {
            **attempts >= agent_evidence_contract_attempts(&context.evidence, key)
                && !runtime.contract_outcomes.contains_key(*key)
        })
        .map(|(key, _)| key.clone())
        .collect();
    exhausted.sort();
    state.exhausted_contract_keys = exhausted;
    state.budget_usage = serde_json::json!({
        "modelRequests": state.token_usage.model_requests,
        "targetRequests": runtime.target_requests,
        "discoveryRounds": runtime.discovery_rounds,
        "contractAttempts": runtime.contract_attempts.len(),
    });
}

/// §6.2: a stop must be provable, not assumed. The transport reports when the
/// cancel was asked for and when the connection was really closed; both land in
/// the run ledger before the terminal state is reduced.
fn note_transport_cancel(context: &AgentRunContext, client: &AgentModelClient) {
    let Some(observation) = client.take_cancel_observation() else {
        return;
    };
    append_runner_log(
        &context.log_path,
        &format!(
            "模型请求已中止：请求于 {} 停止，传输于 {} 关闭（{} 毫秒内落定）",
            observation.cancel_requested_at, observation.transport_closed_at, observation.settle_millis
        ),
    );
    let Some(run) = &context.run else { return };
    if let Err(error) = run.record(
        crate::agent_runtime::contract::AgentEventKind::TransportCancelled,
        serde_json::json!({
            "cancelRequestedAt": observation.cancel_requested_at,
            "transportClosedAt": observation.transport_closed_at,
            "settleMillis": observation.settle_millis,
            "initiator": match observation.initiator {
                crate::agent_runtime::model::CancelInitiator::Peer => "peer",
                crate::agent_runtime::model::CancelInitiator::Local => "task",
            },
        }),
    ) {
        // Nothing further is spent after a stop, so a failed diagnostic write is
        // reported rather than turned into a different terminal state.
        append_runner_log(&context.log_path, &format!("中止记录写入失败：{error}"));
    }
}

fn persistence_stop(context: &AgentRunContext, error: String) -> AgentTargetOutcome {
    append_runner_log(
        &context.log_path,
        &format!("本地记录写入失败，已停止后续模型与目标请求：{error}"),
    );
    // §5.2: the wording the user sees is fixed here, so no call site can report a
    // local write failure as an unexplained stop.
    AgentTargetOutcome::persistence_failure(format!(
        "本地记录失败，已停止以避免重复消耗（{error}）"
    ))
}

/// Stable tool codes, not substring guesses about human/model text. A local
/// storage failure, a revoked capability and an unknown target effect require
/// different user guidance, but all stop this executor before its next call.
fn agent_tool_terminal_outcome(view: &JsonValue) -> Option<AgentTargetOutcome> {
    let code = view.get("code").and_then(JsonValue::as_str)?;
    let outcome = match code {
        "evidence_write_failed" | "finding_persist_failed" => AgentTargetOutcome::persistence_failure(format!(
            "本地记录失败，已停止以避免重复消耗（{}）；已占用请求不退回，先核对执行日志，不自动重放",
            value_first(view, &["error"]),
        )),
        terminal_code::REQUEST_RECONCILIATION_REQUIRED => AgentTargetOutcome::Incomplete(AgentStop::new(
            terminal_code::REQUEST_RECONCILIATION_REQUIRED,
            "请求账本与执行结果需要人工核对，已停止此执行分支及后续请求；预算保留，不自动重放。请核对目标日志与本地证据，不能把未决请求视为成功或未发送；当前没有忽略未决结果后继续的入口",
        )),
        "request_budget_exhausted" => AgentTargetOutcome::Limited(AgentStop::new(
            AGENT_STOP_HARD_REQUESTS, "目标请求共享预算已达到硬上限，停止此执行分支；不是本地存储故障，不自动增加预算",
        )),
        "root_supervision_request_capacity_exhausted" => AgentTargetOutcome::Limited(AgentStop::new(
            AGENT_STOP_HARD_REQUESTS, "后续 Root 监督与只读角色所需的模型请求容量不足，在发送下一条 HTTP 前停止；已付请求保留，不自动增加预算",
        )),
        "wall_time_budget_exhausted" => AgentTargetOutcome::Limited(AgentStop::new(
            terminal_code::HARD_WALL_TIME_BUDGET, "根任务共享墙钟预算已耗尽，停止此执行分支；已发送请求的成本保留，不自动重置时钟或增加预算",
        )),
        "cancelled" => AgentTargetOutcome::Cancelled,
        "agent_attempt_not_active" | "tool_execution_surface_denied" | "tool_execution_plan_unavailable"
        | "tool_authorization_unavailable" | "tool_run_not_found" | "coordinator_not_executable"
        | "tool_policy_or_role_denied" | "tool_capability_or_fencing_denied" | "tool_identity_binding_denied" => AgentTargetOutcome::Incomplete(AgentStop::new(
            terminal_code::EXECUTION_AUTHORIZATION_DENIED,
            format!("当前执行权限、任务状态或身份绑定无法核验，已停止此执行分支；请检查授权与任务状态，不自动续期或扩大权限（{code}）"),
        )),
        terminal_code::EVIDENCE_INTEGRITY => AgentTargetOutcome::Failed(AgentStop::new(
            AGENT_STOP_EVIDENCE_INTEGRITY, "请求账本绑定或计数完整性校验失败，已停止此执行分支；保留现有记录并人工检查，不自动修复计数或重放请求",
        )),
        _ => return None,
    };
    Some(outcome)
}

/// §3.6 and §5.2 keep their own wording: an incompatible checkpoint asks the user to
/// re-run, a failed local write says the run stopped to avoid double spend.
fn rejection_outcome(
    context: &AgentRunContext,
    rejection: &NativeStateRejection,
) -> AgentTargetOutcome {
    append_runner_log(&context.log_path, &rejection.message());
    if rejection.kind == NativeStateRejectionKind::PersistenceFailed {
        return AgentTargetOutcome::persistence_failure(rejection.message());
    }
    AgentTargetOutcome::resume_incompatible(rejection.message())
}

fn native_commit(
    context: &AgentRunContext,
    state: &mut NativeAgentState,
    runtime: &AgentToolRuntime,
    queue: &[String],
) -> Result<(), String> {
    native_sync(context, state, runtime, queue);
    // §5.2: the round is only over once the checkpoint and its snapshot mirror are
    // both durable. Anything else stops the attempt before more budget is spent.
    state.persist(&context.db_path, &context.scan_id, &context.target_url)?;
    if let Some(run) = &context.run {
        run.checkpoint(state)?;
    }
    Ok(())
}

fn sorted_copy(values: &std::collections::HashSet<String>) -> Vec<String> {
    let mut rows = values.iter().cloned().collect::<Vec<_>>();
    rows.sort();
    rows
}

/// The single terminal exit. §15: the state is derived from the ledger, and only
/// `Incomplete` — the recoverable state — leaves the checkpoint resumable; every
/// other terminal state writes its reason so a resume can never re-spend it.
fn native_exit(
    context: &AgentRunContext,
    state: &mut NativeAgentState,
    runtime: &AgentToolRuntime,
    queue: &[String],
    reason: &str,
    terminal_code: &'static str,
    bounded: bool,
) -> AgentTargetOutcome {
    native_sync(context, state, runtime, queue);
    let ledger = agent_ledger_completion(runtime, state, terminal_code);
    let summary = format!("{reason}；{}", ledger.summary);
    let ledger_closed = queue.is_empty() && ledger.uncovered_families.is_empty();
    let has_evidence = runtime.confirmed_findings > 0
        || !runtime.families.is_empty()
        || !runtime.requests.is_empty();
    let outcome = if ledger_closed {
        AgentTargetOutcome::Completed(AgentCompletion {
            summary: summary.clone(),
            ..ledger
        })
    } else if bounded && has_evidence {
        // §15: a plan boundary with usable evidence and named gaps is bounded
        // completion; anything else that still owes work stays resumable.
        AgentTargetOutcome::BoundedCompleted(AgentCompletion {
            summary: summary.clone(),
            ..ledger
        })
    } else {
        AgentTargetOutcome::Incomplete(AgentStop::new(terminal_code, format!(
            "{summary}；剩余 {} 项未完成，可继续同一尝试",
            queue.len()
        )))
    };
    state.terminal_reason = if matches!(outcome, AgentTargetOutcome::Incomplete(_)) {
        String::new()
    } else {
        reason.to_string()
    };
    // §5.2: the terminal boundary itself must be durable. If it is not, the attempt
    // is reported as a persistence failure rather than as a finish it cannot prove.
    let boundary = state
        .persist(&context.db_path, &context.scan_id, &context.target_url)
        .and_then(|_| match &context.run {
            Some(run) => run.checkpoint(state),
            None => Ok(()),
        })
        .and_then(|_| persist_agent_coverage(context, runtime, state, &summary));
    if let Err(error) = boundary {
        return persistence_stop(context, error);
    }
    outcome
}
