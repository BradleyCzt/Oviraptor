//! Phase 1.1：最小 Worker 消费者（DESIGN §5.3、REVIEW §8.1 第 1 条）。
//!
//! 职责边界：只消费 Root→Worker 的既有 mailbox 通道
//! （kind=`homogeneous_slice_dispatch`），开跑 → 只读执行 → 汇报，
//! 让 assignment 自然走到 `reported` 交给主管 accept/reject_retry。
//!
//! 这里**不做**的事（全部留给 Phase 2）：模型驱动的只读分析、`target_touching`
//! /`review` lane 的同质并行、任何写目标的动作。执行体只有 store 读，
//! 因此不产生模型/目标消耗，`consumed_*` 如实写 0。
//!
//! 失败路径必须留痕：工具未授权、lane 未开放、租约/截止到期、用户取消、
//! 切片范围内无证据——都落成 `WorkerReport`（`terminal_hint='failed'`，
//! `coverage_notes` 里带 SPEC §1.1 已有的 reason_code 字面量），
//! 绝不把 assignment 卡在 `running`。reason_code 一律取自 `super::reason`，不新增。

use super::decision::DecisionError;
use super::delivery::MESSAGE_KIND;
use super::{apply, machine, reason, store, types};
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use crate::agent_runtime::multi_agent::mailbox;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

/// 一次消费的审计摘要（供循环侧写 runner log）。
#[derive(Clone, Debug, PartialEq)]
pub struct Consumed {
    pub assignment_id: String,
    pub terminal_hint: String,
    /// 失败路径带 SPEC 已有 reason_code；正常完成为 None。
    pub reason_code: Option<&'static str>,
}

/// 只读执行允许的现网读侧工具串（与 `gate::tool_allowlist_for_lane` 同一来源）。
const READ_ONLY_TOOLS: [&str; 3] = ["evidence.read", "snapshot.read", "mailbox.read"];

/// 单份汇报最多引用多少条证据（与 SPEC 的列表上限保持保守一致）。
const MAX_EVIDENCE_REFS: usize = 16;

fn mailbox_error(detail: String) -> DecisionError {
    DecisionError::new(reason::SCHEMA_INVALID, detail)
}

/// 消费本 root 下所有待执行的同质切片下发。legacy root 零副作用。
pub fn drain_for_lease(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<Vec<Consumed>, DecisionError> {
    let task = match store::load_task_by_root_run(connection, &lease.root_run_id)? {
        Some(task) => task,
        // 非同质任务：不读不改，直接返回空。
        None => return Ok(Vec::new()),
    };
    // 主管只派发；已下发但未开跑的 assigned，以及上一次中断留下的 running，
    // 都由这里接续。其它状态一律不动。
    let candidates = store::list_assignment_ids_in_state(
        connection,
        &lease.root_run_id,
        &["assigned", "running"],
    )?;
    let mut consumed = Vec::new();
    for assignment_id in candidates {
        let assignment = match store::load_assignment(connection, &assignment_id)? {
            Some(assignment) => assignment,
            None => continue,
        };
        if !has_pending_dispatch(connection, lease, &assignment)? {
            continue;
        }
        consumed.push(consume_one(connection, lease, &task, &assignment)?);
    }
    Ok(consumed)
}

/// 该 assignment 是否还有属于本轮 attempt 的未确认下发消息。
/// 已确认（已领取）的不重复执行，避免同一 attempt 汇报两次。
fn has_pending_dispatch(
    connection: &Connection,
    lease: &CoordinatorLease,
    assignment: &types::HomogeneousAssignment,
) -> Result<bool, DecisionError> {
    let run_id = carrier_run(connection, assignment)?;
    let correlation = format!("{}:{}", assignment.assignment_id, assignment.attempt);
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_messages WHERE to_run_id=?1 AND root_run_id=?2 \
             AND kind=?3 AND correlation_id=?4 AND acknowledged_at='')",
            params![run_id, lease.root_run_id, MESSAGE_KIND, correlation],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| mailbox_error(format!("worker_dispatch_lookup:{error}")))
}

