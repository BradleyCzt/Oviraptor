//! Original Mapper SDK diagnostic ownership; never dispatch or tool authority.
use super::owner::{Domain, Owner};
use crate::agent_runtime::{
    contract::AgentRole,
    multi_agent::{lease::CoordinatorLease, scheduler::ScheduledChild, specialist::mapper_loop},
};
use rusqlite::{params, Connection, OptionalExtension};

fn scope(db: &Connection, owner: &Owner) -> Result<(CoordinatorLease, ScheduledChild), String> {
    let (target,epoch,fence):(String,i64,String)=db.query_row(
        "SELECT json_extract(fact_json,'$.scope.targetKey'),json_extract(fact_json,'$.scope.epoch'),json_extract(fact_json,'$.scope.fence') FROM agent_mapper_loop_journal WHERE call_id=?1 AND phase='dispatch' AND root_run_id=?2 AND assignment_id=?3 AND child_run_id=?4 AND worker_attempt_id=?5 AND round_number=?6 AND request_hash=?7",
        params![owner.dispatch_key,owner.root_run_id,owner.assignment_id,owner.run_id,owner.lease_attempt_id,owner.round,owner.request_hash],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"native_sdk_log_original_dispatch_missing")?;
    Ok((
        CoordinatorLease {
            scan_id: owner.scan_id.clone(),
            attempt_number: owner.attempt,
            target_key: target,
            root_run_id: owner.root_run_id.clone(),
            lease_epoch: epoch,
            fencing_token: fence,
            lease_expires_at: String::new(),
        },
        ScheduledChild {
            assignment_id: owner
                .assignment_id
                .clone()
                .ok_or("native_sdk_log_owner_unverified")?,
            run_id: owner.run_id.clone(),
            role: AgentRole::SpaApiMapper,
        },
    ))
}
pub(super) fn load(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    round: i64,
) -> Result<Owner, String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return load(&tx, actor, child, round);
    }
    if child.role != AgentRole::SpaApiMapper || !(1..=2).contains(&round) {
        return Err("native_sdk_log_mapper_scope_invalid".into());
    }
    let (id,worker,worker_id,hash):(String,String,String,String)=db.query_row(
        "SELECT j.call_id,j.worker_attempt_id,x.worker_id,j.request_hash FROM agent_mapper_loop_journal j JOIN agent_assignment_attempts x ON x.id=j.worker_attempt_id AND x.assignment_id=j.assignment_id AND x.child_run_id=j.child_run_id AND x.root_run_id=j.root_run_id JOIN agent_runs r ON r.id=j.child_run_id AND r.root_run_id=j.root_run_id AND r.assignment_id=j.assignment_id WHERE j.root_run_id=?1 AND j.assignment_id=?2 AND j.child_run_id=?3 AND j.round_number=?4 AND j.phase='dispatch' AND r.scan_id=?5 AND r.attempt_number=?6 AND r.target_url=?7 AND r.role='spa_api_mapper' AND r.backend='native' AND r.orchestration_policy='multi' AND x.coordinator_epoch=?8 AND x.coordinator_fencing_token=?9 AND NOT EXISTS(SELECT 1 FROM agent_mapper_loop_journal t WHERE t.call_id=j.call_id AND t.phase<>'dispatch')",
        params![actor.root_run_id,child.assignment_id,child.run_id,round,actor.scan_id,actor.attempt_number,actor.target_key,actor.lease_epoch,actor.fencing_token],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|"native_sdk_log_original_dispatch_missing")?;
    let mut owner = Owner {
        owner_id: String::new(),
        domain: Domain::MapperLoop,
        dispatch_key: id,
        scan_id: actor.scan_id.clone(),
        attempt: actor.attempt_number,
        root_run_id: actor.root_run_id.clone(),
        run_id: child.run_id.clone(),
        assignment_id: Some(child.assignment_id.clone()),
        lease_attempt_id: worker,
        worker_id: Some(worker_id),
        round,
        request_hash: hash,
    };
    owner.seal()?;
    verify(db, &owner, true)?;
    Ok(owner)
}
pub(super) fn verify(db: &Connection, owner: &Owner, pristine: bool) -> Result<(), String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return verify(&tx, owner, pristine);
    }
    let (actor, child) = scope(db, owner)?;
    mapper_loop::verify_dispatch_original_on(db, &actor, &child, &owner.dispatch_key)?;
    let exact:bool=db.query_row("SELECT count(*)=1 FROM agent_mapper_loop_journal j JOIN agent_assignment_attempts x ON x.id=j.worker_attempt_id AND x.assignment_id=j.assignment_id AND x.child_run_id=j.child_run_id AND x.root_run_id=j.root_run_id JOIN agent_runs r ON r.id=j.child_run_id WHERE j.call_id=?1 AND j.phase='dispatch' AND j.root_run_id=?2 AND j.assignment_id=?3 AND j.child_run_id=?4 AND j.worker_attempt_id=?5 AND j.round_number=?6 AND j.request_hash=?7 AND x.worker_id=?8 AND r.scan_id=?9 AND r.attempt_number=?10 AND r.role='spa_api_mapper' AND r.backend='native' AND r.orchestration_policy='multi'",params![owner.dispatch_key,owner.root_run_id,owner.assignment_id,owner.run_id,owner.lease_attempt_id,owner.round,owner.request_hash,owner.worker_id,owner.scan_id,owner.attempt],|r|r.get(0)).map_err(|_|"native_sdk_log_owner_unverified")?;
    if !exact || owner.domain != Domain::MapperLoop || !(1..=2).contains(&owner.round) {
        return Err("native_sdk_log_owner_unverified".into());
    }
    if pristine {
        let changed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_mapper_loop_journal WHERE call_id=?1 AND phase<>'dispatch')",[&owner.dispatch_key],|r|r.get(0)).map_err(|e|e.to_string())?;
        if changed {
            return Err("native_sdk_log_original_dispatch_missing".into());
        }
    }
    Ok(())
}
pub(super) fn cost_phase(db: &Connection, owner: &Owner) -> Result<Option<String>, String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return cost_phase(&tx, owner);
    }
    verify(db, owner, false)?;
    let (actor, child) = scope(db, owner)?;
    let phase = mapper_loop::cost_phase_original_on(db, &actor, &child, &owner.dispatch_key)?;
    let phase = match phase.as_deref() {
        Some("received" | "uncertain" | "unsent") | None => phase,
        // The actual invoice is persisted before known fees; it is not cost_saved.
        Some("invoice") => None,
        Some(_) => return Err("native_sdk_log_cost_unverified".into()),
    };
    Ok(phase)
}

