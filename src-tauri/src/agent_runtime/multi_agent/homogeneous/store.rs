//! DESIGN §4 的落库层。这里只承认 `scheduling_model='homogeneous_workers'` 的行，
//! 并且永远不写旧 `agent_lane_leases`（DESIGN §5.1：避开 UNIQUE(assignment_id) 与互斥 PK）。

use super::decision::DecisionError;
use super::{reason, types};
use crate::agent_runtime::multi_agent::lease::CoordinatorLease;
use crate::agent_runtime::store::stable_hash;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

pub const HOMOGENEOUS_TRIGGER_CODE: &str = "homogeneous-dispatch";

/// 行原文元组：解码只在一处判定，损坏值不当默认值（DESIGN §3.2）。
type SliceRow = (
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    i64,
    i64,
    String,
);
type ReportRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    String,
);

fn hash24(input: &str) -> String {
    stable_hash(input)[..24].to_string()
}

pub fn task_id_for(root_run_id: &str) -> String {
    format!("tsk-{}", hash24(&format!("task:{root_run_id}")))
}

pub fn assignment_id_for(root_run_id: &str, dedup_key: &str) -> String {
    format!("asg-{}", hash24(&format!("{root_run_id}:{dedup_key}")))
}

pub fn verdict_id_for(assignment_id: &str, verdict: &str, attempt: i64) -> String {
    format!(
        "vrd-{}",
        hash24(&format!("verdict:{assignment_id}:{verdict}:{attempt}"))
    )
}

pub fn report_id_for(assignment_id: &str, attempt: i64) -> String {
    format!(
        "rpt-{}",
        hash24(&format!("report:{assignment_id}:{attempt}"))
    )
}

/// SPEC §1.4 目标态去重键：`profile:trigger:target:revision:slice_id`。
pub fn dedup_key_for(
    slice_id: &str,
    target_key: &str,
    evidence_revision: i64,
    trigger_code: &str,
) -> String {
    format!(
        "{}:{trigger_code}:{target_key}:{evidence_revision}:{slice_id}",
        types::PROFILE_HOMOGENEOUS_V1
    )
}

#[derive(Clone, Debug)]
pub enum TaskInsert {
    Created(String),
    Existing(String),
}

