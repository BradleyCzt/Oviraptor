//! Newly born Native attempt supply snapshot. It is not a role capability or
//! executable lease. Existing Roots cannot be hot-equipped via this API.
use super::approval::{self, ToolAdministratorConfiguration};
use crate::agent_runtime::{
    multi_agent::budget::root::RootOwner,
    store::{self, NewlyInsertedNativeRoot},
    web_mode::{withdrawal_issuer::WithdrawalIssuer, ModeFact, VerifiedMode},
};
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};

pub(crate) const SCHEMA: &str = include_str!("frozen_plan/schema.sql");
type OriginalStoredPlanRow = (String, String, String, Vec<u8>, String, i64, String);
const DOMAIN: &str = "oviraptor.original-native-approved-tool-plan.v1";
pub(crate) struct FreshFrozenToolPlan<'tx, 'conn> {
    tx: &'tx Transaction<'conn>,
    configuration: &'tx ToolAdministratorConfiguration,
    root: String,
    plan_id: String,
    mode: VerifiedMode,
    fact: Value,
}
pub(crate) struct VerifiedFrozenToolPlan {
    fact: Value,
}
impl VerifiedFrozenToolPlan {
    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
}
/// Original Root-issued capture only. This is neither current administrator
/// approval nor an independently reverified external signature inventory.
pub(crate) struct VerifiedOriginalArchiveToolPlan {
    fact: Value,
}
impl VerifiedOriginalArchiveToolPlan {
    pub(crate) fn historical_fact(&self) -> &Value {
        &self.fact
    }
}
/// Current supply checks only. Broker must still separately prove actual role,
/// assignment, input scope, runtime isolation, budget, and cancellation.
pub(crate) struct VerifiedCurrentToolPlan {
    original: VerifiedFrozenToolPlan,
}
impl VerifiedCurrentToolPlan {
    pub(crate) fn fact(&self) -> &Value {
        self.original.fact()
    }
}

