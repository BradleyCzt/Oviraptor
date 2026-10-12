fn deliver_readonly_assessment(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    payload: &JsonValue,
    target_dir: Option<&Path>,
) -> Result<JsonValue, String> {
    let complete = || match target_dir {
        Some(path) => complete_assessment_at(connection, lease, child, usage, payload, Some(path)),
        None => complete_readonly_assessment(connection, lease, child, usage, payload),
    };
    let mapper_loop = if child.role == crate::agent_runtime::contract::AgentRole::SpaApiMapper {
        crate::agent_runtime::web_mode::root::read(connection, &lease.root_run_id)?
            .and_then(|mode| mode.bootstrap_dispatch())
            .is_some_and(|contract| {
                contract["schemaVersion"] == 15
                    && contract["mapperLoopRule"]
                        == "original_frozen_mapper_two_round_one_lookup_v1"
            })
    } else {
        false
    };
    // A rejected delivery never enters legacy unsent cleanup. Mapper loop
    // invoices must remain available for their original per-round recovery.
    // The already paid receipt and original grant remain available for explicit local recovery.
    if child.role == crate::agent_runtime::contract::AgentRole::ClientSide
        || child.role == crate::agent_runtime::contract::AgentRole::InputParser
        || child.role == crate::agent_runtime::contract::AgentRole::BusinessLogic
        || mapper_loop
    {
        return complete();
    }
    match complete() {
        Ok(result) => Ok(result),
        Err(error) => {
            // A failed cleanup is not evidence that this worker is paused.
            if let Err(cleanup) =
                stop_failed_child_preserving_usage(connection, lease, child, &error)
            {
                return Err(format!("{error};specialist_cleanup:{cleanup}"));
            }
            complete().map_err(|recovery| format!("{error};readonly_local_recovery:{recovery}"))
        }
    }
}

// A readonly role is not completed until its result is durably consumed. Keep
// accounting, child termination and mailbox acknowledgment in one transaction.
fn complete_readonly_assessment(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    payload: &JsonValue,
) -> Result<JsonValue, String> {
    complete_assessment_at(connection, lease, child, usage, payload, None)
}

fn complete_assessment_at(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    payload: &JsonValue,
    target_dir: Option<&Path>,
) -> Result<JsonValue, String> {
    if child.role == crate::agent_runtime::contract::AgentRole::ClientSide {
        return client_delivery_writer::complete(
            connection, lease, child, usage, payload, target_dir,
        );
    }
    if child.role == crate::agent_runtime::contract::AgentRole::InputParser {
        return complete_assessment_at_checked(
            connection,
            lease,
            child,
            usage,
            payload,
            target_dir,
            |tx| input_parser_validate_received(tx, lease, child, usage, payload, target_dir),
        );
    }
    complete_assessment_at_checked(connection, lease, child, usage, payload, target_dir, |_| {
        Ok(())
    })
}

