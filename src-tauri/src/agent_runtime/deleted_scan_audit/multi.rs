//! Closed ordinary Web Multi original sources; no live lease or execution grant.
use crate::agent_runtime::{
    execution_owner::{self, NativeInvocationOwner},
    multi_agent::{budget, lease::CoordinatorLease, specialist},
};
use rusqlite::{params, Connection};
use serde_json::Value;
pub(super) fn actor(db: &Connection, root: &str) -> Result<CoordinatorLease, String> {
    let raw: String = db
        .query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|_| "deleted_audit_original_control_missing")?;
    let v: Value =
        serde_json::from_str(&raw).map_err(|_| "deleted_audit_original_control_invalid")?;
    let c = &v["coordinator"];
    let actor = CoordinatorLease {
        scan_id: c["scan"]
            .as_str()
            .ok_or("deleted_audit_original_control_invalid")?
            .into(),
        attempt_number: c["attempt"]
            .as_i64()
            .ok_or("deleted_audit_original_control_invalid")?,
        target_key: c["target"]
            .as_str()
            .ok_or("deleted_audit_original_control_invalid")?
            .into(),
        root_run_id: root.into(),
        lease_epoch: c["epoch"]
            .as_i64()
            .ok_or("deleted_audit_original_control_invalid")?,
        fencing_token: c["fence"]
            .as_str()
            .ok_or("deleted_audit_original_control_invalid")?
            .into(),
        lease_expires_at: String::new(),
    };
    budget::root::RootOwner::load_original(db, root)?.require_original_coordinator(db, &actor)?;
    Ok(actor)
}
pub(super) fn verify_closed(db: &Connection, root: &str) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("deleted_audit_snapshot_required".into());
    }
    let actor = actor(db, root)?;
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r WHERE r.id=?1 AND r.root_run_id=?1 AND r.parent_run_id IS NULL
        AND r.assignment_id='' AND r.backend='native' AND r.role='coordinator' AND r.orchestration_policy='multi'
        AND r.scan_id=?2 AND r.attempt_number=?3 AND r.target_url=?4 AND r.status='terminal')",
        params![root,actor.scan_id,actor.attempt_number,actor.target_key],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact {
        return Err("deleted_audit_original_multi_scope_required".into());
    }
    if actor.target_key.starts_with("source:") {
        return super::source::verify_closed(db, &actor);
    }
    if crate::agent_runtime::web_mode::root::read(db, root)?
        .is_none_or(|m| m.mode() != crate::agent_runtime::web_mode::WebMode::Multi)
    {
        return Err("deleted_audit_original_web_mode_required".into());
    }
    budget::clock::FinalClock::verify_original_exit(db, root)?;
    budget::admission::require_settled_for_completion(db, root)?;
    budget::root::model::tick::verify_closed_original(db, &actor)?;
    let invalid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs r WHERE r.root_run_id=?1 AND r.id<>?1
        AND (r.parent_run_id IS NOT ?1 OR r.scan_id<>?2 OR r.attempt_number<>?3 OR r.target_url<>?4
        OR r.backend<>'native' OR r.orchestration_policy<>'multi' OR r.status<>'terminal' OR r.terminal_state NOT IN ('completed','failed','cancelled')
        OR NOT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=r.id
          WHERE a.id=r.assignment_id AND a.coordinator_run_id=?1 AND a.child_run_id=r.id AND a.role=r.role AND a.lane=r.lane AND a.target_key=r.target_url
          AND a.lease_epoch=?5 AND a.fencing_token=?6 AND a.state IN ('completed','failed','cancelled','lease_expired')
          AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND x.root_run_id=?1
          AND x.coordinator_epoch=?5 AND x.coordinator_fencing_token=?6 AND x.state IN ('completed','failed','cancelled','expired') AND x.finished_at<>'')))
        OR EXISTS(SELECT 1 FROM agent_capability_leases WHERE root_run_id=?1 AND revoked_at='')
        OR EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id IN(SELECT id FROM agent_assignments WHERE coordinator_run_id=?1))
        OR EXISTS(SELECT 1 FROM agent_source_model_rounds WHERE root_run_id=?1)",
        params![root,actor.scan_id,actor.attempt_number,actor.target_key,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
    if invalid {
        return Err("deleted_audit_original_workers_not_closed".into());
    }
    let mut q = db
        .prepare(
            "SELECT assignment_id FROM agent_specialist_calls WHERE root_run_id=?1 ORDER BY rowid",
        )
        .map_err(|e| e.to_string())?;
    let calls = q
        .query_map([root], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    for assignment in calls {
        specialist::verify_received_original(db, &actor, &assignment)?;
    }
    if crate::agent_runtime::web_mode::root::read(db, root)?
        .and_then(|mode| mode.bootstrap_dispatch())
        .is_some_and(|contract| contract["schemaVersion"] == 15)
    {
        // Older normal v15 archives predate this additive receipt table.
        // Do not create it or change their bytes. An actually expired worker
        // below still requires its original signed expired receipt and fails
        // closed when that source is absent.
        let has_expired: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM main.sqlite_master WHERE type='table' AND name='agent_mapper_loop_expired_receipts')",
            [], |row| row.get(0),
        ).map_err(|error| error.to_string())?;
        let expired_inventory = if has_expired {
            " UNION SELECT assignment_id,child_run_id FROM main.agent_mapper_loop_expired_receipts WHERE root_run_id=?1 OR child_run_id IN (SELECT id FROM agent_runs WHERE root_run_id=?1)"
        } else {
            ""
        };
        let sql = format!(
            "SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND (role='spa_api_mapper' OR json_type(task_slice_json,'$.mapperLoop') IS NOT NULL)
             UNION SELECT assignment_id,child_run_id FROM agent_mapper_loop_journal WHERE root_run_id=?1 OR child_run_id IN (SELECT id FROM agent_runs WHERE root_run_id=?1)
             UNION SELECT assignment_id,child_run_id FROM agent_mapper_loop_final_receipts WHERE root_run_id=?1 OR child_run_id IN (SELECT id FROM agent_runs WHERE root_run_id=?1)
             {expired_inventory} ORDER BY 1,2",
        );
        let mut query = db.prepare(&sql).map_err(|error| error.to_string())?;
        let children = query
            .query_map([root], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        for (assignment_id, run_id) in children {
            let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
                assignment_id,
                run_id,
                role: crate::agent_runtime::contract::AgentRole::SpaApiMapper,
            };
            let worker = crate::agent_runtime::multi_agent::attempts::current(
                db,
                &actor,
                &child.assignment_id,
            )?;
            if worker.state == "expired" {
                if !has_expired {
                    return Err("deleted_audit_mapper_loop_original_expired_missing".into());
                }
                crate::agent_runtime::multi_agent::specialist::mapper_loop::expired_seal::read_original_on(db, &actor, &child)?
                    .ok_or("deleted_audit_mapper_loop_original_expired_missing")?;
            } else {
                crate::agent_runtime::multi_agent::specialist::mapper_loop::final_seal::read_final_original_on(db, &actor, &child)?
                    .ok_or("deleted_audit_mapper_loop_original_final_missing")?;
            }
        }
    }
    budget::web_lifetime::verify_closed_original(db, &actor)?;
    super::delivery::verify(db, &actor)?;
    Ok(())
}
pub(super) fn require_idle(
    db: &Connection,
    root: &str,
    parents: &mut super::OriginalSourceParents,
) -> Result<(Vec<NativeInvocationOwner>, specialist::IdleSpecialists), String> {
    verify_closed(db, root)?;
    super::financial_schema::require_original(db)?;
    let actor = actor(db, root)?;
    if actor.target_key.starts_with("source:") {
        return super::source::require_idle(db, &actor, parents);
    }
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("deleted_audit_database_missing")?;
    let parent = execution_owner::probe_native_invocation(
        std::path::Path::new(path),
        &actor.scan_id,
        actor.attempt_number,
        "parent-supervisor",
        root,
    )?
    .ok_or("deleted_audit_original_parent_exit_missing")?;
    let mut guards = vec![parent];
    guards.extend(budget::root::model::tick::require_idle_original(
        db, &actor,
    )?);
    let specialists = specialist::require_idle_for_root(db, &actor)?;
    guards.extend(budget::web_lifetime::require_idle_original(db, &actor)?);
    Ok((guards, specialists))
}