#[allow(clippy::too_many_arguments)]
pub fn create_task(
    connection: &Connection,
    root_run_id: &str,
    scan_id: &str,
    attempt_number: i64,
    target_key: &str,
    scheduling_model: &str,
    evidence_revision_base: i64,
    allowed_parallel: i64,
) -> Result<TaskInsert, String> {
    if !types::is_member(
        scheduling_model,
        &[types::SCHEDULING_LEGACY, types::SCHEDULING_HOMOGENEOUS],
    ) {
        return Err(format!("未知 scheduling_model：{scheduling_model}"));
    }
    let task_id = task_id_for(root_run_id);
    let inserted = connection
        .execute(
            "INSERT INTO agent_homogeneous_tasks(task_id,root_run_id,scan_id,attempt_number,target_key,scheduling_model,state,evidence_revision_base,allowed_parallel) \
             VALUES(?1,?2,?3,?4,?5,?6,'created',?7,?8) ON CONFLICT(root_run_id) DO NOTHING",
            params![task_id, root_run_id, scan_id, attempt_number, target_key, scheduling_model, evidence_revision_base, allowed_parallel],
        )
        .map_err(|error| format!("无法创建同质任务：{error}"))?;
    if inserted == 1 {
        return Ok(TaskInsert::Created(task_id));
    }
    let existing: String = connection
        .query_row(
            "SELECT task_id FROM agent_homogeneous_tasks WHERE root_run_id=?1",
            [root_run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法读取同质任务：{error}"))?
        .ok_or_else(|| "同质任务未写入且找不到同一 root_run 的记录".to_string())?;
    Ok(TaskInsert::Existing(existing))
}

#[allow(clippy::type_complexity)]
pub fn load_task(
    connection: &Connection,
    task_id: &str,
) -> Result<Option<types::HomogeneousTask>, DecisionError> {
    let stored: Option<(String, String, String, i64, String, String, String, i64, i64, i64, String, i64, i64, i64)> = connection
        .query_row(
            "SELECT task_id,root_run_id,scan_id,attempt_number,target_key,scheduling_model,state,evidence_revision_base,\
             allowed_parallel,degraded,degrade_reason_code,schema_fail_streak,hard_reject_streak,decided_by_lease_epoch \
             FROM agent_homogeneous_tasks WHERE task_id=?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?)),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, format!("任务不可读：{error}")))?;
    let Some((
        task_id,
        root_run_id,
        scan_id,
        attempt_number,
        target_key,
        scheduling_model,
        state,
        evidence_revision_base,
        allowed_parallel,
        degraded,
        degrade_reason_code,
        schema_fail_streak,
        hard_reject_streak,
        decided_by_lease_epoch,
    )) = stored
    else {
        return Ok(None);
    };
    super::decision::validate_state_word(&state, &types::TASK_STATES)?;
    Ok(Some(types::HomogeneousTask {
        task_id,
        root_run_id,
        scan_id,
        attempt_number,
        target_key,
        scheduling_model,
        state,
        evidence_revision_base,
        allowed_parallel,
        degraded: degraded != 0,
        degrade_reason_code,
        schema_fail_streak,
        hard_reject_streak,
        decided_by_lease_epoch,
    }))
}

pub fn load_task_by_root_run(
    connection: &Connection,
    root_run_id: &str,
) -> Result<Option<types::HomogeneousTask>, DecisionError> {
    let task_id: Option<String> = connection
        .query_row(
            "SELECT task_id FROM agent_homogeneous_tasks WHERE root_run_id=?1",
            [root_run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, format!("任务不可读：{error}"))
        })?;
    match task_id {
        Some(id) => load_task(connection, &id),
        None => Ok(None),
    }
}

/// DESIGN §0.1 / §2.2：任务级开关。历史 run 一律 `legacy_role_pipeline`，
/// 只有显式写成 `homogeneous_workers` 的 run 才建同质任务并走新主管闭环。
pub fn run_scheduling_model(connection: &Connection, root_run_id: &str) -> Result<String, String> {
    connection
        .query_row(
            "SELECT scheduling_model FROM agent_runs WHERE id=?1",
            [root_run_id],
            |row| row.get(0),
        )
        .optional()
        .map(|value| value.unwrap_or_else(|| types::SCHEDULING_LEGACY.to_string()))
        .map_err(|error| format!("无法读取 scheduling_model：{error}"))
}

pub fn set_run_scheduling_model(
    connection: &Connection,
    root_run_id: &str,
    model: &str,
) -> Result<(), String> {
    if !types::is_member(
        model,
        &[types::SCHEDULING_LEGACY, types::SCHEDULING_HOMOGENEOUS],
    ) {
        return Err(format!("未知 scheduling_model：{model}"));
    }
    let changed = connection
        .execute(
            "UPDATE agent_runs SET scheduling_model=?1,updated_at=datetime('now','localtime') WHERE id=?2",
            params![model, root_run_id],
        )
        .map_err(|error| format!("无法写入 scheduling_model：{error}"))?;
    if changed != 1 {
        return Err(format!("agent_runs 不可寻：{root_run_id}"));
    }
    Ok(())
}

/// 任务级开关的取值来源：冻结扫描计划里的 `scheduling_model`（DESIGN §2.2「任务创建时写入」）。
/// 计划缺字段、值非法或非计划任务一律回落 legacy，绝不让缺省配置打开新路径。
pub fn scheduling_model_from_plan(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
) -> Result<&'static str, String> {
    let declared: Option<String> = connection
        .query_row(
            "SELECT json_extract(plan_json,'$.scheduling_model') FROM native_scan_plans WHERE scan_id=?1 AND attempt_number=?2",
            params![scan_id, attempt_number],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("无法读取冻结计划的 scheduling_model：{error}"))?
        .flatten();
    Ok(match declared.as_deref() {
        Some(model) if model == types::SCHEDULING_HOMOGENEOUS => types::SCHEDULING_HOMOGENEOUS,
        _ => types::SCHEDULING_LEGACY,
    })
}

/// Coordinator 激活后调用：run 显式 homogeneous 才建任务，否则零副作用。
pub fn activate_task_for_lease(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<Option<String>, String> {
    if run_scheduling_model(connection, &lease.root_run_id)? != types::SCHEDULING_HOMOGENEOUS {
        return Ok(None);
    }
    let base: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(revision),0) FROM agent_evidence_revisions WHERE root_run_id=?1",
            [lease.root_run_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|error| format!("无法读取证据版本：{error}"))?;
    match create_task(
        connection,
        &lease.root_run_id,
        &lease.scan_id,
        lease.attempt_number,
        &lease.target_key,
        types::SCHEDULING_HOMOGENEOUS,
        base,
        3,
    )? {
        TaskInsert::Created(task_id) | TaskInsert::Existing(task_id) => Ok(Some(task_id)),
    }
}

pub fn load_task_by_target(
    connection: &Connection,
    scan_id: &str,
    attempt_number: i64,
    target_key: &str,
) -> Result<Option<types::HomogeneousTask>, DecisionError> {
    let task_id: Option<String> = connection
        .query_row(
            "SELECT task_id FROM agent_homogeneous_tasks WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND scheduling_model='homogeneous_workers'",
            params![scan_id, attempt_number, target_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, format!("任务不可读：{error}")))?;
    match task_id {
        Some(id) => load_task(connection, &id),
        None => Ok(None),
    }
}

/// DESIGN §5.5：主管已把驳回四字段投影给用户，供 WebExecutor 判定「本轮可继续」。
pub fn reject_feedback_projected(
    connection: &Connection,
    root_run_id: &str,
) -> Result<bool, String> {
    let task_id: Option<String> = connection
        .query_row(
            "SELECT task_id FROM agent_homogeneous_tasks WHERE root_run_id=?1",
            [root_run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(task_id) = task_id else {
        return Ok(false);
    };
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_human_directive_gated WHERE parent_task_id=?1 AND state='rejected' \
             AND coordinator_action='reject' AND reason_code<>'' AND reason_zh<>'' AND suggestion_zh<>'' AND user_feedback_zh<>'')",
            [task_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

/// 同质路径下「本轮引导是否都已由主管裁决」：relay/rewrite/reject 之外的引导仍在等待。
pub fn all_directives_decided(connection: &Connection, root_run_id: &str) -> Result<bool, String> {
    let task_id: Option<String> = connection
        .query_row(
            "SELECT task_id FROM agent_homogeneous_tasks WHERE root_run_id=?1",
            [root_run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(task_id) = task_id else {
        return Ok(false);
    };
    let pending: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_human_directive_gated WHERE parent_task_id=?1 \
             AND coordinator_action='' AND state IN ('received','rule_screened','awaiting_coordinator')",
            [task_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(pending == 0)
}

pub fn set_task_state(
    connection: &Connection,
    task_id: &str,
    state: &str,
    lease_epoch: i64,
) -> Result<(), DecisionError> {
    super::decision::validate_state_word(state, &types::TASK_STATES)?;
    super::transitions::task(connection, task_id, state)?;
    connection
        .execute(
            "UPDATE agent_homogeneous_tasks SET state=?1,decided_by_lease_epoch=?2,updated_at=datetime('now','localtime') WHERE task_id=?3",
            params![state, lease_epoch, task_id],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("任务状态写入失败：{error}")))?;
    Ok(())
}

pub fn set_task_degraded(
    connection: &Connection,
    task_id: &str,
    reason_code: &str,
) -> Result<(), DecisionError> {
    if !reason::is_valid(reason_code) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "降级码不在封闭集合内",
        ));
    }
    connection
        .execute(
            "UPDATE agent_homogeneous_tasks SET degraded=1,degrade_reason_code=?1,updated_at=datetime('now','localtime') WHERE task_id=?2",
            params![reason_code, task_id],
        )
        .map_err(|error| DecisionError::new(reason::DEGRADED_PATH_USED, format!("降级标注失败：{error}")))?;
    Ok(())
}

pub fn bump_schema_fail_streak(connection: &Connection, task_id: &str) -> Result<i64, String> {
    connection.execute(
        "UPDATE agent_homogeneous_tasks SET schema_fail_streak=schema_fail_streak+1,updated_at=datetime('now','localtime') WHERE task_id=?1",
        [task_id],
    ).map_err(|error| format!("无法累计 schema 失败：{error}"))?;
    read_streaks(connection, task_id).map(|(schema, _)| schema)
}

pub fn bump_hard_reject_streak(connection: &Connection, task_id: &str) -> Result<i64, String> {
    connection.execute(
        "UPDATE agent_homogeneous_tasks SET hard_reject_streak=hard_reject_streak+1,updated_at=datetime('now','localtime') WHERE task_id=?1",
        [task_id],
    ).map_err(|error| format!("无法累计硬拒：{error}"))?;
    read_streaks(connection, task_id).map(|(_, hard)| hard)
}

/// DESIGN §7.6：一次合法成功的决策把两条 streak 都清零。
pub fn clear_streaks(connection: &Connection, task_id: &str) -> Result<(), String> {
    connection
        .execute(
            "UPDATE agent_homogeneous_tasks SET schema_fail_streak=0,hard_reject_streak=0,updated_at=datetime('now','localtime') WHERE task_id=?1",
            [task_id],
        )
        .map_err(|error| format!("无法清零 streak：{error}"))?;
    Ok(())
}

pub fn read_streaks(connection: &Connection, task_id: &str) -> Result<(i64, i64), String> {
    connection
        .query_row(
            "SELECT schema_fail_streak,hard_reject_streak FROM agent_homogeneous_tasks WHERE task_id=?1",
            [task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("无法读取 streak：{error}"))
}

pub fn insert_slices(
    connection: &Connection,
    parent_task_id: &str,
    drafts: &[types::SliceDraft],
    decomposition_model: &str,
) -> Result<Vec<String>, DecisionError> {
    if !types::is_member(decomposition_model, &["coordinator", "deterministic"]) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "decomposition_model 非法",
        ));
    }
    let mut ids = Vec::with_capacity(drafts.len());
    for draft in drafts {
        connection
            .execute(
                "INSERT INTO agent_task_slices(slice_id,parent_task_id,scope_kind,scope_value,evidence_revision_base,lane,worker_capability_profile_id,state,max_retries,retry_count,decomposition_model) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7,'pending',?8,0,?9) ON CONFLICT(slice_id) DO NOTHING",
                params![
                    draft.slice_id,
                    parent_task_id,
                    draft.scope_kind,
                    draft.scope_value,
                    draft.evidence_revision_base,
                    draft.lane,
                    draft.worker_capability_profile_id,
                    draft.max_retries,
                    decomposition_model
                ],
            )
            .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("切片写入失败：{error}")))?;
        ids.push(draft.slice_id.clone());
    }
    Ok(ids)
}

