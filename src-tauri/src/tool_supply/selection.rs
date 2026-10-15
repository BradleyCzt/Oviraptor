//! Declared plan selection and signed-inventory preparation. Scope is still
//! caller-declared until the actual new-scan issuer supplies its private proof.
//! There is deliberately no persistence/signing factory or runtime setter here.
use super::approval::{self, ToolAdministratorConfiguration};
use crate::agent_runtime::store;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub(crate) const SCHEMA: &str = include_str!("selection/schema.sql");
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FrozenToolSelection {
    schema_version: u32,
    approval_ids: Vec<String>,
}
fn exact_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
impl FrozenToolSelection {
    /// Absent and explicit empty selections carry no supply authority. Empty
    /// normalizes to None so old Native plan bytes can remain unchanged.
    pub(crate) fn from_plan_value(value: Option<&Value>) -> Result<Option<Self>, String> {
        Ok(Self::parse_preserving_empty(value)?
            .filter(|selection| !selection.approval_ids.is_empty()))
    }
    /// Persistence distinguishes an absent declaration from an explicitly
    /// frozen empty list. Both grant no tools, but only missing omits the field.
    pub(crate) fn parse_preserving_empty(value: Option<&Value>) -> Result<Option<Self>, String> {
        let Some(value) = value else {
            return Ok(None);
        };
        let selection: Self =
            serde_json::from_value(value.clone()).map_err(|_| "tool_selection_declared_invalid")?;
        if selection.schema_version != 1
            || selection.approval_ids.len() > 16
            || selection.approval_ids.iter().any(|id| !exact_uuid(id))
            || selection
                .approval_ids
                .windows(2)
                .any(|ids| ids[0] >= ids[1])
        {
            return Err("tool_selection_declared_invalid".into());
        }
        Ok(Some(selection))
    }
    pub(crate) fn as_plan_value(&self) -> Value {
        json!({"schemaVersion":self.schema_version,"approvalIds":self.approval_ids})
    }
    pub(crate) fn approval_ids(&self) -> &[String] {
        &self.approval_ids
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DeclaredScope {
    scan_id: String,
    attempt_number: i64,
    target_urls: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeclaredSelectionRequest {
    schema_version: u32,
    request_id: String,
    scope: DeclaredScope,
    selection: FrozenToolSelection,
}
impl DeclaredSelectionRequest {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let request: Self =
            serde_json::from_value(value.clone()).map_err(|_| "tool_selection_request_invalid")?;
        if request.schema_version != 1
            || !exact_uuid(&request.request_id)
            || request.scope.scan_id.is_empty()
            || request.scope.scan_id.len() > 128
            || request.scope.attempt_number <= 0
            || request.scope.target_urls.is_empty()
            || request.scope.target_urls.len() > 1000
            || request
                .scope
                .target_urls
                .windows(2)
                .any(|urls| urls[0] >= urls[1])
        {
            return Err("tool_selection_request_invalid".into());
        }
        for target in &request.scope.target_urls {
            if target.len() > 8192 {
                return Err("tool_selection_request_invalid".into());
            }
            let parsed =
                reqwest::Url::parse(target).map_err(|_| "tool_selection_request_invalid")?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.fragment().is_some()
            {
                return Err("tool_selection_request_invalid".into());
            }
        }
        // A task-external selection request is explicit and nonempty; no-tools
        // callers use the old start API rather than persisting an empty request.
        if FrozenToolSelection::from_plan_value(Some(&request.selection.as_plan_value()))?.is_none()
        {
            return Err("tool_selection_request_empty".into());
        }
        Ok(request)
    }
    pub(crate) fn selection(&self) -> &FrozenToolSelection {
        &self.selection
    }
    pub(crate) fn as_value(&self) -> Value {
        json!({"schemaVersion":self.schema_version,"requestId":self.request_id,"scope":{"scanId":self.scope.scan_id,"attemptNumber":self.scope.attempt_number,"targetUrls":self.scope.target_urls},"selection":self.selection.as_plan_value()})
    }
    pub(crate) fn hash(&self) -> String {
        store::stable_hash(&self.as_value().to_string())
    }
}
/// Verified supply inventory, not verified scan birth or permission. No public
/// constructor can manufacture this object from a DTO or caller authority bool.
pub(crate) struct PreparedToolSelection {
    request: DeclaredSelectionRequest,
    fact: Value,
}
impl PreparedToolSelection {
    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
    pub(crate) fn declared_request(&self) -> &DeclaredSelectionRequest {
        &self.request
    }
    pub(crate) fn require_current_on(
        &self,
        db: &Connection,
        configuration: &ToolAdministratorConfiguration,
        request: &DeclaredSelectionRequest,
    ) -> Result<(), String> {
        if request != &self.request {
            return Err("tool_selection_request_binding_changed".into());
        }
        let original = prepare_requested_on(db, configuration, request)?;
        if original.fact != self.fact {
            return Err("tool_selection_original_inventory_changed".into());
        }
        Ok(())
    }
}
pub(crate) fn prepare_requested_on(
    db: &Connection,
    configuration: &ToolAdministratorConfiguration,
    request: &DeclaredSelectionRequest,
) -> Result<PreparedToolSelection, String> {
    // Reparse even private typed declarations: serde Deserialize is declaration
    // parsing only, never the signed-inventory or scan-creation authority.
    let request = DeclaredSelectionRequest::parse(&request.as_value())?;
    let digest = approval::configuration_digest_on(configuration)?;
    let mut inventory = Vec::new();
    let mut tool_ids = std::collections::BTreeSet::new();
    for id in request.selection.approval_ids() {
        let original = approval::read_original_on(db, configuration, id)?
            .ok_or("tool_selection_approval_missing")?;
        if !tool_ids.insert(original.manifest().id.clone()) {
            return Err("tool_selection_release_conflict".into());
        }
        inventory.push(json!({"approvalId":original.id(),"approvalHash":original.envelope_hash(),"manifest":original.manifest(),"runtimeAdapter":original.runtime_adapter(),"runtimeDigest":original.runtime_digest()}));
    }
    if approval::configuration_digest_on(configuration)? != digest {
        return Err("tool_administrator_configuration_changed".into());
    }
    let fact = json!({"schemaVersion":1,"purpose":"prepared_signed_tool_selection_inventory","request":request.as_value(),"requestHash":request.hash(),"configurationDigest":digest,"inventory":inventory,"scopeVerified":false,"executionAllowed":false});
    Ok(PreparedToolSelection { request, fact })
}

#[cfg(all(test, unix))]
#[path = "selection/tests.rs"]
mod tests;

/// A pending request is not signed. Only its later presence in the actual
/// original privately signed Mode policy can provide historical provenance.
pub(crate) struct PendingDraftToolSelection {
    fact: Value,
}
impl PendingDraftToolSelection {
    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
}
fn quoted(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
fn request_schema(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    for name in [
        "native_tool_selection_requests",
        "tool_selection_no_update",
        "tool_selection_no_delete",
        "tool_selection_no_replace",
    ] {
        let actual: String = db
            .query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|_| "tool_selection_schema_changed")?;
        let trusted: String = expected
            .query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if actual != trusted {
            return Err("tool_selection_schema_changed".into());
        }
    }
    let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name IN ('native_tool_selection_requests','sentinel_scan_contexts','sentinel_targets','sentinel_scans','native_web_mode_drafts') OR tbl_name IN ('native_tool_selection_requests','sentinel_scan_contexts'))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    let extra:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE type='trigger' AND (tbl_name='sentinel_scan_contexts' OR (tbl_name='native_tool_selection_requests' AND name NOT IN ('tool_selection_no_update','tool_selection_no_delete','tool_selection_no_replace'))))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if shadow || extra {
        return Err("tool_selection_schema_changed".into());
    }
    Ok(())
}
fn outside_snapshot(db: &Connection, scan: &str, request: &str) -> Result<Value, String> {
    let mut query = db
        .prepare("SELECT name FROM main.sqlite_master WHERE type='table' ORDER BY name")
        .map_err(|e| e.to_string())?;
    let tables = query
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut snapshots = Vec::new();
    for table in tables {
        let without_rowid: bool = db
            .query_row(
                "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                [&table],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let order = if without_rowid {
            let mut info = db
                .prepare(&format!("PRAGMA main.table_info({})", quoted(&table)))
                .map_err(|e| e.to_string())?;
            let mut pk = info
                .query_map([], |r| Ok((r.get::<_, i64>(5)?, r.get::<_, String>(1)?)))
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?;
            pk.retain(|(position, _)| *position > 0);
            pk.sort_by_key(|(position, _)| *position);
            if pk.is_empty() {
                return Err("tool_selection_snapshot_identity_unknown".into());
            }
            pk.iter()
                .map(|(_, column)| quoted(column))
                .collect::<Vec<_>>()
                .join(",")
        } else {
            "rowid".into()
        };
        let predicate = match table.as_str() {
            "sentinel_scan_contexts" => " WHERE scan_id IS NOT ?1",
            "native_tool_selection_requests" => " WHERE request_id IS NOT ?1",
            _ => "",
        };
        let sql = format!(
            "SELECT {}* FROM main.{}{} ORDER BY {}",
            if without_rowid { "" } else { "rowid," },
            quoted(&table),
            predicate,
            order
        );
        let mut statement = db.prepare(&sql).map_err(|e| e.to_string())?;
        let count = statement.column_count();
        let mut rows = if table == "sentinel_scan_contexts" {
            statement.query([scan])
        } else if table == "native_tool_selection_requests" {
            statement.query([request])
        } else {
            statement.query([])
        }
        .map_err(|e| e.to_string())?;
        let mut values = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let mut cells = Vec::new();
            for column in 0..count {
                use rusqlite::types::ValueRef;
                let cell = match row.get_ref(column).map_err(|e| e.to_string())? {
                    ValueRef::Null => json!(["null"]),
                    ValueRef::Integer(v) => json!(["integer", v]),
                    ValueRef::Real(v) => json!(["real", v.to_bits().to_string()]),
                    ValueRef::Text(v) => json!([
                        "text",
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, v)
                    ]),
                    ValueRef::Blob(v) => json!([
                        "blob",
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, v)
                    ]),
                };
                cells.push(cell);
            }
            values.push(cells);
        }
        snapshots.push(json!({"table":table,"rows":values}));
    }
    Ok(json!(snapshots))
}
/// Only an actual creation-window proof can issue the pending snapshot. It
/// preserves the caller's existing authorizer rather than installing/removing
/// a borrowed hook. All possible trigger writes are rejected before stage SQL;
/// a private savepoint rolls back either of the two writes on any mismatch.
pub(crate) fn stage_for_new_draft(
    fresh: &crate::agent_runtime::web_mode::draft::FreshModeDraft<'_, '_>,
    configuration: &ToolAdministratorConfiguration,
    request: Option<&DeclaredSelectionRequest>,
) -> Result<Option<PendingDraftToolSelection>, String> {
    let Some(request) = request else {
        return Ok(None);
    };
    if request.selection.approval_ids.is_empty() {
        return Ok(None);
    }
    let db = fresh.transaction();
    request_schema(db)?;
    let (draft, mode) = crate::agent_runtime::web_mode::draft::read(db, fresh.scan_id())?;
    if draft != fresh.draft_id()
        || mode != crate::agent_runtime::web_mode::WebMode::Multi
        || request.scope.scan_id != fresh.scan_id()
        || request.scope.attempt_number != 1
    {
        return Err("tool_selection_fresh_draft_scope_changed".into());
    }
    let new:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.sentinel_scans WHERE id=?1 AND scan_type='web' AND status='draft' AND attempt_count=0 AND task_path='') AND NOT EXISTS(SELECT 1 FROM main.sentinel_scan_attempts WHERE scan_id=?1) AND NOT EXISTS(SELECT 1 FROM main.agent_runs WHERE scan_id=?1) AND NOT EXISTS(SELECT 1 FROM main.native_tool_selection_requests WHERE scan_id=?1)",[fresh.scan_id()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !new {
        return Err("tool_selection_requires_actual_new_draft".into());
    }
    let mut targets_query = db
        .prepare("SELECT url FROM main.sentinel_targets WHERE scan_id=?1 ORDER BY url")
        .map_err(|e| e.to_string())?;
    let targets = targets_query
        .query_map([fresh.scan_id()], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if targets != request.scope.target_urls {
        return Err("tool_selection_fresh_draft_targets_changed".into());
    }
    let prepared = prepare_requested_on(db, configuration, request)?;
    let old_policy: String = db
        .query_row(
            "SELECT policy_json FROM main.sentinel_scan_contexts WHERE scan_id=?1",
            [fresh.scan_id()],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut policy: Value =
        serde_json::from_str(&old_policy).map_err(|_| "tool_selection_original_policy_invalid")?;
    if serde_json::to_string(&policy).map_err(|e| e.to_string())? != old_policy
        || policy.get("toolSupplySelection").is_some()
    {
        return Err("tool_selection_original_policy_changed".into());
    }
    let fact = json!({"schemaVersion":1,"purpose":"original_new_draft_tool_selection_pending","draftId":fresh.draft_id(),"scanId":fresh.scan_id(),"prepared":prepared.fact(),"signatureStage":"pending_original_mode_policy_signature","scopeVerified":true,"executionAllowed":false});
    policy
        .as_object_mut()
        .ok_or("tool_selection_original_policy_invalid")?
        .insert("toolSupplySelection".into(), fact.clone());
    let new_policy = serde_json::to_string(&policy).map_err(|e| e.to_string())?;
    let snapshot_text = serde_json::to_string(&fact).map_err(|e| e.to_string())?;
    let outside = outside_snapshot(db, fresh.scan_id(), &request.request_id)?;
    db.execute_batch("SAVEPOINT original_tool_selection_stage")
        .map_err(|e| e.to_string())?;
    let result: Result<PendingDraftToolSelection, String> = (|| {
        let inserted=db.execute("INSERT INTO main.native_tool_selection_requests(request_id,scan_id,attempt_number,scope_json,selection_json,inventory_json,configuration_digest,request_hash,draft_id,stage,snapshot_json) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,'draft_pending_original_mode_signature',?9)",rusqlite::params![request.request_id,fresh.scan_id(),serde_json::to_string(&request.scope).map_err(|e|e.to_string())?,request.selection.as_plan_value().to_string(),prepared.fact()["inventory"].to_string(),prepared.fact()["configurationDigest"].as_str().ok_or("tool_selection_original_inventory_changed")?,request.hash(),fresh.draft_id(),fact.to_string()]).map_err(|e|e.to_string())?;
        let updated=db.execute("UPDATE main.sentinel_scan_contexts SET policy_json=?1 WHERE scan_id=?2 AND policy_json=?3",rusqlite::params![new_policy,fresh.scan_id(),old_policy]).map_err(|e|e.to_string())?;
        if inserted != 1 || updated != 1 {
            return Err("tool_selection_stage_insert_unconfirmed".into());
        }
        prepared.require_current_on(db, configuration, request)?;
        request_schema(db)?;
        let saved: String = db
            .query_row(
                "SELECT policy_json FROM main.sentinel_scan_contexts WHERE scan_id=?1",
                [fresh.scan_id()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let snapshot: String = db
            .query_row(
                "SELECT snapshot_json FROM main.native_tool_selection_requests WHERE request_id=?1",
                [&request.request_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if saved != new_policy
            || snapshot != snapshot_text
            || outside_snapshot(db, fresh.scan_id(), &request.request_id)? != outside
        {
            return Err("tool_selection_stage_source_changed".into());
        }
        Ok(PendingDraftToolSelection { fact: fact.clone() })
    })();
    match result {
        Ok(pending) => {
            db.execute_batch("RELEASE original_tool_selection_stage")
                .map_err(|e| e.to_string())?;
            Ok(Some(pending))
        }
        Err(error) => {
            db.execute_batch(
                "ROLLBACK TO original_tool_selection_stage; RELEASE original_tool_selection_stage",
            )
            .map_err(|e| format!("{error};tool_selection_stage_rollback:{e}"))?;
            Err(error)
        }
    }
}

/// Fields stay private. Main must supply a leaf-owned factory that obtains the
/// actual original startup Mode proof and full task effective policy from its
/// private key/marker/task reader. There is no from-JSON or caller-key factory.
pub(crate) struct OriginalSignedToolSelectionPolicy {
    mode: crate::agent_runtime::web_mode::VerifiedMode,
    canonical_policy: String,
}
impl OriginalSignedToolSelectionPolicy {
    pub(crate) fn capture_original_on(
        db: &Connection, scan: &str, attempt: i64,
    ) -> Result<Self, String> {
        let source = crate::scanning::scope_contract::capture_original_tool_selection_policy_on(db, scan, attempt)?;
        Ok(Self {
            mode: source.mode().clone(),
            canonical_policy: source.canonical_policy().into(),
        })
    }
}
pub(crate) struct VerifiedOriginalToolSelection {
    request: DeclaredSelectionRequest,
    fact: Value,
}
impl VerifiedOriginalToolSelection {
    pub(crate) fn selection(&self) -> &FrozenToolSelection { &self.request.selection }
    pub(crate) fn request(&self) -> &DeclaredSelectionRequest {
        &self.request
    }
    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
}
/// An unsigned pending row alone cannot reach this reader. Until its trusted
/// original startup factory is wired, no caller can construct the source type.
pub(crate) fn verify_signed_request_on(
    db: &Connection,
    source: &OriginalSignedToolSelectionPolicy,
    request_id: &str,
) -> Result<VerifiedOriginalToolSelection, String> {
    request_schema(db)?;
    if !exact_uuid(request_id) {
        return Err("tool_selection_request_invalid".into());
    }
    let policy: Value = serde_json::from_str(&source.canonical_policy)
        .map_err(|_| "tool_selection_original_signed_policy_changed")?;
    if serde_json::to_string(&policy).map_err(|e| e.to_string())? != source.canonical_policy
        || store::stable_hash(&source.canonical_policy) != source.mode.fact().policy_hash
        || policy["orchestration"]["mode"] != "multi"
    {
        return Err("tool_selection_original_signed_policy_changed".into());
    }
    type PendingRow = (
        String,
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    );
    let (scan,attempt,scope,selection,inventory,digest,hash,draft,stage,snapshot):PendingRow=db.query_row("SELECT scan_id,attempt_number,scope_json,selection_json,inventory_json,configuration_digest,request_hash,draft_id,stage,snapshot_json FROM main.native_tool_selection_requests WHERE request_id=?1",[request_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).map_err(|e|e.to_string())?;
    let declared = DeclaredSelectionRequest::parse(
        &json!({"schemaVersion":1,"requestId":request_id,"scope":serde_json::from_str::<Value>(&scope).map_err(|e|e.to_string())?,"selection":serde_json::from_str::<Value>(&selection).map_err(|e|e.to_string())?}),
    )?;
    let original: Value =
        serde_json::from_str(&snapshot).map_err(|_| "tool_selection_original_pending_changed")?;
    let prepared = &original["prepared"];
    let canonical_scope = serde_json::to_string(&declared.scope).map_err(|e| e.to_string())?;
    let canonical_selection =
        serde_json::to_string(&declared.selection.as_plan_value()).map_err(|e| e.to_string())?;
    let canonical_snapshot = serde_json::to_string(&original).map_err(|e| e.to_string())?;
    let canonical_inventory =
        serde_json::to_string(&prepared["inventory"]).map_err(|e| e.to_string())?;
    if scan != source.mode.fact().scan_id
        || attempt != 1
        || source.mode.fact().attempt_number < 1
        || declared.scope.scan_id != scan
        || declared.scope.attempt_number != attempt
        || declared.scope.target_urls != source.mode.fact().targets
        || stage != "draft_pending_original_mode_signature"
        || crate::agent_runtime::web_mode::draft::read(db, &scan)?
            != (
                draft.clone(),
                crate::agent_runtime::web_mode::WebMode::Multi,
            )
        || declared.hash() != hash
        || canonical_scope != scope
        || canonical_selection != selection
        || canonical_snapshot != snapshot
        || original["draftId"] != draft
        || original["scanId"] != scan
        || prepared["request"] != declared.as_value()
        || prepared["requestHash"] != hash
        || prepared["configurationDigest"] != digest
        || canonical_inventory != inventory
        || policy.get("toolSupplySelection") != Some(&original)
    {
        return Err("tool_selection_original_pending_changed".into());
    }
    // The original private Mode signature binds this exact policy hash. The
    // mutable current administrator inventory is not consulted for history.
    let fact = json!({"schemaVersion":1,"purpose":"original_signed_new_draft_tool_selection","originalMode":source.mode.fact(),"policyHash":source.mode.fact().policy_hash,"snapshot":original,"executionAllowed":false});
    Ok(VerifiedOriginalToolSelection {
        request: declared,
        fact,
    })
}
pub(crate) fn require_current_signed_on(
    db: &Connection,
    source: &OriginalSignedToolSelectionPolicy,
    configuration: &ToolAdministratorConfiguration,
    request_id: &str,
) -> Result<VerifiedOriginalToolSelection, String> {
    let original = verify_signed_request_on(db, source, request_id)?;
    let current = prepare_requested_on(db, configuration, &original.request)?;
    if current.fact() != &original.fact["snapshot"]["prepared"] {
        return Err("tool_selection_original_inventory_changed".into());
    }
    Ok(original)
}
