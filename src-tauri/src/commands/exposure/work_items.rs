// §Q3 — durable work items for one exposure run. Every planned object gets a row before
// any network call, so "what is left" is a query rather than a guess, and a claim carries
// an owner plus a version so a worker that lost its lease cannot write a terminal state.

use super::state::{shift_seconds_back, utc_now, WorkItemCounts, WorkItemStatus};
use super::transaction::ImmediateTx;
use rusqlite::{params, Connection, OptionalExtension};

/// How long a claim stays valid while a worker is between two database writes.
pub const DEFAULT_LEASE_SECONDS: i64 = 90;

#[derive(Clone, Debug)]
pub struct WorkItemSeed {
    pub work_key: String,
    pub kind: String,
    pub source_key: String,
    pub asset_id: Option<i64>,
    pub origin: String,
    pub entity: String,
    pub input_hash: String,
}

#[derive(Clone, Debug)]
pub struct WorkItem {
    pub id: i64,
    pub run_id: i64,
    pub project_id: i64,
    pub work_key: String,
    pub kind: String,
    pub source_key: String,
    pub asset_id: Option<i64>,
    pub origin: String,
    pub entity: String,
    pub input_hash: String,
    pub status: WorkItemStatus,
    pub owner: String,
    pub claim_version: i64,
    pub attempts: i64,
    pub observations: i64,
    pub requests: i64,
    pub bytes: i64,
}

const ITEM_COLUMNS: &str = "id,run_id,project_id,work_key,kind,source_key,asset_id,origin,entity,input_hash,status,owner,claim_version,attempts,observations,requests,bytes";

fn item_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkItem> {
    let status: String = row.get(10)?;
    Ok(WorkItem {
        id: row.get(0)?,
        run_id: row.get(1)?,
        project_id: row.get(2)?,
        work_key: row.get(3)?,
        kind: row.get(4)?,
        source_key: row.get(5)?,
        asset_id: row.get(6)?,
        origin: row.get(7)?,
        entity: row.get(8)?,
        input_hash: row.get(9)?,
        status: WorkItemStatus::parse(&status).unwrap_or(WorkItemStatus::Pending),
        owner: row.get(11)?,
        claim_version: row.get(12)?,
        attempts: row.get(13)?,
        observations: row.get(14)?,
        requests: row.get(15)?,
        bytes: row.get(16)?,
    })
}

/// Writes the frozen plan. Re-running with the same keys is idempotent: a resumed run does
/// not invent a second work item for the same object.
pub fn plan(
    connection: &Connection,
    run_id: i64,
    project_id: i64,
    attempt_number: i64,
    seeds: &[WorkItemSeed],
) -> Result<usize, String> {
    for seed in seeds {
        if seed.work_key.trim().is_empty() || seed.kind.trim().is_empty() {
            return Err("工作项缺少稳定 key 或类型，拒绝写入计划".to_string());
        }
        connection
            .execute(
                "INSERT INTO exposure_work_items(run_id,project_id,attempt_number,work_key,kind,source_key,asset_id,origin,entity,input_hash,status)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'pending')
                 ON CONFLICT(run_id,work_key) DO NOTHING",
                params![
                    run_id,
                    project_id,
                    attempt_number,
                    seed.work_key,
                    seed.kind,
                    seed.source_key,
                    seed.asset_id,
                    seed.origin,
                    seed.entity,
                    seed.input_hash
                ],
            )
            .map_err(|error| format!("工作项计划写入失败：{error}"))?;
    }
    Ok(seeds.len())
}

