//! DESIGN §5.1 / SPEC §0.6：本轮可执行动作子集 + 候选快照。
//!
//! 现状护栏是「suggestions 必须等于 frame.step()」，新路径把它换成
//! 「action ∈ allowed_actions」，且只对新的 frame kind 生效（DESIGN §5.4）。

use super::decision::DecisionError;
use super::{intent, reason, store, types};
use rusqlite::Connection;

/// DESIGN §7.3：任一条 streak 达到该值即禁止再付费问模型。
pub const STREAK_LIMIT: i64 = 2;

/// DESIGN §4.9：工具候选只能来自注册表真实串（SPEC 示例里的 `http.get` 在仓库中不存在，
/// 若模型照抄会被 tool_not_authorized 拒，这是期望行为）。
pub fn tool_allowlist_for_lane(lane: &str) -> Vec<String> {
    match lane {
        // 现状只读 child 的白名单（scheduler/child_contract.rs:38-47）
        // + Root 本地工具 snapshot.read（budget/root/model/tick/local.rs:162）。
        "read_only_analysis" => vec![
            "evidence.read".into(),
            "snapshot.read".into(),
            "mailbox.read".into(),
            "mailbox.write".into(),
        ],
        // Phase 1 明确不对这两条 lane 开放同质并行，候选仅为完整性存在。
        "target_touching" => vec![
            "evidence.read".into(),
            "mailbox.write".into(),
            "public_surface_get".into(),
            "authorization_probe".into(),
        ],
        "review" => vec!["evidence.read".into(), "review.write".into()],
        _ => Vec::new(),
    }
}

pub(crate) fn tool_allowlist_for_task(connection:&Connection,task:&types::HomogeneousTask,lane:&str)->Result<Vec<String>,DecisionError>{
    let mut tools=tool_allowlist_for_lane(lane);
    if lane=="read_only_analysis" && super::execution_policy::read_original_on(connection,&task.root_run_id)
        .map_err(|e|DecisionError::new(reason::TOOL_NOT_AUTHORIZED,e))?.is_some(){
        super::execution_worker::inventory(connection,&task.root_run_id).map_err(|e|DecisionError::new(reason::TOOL_NOT_AUTHORIZED,e))?;
        tools.push("source_analysis".into());
    }
    if lane=="target_touching" && super::execution_policy::read_original_on(connection,&task.root_run_id).map_err(|e|DecisionError::new(reason::TOOL_NOT_AUTHORIZED,e))?.is_some(){tools=vec!["authorization_probe".into()];}
    Ok(tools)
}

#[allow(clippy::type_complexity)]
fn pending_slice_ids(
    connection: &Connection,
    task_id: &str,
) -> Result<Vec<(String, String)>, DecisionError> {
    // gate 只要存在「还没走完的活」就允许模型提出 assign；
    // 是否真的可派（lane/配额/attempt/重复）由 apply 的硬护栏裁定。
    Ok(store::load_slices(connection, task_id)?
        .into_iter()
        .filter(|slice| slice.state != "accepted" && slice.state != "cancelled")
        .map(|slice| (slice.slice_id, slice.lane))
        .collect())
}

pub fn reported_assignment_ids(
    connection: &Connection,
    root_run_id: &str,
) -> Result<Vec<String>, DecisionError> {
    store::list_assignment_ids_in_state(connection, root_run_id, &["reported"])
}

fn executable_assignment_exists(
    connection: &Connection,
    root_run_id: &str,
) -> Result<bool, DecisionError> {
    let busy = store::list_assignment_ids_in_state(
        connection,
        root_run_id,
        &["created", "assigned", "running", "reported"],
    )?;
    Ok(!busy.is_empty())
}

/// 可被模型引用的 Worker：idle 用于 assign，busy 用于引导定向转发（SPEC §2.6 校验 4）。
fn referencable_workers(
    connection: &Connection,
    root_run_id: &str,
) -> Result<Vec<String>, DecisionError> {
    let mut ids = super::pool::worker_ids_in_state(connection, root_run_id, "idle")?;
    for busy in super::pool::worker_ids_in_state(connection, root_run_id, "busy")? {
        if !ids.contains(&busy) {
            ids.push(busy);
        }
    }
    ids.sort();
    Ok(ids)
}