fn complete_assessment_at_checked(
    connection: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    payload: &JsonValue,
    target_dir: Option<&Path>,
    verify_local: impl Fn(&rusqlite::Connection) -> Result<(), String>,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{lease::require_executable_coordinator, mailbox, scheduler},
    };
    if child.role == AgentRole::SpaApiMapper
        && crate::agent_runtime::web_mode::root::read(connection, &lease.root_run_id)?
            .and_then(|mode| mode.bootstrap_dispatch())
            .is_some_and(|contract| contract["schemaVersion"] == 15)
        && crate::agent_runtime::multi_agent::attempts::current(connection, lease, &child.assignment_id)?.state == "expired"
    {
        verify_local(connection)?;
        return complete_expired_mapper_assessment(connection, lease, child, usage, payload);
    }
    let (kind, prefix) = match child.role {
        AgentRole::SpaApiMapper => ("evidence_summary", "mapper"),
        AgentRole::IdentitySession => ("identity_assessment", "identity"),
        AgentRole::ExternalSurface => ("evidence_summary", "public-surface"),
        AgentRole::BusinessLogic => ("evidence_summary", "business-logic"),
        AgentRole::RepoMapper => ("evidence_summary", "repo-mapper"),
        AgentRole::SourceAnalyst => ("evidence_summary", "source-analyst"),
        AgentRole::ClientSide => ("evidence_summary", "client-side"),
        AgentRole::InputParser => ("input_assessment", "input-parser"),
        _ => return Err("readonly_assessment_role_invalid".into()),
    };
    let payload = crate::agent_runtime::secrets::redact_json(payload);
    let summary = payload["summary"]
        .as_str()
        .ok_or("readonly_assessment_summary_missing")?;
    let tx =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| format!("readonly_assessment_lock:{e}"))?;
    require_executable_coordinator(&tx, lease)?;
    let expired =
        crate::agent_runtime::multi_agent::attempts::ExpiredSavedProof::capture(&tx, lease, child)?;
    if crate::agent_runtime::multi_agent::source::is_source_role(child.role) {
        let binding =
            crate::agent_runtime::multi_agent::source::verify_assignment(&tx, lease, child)?;
        let receipt = crate::agent_runtime::multi_agent::specialist::received_for_reconciliation(
            &tx, lease, child,
        )?;
        if payload["sourceTask"] != binding
            || receipt.text != summary
            || receipt.usage != *usage
            || !receipt.rejection.is_empty()
        {
            return Err("source_assessment_receipt_mismatch".into());
        }
    }
    if child.role == AgentRole::ClientSide {
        client_side_validate_received(&tx, lease, child, usage, &payload, target_dir)?;
    }
    if child.role == AgentRole::ExternalSurface {
        validate_public_surface_receipt(
            &tx,
            lease,
            child,
            &payload,
            target_dir.ok_or("public_surface_artifact_root_required")?,
        )?;
    }
    verify_local(&tx)?;
    let state: String = tx
        .query_row(
            "SELECT state FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mapper_loop = child.role == AgentRole::SpaApiMapper
        && crate::agent_runtime::web_mode::root::read(&tx, &lease.root_run_id)?
            .and_then(|mode| mode.bootstrap_dispatch())
            .is_some_and(|contract| contract["schemaVersion"] == 15);
    let mapper_output_guard = if mapper_loop {
        let output = crate::agent_runtime::multi_agent::specialist::mapper_loop::verified_original_output_on(&tx, lease, child)?;
        if output.usage() != usage || payload != serde_json::json!({"summary":output.text()}) {
            return Err("mapper_loop_delivery_original_output_conflict".into());
        }
        if state == "running" {
            let original = original_root_capture_mode(&tx, &lease.root_run_id)?;
            Some(crate::agent_runtime::multi_agent::specialist::mapper_loop::final_seal::capture_final_before_on(&tx, lease, child, &original)?)
        } else if state == "completed" {
            crate::agent_runtime::multi_agent::specialist::mapper_loop::final_seal::read_final_original_on(&tx, lease, child)?
                .ok_or("mapper_loop_final_original_receipt_missing")?;
            None
        } else {
            return Err("mapper_loop_delivery_original_state_required".into());
        }
    } else { None };
    let surface_output_guard = if child.role == AgentRole::ExternalSurface
        && external_surface_response_receipt::enabled_on(&tx, &lease.root_run_id)? {
        if state == "running" {
            let original = original_root_capture_mode(&tx, &lease.root_run_id)?;
            Some(external_surface_output_receipt::capture_before_on(
                &tx, lease, child,
                target_dir.ok_or("public_surface_artifact_root_required")?, &original,
            )?)
        } else {
            external_surface_output_receipt::read_original_on(&tx, lease, child)?
                .ok_or("external_surface_output_original_receipt_missing")?;
            None
        }
    } else { None };
    let business_output_guard = if child.role == AgentRole::BusinessLogic {
        if state == "running" {
            let original = original_root_capture_mode(&tx, &lease.root_run_id)?;
            Some(business_logic_output_receipt::capture_before_on(
                &tx, lease, child,
                target_dir.ok_or("business_logic_artifact_root_required")?, &original,
            )?)
        } else {
            business_logic_output_receipt::read_original_on(&tx, lease, child)?
                .ok_or("business_logic_output_original_receipt_missing")?;
            None
        }
    } else { None };
    let legacy_received: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1
        AND child_run_id=?2 AND state='received')",
            params![child.assignment_id, child.run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let received = legacy_received || mapper_loop;
    #[cfg(not(test))]
    if !received {
        return Err("readonly_assessment_receipt_required".into());
    }
    // The deterministic role fixture has no provider receipt. Its message is
    // sent only under the live worker, before closure; it grants no saved-result authority.
    let live_message = if !received {
        Some(mailbox::send(
            &tx,
            lease,
            &child.run_id,
            &lease.root_run_id,
            child.role.as_str(),
            "coordinator",
            kind,
            &format!("{prefix}:{}", child.assignment_id),
            &child.assignment_id,
            1,
            &payload,
        )?)
    } else {
        None
    };
    if state == "paused" {
        let received = crate::agent_runtime::multi_agent::specialist::received_for_reconciliation(
            &tx, lease, child,
        )?;
        if received.usage != *usage || received.text != summary || !received.rejection.is_empty() {
            return Err("readonly_assessment_receipt_mismatch".into());
        }
        let recoverable: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
             JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id AND l.attempt_number=r.attempt_number \
               AND l.target_key=r.target_url AND l.lane=a.lane \
             WHERE a.id=?1 AND a.state='paused' AND r.status='paused' AND a.budget_settled_at='' \
               AND (a.failure_class='child_usage_reconciliation_required' OR (?3 AND a.failure_class='worker_lease_expired'))) \
             AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='')",
            params![child.assignment_id,child.run_id,expired.is_some()], |r|r.get(0),
        ).map_err(|e|format!("readonly_assessment_recovery_state:{e}"))?;
        if !recoverable {
            return Err("readonly_assessment_recovery_state_conflict".into());
        }
        settle_child_usage_for_status(&tx, lease, child, usage, "paused")?;
        // Dedicated local transition. Generic scheduling/finish APIs continue
        // to reject paused workers; no capability is reissued here.
        let changed = tx.execute(
            "UPDATE agent_assignments SET state='completed',failure_class='',finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') \
             WHERE id=?1 AND state='paused' AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0",
            [&child.assignment_id],
        ).map_err(|e|e.to_string())?;
        let finished = tx.execute(
            "UPDATE agent_runs SET status='terminal',terminal_state='completed',terminal_code='readonly_receipt_reconciled', \
             terminal_reason='Saved readonly result delivered locally; no model retry or capability restoration', \
             finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1 AND status='paused'",
            [&child.run_id],
        ).map_err(|e|e.to_string())?;
        if changed != 1 || finished != 1 {
            return Err("readonly_assessment_recovery_write_conflict".into());
        }
        crate::agent_runtime::multi_agent::budget::limits::release_slot(
            &tx,
            lease,
            &child.assignment_id,
        )?;
        tx.execute(
            "DELETE FROM agent_lane_leases WHERE assignment_id=?1",
            [&child.assignment_id],
        )
        .map_err(|e| e.to_string())?;
        crate::agent_runtime::multi_agent::attempts::finish_saved(&tx, lease, child)?;
    } else if state == "running" {
        settle_child_usage_in_transaction(&tx, lease, child, usage)?;
        scheduler::finish_child_in_transaction(&tx, lease, child, true, summary)?;
    } else if state == "completed" {
        settle_child_usage_in_transaction(&tx, lease, child, usage)?;
    } else {
        return Err("readonly_assessment_state_conflict".into());
    }
    let message = match live_message {
        Some(id) => id,
        None => mailbox::send_saved_specialist(
            &tx,
            lease,
            &child.run_id,
            &lease.root_run_id,
            child.role.as_str(),
            "coordinator",
            kind,
            &format!("{prefix}:{}", child.assignment_id),
            &child.assignment_id,
            1,
            &payload,
        )?,
    };
    consume_proposal_mailbox_in_transaction(
        &tx,
        lease,
        &lease.root_run_id,
        &message,
        kind,
        &payload,
    )?;
    let complete: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         JOIN agent_messages m ON m.assignment_id=a.id WHERE a.id=?1 AND a.child_run_id=?2 \
         AND a.state='completed' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 \
         AND r.status='terminal' AND r.terminal_state='completed' AND r.used_tokens=?3 AND r.used_requests=?4 \
         AND m.id=?5 AND m.delivered_at<>'' AND m.acknowledged_at<>'' AND m.delivery_attempts=1) \
         AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE child_run_id=?2 AND revoked_at='') \
         AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",
        params![child.assignment_id,child.run_id,usage.total_tokens,usage.model_requests,message],|r|r.get(0),
    ).map_err(|e|format!("readonly_assessment_postcondition:{e}"))?;
    if !complete {
        return Err("readonly_assessment_postcondition".into());
    }
    if child.role == AgentRole::ClientSide {
        client_side_validate_received(&tx, lease, child, usage, &payload, target_dir)?;
    }
    if let Some(proof) = expired {
        proof.verify(&tx, lease, child)?;
    }
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&tx, lease)?;
    crate::agent_runtime::multi_agent::lease::require_executable_coordinator(&tx, lease)?;
    verify_local(&tx)?;
    if let Some(guard) = &surface_output_guard {
        external_surface_output_receipt::issue_after_on(&tx, lease, child, guard, &message)?;
    }
    if let Some(guard) = &business_output_guard {
        business_logic_output_receipt::issue_after_on(&tx, lease, child, guard, &message)?;
    }
    if let Some(guard) = &mapper_output_guard {
        crate::agent_runtime::multi_agent::specialist::mapper_loop::final_seal::issue_final_after_on(&tx, lease, child, guard, &message)?;
    }
    tx.commit()
        .map_err(|e| format!("readonly_assessment_commit:{e}"))?;
    Ok(payload)
}
