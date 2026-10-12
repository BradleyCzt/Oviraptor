// §Q3 — recovery. A restarted application may only continue a run whose frozen plan still
// matches the current authorization and options; otherwise the caller must start a new run.
// Nothing here rewrites history or takes over a claim that is still beating.

use super::adapters::http::RequestMeter;
use super::orchestrator::{
    mark_stage, options_hash, run_plan, scope_hash_of, ItemOutcome, RunReport,
};
use super::scope::ExposureScope;
use super::settings::ExposureSettings;
use super::state::{shift_seconds_back, RunStatus, EXPOSURE_RULE_VERSION};
use super::work_items::{counts, reclaim_expired, DEFAULT_LEASE_SECONDS};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;

#[derive(Clone, Debug)]
pub struct PlanRow {
    pub run_id: i64,
    /// §E2 the settings version and full value this round was started with; a later edit of
    /// the current configuration must not silently change an in-flight or dead round.
    pub settings_version: i64,
    pub settings_json: String,
    pub project_id: i64,
    pub attempt_number: i64,
    pub scope_version: i64,
    pub scope_hash: String,
    pub options_hash: String,
    pub planned_items: i64,
    pub requests_used: i64,
    pub bytes_used: i64,
    pub max_requests: i64,
    pub max_bytes: i64,
}

