//! New Native process diagnostics. Commit rows before sending identity hints.
//! This does not store or modify the helper's machine-readable stdout buffer.
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

pub(crate) mod driver;
mod guard;
mod persist;

#[cfg(test)]
pub(crate) const SCHEMA: &str = include_str!("schema.sql");
const MAX_ROWS: i64 = 8192;
const MAX_BYTES: i64 = 1024 * 1024;
const PAGE_LIMIT: i64 = 300;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Scope {
    pub scan_id: String,
    pub attempt: i64,
    pub branch: String,
    pub dispatch_claim_id: String,
    pub invocation_key: String,
    pub execution_id: String,
    pub stage: String,
}

impl Scope {
    fn valid(&self) -> bool {
        self.attempt > 0
            && matches!(self.branch.as_str(), "source" | "web")
            && uuid::Uuid::parse_str(&self.dispatch_claim_id).is_ok()
            && [
                self.scan_id.as_str(),
                self.execution_id.as_str(),
                self.stage.as_str(),
            ]
            .iter()
            .all(|value| {
                !value.is_empty()
                    && value.len() <= 128
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_:.".contains(&byte))
            })
            && self.invocation_key.len() == 64
            && self
                .invocation_key
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
    }
}

// The journal supplies scan scope; the process mechanism supplies bounded output.
pub(crate) use crate::execution_service::OutputEvent as Event;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Hint {
    #[serde(flatten)]
    pub scope: Scope,
    pub sequence: i64,
    pub stream: String,
    pub stream_sequence: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Row {
    #[serde(flatten)]
    pub hint: Hint,
    pub message: String,
    pub gap: bool,
    pub time: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    pub rows: Vec<Row>,
    pub more: bool,
    pub after_sequence: i64,
    pub available: bool,
    pub execution_state: Option<String>,
    pub latest_sequence: i64,
}

pub(crate) struct Journal {
    connection: Connection,
    scope: Scope,
}

impl Journal {
    pub(crate) fn begin(path: &Path, scope: Scope) -> Result<Self, String> {
        if !scope.valid() {
            return Err("native_process_log_scope_invalid".into());
        }
        let connection = persist::begin(path, &scope)?;
        Ok(Self { connection, scope })
    }

    pub(crate) fn append(
        &mut self,
        event: &Event,
        notify: &mut dyn FnMut(&Hint) -> Result<(), String>,
    ) -> Result<i64, String> {
        self.append_inner(event, None, notify)
    }

    pub(crate) fn finish(
        &mut self,
        state: &str,
        notify: &mut dyn FnMut(&Hint) -> Result<(), String>,
    ) -> Result<i64, String> {
        if !matches!(
            state,
            "completed" | "failed" | "cancelled" | "timeout" | "gap"
        ) {
            return Err("native_process_log_terminal_invalid".into());
        }
        self.append_inner(
            &Event {
                stream: "status",
                stream_sequence: 1,
                message: state.into(),
                gap: state == "gap",
            },
            Some(state),
            notify,
        )
    }

    fn append_inner(
        &mut self,
        event: &Event,
        terminal: Option<&str>,
        notify: &mut dyn FnMut(&Hint) -> Result<(), String>,
    ) -> Result<i64, String> {
        if !matches!(event.stream, "stdout" | "stderr" | "status" | "gap")
            || event.stream_sequence <= 0
            || event.message.len() > 64 * 1024
        {
            return Err("native_process_log_row_invalid".into());
        }
        // Defense at the actual persistence boundary, even for a caller without framing.
        let message = crate::agent_runtime::secrets::redact_text_with(&event.message, None);
        let hint = persist::append(&mut self.connection, &self.scope, event, &message, terminal)?;
        // A failed notification is recoverable by sequence; never undo committed data.
        let _ = notify(&hint);
        Ok(hint.sequence)
    }
}

pub(crate) fn page(
    connection: &Connection,
    scan_id: &str,
    attempt: i64,
    execution_id: Option<&str>,
    after: i64,
    limit: i64,
) -> Result<Page, String> {
    if scan_id.is_empty() || attempt <= 0 || after < 0 || !(1..=PAGE_LIMIT).contains(&limit) {
        return Err("native_process_log_cursor_invalid".into());
    }
    // An unpinned read restores the whole task/attempt even after all hints or
    // remembered execution IDs were lost; pinned reads isolate one process.
    let (available,execution_state,latest_sequence):(bool,Option<String>,i64)=connection.query_row(
        "SELECT COUNT(DISTINCT e.execution_id)>0,
         CASE WHEN ?3 IS NULL THEN NULL ELSE MAX(e.state) END,COALESCE(MAX(r.sequence),0)
         FROM native_process_log_executions e LEFT JOIN native_process_log_rows r ON r.execution_id=e.execution_id
         WHERE e.scan_id=?1 AND e.attempt_number=?2 AND (?3 IS NULL OR e.execution_id=?3)",
        params![scan_id,attempt,execution_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))
        .map_err(|_|"native_process_log_read_failed")?;
    if after > latest_sequence {
        return Err("native_process_log_future_cursor".into());
    }
    let mut statement = connection.prepare("SELECT r.sequence,e.scan_id,e.attempt_number,
        e.invocation_key,e.execution_id,e.stage,e.branch,e.dispatch_claim_id,r.stream,r.stream_sequence,r.message,r.gap,r.created_at
        FROM native_process_log_rows r JOIN native_process_log_executions e ON e.execution_id=r.execution_id
        WHERE e.scan_id=?1 AND e.attempt_number=?2 AND (?3 IS NULL OR e.execution_id=?3) AND r.sequence>?4
        ORDER BY r.sequence LIMIT ?5").map_err(|_| "native_process_log_read_failed")?;
    let mut rows = statement
        .query_map(
            params![scan_id, attempt, execution_id, after, limit + 1],
            |row| {
                Ok(Row {
                    hint: Hint {
                        sequence: row.get(0)?,
                        scope: Scope {
                            scan_id: row.get(1)?,
                            attempt: row.get(2)?,
                            invocation_key: row.get(3)?,
                            execution_id: row.get(4)?,
                            stage: row.get(5)?,
                            branch: row.get(6)?,
                            dispatch_claim_id: row.get(7)?,
                        },
                        stream: row.get(8)?,
                        stream_sequence: row.get(9)?,
                    },
                    message: crate::agent_runtime::secrets::redact_text_with(
                        &row.get::<_, String>(10)?,
                        None,
                    ),
                    gap: row.get(11)?,
                    time: row.get(12)?,
                })
            },
        )
        .map_err(|_| "native_process_log_read_failed")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "native_process_log_read_failed")?;
    let more = rows.len() > limit as usize;
    if more {
        rows.pop();
    }
    let after_sequence = rows.last().map_or(after, |row| row.hint.sequence);
    Ok(Page {
        rows,
        more,
        after_sequence,
        available,
        execution_state,
        latest_sequence,
    })
}

#[cfg(test)]
mod write_guard_tests;
