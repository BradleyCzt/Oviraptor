//! Structural lossless arena tests, not signed worker or paid-source fixtures.
use super::rows::{Cell, RowIdentity, Table};
use rusqlite::{params, Connection};

#[test]
fn deleted_audit_row_identity_legacy_v1_bytes_and_hash_are_unchanged() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE legacy_archive_rows(id TEXT,payload BLOB); INSERT INTO legacy_archive_rows(rowid,id,payload) VALUES(7,'one',X'00FF');").unwrap();
    let table = Table::read(&db, "legacy_archive_rows", "?1 IS NOT NULL", "scope").unwrap();
    let original = r#"{"name":"legacy_archive_rows","columns":["rowid","id","payload"],"rows":[[{"type":"Integer","value":7},{"type":"Text","value":"one"},{"type":"Blob","value":[0,255]}]]}"#;
    assert_eq!(table.canonical().unwrap(), original);
    assert_eq!(
        crate::agent_runtime::store::stable_hash(&table.canonical().unwrap()),
        crate::agent_runtime::store::stable_hash(original)
    );
    let historical: Table = serde_json::from_str(original).unwrap();
    assert!(historical.identity.is_none());
    let arena = Connection::open_in_memory().unwrap();
    historical.restore(&arena).unwrap();
    assert_eq!(
        Table::read(&arena, "legacy_archive_rows", "?1 IS NOT NULL", "scope")
            .unwrap()
            .canonical()
            .unwrap(),
        original
    );
}

fn structural_original_business_receipt() -> Table {
    // This is only the trusted table shape. It does not claim a valid HMAC,
    // original worker, provider invoice or executable dispatch publication.
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!(
        "../../commands/agent_native/business_logic_dispatch_receipt.sql"
    ))
    .unwrap();
    db.execute("INSERT INTO agent_business_logic_dispatch_receipts(receipt_id,root_run_id,assignment_id,child_run_id,worker_attempt_id,event_sequence,fact_json,fact_hash,signature) VALUES(?1,'root','assignment','child','worker',1,'{}',?2,?3)",
        params!["a".repeat(64),"b".repeat(64),vec![0u8;32]]).unwrap();
    Table::read(
        &db,
        "agent_business_logic_dispatch_receipts",
        "?1 IS NOT NULL",
        "scope",
    )
    .unwrap()
}

#[test]
fn deleted_audit_row_identity_trusted_wr_restores_all_columns_and_signature_bytes() {
    let table = structural_original_business_receipt();
    assert!(
        matches!(&table.identity, Some(RowIdentity::PrimaryKey { columns }) if columns==&["receipt_id"])
    );
    assert!(!table.columns.iter().any(|c| c == "rowid"));
    let arena = Connection::open_in_memory().unwrap();
    table.restore(&arena).unwrap();
    let restored = Table::read(&arena, &table.name, "?1 IS NOT NULL", "scope").unwrap();
    assert_eq!(restored.canonical().unwrap(), table.canonical().unwrap());
    assert_eq!(
        arena
            .query_row(
                "SELECT signature FROM agent_business_logic_dispatch_receipts",
                [],
                |r| r.get::<_, Vec<u8>>(0)
            )
            .unwrap(),
        vec![0; 32]
    );
}

#[test]
fn deleted_audit_row_identity_composite_pk_read_and_outside_protection_are_exact() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE agent_composite_archive_fixture(province TEXT,company TEXT,payload BLOB,PRIMARY KEY(province,company)) WITHOUT ROWID; INSERT INTO agent_composite_archive_fixture VALUES('A','x',X'00FF'),('A','y',X'0102'),('B','x',X'03');").unwrap();
    let selected = Table::read(
        &db,
        "agent_composite_archive_fixture",
        "province=CAST(X'41' AS TEXT) AND ?1 IS NOT NULL",
        "scope",
    )
    .unwrap();
    assert!(
        matches!(&selected.identity, Some(RowIdentity::PrimaryKey { columns }) if columns==&["province","company"])
    );
    assert_eq!(selected.rows.len(), 2);
    let exact = Table::read(
        &db,
        &selected.name,
        &selected.original_filter().unwrap(),
        "scope",
    )
    .unwrap();
    assert_eq!(exact.canonical().unwrap(), selected.canonical().unwrap());
    let outside = Table::read(
        &db,
        &selected.name,
        &selected.outside_filter().unwrap(),
        "scope",
    )
    .unwrap();
    assert_eq!(outside.rows.len(), 1);
    assert!(
        outside.rows[0]
            == vec![
                Cell::Text("B".into()),
                Cell::Text("x".into()),
                Cell::Blob(vec![3])
            ]
    );
    db.execute(
        "UPDATE agent_composite_archive_fixture SET payload=X'99' WHERE province='B'",
        [],
    )
    .unwrap();
    let changed = Table::read(
        &db,
        &selected.name,
        &selected.outside_filter().unwrap(),
        "scope",
    )
    .unwrap();
    assert_ne!(changed.canonical().unwrap(), outside.canonical().unwrap());
    assert_eq!(
        selected
            .restore(&Connection::open_in_memory().unwrap())
            .unwrap_err(),
        "deleted_audit_primary_key_schema_unknown"
    );
}

#[test]
fn deleted_audit_row_identity_forged_pk_duplicate_null_and_column_drift_are_refused() {
    let original = structural_original_business_receipt();
    let mut forged = original.clone();
    forged.identity = Some(RowIdentity::PrimaryKey {
        columns: vec!["assignment_id".into()],
    });
    assert_eq!(
        forged
            .restore(&Connection::open_in_memory().unwrap())
            .unwrap_err(),
        "deleted_audit_original_schema_identity_changed"
    );
    let mut repeated = original.clone();
    repeated.rows.push(repeated.rows[0].clone());
    assert_eq!(
        repeated.original_filter().unwrap_err(),
        "deleted_audit_primary_key_duplicate"
    );
    let mut missing = original.clone();
    missing.rows[0][0] = Cell::Null;
    assert_eq!(
        missing
            .restore(&Connection::open_in_memory().unwrap())
            .unwrap_err(),
        "deleted_audit_primary_key_null_or_invalid"
    );
    let mut drift = original;
    drift.columns.push("invented_column".into());
    drift.rows[0].push(Cell::Text("extra".into()));
    assert_eq!(
        drift
            .restore(&Connection::open_in_memory().unwrap())
            .unwrap_err(),
        "deleted_audit_original_schema_identity_changed"
    );
}