fn schema(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    for name in [
        "agent_frozen_tool_supply_plans",
        "frozen_tool_supply_no_update",
        "frozen_tool_supply_no_delete",
        "frozen_tool_supply_no_replace",
    ] {
        let actual: String = db
            .query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|_| "tool_supply_frozen_schema_changed")?;
        let trusted: String = expected
            .query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let shadow:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_temp_master WHERE name=?1 OR tbl_name='agent_frozen_tool_supply_plans')",[name],|r|r.get(0)).map_err(|e|e.to_string())?;
        if shadow || actual != trusted {
            return Err("tool_supply_frozen_schema_changed".into());
        }
    }
    let extra:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE type='trigger' AND tbl_name='agent_frozen_tool_supply_plans' AND name NOT IN ('frozen_tool_supply_no_update','frozen_tool_supply_no_delete','frozen_tool_supply_no_replace'))",[],|r|r.get(0)).map_err(|e|e.to_string())?;
    if extra {
        return Err("tool_supply_frozen_schema_changed".into());
    }
    Ok(())
}
fn scope_on(db: &Connection, root: &str) -> Result<(Value, VerifiedMode, Vec<String>), String> {
    RootOwner::load_original(db, root)?;
    let mode = crate::commands::original_root_capture_mode(db, root)?;
    let declaration = crate::agent_runtime::web_mode::root::read(db, root)?
        .ok_or("tool_supply_original_mode_missing")?;
    let web: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM main.sentinel_scans WHERE id=?1 AND scan_type='web')",
            [&mode.fact().scan_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if declaration.mode() != crate::agent_runtime::web_mode::WebMode::Multi || !web {
        return Err("tool_supply_original_mode_unsupported".into());
    }
    let (scan,attempt,target,plan_hash,text):(String,i64,String,String,String)=db.query_row("SELECT scan_id,attempt_number,target_url,plan_hash,plan_json FROM main.agent_runs WHERE id=?1 AND backend='native' AND role='coordinator' AND parent_run_id IS NULL AND assignment_id=''",[root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|"tool_supply_original_root_scope_changed")?;
    if scan != mode.fact().scan_id || attempt != mode.fact().attempt_number || attempt <= 0 {
        return Err("tool_supply_original_root_scope_changed".into());
    }
    let plan: Value =
        serde_json::from_str(&text).map_err(|_| "tool_supply_original_plan_invalid")?;
    let mut ids = Vec::new();
    if let Some(supply) = plan.get("toolSupply") {
        let object = supply
            .as_object()
            .ok_or("tool_supply_original_plan_invalid")?;
        if object.len() != 2 || object.get("schemaVersion") != Some(&json!(1)) {
            return Err("tool_supply_original_plan_invalid".into());
        }
        let values = object
            .get("approvalIds")
            .and_then(Value::as_array)
            .ok_or("tool_supply_original_plan_invalid")?;
        if values.len() > 16 {
            return Err("tool_supply_original_plan_invalid".into());
        }
        for value in values {
            let id = value.as_str().ok_or("tool_supply_original_plan_invalid")?;
            if !uuid::Uuid::parse_str(id).is_ok_and(|uuid| uuid.to_string() == id)
                || ids
                    .last()
                    .is_some_and(|previous: &String| previous.as_str() >= id)
            {
                return Err("tool_supply_original_plan_invalid".into());
            }
            ids.push(id.to_string());
        }
    }
    Ok((
        json!({"rootRunId":root,"scanId":scan,"attemptNumber":attempt,"targetUrl":target,"planHash":plan_hash,"nativePlanTextHash":store::stable_hash(&text),"nativePlan":text,"originalMode":mode.fact()}),
        mode,
        ids,
    ))
}
fn fact_on(
    db: &Connection,
    configuration: &ToolAdministratorConfiguration,
    root: &str,
    plan_id: &str,
) -> Result<(Value, VerifiedMode), String> {
    let (scope, mode, ids) = scope_on(db, root)?;
    if ids.is_empty() {
        return Err("tool_supply_original_plan_has_no_approved_tools".into());
    }
    let configuration_hash = approval::configuration_digest_on(configuration)?;
    let mut tools = Vec::new();
    let mut tool_ids = std::collections::BTreeSet::new();
    for id in ids {
        let original = approval::read_original_on(db, configuration, &id)?
            .ok_or("tool_supply_original_approval_missing")?;
        if !tool_ids.insert(original.manifest().id.clone()) {
            return Err("tool_supply_original_tool_release_conflict".into());
        }
        tools.push(json!({"approvalId":original.id(),"approvalHash":original.envelope_hash(),"manifest":original.manifest(),"runtimeAdapter":original.runtime_adapter(),"runtimeDigest":original.runtime_digest()}));
    }
    if approval::configuration_digest_on(configuration)? != configuration_hash {
        return Err("tool_administrator_configuration_changed".into());
    }
    Ok((
        json!({"schemaVersion":1,"purpose":"original_native_approved_tool_supply_plan","domain":DOMAIN,"planId":plan_id,"scope":scope,"configurationDigest":configuration_hash,"tools":tools,"executionAllowed":false}),
        mode,
    ))
}
pub(crate) fn capture_for_new_root<'tx, 'conn>(
    fresh: &'tx NewlyInsertedNativeRoot<'tx, 'conn>,
    configuration: &'tx ToolAdministratorConfiguration,
) -> Result<Option<FreshFrozenToolPlan<'tx, 'conn>>, String> {
    let tx = fresh.transaction();
    schema(tx)?;
    let (_, _, ids) = scope_on(tx, fresh.id())?;
    if ids.is_empty() {
        return Ok(None);
    }
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM main.agent_frozen_tool_supply_plans WHERE root_run_id=?1)",
            [fresh.id()],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists {
        return Err("tool_supply_fresh_root_proof_reused".into());
    }
    let plan_id = uuid::Uuid::new_v4().to_string();
    let (fact, mode) = fact_on(tx, configuration, fresh.id(), &plan_id)?;
    Ok(Some(FreshFrozenToolPlan {
        tx,
        configuration,
        root: fresh.id().into(),
        plan_id,
        mode,
        fact,
    }))
}
impl FreshFrozenToolPlan<'_, '_> {
    pub(crate) fn require_original_root(&self, root: &str) -> Result<(), String> {
        if self.root != root {
            return Err("tool_supply_fresh_root_binding_changed".into());
        }
        Ok(())
    }
    /// Only the dedicated issuer can sign this original fresh source. Complete
    /// source revalidation precedes encoding; saved JSON cannot construct it.
    pub(crate) fn encoded_for_original(&self, mode: &ModeFact) -> Result<Vec<u8>, String> {
        if self.tx.is_autocommit() || self.mode.fact() != mode {
            return Err("tool_supply_fresh_root_binding_changed".into());
        }
        let (fact, original_mode) =
            fact_on(self.tx, self.configuration, &self.root, &self.plan_id)?;
        if fact != self.fact || original_mode.fact() != self.mode.fact() {
            return Err("tool_supply_fresh_root_binding_changed".into());
        }
        serde_json::to_vec(&self.fact).map_err(|e| e.to_string())
    }
}
pub(crate) fn issue_new(
    fresh: FreshFrozenToolPlan<'_, '_>,
) -> Result<VerifiedFrozenToolPlan, String> {
    schema(fresh.tx)?;
    let encoded = fresh.encoded_for_original(fresh.mode.fact())?;
    let signature =
        WithdrawalIssuer::from_verified_birth_on(fresh.tx, &fresh.root, fresh.mode.clone())?
            .sign_fresh_frozen_tool_plan(&fresh)?;
    let text = String::from_utf8(encoded).map_err(|e| e.to_string())?;
    let changed=fresh.tx.execute("INSERT INTO main.agent_frozen_tool_supply_plans(plan_id,root_run_id,scan_id,attempt_number,native_plan_text_hash,fact_json,fact_hash,signature) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",rusqlite::params![fresh.plan_id,fresh.root,fresh.mode.fact().scan_id,fresh.mode.fact().attempt_number,fresh.fact["scope"]["nativePlanTextHash"].as_str().ok_or("tool_supply_fresh_root_binding_changed")?,text,store::stable_hash(&text),signature]).map_err(|e|e.to_string())?;
    if changed != 1 {
        return Err("tool_supply_frozen_insert_unconfirmed".into());
    }
    read_historical_on(fresh.tx, fresh.configuration, &fresh.root)?
        .ok_or_else(|| "tool_supply_frozen_insert_unconfirmed".into())
}
/// Verify only the original captured fact and its private Root seal.
/// Mutable administrator trust does not erase history or grant execution.
pub(crate) fn read_original_seal_on(
    db: &Connection,
    root: &str,
) -> Result<Option<VerifiedOriginalArchiveToolPlan>, String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return read_original_seal_on(&tx, root);
    }
    schema(db)?;
    let (scope, mode, ids) = scope_on(db, root)?;
    let row:Option<OriginalStoredPlanRow>=db.query_row("SELECT plan_id,fact_json,fact_hash,signature,scan_id,attempt_number,native_plan_text_hash FROM main.agent_frozen_tool_supply_plans WHERE root_run_id=?1",[root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional().map_err(|e|e.to_string())?;
    let Some((id, text, hash, signature, scan, attempt, plan_hash)) = row else {
        if ids.is_empty() {
            return Ok(None);
        }
        return Err("tool_supply_original_frozen_plan_missing".into());
    };
    if !uuid::Uuid::parse_str(&id).is_ok_and(|uuid| uuid.to_string() == id)
        || scan != mode.fact().scan_id
        || attempt != mode.fact().attempt_number
        || Some(plan_hash.as_str()) != scope["nativePlanTextHash"].as_str()
        || store::stable_hash(&text) != hash
    {
        return Err("tool_supply_frozen_plan_changed".into());
    }
    let fact: Value = serde_json::from_str(&text).map_err(|_| "tool_supply_frozen_plan_changed")?;
    let canonical = serde_json::to_string(&fact).map_err(|e| e.to_string())?;
    if canonical != text {
        return Err("tool_supply_frozen_plan_changed".into());
    }
    WithdrawalIssuer::from_verified_birth_on(db, root, mode.clone())?
        .verify_frozen_tool_plan(text.as_bytes(), &signature)?;
    let fields = fact.as_object().ok_or("tool_supply_frozen_plan_changed")?;
    let tools = fact["tools"]
        .as_array()
        .ok_or("tool_supply_frozen_plan_changed")?;
    if fields.len() != 8
        || fact["schemaVersion"] != json!(1)
        || fact["purpose"] != "original_native_approved_tool_supply_plan"
        || fact["domain"] != DOMAIN
        || fact["planId"] != id
        || fact["scope"] != scope
        || fact["executionAllowed"] != false
        || fact["configurationDigest"]
            .as_str()
            .is_none_or(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
        || tools.len() != ids.len()
        || ids.is_empty()
    {
        return Err("tool_supply_frozen_plan_changed".into());
    }
    for (tool, approval_id) in tools.iter().zip(ids.iter()) {
        if tool.as_object().is_none_or(|fields| fields.len() != 5)
            || tool["approvalId"] != *approval_id
            || tool["approvalHash"]
                .as_str()
                .is_none_or(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
            || tool.get("manifest").is_none()
            || tool.get("runtimeAdapter").is_none()
            || tool.get("runtimeDigest").is_none()
        {
            return Err("tool_supply_frozen_plan_changed".into());
        }
    }
    Ok(Some(VerifiedOriginalArchiveToolPlan { fact }))
}

pub(crate) fn read_historical_on(
    db: &Connection,
    configuration: &ToolAdministratorConfiguration,
    root: &str,
) -> Result<Option<VerifiedFrozenToolPlan>, String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return read_historical_on(&tx, configuration, root);
    }
    let Some(seal) = read_original_seal_on(db, root)? else {
        return Ok(None);
    };
    let fact = seal.fact;
    let id = fact["planId"]
        .as_str()
        .ok_or("tool_supply_frozen_plan_changed")?;
    if approval::configuration_digest_on(configuration)
        .map_err(|e| format!("tool_supply_historical_trust_unavailable:{e}"))?
        != fact["configurationDigest"]
            .as_str()
            .ok_or("tool_supply_frozen_plan_changed")?
    {
        return Err(
            "tool_supply_historical_trust_unavailable:tool_administrator_configuration_changed"
                .into(),
        );
    }
    let (original, _) = fact_on(db, configuration, root, id)
        .map_err(|e| format!("tool_supply_historical_trust_unavailable:{e}"))?;
    if original != fact {
        return Err("tool_supply_frozen_plan_changed".into());
    }
    Ok(Some(VerifiedFrozenToolPlan { fact }))
}
pub(crate) fn require_current_on(
    tx: &Transaction<'_>,
    configuration: &ToolAdministratorConfiguration,
    root: &str,
) -> Result<VerifiedCurrentToolPlan, String> {
    RootOwner::load_original(tx, root)?.require_live(tx)?;
    let original =
        read_historical_on(tx, configuration, root)?.ok_or("tool_supply_no_frozen_tools")?;
    Ok(VerifiedCurrentToolPlan { original })
}

#[cfg(test)]
#[path = "frozen_plan/tests.rs"]
pub(crate) mod tests;