pub fn load_slice(
    connection: &Connection,
    slice_id: &str,
) -> Result<Option<types::TaskSlice>, DecisionError> {
    let stored: Option<SliceRow> = connection
        .query_row(
            "SELECT slice_id,parent_task_id,scope_kind,scope_value,evidence_revision_base,lane,worker_capability_profile_id,state,max_retries,retry_count,decomposition_model \
             FROM agent_task_slices WHERE slice_id=?1",
            [slice_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?)),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("切片不可读：{error}")))?;
    let Some((
        slice_id,
        parent_task_id,
        scope_kind,
        scope_value,
        evidence_revision_base,
        lane,
        worker_capability_profile_id,
        state,
        max_retries,
        retry_count,
        decomposition_model,
    )) = stored
    else {
        return Ok(None);
    };
    super::decision::validate_state_word(&state, &types::SLICE_STATES)?;
    Ok(Some(types::TaskSlice {
        slice_id,
        parent_task_id,
        scope_kind,
        scope_value,
        evidence_revision_base,
        lane,
        worker_capability_profile_id,
        state,
        max_retries,
        retry_count,
        decomposition_model,
    }))
}

pub fn load_slices(
    connection: &Connection,
    parent_task_id: &str,
) -> Result<Vec<types::TaskSlice>, DecisionError> {
    let mut statement = connection
        .prepare(
            "SELECT slice_id FROM agent_task_slices WHERE parent_task_id=?1 ORDER BY scope_value",
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let ids = statement
        .query_map([parent_task_id], |row| row.get::<_, String>(0))
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    drop(statement);
    let mut slices = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(slice) = load_slice(connection, &id)? {
            slices.push(slice);
        }
    }
    Ok(slices)
}

