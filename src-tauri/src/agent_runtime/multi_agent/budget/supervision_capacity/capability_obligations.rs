//! Original v10 capability closure obligations. Pure reads, no grants or bills.
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{attempts, contract_owner, lease::CoordinatorLease, scheduler::ScheduledChild},
    web_mode,
};
use rusqlite::{params, Connection};
use serde_json::Value;

pub(super) fn enabled_on(db: &Connection, root: &str) -> Result<bool, String> {
    if db.is_autocommit() {
        return Err("supervision_capacity_snapshot_required".into());
    }
    let contract: String = db
        .query_row(
            "SELECT contract_json FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [root],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let contract: Value = serde_json::from_str(&contract)
        .map_err(|_| "root_supervision_capability_contract_changed")?;
    let bootstrap = &contract["root"]["bootstrapDispatch"];
    if !matches!(bootstrap["schemaVersion"].as_i64(), Some(10..=15)) {
        // No new financial-owner gate for unchanged old contracts. A mode
        // still declaring v10 cannot downgrade by a cold saved-version edit.
        if web_mode::root::read(db, root)?.is_some_and(|m| {
            m.bootstrap_dispatch()
                .is_some_and(|p| matches!(p["schemaVersion"].as_i64(), Some(10..=15)))
        }) {
            return Err("root_supervision_capability_contract_changed".into());
        }
        return Ok(false);
    }
    super::super::root::RootOwner::load_original(db, root)?;
    let mode = web_mode::root::read(db, root)?.ok_or("web_mode_root_missing")?;
    let protocol = mode.bootstrap_dispatch().unwrap_or(Value::Null);
    if protocol != *bootstrap
        || protocol["capabilityTransitionRule"] != web_mode::capability_transition_receipt::RULE
    {
        return Err("root_supervision_capability_contract_changed".into());
    }
    if matches!(protocol["schemaVersion"].as_i64(), Some(11..=15))
        && !super::authorization_obligations::enabled_on(db, root)?
    {
        return Err("root_supervision_capability_contract_changed".into());
    }
    Ok(true)
}

pub(super) fn pending_on(db: &Connection, actor: &CoordinatorLease) -> Result<i64, String> {
    if !enabled_on(db, &actor.root_run_id)? {
        return Ok(0);
    }
    super::super::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    // Union all original assignments, worker grants, reserve rows, cap sets and
    // sealed receipts. No mutable role filter may hide an original obligation.
    let sql = "SELECT id FROM agent_assignments WHERE coordinator_run_id=?1
        UNION SELECT assignment_id FROM agent_assignment_attempts WHERE root_run_id=?1
        UNION SELECT assignment_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id<>'' AND kind='reserve' AND source_id='assignment:'||assignment_id
        UNION SELECT assignment_id FROM agent_capability_leases WHERE root_run_id=?1
        UNION SELECT assignment_id FROM main.agent_web_capability_transition_receipts WHERE root_run_id=?1";
    let ids = db
        .prepare(sql)
        .map_err(|e| e.to_string())?
        .query_map([&actor.root_run_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    let mut pending = 0_i64;
    for id in ids {
        let (run, role, trigger, revision):(String,String,String,i64) = db.query_row(
            "SELECT a.child_run_id,a.role,a.trigger_code,a.evidence_revision FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND a.coordinator_run_id=?2 AND a.target_key=?3 AND a.lease_epoch=?4 AND a.fencing_token=?5 AND r.assignment_id=a.id AND r.root_run_id=?2 AND r.parent_run_id=?2 AND r.scan_id=?6 AND r.attempt_number=?7 AND r.target_url=?3 AND r.role=a.role AND r.backend='native' AND r.orchestration_policy='multi'",params![id,actor.root_run_id,actor.target_key,actor.lease_epoch,actor.fencing_token,actor.scan_id,actor.attempt_number],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"root_supervision_capability_inventory_changed")?;
        let key = contract_owner::schedule_contract_key(
            actor.attempt_number,
            &actor.target_key,
            &role,
            &trigger,
            revision,
        );
        let original:bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_contract_owners WHERE assignment_id=?1 AND root_run_id=?2 AND contract_key=?3 AND lease_epoch=?4 AND fencing_token=?5)",params![id,actor.root_run_id,key,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
        if !original {
            return Err("root_supervision_capability_inventory_changed".into());
        }
        let worker = attempts::current(db, actor, &id)?;
        if worker.child_run_id != run || worker.lease_epoch != 1 {
            return Err("root_supervision_capability_worker_changed".into());
        }
        // Every issued worker has an original reserve, even after settlement.
        let original_reserve:bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND kind='reserve' AND source_id=?4)",params![actor.root_run_id,id,worker.id,format!("assignment:{id}")],|r|r.get(0)).map_err(|e|e.to_string())?;
        if role != AgentRole::WebExecutor.as_str() {
            let sealed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_web_capability_transition_receipts WHERE assignment_id=?1)",[&id],|r|r.get(0)).map_err(|e|e.to_string())?;
            let web_mark:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 AND substr(dedup_key,1,13)='web_executor:') OR EXISTS(SELECT 1 FROM agent_web_model_journal WHERE child_run_id=?2) OR EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND capability IN ('replay_http','fetch_page','compare_identities'))",params![id,run],|r|r.get(0)).map_err(|e|e.to_string())?;
            if sealed || web_mark {
                return Err("root_supervision_capability_role_changed".into());
            }
            continue;
        }
        let finite: bool = db
            .query_row(
                "SELECT hard_token_budget>0 OR hard_request_budget>0 FROM agent_runs WHERE id=?1",
                [&actor.root_run_id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if finite && !original_reserve {
            return Err("root_supervision_capability_grant_missing".into());
        }
        let child = ScheduledChild {
            assignment_id: id,
            run_id: run,
            role: AgentRole::WebExecutor,
        };
        // Signed receipt alone, settled fees or revoked caps do not discharge
        // the request. Only the independently verified final paid publication.
        let published = if worker.state == "expired"
            && worker.failure_class == "worker_lease_expired"
        {
            // The dedicated signed expiry delivery and its original paid final
            // assessment can close this worker's obligation. State alone is
            // never enough; normal running/completed workers must not enter
            // the expiry reader or adopt an unrelated withdrawal cause.
            crate::commands::native_coordinator_web_expiry_publication_replay_on(db, actor, &child)?
        } else {
            crate::commands::native_coordinator_capability_transition_publication_replay_on(
                db, actor, &child,
            )?
        };
        if published.is_none() {
            pending = pending.checked_add(1).ok_or("budget_amount_overflow")?;
        }
    }
    Ok(pending)
}
