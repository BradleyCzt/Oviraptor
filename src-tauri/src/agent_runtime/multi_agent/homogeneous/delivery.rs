//! Phase 1 审核 P0-2：`assign` 之后的执行载体。
//!
//! 只写现网已有的 child run / assignment attempt / mailbox 行，
//! 不修改 `mailbox/authority.rs` 的路由规则；为了让同质行通过既有的
//! live 校验，`attempts::require_live_for_run` 只增加
//! `scheduling_model='homogeneous_workers'` 的判定分支，legacy 条件原样保留。
//!
//! 载体角色固定用现网既有的只读角色 `deep_investigator`
//! （`scheduler/child_contract.rs` 已把它绑在 `read_only_analysis` lane 上）。
//! 同质 Worker 本身仍是无固定角色的 `homogeneous_v1` 画像：这里只是执行容器，
//! 不是第 8 个角色。
//!
//! attempt 状态机由 `assignment_attempt_transition` 触发器把守（不可回退），
//! 因此一个 assignment 只保留一行 attempt：重派只刷新租约与到期时间，
//! 终态只在裁决落地时写入。

use super::decision::DecisionError;
use super::{reason, types};
use crate::agent_runtime::contract::{
    AgentBackendKind, AgentLane, AgentRole, AgentRunStatus, MultiAgentPolicy,
};
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use crate::agent_runtime::multi_agent::mailbox;
use crate::agent_runtime::store::{self as run_store, AgentRunRow};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

/// 下发切片任务的 mailbox 消息 kind。
pub const MESSAGE_KIND: &str = "homogeneous_slice_dispatch";
/// 执行载体角色（现网既有只读角色）。
const CARRIER_ROLE: AgentRole = AgentRole::DeepInvestigator;

/// 载体写入失败沿用 SPEC 已有的 `schema_invalid`（与 store 层写入失败同例），
/// detail 保留原始现网错误码，不新增 reason_code。
fn carrier_error(detail: String) -> DecisionError {
    DecisionError::new(reason::SCHEMA_INVALID, detail)
}

fn child_run_id(assignment_id: &str, attempt: i64) -> String {
    // 每个 attempt 一条新的 worker 身份：`agent_assignment_attempts` 只允许
    // 同一 assignment 存在一条 open attempt，且 child_run_id/worker_id 唯一。
    if attempt <= 1 {
        return format!("run-{assignment_id}");
    }
    format!("run-{assignment_id}-a{attempt}")
}

