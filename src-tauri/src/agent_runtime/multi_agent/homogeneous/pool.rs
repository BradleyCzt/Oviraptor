//! DESIGN §4.7 的轻量 Worker 投影。
//!
//! 身份只在 `apply_assign` 的事务里签发（与现状 `attempts::issue_first` 同一纪律：
//! 读路径永不签发身份）。id 由 root_run + 序号确定性派生，满足 SPEC 的 `^wrk-` pattern。

use super::decision::DecisionError;
use super::{reason, store, types};
use crate::agent_runtime::store::stable_hash;
use rusqlite::{params, Connection, OptionalExtension};

pub const PHASE1_PROFILES: [&str; 1] = [types::PROFILE_HOMOGENEOUS_V1];

pub fn worker_id_for(root_run_id: &str, index: i64) -> String {
    format!(
        "wrk-{}",
        &stable_hash(&format!("wrk:{root_run_id}:{index}"))[..24]
    )
}

/// 为任务预置 `count` 个 idle Worker（Rust 侧 provisioning，模型只能挑 id）。
pub fn provision_workers(
    connection: &Connection,
    task: &types::HomogeneousTask,
    lease_epoch: i64,
    count: i64,
) -> Result<Vec<String>, DecisionError> {
    let limit = count.clamp(1, 16);
    let mut ids: Vec<String> = connection
        .prepare(
            "SELECT worker_id FROM agent_worker_instances WHERE root_run_id=?1 ORDER BY worker_id",
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
        .query_map([task.root_run_id.as_str()], |row| row.get::<_, String>(0))
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    // 历史 dead 身份保留审计，但不占现有容量；替换仍限制在既有配额内，绝不复活 dead。
    let mut live: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_worker_instances WHERE root_run_id=?1 AND state<>'dead'",
            [&task.root_run_id],
            |row| row.get(0),
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let mut index = 0i64;
    while live < limit {
        let candidate = worker_id_for(&task.root_run_id, index);
        index += 1;
        if ids.contains(&candidate) {
            continue;
        }
        let inserted = connection
            .execute(
                "INSERT INTO agent_worker_instances(worker_id,root_run_id,capability_profile_id,assignment_id,state,lease_epoch)\
                 VALUES(?1,?2,?3,'','idle',?4) ON CONFLICT(worker_id) DO NOTHING",
                params![candidate, task.root_run_id, types::PROFILE_HOMOGENEOUS_V1, lease_epoch.max(1)],
            )
            .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("Worker 投影写入失败：{error}")))?;
        if inserted == 1 {
            ids.push(candidate);
            live += 1;
        }
    }
    Ok(ids)
}

pub fn worker_ids_in_state(
    connection: &Connection,
    root_run_id: &str,
    state: &str,
) -> Result<Vec<String>, DecisionError> {
    super::decision::validate_state_word(state, &types::WORKER_STATES)?;
    let mut statement = connection
        .prepare("SELECT worker_id FROM agent_worker_instances WHERE root_run_id=?1 AND state=?2 ORDER BY worker_id")
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let rows = statement
        .query_map(params![root_run_id, state], |row| row.get::<_, String>(0))
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    rows.collect::<Result<Vec<String>, _>>()
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))
}

pub fn load_worker(
    connection: &Connection,
    worker_id: &str,
) -> Result<Option<types::WorkerInstance>, DecisionError> {
    let stored: Option<(String, String, String, String, i64)> = connection
        .query_row(
            "SELECT worker_id,capability_profile_id,assignment_id,state,lease_epoch FROM agent_worker_instances WHERE worker_id=?1",
            [worker_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::WORKER_NOT_FOUND, format!("Worker 不可读：{error}")))?;
    let Some((worker_id, capability_profile_id, assignment_id, state, lease_epoch)) = stored else {
        return Ok(None);
    };
    super::decision::validate_state_word(&state, &types::WORKER_STATES)?;
    if capability_profile_id != types::PROFILE_HOMOGENEOUS_V1 {
        return Err(DecisionError::new(
            reason::INVENTED_ROLE,
            format!("能力画像不在封闭集合内：{capability_profile_id}"),
        ));
    }
    Ok(Some(types::WorkerInstance {
        worker_id,
        capability_profile_id,
        assignment_id,
        state,
        lease_epoch,
    }))
}

/// SPEC §3.4：只允许 idle→busy、busy→idle、idle|busy→draining→dead；dead 不得复活。
pub fn transition_worker(
    connection: &Connection,
    worker_id: &str,
    from: &str,
    to: &str,
    assignment_id: &str,
) -> Result<(), DecisionError> {
    super::decision::validate_state_word(to, &types::WORKER_STATES)?;
    let allowed: &[&str] = match from {
        "idle" => &["busy", "draining", "dead"],
        "busy" => &["idle", "draining"],
        "draining" => &["dead"],
        _ => &[],
    };
    if !allowed.contains(&to) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("Worker 状态迁移非法：{from}→{to}"),
        ));
    }
    let changed = connection
        .execute(
            "UPDATE agent_worker_instances SET state=?1,assignment_id=?2,updated_at=datetime('now','localtime') WHERE worker_id=?3 AND state=?4",
            params![to, assignment_id, worker_id, from],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("Worker 状态写入失败：{error}")))?;
    if changed != 1 {
        return Err(DecisionError::new(
            reason::WORKER_NOT_FOUND,
            format!("Worker 不在 {from} 状态：{worker_id}"),
        ));
    }
    Ok(())
}

pub fn mark_busy(
    connection: &Connection,
    worker_id: &str,
    assignment_id: &str,
) -> Result<(), DecisionError> {
    transition_worker(connection, worker_id, "idle", "busy", assignment_id)
}

