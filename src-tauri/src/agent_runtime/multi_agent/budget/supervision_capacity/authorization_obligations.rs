//! Original v11 Authorization output assessment obligations. No writes/grants.
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{attempts, contract_owner, lease::CoordinatorLease, scheduler::ScheduledChild},
    web_mode,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::HashSet;

pub(super) const OUTPUT_RULE: &str = "original_closed_three_side_authorization_output_advisory_v1";
pub(super) const CAPACITY_RULE: &str =
    "retain_one_original_authorization_output_assessment_per_worker_v1";

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
                    .map_err(|_| "root_supervision_authorization_contract_changed")
            })
            .transpose()?;
        let original = birth
            .as_ref()
            .map(|b| &b["definition"]["bootstrapDispatch"]);
        let new_protocol = |p: &Value| {
            matches!(p["schemaVersion"].as_i64(), Some(11..=15))
                || p.get("authorizationOutputRule").is_some()
                || p.get("authorizationOutputRequestCapacityRule").is_some()
        };
        if original.is_some_and(new_protocol)
            || mode
                .as_ref()
                .and_then(|m| m.bootstrap_dispatch())
                .as_ref()
                .is_some_and(new_protocol)
        {
            return Err("root_supervision_authorization_original_contract_missing".into());
        }
        if birth.is_some() {
            // Validate the old signed declaration before accepting old no-op;
            // missing definition/key/signature and scope drift propagate.
            let proof = crate::scanning::scope_contract::original_root_capture_mode(db, root)?;
            web_mode::root::verify_birth(db, root, &proof)?;
        }
        return Ok(false);
    };
    let contract: Value = serde_json::from_str(&raw)
        .map_err(|_| "root_supervision_authorization_contract_changed")?;
    let saved = &contract["root"]["bootstrapDispatch"];
    if !matches!(saved["schemaVersion"].as_i64(), Some(11..=15)) {
        if mode.is_some_and(|m| {
            m.bootstrap_dispatch()
                .is_some_and(|p| matches!(p["schemaVersion"].as_i64(), Some(11..=15)))
        }) {
            return Err("root_supervision_authorization_contract_changed".into());
        }
        return Ok(false);
    }
    super::super::root::RootOwner::load_original(db, root)?;
    let mode = mode.ok_or("web_mode_root_missing")?;
    let original = crate::scanning::scope_contract::original_root_capture_mode(db, root)?;
    web_mode::root::verify_birth(db, root, &original)?;
    let protocol = mode.bootstrap_dispatch().unwrap_or(Value::Null);
    if mode.mode() != web_mode::WebMode::Multi
        || mode.fact() != original.fact()
        || protocol != *saved
        || protocol["authorizationOutputRule"] != OUTPUT_RULE
        || protocol["authorizationOutputRequestCapacityRule"] != CAPACITY_RULE
    {
        return Err("root_supervision_authorization_contract_changed".into());
    }
    Ok(true)
}

