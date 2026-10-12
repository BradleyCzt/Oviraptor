//! G4-2 采集中的实时进度（只读）。阶段、当前来源、各来源工作项完成/总数、请求与字节用量、
//! 最近一次落盘时间、缺口。百分比只按工作项算，并说明它不代表请求量或剩余时间。

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;

fn storage(error: impl std::fmt::Display) -> String {
    format!("[exposure_live_storage] {error}")
}

pub const GAP_SAMPLE: i64 = 20;

/// The latest round of the project, or `None` when it has never collected.
pub fn live_run(connection: &Connection, project_id: i64) -> Result<Option<JsonValue>, String> {
    type RunHead = (i64, String, String, String, i64, String, String, String, String);
    let run: Option<RunHead> = connection
        .query_row(
            "SELECT id,status,stage,current_source,cancel_requested,created_at,started_at,completed_at,error FROM exposure_runs WHERE project_id=?1 ORDER BY id DESC LIMIT 1",
            [project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?)),
        )
        .optional()
        .map_err(storage)?;
    let Some((run_id, status, stage, current_source, cancel_requested, created_at, started_at, completed_at, error)) = run else {
        return Ok(None);
    };
    let mut sources: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
    let (mut total, mut terminal, mut requests, mut bytes) = (0i64, 0i64, 0i64, 0i64);
    let mut last_item = String::new();
    {
        let mut statement = connection
            .prepare("SELECT source_key,status,COUNT(*),COALESCE(SUM(requests),0),COALESCE(SUM(bytes),0),COALESCE(MAX(updated_at),'') FROM exposure_work_items WHERE run_id=?1 AND project_id=?2 GROUP BY 1,2")
            .map_err(storage)?;
        let mut rows = statement.query(rusqlite::params![run_id, project_id]).map_err(storage)?;
        while let Some(row) = rows.next().map_err(storage)? {
            let source: String = row.get(0).map_err(storage)?;
            let item_status: String = row.get(1).map_err(storage)?;
            let count: i64 = row.get(2).map_err(storage)?;
            requests += row.get::<_, i64>(3).map_err(storage)?;
            bytes += row.get::<_, i64>(4).map_err(storage)?;
            let updated: String = row.get(5).map_err(storage)?;
            if updated > last_item {
                last_item = updated;
            }
            total += count;
            let is_terminal = super::state::WorkItemStatus::parse(&item_status).is_some_and(|value| value.is_terminal());
            if is_terminal {
                terminal += count;
            }
            let entry = sources.entry(source).or_default();
            *entry.entry(item_status).or_insert(0) += count;
            *entry.entry("total".to_string()).or_insert(0) += count;
        }
    }
    let last_observation: String = connection
        .query_row("SELECT COALESCE(MAX(queried_at),'') FROM exposure_observations WHERE run_id=?1 AND project_id=?2", rusqlite::params![run_id, project_id], |row| row.get(0))
        .map_err(storage)?;
    let budget: Option<(i64, i64, i64, i64, i64)> = connection
        .query_row(
            "SELECT budget_requests,budget_bytes,requests_used,bytes_used,planned_items FROM exposure_run_plans WHERE run_id=?1 AND project_id=?2",
            rusqlite::params![run_id, project_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()
        .map_err(storage)?;
    let gaps_total: i64 = connection
        .query_row("SELECT COUNT(*) FROM exposure_work_items WHERE run_id=?1 AND project_id=?2 AND status<>'completed'", rusqlite::params![run_id, project_id], |row| row.get(0))
        .map_err(storage)?;
    let gaps: Vec<JsonValue> = super::work_items::enumerate_gaps(connection, run_id, GAP_SAMPLE)?
        .into_iter()
        .map(|(key, item_status, reason)| json!({"workKey": key, "status": item_status, "reason": reason}))
        .collect();
    let active = matches!(status.as_str(), "queued" | "running");
    let last_persisted = if last_observation > last_item { last_observation } else { last_item };
    let source_rows: Vec<JsonValue> = sources
        .into_iter()
        .map(|(source, counts)| {
            let completed = counts.get("completed").copied().unwrap_or(0);
            let source_total = counts.get("total").copied().unwrap_or(0);
            json!({"sourceKey": source, "completed": completed, "total": source_total, "byStatus": counts})
        })
        .collect();
    Ok(Some(json!({
        "runId": run_id,
        "status": status,
        "active": active,
        "stage": stage,
        "currentSource": current_source,
        "cancelRequested": cancel_requested != 0,
        "resumable": matches!(status.as_str(), "interrupted" | "cancelled"),
        "createdAt": created_at,
        "startedAt": started_at,
        "completedAt": completed_at,
        "error": error,
        "sources": source_rows,
        "progress": {
            "basis": "work_items",
            "done": terminal,
            "total": total,
            "percent": if total > 0 { Some(terminal * 100 / total) } else { None },
            "note": "按工作项计数，不代表请求量、字节量或剩余时间",
        },
        "usage": {
            "requests": budget.map(|row| row.2).unwrap_or(requests),
            "bytes": budget.map(|row| row.3).unwrap_or(bytes),
            "budgetRequests": budget.map(|row| row.0),
            "budgetBytes": budget.map(|row| row.1),
            "plannedItems": budget.map(|row| row.4),
        },
        "lastPersistedAt": last_persisted,
        "gapsTotal": gaps_total,
        "gaps": gaps,
        "gapsTruncated": gaps_total > GAP_SAMPLE,
    })))
}
