//! Old-schema read-only compatibility boundaries, not a full paid archive fixture.
use super::*;
fn old_schema() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE native_deleted_scan_audits(scan_id TEXT PRIMARY KEY,version INTEGER NOT NULL CHECK(version=1),audit_json TEXT NOT NULL,audit_hash TEXT NOT NULL,deleted_at TEXT NOT NULL);
        CREATE TABLE agent_root_budget_attempts(contract_json TEXT NOT NULL);").unwrap();
    db
}
fn snapshot(db: &Connection) -> Vec<Vec<rusqlite::types::Value>> {
    let mut result = Vec::new();
    for sql in [
        "SELECT type,name,tbl_name,rootpage,sql FROM main.sqlite_master ORDER BY name",
        "SELECT rowid,* FROM native_deleted_scan_audits ORDER BY rowid",
        "SELECT rowid,* FROM agent_root_budget_attempts ORDER BY rowid",
    ] {
        let mut statement = db.prepare(sql).unwrap();
        let count = statement.column_count();
        result.extend(
            statement
                .query_map([], |row| {
                    (0..count)
                        .map(|index| row.get(index))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap(),
        );
    }
    result
}
#[test]
fn deleted_archive_old_schema_without_v2_no_archive_is_readonly() {
    let db = old_schema();
    let before = snapshot(&db);
    let changes = db.total_changes();
    db.pragma_update(None, "query_only", true).unwrap();
    assert!(!verify_deleted(&db, "old-scan").unwrap());
    assert_eq!(db.total_changes(), changes);
    assert_eq!(snapshot(&db), before);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM main.sqlite_master WHERE name='native_deleted_scan_audits_v2'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
#[test]
fn deleted_archive_old_schema_without_v2_rejects_original_bad_payload_without_writes() {
    let db = old_schema();
    db.execute(
        "INSERT INTO native_deleted_scan_audits VALUES('old-scan',1,'{}',?1,'original-stamp')",
        [crate::agent_runtime::store::stable_hash("{}")],
    )
    .unwrap();
    let before = snapshot(&db);
    let changes = db.total_changes();
    db.pragma_update(None, "query_only", true).unwrap();
    assert_eq!(
        verify_deleted(&db, "old-scan").err().as_deref(),
        Some("deleted_audit_payload_invalid")
    );
    assert_eq!(db.total_changes(), changes);
    assert_eq!(snapshot(&db), before);
}
