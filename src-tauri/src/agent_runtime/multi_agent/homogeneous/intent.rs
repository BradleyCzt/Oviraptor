//! 深度分析 §J：意图仍是 TaskSlice + assignment，不增加主管动作或 Worker 权限。
//! 证据按回合开始的真实状态核验；最后一次 accept 可在下一 tick stop。

use super::decision::{DecisionError, DecisionPayload};
use super::{reason, store, types};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InflightAssignment {
    pub assignment_id: String,
    pub state: String,
    pub attempt: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct IntentSnapshot {
    pub pending_slice_ids: Vec<String>,
    pub retryable_slice_ids: Vec<String>,
    pub inflight_assignments: Vec<InflightAssignment>,
}

impl IntentSnapshot {
    pub fn is_empty(&self) -> bool {
        self.pending_slice_ids.is_empty()
            && self.retryable_slice_ids.is_empty()
            && self.inflight_assignments.is_empty()
    }
}

pub fn snapshot(
    connection: &Connection,
    task: &types::HomogeneousTask,
) -> Result<IntentSnapshot, DecisionError> {
    let slices = store::load_slices(connection, &task.task_id)?;
    let mut result = IntentSnapshot::default();
    for slice in &slices {
        match slice.state.as_str() {
            "pending" => result.pending_slice_ids.push(slice.slice_id.clone()),
            "rejected" if slice.retry_count < slice.max_retries => {
                result.retryable_slice_ids.push(slice.slice_id.clone());
            }
            _ => {}
        }
    }
    // load_assignment 先过滤 scheduling_model；再核验 task 身份，不能借别的任务空转。
    for id in store::list_assignment_ids_in_state(
        connection,
        &task.root_run_id,
        &["assigned", "running", "reported"],
    )? {
        let assignment = store::load_assignment(connection, &id)?
            .ok_or_else(|| illegal("assignment 快照已失效"))?;
        if assignment.parent_task_id == task.task_id {
            result.inflight_assignments.push(InflightAssignment {
                assignment_id: id,
                state: assignment.state,
                attempt: assignment.attempt,
            });
        }
    }
    Ok(result)
}

pub fn illegal(detail: impl Into<String>) -> DecisionError {
    DecisionError::new(reason::ZERO_INTENT_ILLEGAL, detail)
}

/// 按事务执行前后实际状态计算产出，不相信模型声明。
pub fn produced(
    connection: &Connection,
    task: &types::HomogeneousTask,
    before: &IntentSnapshot,
    payload: &DecisionPayload,
) -> Result<bool, DecisionError> {
    match payload {
        DecisionPayload::Decompose(body) => {
            for draft in &body.slices {
                if !before.pending_slice_ids.contains(&draft.slice_id)
                    && store::load_slice(connection, &draft.slice_id)?.is_some_and(|slice| {
                        slice.parent_task_id == task.task_id && slice.state == "pending"
                    })
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        DecisionPayload::Assign(body) => {
            for id in
                store::list_assignment_ids_in_state(connection, &task.root_run_id, &["assigned"])?
            {
                let Some(assignment) = store::load_assignment(connection, &id)? else {
                    continue;
                };
                if assignment.slice_id == body.slice_id && assignment.attempt == body.attempt {
                    return Ok(!before
                        .inflight_assignments
                        .iter()
                        .any(|old| old.assignment_id == id && old.attempt == assignment.attempt));
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

pub fn validate_zero(
    before: &IntentSnapshot,
    zero: Option<&types::ZeroIntent>,
) -> Result<(), DecisionError> {
    if before.is_empty() {
        return Err(illegal(
            "目标未达成且无在跑/排队意图，本轮必须 decompose 或 assign",
        ));
    }
    let zero = zero.ok_or_else(|| illegal("0 新意图必须声明 zero_intent"))?;
    if !types::ZERO_INTENT_REASONS.contains(&zero.reason.as_str())
        || zero.evidence_assignment_ids.is_empty()
        || !types::valid_readable_text(&zero.note_zh, 80)
    {
        return Err(illegal("zero_intent 理由或审核字段不完整"));
    }
    for id in &zero.evidence_assignment_ids {
        if !before
            .inflight_assignments
            .iter()
            .any(|assignment| assignment.assignment_id == *id)
        {
            return Err(illegal(format!(
                "zero_intent 证据不是本任务回合开始的真实 inflight：{id}"
            )));
        }
    }
    Ok(())
}

/// 粗动作候选只读稳定的子账投影，避免 Root 本轮收费改变已冻结的动作集合。
/// hard=0 沿用原账本的 unlimited 约定，不用剩余数值 0 代表耗尽。
pub fn coarse_budget_capacity(
    connection: &Connection,
    root_run_id: &str,
) -> Result<(Option<i64>, Option<i64>), DecisionError> {
    let (tokens, requests, reserved_tokens, reserved_requests, spent_tokens, spent_requests):
        (i64, i64, i64, i64, i64, i64) = connection
        .query_row(
            "SELECT COALESCE(l.total_tokens,r.hard_token_budget),COALESCE(l.total_requests,r.hard_request_budget),COALESCE(l.reserved_tokens,0),COALESCE(l.reserved_requests,0),COALESCE(l.spent_tokens,0),COALESCE(l.spent_requests,0) FROM agent_runs r LEFT JOIN agent_budget_ledger l ON l.root_run_id=r.id WHERE r.id=?1",
            [root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .map_err(|error| {
            DecisionError::new(reason::BUDGET_EXCEEDED, format!("无法读取粗预算容量：{error}"))
        })?;
    let finite = |total: i64, reserved: i64, spent: i64| {
        if total < 0 || reserved < 0 || spent < 0 {
            return Err(DecisionError::new(reason::BUDGET_EXCEEDED, "预算容量为负"));
        }
        if total == 0 {
            return Ok(None);
        }
        total
            .checked_sub(reserved)
            .and_then(|value| value.checked_sub(spent))
            .map(|value| Some(value.max(0)))
            .ok_or_else(|| DecisionError::new(reason::BUDGET_EXCEEDED, "预算容量溢出"))
    };
    Ok((
        finite(tokens, reserved_tokens, spent_tokens)?,
        finite(requests, reserved_requests, spent_requests)?,
    ))
}

/// stop 硬校验纳入原共享账中 Root 本轮付费；错误拒单，不能当作耗尽。
pub fn budget_exhausted(connection: &Connection, root_run_id: &str) -> Result<bool, DecisionError> {
    let configured: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_budget_limits WHERE root_run_id=?1)",
            [root_run_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            DecisionError::new(
                reason::BUDGET_EXCEEDED,
                format!("无法核实原预算配置：{error}"),
            )
        })?;
    let (tokens, requests) = if configured {
        crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
            connection,
            root_run_id,
        )
        .map_err(|error| DecisionError::new(reason::BUDGET_EXCEEDED, error))?
    } else {
        coarse_budget_capacity(connection, root_run_id)?
    };
    Ok(tokens.is_some_and(|value| value <= 0) || requests.is_some_and(|value| value <= 0))
}

/// 合法 stop 是收尾，不能用任意非 goal stop 绕过首轮推进。
pub fn validate_stop(
    connection: &Connection,
    task: &types::HomogeneousTask,
    body: &types::StopPayload,
) -> Result<(), DecisionError> {
    let slices = store::load_slices(connection, &task.task_id)?;
    let legitimate = match body.stop_reason.as_str() {
        "goal_reached" => !body.accepted_slice_ids.is_empty() && crate::scanning::homogeneous_execution::goal_ready(connection,task).map_err(|e|illegal(e))?,
        "budget_exhausted" => budget_exhausted(connection, &task.root_run_id)?,
        "retry_limit_exceeded" => {
            !slices.is_empty()
                && slices.iter().all(|slice| {
                    slice.state == "accepted"
                        || slice.state == "cancelled"
                        || (slice.state == "rejected" && slice.retry_count >= slice.max_retries)
                })
                && slices.iter().any(|slice| slice.state == "rejected")
        }
        "degraded_path_used" => task.degraded,
        "user_cancelled" => connection
            .query_row(
                "SELECT status IN ('cancelled','cancelling') FROM sentinel_scans WHERE id=?1",
                [&task.scan_id],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false),
        _ => false,
    };
    if legitimate {
        Ok(())
    } else {
        Err(illegal(
            "stop_reason 未满足真实收尾条件，禁止用 stop 代替推进",
        ))
    }
}

pub fn audit(
    connection: &Connection,
    task: &types::HomogeneousTask,
    decision: &types::CoordinatorDecision,
    payload: &DecisionPayload,
    before: &IntentSnapshot,
    produced_intent: bool,
    lease_epoch: i64,
    trigger_reason_code: &str,
) -> Result<(), DecisionError> {
    let stop_reason = match payload {
        DecisionPayload::Stop(body) => body.stop_reason.as_str(),
        _ => "",
    };
    connection.execute(
        "INSERT INTO agent_homogeneous_decision_audit(decision_id,parent_task_id,action,stop_reason,zero_intent_json,intent_snapshot_json,produced_intent,decided_by_lease_epoch,trigger_reason_code) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![decision.decision_id, task.task_id, decision.action, stop_reason,
            serde_json::to_string(&decision.zero_intent).map_err(|error| illegal(error.to_string()))?,
            serde_json::to_string(before).map_err(|error| illegal(error.to_string()))?,
            produced_intent, lease_epoch, trigger_reason_code],
    ).map_err(|error| illegal(format!("意图审核落库失败：{error}")))?;
    Ok(())
}

pub fn latest_stop_reason(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<String>, DecisionError> {
    connection.query_row(
        "SELECT stop_reason FROM agent_homogeneous_decision_audit WHERE parent_task_id=?1 AND action='stop' ORDER BY audit_id DESC LIMIT 1",
        [task_id], |row| row.get(0),
    ).optional().map_err(|error| illegal(error.to_string()))
}