pub fn set_slice_state(
    connection: &Connection,
    slice_id: &str,
    state: &str,
) -> Result<(), DecisionError> {
    super::decision::validate_state_word(state, &types::SLICE_STATES)?;
    super::transitions::slice(connection, slice_id, state)?;
    let changed = connection
        .execute(
            "UPDATE agent_task_slices SET state=?1,updated_at=datetime('now','localtime') WHERE slice_id=?2",
            params![state, slice_id],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("切片状态写入失败：{error}")))?;
    if changed != 1 {
        return Err(DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            format!("切片不可寻：{slice_id}"),
        ));
    }
    Ok(())
}

pub fn bump_slice_retry(connection: &Connection, slice_id: &str) -> Result<(), DecisionError> {
    let changed = connection
        .execute(
            "UPDATE agent_task_slices SET retry_count=retry_count+1,updated_at=datetime('now','localtime') WHERE slice_id=?1 AND retry_count<max_retries",
            [slice_id],
        )
        .map_err(|error| DecisionError::new(reason::RETRY_LIMIT_EXCEEDED, format!("重试计数写入失败：{error}")))?;
    if changed != 1 {
        return Err(DecisionError::new(
            reason::RETRY_LIMIT_EXCEEDED,
            "已达 max_retries，不得再重试",
        ));
    }
    Ok(())
}

pub fn cancel_leftover_slices(
    connection: &Connection,
    parent_task_id: &str,
) -> Result<usize, DecisionError> {
    connection
        .execute(
            "UPDATE agent_task_slices SET state='cancelled',updated_at=datetime('now','localtime') WHERE parent_task_id=?1 AND state IN ('pending','assigned','in_progress','reported','rejected')",
            [parent_task_id],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("残留切片收尾失败：{error}")))
}

/// DESIGN §4.4：Phase 1 只有只读 lane 可以扩量，其余恒为 1。
pub fn upsert_quota(
    connection: &Connection,
    root_run_id: &str,
    lane: &str,
    max_parallel: i64,
) -> Result<i64, DecisionError> {
    if !types::is_member(lane, &types::LANES) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            format!("lane 非法：{lane}"),
        ));
    }
    let effective = if lane == "read_only_analysis" {
        max_parallel.clamp(1, 16)
    } else {
        1
    };
    connection
        .execute(
            "INSERT INTO agent_lane_quotas(root_run_id,lane,max_parallel) VALUES(?1,?2,?3) ON CONFLICT(root_run_id,lane) DO UPDATE SET max_parallel=excluded.max_parallel",
            params![root_run_id, lane, effective],
        )
        .map_err(|error| DecisionError::new(reason::LANE_QUOTA_EXCEEDED, format!("配额写入失败：{error}")))?;
    Ok(effective)
}

pub fn quota_limit(
    connection: &Connection,
    root_run_id: &str,
    lane: &str,
) -> Result<i64, DecisionError> {
    connection
        .query_row(
            "SELECT max_parallel FROM agent_lane_quotas WHERE root_run_id=?1 AND lane=?2",
            params![root_run_id, lane],
            |row| row.get(0),
        )
        .optional()
        .map(|value| value.unwrap_or(1))
        .map_err(|error| {
            DecisionError::new(
                reason::LANE_QUOTA_EXCEEDED,
                format!("无法读取 lane 配额：{error}"),
            )
        })
}

pub fn active_slot_count(
    connection: &Connection,
    root_run_id: &str,
    lane: &str,
) -> Result<i64, DecisionError> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM agent_lane_slots WHERE root_run_id=?1 AND lane=?2 AND released_at=''",
            params![root_run_id, lane],
            |row| row.get(0),
        )
        .map_err(|error| DecisionError::new(reason::LANE_QUOTA_EXCEEDED, format!("无法统计 lane 占用：{error}")))
}

pub fn take_slot(
    connection: &Connection,
    root_run_id: &str,
    lane: &str,
    slice_id: &str,
    assignment_id: &str,
) -> Result<(), DecisionError> {
    let holder: Option<(String, String)> = connection
        .query_row(
            "SELECT assignment_id,released_at FROM agent_lane_slots WHERE root_run_id=?1 AND lane=?2 AND slice_id=?3",
            params![root_run_id, lane, slice_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::LANE_QUOTA_EXCEEDED, error.to_string()))?;
    if let Some((owner, released_at)) = holder {
        if owner != assignment_id {
            return Err(DecisionError::new(
                reason::DUPLICATE_SLICE,
                format!("同一切片已被 {owner} 占用"),
            ));
        }
        if released_at.is_empty() {
            return Ok(());
        }
        // 新 attempt 复用同一行槽位：必须重新占用，否则配额统计会漏算（并发超发）。
        let changed = connection
            .execute(
                "UPDATE agent_lane_slots SET released_at='',acquired_at=datetime('now','localtime') \
                 WHERE root_run_id=?1 AND lane=?2 AND slice_id=?3 AND assignment_id=?4 AND released_at<>''",
                params![root_run_id, lane, slice_id, assignment_id],
            )
            .map_err(|error| DecisionError::new(reason::LANE_QUOTA_EXCEEDED, format!("重占槽位失败：{error}")))?;
        if changed != 1 {
            return Err(DecisionError::new(
                reason::LANE_QUOTA_EXCEEDED,
                "槽位重占未生效",
            ));
        }
        return Ok(());
    }
    let limit = quota_limit(connection, root_run_id, lane)?;
    let active = active_slot_count(connection, root_run_id, lane)?;
    if active >= limit {
        return Err(DecisionError::new(
            reason::LANE_QUOTA_EXCEEDED,
            format!("lane {lane} 并发已满：{active}/{limit}"),
        ));
    }
    connection
        .execute(
            "INSERT INTO agent_lane_slots(root_run_id,lane,slice_id,assignment_id) VALUES(?1,?2,?3,?4) ON CONFLICT(root_run_id,lane,slice_id) DO NOTHING",
            params![root_run_id, lane, slice_id, assignment_id],
        )
        .map_err(|error| DecisionError::new(reason::LANE_QUOTA_EXCEEDED, format!("占用失败：{error}")))?;
    Ok(())
}

