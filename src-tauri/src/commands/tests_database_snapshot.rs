// Shared rollback assertion: include every non-internal table.
pub(super) fn application_table_snapshot(
    connection: &rusqlite::Connection,
) -> Vec<(String, Vec<String>)> {
    let tables = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    tables
        .into_iter()
        .map(|name| {
            let without_rowid: bool = connection
                .query_row(
                    "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                    [&name],
                    |r| r.get(0),
                )
                .unwrap();
            let quoted = format!("\"{}\"", name.replace('"', "\"\""));
            let sql = if without_rowid {
                let primary_keys: Vec<String> = connection
                    .prepare("SELECT name FROM pragma_table_info(?1,'main') WHERE pk>0 ORDER BY pk")
                    .unwrap()
                    .query_map([&name], |r| r.get::<_, String>(0))
                    .unwrap()
                    .collect::<rusqlite::Result<_>>()
                    .unwrap();
                assert!(
                    !primary_keys.is_empty(),
                    "WITHOUT ROWID table must have its original primary key"
                );
                let order = primary_keys
                    .iter()
                    .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
                    .collect::<Vec<_>>()
                    .join(",");
                format!("SELECT * FROM {quoted} ORDER BY {order}")
            } else {
                format!("SELECT rowid,* FROM {quoted} ORDER BY rowid")
            };
            let mut statement = connection.prepare(&sql).unwrap();
            let count = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    let values = (0..count)
                        .map(|i| row.get::<_, rusqlite::types::Value>(i))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(format!("{values:?}"))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}

fn assert_application_tables_unchanged(
    connection: &rusqlite::Connection,
    before: &[(String, Vec<String>)],
) {
    let after = application_table_snapshot(connection);
    let changed: Vec<_> = before
        .iter()
        .chain(after.iter())
        .filter_map(|(name, _)| {
            (before.iter().find(|(key, _)| key == name)
                != after.iter().find(|(key, _)| key == name))
            .then_some(name)
        })
        .collect();
    assert!(
        changed.is_empty(),
        "operation changed application tables: {changed:?}"
    );
}

#[test]
fn database_snapshot_detects_physical_row_replacement_and_without_rowid_damage() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE ordinary(id TEXT PRIMARY KEY,value TEXT); INSERT INTO ordinary(rowid,id,value) VALUES(1,'same','same'); CREATE TABLE special(id TEXT PRIMARY KEY,value TEXT) WITHOUT ROWID; INSERT INTO special VALUES('one','keep');").unwrap();
    let before = application_table_snapshot(&db);
    assert_eq!(application_table_snapshot(&db), before);
    let tx = db.unchecked_transaction().unwrap();
    tx.execute_batch(
        "DELETE FROM ordinary; INSERT INTO ordinary(rowid,id,value) VALUES(2,'same','same');",
    )
    .unwrap();
    assert_ne!(application_table_snapshot(&tx), before);
    drop(tx);
    assert_eq!(application_table_snapshot(&db), before);
    let tx = db.unchecked_transaction().unwrap();
    tx.execute("UPDATE special SET value='changed'", [])
        .unwrap();
    assert_ne!(application_table_snapshot(&tx), before);
    drop(tx);
    assert_eq!(application_table_snapshot(&db), before);
}