fn carrier_run(
    connection: &Connection,
    assignment: &types::HomogeneousAssignment,
) -> Result<String, DecisionError> {
    connection
        .query_row(
            "SELECT child_run_id FROM agent_assignments WHERE id=?1 AND scheduling_model='homogeneous_workers'",
            [assignment.assignment_id.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| mailbox_error(format!("worker_carrier_read:{error}")))
        .and_then(|run| {
            run.filter(|value| !value.is_empty()).ok_or_else(|| {
                DecisionError::new(
                    reason::ASSIGNMENT_NOT_FOUND,
                    format!("assignment {} 没有执行载体", assignment.assignment_id),
                )
            })
        })
}

/// 领取 → 开跑 → 只读执行 → 汇报。开跑之后任何一步失败都必须写出汇报。
fn consume_one(
    connection: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
    assignment: &types::HomogeneousAssignment,
) -> Result<Consumed, DecisionError> {
    let run_id = carrier_run(connection, assignment)?;
    if assignment.state == "assigned" {
        apply::mark_started(connection, &assignment.assignment_id)?;
    }
    let work = receive_and_read(connection, lease, task, assignment, &run_id);
    let (evidence_refs, notes, terminal_hint, reason_code) = match work {
        Ok(outcome) => outcome,
        Err(error) => (
            Vec::new(),
            vec![error.reason_code.to_string()],
            "failed".to_string(),
            Some(error.reason_code),
        ),
    };
    let report = types::WorkerReport {
        report_id: store::report_id_for(&assignment.assignment_id, assignment.attempt),
        assignment_id: assignment.assignment_id.clone(),
        slice_id: assignment.slice_id.clone(),
        worker_id: assignment.worker_id.clone(),
        evidence_refs,
        output: serde_json::json!({
            "summary_zh": summary_zh(assignment, terminal_hint == "completed", reason_code),
            "findings": [],
            "coverage_notes": notes,
        }),
        // 最小消费者只做 store 读，不产生模型或目标消耗，如实写 0。
        consumed_tokens: 0,
        consumed_requests: 0,
        terminal_hint,
    };
    // 汇报落库即把 assignment/slice 推到 reported；失败原因由 machine 复核。
    apply::record_worker_report(connection, lease, &report)?;
    Ok(Consumed {
        assignment_id: assignment.assignment_id.clone(),
        terminal_hint: report.terminal_hint.clone(),
        reason_code,
    })
}

type WorkOutcome = (Vec<String>, Vec<String>, String, Option<&'static str>);

/// 领取消息（现网收方 API，含 authority/租约三元组校验）并在切片 allowlist 内做只读执行。
fn receive_and_read(
    connection: &Connection,
    lease: &CoordinatorLease,
    task: &types::HomogeneousTask,
    assignment: &types::HomogeneousAssignment,
    run_id: &str,
) -> Result<WorkOutcome, DecisionError> {
    let message_id = pending_message_id(connection, lease, assignment, run_id)?;
    if let Some(message_id) = message_id {
        let message = mailbox::deliver_expected(connection, lease, run_id, &message_id)
            .map_err(|error| mailbox_error(format!("worker_message_delivery:{error}")))?;
        if message.kind != MESSAGE_KIND {
            return Err(mailbox_error(format!(
                "worker_message_kind:{}",
                message.kind
            )));
        }
        mailbox::acknowledge(connection, lease, run_id, &message_id)
            .map_err(|error| mailbox_error(format!("worker_message_ack:{error}")))?;
    }
    read_only_work(connection, task, assignment)
}

/// 待领取消息 id；已确认过的返回 None（幂等接续，不重复消费）。
fn pending_message_id(
    connection: &Connection,
    lease: &CoordinatorLease,
    assignment: &types::HomogeneousAssignment,
    run_id: &str,
) -> Result<Option<String>, DecisionError> {
    let correlation = format!("{}:{}", assignment.assignment_id, assignment.attempt);
    connection
        .query_row(
            "SELECT id FROM agent_messages WHERE to_run_id=?1 AND root_run_id=?2 \
             AND kind=?3 AND correlation_id=?4 AND acknowledged_at='' ORDER BY created_at,id LIMIT 1",
            params![run_id, lease.root_run_id, MESSAGE_KIND, correlation],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| mailbox_error(format!("worker_message_read:{error}")))
}

fn summary_zh(
    assignment: &types::HomogeneousAssignment,
    completed: bool,
    code: Option<&'static str>,
) -> String {
    if completed {
        return format!(
            "只读消费切片 {}（范围 {}={}），引用已核证据。",
            assignment.slice_id, assignment.scope_kind, assignment.scope_value
        );
    }
    format!(
        "切片 {} 只读执行未完成：{}。",
        assignment.slice_id,
        code.unwrap_or(reason::EVIDENCE_MISSING)
    )
}

/// 失败原因判定顺序：先问「有没有资格做这件事」，再问「有没有可引用的证据」。
fn read_only_work(
    connection: &Connection,
    task: &types::HomogeneousTask,
    assignment: &types::HomogeneousAssignment,
) -> Result<WorkOutcome, DecisionError> {
    // DESIGN 附录 C / §5.3：Phase 1 只开放只读 lane 的同质并行。
    if assignment.lane != "read_only_analysis" {
        return Ok(failed(reason::LANE_QUOTA_EXCEEDED));
    }
    if cancelled(connection, &task.root_run_id)? {
        return Ok(failed(reason::USER_CANCELLED));
    }
    if expired(connection, &assignment.assignment_id)? {
        return Ok(failed(reason::LEASE_INVALID));
    }
    // 切片的 capability_lease 就是 allowlist：没有读侧工具就不许读，也不许猜。
    if !assignment
        .capability_lease
        .iter()
        .any(|tool| READ_ONLY_TOOLS.contains(&tool.as_str()))
    {
        return Ok(failed(reason::TOOL_NOT_AUTHORIZED));
    }
    let slice = store::load_slice(connection, &assignment.slice_id)?
        .ok_or_else(|| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "切片不可寻"))?;
    let refs = in_scope_evidence(connection, task, &slice)?;
    if refs.is_empty() {
        // 范围内没有可用证据：如实报失败，主管会按 evidence_missing 决定重派。
        return Ok(failed(reason::EVIDENCE_MISSING));
    }
    let notes: Vec<String> = refs
        .iter()
        .take(MAX_EVIDENCE_REFS)
        .map(|alias| format!("已引用切片范围内证据 {alias}"))
        .collect();
    Ok((
        refs.iter().take(MAX_EVIDENCE_REFS).cloned().collect(),
        notes,
        "completed".to_string(),
        None,
    ))
}

fn failed(code: &'static str) -> WorkOutcome {
    (
        Vec::new(),
        vec![code.to_string()],
        "failed".to_string(),
        Some(code),
    )
}

fn cancelled(connection: &Connection, root_run_id: &str) -> Result<bool, DecisionError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND cancel_requested_at<>'')",
            [root_run_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| mailbox_error(format!("worker_cancel_read:{error}")))
}

