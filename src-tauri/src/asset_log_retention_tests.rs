use super::*;
fn fixture() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;
        CREATE TABLE runs(id INTEGER PRIMARY KEY,project_id INTEGER NOT NULL);INSERT INTO runs VALUES(1,10),(2,20);
        CREATE TABLE logs(id INTEGER PRIMARY KEY AUTOINCREMENT,run_id INTEGER REFERENCES runs(id),level TEXT NOT NULL,stage TEXT NOT NULL,message TEXT NOT NULL,created_at TEXT NOT NULL DEFAULT(datetime('now','localtime')));
        CREATE TABLE asset_events(id INTEGER PRIMARY KEY,description TEXT);INSERT INTO asset_events VALUES(1,'manual audit remains');").unwrap();
    connection
}
fn policy() -> LogLimits {
    LogLimits {
        per_run: 3,
        global: 5,
        days: 30,
    }
}
fn count(connection: &Connection) -> i64 {
    connection
        .query_row("SELECT count(*) FROM logs", [], |row| row.get(0))
        .unwrap()
}
#[test]
fn log_retention_caps_repetition_only_after_limit_with_durable_visible_notice() {
    let mut connection = fixture();
    for _ in 0..7 {
        append_with_limits(
            &mut connection,
            1,
            "info",
            "collect",
            "repeated",
            |_| {},
            policy(),
        )
        .unwrap();
    }
    assert_eq!(count(&connection), 3);
    let metadata = retention_metadata(&connection, Some(1), Some(10)).unwrap();
    assert_eq!(metadata["discardedRows"], 4);
    assert_eq!(metadata["repeatedNoiseRows"], 4);
    let rows = read(&connection, Some(1), Some(10), Some(2)).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, 0);
    assert_eq!(rows[0].stage, "retention");
    assert!(rows[0].message.contains("4 rows"));
    for _ in 0..4 {
        read(&connection, None, None, None).unwrap();
    }
    assert_eq!(count(&connection), 3);
    assert!(read(&connection, Some(1), Some(20), None)
        .unwrap()
        .is_empty());
}
#[test]
fn log_retention_global_cap_counts_across_projects_but_not_project_notice_leak() {
    let mut connection = fixture();
    let limits = LogLimits {
        per_run: 5,
        global: 5,
        days: 30,
    };
    for run in [1, 1, 1, 1, 2, 2, 2, 2] {
        append_with_limits(
            &mut connection,
            run,
            "warn",
            "worker",
            "diagnostic",
            |_| {},
            limits,
        )
        .unwrap();
    }
    assert_eq!(count(&connection), 5);
    assert_eq!(
        retention_metadata(&connection, None, None).unwrap()["discardedRows"],
        3
    );
    assert_eq!(
        retention_metadata(&connection, None, Some(10)).unwrap()["discardedRows"],
        3
    );
    assert_eq!(
        retention_metadata(&connection, None, Some(20)).unwrap()["discardedRows"],
        0
    );
    assert!(read(&connection, None, Some(20), None)
        .unwrap()
        .iter()
        .all(|row| row.stage != "retention"));
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM asset_events", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn log_retention_ages_diagnostic_rows_without_deleting_manual_audit_or_guessing_invalid_date() {
    let mut connection = fixture();
    connection.execute_batch("INSERT INTO logs(run_id,level,stage,message,created_at) VALUES
        (1,'error','old','old diagnostic','2000-01-01'),(1,'warn','legacy','date unknown','fixture-time');").unwrap();
    append(&mut connection, 1, "info", "collect", "current", |_| {}).unwrap();
    assert_eq!(count(&connection), 2);
    assert_eq!(
        retention_metadata(&connection, Some(1), None).unwrap()["expiredRows"],
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT description FROM asset_events WHERE id=1",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "manual audit remains"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM logs WHERE stage='legacy'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}
#[test]
fn log_retention_default_real_per_run_limit_keeps_exactly_five_thousand_rows() {
    let mut connection = fixture();
    connection
        .execute_batch(
            "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<5002)
        INSERT INTO logs(run_id,level,stage,message) SELECT 1,'info','collect','bounded' FROM n;",
        )
        .unwrap();
    append(&mut connection, 1, "info", "collect", "latest", |_| {}).unwrap();
    assert_eq!(count(&connection), 5000);
    let metadata = retention_metadata(&connection, None, None).unwrap();
    assert_eq!(metadata["discardedRows"], 3);
    assert_eq!(metadata["globalLimit"], 100000);
    assert_eq!(metadata["ageDays"], 30);
    assert_eq!(metadata["manualAuditTablesPruned"], false);
}
#[test]
fn log_retention_failed_commit_rolls_back_trimming_and_counter_changes() {
    let mut connection = fixture();
    initialize(&connection).unwrap();
    connection
        .execute_batch(
            "INSERT INTO logs(run_id,level,stage,message) VALUES(1,'info','before','original');
        CREATE TABLE pending(id INTEGER REFERENCES runs(id) DEFERRABLE INITIALLY DEFERRED);
        CREATE TRIGGER fail_commit AFTER INSERT ON logs BEGIN INSERT INTO pending VALUES(99);END;",
        )
        .unwrap();
    let mut notified = false;
    assert!(append_with_limits(
        &mut connection,
        1,
        "info",
        "after",
        "not committed",
        |_| notified = true,
        LogLimits {
            per_run: 1,
            global: 1,
            days: 30
        }
    )
    .is_err());
    assert!(!notified);
    assert_eq!(count(&connection), 1);
    assert_eq!(
        retention_metadata(&connection, None, None).unwrap()["discardedRows"],
        0
    );
    assert_eq!(
        connection
            .query_row("SELECT message FROM logs", [], |row| row
                .get::<_, String>(0))
            .unwrap(),
        "original"
    );
}
#[test]
fn log_retention_global_counter_includes_unattributed_legacy_rows_and_survives_run_deletion() {
    let mut connection = fixture();
    connection.execute_batch("INSERT INTO logs(run_id,level,stage,message) VALUES(NULL,'debug','legacy','unattributed'),(1,'info','old','old');").unwrap();
    append_with_limits(
        &mut connection,
        2,
        "info",
        "new",
        "new",
        |_| {},
        LogLimits {
            per_run: 1,
            global: 1,
            days: 30,
        },
    )
    .unwrap();
    assert_eq!(count(&connection), 1);
    assert_eq!(
        retention_metadata(&connection, None, None).unwrap()["discardedRows"],
        2
    );
    connection
        .execute("DELETE FROM runs WHERE id=1", [])
        .unwrap();
    assert_eq!(
        retention_metadata(&connection, None, None).unwrap()["discardedRows"],
        2
    );
    assert_eq!(
        retention_metadata(&connection, None, Some(10)).unwrap()["discardedRows"],
        0
    );
}