/// Claims the next unfinished item for `owner`, taking over claims whose lease expired.
/// The candidate row and the claim update are in one transaction so two workers cannot both
/// believe they own the same object.
pub fn claim_next(
    connection: &rusqlite::Connection,
    run_id: i64,
    owner: &str,
    lease_seconds: i64,
) -> Result<Option<WorkItem>, String> {
    reclaim_expired(connection, run_id, lease_seconds)?;
    // Immediate so two concurrent claimants serialize instead of both believing they won.
    let transaction = ImmediateTx::begin(connection)?;
    loop {
        let candidate: Option<i64> = transaction
            .connection()
            .query_row(
                "SELECT id FROM exposure_work_items
                 WHERE run_id=?1 AND status IN ('pending','orphaned')
                 ORDER BY work_key LIMIT 1",
                [run_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(id) = candidate else {
            transaction.commit()?;
            return Ok(None);
        };
        let now = utc_now();
        let updated = transaction
            .connection()
            .execute(
                "UPDATE exposure_work_items
                 SET status='claimed', owner=?2, claim_version=claim_version+1,
                     claimed_at=?3, heartbeat_at=?3, attempts=attempts+1, updated_at=?3
                 WHERE id=?1 AND status IN ('pending','orphaned')",
                params![id, owner, now],
            )
            .map_err(|error| error.to_string())?;
        if updated == 1 {
            let item: Option<WorkItem> = transaction
                .connection()
                .query_row(
                    &format!("SELECT {ITEM_COLUMNS} FROM exposure_work_items WHERE id=?1"),
                    [id],
                    item_from_row,
                )
                .optional()
                .map_err(|error| error.to_string())?;
            transaction.commit()?;
            return Ok(item);
        }
    }
}

pub fn heartbeat(
    connection: &rusqlite::Connection,
    id: i64,
    owner: &str,
    claim_version: i64,
) -> Result<bool, String> {
    let now = utc_now();
    let updated = connection
        .execute(
            "UPDATE exposure_work_items SET heartbeat_at=?4, status='in_progress'
             WHERE id=?1 AND owner=?2 AND claim_version=?3 AND status IN ('claimed','in_progress')",
            params![id, owner, claim_version, now],
        )
        .map_err(|error| error.to_string())?;
    Ok(updated == 1)
}

/// Terminal transition, guarded by owner and claim version. Returns false when the caller
/// no longer holds the claim, which is how a late worker is refused after a takeover.
#[allow(clippy::too_many_arguments)]
pub fn finish(
    connection: &rusqlite::Connection,
    id: i64,
    owner: &str,
    claim_version: i64,
    status: WorkItemStatus,
    reason: &str,
    observations: i64,
    requests: i64,
    bytes: i64,
) -> Result<bool, String> {
    if !status.is_terminal() {
        return Err("非终态不能作为工作项收尾写入".to_string());
    }
    let now = utc_now();
    let updated = connection
        .execute(
            "UPDATE exposure_work_items
             SET status=?4, reason=?5, observations=observations+?6, requests=requests+?7,
                 bytes=bytes+?8, finished_at=?3, heartbeat_at=?3, updated_at=?3
             WHERE id=?1 AND owner=?2 AND claim_version=?9 AND status IN ('claimed','in_progress')",
            params![
                id,
                owner,
                now,
                status.as_str(),
                reason,
                observations,
                requests,
                bytes,
                claim_version
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(updated == 1)
}

/// Drops claims whose owner stopped beating, without touching fresh ones.
pub fn reclaim_expired(
    connection: &rusqlite::Connection,
    run_id: i64,
    lease_seconds: i64,
) -> Result<i64, String> {
    let cutoff = shift_seconds_back(lease_seconds);
    let updated = connection
        .execute(
            "UPDATE exposure_work_items SET status='orphaned', reason='lease_expired', updated_at=?3
             WHERE run_id=?1 AND status IN ('claimed','in_progress')
               AND claimed_at<>'' AND (heartbeat_at='' OR heartbeat_at<?2)",
            params![run_id, cutoff, utc_now()],
        )
        .map_err(|error| error.to_string())?;
    Ok(updated as i64)
}

pub fn counts(connection: &rusqlite::Connection, run_id: i64) -> Result<WorkItemCounts, String> {
    let mut statement = connection
        .prepare("SELECT status,COUNT(*) FROM exposure_work_items WHERE run_id=?1 GROUP BY status")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([run_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| error.to_string())?;
    let mut counts = WorkItemCounts::default();
    for row in rows.flatten() {
        let Some(status) = WorkItemStatus::parse(&row.0) else {
            // Corrupt vocabulary is counted as open: it must never disappear from coverage.
            counts.open += row.1;
            counts.total += row.1;
            continue;
        };
        counts.total += row.1;
        if status.is_terminal() {
            counts.terminal += row.1;
        } else {
            counts.open += row.1;
        }
        match status {
            WorkItemStatus::Completed => counts.completed += row.1,
            WorkItemStatus::Failed => counts.failed += row.1,
            WorkItemStatus::Cancelled => counts.cancelled += row.1,
            WorkItemStatus::SkippedUnconfigured => counts.unconfigured += row.1,
            WorkItemStatus::BlockedQuota => counts.quota += row.1,
            WorkItemStatus::BlockedAuthorization => counts.authorization += row.1,
            WorkItemStatus::Orphaned => counts.orphaned += row.1,
            _ => {}
        }
    }
    Ok(counts)
}

/// The named leftovers, so `partial` can point at objects instead of an error string.
pub fn enumerate_gaps(
    connection: &rusqlite::Connection,
    run_id: i64,
    limit: i64,
) -> Result<Vec<(String, String, String)>, String> {
    let mut statement = connection
        .prepare(
            "SELECT work_key,status,reason FROM exposure_work_items
             WHERE run_id=?1 AND status<>'completed' ORDER BY work_key LIMIT ?2",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id, limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .map_err(|error| error.to_string())?;
    Ok(rows.flatten().collect())
}

/// The per-item counters that a resumed round must keep charging against.
#[derive(Clone, Debug, Default)]
pub struct RunUsage {
    pub planned: i64,
    pub requests_used: i64,
    pub bytes_used: i64,
    pub max_requests: i64,
    pub max_bytes: i64,
}

pub fn record_usage(
    connection: &rusqlite::Connection,
    run_id: i64,
    requests: i64,
    bytes: i64,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE exposure_run_plans SET requests_used=requests_used+?2, bytes_used=bytes_used+?3 WHERE run_id=?1",
            params![run_id, requests, bytes],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn usage(connection: &rusqlite::Connection, run_id: i64) -> Result<Option<RunUsage>, String> {
    let mut statement = connection
        .prepare(
            "SELECT planned_items,requests_used,bytes_used,budget_requests,budget_bytes
             FROM exposure_run_plans WHERE run_id=?1",
        )
        .map_err(|error| error.to_string())?;
    let row = statement
        .query_map([run_id], |row| {
            Ok(RunUsage {
                planned: row.get(0)?,
                requests_used: row.get(1)?,
                bytes_used: row.get(2)?,
                max_requests: row.get(3)?,
                max_bytes: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .flatten()
        .next();
    Ok(row)
}

pub fn item_status(connection: &rusqlite::Connection, id: i64) -> Result<(String, String), String> {
    connection
        .query_row(
            "SELECT status,reason FROM exposure_work_items WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map(|value| value.unwrap_or_default())
        .map_err(|error| error.to_string())
}
