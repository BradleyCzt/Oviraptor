//! Explicit birth-sealed Phase 2 carrier. Business profile remains homogeneous_v1.
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
use super::delivery::MESSAGE_KIND;
/// 执行载体角色（现网既有只读角色）。
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
    super::execution_policy::require_on(connection, lease).map_err(carrier_error)?;
    let carrier_role = match assignment.lane.as_str() {
        "target_touching" => AgentRole::Authorization,
        "review" => AgentRole::EvidenceReviewer,
        _ => return Err(carrier_error("homogeneous_execution_lane_invalid".into())),
    };
    let role = carrier_role.as_str();
    let mut row = AgentRunRow::new(
        run_id.clone(),
        lease.scan_id.clone(),
        lease.attempt_number,
        lease.target_key.clone(),
        AgentBackendKind::Native,
        carrier_role,
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
    row.lane = Some(if assignment.lane == "review" {
        AgentLane::Review
    } else {
        AgentLane::TargetTouching
    });

    row.orchestration_policy = MultiAgentPolicy::Multi;
    row.capability_lease = assignment.capability_lease.clone();
    row.reserved_tokens = assignment.reserved_tokens.max(0);
    row.reserved_requests = assignment.reserved_requests.max(0);
    row.hard_token_budget = assignment.reserved_tokens.max(0);
    row.hard_request_budget = assignment.reserved_requests.max(0);
    // mailbox 的 live 校验要求 child run 自带未过期租约。
    row.lease_expires_at = lease.lease_expires_at.clone();
    run_store::create_run(connection, &row).map_err(carrier_error)?;
    connection
        .execute(
            "UPDATE agent_runs SET scheduling_model='homogeneous_workers' WHERE id=?1",
            [&run_id],
        )
        .map_err(|e| carrier_error(e.to_string()))?;

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
    connection.execute("INSERT INTO agent_lane_leases(scan_id,attempt_number,target_key,lane,assignment_id) VALUES(?1,?2,?3,?4,?5)",params![lease.scan_id,lease.attempt_number,lease.target_key,assignment.lane,assignment.assignment_id]).map_err(|e|carrier_error(e.to_string()))?;
    for capability in &assignment.capability_lease {
        connection.execute("INSERT INTO agent_capability_leases(id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,lease_expires_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![uuid::Uuid::new_v4().to_string(),lease.root_run_id,assignment.assignment_id,run_id,capability,lease.lease_epoch,lease.fencing_token,lease.lease_expires_at]).map_err(|e|carrier_error(e.to_string()))?;
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