pub fn mark_idle(connection: &Connection, worker_id: &str) -> Result<(), DecisionError> {
    transition_worker(connection, worker_id, "busy", "idle", "")
}

/// DESIGN §6：租约到期/崩溃回收后 worker 终结；slot 释放，切片回可重试状态。
pub fn mark_dead(connection: &Connection, worker_id: &str) -> Result<(), DecisionError> {
    let current: String = connection
        .query_row(
            "SELECT state FROM agent_worker_instances WHERE worker_id=?1",
            [worker_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::WORKER_NOT_FOUND, error.to_string()))?
        .ok_or_else(|| {
            DecisionError::new(
                reason::WORKER_NOT_FOUND,
                format!("Worker 不可寻：{worker_id}"),
            )
        })?;
    if current == "dead" {
        return Ok(());
    }
    if current != "draining" {
        transition_worker(connection, worker_id, &current, "draining", "")?;
    }
    transition_worker(connection, worker_id, "draining", "dead", "")
}

pub fn current_assignment_of(
    connection: &Connection,
    worker_id: &str,
) -> Result<String, DecisionError> {
    connection
        .query_row(
            "SELECT assignment_id FROM agent_worker_instances WHERE worker_id=?1",
            [worker_id],
            |row| row.get(0),
        )
        .map_err(|error| {
            DecisionError::new(reason::WORKER_NOT_FOUND, format!("Worker 不可读：{error}"))
        })
}

/// P1-3：现网 worker 租约到期后的回收入口（DESIGN §6）。
/// 主管每一轮开始前先回收已过期的 busy 载体：释放 slot、切片回 rejected、
/// Worker 标 dead，然后才计算允许动作——否则过期 Worker 会一直虚占只读配额。
pub fn reclaim_expired_for_task(
    connection: &rusqlite::Connection,
    task: &types::HomogeneousTask,
) -> Result<Vec<String>, DecisionError> {
    let mut statement = connection
        .prepare(
            // 用同质 Worker 身份（wrk-*），attempt 行的 worker_id 是现网载体标识。
            "SELECT a.homogeneous_worker_id, a.id FROM agent_assignment_attempts x
             JOIN agent_assignments a
               ON a.id=x.assignment_id AND a.child_run_id=x.child_run_id
             WHERE a.coordinator_run_id=?1 AND a.scheduling_model='homogeneous_workers'
               AND a.state IN ('assigned','running')
               AND datetime(x.expires_at) <= datetime('now','localtime')
             ORDER BY x.expires_at, a.id",
        )
        .map_err(|error| {
            DecisionError::new(reason::SCHEMA_INVALID, format!("到期回收读取失败：{error}"))
        })?;
    let expired: Vec<(String, String)> = statement
        .query_map([task.root_run_id.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| {
            DecisionError::new(reason::SCHEMA_INVALID, format!("到期回收查询失败：{error}"))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            DecisionError::new(reason::SCHEMA_INVALID, format!("到期回收解码失败：{error}"))
        })?;
    drop(statement);
    let mut reclaimed = Vec::new();
    for (worker_id, assignment_id) in expired {
        reclaim_expired_worker(connection, task, &worker_id, &assignment_id)?;
        reclaimed.push(assignment_id);
    }
    Ok(reclaimed)
}

/// 现网心跳/扫描侧的等价入口：按 root_run_id 回收已过期的同质载体。
/// 非同质 root（没有同质任务）零副作用返回 0，legacy 语义完全不受影响。
pub fn reclaim_expired_for_root(
    connection: &Connection,
    root_run_id: &str,
) -> Result<usize, DecisionError> {
    let task = match store::load_task_by_root_run(connection, root_run_id)? {
        Some(task) => task,
        None => return Ok(0),
    };
    Ok(reclaim_expired_for_task(connection, &task)?.len())
}

/// 租约到期回收：assignment→cancelled，slot 释放，切片回 rejected（不计质量失败，
/// 因此不动 retry_count——DESIGN §6 的取值），并关闭它的执行载体。
pub fn reclaim_expired_worker(
    connection: &Connection,
    task: &types::HomogeneousTask,
    worker_id: &str,
    assignment_id: &str,
) -> Result<(), DecisionError> {
    let transaction =
        rusqlite::Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let connection: &Connection = &transaction;
    let assignment = store::load_assignment(connection, assignment_id)?
        .ok_or_else(|| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, "assignment 不可寻"))?;
    if assignment.parent_task_id != task.task_id || assignment.worker_id != worker_id {
        return Err(DecisionError::new(
            reason::CROSS_SLICE_WRITE,
            "回收身份不属于本任务/Worker",
        ));
    }
    // 同一到期载体重复回收必须零副作用，特别是不能重复释放预算。
    if !["assigned", "running"].contains(&assignment.state.as_str()) {
        return Ok(());
    }
    store::set_assignment_state(connection, assignment_id, "cancelled")?;
    store::release_slot(
        connection,
        &task.root_run_id,
        &assignment.lane,
        &assignment.slice_id,
    )?;
    store::set_slice_state(connection, &assignment.slice_id, "rejected")?;
    // 载体随取消一起关闭（attempt→cancelled、run→terminal），否则租约槽位会虚占。
    super::delivery::close_carrier(connection, assignment_id)?;
    mark_dead(connection, worker_id)?;
    store::settle_budget(
        connection,
        &task.root_run_id,
        assignment.reserved_tokens,
        assignment.reserved_requests,
        0,
        0,
    )?;
    super::budget_bridge::reclaim(&transaction, task, &assignment)?;
    transaction
        .commit()
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    Ok(())
}
