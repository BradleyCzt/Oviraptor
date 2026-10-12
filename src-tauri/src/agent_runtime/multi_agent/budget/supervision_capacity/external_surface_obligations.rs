//! Original v12 ExternalSurface output assessment obligations. No writes/grants.
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{attempts, contract_owner, lease::CoordinatorLease, scheduler::ScheduledChild},
    web_mode,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

pub(super) const OUTPUT_RULE: &str = "original_closed_anonymous_entry_surface_output_advisory_v1";
pub(super) const CAPACITY_RULE: &str =
    "retain_one_original_surface_output_assessment_per_worker_v1";

pub(super) fn enabled_on(db: &Connection, root: &str) -> Result<bool, String> {
    if db.is_autocommit() {
        return Err("supervision_capacity_snapshot_required".into());
    }
    let raw: Option<String> = db
        .query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let mode = web_mode::root::read(db, root)?;
    let Some(raw) = raw else {
        // Old journal-only Authorization fixtures legitimately have no Root
        // financial owner. A saved new signed birth is never such a legacy
        // case, even if its mutable definition/version was removed or changed.
        let birth: Option<String> = db
            .query_row(
                "SELECT binding_json FROM agent_root_mode_birth_seals WHERE root_run_id=?1",
                [root],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let birth = birth
            .map(|raw| {
                serde_json::from_str::<Value>(&raw)
                    .map_err(|_| "root_supervision_surface_contract_changed")
            })
            .transpose()?;
        let original = birth
            .as_ref()
            .map(|b| &b["definition"]["bootstrapDispatch"]);
        let new_protocol = |p: &Value| {
            matches!(p["schemaVersion"].as_i64(), Some(12..=15))
                || p.get("externalSurfaceResponseRule").is_some()
                || p.get("externalSurfaceOutputRule").is_some()
                || p.get("externalSurfaceOutputRequestCapacityRule").is_some()
        };
        if original.is_some_and(new_protocol)
            || mode
                .as_ref()
                .and_then(|m| m.bootstrap_dispatch())
                .as_ref()
                .is_some_and(new_protocol)
        {
            return Err("root_supervision_surface_original_contract_missing".into());
        }
        if birth.is_some() {
            // Validate the old signed declaration before accepting old no-op;
            // missing definition/key/signature and scope drift propagate.
            let proof = crate::commands::original_root_capture_mode(db, root)?;
            web_mode::root::verify_birth(db, root, &proof)?;
        }
        return Ok(false);
    };
    let contract: Value =
        serde_json::from_str(&raw).map_err(|_| "root_supervision_surface_contract_changed")?;
    let saved = &contract["root"]["bootstrapDispatch"];
    if !matches!(saved["schemaVersion"].as_i64(), Some(12..=15)) {
        if mode.is_some_and(|m| {
            m.bootstrap_dispatch()
                .is_some_and(|p| matches!(p["schemaVersion"].as_i64(), Some(12..=15)))
        }) {
            return Err("root_supervision_surface_contract_changed".into());
        }
        return Ok(false);
    }
    super::super::root::RootOwner::load_original(db, root)?;
    let mode = mode.ok_or("web_mode_root_missing")?;
    let original = crate::commands::original_root_capture_mode(db, root)?;
    web_mode::root::verify_birth(db, root, &original)?;
    let protocol = mode.bootstrap_dispatch().unwrap_or(Value::Null);
    if mode.mode() != web_mode::WebMode::Multi
        || mode.fact() != original.fact()
        || protocol != *saved
        || protocol["externalSurfaceOutputRule"] != OUTPUT_RULE
        || protocol["externalSurfaceOutputRequestCapacityRule"] != CAPACITY_RULE
        || protocol["externalSurfaceResponseRule"]
            != "original_received_anonymous_public_surface_response_v1"
    {
        return Err("root_supervision_surface_contract_changed".into());
    }
    Ok(true)
}

pub(super) fn pending_on(db: &Connection, actor: &CoordinatorLease) -> Result<i64, String> {
    if !enabled_on(db, &actor.root_run_id)? {
        return Ok(0);
    }
    super::super::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    // Inventory all original sources before inspecting mutable role/state.
    let sql="SELECT id FROM agent_assignments WHERE coordinator_run_id=?1
      UNION SELECT assignment_id FROM agent_assignment_attempts WHERE root_run_id=?1
      UNION SELECT assignment_id FROM agent_capability_leases WHERE root_run_id=?1
      UNION SELECT assignment_id FROM main.agent_public_surface_response_receipts WHERE root_run_id=?1
      UNION SELECT assignment_id FROM main.agent_external_surface_output_receipts WHERE root_run_id=?1
      UNION SELECT assignment_id FROM agent_external_surface_captures WHERE scan_id=?2 AND attempt_number=?3 AND target_url=?4
      UNION SELECT assignment_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id<>'' AND dimension='target_requests' AND substr(source_id,1,7)='public:'";
    let ids = db
        .prepare(sql)
        .map_err(|e| e.to_string())?
        .query_map(
            params![
                actor.root_run_id,
                actor.scan_id,
                actor.attempt_number,
                actor.target_key
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut pending = 0_i64;
    for id in ids {
        let (run,role,trigger,revision,raw):(String,String,String,i64,String)=db.query_row("SELECT a.child_run_id,a.role,a.trigger_code,a.evidence_revision,a.task_slice_json FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.target_key=?3 AND a.lease_epoch=?4 AND a.fencing_token=?5 AND r.assignment_id=a.id AND r.root_run_id=?2 AND r.parent_run_id=?2 AND r.scan_id=?6 AND r.attempt_number=?7 AND r.target_url=?3 AND r.role=a.role AND r.backend='native' AND r.orchestration_policy='multi'",params![id,actor.root_run_id,actor.target_key,actor.lease_epoch,actor.fencing_token,actor.scan_id,actor.attempt_number],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|"root_supervision_surface_inventory_changed")?;
        let key = contract_owner::schedule_contract_key(
            actor.attempt_number,
            &actor.target_key,
            &role,
            &trigger,
            revision,
        );
        let original:bool=db.query_row("SELECT count(*)=1 FROM agent_contract_owners WHERE assignment_id=?1 AND root_run_id=?2 AND contract_key=?3 AND lease_epoch=?4 AND fencing_token=?5",params![id,actor.root_run_id,key,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !original {
            return Err("root_supervision_surface_inventory_changed".into());
        }
        let worker = attempts::current(db, actor, &id)?;
        if worker.child_run_id != run || worker.lease_epoch != 1 {
            return Err("root_supervision_surface_worker_changed".into());
        }
        if role != AgentRole::ExternalSurface.as_str() {
            let marker:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND capability='public_surface_get') OR EXISTS(SELECT 1 FROM main.agent_public_surface_response_receipts WHERE assignment_id=?1 OR child_run_id=?2) OR EXISTS(SELECT 1 FROM main.agent_external_surface_output_receipts WHERE assignment_id=?1 OR child_run_id=?2) OR EXISTS(SELECT 1 FROM agent_external_surface_captures WHERE assignment_id=?1 OR child_run_id=?2) OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?3 AND assignment_id=?1 AND dimension='target_requests' AND substr(source_id,1,7)='public:')",params![id,run,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if marker || trigger == "anonymous_entry_contract" {
                return Err("root_supervision_surface_role_changed".into());
            }
            continue;
        }
        require_task_on(db, actor, &id, &run, &raw, &trigger)?;
        let child = ScheduledChild {
            assignment_id: id,
            run_id: run,
            role: AgentRole::ExternalSurface,
        };
        if crate::commands::native_coordinator_external_surface_output_publication_replay_on(
            db, actor, &child,
        )?
        .is_none()
        {
            pending = pending.checked_add(1).ok_or("budget_amount_overflow")?;
        }
    }
    Ok(pending)
}

fn require_task_on(
    db: &Connection,
    actor: &CoordinatorLease,
    id: &str,
    run: &str,
    raw: &str,
    trigger: &str,
) -> Result<(), String> {
    let task: Value =
        serde_json::from_str(raw).map_err(|_| "root_supervision_surface_task_changed")?;
    let canonical = task.to_string();
    let url = reqwest::Url::parse(
        task["url"]
            .as_str()
            .ok_or("root_supervision_surface_task_changed")?,
    )
    .map_err(|_| "root_supervision_surface_task_changed")?;
    if canonical != raw
        || trigger != "anonymous_entry_contract"
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || task
            != serde_json::json!({"schemaVersion":1,"url":url.as_str(),"method":"GET","anonymousOnly":true,"maxRequests":1,"followRedirects":false})
    {
        return Err("root_supervision_surface_task_changed".into());
    }
    let (caps,shape):(String,bool)=db.query_row("SELECT a.capability_lease_json,a.lane='target_touching' AND r.lane=a.lane AND a.contract_keys_json='[]' AND a.identity_handles_json='[]' FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2",params![id,run],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let mut caps: Vec<String> =
        serde_json::from_str(&caps).map_err(|_| "root_supervision_surface_capabilities_changed")?;
    caps.sort();
    let physical:bool=db.query_row("SELECT count(*)=3 AND count(DISTINCT capability)=3 AND sum(capability IN ('evidence.read','mailbox.write','public_surface_get'))=3 FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5",params![id,run,actor.root_run_id,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
    let all_caps:i64=db.query_row("SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2",params![id,run],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !shape
        || !physical
        || all_caps != 3
        || caps != ["evidence.read", "mailbox.write", "public_surface_get"]
    {
        return Err("root_supervision_surface_capabilities_changed".into());
    }
    let original = super::super::receipts::original_owner(db, run, actor, id)?;
    original.verify(db)?;
    Ok(())
}
