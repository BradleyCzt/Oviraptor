//! DESIGN §6：状态迁移守卫。非法迁移必须整单拒绝且不写库。
//!
//! 表内容严格照 SPEC §3.1–§3.4 的允许迁移；同态推进（幂等重放）放行。
//! 拒绝原因一律用 SPEC 已有的 `allowlist_violation`——「当前上下文不允许这次副作用」，
//! 不为本类拒绝新增 reason_code。

use super::decision::DecisionError;
use super::{reason, types};
use rusqlite::{Connection, OptionalExtension};

type TransitionTable = &'static [(&'static str, &'static [&'static str])];

const TASK_TRANSITIONS: TransitionTable = &[
    ("created", &["decomposed", "cancelled", "stopped"]),
    ("decomposed", &["running", "cancelled", "stopped"]),
    (
        "running",
        &[
            "partial_success",
            "succeeded",
            "failed",
            "cancelled",
            "stopped",
        ],
    ),
    ("partial_success", &["running", "cancelled", "stopped"]),
    ("succeeded", &["stopped"]),
    ("failed", &["stopped"]),
    ("cancelled", &[]),
    ("stopped", &[]),
];

/// SPEC §3.2。`reported → assigned` 与 `rejected → assigned` 都是新 attempt 重派；
/// 明确禁止 `accepted → 任何`。
///
/// `assigned|in_progress → rejected` 只服务 DESIGN §6 选定的一条待核实结论
/// （原文：「slice 回 rejected 或保持可 retry……待核实后选『可 retry』」），
/// 用于租约到期回收：基础设施失败不是质量失败，故不动 retry_count。
const SLICE_TRANSITIONS: TransitionTable = &[
    ("pending", &["assigned", "cancelled"]),
    (
        "assigned",
        &["in_progress", "reported", "rejected", "cancelled"],
    ),
    ("in_progress", &["reported", "rejected", "cancelled"]),
    ("reported", &["accepted", "rejected", "cancelled"]),
    (
        "rejected",
        &["assigned", "in_progress", "reported", "cancelled"],
    ),
    ("accepted", &[]),
    ("cancelled", &[]),
];

/// SPEC §3.3。`rejected_retry → assigned` 是本实现里「同一行换新 attempt」的形态
/// （SPEC §1.4 的去重键含 slice_id，决定同一切片不能另起一行）。
const ASSIGNMENT_TRANSITIONS: TransitionTable = &[
    (
        "created",
        &["assigned", "running", "reported", "cancelled", "stopped"],
    ),
    ("assigned", &["running", "reported", "cancelled", "stopped"]),
    ("running", &["reported", "cancelled", "stopped"]),
    (
        "reported",
        &["accepted", "rejected_retry", "cancelled", "stopped"],
    ),
    (
        "rejected_retry",
        &["assigned", "running", "reported", "cancelled", "stopped"],
    ),
    ("accepted", &[]),
    ("stopped", &[]),
    ("cancelled", &[]),
];

/// 一张受守卫的状态表：STATE 词表、迁移表与所在物理表。
struct GuardedStates {
    kind: &'static str,
    table_name: &'static str,
    id_column: &'static str,
    words: &'static [&'static str],
    transitions: TransitionTable,
}

const TASK_STATES: GuardedStates = GuardedStates {
    kind: "task",
    table_name: "agent_homogeneous_tasks",
    id_column: "task_id",
    words: &types::TASK_STATES,
    transitions: TASK_TRANSITIONS,
};

const SLICE_STATES: GuardedStates = GuardedStates {
    kind: "slice",
    table_name: "agent_task_slices",
    id_column: "slice_id",
    words: &types::SLICE_STATES,
    transitions: SLICE_TRANSITIONS,
};

const ASSIGNMENT_STATES: GuardedStates = GuardedStates {
    kind: "assignment",
    table_name: "agent_assignments",
    id_column: "id",
    words: &types::ASSIGNMENT_STATES,
    transitions: ASSIGNMENT_TRANSITIONS,
};

fn is_legal(from: &str, to: &str, table: TransitionTable) -> bool {
    from == to
        || table
            .iter()
            .find(|(source, _)| *source == from)
            .map(|(_, targets)| targets.contains(&to))
            .unwrap_or(false)
}

fn check(
    connection: &Connection,
    states: &GuardedStates,
    id: &str,
    to: &str,
) -> Result<(), DecisionError> {
    let from: Option<String> = connection
        .query_row(
            &format!(
                "SELECT state FROM {} WHERE {}=?1",
                states.table_name, states.id_column
            ),
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            DecisionError::new(
                reason::SCHEMA_INVALID,
                format!("{} 状态不可读：{error}", states.kind),
            )
        })?;
    let from = from.ok_or_else(|| {
        DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            format!("{} 不可寻：{id}", states.kind),
        )
    })?;
    super::decision::validate_state_word(&from, states.words)?;
    if is_legal(&from, to, states.transitions) {
        return Ok(());
    }
    Err(DecisionError::new(
        reason::ALLOWLIST_VIOLATION,
        format!("{} 状态迁移不被允许：{from}→{to}", states.kind),
    ))
}

pub fn task(connection: &Connection, task_id: &str, to: &str) -> Result<(), DecisionError> {
    check(connection, &TASK_STATES, task_id, to)
}

pub fn slice(connection: &Connection, slice_id: &str, to: &str) -> Result<(), DecisionError> {
    check(connection, &SLICE_STATES, slice_id, to)
}

pub fn assignment(
    connection: &Connection,
    assignment_id: &str,
    to: &str,
) -> Result<(), DecisionError> {
    check(connection, &ASSIGNMENT_STATES, assignment_id, to)
}
