/// Every post-bootstrap exit uses the same atomic root/directive closure. A
/// persistence failure must remain visible alongside, not instead of, its cause.
fn executor_settlement_outcome(outcome: &AgentTargetOutcome, error: &str) -> AgentTargetOutcome {
    let (stage, cause) = match error.strip_prefix("executor_followup_failed:").and_then(|value| value.split_once(':')) {
        Some((stage, cause)) => (format!("executor_followup_failed:{stage}"), cause),
        None => ("executor_settlement_failed".to_string(), error),
    };
    let reason = crate::agent_runtime::secrets::redact_text_with(
        &format!(
            "{stage}:{cause}; executor_code={}; executor_reason={}",
            outcome.terminal_code(),
            outcome.detail()
        ),
        None,
    );
    if matches!(
        cause,
        "budget_indeterminate_requires_reconciliation"
            | "budget_history_requires_reconciliation"
            | "budget_reservations_require_settlement"
    ) {
        AgentTargetOutcome::Incomplete(AgentStop::new(
            terminal_code::REQUEST_RECONCILIATION_REQUIRED,
            reason,
        ))
    } else {
        AgentTargetOutcome::failed(reason)
    }
}

fn finalize_agent_target(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    outcome: AgentTargetOutcome,
) -> AgentTargetOutcome {
    let result = db::open(&context.db_path)
        .and_then(|connection| finish_coordinator_run(&connection, lease, &outcome));
    match result {
        Ok(()) => outcome,
        Err(error) if error == "budget_wall_time_exhausted" => {
            let limited = AgentTargetOutcome::Limited(AgentStop::new(
                terminal_code::HARD_WALL_TIME_BUDGET,
                "根任务共享墙钟预算已耗尽，已停止；保留调用成本与未决回执，不重置时钟或自动重放",
            ));
            match db::open(&context.db_path)
                .and_then(|db| finish_coordinator_run(&db, lease, &limited))
            {
                Ok(()) => limited,
                Err(persist) => coordinator_finalize_failure(context, &limited, &persist),
            }
        }
        Err(error) => coordinator_finalize_failure(context, &outcome, &error),
    }
}

fn coordinator_finalize_failure(
    context: &AgentRunContext,
    outcome: &AgentTargetOutcome,
    error: &str,
) -> AgentTargetOutcome {
    if matches!(
        error,
        "budget_indeterminate_requires_reconciliation"
            | "budget_history_requires_reconciliation"
            | "budget_reservations_require_settlement"
    ) {
        append_runner_log(&context.log_path, error);
        return AgentTargetOutcome::Incomplete(AgentStop::new(terminal_code::REQUEST_RECONCILIATION_REQUIRED,
            "根任务预算仍有预留、未决成本或缺失的审计来源，不能标记完成；原调用与回执已保留，请先核对。没有自动退款、重放或忽略未决成本后继续的授权"));
    }
    let reason = crate::agent_runtime::secrets::redact_text_with(
        &format!(
            "coordinator_finalize_failed:{error}; original_code={}; original_reason={}",
            outcome.terminal_code(),
            outcome.detail()
        ),
        None,
    );
    append_runner_log(&context.log_path, &reason);
    AgentTargetOutcome::persistence_failure(reason)
}
