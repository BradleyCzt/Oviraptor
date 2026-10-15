//! Explicit task-local handling of operator-authorized identity material.
//! Missing policy retains the legacy behavior; financial/provider credentials
//! never enter this payload policy.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

pub(crate) fn plaintext(db: &Connection, scan: &str, attempt: i64) -> Result<bool, String> {
    if !super::model_worker::enabled(db, scan, attempt)? { return Ok(false); }
    let text: String = db.query_row(
        "SELECT plan_json FROM native_scan_plans WHERE scan_id=?1 AND attempt_number=?2",
        params![scan, attempt], |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    let plan: Value = serde_json::from_str(&text).map_err(|_| "homogeneous_frozen_plan_invalid")?;
    Ok(plan["credentialVisibility"] == "plaintext")
}

pub(crate) fn for_root(db: &Connection, root: &str) -> Result<bool, String> {
    let scope: Option<(String, i64)> = db.query_row(
        "SELECT scan_id,attempt_number FROM agent_runs WHERE id=?1 AND role='coordinator'",
        [root], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional().map_err(|e| e.to_string())?;
    match scope { Some((scan, attempt)) => {
        // A Web birth receipt seals the original Root plan even if mutable
        // Root JSON has lost the execution discriminator entirely.
        let _original_web_mode = crate::agent_runtime::web_mode::root::read(db,root)?;
        // Verify an original execution binding before considering a different
        // current protocol. A rehashed Source-shaped row cannot erase birth.
        let original_execution = super::execution_policy::read_original_on(db,root)?.is_some();
        if super::execution_policy::Binding::capture(db,&scan,attempt)?.is_some()
            && !original_execution{return Ok(false);}
        plaintext(db,&scan,attempt)
    }, None => Ok(false) }
}

pub(crate) fn text(db: &Connection, root: &str, text: &str) -> Result<String, String> {
    Ok(if for_root(db, root)? { text.to_string() }
        else { crate::agent_runtime::secrets::redact_text_with(text, None) })
}

pub(crate) fn json(db: &Connection, root: &str, value: &Value) -> Result<Value, String> {
    Ok(if for_root(db, root)? { value.clone() }
        else { crate::agent_runtime::secrets::redact_json(value) })
}
