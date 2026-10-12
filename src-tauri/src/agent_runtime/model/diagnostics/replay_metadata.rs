//! Bounded, same-snapshot diagnostic metadata; no guessed missing stages.
use super::owner::Owner;
use crate::agent_runtime::store::stable_hash;
use rusqlite::{params, Connection};
use serde::Serialize;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Gap {
    pub owner_id: String,
    pub after_ordinal: i64,
    pub failed_stage: String,
    pub code: String,
    pub time: String,
}
pub(super) struct Metadata {
    pub owners: Vec<Owner>,
    pub owners_truncated: bool,
    pub gaps: Vec<Gap>,
    pub gaps_truncated: bool,
    pub incomplete_owners: Vec<String>,
    pub incomplete_owners_truncated: bool,
}
pub(super) fn load(
    db: &Connection,
    scan: &str,
    attempt: i64,
    owner: Option<&str>,
) -> Result<Metadata, String> {
    let mut q=db.prepare("SELECT owner_id,domain,dispatch_key,scan_id,attempt_number,root_run_id,run_id,assignment_id,lease_attempt_id,worker_id,round_number,request_hash,identity_hash FROM native_sdk_log_owners WHERE scan_id=?1 AND attempt_number=?2 AND (?3 IS NULL OR owner_id=?3) ORDER BY created_at,owner_id LIMIT 301").map_err(|_|"native_sdk_log_read_failed")?;
    let values = q
        .query_map(params![scan, attempt, owner], |r| {
            Ok((
                Owner {
                    owner_id: r.get(0)?,
                    domain: r.get(1)?,
                    dispatch_key: r.get(2)?,
                    scan_id: r.get(3)?,
                    attempt: r.get(4)?,
                    root_run_id: r.get(5)?,
                    run_id: r.get(6)?,
                    assignment_id: r.get(7)?,
                    lease_attempt_id: r.get(8)?,
                    worker_id: r.get(9)?,
                    round: r.get(10)?,
                    request_hash: r.get(11)?,
                },
                r.get::<_, String>(12)?,
            ))
        })
        .map_err(|_| "native_sdk_log_read_failed")?;
    let mut owners = Vec::new();
    for value in values {
        let (mut o, identity) = value.map_err(|_| "native_sdk_log_read_failed")?;
        super::owner_mapper_loop::restore_logical(db, &mut o, &identity)?;
        if stable_hash(&serde_json::to_string(&o).map_err(|_| "native_sdk_log_read_failed")?)
            != identity
        {
            return Err("native_sdk_log_owner_unverified".into());
        }
        super::owner_saved::verify(db, &o)?;
        super::transitions::verify_prefix(db, &o.owner_id)?;
        let valid:bool=db.query_row("SELECT COUNT(*)=COALESCE(MAX(ordinal),0) FROM native_sdk_log_rows WHERE owner_id=?1",[&o.owner_id],|r|r.get(0)).map_err(|_|"native_sdk_log_read_failed")?;
        if !valid {
            return Err("native_sdk_log_stage_invalid".into());
        }
        owners.push(o);
    }
    let owners_truncated = owners.len() > 300;
    owners.truncate(300);
    let mut q=db.prepare("SELECT g.owner_id,g.after_ordinal,g.failed_stage,g.code,g.created_at FROM native_sdk_log_gaps g JOIN native_sdk_log_owners o ON o.owner_id=g.owner_id WHERE o.scan_id=?1 AND o.attempt_number=?2 AND (?3 IS NULL OR o.owner_id=?3) ORDER BY g.gap_id LIMIT 301").map_err(|_|"native_sdk_log_read_failed")?;
    let mut gaps = q
        .query_map(params![scan, attempt, owner], |r| {
            Ok(Gap {
                owner_id: r.get(0)?,
                after_ordinal: r.get(1)?,
                failed_stage: r.get(2)?,
                code: r.get(3)?,
                time: r.get(4)?,
            })
        })
        .map_err(|_| "native_sdk_log_read_failed")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "native_sdk_log_read_failed")?;
    for g in &gaps {
        verify_id(db, &g.owner_id)?;
        if g.code != "stage_write_failed"
            || g.after_ordinal < 0
            || g.after_ordinal > 6
            || !matches!(
                g.failed_stage.as_str(),
                "prepared" | "sent" | "response_received" | "cost_saved" | "validated" | "terminal"
            )
        {
            return Err("native_sdk_log_gap_unverified".into());
        }
        let ordinal: i64 = db
            .query_row(
                "SELECT COALESCE(MAX(ordinal),0) FROM native_sdk_log_rows WHERE owner_id=?1",
                [&g.owner_id],
                |r| r.get(0),
            )
            .map_err(|_| "native_sdk_log_read_failed")?;
        if ordinal != g.after_ordinal {
            return Err("native_sdk_log_gap_unverified".into());
        }
    }
    let gaps_truncated = gaps.len() > 300;
    gaps.truncate(300);
    let mut q=db.prepare("SELECT o.owner_id FROM native_sdk_log_owners o WHERE o.scan_id=?1 AND o.attempt_number=?2 AND (?3 IS NULL OR o.owner_id=?3) AND NOT EXISTS(SELECT 1 FROM native_sdk_log_rows r WHERE r.owner_id=o.owner_id AND r.stage='terminal') ORDER BY o.created_at,o.owner_id LIMIT 301").map_err(|_|"native_sdk_log_read_failed")?;
    let mut incomplete_owners = q
        .query_map(params![scan, attempt, owner], |r| r.get(0))
        .map_err(|_| "native_sdk_log_read_failed")?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|_| "native_sdk_log_read_failed")?;
    for id in &incomplete_owners {
        verify_id(db, id)?;
    }
    let incomplete_owners_truncated = incomplete_owners.len() > 300;
    incomplete_owners.truncate(300);
    Ok(Metadata {
        owners,
        owners_truncated,
        gaps,
        gaps_truncated,
        incomplete_owners,
        incomplete_owners_truncated,
    })
}

fn verify_id(db: &Connection, id: &str) -> Result<(), String> {
    let(mut o,identity)=db.query_row("SELECT owner_id,domain,dispatch_key,scan_id,attempt_number,root_run_id,run_id,assignment_id,lease_attempt_id,worker_id,round_number,request_hash,identity_hash FROM native_sdk_log_owners WHERE owner_id=?1",[id],|r|Ok((Owner{owner_id:r.get(0)?,domain:r.get(1)?,dispatch_key:r.get(2)?,scan_id:r.get(3)?,attempt:r.get(4)?,root_run_id:r.get(5)?,run_id:r.get(6)?,assignment_id:r.get(7)?,lease_attempt_id:r.get(8)?,worker_id:r.get(9)?,round:r.get(10)?,request_hash:r.get(11)?},r.get::<_,String>(12)?))).map_err(|_|"native_sdk_log_owner_unverified")?;
    super::owner_mapper_loop::restore_logical(db, &mut o, &identity)?;
    if stable_hash(&serde_json::to_string(&o).map_err(|_| "native_sdk_log_owner_unverified")?)
        != identity
    {
        return Err("native_sdk_log_owner_unverified".into());
    }
    super::owner_saved::verify(db, &o)?;
    super::transitions::verify_prefix(db, id)
}
