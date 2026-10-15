//! Actual SDK diagnostics writer/reader against the full application schema.
use super::{owner::Owner, persist, replay};
use crate::agent_runtime::multi_agent::budget::root::model::RootModelCall;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;
mod reads;
mod writes;
pub(crate) use reads::read_contract;
pub(crate) use writes::write_contract;
fn original(db: &Connection, root: &str) -> Owner {
    Owner::root(db, root, 1).unwrap()
}
fn snapshot(db: &Connection) -> Vec<(String, Vec<String>)> {
    let names=db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
  .query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    names
        .into_iter()
        .map(|name| {
            let without_rowid: bool = db
                .query_row(
                    "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                    [&name],
                    |row| row.get(0),
                )
                .unwrap();
            let quote = |identifier: &str| format!("\"{}\"", identifier.replace('"', "\"\""));
            let table = quote(&name);
            let sql = if without_rowid {
                let keys: Vec<String> = db
                    .prepare("SELECT name FROM pragma_table_info(?1,'main') WHERE pk>0 ORDER BY pk")
                    .unwrap()
                    .query_map([&name], |row| row.get(0))
                    .unwrap()
                    .collect::<rusqlite::Result<_>>()
                    .unwrap();
                assert!(!keys.is_empty(), "WITHOUT ROWID table requires its original primary key");
                let order = keys.iter().map(|key| quote(key)).collect::<Vec<_>>().join(",");
                format!("SELECT * FROM {table} ORDER BY {order}")
            } else {
                // Physical replacement must remain visible even when all payloads match.
                format!("SELECT rowid,* FROM {table} ORDER BY rowid")
            };
            let mut q = db.prepare(&sql).unwrap();
            let n = q.column_count();
            let rows = q
                .query_map([], |r| {
                    Ok(format!(
                        "{:?}",
                        (0..n)
                            .map(|i| r.get::<_, rusqlite::types::Value>(i))
                            .collect::<Result<Vec<_>, _>>()?
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

#[test]
fn snapshot_preserves_row_identity_and_quoted_composite_primary_keys() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE ordinary(id TEXT PRIMARY KEY,value TEXT);
         INSERT INTO ordinary(rowid,id,value) VALUES(1,'same','same');
         CREATE TABLE \"quoted\"\"table\"(\"first\"\"key\" TEXT,second TEXT,value TEXT,PRIMARY KEY(\"first\"\"key\",second)) WITHOUT ROWID;
         INSERT INTO \"quoted\"\"table\" VALUES('z','a','last'),('a','z','first');",
    ).unwrap();
    let before = snapshot(&db);
    assert_eq!(snapshot(&db), before);
    let special = &before.iter().find(|(name, _)| name == "quoted\"table").unwrap().1;
    assert!(special[0].contains("first"));
    assert!(special[1].contains("last"));
    let tx = db.unchecked_transaction().unwrap();
    tx.execute_batch("DELETE FROM ordinary; INSERT INTO ordinary(rowid,id,value) VALUES(2,'same','same');").unwrap();
    assert_ne!(snapshot(&tx), before);
    drop(tx);
    assert_eq!(snapshot(&db), before);
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("UPDATE \"quoted\"\"table\" SET value='changed' WHERE \"first\"\"key\"='z' AND second='a'", []).unwrap();
    assert_ne!(snapshot(&tx), before);
    drop(tx);
    assert_eq!(snapshot(&db), before);
}
fn stages(db: &Connection, id: &str) -> Vec<String> {
    db.prepare("SELECT stage FROM native_sdk_log_rows WHERE owner_id=?1 ORDER BY ordinal")
        .unwrap()
        .query_map([id], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}
fn start(path: &Path, root: &str) -> (Connection, Owner) {
    let mut db = persist::connection(path).unwrap();
    let owner = original(&db, root);
    persist::begin(&mut db, &owner).unwrap();
    // SDK 02 had a non-atomic first stage. This setup preserves its normal prefix;
    // the dedicated atomic-begin negative never performs this fallback.
    if stages(&db, &owner.owner_id).is_empty() {
        persist::append(&mut db, &owner, "prepared", "", "").unwrap();
    }
    (db, owner)
}
fn read(path: &Path, scan: &str, attempt: i64, cursor: Option<i64>, after: i64) -> Value {
    serde_json::to_value(replay::read(path, scan, attempt, cursor, None, after, 300).unwrap())
        .unwrap()
}