pub fn release_slot(
    connection: &Connection,
    root_run_id: &str,
    lane: &str,
    slice_id: &str,
) -> Result<(), DecisionError> {
    connection
        .execute(
            "UPDATE agent_lane_slots SET released_at=datetime('now','localtime') WHERE root_run_id=?1 AND lane=?2 AND slice_id=?3 AND released_at=''",
            params![root_run_id, lane, slice_id],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("释放失败：{error}")))?;
    Ok(())
}

pub fn release_all_slots(
    connection: &Connection,
    root_run_id: &str,
) -> Result<usize, DecisionError> {
    connection
        .execute(
            "UPDATE agent_lane_slots SET released_at=datetime('now','localtime') WHERE root_run_id=?1 AND released_at=''",
            [root_run_id],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("批量释放失败：{error}")))
}

/// 复用现状 `initialize_child_budget` 的 SQL 形状：总额取 agent_runs 硬帽。
pub fn ensure_ledger(
    connection: &Connection,
    lease: &CoordinatorLease,
) -> Result<(), DecisionError> {
    connection
        .execute(
            "INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,reserved_tokens,reserved_requests,spent_tokens,spent_requests,lease_epoch,fencing_token) \
             SELECT id,hard_token_budget,hard_request_budget,0,0,0,0,?1,?2 FROM agent_runs WHERE id=?3 \
             ON CONFLICT(root_run_id) DO UPDATE SET lease_epoch=excluded.lease_epoch,fencing_token=excluded.fencing_token,updated_at=datetime('now','localtime')",
            params![lease.lease_epoch, lease.fencing_token, lease.root_run_id],
        )
        .map_err(|error| DecisionError::new(reason::BUDGET_EXCEEDED, format!("预算账本初始化失败：{error}")))?;
    Ok(())
}

