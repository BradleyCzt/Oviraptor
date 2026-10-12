// Completed InputParser authority is original invoice + consumed output,
// never a renewed worker capability or a replacement reservation.
fn input_parser_completed_original_replay(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    target_dir: &Path,
) -> Result<bool, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{attempts, budget, mailbox, specialist},
    };
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return input_parser_completed_original_replay(&tx, actor, child, target_dir);
    }
    if child.role != AgentRole::InputParser {
        return Err("input_parser_replay_role_conflict".into());
    }
    let state: String = db
        .query_row(
            "SELECT state FROM agent_assignments WHERE id=?1 AND child_run_id=?2",
            params![child.assignment_id, child.run_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if state != "completed" {
        return Ok(false);
    }
    specialist::verify_received_original(db, actor, &child.assignment_id)?;
    let receipt = specialist::received_for_local_delivery(db, actor, child)?;
    let worker = attempts::current(db, actor, &child.assignment_id)?;
    if worker.finished_at.is_empty() || worker.child_run_id != child.run_id {
        return Err("input_parser_completed_original_conflict".into());
    }
    match worker.state.as_str() {
        "completed" => {},
        "expired" => {
            if !crate::agent_runtime::web_mode::root::read(db,&actor.root_run_id)?
                .is_some_and(|d|d.lease_expiry_supervision().is_some()) {
                return Err("input_parser_expired_original_protocol_required".into());
            }
            original_withdrawal_issuer_on(db,actor)?.ok_or("input_parser_expired_original_issuer_required")?
                .verify_issued(db,&worker.id)?;
            attempts::verify_expired_closed_audit(db,actor,child)?;
        },
        _ => return Err("input_parser_completed_original_conflict".into()),
    }
    let totals: (i64, i64, i64) = db
        .query_row(
            "SELECT r.used_tokens,r.used_cached_tokens,r.used_requests FROM agent_runs r
         JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id
         WHERE r.id=?1 AND r.root_run_id=?2 AND r.finished_at<>'' AND a.finished_at<>''",
            params![child.run_id, actor.root_run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "input_parser_completed_usage_conflict")?;
    if totals
        != (
            receipt.usage.total_tokens,
            receipt.usage.cached_input_tokens,
            receipt.usage.model_requests,
        )
    {
        return Err("input_parser_completed_usage_conflict".into());
    }
    let (slice, _) = input_parser_assignment_slice(db, actor, child)?;
    let (floor_tokens, floor_requests) = native_input_parser_retained_floor();
    if slice["grant"]
        != json!({"reservedModelTokens":NATIVE_INPUT_PARSER_GRANT_TOKENS,
        "reservedModelRequests":NATIVE_INPUT_PARSER_GRANT_REQUESTS,"targetRequestsGranted":0,
        "retainedFloor":{"modelTokens":floor_tokens,"modelRequests":floor_requests}})
    {
        return Err("input_parser_original_grant_conflict".into());
    }
    for (dimension, grant, spent) in [
        ("model_input_tokens", 4000, receipt.usage.input_tokens),
        (
            "model_cached_tokens",
            4000,
            receipt.usage.cached_input_tokens,
        ),
        ("model_output_tokens", 4000, receipt.usage.output_tokens),
        ("model_requests", 1, receipt.usage.model_requests),
    ] {
        let logical = format!("reserve:{}:{dimension}", child.assignment_id);
        let key = if worker.lease_epoch == 1 {
            logical
        } else {
            format!("worker:{}:{logical}", worker.id)
        };
        let count: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_budget_entries
            WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension=?4
            AND kind='reserve' AND amount=?5 AND idempotency_key=?6 AND source_id=?7",
                params![
                    actor.root_run_id,
                    child.assignment_id,
                    worker.id,
                    dimension,
                    grant,
                    key,
                    format!("assignment:{}", child.assignment_id)
                ],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let own = budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            &worker.id,
            dimension,
        )?;
        if count != 1 || own.reserved != 0 || own.indeterminate != 0 || own.consumed != spent {
            return Err("input_parser_original_grant_conflict".into());
        }
    }
    if worker.state == "expired" {
        for (index,dimension) in budget::DIMENSIONS.iter().enumerate().skip(4) {
            let own = budget::balance_for_attempt(db,&actor.root_run_id,&child.assignment_id,&worker.id,dimension)?;
            if own.reserved != 0 || own.indeterminate != 0 || (index<8 && own.consumed!=0) {
                return Err("input_parser_expired_original_budget_unresolved".into());
            }
        }
    }
    let correlation = format!("input-parser:{}", child.assignment_id);
    let messages: Vec<String> = {
        let mut q=db.prepare("SELECT id FROM agent_messages WHERE assignment_id=?1 AND from_run_id=?2
            AND to_run_id=?3 AND run_id=?3 AND root_run_id=?3 AND from_agent='input_parser' AND to_agent='coordinator'
            AND kind='input_assessment' AND correlation_id=?4 AND evidence_revision=1
            AND dedup_key=?1||':input_assessment:'||?4||':1' AND artifact_refs_json='[]'")
            .map_err(|e|e.to_string())?;
        let rows = q
            .query_map(
                params![
                    child.assignment_id,
                    child.run_id,
                    actor.root_run_id,
                    correlation
                ],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
    };
    if messages.len() != 1 {
        return Err("input_parser_completed_delivery_conflict".into());
    }
    let message = mailbox::verified_completed_output(db, actor, &messages[0])?;
    input_parser_validate_received(
        db,
        actor,
        child,
        &receipt.usage,
        &message.payload,
        Some(target_dir),
    )?;
    let expected = crate::agent_runtime::secrets::redact_json(&json!({"summary":receipt.text,
        "inputParserTask":input_parser_build_surface(db,actor,child,target_dir)?}));
    if message.payload != expected {
        return Err("input_parser_completed_delivery_conflict".into());
    }
    let (events,physical):(i64,i64)=db.query_row("SELECT count(DISTINCT entity_type),count(*) FROM agent_collaboration_events
        WHERE scan_id=?1 AND attempt_number=?2 AND entity_type=event_type AND (
         (entity_type='assignment' AND entity_id=?3 AND payload_json=json_object('role','input_parser','state','completed'))
         OR (entity_type='agent_run' AND entity_id=?4 AND payload_json=json_object('role','input_parser','status','terminal','terminalState','completed'))
         OR (entity_type='mailbox_message' AND entity_id=?5 AND payload_json=(SELECT
          json_object('kind','input_assessment','deliveredAt',delivered_at,'acknowledgedAt',acknowledged_at)
          FROM agent_messages WHERE id=?5)))",
        params![actor.scan_id,actor.attempt_number,child.assignment_id,child.run_id,message.id],|r|Ok((r.get(0)?,r.get(1)?)))
        .map_err(|e|e.to_string())?;
    if events != 3 || (worker.state == "expired" && physical != 3) {
        return Err("input_parser_completed_event_conflict".into());
    }
    Ok(true)
}
