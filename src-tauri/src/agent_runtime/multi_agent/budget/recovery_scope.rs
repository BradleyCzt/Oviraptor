//! Private primitives for a future dedicated Recovery birth. Tests are ledger
//! evidence only; no production factory can yet issue these private proofs.
use rusqlite::{params, Transaction};
const SCHEMA: &str = include_str!("recovery_scope.schema.sql");
const APPROVED_USD: &str = "approved_usd_nanos";
struct VerifiedScopeCharge {
    owner: String,
    root: String,
    ticket: String,
    original_source_hash: String,
    reserved: [i64; 10],
    approved_usd_nanos: i64,
}
struct OriginalScopeTerminal {
    original_source_hash: String,
    known: Option<([i64; 10], i64)>,
}
fn dimensions() -> impl Iterator<Item = &'static str> {
    super::DIMENSIONS.into_iter().chain([APPROVED_USD])
}
fn amounts(values: [i64; 10], usd: i64) -> [i64; 11] {
    let mut all = [0; 11];
    all[..10].copy_from_slice(&values);
    all[10] = usd;
    all
}
fn total(tx: &Transaction<'_>, owner: &str, dimension: &str) -> Result<(i64, i64, i64), String> {
    let mut q=tx.prepare("SELECT e.kind,e.amount FROM main.agent_recovery_scope_entries e JOIN main.agent_recovery_scope_tickets t ON t.ticket_id=e.ticket_id WHERE t.owner_id=?1 AND e.dimension=?2 ORDER BY e.rowid").map_err(|e|e.to_string())?;
    let rows = q
        .query_map(params![owner, dimension], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let (mut reserved, mut consumed, mut unknown) = (0i64, 0i64, 0i64);
    for row in rows {
        let (kind, n) = row.map_err(|e| e.to_string())?;
        match kind.as_str() {
            "reserve" => reserved = reserved.checked_add(n).ok_or("recovery_scope_overflow")?,
            "release" => {
                reserved = reserved
                    .checked_sub(n)
                    .filter(|v| *v >= 0)
                    .ok_or("recovery_scope_conservation_invalid")?
            }
            "consume" => consumed = consumed.checked_add(n).ok_or("recovery_scope_overflow")?,
            "forfeit" => unknown = unknown.checked_add(n).ok_or("recovery_scope_overflow")?,
            _ => return Err("recovery_scope_entry_invalid".into()),
        }
    }
    Ok((reserved, consumed, unknown))
}
fn limits(tx: &Transaction<'_>, owner: &str) -> Result<[Option<i64>; 11], String> {
    let (text,usd):(String,i64)=tx.query_row("SELECT limits_json,max_approved_usd_nanos FROM main.agent_recovery_scope_owners WHERE owner_id=?1",[owner],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    let original: [Option<i64>; 10] = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut result = [None; 11];
    result[..10].copy_from_slice(&original);
    result[10] = Some(usd);
    Ok(result)
}
fn reserve_inner(tx: &Transaction<'_>, proof: &VerifiedScopeCharge) -> Result<(), String> {
    let bound:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_recovery_scope_root_bindings WHERE owner_id=?1 AND root_run_id=?2) AND NOT EXISTS(SELECT 1 FROM main.agent_recovery_scope_stops WHERE owner_id=?1)",params![proof.owner,proof.root],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound {
        return Err("recovery_scope_root_unbound_or_stopped".into());
    }
    let caps = limits(tx, &proof.owner)?;
    let values = amounts(proof.reserved, proof.approved_usd_nanos);
    for ((dimension, n), cap) in dimensions().zip(values).zip(caps) {
        if n < 0 {
            return Err("recovery_scope_negative_charge".into());
        }
        let (r, c, u) = total(tx, &proof.owner, dimension)?;
        if u != 0 {
            return Err("recovery_scope_unknown_cost".into());
        }
        let sum = r
            .checked_add(c)
            .and_then(|v| v.checked_add(n))
            .ok_or("recovery_scope_overflow")?;
        if cap.is_some_and(|v| v < 0 || sum > v) {
            return Err("recovery_scope_limit_exceeded".into());
        }
    }
    tx.execute(
        "INSERT INTO main.agent_recovery_scope_tickets VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            proof.ticket,
            proof.owner,
            proof.root,
            proof.original_source_hash,
            serde_json::to_string(&proof.reserved).map_err(|e| e.to_string())?,
            proof.approved_usd_nanos
        ],
    )
    .map_err(|e| e.to_string())?;
    for (dimension, n) in dimensions().zip(values) {
        append(
            tx,
            proof,
            dimension,
            "reserve",
            n,
            &proof.original_source_hash,
        )?;
    }
    Ok(())
}
fn append(
    tx: &Transaction<'_>,
    p: &VerifiedScopeCharge,
    dimension: &str,
    kind: &str,
    n: i64,
    source: &str,
) -> Result<(), String> {
    let changed = tx
        .execute(
            "INSERT INTO main.agent_recovery_scope_entries VALUES(?1,?2,?3,?4,?5)",
            params![p.ticket, dimension, kind, n, source],
        )
        .map_err(|e| e.to_string())?;
    if changed != 1 {
        return Err("recovery_scope_entry_unconfirmed".into());
    }
    Ok(())
}
fn settle_inner(
    tx: &Transaction<'_>,
    p: &VerifiedScopeCharge,
    receipt: &OriginalScopeTerminal,
) -> Result<(), String> {
    let row:(String,String,String,String,i64)=tx.query_row("SELECT owner_id,root_run_id,original_source_hash,reserved_json,reserved_approved_usd_nanos FROM main.agent_recovery_scope_tickets WHERE ticket_id=?1",[&p.ticket],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(|e|e.to_string())?;
    if row
        != (
            p.owner.clone(),
            p.root.clone(),
            p.original_source_hash.clone(),
            serde_json::to_string(&p.reserved).map_err(|e| e.to_string())?,
            p.approved_usd_nanos,
        )
    {
        return Err("recovery_scope_original_ticket_changed".into());
    }
    let reserved = amounts(p.reserved, p.approved_usd_nanos);
    let actual = receipt
        .known
        .map(|(v, u)| amounts(v, u))
        .unwrap_or(reserved);
    if actual.iter().any(|v| *v < 0) {
        return Err("recovery_scope_negative_charge".into());
    }
    tx.execute(
        "INSERT INTO main.agent_recovery_scope_terminals VALUES(?1,?2,?3)",
        params![
            p.ticket,
            if receipt.known.is_some() {
                "known"
            } else {
                "unknown"
            },
            receipt.original_source_hash
        ],
    )
    .map_err(|e| e.to_string())?;
    for ((dimension, r), n) in dimensions().zip(reserved).zip(actual) {
        append(
            tx,
            p,
            dimension,
            "release",
            r,
            &receipt.original_source_hash,
        )?;
        append(
            tx,
            p,
            dimension,
            if receipt.known.is_some() {
                "consume"
            } else {
                "forfeit"
            },
            n,
            &receipt.original_source_hash,
        )?;
    }
    let caps = limits(tx, &p.owner)?;
    let mut exceeded = false;
    for (dimension, cap) in dimensions().zip(caps) {
        let (r, c, u) = total(tx, &p.owner, dimension)?;
        let sum = r
            .checked_add(c)
            .and_then(|v| v.checked_add(u))
            .ok_or("recovery_scope_overflow")?;
        exceeded |= cap.is_some_and(|limit| sum > limit);
    }
    if receipt.known.is_none() || exceeded {
        tx.execute(
            "INSERT INTO main.agent_recovery_scope_stops VALUES(?1,?2,?3)",
            params![
                p.owner,
                p.ticket,
                if receipt.known.is_none() {
                    "unknown_cost"
                } else {
                    "approved_limit_exceeded"
                }
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn atomic(tx: &Transaction<'_>, action: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    tx.execute_batch("SAVEPOINT recovery_scope_primitive")
        .map_err(|e| e.to_string())?;
    match action() {
        Ok(()) => tx
            .execute_batch("RELEASE recovery_scope_primitive")
            .map_err(|e| e.to_string()),
        Err(e) => {
            tx.execute_batch(
                "ROLLBACK TO recovery_scope_primitive; RELEASE recovery_scope_primitive",
            )
            .map_err(|r| format!("{e};recovery_scope_rollback:{r}"))?;
            Err(e)
        }
    }
}
fn reserve(tx: &Transaction<'_>, p: &VerifiedScopeCharge) -> Result<(), String> {
    atomic(tx, || reserve_inner(tx, p))
}
fn settle(
    tx: &Transaction<'_>,
    p: &VerifiedScopeCharge,
    r: &OriginalScopeTerminal,
) -> Result<(), String> {
    atomic(tx, || settle_inner(tx, p, r))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn setup(tx: &Transaction<'_>) {
        tx.execute_batch(SCHEMA).unwrap();
        let caps = [Some(10); 10];
        tx.execute("INSERT INTO agent_recovery_scope_owners VALUES('owner','scan','private-test-contract','testhash',?1,10)",[serde_json::to_string(&caps).unwrap()]).unwrap();
        for root in ["root-a", "root-b"] {
            tx.execute("INSERT INTO agent_recovery_scope_root_bindings VALUES(?1,'owner','original-test-birth')",[root]).unwrap();
        }
    }
    fn charge(root: &str, ticket: &str, n: i64) -> VerifiedScopeCharge {
        VerifiedScopeCharge {
            owner: "owner".into(),
            root: root.into(),
            ticket: ticket.into(),
            original_source_hash: format!("private-test-source-{ticket}"),
            reserved: [n; 10],
            approved_usd_nanos: n,
        }
    }
    #[test]
    fn recovery_scope_private_primitive_two_roots_share_limits_and_dedup_is_atomic() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let tx = db.unchecked_transaction().unwrap();
        setup(&tx);
        let a = charge("root-a", "a", 6);
        reserve(&tx, &a).unwrap();
        let before: i64 = tx
            .query_row(
                "SELECT count(*) FROM agent_recovery_scope_entries",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            reserve(&tx, &charge("root-b", "b", 5)).unwrap_err(),
            "recovery_scope_limit_exceeded"
        );
        assert!(reserve(&tx, &a).is_err());
        assert_eq!(
            tx.query_row(
                "SELECT count(*) FROM agent_recovery_scope_entries",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            before
        );
        settle(
            &tx,
            &a,
            &OriginalScopeTerminal {
                original_source_hash: "private-known-invoice".into(),
                known: Some(([2; 10], 2)),
            },
        )
        .unwrap();
        reserve(&tx, &charge("root-b", "b", 8)).unwrap();
        for d in dimensions() {
            assert_eq!(total(&tx, "owner", d).unwrap(), (8, 2, 0));
        }
    }
    #[test]
    fn recovery_scope_private_primitive_unknown_and_actual_overrun_preserve_fees_and_stop() {
        for unknown in [true, false] {
            let db = rusqlite::Connection::open_in_memory().unwrap();
            let tx = db.unchecked_transaction().unwrap();
            setup(&tx);
            let p = charge("root-a", "a", 6);
            reserve(&tx, &p).unwrap();
            settle(
                &tx,
                &p,
                &OriginalScopeTerminal {
                    original_source_hash: "private-original-terminal".into(),
                    known: if unknown { None } else { Some(([12; 10], 12)) },
                },
            )
            .unwrap();
            for d in dimensions() {
                assert_eq!(
                    total(&tx, "owner", d).unwrap(),
                    if unknown { (0, 0, 6) } else { (0, 12, 0) }
                );
            }
            assert_eq!(
                reserve(&tx, &charge("root-b", "b", 1)).unwrap_err(),
                "recovery_scope_root_unbound_or_stopped"
            );
            assert!(settle(
                &tx,
                &p,
                &OriginalScopeTerminal {
                    original_source_hash: "duplicate".into(),
                    known: Some(([0; 10], 0))
                }
            )
            .is_err());
        }
    }
}
