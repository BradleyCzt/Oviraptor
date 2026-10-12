//! SDK-only cursor. It never reads or changes Native process cursor meanings.
use super::owner::Owner;
use super::replay_metadata::Gap;
use crate::agent_runtime::store::stable_hash;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::{path::Path, time::Duration};
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Row {
    #[serde(flatten)]
    pub owner: Owner,
    pub sequence: i64,
    pub ordinal: i64,
    pub stage: String,
    pub cost_phase: String,
    pub terminal_state: String,
    pub time: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub schema_version: u8,
    pub channel: &'static str,
    pub scan_id: String,
    pub attempt: i64,
    pub owner_id: Option<String>,
    pub requested_after_sequence: i64,
    pub reset_cursor: bool,
    pub after_sequence: i64,
    pub latest_sequence: i64,
    pub more: bool,
    pub available: bool,
    pub rows: Vec<Row>,
    pub owners: Vec<Owner>,
    pub owners_truncated: bool,
    pub gaps: Vec<Gap>,
    pub gaps_truncated: bool,
    pub incomplete_owners: Vec<String>,
    pub incomplete_owners_truncated: bool,
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn read(
    path: &Path,
    scan: &str,
    attempt: i64,
    cursor: Option<i64>,
    owner: Option<&str>,
    after: i64,
    limit: i64,
) -> Result<Snapshot, String> {
    if scan.is_empty()
        || scan.len() > 128
        || attempt < 0
        || after < 0
        || !(1..=300).contains(&limit)
        || cursor.is_some_and(|v| v <= 0)
        || (after > 0 && cursor.is_none())
        || owner.is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("native_sdk_log_cursor_invalid".into());
    }
    let mut db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|_| "native_sdk_log_database_unavailable")?;
    db.busy_timeout(Duration::from_millis(25))
        .map_err(|_| "native_sdk_log_database_unavailable")?;
    db.pragma_update(None, "query_only", true)
        .map_err(|_| "native_sdk_log_database_unavailable")?;
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .map_err(|_| "native_sdk_log_read_failed")?;
    let result = read_snapshot(&tx, scan, attempt, cursor, owner, after, limit)?;
    tx.commit().map_err(|_| "native_sdk_log_read_failed")?;
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn read_snapshot(
    tx: &Connection,
    scan: &str,
    attempt: i64,
    cursor: Option<i64>,
    owner: Option<&str>,
    after: i64,
    limit: i64,
) -> Result<Snapshot, String> {
    let latest:Option<i64>=tx.query_row("SELECT attempt_count FROM sentinel_scans WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans WHERE scan_id=?1)",[scan],|r|r.get(0)).optional().map_err(|_|"native_sdk_log_read_failed")?;
    let latest = latest.ok_or("native_sdk_log_scan_unavailable")?;
    let actual = if attempt == 0 { latest } else { attempt };
    let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2)",params![scan,actual],|r|r.get(0)).map_err(|_|"native_sdk_log_read_failed")?;
    if actual < 1 || !exists {
        return Err("native_sdk_log_attempt_unavailable".into());
    }
    let reset = cursor.is_some_and(|v| v != actual);
    if reset && (attempt > 0 || owner.is_some()) {
        return Err("native_sdk_log_cursor_scope_mismatch".into());
    }
    if let Some(owner) = owner {
        let owned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM native_sdk_log_owners WHERE owner_id=?1 AND scan_id=?2 AND attempt_number=?3)",params![owner,scan,actual],|r|r.get(0)).map_err(|_|"native_sdk_log_read_failed")?;
        if !owned {
            return Err("native_sdk_log_owner_unavailable".into());
        }
    }
    let (available,high):(bool,i64)=tx.query_row("SELECT COUNT(DISTINCT o.owner_id)>0,COALESCE(MAX(r.sequence),0) FROM native_sdk_log_owners o LEFT JOIN native_sdk_log_rows r ON r.owner_id=o.owner_id WHERE o.scan_id=?1 AND o.attempt_number=?2 AND (?3 IS NULL OR o.owner_id=?3)",params![scan,actual,owner],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"native_sdk_log_read_failed")?;
    let metadata = super::replay_metadata::load(tx, scan, actual, owner)?;
    let requested_after_sequence = after;
    let after = if reset { 0 } else { after };
    if after > high {
        return Err("native_sdk_log_future_cursor".into());
    }
    let mut rows = {
        let mut query=tx.prepare("SELECT o.owner_id,o.domain,o.dispatch_key,o.scan_id,o.attempt_number,o.root_run_id,o.run_id,
    o.assignment_id,o.lease_attempt_id,o.worker_id,o.round_number,o.request_hash,o.identity_hash,
    r.sequence,r.ordinal,r.stage,r.cost_phase,r.terminal_state,r.created_at FROM native_sdk_log_rows r
    JOIN native_sdk_log_owners o ON o.owner_id=r.owner_id WHERE o.scan_id=?1 AND o.attempt_number=?2
      AND (?3 IS NULL OR o.owner_id=?3) AND r.sequence>?4 ORDER BY r.sequence LIMIT ?5").map_err(|_|"native_sdk_log_read_failed")?;
        let values = query
            .query_map(params![scan, actual, owner, after, limit + 1], |r| {
                Ok((
                    Row {
                        owner: Owner {
                            owner_id: r.get(0)?,
                            domain: r.get(1)?,
                            dispatch_key: r.get(2)?,
                            scan_id: r.get(3)?,
                            attempt: r.get(4)?,
                            root_run_id: r.get(5)?,
                            run_id: r.get(6)?,
                            assignment_id: r.get(7)?,
                            lease_attempt_id: r.get(8)?,
                            worker_id: r.get(9)?,
                            round: r.get(10)?,
                            request_hash: r.get(11)?,
                        },
                        sequence: r.get(13)?,
                        ordinal: r.get(14)?,
                        stage: r.get(15)?,
                        cost_phase: r.get(16)?,
                        terminal_state: r.get(17)?,
                        time: r.get(18)?,
                    },
                    r.get::<_, String>(12)?,
                ))
            })
            .map_err(|_| "native_sdk_log_read_failed")?;
        let mut rows = Vec::new();
        for value in values {
            let (mut row, identity) = value.map_err(|_| "native_sdk_log_read_failed")?;
            super::owner_mapper_loop::restore_logical(tx, &mut row.owner, &identity)?;
            if row.owner.scan_id != scan
                || row.owner.attempt != actual
                || stable_hash(
                    &serde_json::to_string(&row.owner).map_err(|_| "native_sdk_log_read_failed")?,
                ) != identity
                || row.owner.owner_id
                    != stable_hash(
                        &serde_json::json!([row.owner.domain, row.owner.dispatch_key]).to_string(),
                    )
            {
                return Err("native_sdk_log_owner_unverified".into());
            }
            super::owner_saved::verify(tx, &row.owner)?;
            super::transitions::verify_prefix(tx, &row.owner.owner_id)?;
            rows.push(row);
        }
        rows
    };
    let more = rows.len() > limit as usize;
    if more {
        rows.pop();
    }
    let delivered = rows.last().map_or(after, |r| r.sequence);
    Ok(Snapshot {
        schema_version: 2,
        channel: "native_sdk",
        scan_id: scan.into(),
        attempt: actual,
        owner_id: owner.map(str::to_string),
        requested_after_sequence,
        reset_cursor: reset,
        after_sequence: delivered,
        latest_sequence: high,
        more,
        available,
        rows,
        owners: metadata.owners,
        owners_truncated: metadata.owners_truncated,
        gaps: metadata.gaps,
        gaps_truncated: metadata.gaps_truncated,
        incomplete_owners: metadata.incomplete_owners,
        incomplete_owners_truncated: metadata.incomplete_owners_truncated,
    })
}
