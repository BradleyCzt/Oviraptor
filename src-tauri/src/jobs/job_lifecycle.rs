use super::JobProgressEvent;
use rusqlite::{params, OptionalExtension};

pub(super) fn request_cancel_on(
    connection: &rusqlite::Connection,
    run_id: i64,
) -> Result<(), String> {
    let changed = connection
        .execute(
            "UPDATE runs SET status='cancel_requested' WHERE id=?1 AND status IN ('queued','running','cancel_requested')",
            [run_id],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("任务已结束或不在运行中".into());
    }
    Ok(())
}

fn finish_on(
    connection: &mut rusqlite::Connection,
    run_id: i64,
    result: Result<(), String>,
    cancelled: bool,
) -> Result<JobProgressEvent, String> {
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let status: Option<String> = transaction
        .query_row("SELECT status FROM runs WHERE id=?1", [run_id], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|error| error.to_string())?;
    let status = status.ok_or("任务不存在，未写入终态")?;
    if !matches!(status.as_str(), "queued" | "running" | "cancel_requested") {
        return Err(format!("任务已处于 {status}，未覆盖既有终态"));
    }
    let (terminal, error, message, progress) = match result {
        Ok(()) if cancelled || status == "cancel_requested" => (
            "cancelled",
            String::new(),
            "任务已取消；资产任务可从原目录断点继续".to_string(),
            0.0,
        ),
        Ok(()) => ("completed", String::new(), "任务完成".to_string(), 100.0),
        Err(error) if error == "__CANCELLED__" => (
            "cancelled",
            String::new(),
            "任务已取消；资产任务可从原目录断点继续".to_string(),
            0.0,
        ),
        Err(error) => {
            let error = crate::log_display::text(&error, 8000);
            (
                "failed",
                error.clone(),
                crate::log_display::line(&error),
                0.0,
            )
        }
    };
    let changed = transaction
        .execute(
            "UPDATE runs SET status=?1,stage=?1,error=?2,finished_at=datetime('now','localtime'),progress=CASE WHEN ?1='completed' THEN 100 ELSE progress END WHERE id=?3",
            params![terminal, error, run_id],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("任务终态更新未生效，未发送完成通知".into());
    }
    let persisted: (String, String, String, Option<String>, f64) = transaction
        .query_row(
            "SELECT status,stage,error,finished_at,progress FROM runs WHERE id=?1",
            [run_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    if persisted.0 != terminal
        || persisted.1 != terminal
        || persisted.2 != error
        || persisted.3.as_deref().is_none_or(str::is_empty)
        || (terminal == "completed" && persisted.4 != 100.0)
    {
        return Err("任务终态校验失败，未发送完成通知".into());
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(JobProgressEvent {
        run_id,
        status: terminal.into(),
        stage: terminal.into(),
        progress,
        message,
    })
}

pub(super) fn finish_with_fallback(
    connection: &mut rusqlite::Connection,
    run_id: i64,
    result: Result<(), String>,
    cancelled: bool,
) -> Result<JobProgressEvent, String> {
    let original_error = result.as_ref().err().cloned();
    match finish_on(connection, run_id, result, cancelled) {
        Ok(event) => Ok(event),
        Err(error) => {
            // A rejected completed/cancelled write must never emit success. If
            // storage still permits a failed terminal record, persist the cause.
            let error = match original_error {
                Some(original) => format!("{original}；任务终态持久化失败：{error}"),
                None => format!("任务终态持久化失败：{error}"),
            };
            finish_on(connection, run_id, Err(error.clone()), false)
                .map_err(|fallback| format!("{error}；失败状态也未能持久化：{fallback}"))
        }
    }
}

#[cfg(test)]
#[path = "job_lifecycle_tests.rs"]
mod tests;
