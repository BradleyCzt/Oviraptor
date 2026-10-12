// Local delivery of an original paid output after signed worker expiry.
fn complete_expired_mapper_assessment(
    connection: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    payload: &JsonValue,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::{budget, mailbox, specialist::mapper_loop};
    mapper_expired_delivery_writer::run(connection, actor, child, |tx| {
    if let Some(receipt) = mapper_loop::expired_seal::read_original_on(tx, actor, child)? {
        let original = mapper_loop::expired_saved::read_closed_original_on(tx, actor, child)?;
        if original.output().usage() != usage
            || *payload != serde_json::json!({"summary":original.output().text()})
            || receipt.message_id().is_empty()
        {
            return Err("mapper_loop_expired_delivery_original_conflict".into());
        }
        return Ok(payload.clone());
    }
    // A held original SDK exit guard and borrowed transaction prevent transport
    // or a new transaction from replacing the paid source during local closure.
    let financial = mapper_loop::expired_saved::capture_expired_output_on(tx, actor, child)?;
    if financial.output().usage() != usage
        || *payload != serde_json::json!({"summary":financial.output().text()})
    {
        return Err("mapper_loop_expired_delivery_original_conflict".into());
    }
    let seal = mapper_loop::expired_seal::capture_before_on(tx, actor, child, &financial)?;
    financial.require_before_on(tx, actor, child)?;
    settle_child_usage_for_status(tx, actor, child, usage, "paused")?;
    let assignment = tx.execute(
        "UPDATE agent_assignments SET state='completed',failure_class='',finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1 AND state='paused' AND failure_class='worker_lease_expired' AND budget_settled_at<>'' AND reserved_tokens=0 AND reserved_requests=0",
        [&child.assignment_id],
    ).map_err(|e| e.to_string())?;
    let run = tx.execute(
        "UPDATE agent_runs SET status='terminal',terminal_state='completed',terminal_code='readonly_expired_mapper_delivered',terminal_reason=?2,finished_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1 AND status='paused' AND cancel_requested_at=''",
        params![child.run_id, financial.output().text()],
    ).map_err(|e| e.to_string())?;
    if assignment != 1 || run != 1 {
        return Err("mapper_loop_expired_delivery_write_conflict".into());
    }
    budget::limits::release_slot(tx, actor, &child.assignment_id)?;
    tx.execute("DELETE FROM agent_lane_leases WHERE assignment_id=?1", [&child.assignment_id])
        .map_err(|e| e.to_string())?;
    // The original worker remains expired; generic finish_saved is a legacy
    // single-call reader and must not mutate this immutable expiry source.
    financial.require_closed_original_on(tx, actor, child)?;
    let message = mailbox::send_saved_specialist(
        tx, actor, &child.run_id, &actor.root_run_id, "spa_api_mapper", "coordinator",
        "evidence_summary", &format!("mapper:{}", child.assignment_id),
        &child.assignment_id, 1, payload,
    )?;
    consume_proposal_mailbox_in_transaction(
        tx, actor, &actor.root_run_id, &message, "evidence_summary", payload,
    )?;
    mapper_loop::expired_seal::issue_after_on(tx, actor, child, &seal, &message)?;
    drop(seal);
    drop(financial);
    Ok(payload.clone())
    })
}
