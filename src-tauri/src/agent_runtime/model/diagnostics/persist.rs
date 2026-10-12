//! Private FULL-synchronous SDK diagnostics cannot write any business lane.
use super::owner::Owner;
use crate::agent_runtime::store::stable_hash;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    params, Connection, OpenFlags,
};
use serde::Serialize;
use std::{path::Path, time::Duration};
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Hint {
    pub scan_id: String,
    pub attempt: i64,
    pub owner_id: String,
    pub domain: super::owner::Domain,
    pub dispatch_key: String,
    pub run_id: String,
    pub worker_id: Option<String>,
    pub sequence: i64,
    pub round: i64,
}
fn protect<T>(
    db: &Connection,
    owner_insert: bool,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    db.authorizer(Some(move |c: AuthContext<'_>| {
        let direct = c.database_name == Some("main") && c.accessor.is_none();
        let allowed = match c.action {
            AuthAction::Insert { table_name } if direct => {
                matches!(table_name, "native_sdk_log_rows" | "native_sdk_log_gaps")
                    || (owner_insert
                        && matches!(
                            table_name,
                            "native_sdk_log_owners" | "native_sdk_log_mapper_owners"
                        ))
            }
            AuthAction::Read { .. }
            | AuthAction::Select
            | AuthAction::Function { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. }
            | AuthAction::Recursive => true,
            _ => false,
        };
        if allowed {
            Authorization::Allow
        } else {
            Authorization::Deny
        }
    }))
    .map_err(|_| "native_sdk_log_guard_failed")?;
    let value = work();
    let cleared = db
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(|_| "native_sdk_log_guard_failed");
    let value = value?;
    cleared?;
    Ok(value)
}
pub(super) fn connection(path: &Path) -> Result<Connection, String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|_| "native_sdk_log_database_unavailable")?;
    db.busy_timeout(Duration::from_millis(25))
        .map_err(|_| "native_sdk_log_database_unavailable")?;
    db.pragma_update(None, "foreign_keys", true)
        .map_err(|_| "native_sdk_log_database_unavailable")?;
    db.pragma_update(None, "synchronous", "FULL")
        .map_err(|_| "native_sdk_log_database_unavailable")?;
    Ok(db)
}
fn expected(owner: &Owner) -> String {
    serde_json::to_string(owner).unwrap_or_default()
}
fn exact_owner(db: &Connection, owner: &Owner, time: &str) -> Result<bool, String> {
    super::owner_mapper_loop::verify_extension_if_saved(db, owner)?;
    let identity = stable_hash(&expected(owner));
    db.query_row("SELECT COUNT(*)=1 FROM native_sdk_log_owners WHERE owner_id=?1 AND domain=?2 AND dispatch_key=?3
   AND scan_id=?4 AND attempt_number=?5 AND root_run_id=?6 AND run_id=?7 AND assignment_id IS ?8
   AND lease_attempt_id=?9 AND worker_id IS ?10 AND round_number=?11 AND request_hash=?12 AND identity_hash=?13 AND created_at=?14",
   params![owner.owner_id,owner.domain.storage(),owner.dispatch_key,owner.scan_id,owner.attempt,owner.root_run_id,owner.run_id,owner.assignment_id,owner.lease_attempt_id,owner.worker_id,owner.round,owner.request_hash,identity,time],|r|r.get(0))
   .map_err(|_|"native_sdk_log_owner_unverified".into())
}
pub(super) fn begin(db: &mut Connection, owner: &Owner) -> Result<Hint, String> {
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "native_sdk_log_commit_failed")?;
    let hint = protect(&tx, true, || {
        owner.verify_pristine(&tx)?;
        let time: String = tx
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|_| "native_sdk_log_commit_failed")?;
        let n=tx.execute("INSERT INTO native_sdk_log_owners(owner_id,domain,dispatch_key,scan_id,attempt_number,root_run_id,run_id,assignment_id,
    lease_attempt_id,worker_id,round_number,request_hash,identity_hash,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
    params![owner.owner_id,owner.domain.storage(),owner.dispatch_key,owner.scan_id,owner.attempt,owner.root_run_id,owner.run_id,owner.assignment_id,owner.lease_attempt_id,owner.worker_id,owner.round,owner.request_hash,stable_hash(&expected(owner)),time]).map_err(|_|"native_sdk_log_owner_not_persisted")?;
        if owner.domain == super::owner::Domain::MapperLoop {
            tx.execute("INSERT INTO main.native_sdk_log_mapper_owners(owner_id,logical_domain,identity_hash) VALUES(?1,'mapper_loop',?2)", params![owner.owner_id, stable_hash(&expected(owner))]).map_err(|_| "native_sdk_log_mapper_owner_not_persisted")?;
        }
        if n != 1 || !exact_owner(&tx, owner, &time)? {
            return Err("native_sdk_log_owner_not_persisted".into());
        }
        super::owner_mapper_loop::verify_extension(&tx, owner)?;
        owner.verify_pristine(&tx)?;
        append_row(&tx, owner, "prepared", "", "")
    })?;
    tx.commit().map_err(|_| "native_sdk_log_commit_failed")?;
    Ok(hint)
}
pub(super) fn append(
    db: &mut Connection,
    owner: &Owner,
    stage: &str,
    cost: &str,
    terminal: &str,
) -> Result<Hint, String> {
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "native_sdk_log_commit_failed")?;
    let hint = protect(&tx, false, || append_row(&tx, owner, stage, cost, terminal))?;
    tx.commit().map_err(|_| "native_sdk_log_commit_failed")?;
    Ok(hint)
}
fn append_row(
    tx: &Connection,
    owner: &Owner,
    stage: &str,
    cost: &str,
    terminal: &str,
) -> Result<Hint, String> {
    let time: String = tx
        .query_row(
            "SELECT created_at FROM native_sdk_log_owners WHERE owner_id=?1",
            [&owner.owner_id],
            |r| r.get(0),
        )
        .map_err(|_| "native_sdk_log_owner_unverified")?;
    if !exact_owner(tx, owner, &time)? {
        return Err("native_sdk_log_owner_unverified".into());
    }
    let has_gap: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM native_sdk_log_gaps WHERE owner_id=?1)",
            [&owner.owner_id],
            |r| r.get(0),
        )
        .map_err(|_| "native_sdk_log_gap_unverified")?;
    if has_gap {
        return Err("native_sdk_log_closed_gap".into());
    }
    super::owner_saved::verify(tx, owner)?;
    super::transitions::validate(tx, &owner.owner_id, stage, cost, terminal)?;
    // Later cancellation/fees keep ORIGINAL diagnostic identity, never new rights.
    let (count,last,closed):(i64,i64,bool)=tx.query_row("SELECT COUNT(*),COALESCE(MAX(ordinal),0),COALESCE(MAX(stage='terminal'),0) FROM native_sdk_log_rows WHERE owner_id=?1",[&owner.owner_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"native_sdk_log_commit_failed")?;
    if count != last
        || count >= 6
        || closed
        || !matches!(
            stage,
            "prepared" | "sent" | "response_received" | "cost_saved" | "validated" | "terminal"
        )
        || (count == 0 && stage != "prepared")
        || !matches!(cost, "" | "received" | "unsent" | "uncertain")
        || !matches!(
            terminal,
            "" | "returned" | "withheld" | "uncertain" | "unsent" | "failed"
        )
        || (stage == "terminal") != (!terminal.is_empty())
    {
        return Err("native_sdk_log_stage_invalid".into());
    }
    if !cost.is_empty() && owner.cost_phase(tx)?.as_deref() != Some(cost) {
        return Err("native_sdk_log_cost_unverified".into());
    }
    let now: String = tx
        .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
        .map_err(|_| "native_sdk_log_commit_failed")?;
    let n=tx.execute("INSERT INTO native_sdk_log_rows(owner_id,ordinal,stage,cost_phase,terminal_state,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![owner.owner_id,count+1,stage,cost,terminal,now]).map_err(|_|"native_sdk_log_row_not_persisted")?;
    let sequence = tx.last_insert_rowid();
    let exact:bool=tx.query_row("SELECT COUNT(*)=1 FROM native_sdk_log_rows WHERE sequence=?1 AND owner_id=?2 AND ordinal=?3 AND stage=?4 AND cost_phase=?5 AND terminal_state=?6 AND created_at=?7",params![sequence,owner.owner_id,count+1,stage,cost,terminal,now],|r|r.get(0)).map_err(|_|"native_sdk_log_row_not_persisted")?;
    let rows: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM native_sdk_log_rows WHERE owner_id=?1",
            [&owner.owner_id],
            |r| r.get(0),
        )
        .map_err(|_| "native_sdk_log_row_not_persisted")?;
    if n != 1 || !exact || rows != count + 1 || !exact_owner(tx, owner, &time)? {
        return Err("native_sdk_log_row_not_persisted".into());
    }
    Ok(Hint {
        scan_id: owner.scan_id.clone(),
        attempt: owner.attempt,
        owner_id: owner.owner_id.clone(),
        domain: owner.domain,
        dispatch_key: owner.dispatch_key.clone(),
        run_id: owner.run_id.clone(),
        worker_id: owner.worker_id.clone(),
        round: owner.round,
        sequence,
    })
}

