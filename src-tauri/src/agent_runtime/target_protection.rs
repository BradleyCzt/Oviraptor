//! Pure durable target denial; no execution, signing or repair authority.
use super::contract::terminal_code;
use rusqlite::params;
use serde_json::Value as JsonValue;

pub(crate) struct Stop {
    pub code: &'static str,
    pub reason: String,
}

// A durable denial, never a signed response proof or an execution/recovery grant.
pub(crate) fn read_on(
    db: &rusqlite::Connection,
    scan: &str,
    target: &str,
) -> Result<Option<Stop>, String> {
    use rusqlite::OptionalExtension;
    verify_schema_on(db)?;
    let stored: Option<(String, String, String, String, i64, String)> = db.query_row(
        "SELECT stop_code,reason,claim_json,claim_hash,source_attempt_number,source_run_id FROM main.agent_protection_stops
         WHERE scan_id=?1 AND target_url=?2",
        params![scan, target],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
    ).optional().map_err(|_| "protection_stop_read_failed")?;
    let Some((code, reason, original, hash, attempt, run)) = stored else {
        return Ok(None);
    };
    let code = validate_code(&code)?;
    let fact: JsonValue =
        serde_json::from_str(&original).map_err(|_| "protection_stop_original_claim_invalid")?;
    if fact["scanId"] != scan
        || fact["targetUrl"] != target
        || fact["attemptNumber"].as_i64().is_none_or(|v| v <= 0)
        || fact["attemptNumber"] != attempt
        || fact["runId"] != run
        || run.is_empty()
        || ["rowid", "budgetAttempt", "ordinal", "requestIndex"]
            .iter()
            .any(|key| fact[*key].as_i64().is_none_or(|v| v <= 0))
        || [
            "runId",
            "invocationId",
            "toolName",
            "identityHandle",
            "requestHash",
            "createdAt",
        ]
        .iter()
        .any(|key| fact[*key].as_str().is_none_or(str::is_empty))
        || fact["responseStatus"]
            .as_u64()
            .is_none_or(|v| !(100..=599).contains(&v))
        || fact["receivedAt"].as_str().is_none_or(str::is_empty)
        || crate::agent_runtime::store::stable_hash(&original) != hash
        || reason.is_empty()
        || reason.chars().count() > 2000
        || (code == terminal_code::PERSISTENT_RATE_LIMIT && fact["responseStatus"] != 429)
    {
        return Err("protection_stop_original_claim_invalid".into());
    }
    Ok(Some(Stop { code, reason }))
}

pub(crate) fn validate_code(code: &str) -> Result<&'static str, String> {
    match code {
        terminal_code::CONFIRMED_CHALLENGE => Ok(terminal_code::CONFIRMED_CHALLENGE),
        terminal_code::PERSISTENT_RATE_LIMIT => Ok(terminal_code::PERSISTENT_RATE_LIMIT),
        _ => Err("protection_stop_code_invalid".into()),
    }
}

pub(crate) fn verify_schema_on(db: &rusqlite::Connection) -> Result<(), String> {
    let shadow: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name='agent_protection_stops')",
            [],
            |r| r.get(0),
        )
        .map_err(|_| "protection_stop_schema_unavailable")?;
    let present: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE type='table' AND name='agent_protection_stops')",
        [], |r| r.get(0),
    ).map_err(|_| "protection_stop_schema_unavailable")?;
    if shadow || !present {
        return Err("protection_stop_schema_unavailable".into());
    }
    for (name, expected) in PROTECTION_STOP_SCHEMA {
        let actual: String = db
            .query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1 AND sql IS NOT NULL",
                [name],
                |r| r.get(0),
            )
            .map_err(|_| "protection_stop_schema_unavailable")?;
        let normalize = |sql: &str| {
            sql.replace("IF NOT EXISTS ", "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .trim_end_matches(';')
                .to_string()
        };
        if normalize(&actual) != normalize(expected) {
            return Err("protection_stop_schema_changed".into());
        }
    }
    Ok(())
}

// Keep exact isolated DDL in sync with db_schema.rs. Missing or weakened
// immutability must refuse target admission even when no latch row is present.
const PROTECTION_STOP_SCHEMA: &[(&str, &str)] = &[
    ("agent_protection_stops", "CREATE TABLE IF NOT EXISTS agent_protection_stops (
    scan_id TEXT NOT NULL CHECK(length(scan_id)>0),
    target_url TEXT NOT NULL CHECK(length(target_url)>0),
    source_attempt_number INTEGER NOT NULL CHECK(source_attempt_number>0),
    source_run_id TEXT NOT NULL CHECK(length(source_run_id)>0),
    stop_code TEXT NOT NULL CHECK(stop_code IN ('confirmed_waf_or_challenge','persistent_rate_limit')),
    reason TEXT NOT NULL CHECK(length(reason)>0 AND length(reason)<=2000),
    claim_json TEXT NOT NULL CHECK(json_valid(claim_json)),
    claim_hash TEXT NOT NULL CHECK(length(claim_hash)=64),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,target_url)
);"),
    ("protection_stop_no_update", "CREATE TRIGGER IF NOT EXISTS protection_stop_no_update
BEFORE UPDATE ON agent_protection_stops
BEGIN SELECT RAISE(ABORT,'protection_stop_immutable'); END;"),
    ("protection_stop_no_delete", "CREATE TRIGGER IF NOT EXISTS protection_stop_no_delete
BEFORE DELETE ON agent_protection_stops
BEGIN SELECT RAISE(ABORT,'protection_stop_immutable'); END;"),
    ("protection_stop_no_replace", "CREATE TRIGGER IF NOT EXISTS protection_stop_no_replace
BEFORE INSERT ON agent_protection_stops
WHEN EXISTS(SELECT 1 FROM agent_protection_stops old WHERE old.rowid=NEW.rowid
    OR (old.scan_id=NEW.scan_id AND old.target_url=NEW.target_url))
BEGIN SELECT RAISE(ABORT,'protection_stop_immutable'); END;"),
];

pub(crate) fn require_clear_on(
    db: &rusqlite::Connection,
    scan: &str,
    target: &str,
) -> Result<(), String> {
    if read_on(db, scan, target)?.is_some() {
        return Err("target_protection_stop_active".into());
    }
    Ok(())
}