/// 建立/复用 child run 与 attempt，并把切片任务投进现有 mailbox。
/// 与调用方（`apply::apply_assign`）共用同一事务：任何一步失败都不会留下
/// 「已派发但无执行载体」的切片。
pub fn dispatch_slice(
    connection: &Connection,
    lease: &CoordinatorLease,
    assignment: &types::HomogeneousAssignment,
) -> Result<String, DecisionError> {
    let run_id = child_run_id(&assignment.assignment_id, assignment.attempt);
    let role = CARRIER_ROLE.as_str();
    let mut row = AgentRunRow::new(
        run_id.clone(),
        lease.scan_id.clone(),
        lease.attempt_number,
        lease.target_key.clone(),
        AgentBackendKind::Native,
        CARRIER_ROLE,
        String::new(),
        String::new(),
    );
    row.parent_run_id = Some(lease.root_run_id.clone());
    // 重派时 attempt 已在 running（状态机不许回退），child run 也必须停在 running，
    // 否则 mailbox 的 live 三元组对不上。
    row.status = match attempt_state(connection, &assignment.assignment_id, &lease.root_run_id)? {
        Some(state) if state == "running" => AgentRunStatus::Running,
        _ => AgentRunStatus::Prepared,
    };
    row.root_run_id = lease.root_run_id.clone();
    row.assignment_id = assignment.assignment_id.clone();
    row.lane = Some(AgentLane::ReadOnlyAnalysis);
    row.orchestration_policy = MultiAgentPolicy::Multi;
    row.capability_lease = assignment.capability_lease.clone();
    row.reserved_tokens = assignment.reserved_tokens.max(0);
    row.reserved_requests = assignment.reserved_requests.max(0);
    row.hard_token_budget = assignment.reserved_tokens.max(0);
    row.hard_request_budget = assignment.reserved_requests.max(0);
    // mailbox 的 live 校验要求 child run 自带未过期租约。
    row.lease_expires_at = lease.lease_expires_at.clone();
    run_store::create_run(connection, &row).map_err(carrier_error)?;

    let claimed = connection
        .execute(
            "UPDATE agent_assignments SET child_run_id=?1,role=?2,lease_epoch=?3,fencing_token=?4,lease_expires_at=?5,\
             deadline_at=CASE WHEN deadline_at='' THEN ?5 ELSE deadline_at END,\
             updated_at=datetime('now','localtime') \
             WHERE id=?6 AND scheduling_model='homogeneous_workers' AND state='assigned'",
            params![
                run_id,
                role,
                lease.lease_epoch,
                lease.fencing_token,
                lease.lease_expires_at,
                assignment.assignment_id
            ],
        )
        .map_err(|error| format!("homogeneous_dispatch_claim:{error}"))
        .map_err(carrier_error)?;
    if claimed != 1 {
        return Err(carrier_error("homogeneous_dispatch_claim_conflict".into()));
    }
    issue_attempt(
        connection,
        lease,
        &assignment.assignment_id,
        &run_id,
        assignment.attempt,
    )?;

    let payload: JsonValue = serde_json::json!({
        "assignmentId": assignment.assignment_id,
        "sliceId": assignment.slice_id,
        "scopeKind": assignment.scope_kind,
        "scopeValue": assignment.scope_value,
        "lane": assignment.lane,
        "attempt": assignment.attempt,
        "capabilityLease": assignment.capability_lease,
        "reservedTokens": assignment.reserved_tokens,
        "reservedRequests": assignment.reserved_requests,
        "evidenceRevisionBase": assignment.evidence_revision_base,
        "workerProfileId": assignment.worker_profile_id,
        "homogeneousWorkerId": assignment.worker_id,
    });
    let correlation = format!("{}:{}", assignment.assignment_id, assignment.attempt);
    mailbox::send(
        connection,
        lease,
        &lease.root_run_id,
        &run_id,
        "coordinator",
        role,
        MESSAGE_KIND,
        &correlation,
        &assignment.assignment_id,
        assignment.evidence_revision_base,
        &payload,
    )
    .map(|_| run_id)
    .map_err(carrier_error)
}

/// Worker 真正开跑：child run 与 attempt 同步进入 running，
/// 使 mailbox 的 live 校验在汇报前一直成立。
pub fn start_carrier(connection: &Connection, assignment_id: &str) -> Result<(), DecisionError> {
    let Some(run_id) = carrier_run(connection, assignment_id)? else {
        return Ok(());
    };
    let changed = connection
        .execute(
            "UPDATE agent_runs SET status='running',started_at=CASE WHEN started_at='' THEN datetime('now','localtime') ELSE started_at END,\
             updated_at=datetime('now','localtime') WHERE id=?1 AND backend='native' AND status IN ('prepared','running')",
            [run_id.as_str()],
        )
        .map_err(|error| format!("homogeneous_carrier_start:{error}"))
        .map_err(carrier_error)?;
    if changed != 1 {
        return Err(carrier_error("homogeneous_carrier_start_conflict".into()));
    }
    let promoted = connection
        .execute(
            "UPDATE agent_assignment_attempts SET state='running' WHERE child_run_id=?1 AND state='leased'",
            [run_id.as_str()],
        )
        .map_err(|error| format!("homogeneous_attempt_start:{error}"))
        .map_err(carrier_error)?;
    // 0 = 重派时 attempt 已是 running（状态机不允许回退，保持原样）。
    if promoted > 1 {
        return Err(carrier_error("homogeneous_attempt_start_ambiguous".into()));
    }
    Ok(())
}

/// 汇报落库：只确认下发消息。载体保持存活到裁决落地，
/// 因为 `reject_retry` 之后的重派必须复用同一 attempt 行。
pub fn report_carrier(connection: &Connection, assignment_id: &str) -> Result<(), DecisionError> {
    let Some(run_id) = carrier_run(connection, assignment_id)? else {
        return Ok(());
    };
    connection
        .execute(
            "UPDATE agent_messages SET acknowledged_at=datetime('now','localtime') \
             WHERE to_run_id=?1 AND kind=?2 AND acknowledged_at=''",
            params![run_id, MESSAGE_KIND],
        )
        .map_err(|error| format!("homogeneous_message_ack:{error}"))
        .map_err(carrier_error)?;
    Ok(())
}