fn require_extension_schema(db: &Connection) -> Result<(), String> {
    let expected =
        Connection::open_in_memory().map_err(|_| "native_sdk_log_mapper_extension_invalid")?;
    expected
        .execute_batch(include_str!("schema.sql"))
        .map_err(|_| "native_sdk_log_mapper_extension_invalid")?;
    for name in [
        "native_sdk_log_mapper_owners",
        "native_sdk_log_mapper_no_update",
        "native_sdk_log_mapper_no_delete",
        "native_sdk_log_mapper_no_replace",
    ] {
        let read = |connection: &Connection| -> Result<String, String> {
            connection
                .query_row(
                    "SELECT sql FROM main.sqlite_master WHERE name=?1",
                    [name],
                    |r| r.get(0),
                )
                .map_err(|_| "native_sdk_log_mapper_extension_invalid".into())
        };
        if read(db)? != read(&expected)? {
            return Err("native_sdk_log_mapper_extension_invalid".into());
        }
    }
    Ok(())
}

pub(super) fn restore_logical(
    db: &Connection,
    owner: &mut Owner,
    identity: &str,
) -> Result<(), String> {
    require_extension_schema(db)?;
    if owner.domain == Domain::MapperLoop {
        return Err("native_sdk_log_mapper_extension_invalid".into());
    }
    let extension: Option<(String,String)> = db.query_row("SELECT logical_domain,identity_hash FROM main.native_sdk_log_mapper_owners WHERE owner_id=?1", [&owner.owner_id], |r| Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_| "native_sdk_log_mapper_extension_invalid")?;
    if let Some((domain, hash)) = extension {
        if owner.domain != Domain::Specialist || domain != "mapper_loop" || hash != identity {
            return Err("native_sdk_log_mapper_extension_invalid".into());
        }
        owner.domain = Domain::MapperLoop;
        verify(db, owner, false)?;
    }
    Ok(())
}
pub(super) fn verify_extension(db: &Connection, owner: &Owner) -> Result<(), String> {
    let identity = crate::agent_runtime::store::stable_hash(
        &serde_json::to_string(owner).map_err(|_| "native_sdk_log_mapper_extension_invalid")?,
    );
    let mut storage = owner.clone();
    storage.domain = storage.domain.storage();
    restore_logical(db, &mut storage, &identity)?;
    if storage != *owner {
        return Err("native_sdk_log_mapper_extension_invalid".into());
    }
    Ok(())
}
pub(super) fn verify_extension_if_saved(db: &Connection, owner: &Owner) -> Result<(), String> {
    let exists: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM native_sdk_log_owners WHERE owner_id=?1)",
            [&owner.owner_id],
            |r| r.get(0),
        )
        .map_err(|_| "native_sdk_log_mapper_extension_invalid")?;
    if exists {
        verify_extension(db, owner)?;
    }
    Ok(())
}