/// DESIGN §6：当前可执行动作子集。空集合意味着「不该再问模型」。
pub fn allowed_actions(
    connection: &Connection,
    task: &types::HomogeneousTask,
) -> Result<Vec<String>, DecisionError> {
    if task.schema_fail_streak >= STREAK_LIMIT || task.hard_reject_streak >= STREAK_LIMIT {
        return Ok(Vec::new());
    }
    if types::is_member(
        &task.state,
        &["succeeded", "failed", "cancelled", "stopped"],
    ) {
        return Ok(Vec::new());
    }
    let slices = store::load_slices(connection, &task.task_id)?;
    if task.state == "created" && slices.is_empty() {
        // §J.4：有限冻结预算允许既有 stop 候选，apply_stop 再核真实耗尽。
        // 候选不纳入 Root 本轮收费，避免已冻结 basis 在 paid tick 中漂移。
        let (tokens, requests) = intent::coarse_budget_capacity(connection, &task.root_run_id)?;
        if tokens.is_some() || requests.is_some() {
            return Ok(vec!["decompose".to_string(), "stop".to_string()]);
        }
        return Ok(vec!["decompose".to_string()]);
    }
    let mut actions: Vec<String> = vec!["stop".to_string()];
    // §J 允许补方向；已验收的切片仍保持终态，追加只新增 pending 身份。
    if slices.iter().any(|slice| slice.state != "accepted") || slices.is_empty() || super::execution_policy::read_original_on(connection,&task.root_run_id).map_err(|e|DecisionError::new(reason::SCHEMA_INVALID,e))?.is_some() {
        actions.push("decompose".to_string());
    }
    let pending = pending_slice_ids(connection, &task.task_id)?;
    let idle_workers = super::pool::worker_ids_in_state(connection, &task.root_run_id, "idle")?;
    let (remaining_tokens, remaining_requests) =
        intent::coarse_budget_capacity(connection, &task.root_run_id)?;
    // gate 只做「粗粒度可用性」；lane 限制、配额与预算都是 apply 的硬护栏，
    // 这样被违反时对外报的是 SPEC 的 lane_quota_exceeded/budget_exceeded，
    // 而不是把对抗样本挡在动作集合之外（DESIGN 附录 C）。
    if !pending.is_empty()
        && !idle_workers.is_empty()
        && (remaining_tokens.map_or(true, |value| value > 0)
            || remaining_requests.map_or(true, |value| value > 0))
    {
        actions.insert(0, "assign".to_string());
    }
    if !reported_assignment_ids(connection, &task.root_run_id)?.is_empty() {
        actions.push("accept".to_string());
        actions.push("reject_retry".to_string());
    }
    if !store::awaiting_directive_ids(connection, &task.task_id)?.is_empty() {
        actions.push("relay_directive".to_string());
        actions.push("reject_directive".to_string());
    }
    actions.sort();
    actions.dedup();
    Ok(actions)
}

/// 六类候选 + 剩余预算 + basis_hash。模型只能从这里挑 id。
pub fn snapshot_for(
    connection: &Connection,
    task: &types::HomogeneousTask,
) -> Result<super::allowlist::AllowlistSnapshot, DecisionError> {
    snapshot_for_directives(connection, task, None)
}

// A paid frame may defer newly confirmed guidance until the next tick. Existing
// candidates, budgets and all original directives remain strictly unchanged.
pub fn snapshot_for_directives(
    connection: &Connection,
    task: &types::HomogeneousTask,
    frozen_directives: Option<&[String]>,
) -> Result<super::allowlist::AllowlistSnapshot, DecisionError> {
    let slices = store::load_slices(connection, &task.task_id)?;
    let pending: Vec<String> = slices
        .iter()
        .filter(|slice| slice.state == "pending" || slice.state == "rejected")
        .map(|slice| slice.slice_id.clone())
        .collect();
    let accepted: Vec<String> = slices.iter().map(|slice| slice.slice_id.clone()).collect();
    let workers = referencable_workers(connection, &task.root_run_id)?;
    let intent_state = intent::snapshot(connection, task)?;
    let assignments: Vec<String> = intent_state
        .inflight_assignments
        .iter()
        .map(|assignment| assignment.assignment_id.clone())
        .collect();
    let mut directives = store::awaiting_directive_ids(connection, &task.task_id)?;
    if let Some(frozen) = frozen_directives {
        if frozen.iter().any(|id| !directives.contains(id)) {
            return Err(DecisionError::new(reason::ALLOWLIST_VIOLATION, "原付费帧的引导候选发生变化"));
        }
        directives.retain(|id| frozen.contains(id));
    }
    let mut tools: Vec<String> = Vec::new();
    for lane in &types::LANES {
        for tool in tool_allowlist_for_task(connection,task,lane)? {
            if !tools.contains(&tool) {
                tools.push(tool);
            }
        }
    }
    let (remaining_tokens, remaining_requests) =
        store::remaining_budget(connection, &task.root_run_id)?;
    let mut basis = serde_json::json!({
        "task": task.task_id,
        "state": task.state,
        "schema_fail_streak": task.schema_fail_streak,
        "hard_reject_streak": task.hard_reject_streak,
        "slices": accepted,
        "pending": pending,
        "workers": workers,
        "assignments": assignments,
        "intent_state": intent_state,
        "slice_states": slices.iter().map(|slice| (&slice.slice_id, &slice.state, slice.retry_count, slice.max_retries)).collect::<Vec<_>>(),
        "directives": directives,
        "tools": tools,
        "targets": [task.target_key],
        "remaining_tokens": remaining_tokens,
        "remaining_requests": remaining_requests,
    });
    if let Some(map) = basis.as_object_mut() {
        map.insert(
            "revision".into(),
            serde_json::json!(task.evidence_revision_base),
        );
    }
    let basis_hash = crate::agent_runtime::store::stable_hash(&basis.to_string());
    Ok(super::allowlist::AllowlistSnapshot {
        // 候选切片含全部已分解切片：stop 需要引用 accepted/rejected 的 id。
        slice_ids: accepted,
        worker_ids: workers,
        tool_ids: tools,
        target_keys: vec![task.target_key.clone()],
        directive_ids: directives,
        assignment_ids: assignments,
        remaining_tokens,
        remaining_requests,
        basis_hash,
    })
}

pub fn unknown_context(code: &'static str, detail: impl Into<String>) -> DecisionError {
    DecisionError::new(code, detail)
}

pub fn not_found(detail: impl Into<String>) -> DecisionError {
    DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, detail)
}