/// Reads the frozen plan and refuses to answer for a run that belongs to another project or
/// that predates this structure.
pub fn load_plan(
    connection: &Connection,
    run_id: i64,
    expected_project_id: i64,
) -> Result<PlanRow, String> {
    let row: Option<PlanRow> = connection
        .query_row(
            "SELECT p.run_id,p.project_id,p.attempt_number,p.scope_version,p.scope_hash,p.options_hash,p.planned_items,p.requests_used,p.bytes_used,p.budget_requests,p.budget_bytes,COALESCE(p.settings_version,1),COALESCE(p.settings_json,'{}')
             FROM exposure_run_plans p JOIN exposure_runs r ON r.id=p.run_id
             WHERE p.run_id=?1 AND r.project_id=?2",
            params![run_id, expected_project_id],
            |row| {
                Ok(PlanRow {
                    run_id: row.get(0)?,
                    project_id: row.get(1)?,
                    attempt_number: row.get(2)?,
                    scope_version: row.get(3)?,
                    scope_hash: row.get(4)?,
                    options_hash: row.get(5)?,
                    planned_items: row.get(6)?,
                    requests_used: row.get(7)?,
                    bytes_used: row.get(8)?,
                    max_requests: row.get(9)?,
                    max_bytes: row.get(10)?,
                    settings_version: row.get(11)?,
                    settings_json: row.get(12)?,
                })
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    row.ok_or_else(|| {
        format!("exposure_plan_missing:任务 {run_id} 没有冻结计划（历史任务或跨项目访问），不能作为新合同恢复")
    })
}

/// The settings the round was actually started with, so a resume keeps its own budget.
/// G1-0: the production reader; see `ExposureSettings::from_frozen_strict`.
pub fn frozen_settings_strict(plan_row: &PlanRow) -> Result<ExposureSettings, String> {
    ExposureSettings::from_frozen_strict(
        plan_row.project_id,
        plan_row.settings_version,
        &plan_row.settings_json,
    )
}

/// Lenient legacy reader kept for tests that inspect an already-frozen value.
pub fn frozen_settings(plan_row: &PlanRow) -> ExposureSettings {
    ExposureSettings::from_frozen(
        plan_row.project_id,
        plan_row.settings_version,
        &plan_row.settings_json,
    )
}

impl PlanRow {
    /// §E2 rounds started before the settings contract have no frozen value at all. Their
    /// options hash cannot be re-derived, so resuming them would quietly hand an old round a
    /// budget it never recorded; such a round has to be started again instead.
    pub fn settings_frozen(&self) -> bool {
        serde_json::from_str::<JsonValue>(&self.settings_json)
            .ok()
            .and_then(|row| row.as_object().map(|map| !map.is_empty()))
            .unwrap_or(false)
    }
}

/// Refuses reuse when the authorization scope or the collection options moved, so a resumed
/// round cannot quietly point an old plan at new targets or new budgets.
pub fn verify_frozen(
    plan_row: &PlanRow,
    scope: &ExposureScope,
    settings: &JsonValue,
) -> Result<(), String> {
    if !plan_row.settings_frozen() {
        return Err(
            "exposure_plan_settings_unfrozen：这一轮启动时没有冻结采集配置，无法按当前合同确认它原来的额度；\
             请另起一轮，已花额度与旧结果保留"
                .to_string(),
        );
    }
    if !plan_row.scope_hash.starts_with(super::orchestrator::SCOPE_HASH_PREFIX) {
        return Err(
            "exposure_plan_scope_hash_legacy：这一轮的范围指纹含签发时间（G1 之前写入），无法核对是否仍是同一范围；\
             请另起一轮，已花额度与旧结果保留"
                .to_string(),
        );
    }
    let mut moved = Vec::new();
    if plan_row.scope_hash != scope_hash_of(scope) {
        moved.push("授权范围或归属版本已变化");
    }
    if plan_row.options_hash != options_hash(settings) {
        moved.push("冻结计划与记录的配置不一致");
    }
    if moved.is_empty() {
        return Ok(());
    }
    Err(format!(
        "exposure_plan_frozen_mismatch：{}；请另起新一轮采集，旧结果保留",
        moved.join("，")
    ))
}

pub fn live_claims(
    connection: &Connection,
    run_id: i64,
    lease_seconds: i64,
) -> Result<i64, String> {
    let cutoff = shift_seconds_back(lease_seconds);
    connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_work_items
             WHERE run_id=?1 AND status IN ('claimed','in_progress') AND heartbeat_at<>'' AND heartbeat_at>=?2",
            params![run_id, cutoff],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
}

/// Marks a run interrupted only when nothing in it can still be proven to be working. A run
/// with a fresh claim is left exactly as it is, because the owning thread may still be alive.
pub fn mark_interrupted_if_dead(
    connection: &Connection,
    run_id: i64,
    lease_seconds: i64,
) -> Result<bool, String> {
    let active: Option<String> = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if !matches!(active.as_deref(), Some("queued") | Some("running")) {
        return Ok(false);
    }
    if live_claims(connection, run_id, lease_seconds)? > 0 {
        return Ok(false);
    }
    let orphans = reclaim_expired(connection, run_id, lease_seconds)?;
    let open = counts(connection, run_id)?.open;
    connection
        .execute(
            "UPDATE exposure_runs SET status='interrupted',stage='interrupted',error=?2,completed_at=?3 WHERE id=?1 AND status IN ('queued','running')",
            params![
                run_id,
                format!(
                    "{{\"status\":\"interrupted\",\"reasons\":[\"进程已退出，{orphans} 个认领成为孤儿，{open} 个工作项未到终态\"],\"gaps\":[\"restart_orphans:{orphans}\",\"work_items_open:{open}\"],\"ruleVersion\":\"{EXPOSURE_RULE_VERSION}\"}}"
                ),
                super::state::utc_now()
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(true)
}

/// A run without a plan row is a pre-Q3 round. It keeps its own provenance label instead of
/// being back-filled with work items that were never planned.
pub fn legacy_projection(connection: &Connection, run_id: i64) -> Result<Option<String>, String> {
    let planned: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM exposure_run_plans WHERE run_id=?1",
            [run_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if planned > 0 {
        return Ok(None);
    }
    let status: Option<String> = connection
        .query_row(
            "SELECT status FROM exposure_runs WHERE id=?1",
            [run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(status.map(|value| {
        format!("legacy_passive_web(状态 {value})：该轮没有工作项台账，覆盖范围以当时的有限采集为准，不能按新合同解释")
    }))
}

/// Continues the frozen plan: already-closed items are not re-fetched, and the round keeps
/// charging against the budget it already spent.
pub fn resume(
    connection: &Connection,
    run_id: i64,
    expected_project_id: i64,
    owner: &str,
    settings: &JsonValue,
    executor: &mut dyn FnMut(&super::work_items::WorkItem) -> Result<ItemOutcome, String>,
) -> Result<RunReport, String> {
    let plan_row = load_plan(connection, run_id, expected_project_id)?;
    if plan_row.project_id != expected_project_id {
        return Err("exposure_scope_mismatch".to_string());
    }
    let scope = ExposureScope::load(connection, expected_project_id, settings)
        .map_err(|denial| format!("{}:{}", denial.code, denial.message))?;
    verify_frozen(&plan_row, &scope, settings)?;
    mark_stage(connection, run_id, "resuming", "");
    run_plan(connection, run_id, owner, executor)
}

/// The meter a resumed round must start from: the plan's own spend, not a fresh budget.
pub fn meter_for(plan_row: &PlanRow) -> RequestMeter {
    RequestMeter {
        requests_used: plan_row.requests_used,
        bytes_used: plan_row.bytes_used,
        max_requests: plan_row.max_requests,
        max_bytes: plan_row.max_bytes,
    }
}

/// True when the run's own status says it stopped without closing its plan.
pub fn is_interrupted(status: RunStatus) -> bool {
    status == RunStatus::Interrupted
}

pub fn default_lease() -> i64 {
    DEFAULT_LEASE_SECONDS
}
