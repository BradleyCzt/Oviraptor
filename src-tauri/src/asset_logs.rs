//! Durable asset log records. Notifications contain identity only, never text.
use crate::{log_display, models::LogEntry};
use rusqlite::{params, Connection, OptionalExtension};

/// Diagnostic retention only. Asset events and ownership/manual audit tables
/// are deliberately outside this policy. Counters survive diagnostic trimming.
pub(crate) const RETENTION_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS asset_log_retention (
 run_id INTEGER PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
 discarded_rows INTEGER NOT NULL DEFAULT 0, noise_rows INTEGER NOT NULL DEFAULT 0,
 expired_rows INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS asset_log_retention_totals (
 id INTEGER PRIMARY KEY CHECK(id=1), discarded_rows INTEGER NOT NULL DEFAULT 0,
 noise_rows INTEGER NOT NULL DEFAULT 0, expired_rows INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_asset_logs_run_id_id ON logs(run_id,id);
CREATE INDEX IF NOT EXISTS idx_asset_logs_created_at ON logs(created_at,id);
";
pub(crate) fn initialize(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(RETENTION_SCHEMA)
        .map_err(|e| e.to_string())
}
#[derive(Clone, Copy)]
struct LogLimits {
    per_run: i64,
    global: i64,
    days: i64,
}
impl Default for LogLimits {
    fn default() -> Self {
        Self {
            per_run: 5000,
            global: 100000,
            days: 30,
        }
    }
}

fn discard(
    connection: &Connection,
    rows: Vec<(i64, Option<i64>, String, String, String)>,
    expired: bool,
) -> Result<(), String> {
    for (id, run, level, stage, message) in rows {
        // Only classify an actual repeated neighbouring diagnostic as noise.
        // Healthy repeated messages remain separate rows while under budget.
        let noise = if !expired && matches!(level.as_str(), "trace" | "debug" | "info") {
            connection.query_row("SELECT level=?2 AND stage=?3 AND message=?4 FROM logs WHERE run_id IS ?1 AND id>?5 ORDER BY id LIMIT 1",
                params![run,level,stage,message,id],|row|row.get::<_,bool>(0)).optional().map_err(|e|e.to_string())?.unwrap_or(false)
        } else {
            false
        };
        connection
            .execute("DELETE FROM logs WHERE id=?1", [id])
            .map_err(|e| e.to_string())?;
        let noise = i64::from(noise);
        let expired = i64::from(expired);
        if let Some(run) = run {
            connection.execute("INSERT INTO asset_log_retention(run_id,discarded_rows,noise_rows,expired_rows,updated_at)
                VALUES(?1,1,?2,?3,datetime('now','localtime')) ON CONFLICT(run_id) DO UPDATE SET
                discarded_rows=discarded_rows+1,noise_rows=noise_rows+excluded.noise_rows,expired_rows=expired_rows+excluded.expired_rows,updated_at=excluded.updated_at",
                params![run,noise,expired]).map_err(|e|e.to_string())?;
        }
        connection.execute("INSERT INTO asset_log_retention_totals(id,discarded_rows,noise_rows,expired_rows,updated_at)
            VALUES(1,1,?1,?2,datetime('now','localtime')) ON CONFLICT(id) DO UPDATE SET
            discarded_rows=discarded_rows+1,noise_rows=noise_rows+excluded.noise_rows,expired_rows=expired_rows+excluded.expired_rows,updated_at=excluded.updated_at",
            params![noise,expired]).map_err(|e|e.to_string())?;
    }
    Ok(())
}
fn trim_query(
    connection: &Connection,
    sql: &str,
    arg: i64,
    expired: bool,
) -> Result<usize, String> {
    let mut statement = connection.prepare(sql).map_err(|e| e.to_string())?;
    let rows = statement
        .query_map([arg], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let count = rows.len();
    drop(statement);
    discard(connection, rows, expired)?;
    Ok(count)
}
fn enforce(connection: &Connection, limits: LogLimits) -> Result<(), String> {
    // Process old databases in bounded memory. Invalid legacy date strings are
    // never guessed to be expired; row caps still bound their retained count.
    while trim_query(
        connection,
        "SELECT id,run_id,level,stage,message FROM logs
        WHERE julianday(created_at)<julianday('now','localtime')-?1 ORDER BY id LIMIT 512",
        limits.days,
        true,
    )? > 0
    {}
    let overfull = {
        let mut statement=connection.prepare("SELECT run_id FROM logs WHERE run_id IS NOT NULL GROUP BY run_id HAVING count(*)>?1").map_err(|e|e.to_string())?;
        let rows = statement
            .query_map([limits.per_run], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    for run in overfull {
        loop {
            let count = connection
                .query_row("SELECT count(*) FROM logs WHERE run_id=?1", [run], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(|e| e.to_string())?;
            if count <= limits.per_run {
                break;
            }
            let mut statement=connection.prepare("SELECT id,run_id,level,stage,message FROM logs WHERE run_id=?1 ORDER BY id LIMIT ?2").map_err(|e|e.to_string())?;
            let rows = statement
                .query_map(params![run, (count - limits.per_run).min(512)], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            drop(statement);
            discard(connection, rows, false)?;
        }
    }
    loop {
        let count = connection
            .query_row("SELECT count(*) FROM logs", [], |row| row.get::<_, i64>(0))
            .map_err(|e| e.to_string())?;
        if count <= limits.global {
            break;
        }
        trim_query(
            connection,
            "SELECT id,run_id,level,stage,message FROM logs ORDER BY id LIMIT ?1",
            (count - limits.global).min(512),
            false,
        )?;
    }
    Ok(())
}

/// Read-only metadata for the existing list_logs surface. No marker append loop.
pub(crate) fn retention_metadata(
    connection: &Connection,
    run_id: Option<i64>,
    project_id: Option<i64>,
) -> Result<serde_json::Value, String> {
    let available=connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='asset_log_retention_totals')",[],|row|row.get::<_,bool>(0)).map_err(|e|e.to_string())?;
    let (discarded, noise, expired, updated): (i64, i64, i64, String) = if !available {
        (0, 0, 0, String::new())
    } else if run_id.is_none() && project_id.is_none() {
        connection.query_row("SELECT discarded_rows,noise_rows,expired_rows,updated_at FROM asset_log_retention_totals WHERE id=1",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).optional().map_err(|e|e.to_string())?.unwrap_or_default()
    } else {
        connection.query_row("SELECT coalesce(sum(t.discarded_rows),0),coalesce(sum(t.noise_rows),0),coalesce(sum(t.expired_rows),0),coalesce(max(t.updated_at),'')
                FROM asset_log_retention t JOIN runs r ON r.id=t.run_id WHERE (?1 IS NULL OR t.run_id=?1) AND (?2 IS NULL OR r.project_id=?2)",params![run_id,project_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).map_err(|e|e.to_string())?
    };
    Ok(
        serde_json::json!({"schemaVersion":1,"scope":"diagnostic_logs_only","discardedRows":discarded,"repeatedNoiseRows":noise,"expiredRows":expired,"updatedAt":updated,
        "perRunLimit":5000,"globalLimit":100000,"ageDays":30,"manualAuditTablesPruned":false}),
    )
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogHint {
    pub log_id: i64,
    pub run_id: i64,
    pub project_id: Option<i64>,
}

pub(crate) fn append(
    connection: &mut Connection,
    run_id: i64,
    level: &str,
    stage: &str,
    message: &str,
    notify: impl FnOnce(&LogHint),
) -> Result<(), String> {
    append_with_limits(
        connection,
        run_id,
        level,
        stage,
        message,
        notify,
        LogLimits::default(),
    )
}
fn append_with_limits(
    connection: &mut Connection,
    run_id: i64,
    level: &str,
    stage: &str,
    message: &str,
    notify: impl FnOnce(&LogHint),
    limits: LogLimits,
) -> Result<(), String> {
    initialize(connection)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "INSERT INTO logs(run_id,level,stage,message) VALUES(?1,?2,?3,?4)",
            params![
                run_id,
                log_display::text(level, 32),
                log_display::text(stage, 120),
                log_display::line(message)
            ],
        )
        .map_err(|error| error.to_string())?;
    let hint = LogHint {
        log_id: transaction.last_insert_rowid(),
        run_id,
        project_id: transaction
            .query_row("SELECT project_id FROM runs WHERE id=?1", [run_id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|error| error.to_string())?,
    };
    enforce(&transaction, limits)?;
    transaction.commit().map_err(|error| error.to_string())?;
    notify(&hint);
    Ok(())
}

pub(crate) fn read(
    connection: &Connection,
    run_id: Option<i64>,
    project_id: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<LogEntry>, String> {
    let mut statement = connection
        .prepare(
            "SELECT l.id,l.run_id,l.level,l.stage,l.message,l.created_at
         FROM logs l LEFT JOIN runs r ON r.id=l.run_id
         WHERE (?1 IS NULL OR l.run_id=?1) AND (?2 IS NULL OR r.project_id=?2)
         ORDER BY l.id DESC LIMIT ?3",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![run_id, project_id, limit.unwrap_or(500).clamp(1, 2000)],
            |row| {
                Ok(LogEntry {
                    id: row.get(0)?,
                    run_id: row.get(1)?,
                    level: log_display::text(&row.get::<_, String>(2)?, 32),
                    stage: log_display::text(&row.get::<_, String>(3)?, 120),
                    message: log_display::line(&row.get::<_, String>(4)?),
                    created_at: row.get(5)?,
                })
            },
        )
        .map_err(|error| error.to_string())?;
    let mut rows = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let metadata = retention_metadata(connection, run_id, project_id)?;
    if let Some(discarded) = metadata["discardedRows"]
        .as_i64()
        .filter(|count| *count > 0)
    {
        rows.truncate(limit.unwrap_or(500).clamp(1, 2000) as usize - 1);
        // id=0 is a synthetic view notice, never a persisted log row identity.
        rows.insert(0,LogEntry{id:0,run_id,level:"info".into(),stage:"retention".into(),
            message:format!("Diagnostic log retention removed {discarded} rows ({} expired, {} repeated noise). Limits: 5000/run, 100000 total, 30 days. Manual audit records are separate.",metadata["expiredRows"],metadata["repeatedNoiseRows"]),
            created_at:metadata["updatedAt"].as_str().unwrap_or("").into()});
    }
    Ok(rows)
}

#[cfg(test)]
#[path = "asset_logs_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "asset_log_retention_tests.rs"]
mod retention_tests;