pub(super) fn gap(db: &mut Connection, owner: &Owner, stage: &str) -> Result<Hint, String> {
    let tx = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "native_sdk_log_commit_failed")?;
    let hint = protect(&tx, false, || {
        super::owner_saved::verify(&tx, owner)?;
        let time: String = tx
            .query_row(
                "SELECT created_at FROM native_sdk_log_owners WHERE owner_id=?1",
                [&owner.owner_id],
                |r| r.get(0),
            )
            .map_err(|_| "native_sdk_log_owner_unverified")?;
        if !exact_owner(&tx, owner, &time)? {
            return Err("native_sdk_log_owner_unverified".into());
        }
        let (ordinal,sequence):(i64,i64)=tx.query_row("SELECT COALESCE(MAX(ordinal),0),COALESCE(MAX(sequence),0) FROM native_sdk_log_rows WHERE owner_id=?1",[&owner.owner_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|_|"native_sdk_log_commit_failed")?;
        let now: String = tx
            .query_row("SELECT datetime('now','localtime')", [], |r| r.get(0))
            .map_err(|_| "native_sdk_log_commit_failed")?;
        let n=tx.execute("INSERT INTO native_sdk_log_gaps(owner_id,after_ordinal,failed_stage,code,created_at) VALUES(?1,?2,?3,'stage_write_failed',?4)",params![owner.owner_id,ordinal,stage,now]).map_err(|_|"native_sdk_log_gap_not_persisted")?;
        let exact:bool=tx.query_row("SELECT COUNT(*)=1 FROM native_sdk_log_gaps WHERE owner_id=?1 AND after_ordinal=?2 AND failed_stage=?3 AND code='stage_write_failed' AND created_at=?4",params![owner.owner_id,ordinal,stage,now],|r|r.get(0)).map_err(|_|"native_sdk_log_gap_not_persisted")?;
        if n != 1 || !exact || !exact_owner(&tx, owner, &time)? {
            return Err("native_sdk_log_gap_not_persisted".into());
        }
        Ok(Hint {
            scan_id: owner.scan_id.clone(),
            attempt: owner.attempt,
            owner_id: owner.owner_id.clone(),
            domain: owner.domain,
            dispatch_key: owner.dispatch_key.clone(),
            run_id: owner.run_id.clone(),
            worker_id: owner.worker_id.clone(),
            round: owner.round,
            sequence,
        })
    })?;
    tx.commit().map_err(|_| "native_sdk_log_commit_failed")?;
    Ok(hint)
}
