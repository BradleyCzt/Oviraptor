//! Immutable independent new-action provenance; no inherited execution rights.
use super::draft::FreshModeDraft;
use crate::agent_runtime::store;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    params, Connection, OptionalExtension,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct NewWebAction {
    pub schema_version: u8,
    pub kind: String,
    pub request_id: String,
    pub source_scan_id: String,
    pub source_hash: String,
    pub request_hash: String,
    pub targets: Vec<String>,
}
impl NewWebAction {
    fn verify(&self, db: &Connection, scan: &str) -> Result<(), String> {
        if self.schema_version != 1
            || uuid::Uuid::parse_str(&self.request_id).is_err()
            || self.source_scan_id == scan
            || self.source_scan_id.is_empty()
            || [&self.source_hash, &self.request_hash]
                .iter()
                .any(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
            || self.targets.is_empty()
            || self.targets.windows(2).any(|v| v[0] >= v[1])
        {
            return Err("web_mode_action_invalid".into());
        }
        // Public action hashes cannot reclassify a signed recovery relation as
        // another independent action or combine two incompatible provenances.
        let recovery: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.native_protection_recovery_drafts WHERE scan_id=?1)",
            [scan], |r| r.get(0),
        ).map_err(|e| e.to_string())?;
        if recovery != (self.kind == "protection_recovery") {
            return Err("web_mode_recovery_action_kind_changed".into());
        }
        if recovery && db.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.agent_gap_followups WHERE scan_id=?1) OR EXISTS(SELECT 1 FROM main.native_web_closure_handoffs WHERE scan_id=?1)",
            [scan], |r| r.get::<_, bool>(0),
        ).map_err(|e| e.to_string())? {
            return Err("web_mode_recovery_action_conflicting_relation".into());
        }
        let actual: Vec<String> = {
            let mut q = db
                .prepare("SELECT url FROM sentinel_targets WHERE scan_id=?1 ORDER BY url")
                .map_err(|e| e.to_string())?;
            let targets = q
                .query_map([scan], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<_>>()
                .map_err(|e| e.to_string())?;
            targets
        };
        let tuple:(String,String,String,String)=match self.kind.as_str() {
            "gap_followup"=>db.query_row("SELECT request_id,source_scan_id,source_hash,request_hash FROM agent_gap_followups WHERE scan_id=?1",[scan],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))),
            "closure_handoff"=>db.query_row("SELECT request_id,source_scan_id,json_extract(preview_json,'$.sourceHash'),input_hash FROM native_web_closure_handoffs WHERE scan_id=?1",[scan],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))),
            "protection_recovery"=> {
                crate::commands::native_protection_recovery_draft_action_on(db,scan)?;
                db.query_row("SELECT request_id,source_scan_id,proposal_hash,approval_id FROM main.native_protection_recovery_drafts WHERE scan_id=?1",[scan],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
            },
            _=>return Err("web_mode_action_kind_invalid".into()),
        }.map_err(|_|"web_mode_action_original_relation_missing")?;
        if tuple
            != (
                self.request_id.clone(),
                self.source_scan_id.clone(),
                self.source_hash.clone(),
                self.request_hash.clone(),
            )
            || actual != self.targets
        {
            return Err("web_mode_action_original_scope_changed".into());
        }
        Ok(())
    }
    fn hash(&self) -> Result<String, String> {
        Ok(store::stable_hash(
            &serde_json::to_string(self).map_err(|e| e.to_string())?,
        ))
    }
}
pub(crate) fn freeze_new(fresh: FreshModeDraft<'_, '_>, fact: &NewWebAction) -> Result<(), String> {
    let db = fresh.transaction();
    fact.verify(db, fresh.scan_id())?;
    db.authorizer(Some(|c: AuthContext<'_>| match c.action {
        AuthAction::Insert {
            table_name: "native_web_mode_actions",
        } if c.database_name == Some("main") && c.accessor.is_none() => Authorization::Allow,
        // Recovery action verification checks original physical DB identity.
        AuthAction::Pragma {
            pragma_name: "database_list",
            ..
        } => Authorization::Allow,
        AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Function { .. }
        | AuthAction::Transaction { .. }
        | AuthAction::Savepoint { .. }
        | AuthAction::Recursive => Authorization::Allow,
        _ => Authorization::Deny,
    }))
    .map_err(|e| e.to_string())?;
    let result: Result<(), String> = (|| {
        let text = serde_json::to_string(fact).map_err(|e| e.to_string())?;
        let hash = fact.hash()?;
        let n=db.execute("INSERT INTO native_web_mode_actions(scan_id,draft_id,action_json,action_hash) VALUES(?1,?2,?3,?4)",params![fresh.scan_id(),fresh.draft_id(),text,hash]).map_err(|e|e.to_string())?;
        if n != 1 || read_hash(db, fresh.scan_id())? != Some(hash) {
            return Err("web_mode_action_not_saved".into());
        }
        Ok(())
    })();
    let clear = db
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|e| e.to_string());
    result?;
    clear
}
pub(crate) fn read_hash(db: &Connection, scan: &str) -> Result<Option<String>, String> {
    let row: Option<(String, String, String)> = db
        .query_row(
            "SELECT draft_id,action_json,action_hash FROM native_web_mode_actions WHERE scan_id=?1",
            [scan],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((draft, text, hash)) = row else {
        let independent:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_gap_followups WHERE scan_id=?1) OR EXISTS(SELECT 1 FROM native_web_closure_handoffs WHERE scan_id=?1) OR EXISTS(SELECT 1 FROM main.native_protection_recovery_drafts WHERE scan_id=?1)",[scan],|r|r.get(0)).map_err(|e|e.to_string())?;
        return if independent {
            Err("web_mode_independent_action_missing".into())
        } else {
            Ok(None)
        };
    };
    let fact: NewWebAction = serde_json::from_str(&text).map_err(|_| "web_mode_action_corrupt")?;
    if serde_json::to_string(&fact).map_err(|e| e.to_string())? != text
        || fact.hash()? != hash
        || super::draft::read(db, scan)?.0 != draft
    {
        return Err("web_mode_action_binding_changed".into());
    }
    fact.verify(db, scan)?;
    Ok(Some(hash))
}
