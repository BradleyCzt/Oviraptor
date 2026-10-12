//! Original completed worker delivery; pure audit without mailbox/live-C APIs.
use crate::agent_runtime::multi_agent::{lease::CoordinatorLease, specialist};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
pub(super) fn verify(db: &Connection, actor: &CoordinatorLease) -> Result<(), String> {
    let mut q=db.prepare("SELECT r.id,r.assignment_id,r.role,r.terminal_reason,r.used_tokens,r.used_requests,a.evidence_revision
        FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id
        WHERE r.root_run_id=?1 AND r.id<>?1 ORDER BY r.id").map_err(|e|e.to_string())?;
    let workers = q
        .query_map([&actor.root_run_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for (run, assignment, role, reason, tokens, requests, revision) in workers {
        let (kind, prefix) = match role.as_str() {
            "spa_api_mapper" => ("evidence_summary", "mapper"),
            "identity_session" => ("identity_assessment", "identity"),
            "external_surface" => ("evidence_summary", "public-surface"),
            "client_side" => ("evidence_summary", "client-side"),
            "web_executor" => ("execution_result", "executor-result"),
            _ => return Err("deleted_audit_worker_delivery_contract_unsupported".into()),
        };
        let correlation = format!("{prefix}:{assignment}");
        let dedup = format!("{assignment}:{kind}:{correlation}:{revision}");
        let mut q=db.prepare("SELECT id,payload_json,delivered_at,acknowledged_at FROM agent_messages
            WHERE run_id=?1 AND root_run_id=?1 AND from_run_id=?2 AND to_run_id=?1 AND assignment_id=?3
            AND from_agent=?4 AND to_agent='coordinator' AND kind=?5 AND correlation_id=?6 AND dedup_key=?7
            AND evidence_revision=?8 AND artifact_refs_json='[]' AND delivered_at<>'' AND acknowledged_at<>'' AND delivery_attempts=1").map_err(|e|e.to_string())?;
        let messages = q
            .query_map(
                params![
                    actor.root_run_id,
                    run,
                    assignment,
                    role,
                    kind,
                    correlation,
                    dedup,
                    revision
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        if messages.len() != 1 {
            return Err("deleted_audit_original_delivery_receipt_missing".into());
        }
        let (id, raw, delivered, ack) = &messages[0];
        let payload: Value = serde_json::from_str(raw)
            .map_err(|_| "deleted_audit_original_delivery_payload_invalid")?;
        let canonical = payload.to_string();
        if canonical != *raw || payload["summary"] != reason {
            return Err("deleted_audit_original_delivery_payload_conflict".into());
        }
        if role == "web_executor" {
            let usage:(i64,i64)=db.query_row("SELECT COALESCE(SUM(json_extract(receipt_json,'$.usage.totalTokens')),0),count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'",[&run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
            if usage != (tokens, requests)
                || payload["usage"]["totalTokens"] != tokens
                || payload["usage"]["modelRequests"] != requests
            {
                return Err("deleted_audit_original_delivery_usage_conflict".into());
            }
        } else if role == "spa_api_mapper"
            && crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
                .and_then(|mode| mode.bootstrap_dispatch())
                .is_some_and(|contract| contract["schemaVersion"] == 15)
        {
            let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
                assignment_id: assignment.clone(),
                run_id: run.clone(),
                role: crate::agent_runtime::contract::AgentRole::SpaApiMapper,
            };
            let worker = crate::agent_runtime::multi_agent::attempts::current(db, actor, &assignment)?;
            let original_message = if worker.state == "expired" {
                specialist::mapper_loop::expired_seal::read_original_on(db, actor, &child)?
                    .ok_or("deleted_audit_mapper_loop_original_expired_missing")?.message_id().to_owned()
            } else {
                specialist::mapper_loop::final_seal::read_final_original_on(db, actor, &child)?
                    .ok_or("deleted_audit_mapper_loop_original_final_missing")?.message_id().to_owned()
            };
            let output = specialist::mapper_loop::verified_original_output_on(db, actor, &child)?;
            if original_message != *id
                || payload != json!({"summary":output.text()})
                || output.usage().total_tokens != tokens
                || output.usage().model_requests != requests
            {
                return Err("deleted_audit_original_delivery_usage_conflict".into());
            }
        } else {
            specialist::verify_received_original(db, actor, &assignment)?;
            let (response,usage):(String,String)=db.query_row("SELECT response_json,usage_json FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?2",params![assignment,run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
            let response: Value = serde_json::from_str(&response)
                .map_err(|_| "deleted_audit_original_delivery_payload_invalid")?;
            let usage: Value = serde_json::from_str(&usage)
                .map_err(|_| "deleted_audit_original_delivery_usage_conflict")?;
            if payload["summary"] != response["text"]
                || usage["totalTokens"] != tokens
                || usage["modelRequests"] != requests
            {
                return Err("deleted_audit_original_delivery_usage_conflict".into());
            }
        }
        for (entity, key, wanted) in [
            (
                "assignment",
                assignment.as_str(),
                vec![json!({"role":role,"state":"completed"})],
            ),
            (
                "agent_run",
                run.as_str(),
                vec![json!({"role":role,"status":"terminal","terminalState":"completed"})],
            ),
            (
                "mailbox_message",
                id.as_str(),
                vec![
                    json!({"kind":kind,"deliveredAt":"","acknowledgedAt":""}),
                    json!({"kind":kind,"deliveredAt":delivered,"acknowledgedAt":ack}),
                ],
            ),
        ] {
            let mut q=db.prepare("SELECT payload_json FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2 AND entity_type=?3 AND event_type=?3 AND entity_id=?4 ORDER BY sequence").map_err(|e|e.to_string())?;
            let events = q
                .query_map(
                    params![actor.scan_id, actor.attempt_number, entity, key],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|s| {
                    serde_json::from_str::<Value>(&s)
                        .map_err(|_| "deleted_audit_original_delivery_event_invalid".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            for expected in &wanted {
                if events.iter().filter(|v| *v == expected).count() != 1 {
                    return Err("deleted_audit_original_delivery_event_conflict".into());
                }
            }
            if entity == "mailbox_message" && events.len() != wanted.len() {
                return Err("deleted_audit_original_delivery_event_conflict".into());
            }
        }
    }
    Ok(())
}