pub fn remaining_budget(
    connection: &Connection,
    root_run_id: &str,
) -> Result<(i64, i64), DecisionError> {
    let totals: Option<(i64, i64, i64, i64, i64, i64)> = connection
        .query_row(
            "SELECT total_tokens,total_requests,reserved_tokens,reserved_requests,spent_tokens,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::BUDGET_EXCEEDED, format!("无法读取预算：{error}")))?;
    let Some((
        total_tokens,
        total_requests,
        reserved_tokens,
        reserved_requests,
        spent_tokens,
        spent_requests,
    )) = totals
    else {
        return Ok((0, 0));
    };
    Ok((
        (total_tokens - reserved_tokens - spent_tokens).max(0),
        (total_requests - reserved_requests - spent_requests).max(0),
    ))
}

/// 与现状同形的带约束 UPDATE：`(total=0 OR reserved+spent+n<=total)`，防双花。
pub fn reserve_budget(
    connection: &Connection,
    lease: &CoordinatorLease,
    reserved_tokens: i64,
    reserved_requests: i64,
) -> Result<(), DecisionError> {
    let changed = connection
        .execute(
            "UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens+?1,reserved_requests=reserved_requests+?2,\
             updated_at=datetime('now','localtime') WHERE root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 \
             AND (total_tokens=0 OR reserved_tokens+spent_tokens+?1<=total_tokens) \
             AND (total_requests=0 OR reserved_requests+spent_requests+?2<=total_requests)",
            params![reserved_tokens, reserved_requests, lease.root_run_id, lease.lease_epoch, lease.fencing_token],
        )
        .map_err(|error| DecisionError::new(reason::BUDGET_EXCEEDED, format!("预算预留失败：{error}")))?;
    if changed != 1 {
        // 现状串 child_budget_reservation_exceeded_or_stale 只留在 legacy 侧；
        // 新路径对外一律用 SPEC 的 reason_code（DESIGN §5.2）。
        return Err(DecisionError::new(
            reason::BUDGET_EXCEEDED,
            "预算预留超出剩余或租约已过期",
        ));
    }
    Ok(())
}

pub fn settle_budget(
    connection: &Connection,
    root_run_id: &str,
    reserved_tokens: i64,
    reserved_requests: i64,
    consumed_tokens: i64,
    consumed_requests: i64,
) -> Result<(), DecisionError> {
    let changed = connection
        .execute(
            "UPDATE agent_budget_ledger SET reserved_tokens=MAX(0,reserved_tokens-?1),reserved_requests=MAX(0,reserved_requests-?2),\
             spent_tokens=spent_tokens+?3,spent_requests=spent_requests+?4,updated_at=datetime('now','localtime') WHERE root_run_id=?5",
            params![reserved_tokens, reserved_requests, consumed_tokens, consumed_requests, root_run_id],
        )
        .map_err(|error| DecisionError::new(reason::BUDGET_EXCEEDED, format!("预算结算失败：{error}")))?;
    if changed != 1 {
        return Err(DecisionError::new(
            reason::BUDGET_EXCEEDED,
            "预算账本不存在该 root_run",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub enum AssignmentInsert {
    Inserted(String),
    Existing(String),
}

pub fn insert_assignment(
    connection: &Connection,
    assignment: &types::HomogeneousAssignment,
) -> Result<AssignmentInsert, DecisionError> {
    super::decision::validate_state_word(&assignment.state, &types::ASSIGNMENT_STATES)?;
    let lease = serde_json::to_string(&assignment.capability_lease)
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let slice_json = serde_json::to_string(&serde_json::json!({
        "scope_kind": assignment.scope_kind,
        "scope_value": assignment.scope_value,
        "slice_id": assignment.slice_id,
    }))
    .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let inserted = connection
        .execute(
            "INSERT INTO agent_assignments(id,coordinator_run_id,child_run_id,role,lane,target_key,state,dedup_key,trigger_code,\
             task_slice_json,evidence_revision,contract_keys_json,identity_handles_json,reserved_tokens,reserved_requests,\
             capability_lease_json,deadline_at,failure_class,slice_id,scheduling_model,worker_profile_id,homogeneous_worker_id,attempt)\
             VALUES(?1,?2,'','',?3,?4,?5,?6,?7,?8,?9,'[]','[]',?10,?11,?12,'','',?13,'homogeneous_workers',?14,?15,?16)\
             ON CONFLICT(coordinator_run_id,dedup_key) DO NOTHING",
            params![
                assignment.assignment_id,
                assignment.coordinator_run_id,
                assignment.lane,
                assignment.target_key,
                assignment.state,
                assignment.dedup_key,
                assignment.trigger_code,
                slice_json,
                assignment.evidence_revision_base,
                assignment.reserved_tokens,
                assignment.reserved_requests,
                lease,
                assignment.slice_id,
                assignment.worker_profile_id,
                assignment.worker_id,
                assignment.attempt
            ],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("assignment 写入失败：{error}")))?;
    if inserted == 1 {
        return Ok(AssignmentInsert::Inserted(assignment.assignment_id.clone()));
    }
    let stored: Option<(String, String, i64, i64)> = connection
        .query_row(
            "SELECT id,slice_id,reserved_tokens,reserved_requests FROM agent_assignments WHERE coordinator_run_id=?1 AND dedup_key=?2",
            params![assignment.coordinator_run_id, assignment.dedup_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::DUPLICATE_SLICE, error.to_string()))?;
    let Some((id, slice_id, reserved_tokens, reserved_requests)) = stored else {
        return Err(DecisionError::new(
            reason::DUPLICATE_SLICE,
            "dedup key 冲突但找不到原行",
        ));
    };
    if slice_id != assignment.slice_id
        || reserved_tokens != assignment.reserved_tokens
        || reserved_requests != assignment.reserved_requests
    {
        return Err(DecisionError::new(
            reason::DUPLICATE_SLICE,
            format!("同一 dedup key 已存在不同计划：{}", assignment.dedup_key),
        ));
    }
    Ok(AssignmentInsert::Existing(id))
}

#[allow(clippy::type_complexity)]
pub fn load_assignment(
    connection: &Connection,
    assignment_id: &str,
) -> Result<Option<types::HomogeneousAssignment>, DecisionError> {
    let stored: Option<(String, String, String, String, String, String, String, String, i64, i64, i64, String, String, String, i64, String, String)> = connection
        .query_row(
            "SELECT id,coordinator_run_id,lane,target_key,state,dedup_key,trigger_code,capability_lease_json,\
             reserved_tokens,reserved_requests,evidence_revision,slice_id,homogeneous_worker_id,worker_profile_id,attempt,\
             json_extract(task_slice_json,'$.scope_kind'),json_extract(task_slice_json,'$.scope_value') \
             FROM agent_assignments WHERE id=?1 AND scheduling_model='homogeneous_workers'",
            [assignment_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?, row.get::<_, Option<String>>(15)?.unwrap_or_default(), row.get::<_, Option<String>>(16)?.unwrap_or_default())),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, format!("assignment 不可读：{error}")))?;
    let Some((
        id,
        coordinator_run_id,
        lane,
        target_key,
        state,
        dedup_key,
        trigger_code,
        lease_json,
        reserved_tokens,
        reserved_requests,
        evidence_revision,
        slice_id,
        worker_id,
        worker_profile_id,
        attempt,
        scope_kind,
        scope_value,
    )) = stored
    else {
        return Ok(None);
    };
    super::decision::validate_state_word(&state, &types::ASSIGNMENT_STATES)?;
    let capability_lease: Vec<String> = serde_json::from_str(&lease_json).unwrap_or_default();
    let parent_task_id = load_task_by_root_run(connection, &coordinator_run_id)?
        .map(|task| task.task_id)
        .unwrap_or_default();
    Ok(Some(types::HomogeneousAssignment {
        assignment_id: id,
        coordinator_run_id,
        parent_task_id,
        slice_id,
        worker_id,
        worker_profile_id,
        target_key,
        lane,
        state,
        reserved_tokens,
        reserved_requests,
        capability_lease,
        evidence_revision_base: evidence_revision,
        attempt,
        dedup_key,
        trigger_code,
        scope_kind,
        scope_value,
    }))
}

pub fn find_assignment_by_slice_attempt(
    connection: &Connection,
    slice_id: &str,
    attempt: i64,
) -> Result<Option<types::HomogeneousAssignment>, DecisionError> {
    let id: Option<String> = connection
        .query_row(
            "SELECT id FROM agent_assignments WHERE slice_id=?1 AND attempt=?2 AND scheduling_model='homogeneous_workers'",
            params![slice_id, attempt],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::ASSIGNMENT_NOT_FOUND, error.to_string()))?;
    match id {
        Some(id) => load_assignment(connection, &id),
        None => Ok(None),
    }
}

pub fn set_assignment_state(
    connection: &Connection,
    assignment_id: &str,
    state: &str,
) -> Result<(), DecisionError> {
    super::decision::validate_state_word(state, &types::ASSIGNMENT_STATES)?;
    super::transitions::assignment(connection, assignment_id, state)?;
    let changed = connection
        .execute(
            "UPDATE agent_assignments SET state=?1,updated_at=datetime('now','localtime') WHERE id=?2 AND scheduling_model='homogeneous_workers'",
            params![state, assignment_id],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("assignment 状态写入失败：{error}")))?;
    if changed != 1 {
        return Err(DecisionError::new(
            reason::ASSIGNMENT_NOT_FOUND,
            format!("assignment 不可寻：{assignment_id}"),
        ));
    }
    Ok(())
}

