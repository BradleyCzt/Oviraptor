//! Root-approved constraints travel through the existing assignment mailbox.
//! A running one-shot input is immutable: late guidance is retained for the next
//! eligible Worker, and is only applied after a Worker actually reads it.
use super::{directive_gate, store, types};
use crate::agent_runtime::multi_agent::{lease::CoordinatorLease, mailbox};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
pub const KIND: &str = "homogeneous_guidance";
pub const MAX_BYTES: usize = 4096;

pub fn input(
    db: &Connection,
    task: &types::HomogeneousTask,
    slice: &types::TaskSlice,
) -> Result<Value, String> {
    let mut q=db.prepare("SELECT directive_id,rewritten_text,broadcast_constraint FROM agent_human_directive_gated WHERE parent_task_id=?1 AND state IN ('relayed','rewritten','applied') AND (broadcast_constraint<>'' OR target_worker_id IN (SELECT homogeneous_worker_id FROM agent_assignments WHERE slice_id=?2 AND coordinator_run_id=?3 AND scheduling_model='homogeneous_workers' AND state IN ('assigned','running'))) ORDER BY directive_id").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(
            params![task.task_id, slice.slice_id, task.root_run_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    let mut cards = Vec::new();
    let mut bytes = 2;
    for row in rows {
        let (id, text, broadcast) = row.map_err(|e| e.to_string())?;
        let card =
            json!({"directive_id":id,"constraint":if broadcast.is_empty(){text}else{broadcast}});
        let size = card.to_string().len() + 1;
        if bytes + size > MAX_BYTES || cards.len() == 8 {
            break;
        }
        bytes += size;
        cards.push(card);
    }
    Ok(json!(cards))
}

pub fn relay(db: &Connection, actor: &CoordinatorLease, directive_id: &str) -> Result<(), String> {
    let d = store::load_gated_directive(db, directive_id)
        .map_err(|e| e.detail)?
        .ok_or("homogeneous_guidance_missing")?;
    let mut q=db.prepare("SELECT id,child_run_id,evidence_revision,homogeneous_worker_id FROM agent_assignments WHERE coordinator_run_id=?1 AND scheduling_model='homogeneous_workers' AND state IN ('assigned','running') AND (?2<>'' OR homogeneous_worker_id=?3)").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(
            params![
                actor.root_run_id,
                d.broadcast_constraint,
                d.target_worker_id
            ],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for (assignment, child, revision, worker) in rows {
        mailbox::send(
            db,
            actor,
            &actor.root_run_id,
            &child,
            "coordinator",
            "deep_investigator",
            KIND,
            &d.directive_id,
            &assignment,
            revision,
            &json!({"directiveId":d.directive_id,"workerId":worker,"constraint":if d.broadcast_constraint.is_empty(){&d.rewritten_text}else{&d.broadcast_constraint},"summaryZh":"主管已投递用户引导；等待 Worker 读取，已发送的模型上下文保持原样。"}),
        )?;
    }
    Ok(())
}

pub fn acknowledge_input(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &str,
    assignment: &str,
    data: &Value,
) -> Result<(), String> {
    for card in data["guidance"].as_array().into_iter().flatten() {
        let id = card["directive_id"]
            .as_str()
            .ok_or("homogeneous_guidance_invalid")?;
        let d = store::load_gated_directive(db, id)
            .map_err(|e| e.detail)?
            .ok_or("homogeneous_guidance_missing")?;
        let revision: i64 = db
            .query_row(
                "SELECT evidence_revision FROM agent_assignments WHERE id=?1",
                [assignment],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let message = mailbox::send(
            db,
            actor,
            &actor.root_run_id,
            child,
            "coordinator",
            "deep_investigator",
            KIND,
            id,
            assignment,
            revision,
            &json!({"directiveId":id,"workerId":db.query_row("SELECT homogeneous_worker_id FROM agent_assignments WHERE id=?1",[assignment],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?,"constraint":card["constraint"],"summaryZh":"主管已投递用户引导；等待 Worker 读取，已发送的模型上下文保持原样。"}),
        )?;
        mailbox::deliver_expected(db, actor, child, &message)?;
        mailbox::acknowledge(db, actor, child, &message)?;
        if matches!(d.state.as_str(), "relayed" | "rewritten") {
            directive_gate::mark_applied(db, id).map_err(|e| e.detail)?;
        }
    }
    Ok(())
}

pub fn verify_frozen_input(current: &Value, frozen: &Value) -> Result<(), String> {
    let mut a = current.clone();
    let mut b = frozen.clone();
    a.as_object_mut()
        .ok_or("homogeneous_worker_input_invalid")?
        .remove("guidance");
    b.as_object_mut()
        .ok_or("homogeneous_worker_input_invalid")?
        .remove("guidance");
    if a != b
        || !frozen["guidance"].as_array().is_some_and(|items| {
            items.iter().all(|item| {
                current["guidance"]
                    .as_array()
                    .is_some_and(|now| now.contains(item))
            })
        })
    {
        return Err("homogeneous_worker_input_changed".into());
    }
    Ok(())
}