fn expired(connection: &Connection, assignment_id: &str) -> Result<bool, DecisionError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 \
             AND deadline_at<>'' AND datetime(deadline_at)<=datetime('now','localtime'))",
            [assignment_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|error| mailbox_error(format!("worker_deadline_read:{error}")))
}

/// 只读执行本体：读现网已观测证据节点，取切片范围内、版本不早于基线、
/// 且未被同任务其它切片声称的条目，并为它们登记 DESIGN §4.8 的别名。
/// 没有任何网络或模型调用，因此不可能扩权。
fn in_scope_evidence(
    connection: &Connection,
    task: &types::HomogeneousTask,
    slice: &types::TaskSlice,
) -> Result<Vec<String>, DecisionError> {
    let mut statement = connection
        .prepare(
            "SELECT id,revision,payload_json FROM agent_evidence_nodes \
             WHERE root_run_id=?1 AND revision>=?2 AND provenance='observed' \
             ORDER BY revision,id",
        )
        .map_err(|error| mailbox_error(format!("worker_evidence_read:{error}")))?;
    let rows: Vec<(String, i64, JsonValue)> = statement
        .query_map(
            params![task.root_run_id, slice.evidence_revision_base],
            |row| {
                let raw: String = row.get(2)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    serde_json::from_str(&raw).unwrap_or(JsonValue::Null),
                ))
            },
        )
        .map_err(|error| mailbox_error(format!("worker_evidence_query:{error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| mailbox_error(format!("worker_evidence_decode:{error}")))?;
    drop(statement);
    let mut refs: Vec<String> = Vec::new();
    for (internal_id, revision, payload) in rows {
        if refs.len() >= MAX_EVIDENCE_REFS {
            break;
        }
        if !machine::within_scope(slice, &payload) {
            continue;
        }
        let alias =
            store::register_evidence_alias(connection, &internal_id, &task.root_run_id, revision)?;
        // 复核失败按「已被别人声称」处理：宁可不引用，也不制造跨切片声称。
        if machine::claimed_by_other_slice(connection, &task.task_id, &slice.slice_id, &alias)
            .unwrap_or(true)
        {
            continue;
        }
        refs.push(alias);
    }
    Ok(refs)
}