pub fn list_assignment_ids_in_state(
    connection: &Connection,
    root_run_id: &str,
    states: &[&str],
) -> Result<Vec<String>, DecisionError> {
    // 逐状态查询而不是拼 IN 列表：既避免动态 SQL，也让顺序可控。
    let mut ids: Vec<String> = Vec::new();
    for state in states {
        let mut statement = connection
            .prepare(
                "SELECT id FROM agent_assignments WHERE coordinator_run_id=?1 AND scheduling_model='homogeneous_workers' AND state=?2 ORDER BY created_at",
            )
            .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
        let found = statement
            .query_map(params![root_run_id, state], |row| row.get::<_, String>(0))
            .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
        ids.extend(found);
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

pub fn insert_report(
    connection: &Connection,
    report: &types::WorkerReport,
    lease_epoch: i64,
) -> Result<(), DecisionError> {
    if !types::is_member(&report.terminal_hint, &types::TERMINAL_HINTS) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "terminal_hint 不在封闭集合内",
        ));
    }
    let references = serde_json::to_string(&report.evidence_refs)
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let output = serde_json::to_string(&report.output)
        .map_err(|error| DecisionError::new(reason::OUTPUT_SCHEMA_INVALID, error.to_string()))?;
    connection
        .execute(
            "INSERT INTO agent_worker_reports(report_id,assignment_id,slice_id,worker_id,evidence_refs_json,output_json,consumed_tokens,consumed_requests,terminal_hint,lease_epoch)\
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(report_id) DO NOTHING",
            params![
                report.report_id,
                report.assignment_id,
                report.slice_id,
                report.worker_id,
                references,
                output,
                report.consumed_tokens,
                report.consumed_requests,
                report.terminal_hint,
                lease_epoch
            ],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("汇报写入失败：{error}")))?;
    Ok(())
}

pub fn load_report(
    connection: &Connection,
    report_id: &str,
) -> Result<Option<types::WorkerReport>, DecisionError> {
    let stored: Option<ReportRow> = connection
        .query_row(
            "SELECT report_id,assignment_id,slice_id,worker_id,evidence_refs_json,output_json,consumed_tokens,consumed_requests,terminal_hint \
             FROM agent_worker_reports WHERE report_id=?1",
            [report_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?)),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::EVIDENCE_MISSING, format!("汇报不可读：{error}")))?;
    let Some((
        report_id,
        assignment_id,
        slice_id,
        worker_id,
        references,
        output,
        consumed_tokens,
        consumed_requests,
        terminal_hint,
    )) = stored
    else {
        return Ok(None);
    };
    let evidence_refs: Vec<String> = serde_json::from_str(&references).unwrap_or_default();
    let output: JsonValue = serde_json::from_str(&output).unwrap_or(JsonValue::Null);
    Ok(Some(types::WorkerReport {
        report_id,
        assignment_id,
        slice_id,
        worker_id,
        evidence_refs,
        output,
        consumed_tokens,
        consumed_requests,
        terminal_hint,
    }))
}

pub fn insert_verdict(
    connection: &Connection,
    verdict: &types::AcceptanceVerdict,
) -> Result<(), DecisionError> {
    if !types::is_member(&verdict.verdict, &types::VERDICTS) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "verdict 不在封闭集合内",
        ));
    }
    let checks = serde_json::to_value(&verdict.machine_checks)
        .map(|value| serde_json::to_string(&value).unwrap_or_default())
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let judgement = match verdict.model_judgement.as_ref() {
        Some(body) => serde_json::to_string(&serde_json::json!({
            "adequate": body.adequate,
            "question_zh": body.question_zh,
            "answer_zh": body.answer_zh,
        }))
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?,
        None => String::new(),
    };
    let codes = serde_json::to_string(&verdict.reason_codes)
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let references = serde_json::to_string(&verdict.evidence_refs)
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    connection
        .execute(
            "INSERT INTO agent_acceptance_verdicts(verdict_id,assignment_id,slice_id,verdict,machine_checks_json,model_judgement_json,reason_codes_json,evidence_refs_json,decided_by_lease_epoch,attempt)\
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(assignment_id,verdict,attempt) DO NOTHING",
            params![
                verdict.verdict_id,
                verdict.assignment_id,
                verdict.slice_id,
                verdict.verdict,
                checks,
                judgement,
                codes,
                references,
                verdict.decided_by_lease_epoch,
                verdict.attempt
            ],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("verdict 写入失败：{error}")))?;
    Ok(())
}

