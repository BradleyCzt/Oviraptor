//! Typed repository for live `agent_assignments` (§5.2, §6).
//!
//! Role, lane, state, budget and lifecycle live in their own columns; only the task
//! slice and the reference lists are JSON. Every write redacts first, and every
//! failure comes back as `Err` — a caller can never lose an assignment to a
//! swallowed database error.

// The fenced scheduler and child lifecycle consume this repository. Some query
// helpers remain acceptance-test-only, so the reachability warning is expected.
#![allow(dead_code)]
use super::super::contract::{can_transition, AgentLane, AgentRole, AssignmentState};
use super::super::secrets::redact_json;
use super::super::store::decode_json;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

pub(crate) const ASSIGNMENT_COLUMNS: &str =
    "id,coordinator_run_id,child_run_id,role,lane,target_key,state,dedup_key,\
     trigger_code,task_slice_json,evidence_revision,contract_keys_json,identity_handles_json,\
     reserved_tokens,reserved_requests,capability_lease_json,deadline_at,failure_class,\
     created_at,leased_at,started_at,finished_at,updated_at";

#[derive(Clone, Debug, PartialEq)]
pub struct AgentAssignment {
    pub id: String,
    pub coordinator_run_id: String,
    pub child_run_id: String,
    pub role: AgentRole,
    pub lane: AgentLane,
    pub target_key: String,
    pub state: AssignmentState,
    pub dedup_key: String,
    pub trigger_code: String,
    pub task_slice: JsonValue,
    pub evidence_revision: i64,
    pub contract_keys: Vec<String>,
    pub identity_handles: Vec<String>,
    pub reserved_tokens: i64,
    pub reserved_requests: i64,
    pub capability_lease: Vec<String>,
    pub deadline_at: String,
    pub failure_class: String,
    pub created_at: String,
    pub leased_at: String,
    pub started_at: String,
    pub finished_at: String,
    pub updated_at: String,
}

