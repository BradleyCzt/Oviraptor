fn settle_child_usage(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
) -> Result<(), String> {
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(connection, lease)?;
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| format!("无法锁定子智能体预算结算：{error}"))?;
    settle_child_usage_in_transaction(&transaction, lease, child, usage)?;
    transaction
        .commit()
        .map_err(|error| format!("无法提交子智能体预算结算：{error}"))
}

fn settle_child_usage_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
) -> Result<(), String> {
    settle_child_usage_for_status(transaction, lease, child, usage, "running")
}

// Only the receipt-verified, local readonly completion path may pass `paused`.
// Never transiently mark a paused worker as running just to settle its costs.
fn settle_child_usage_for_status(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    run_status: &str,
) -> Result<(), String> {
    settle_child_usage_for_status_on(transaction, lease, child, usage, run_status, false)
}

fn settle_original_web_child_usage(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<(AgentTokenUsage, Vec<crate::agent_runtime::execution_owner::NativeInvocationOwner>), String> {
    let (usage, idle) = crate::agent_runtime::multi_agent::budget::model::settle_original_web_receipt(
        transaction, lease, &child.assignment_id, &child.run_id,
    )?;
    settle_child_usage_for_status_on(transaction, lease, child, &usage, "running", true)?;
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(transaction, &lease.root_run_id)?
        .require_original_coordinator(transaction, lease)?;
    let mode = crate::agent_runtime::web_mode::root::read(transaction, &lease.root_run_id)?
        .ok_or("web_model_settlement_original_mode_missing")?;
    if mode.new_evidence_supervision().is_none() {
        return Err("web_model_settlement_original_protocol_missing".into());
    }
    let verified = crate::agent_runtime::multi_agent::budget::web_lifetime::child_usage_original(
        transaction, lease, &child.assignment_id, &child.run_id,
    )?;
    if verified != usage { return Err("web_model_settlement_usage_conflict".into()); }
    Ok((usage, idle))
}

fn settle_child_usage_for_status_on(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    run_status: &str,
    original_web_receipt: bool,
) -> Result<(), String> {
    if !original_web_receipt {
        crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(transaction, lease)?;
    }
    let (reserved_tokens, reserved_requests, budget_settled_at): (i64, i64, String) = transaction
        .query_row(
            "SELECT reserved_tokens,reserved_requests,budget_settled_at FROM agent_assignments \
             WHERE id=?1 AND child_run_id=?2 AND lease_epoch=?3 AND fencing_token=?4",
            params![
                child.assignment_id,
                child.run_id,
                lease.lease_epoch,
                lease.fencing_token
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| format!("无法读取子智能体预算预留：{error}"))?;
    let spent_tokens = usage.total_tokens.max(0);
    let spent_requests = usage.model_requests.max(0);
    if !budget_settled_at.is_empty() {
        let stored: (i64, i64, i64) = transaction
            .query_row(
                "SELECT used_tokens,used_cached_tokens,used_requests FROM agent_runs WHERE id=?1 AND root_run_id=?2",
                params![child.run_id, lease.root_run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|error| format!("无法确认子智能体预算重放：{error}"))?;
        if stored
            != (
                spent_tokens,
                usage.cached_input_tokens.max(0),
                spent_requests,
            )
        {
            return Err("child_budget_settlement_replay_conflict".into());
        }
        settle_child_model_usage_on(transaction, lease, child, usage, original_web_receipt)?;
        return Ok(());
    }
    let settled = transaction
        .execute(
            "UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens-?1,reserved_requests=reserved_requests-?2,\
             spent_tokens=spent_tokens+?3,spent_requests=spent_requests+?4,updated_at=datetime('now','localtime') \
             WHERE root_run_id=?5 AND lease_epoch=?6 AND fencing_token=?7 AND reserved_tokens>=?1 AND reserved_requests>=?2 \
             AND (total_tokens=0 OR ?3<=?1) AND (total_requests=0 OR ?4<=?2) \
             AND (total_tokens=0 OR spent_tokens+reserved_tokens-?1+?3<=total_tokens) \
             AND (total_requests=0 OR spent_requests+reserved_requests-?2+?4<=total_requests)",
            params![
                reserved_tokens,
                reserved_requests,
                spent_tokens,
                spent_requests,
                lease.root_run_id,
                lease.lease_epoch,
                lease.fencing_token
            ],
        )
        .map_err(|error| format!("无法结算多智能体预算账本：{error}"))?;
    if settled != 1 {
        return Err("child_budget_settlement_exceeded_or_stale".into());
    }
    // Both ledgers remain in this transaction. A real summary writer failure
    // must remain visible; vector rejection rolls its tentative update back.
    settle_child_model_usage_on(transaction, lease, child, usage, original_web_receipt)
    .map_err(|error| {
        if error == "budget_usage_exceeds_reservation" {
            "child_budget_settlement_exceeded_or_stale".into()
        } else {
            error
        }
    })?;
    let run_settled = transaction
        .execute(
            "UPDATE agent_runs SET used_tokens=?1,used_cached_tokens=?2,used_requests=?3,\
             updated_at=datetime('now','localtime') WHERE id=?4 AND root_run_id=?5 AND status=?6",
            params![
                spent_tokens,
                usage.cached_input_tokens.max(0),
                spent_requests,
                child.run_id,
                lease.root_run_id,
                run_status
            ],
        )
        .map_err(|error| format!("无法结算子智能体预算：{error}"))?;
    if run_settled != 1 {
        return Err("child_budget_run_state_conflict".into());
    }
    let assignment_settled = transaction
        .execute(
            "UPDATE agent_assignments SET reserved_tokens=0,reserved_requests=0,budget_settled_at=datetime('now','localtime'),\
             updated_at=datetime('now','localtime') WHERE id=?1 AND child_run_id=?2 AND lease_epoch=?3 \
             AND fencing_token=?4 AND budget_settled_at=''",
            params![child.assignment_id, child.run_id, lease.lease_epoch, lease.fencing_token],
        )
        .map_err(|error| format!("无法清理子智能体预算预留：{error}"))?;
    if assignment_settled != 1 {
        return Err("child_budget_settlement_replayed_or_stale".into());
    }
    Ok(())
}

fn settle_child_model_usage_on(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    original_web_receipt: bool,
) -> Result<(), String> {
    if original_web_receipt {
        let original = crate::agent_runtime::multi_agent::budget::web_lifetime::child_usage_original(
            transaction, lease, &child.assignment_id, &child.run_id,
        )?;
        if original != *usage { return Err("web_model_settlement_usage_conflict".into()); }
        Ok(())
    } else {
        if child.role == crate::agent_runtime::contract::AgentRole::SpaApiMapper
            && crate::agent_runtime::web_mode::root::read(transaction, &lease.root_run_id)?
                .and_then(|mode| mode.bootstrap_dispatch())
                .is_some_and(|contract| contract["schemaVersion"] == 15)
        {
            let original = crate::agent_runtime::multi_agent::specialist::mapper_loop::verified_original_output_on(
                transaction, lease, child,
            )?;
            if original.usage() != usage {
                return Err("mapper_loop_settlement_original_usage_conflict".into());
            }
            for (dimension, amount) in crate::agent_runtime::multi_agent::budget::DIMENSIONS[..4]
                .iter().zip([
                    usage.input_tokens,
                    usage.cached_input_tokens,
                    usage.output_tokens,
                    usage.model_requests,
                ])
            {
                let balance = crate::agent_runtime::multi_agent::budget::balance(
                    transaction, &lease.root_run_id, Some(&child.assignment_id), dimension,
                )?;
                if balance.consumed != amount || balance.indeterminate != 0 {
                    return Err("mapper_loop_settlement_original_cost_conflict".into());
                }
            }
            // Original rounds already paid the exact cumulative usage. Generic
            // settlement may only release the remaining original reservation.
        }
        crate::agent_runtime::multi_agent::budget::model::settle(transaction, lease, &child.assignment_id, usage)
    }
}

fn finish_coordinator_run(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    outcome: &AgentTargetOutcome,
) -> Result<(), String> {
    crate::agent_runtime::multi_agent::budget::clock::elapsed_fact::record_if_exceptional(
        connection, lease,
    )?;
    crate::agent_runtime::multi_agent::budget::clock::closure_write(
        connection,
        lease,
        false,
        |tx| finish_coordinator_run_in_transaction(tx, lease, outcome).map(|_| ()),
    )
}

fn finish_coordinator_run_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    outcome: &AgentTargetOutcome,
) -> Result<crate::agent_runtime::multi_agent::budget::clock::FinalClock, String> {
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(transaction, lease)?;
    let mut final_clock =
        crate::agent_runtime::multi_agent::budget::clock::FinalClock::capture(transaction, lease)?;
    if outcome.completion().is_some() {
        crate::agent_runtime::multi_agent::budget::admission::require_settled_for_completion(
            transaction,
            &lease.root_run_id,
        )?;
    }
    // Already elapsed time is a financial fact for every outcome.
    final_clock.sample(transaction)?;
    let terminal_state = outcome.terminal_status();
    let terminal_code = outcome.terminal_code();
    let terminal_reason = crate::agent_runtime::secrets::redact_text_with(&outcome.detail(), None);
    if final_clock.was_terminal() {
        let replay_matches:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1
            AND status='terminal' AND terminal_state=?2 AND terminal_code=?3 AND terminal_reason=?4 AND finished_at=?5)",
            params![lease.root_run_id,terminal_state,terminal_code,terminal_reason,final_clock.cutoff()],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !replay_matches {
            return Err("coordinator_terminal_fencing_or_state_conflict".into());
        }
        // Original terminal replay is pure verification: no UPDATE, directive
        // closure, sampling to now, lease renewal or retrospective backfill.
        final_clock.seal(transaction, terminal_state, terminal_code, &terminal_reason)?;
        return Ok(final_clock);
    }
    let changed = transaction
        .execute(
            "UPDATE agent_runs SET status='terminal',terminal_state=?1,terminal_code=?2,terminal_reason=?3,\
             finished_at=?5,updated_at=datetime('now','localtime') \
             WHERE id=?4 AND role='coordinator' AND root_run_id=id AND status<>'terminal'",
            params![
                terminal_state,
                terminal_code,
                terminal_reason,
                lease.root_run_id,
                final_clock.cutoff()
            ],
        )
        .map_err(|error| format!("无法写入 Coordinator 终态：{error}"))?;
    if changed != 1 {
        let replay_matches: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND role='coordinator' \
                 AND root_run_id=id AND status='terminal' AND terminal_state=?2 AND terminal_code=?3)",
                params![lease.root_run_id, terminal_state, terminal_code],
                |row| row.get(0),
            )
            .map_err(|error| format!("无法确认 Coordinator 终态重放：{error}"))?;
        if !replay_matches {
            return Err("coordinator_terminal_fencing_or_state_conflict".into());
        }
    }
    let _idle_source_sdk = crate::agent_runtime::multi_agent::source_rounds::require_idle_for_root(transaction,lease)?;
    let _idle_web_sdk = native_web_model_idle_original(transaction,lease)?;
    let _idle_root_sdk = crate::agent_runtime::multi_agent::budget::root::model::tick::require_idle_original(transaction, lease)?;
    // The original terminal row and all SDK probes share this write lock.
    // A later claimant must recheck this terminal row before any dispatch.
    let _idle_specialists = crate::agent_runtime::multi_agent::specialist::require_idle_for_root(transaction, lease)?;
    close_human_directives_in_transaction(transaction, lease, terminal_code)?;
    _idle_specialists.close_readonly(transaction)?;
    final_clock.seal(transaction, terminal_state, terminal_code, &terminal_reason)?;
    if outcome.completion().is_some() {
        crate::agent_runtime::multi_agent::budget::admission::require_settled_for_completion(
            transaction,
            &lease.root_run_id,
        )?;
        let configured: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?1)",
                [&lease.root_run_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if configured {
            crate::agent_runtime::multi_agent::budget::clock::remaining(
                transaction,
                &lease.root_run_id,
            )?;
        }
    }
    Ok(final_clock)
}