pub(super) fn pending_on(db: &Connection, actor: &CoordinatorLease) -> Result<i64, String> {
    if !enabled_on(db, &actor.root_run_id)? {
        return Ok(0);
    }
    super::super::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    // No role filter: deleted joins and cold role edits cannot erase the
    // original worker, side dispatch, capability or aggregate signed source.
    let sql="SELECT id FROM agent_assignments WHERE coordinator_run_id=?1
      UNION SELECT assignment_id FROM agent_assignment_attempts WHERE root_run_id=?1
      UNION SELECT assignment_id FROM agent_capability_leases WHERE root_run_id=?1
      UNION SELECT assignment_id FROM main.agent_authorization_side_receipts WHERE root_run_id=?1
      UNION SELECT assignment_id FROM main.agent_authorization_output_receipts WHERE root_run_id=?1
      UNION SELECT assignment_id FROM agent_authorization_probe_claims WHERE scan_id=?2 AND attempt_number=?3 AND target_url=?4
      UNION SELECT assignment_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id<>'' AND dimension='target_requests' AND substr(source_id,1,14)='authorization:'";
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
        let (run,role,trigger,revision,task):(String,String,String,i64,String)=db.query_row("SELECT a.child_run_id,a.role,a.trigger_code,a.evidence_revision,a.task_slice_json FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.target_key=?3 AND a.lease_epoch=?4 AND a.fencing_token=?5 AND r.assignment_id=a.id AND r.root_run_id=?2 AND r.parent_run_id=?2 AND r.scan_id=?6 AND r.attempt_number=?7 AND r.target_url=?3 AND r.role=a.role AND r.backend='native' AND r.orchestration_policy='multi'",params![id,actor.root_run_id,actor.target_key,actor.lease_epoch,actor.fencing_token,actor.scan_id,actor.attempt_number],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|_|"root_supervision_authorization_inventory_changed")?;
        let key = contract_owner::schedule_contract_key(
            actor.attempt_number,
            &actor.target_key,
            &role,
            &trigger,
            revision,
        );
        let original:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_contract_owners WHERE assignment_id=?1 AND root_run_id=?2 AND contract_key=?3 AND lease_epoch=?4 AND fencing_token=?5)",params![id,actor.root_run_id,key,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !original {
            return Err("root_supervision_authorization_inventory_changed".into());
        }
        let worker = attempts::current(db, actor, &id)?;
        if worker.child_run_id != run || worker.lease_epoch != 1 {
            return Err("root_supervision_authorization_worker_changed".into());
        }
        if role != AgentRole::Authorization.as_str() {
            let marker:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND capability='authorization_probe') OR EXISTS(SELECT 1 FROM main.agent_authorization_side_receipts WHERE assignment_id=?1 OR child_run_id=?2) OR EXISTS(SELECT 1 FROM main.agent_authorization_output_receipts WHERE assignment_id=?1 OR child_run_id=?2) OR EXISTS(SELECT 1 FROM agent_authorization_probe_claims WHERE assignment_id=?1 OR child_run_id=?2) OR EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?3 AND assignment_id=?1 AND dimension='target_requests' AND substr(source_id,1,14)='authorization:')",params![id,run,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
            if marker || trigger == "operator_authorization_control_ready" {
                return Err("root_supervision_authorization_role_changed".into());
            }
            continue;
        }
        require_task_on(db, actor, &id, &run, &task, &trigger)?;
        let child = ScheduledChild {
            assignment_id: id,
            run_id: run,
            role: AgentRole::Authorization,
        };
        // N three-side controls are one frozen aggregate worker output. No
        // per-side/settled/revoked/failed state can erase its one obligation.
        if crate::scanning::publication_replay::native_coordinator_authorization_output_publication_replay_on(
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
    assignment: &str,
    run: &str,
    raw: &str,
    trigger: &str,
) -> Result<(), String> {
    let task: Value =
        serde_json::from_str(raw).map_err(|_| "root_supervision_authorization_task_changed")?;
    let keys = task["contractKeys"]
        .as_array()
        .ok_or("root_supervision_authorization_task_changed")?;
    let mut unique = HashSet::new();
    let canonical = task.to_string();
    if canonical != raw
        || trigger != "operator_authorization_control_ready"
        || keys.is_empty()
        || keys.len() > 4
        || task["objective"] != "仅执行已登记的 A→X、B→X、B→Y GET 控制组"
        || task.as_object().is_none_or(|o| o.len() != 2)
    {
        return Err("root_supervision_authorization_task_changed".into());
    }
    for value in keys {
        let key = value
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("root_supervision_authorization_task_changed")?;
        if !unique.insert(key) {
            return Err("root_supervision_authorization_task_changed".into());
        }
        let control:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND contract_key=?4 AND method='GET' AND owner_identity<>tester_identity)",params![actor.scan_id,actor.attempt_number,actor.target_key,key],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !control {
            return Err("root_supervision_authorization_task_changed".into());
        }
    }
    let original_count:i64=db.query_row("SELECT count(*) FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3",params![actor.scan_id,actor.attempt_number,actor.target_key],|r|r.get(0)).map_err(|e|e.to_string())?;
    if usize::try_from(original_count).ok() != Some(unique.len()) {
        return Err("root_supervision_authorization_task_changed".into());
    }
    let shape:bool=db.query_row(r#"SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.lane='target_touching' AND r.lane=a.lane AND a.capability_lease_json='["authorization_probe"]' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.contract_keys_json='[]' AND a.identity_handles_json='[]') AND EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND capability='authorization_probe') AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND capability<>'authorization_probe')"#,params![assignment,run],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !shape {
        return Err("root_supervision_authorization_task_changed".into());
    }
    Ok(())
}