impl AgentAssignment {
    /// A prepared slice for a role. The state machine starts at `prepared`; nothing
    /// in this type implies the assignment was ever leased or executed.
    pub fn new(
        id: impl Into<String>,
        coordinator_run_id: impl Into<String>,
        role: AgentRole,
        lane: AgentLane,
        target_key: impl Into<String>,
        dedup_key: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            coordinator_run_id: coordinator_run_id.into(),
            child_run_id: String::new(),
            role,
            lane,
            target_key: target_key.into(),
            state: AssignmentState::Prepared,
            dedup_key: dedup_key.into(),
            trigger_code: String::new(),
            task_slice: JsonValue::Object(serde_json::Map::new()),
            evidence_revision: 0,
            contract_keys: Vec::new(),
            identity_handles: Vec::new(),
            reserved_tokens: 0,
            reserved_requests: 0,
            capability_lease: Vec::new(),
            deadline_at: String::new(),
            failure_class: String::new(),
            created_at: String::new(),
            leased_at: String::new(),
            started_at: String::new(),
            finished_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// The first-plan identity of the row: everything a replay must reproduce
    /// exactly, and nothing the lifecycle writes afterwards (§3.4). Timestamps,
    /// state and the child run are deliberately absent.
    pub(crate) fn content_key(&self) -> JsonValue {
        redact_json(&serde_json::json!({
            "coordinatorRunId": self.coordinator_run_id,
            "role": self.role.as_str(),
            "lane": self.lane.as_str(),
            "targetKey": self.target_key,
            "dedupKey": self.dedup_key,
            "triggerCode": self.trigger_code,
            "taskSlice": self.task_slice,
            "evidenceRevision": self.evidence_revision,
            "contractKeys": self.contract_keys,
            "identityHandles": self.identity_handles,
            "reservedTokens": self.reserved_tokens,
            "reservedRequests": self.reserved_requests,
            "capabilityLease": self.capability_lease,
            "deadlineAt": self.deadline_at,
        }))
    }

    fn text_list(&self) -> Result<(String, String, String), String> {
        Ok((
            redact_json(&serde_json::json!(self.contract_keys)).to_string(),
            redact_json(&serde_json::json!(self.identity_handles)).to_string(),
            redact_json(&serde_json::json!(self.capability_lease)).to_string(),
        ))
    }
}

/// A stored row with the enum words and JSON texts kept as text: `decode_stored` is
/// the only place that judges them, so a corrupt value cannot become a default (§3.2).
pub(crate) struct StoredAssignment {
    assignment: AgentAssignment,
    role: String,
    lane: String,
    state: String,
    task_slice: String,
    contracts: String,
    identities: String,
    lease: String,
}

const TABLE: &str = "agent_assignments";

pub(crate) fn read_stored(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredAssignment> {
    Ok(StoredAssignment {
        assignment: AgentAssignment {
            id: row.get(0)?,
            coordinator_run_id: row.get(1)?,
            child_run_id: row.get(2)?,
            role: AgentRole::Coordinator,
            lane: AgentLane::ReadOnlyAnalysis,
            target_key: row.get(5)?,
            state: AssignmentState::Prepared,
            dedup_key: row.get(7)?,
            trigger_code: row.get(8)?,
            task_slice: JsonValue::Null,
            evidence_revision: row.get(10)?,
            contract_keys: Vec::new(),
            identity_handles: Vec::new(),
            reserved_tokens: row.get(13)?,
            reserved_requests: row.get(14)?,
            capability_lease: Vec::new(),
            deadline_at: row.get(16)?,
            failure_class: row.get(17)?,
            created_at: row.get(18)?,
            leased_at: row.get(19)?,
            started_at: row.get(20)?,
            finished_at: row.get(21)?,
            updated_at: row.get(22)?,
        },
        role: row.get(3)?,
        lane: row.get(4)?,
        state: row.get(6)?,
        task_slice: row.get(9)?,
        contracts: row.get(11)?,
        identities: row.get(12)?,
        lease: row.get(15)?,
    })
}

pub(crate) fn decode_stored(stored: StoredAssignment) -> Result<AgentAssignment, String> {
    let StoredAssignment {
        mut assignment,
        role,
        lane,
        state,
        task_slice,
        contracts,
        identities,
        lease,
    } = stored;
    let id = assignment.id.clone();
    assignment.role = AgentRole::try_parse(&role)
        .ok_or_else(|| format!("表 {TABLE} 的 role 不是已知角色（记录 {id}）：{role}"))?;
    assignment.lane = AgentLane::try_parse(&lane)
        .ok_or_else(|| format!("表 {TABLE} 的 lane 不是已知泳道（记录 {id}）：{lane}"))?;
    assignment.state = AssignmentState::try_parse(&state)
        .ok_or_else(|| format!("表 {TABLE} 的 state 不是已知生命周期（记录 {id}）：{state}"))?;
    assignment.task_slice = decode_json("task_slice_json", TABLE, &id, &task_slice)?;
    assignment.contract_keys = decode_json("contract_keys_json", TABLE, &id, &contracts)?;
    assignment.identity_handles = decode_json("identity_handles_json", TABLE, &id, &identities)?;
    assignment.capability_lease = decode_json("capability_lease_json", TABLE, &id, &lease)?;
    Ok(assignment)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssignmentInsert {
    /// Newly stored under this id.
    Inserted(String),
    /// The same dedup key already exists for this coordinator, so nothing was added.
    Existing(String),
}

/// Store one assignment slice. Replaying the same `(coordinator_run_id, dedup_key)`
/// returns the row that is already there instead of creating a second task for the
/// same work (§11.2); two coordinators may use the same dedup key. A replay whose
/// first-plan content differs is a conflict, never a silent `Existing` (§3.4).
pub fn insert_assignment(
    connection: &Connection,
    assignment: &AgentAssignment,
) -> Result<AssignmentInsert, String> {
    if assignment.id.trim().is_empty() {
        return Err("assignment 缺少 id".to_string());
    }
    if assignment.dedup_key.trim().is_empty() {
        return Err("assignment 缺少 dedup_key".to_string());
    }
    let slice = redact_json(&assignment.task_slice).to_string();
    let (contracts, identities, lease) = assignment.text_list()?;
    // An id that already belongs to another coordinator/dedup pair is a caller bug,
    // not a replay: answering `Existing` would silently drop this task.
    let owner: Option<(String, String)> = connection
        .query_row(
            "SELECT coordinator_run_id,dedup_key FROM agent_assignments WHERE id=?1",
            [&assignment.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("无法检查 assignment id：{error}"))?;
    if let Some((coordinator, dedup)) = owner {
        if coordinator != assignment.coordinator_run_id || dedup != assignment.dedup_key {
            return Err(format!(
                "assignment id 已被其它计划占用，拒绝写入：{}（已属于 {coordinator}/{dedup}）",
                assignment.id
            ));
        }
    }
    let inserted = connection
        .execute(
            &format!(
                "INSERT INTO agent_assignments({}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,datetime('now','localtime'),'',?,?,datetime('now','localtime')) ON CONFLICT(coordinator_run_id,dedup_key) DO NOTHING",
                ASSIGNMENT_COLUMNS
            ),
            params![
                assignment.id,
                assignment.coordinator_run_id,
                assignment.child_run_id,
                assignment.role.as_str(),
                assignment.lane.as_str(),
                assignment.target_key,
                assignment.state.as_str(),
                assignment.dedup_key,
                assignment.trigger_code,
                slice,
                assignment.evidence_revision,
                contracts,
                identities,
                assignment.reserved_tokens,
                assignment.reserved_requests,
                lease,
                assignment.deadline_at,
                assignment.failure_class,
                // created_at, leased_at and updated_at are stamped by the database; the
                // caller's copy is never trusted over the clock.
                assignment.started_at,
                assignment.finished_at,
            ],
        )
        .map_err(|error| format!("无法写入 assignment：{error}"))?;
    if inserted == 1 {
        return Ok(AssignmentInsert::Inserted(assignment.id.clone()));
    }
    let stored: Option<StoredAssignment> = connection
        .query_row(
                &format!(
                    "SELECT {ASSIGNMENT_COLUMNS} FROM agent_assignments WHERE coordinator_run_id=?1 AND dedup_key=?2"
                ),
                params![assignment.coordinator_run_id, assignment.dedup_key],
                read_stored,
            )
            .optional()
            .map_err(|error| format!("无法确认重放的 assignment：{error}"))?;
    let existing = decode_stored(stored.ok_or_else(|| {
        format!(
            "assignment 未写入且找不到同 dedup key 的记录：{}/{}",
            assignment.coordinator_run_id, assignment.dedup_key
        )
    })?)?;
    if existing.content_key() == assignment.content_key() {
        return Ok(AssignmentInsert::Existing(existing.id));
    }
    Err(format!(
        "assignment_dedup_conflict：同一 dedup key 已存在不同计划，拒绝覆盖：{}",
        assignment.dedup_key
    ))
}

pub fn load_assignment(
    connection: &Connection,
    id: &str,
) -> Result<Option<AgentAssignment>, String> {
    let stored = connection
        .query_row(
            &format!("SELECT {ASSIGNMENT_COLUMNS} FROM agent_assignments WHERE id=?1"),
            [id],
            read_stored,
        )
        .optional()
        .map_err(|error| format!("无法读取 assignment：{error}"))?;
    stored.map(decode_stored).transpose()
}

pub fn list_assignments_for_coordinator(
    connection: &Connection,
    coordinator_run_id: &str,
) -> Result<Vec<AgentAssignment>, String> {
    let mut statement = connection
        .prepare(
            &format!(
                "SELECT {ASSIGNMENT_COLUMNS} FROM agent_assignments WHERE coordinator_run_id=?1 AND scheduling_model='legacy_role_pipeline' ORDER BY created_at,id"
            ),
        )
        .map_err(|error| format!("无法准备 assignment 查询：{error}"))?;
    let rows = statement
        .query_map([coordinator_run_id], read_stored)
        .map_err(|error| format!("无法读取 assignment 列表：{error}"))?;
    rows.map(|item| {
        item.map_err(|error| format!("无法解析 assignment 列表：{error}"))
            .and_then(decode_stored)
    })
    .collect()
}

/// Compare-and-set the lifecycle. The stored state must still be `from`, and the
/// move must be legal; anything else leaves the row untouched and answers `false`
/// rather than rewriting history.
pub fn transition_assignment(
    connection: &Connection,
    id: &str,
    from: AssignmentState,
    to: AssignmentState,
) -> Result<bool, String> {
    if !can_transition(from, to) {
        return Err(format!(
            "不允许的 assignment 状态转换：{} -> {}",
            from.as_str(),
            to.as_str()
        ));
    }
    let stamped = match to {
        AssignmentState::Leased => ",leased_at=datetime('now','localtime')",
        AssignmentState::Running => ",started_at=datetime('now','localtime')",
        state if state.is_terminal() => ",finished_at=datetime('now','localtime')",
        _ => "",
    };
    let changed = connection
        .execute(
            &format!(
                "UPDATE agent_assignments SET state=?1{stamped},updated_at=datetime('now','localtime') \
                 WHERE id=?2 AND state=?3"
            ),
            params![to.as_str(), id, from.as_str()],
        )
        .map_err(|error| format!("无法更新 assignment 状态：{error}"))?;
    Ok(changed > 0)
}
