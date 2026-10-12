//! Creation-only immutable mode declaration. Caller owns the new draft TX.
use super::WebMode;
use rusqlite::{params, Connection, Transaction};
use serde_json::Value;

pub(crate) struct DraftCreationWindow<'tx, 'db> {
    tx: &'tx Transaction<'db>,
    original_scans: Vec<String>,
}
pub(crate) struct FreshModeDraft<'tx, 'db> {
    tx: &'tx Transaction<'db>,
    scan: String,
    draft: String,
}
impl<'tx, 'db> FreshModeDraft<'tx, 'db> {
    pub(crate) fn transaction(&self) -> &'tx Transaction<'db> {
        self.tx
    }
    pub(crate) fn scan_id(&self) -> &str {
        &self.scan
    }
    pub(crate) fn draft_id(&self) -> &str {
        &self.draft
    }
}
impl<'tx, 'db> DraftCreationWindow<'tx, 'db> {
    // Capture BEFORE the ordinary creator inserts its newly generated scan id.
    // An existing draft is denied even when it has never spent any budget.
    pub(crate) fn begin(tx: &'tx Transaction<'db>) -> Result<Self, String> {
        let mut statement = tx
            .prepare("SELECT id FROM sentinel_scans ORDER BY id")
            .map_err(|e| e.to_string())?;
        let original_scans = statement
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string())?;
        Ok(Self { tx, original_scans })
    }
}
pub(crate) fn register_new<'tx, 'db>(
    window: DraftCreationWindow<'tx, 'db>,
    scan: &str,
    mode: WebMode,
) -> Result<FreshModeDraft<'tx, 'db>, String> {
    if window.original_scans.iter().any(|old| old == scan) {
        return Err("web_mode_requires_new_draft".into());
    }
    let tx = window.tx;
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    tx.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Insert {
            table_name: "native_web_mode_drafts",
        } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
        AuthAction::Update {
            table_name: "sentinel_scan_contexts",
            column_name: "policy_json",
        } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }))
    .map_err(|e| e.to_string())?;
    let result = register_new_on(tx, scan, mode);
    let clear = tx
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    result?;
    clear?;
    Ok(FreshModeDraft {
        tx,
        scan: scan.into(),
        draft: read(tx, scan)?.0,
    })
}

fn register_new_on(tx: &Transaction<'_>, scan: &str, mode: WebMode) -> Result<(), String> {
    let fresh: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=?1 AND scan_type='web'
        AND status='draft' AND attempt_count=0 AND task_path='')
        AND NOT EXISTS(SELECT 1 FROM sentinel_scan_attempts WHERE scan_id=?1)
        AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE scan_id=?1)
        AND NOT EXISTS(SELECT 1 FROM native_branch_dispatches WHERE scan_id=?1)
        AND NOT EXISTS(SELECT 1 FROM native_web_mode_drafts WHERE scan_id=?1)",
            [scan],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !fresh {
        return Err("web_mode_requires_new_draft".into());
    }
    let text: String = tx
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut policy: Value = serde_json::from_str(&text).map_err(|_| "web_mode_policy_invalid")?;
    let fields = policy.as_object_mut().ok_or("web_mode_policy_invalid")?;
    if fields.contains_key("orchestration") {
        return Err("web_mode_already_declared".into());
    }
    fields.insert(
        "orchestration".into(),
        serde_json::json!({"version":1,"mode":mode}),
    );
    if tx
        .execute(
            "UPDATE sentinel_scan_contexts SET policy_json=?2 WHERE scan_id=?1",
            params![scan, policy.to_string()],
        )
        .map_err(|e| e.to_string())?
        != 1
    {
        return Err("web_mode_draft_policy_missing".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    if tx
        .execute(
            "INSERT INTO native_web_mode_drafts(scan_id,draft_id,mode) VALUES(?1,?2,?3)",
            params![scan, id, mode.as_str()],
        )
        .map_err(|e| e.to_string())?
        != 1
    {
        return Err("web_mode_draft_missing".into());
    }
    let saved: String = tx
        .query_row(
            "SELECT policy_json FROM sentinel_scan_contexts WHERE scan_id=?1",
            [scan],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let canonical_policy = serde_json::to_string(&policy).map_err(|_| "web_mode_policy_invalid")?;
    if saved != canonical_policy || read(tx, scan)? != (id, mode) {
        return Err("web_mode_draft_changed".into());
    }
    // This action registers only mode. The existing creator owns identities,
    // target scope and confirmation; no Root/C/capability is created here.
    Ok(())
}

pub(crate) fn read(db: &Connection, scan: &str) -> Result<(String, WebMode), String> {
    let (id, text): (String, String) = db
        .query_row(
            "SELECT draft_id,mode FROM native_web_mode_drafts WHERE scan_id=?1",
            [scan],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "web_mode_draft_missing")?;
    if uuid::Uuid::parse_str(&id).is_err() {
        return Err("web_mode_draft_invalid".into());
    }
    let mode = match text.as_str() {
        "single" => WebMode::Single,
        "multi" => WebMode::Multi,
        _ => return Err("web_mode_draft_invalid".into()),
    };
    Ok((id, mode))
}
