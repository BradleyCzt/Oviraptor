// Preserve every pre-existing physical row and every other Root/dimension.
// Only this captured original Root may append one wall pair or elapsed fact.
type SourceExitSnapshot = Vec<(String, Vec<Vec<rusqlite::types::Value>>)>;
fn source_exit_snapshot(db: &rusqlite::Connection) -> SourceExitSnapshot {
    let names = db
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
            let without_rowid: bool = db
                .query_row(
                    "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1",
                    [&name],
                    |r| r.get(0),
                )
                .unwrap();
            let sql = if without_rowid {
                // Retain every original column. These immutable receipt tables
                // have no rowid; their declared composite PK defines stable order.
                let keys = db
                    .prepare("SELECT name FROM pragma_table_info(?1) WHERE pk>0 ORDER BY pk")
                    .unwrap()
                    .query_map([&name], |r| r.get::<_, String>(0))
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                assert!(
                    !keys.is_empty(),
                    "WITHOUT ROWID table missing original PK: {name}"
                );
                format!(
                    "SELECT * FROM {} ORDER BY {}",
                    quote(&name),
                    keys.iter()
                        .map(|key| quote(key))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            } else {
                // Keep the original rowid prefix and index positions used by
                // financial append assertions below, rather than reshaping them.
                format!("SELECT rowid,* FROM {} ORDER BY rowid", quote(&name))
            };
            let mut query = db.prepare(&sql).unwrap();
            let count = query.column_count();
            let rows = query
                .query_map([], |r| {
                    (0..count)
                        .map(|i| r.get(i))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            (name, rows)
        })
        .collect()
}
fn source_exit_assert_snapshot(
    db: &rusqlite::Connection,
    c: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    before: &SourceExitSnapshot,
) {
    use rusqlite::types::Value;
    let after = source_exit_snapshot(db);
    assert_eq!(before.len(), after.len());
    let control: String = db
        .query_row(
            "SELECT id FROM agent_root_budget_attempts WHERE root_run_id=?1",
            [&c.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    for ((name, old), (actual, new)) in before.iter().zip(after.iter()) {
        assert_eq!(name, actual);
        assert!(
            new.len() >= old.len() && &new[..old.len()] == old.as_slice(),
            "original physical rows changed: {name}"
        );
        let added = &new[old.len()..];
        match name.as_str() {
            "agent_budget_entries" => {
                assert!(
                    added.is_empty() || added.len() == 2,
                    "unexpected financial append count"
                );
                for row in added {
                    assert_eq!(&row[2], &Value::Text(c.root_run_id.clone()));
                    assert_eq!(&row[3], &Value::Text(String::new()));
                    assert_eq!(&row[4], &Value::Text(control.clone()));
                    assert_eq!(&row[5], &Value::Text("wall_time_ms".into()));
                }
                if !added.is_empty() {
                    assert_eq!(&added[0][6], &Value::Text("reserve".into()));
                    assert_eq!(&added[1][6], &Value::Text("consume".into()));
                    assert_eq!(added[0][7], added[1][7]);
                    assert_eq!(added[0][9], added[1][9]);
                    let Value::Integer(amount) = added[0][7] else {
                        panic!("wall amount")
                    };
                    assert!(amount > 0);
                    let Value::Text(source) = &added[0][9] else {
                        panic!("wall source")
                    };
                    assert!(source.starts_with("root-clock:"));
                    assert_eq!(
                        added[0][8],
                        Value::Text(format!("root:{control}:{source}:reserve"))
                    );
                    assert_eq!(
                        added[1][8],
                        Value::Text(format!("root:{control}:{source}:consume"))
                    );
                }
            }
            "agent_root_elapsed_facts" => {
                assert!(added.len() <= 1, "multiple exceptional observations");
                if !added.is_empty() {
                    assert_eq!(&added[0][2], &Value::Text(c.root_run_id.clone()));
                    let fact =
                        crate::agent_runtime::multi_agent::budget::clock::elapsed_fact::read(
                            db,
                            &c.root_run_id,
                        )
                        .unwrap()
                        .unwrap();
                    source_exit_require_fact(db, c, fact.exhausted);
                    assert_eq!(fact.owner_id, control);
                }
            }
            _ => assert!(added.is_empty(), "business/authority rows appended: {name}"),
        }
    }
}
fn source_exit_wall(db: &rusqlite::Connection, root: &str) -> i64 {
    crate::agent_runtime::multi_agent::budget::balance(db, root, None, "wall_time_ms")
        .unwrap()
        .consumed
}
fn source_exit_require_fact(
    db: &rusqlite::Connection,
    c: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    exhausted: bool,
) {
    let f =
        crate::agent_runtime::multi_agent::budget::clock::elapsed_fact::read(db, &c.root_run_id)
            .unwrap()
            .expect("original Source exit must persist exceptional elapsed");
    assert_eq!(
        (
            f.root.as_str(),
            f.scan.as_str(),
            f.attempt,
            f.target.as_str(),
            f.epoch,
            f.fence.as_str()
        ),
        (
            c.root_run_id.as_str(),
            c.scan_id.as_str(),
            c.attempt_number,
            c.target_key.as_str(),
            c.lease_epoch,
            c.fencing_token.as_str()
        )
    );
    assert_eq!(f.exhausted, exhausted);
    assert_eq!(f.elapsed, f.journaled + f.unsettled);
    assert_eq!(f.owner_kind, "root_control");
    assert!(f.assignment.is_empty());
    assert!(Uuid::parse_str(&f.owner_id).is_ok());
    assert!(f.elapsed >= source_exit_wall(db, &c.root_run_id));
}
fn source_exit_paid_rows(db: &rusqlite::Connection, root: &str) -> Vec<(String, Vec<String>)> {
    ["agent_specialist_calls","agent_source_model_rounds","agent_messages","agent_source_tool_receipts"].into_iter().map(|table| {
        let mut q=db.prepare(&format!("SELECT rowid,* FROM {table} WHERE {} ORDER BY rowid",if table=="agent_source_tool_receipts" {"assignment_id IN (SELECT id FROM agent_assignments WHERE coordinator_run_id=?1)"} else {"root_run_id=?1"})).unwrap();
        let columns=q.column_count();let rows=q.query_map([root],|r|Ok(format!("{:?}",(0..columns).map(|i|r.get::<_,rusqlite::types::Value>(i)).collect::<rusqlite::Result<Vec<_>>>()?))).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();(table.into(),rows)
    }).collect()
}
