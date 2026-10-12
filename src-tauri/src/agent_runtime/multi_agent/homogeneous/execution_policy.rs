//! Explicit Phase 2 binding, included in the original signed Native plan.
//! A mutable UI flag or a readonly Phase 1 lease cannot enable execution.
use crate::agent_runtime::{
    multi_agent::{budget::root::RootOwner, lease},
    store::stable_hash,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
pub(crate) const MODE: &str = "model_broker_v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Binding {
    schema_version: u32,
    scan_id: String,
    attempt_number: i64,
    native_task_plan_sha256: String,
    worker_execution: String,
    allowed_parallel: u32,
    target_parallel: u32,
    review_parallel: u32,
    credential_visibility: String,
}
impl Binding {
    pub(crate) fn capture(
        db: &Connection,
        scan: &str,
        attempt: i64,
    ) -> Result<Option<Self>, String> {
        let row:Option<(String,String)>=db.query_row("SELECT plan_json,plan_hash FROM native_scan_plans WHERE scan_id=?1 AND attempt_number=?2",params![scan,attempt],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
        let Some((text, hash)) = row else {
            return Ok(None);
        };
        let plan: Value =
            serde_json::from_str(&text).map_err(|_| "homogeneous_execution_plan_invalid")?;
        if stable_hash(&text) != hash {
            return Err("homogeneous_execution_plan_hash_changed".into());
        }
        if plan["workerExecution"] != MODE {
            return Ok(None);
        }
        if plan["schemaVersion"] != 1
            || plan["scanId"] != scan
            || plan["attemptNumber"] != attempt
            || plan["scheduling_model"] != "homogeneous_workers"
            || plan["allowedParallel"] != 3
            || plan["lane"] != "read_only_analysis"
            || plan["targetParallel"] != 1
            || plan["reviewParallel"] != 1
            || plan["credentialVisibility"] != "plaintext"
        {
            return Err("homogeneous_execution_plan_invalid".into());
        }
        Ok(Some(Self {
            schema_version: 1,
            scan_id: scan.into(),
            attempt_number: attempt,
            native_task_plan_sha256: hash,
            worker_execution: MODE.into(),
            allowed_parallel: 3,
            target_parallel: 1,
            review_parallel: 1,
            credential_visibility: "plaintext".into(),
        }))
    }
    pub(crate) fn parse(value: Option<&Value>) -> Result<Option<Self>, String> {
        let Some(value) = value else { return Ok(None) };
        let parsed: Self = serde_json::from_value(value.clone())
            .map_err(|_| "homogeneous_execution_binding_invalid")?;
        if parsed.schema_version != 1
            || parsed.worker_execution != MODE
            || parsed.allowed_parallel != 3
            || parsed.target_parallel != 1
            || parsed.review_parallel != 1
            || parsed.credential_visibility != "plaintext"
            || parsed.attempt_number <= 0
            || !uuid::Uuid::parse_str(&parsed.scan_id)
                .is_ok_and(|u| u.to_string() == parsed.scan_id)
            || parsed.native_task_plan_sha256.len() != 64
            || !parsed
                .native_task_plan_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("homogeneous_execution_binding_invalid".into());
        }
        Ok(Some(parsed))
    }
    pub(crate) fn as_value(&self) -> Value {
        serde_json::to_value(self).expect("fixed execution binding encoding")
    }
}

pub(crate) fn require_on(
    db: &Connection,
    actor: &lease::CoordinatorLease,
) -> Result<Binding, String> {
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    RootOwner::load_original(db, &actor.root_run_id)?.require_live(db)?;
    read_original_on(db, &actor.root_run_id)?
        .ok_or_else(|| "homogeneous_execution_not_frozen_at_birth".into())
}

/// Inside an authorization probe's own claim/response revalidation the single
/// claimed target request is indeterminate until reconciled; the budget gate
/// tolerates exactly that entry. Lease, executable-root and original frozen
/// binding checks stay identical to require_on.
pub(crate) fn require_on_during_own_probe(
    db: &Connection,
    actor: &lease::CoordinatorLease,
) -> Result<Binding, String> {
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    RootOwner::load_original(db, &actor.root_run_id)?.require_executable(db)?;
    crate::agent_runtime::multi_agent::budget::admission::require_determinate_tolerating_own_probe_claim(
        db,
        &actor.root_run_id,
    )?;
    read_original_on(db, &actor.root_run_id)?
        .ok_or_else(|| "homogeneous_execution_not_frozen_at_birth".into())
}

/// Historical binding verification never grants a live capability.
pub(crate) fn read_original_on(db: &Connection, root: &str) -> Result<Option<Binding>, String> {
    let row:Option<(String,String,i64)>=db.query_row("SELECT plan_json,scan_id,attempt_number FROM agent_runs WHERE id=?1 AND root_run_id=id AND backend='native' AND role='coordinator'",[root],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e|e.to_string())?;
    let Some((original, scan, attempt)) = row else {
        return Ok(None);
    };
    let original: Value = serde_json::from_str(&original)
        .map_err(|_| "homogeneous_execution_original_plan_invalid")?;
    let Some(binding) = Binding::parse(original.get("homogeneousExecution"))? else {
        return Ok(None);
    };
    RootOwner::load_original(db, root)?;
    crate::agent_runtime::web_mode::root::read(db, root)?
        .ok_or("homogeneous_execution_original_mode_missing")?;
    let current =
        Binding::capture(db, &scan, attempt)?.ok_or("homogeneous_execution_plan_missing")?;
    if binding != current {
        return Err("homogeneous_execution_original_binding_changed".into());
    }
    Ok(Some(binding))
}