/// 裁决落地（accept / stop / cancel / 到期回收）后关闭载体。
pub fn close_carrier(connection: &Connection, assignment_id: &str) -> Result<(), DecisionError> {
    let Some(run_id) = carrier_run(connection, assignment_id)? else {
        return Ok(());
    };
    let attempt: Option<(String, String, String)> = connection
        .query_row(
            "SELECT id,state,fencing_token FROM agent_assignment_attempts WHERE child_run_id=?1 AND state IN ('leased','running')",
            [run_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| format!("homogeneous_attempt_read:{error}"))
        .map_err(carrier_error)?;
    if let Some((attempt_id, state, fence)) = attempt {
        // 触发器允许 running→completed、leased→cancelled。
        let next = if state == "running" {
            "completed"
        } else {
            "cancelled"
        };
        let changed = connection
            .execute(
                "UPDATE agent_assignment_attempts SET state=?3,finished_at=datetime('now','localtime') WHERE id=?1 AND fencing_token=?2",
                params![attempt_id, fence, next],
            )
            .map_err(|error| format!("homogeneous_attempt_finish:{error}"))
            .map_err(carrier_error)?;
        if changed != 1 {
            return Err(carrier_error("homogeneous_attempt_finish_conflict".into()));
        }
    }
    connection
        .execute(
            "UPDATE agent_runs SET status='terminal',terminal_reason='settled',finished_at=datetime('now','localtime'),\
             updated_at=datetime('now','localtime') WHERE id=?1 AND backend='native' AND status<>'terminal'",
            [run_id.as_str()],
        )
        .map_err(|error| format!("homogeneous_carrier_finish:{error}"))
        .map_err(carrier_error)?;
    Ok(())
}

fn attempt_state(
    connection: &Connection,
    assignment_id: &str,
    root_run_id: &str,
) -> Result<Option<String>, DecisionError> {
    connection
        .query_row(
            "SELECT state FROM agent_assignment_attempts WHERE assignment_id=?1 AND root_run_id=?2",
            params![assignment_id, root_run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| carrier_error(format!("homogeneous_attempt_lookup:{error}")))
}

fn carrier_run(
    connection: &Connection,
    assignment_id: &str,
) -> Result<Option<String>, DecisionError> {
    let run: Option<String> = connection
        .query_row(
            "SELECT child_run_id FROM agent_assignments WHERE id=?1 AND scheduling_model='homogeneous_workers'",
            [assignment_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("homogeneous_carrier_read:{error}"))
        .map_err(carrier_error)?;
    Ok(run.filter(|value| !value.is_empty()))
}

/// 每个 attempt 写一行新的 worker 身份（与 `attempts::issue_first` 同构：
/// `lease_epoch` 就是 attempt 序号，满足 `UNIQUE(assignment_id,lease_epoch)`）。
/// 上一 attempt 必须已关闭（裁决落地或到期回收），否则拒绝重复下发。
fn issue_attempt(
    connection: &Connection,
    lease: &CoordinatorLease,
    assignment_id: &str,
    run_id: &str,
    attempt: i64,
) -> Result<(), DecisionError> {
    let open: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignment_attempts WHERE assignment_id=?1 AND state IN ('leased','running','paused')",
            [assignment_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("homogeneous_attempt_lookup:{error}"))
        .map_err(carrier_error)?;
    if open != 0 {
        return Err(carrier_error("homogeneous_attempt_already_open".into()));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let fence = uuid::Uuid::new_v4().to_string();
    let worker = uuid::Uuid::new_v4().to_string();
    let inserted = connection
        .execute(
            "INSERT INTO agent_assignment_attempts(id,root_run_id,assignment_id,child_run_id,\
             coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,state,expires_at)\
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'leased',?10)",
            params![
                id,
                lease.root_run_id,
                assignment_id,
                run_id,
                lease.lease_epoch,
                lease.fencing_token,
                attempt.max(1),
                fence,
                worker,
                lease.lease_expires_at
            ],
        )
        .map_err(|error| format!("homogeneous_attempt_issue:{error}"))
        .map_err(carrier_error)?;
    if inserted != 1 {
        return Err(carrier_error("homogeneous_attempt_issue_conflict".into()));
    }
    Ok(())
}