pub fn upsert_gated_directive(
    connection: &Connection,
    directive: &types::GatedDirective,
) -> Result<(), DecisionError> {
    super::decision::validate_state_word(&directive.state, &types::DIRECTIVE_STATES)?;
    if !types::is_member(&directive.rule_screen_result, &types::RULE_SCREEN_RESULTS) {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "rule_screen_result 非法",
        ));
    }
    if !directive.coordinator_action.is_empty()
        && !types::is_member(
            &directive.coordinator_action,
            &types::DIRECTIVE_COORDINATOR_ACTIONS,
        )
    {
        return Err(DecisionError::new(
            reason::SCHEMA_INVALID,
            "coordinator_action 非法",
        ));
    }
    let codes = serde_json::to_string(&directive.rule_reason_codes)
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    connection
        .execute(
            "INSERT INTO agent_human_directive_gated(directive_id,parent_task_id,source_user_directive_id,raw_text_redacted,state,rule_screen_result,rule_reason_codes_json,\
             coordinator_action,rewritten_text,target_worker_id,broadcast_constraint,reason_code,reason_zh,suggestion_zh,user_feedback_zh)\
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)\
             ON CONFLICT(directive_id) DO UPDATE SET state=excluded.state,rule_screen_result=excluded.rule_screen_result,rule_reason_codes_json=excluded.rule_reason_codes_json,\
             coordinator_action=excluded.coordinator_action,rewritten_text=excluded.rewritten_text,target_worker_id=excluded.target_worker_id,\
             broadcast_constraint=excluded.broadcast_constraint,reason_code=excluded.reason_code,reason_zh=excluded.reason_zh,\
             suggestion_zh=excluded.suggestion_zh,user_feedback_zh=excluded.user_feedback_zh,updated_at=datetime('now','localtime')",
            params![
                directive.directive_id,
                directive.parent_task_id,
                directive.source_user_directive_id,
                directive.raw_text_redacted,
                directive.state,
                directive.rule_screen_result,
                codes,
                directive.coordinator_action,
                directive.rewritten_text,
                directive.target_worker_id,
                directive.broadcast_constraint,
                directive.reason_code,
                directive.reason_zh,
                directive.suggestion_zh,
                directive.user_feedback_zh
            ],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("引导闸门写入失败：{error}")))?;
    Ok(())
}

#[allow(clippy::type_complexity)]
pub fn load_gated_directive(
    connection: &Connection,
    directive_id: &str,
) -> Result<Option<types::GatedDirective>, DecisionError> {
    let stored: Option<(String, String, String, String, String, String, String, String, String, String, String, String, String, String, String)> = connection
        .query_row(
            "SELECT directive_id,parent_task_id,source_user_directive_id,raw_text_redacted,state,rule_screen_result,rule_reason_codes_json,\
             coordinator_action,rewritten_text,target_worker_id,broadcast_constraint,reason_code,reason_zh,suggestion_zh,user_feedback_zh \
             FROM agent_human_directive_gated WHERE directive_id=?1",
            [directive_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?, row.get(12)?, row.get(13)?, row.get(14)?)),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::ALLOWLIST_VIOLATION, format!("闸门记录不可读：{error}")))?;
    let Some((
        directive_id,
        parent_task_id,
        source_user_directive_id,
        raw_text_redacted,
        state,
        rule_screen_result,
        codes,
        coordinator_action,
        rewritten_text,
        target_worker_id,
        broadcast_constraint,
        reason_code,
        reason_zh,
        suggestion_zh,
        user_feedback_zh,
    )) = stored
    else {
        return Ok(None);
    };
    super::decision::validate_state_word(&state, &types::DIRECTIVE_STATES)?;
    let rule_reason_codes: Vec<String> = serde_json::from_str(&codes).unwrap_or_default();
    Ok(Some(types::GatedDirective {
        directive_id,
        parent_task_id,
        source_user_directive_id,
        raw_text_redacted,
        state,
        rule_screen_result,
        rule_reason_codes,
        coordinator_action,
        rewritten_text,
        target_worker_id,
        broadcast_constraint,
        reason_code,
        reason_zh,
        suggestion_zh,
        user_feedback_zh,
    }))
}

pub fn awaiting_directive_ids(
    connection: &Connection,
    parent_task_id: &str,
) -> Result<Vec<String>, DecisionError> {
    let mut statement = connection
        .prepare("SELECT directive_id FROM agent_human_directive_gated WHERE parent_task_id=?1 AND state IN ('rule_screened','awaiting_coordinator') ORDER BY created_at")
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    let rows = statement
        .query_map([parent_task_id], |row| row.get::<_, String>(0))
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    rows.collect::<Result<Vec<String>, _>>()
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))
}

/// DESIGN §4.8：契约层 `evd-` 别名 ↔ 现状 64-hex 证据节点 id，不放宽 SPEC pattern。
pub fn register_evidence_alias(
    connection: &Connection,
    internal_id: &str,
    root_run_id: &str,
    revision: i64,
) -> Result<String, DecisionError> {
    let existing: Option<String> = connection
        .query_row(
            "SELECT evd_id FROM agent_evidence_aliases WHERE internal_id=?1",
            [internal_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, error.to_string()))?;
    if let Some(alias) = existing {
        return Ok(alias);
    }
    let alias = format!("evd-{}", &stable_hash(internal_id)[..20]);
    connection
        .execute(
            "INSERT INTO agent_evidence_aliases(evd_id,internal_id,root_run_id,revision) VALUES(?1,?2,?3,?4) ON CONFLICT(internal_id) DO NOTHING",
            params![alias, internal_id, root_run_id, revision],
        )
        .map_err(|error| DecisionError::new(reason::SCHEMA_INVALID, format!("别名写入失败：{error}")))?;
    Ok(alias)
}

pub fn resolve_evidence_alias(
    connection: &Connection,
    alias: &str,
) -> Result<Option<(String, String, i64)>, DecisionError> {
    connection
        .query_row(
            "SELECT internal_id,root_run_id,revision FROM agent_evidence_aliases WHERE evd_id=?1",
            [alias],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| {
            DecisionError::new(reason::EVIDENCE_MISSING, format!("别名不可解析：{error}"))
        })
}
