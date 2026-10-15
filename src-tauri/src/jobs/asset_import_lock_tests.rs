use super::*;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use std::sync::mpsc;

#[test]
fn asset_import_exhausted_write_wait_leaves_database_and_source_unchanged() {
    let mut f = Fixture::new();
    let path = f.root.join("blocked.csv");
    let csv = "company,asset_key,link,protocol\n集团,http://blocked.example/,http://blocked.example/,http\n";
    fs::write(&path, csv).unwrap();
    let mut other = db::open(Path::new(f.connection.path().unwrap())).unwrap();
    let tx = other
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    f.connection
        .busy_timeout(Duration::from_millis(30))
        .unwrap();
    let result = import_csv_with_progress(&mut f.connection, 9001, 9001, &path, |_| {
        panic!("no progress before write admission")
    });
    assert!(result.unwrap_err().contains("database is locked"));
    assert!(f.connection.is_autocommit());
    assert_eq!(
        f.connection
            .query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), csv);
    tx.rollback().unwrap();
    assert_eq!(
        import_csv_with_progress(&mut f.connection, 9001, 9001, &path, |_| {})
            .unwrap()
            .inserted,
        1
    );
}

// Start the competing commit after import_title_rules has read the snapshot,
// but before the importer writes its first asset. The production importer,
// not a SQL facsimile, must survive this WAL read-to-write race.
#[test]
fn asset_import_survives_writer_between_snapshot_and_first_asset() {
    let mut f = Fixture::new();
    let db_path = PathBuf::from(f.connection.path().unwrap());
    let (start_tx, start_rx) = mpsc::channel();
    let (attempt_tx, attempt_rx) = mpsc::channel();
    let (finish_tx, finish_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let other = db::open(&db_path).unwrap();
        other.busy_timeout(Duration::from_millis(50)).unwrap();
        start_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let attempt = other.execute("UPDATE projects SET name='并发修改' WHERE id=9001", []);
        let blocked = match attempt {
            Ok(1) => false,
            Err(rusqlite::Error::SqliteFailure(code, _))
                if code.code == rusqlite::ErrorCode::DatabaseBusy =>
            {
                true
            }
            other => panic!("unexpected competing write: {other:?}"),
        };
        attempt_tx.send(()).unwrap();
        finish_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        if blocked {
            other.busy_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(
                other
                    .execute("UPDATE projects SET name='并发修改' WHERE id=9001", [])
                    .unwrap(),
                1
            );
        }
    });
    let mut triggered = false;
    f.connection
        .authorizer(Some(move |context: AuthContext<'_>| {
            if !triggered
                && matches!(
                    context.action,
                    AuthAction::Read {
                        table_name: "assets",
                        ..
                    }
                )
            {
                triggered = true;
                start_tx.send(()).unwrap();
                attempt_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            Authorization::Allow
        }))
        .unwrap();
    let path = f.root.join("concurrent.csv");
    fs::write(&path, "company,asset_key,link,protocol\n集团,http://concurrent.example/,http://concurrent.example/,http\n").unwrap();
    let result = import_csv_with_progress(&mut f.connection, 9001, 9001, &path, |_| {});
    f.connection
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    finish_tx.send(()).unwrap();
    writer.join().unwrap();
    let result = result.expect("real asset importer must not fail with SQLITE_BUSY_SNAPSHOT");
    assert_eq!(result.inserted, 1);
    assert_eq!(
        f.connection
            .query_row("SELECT name FROM projects WHERE id=9001", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "并发修改"
    );
    assert_eq!(
        f.connection
            .query_row(
                "SELECT count(*) FROM project_assets WHERE project_id=9001",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn asset_import_waits_for_existing_writer_and_keeps_manual_state_on_reentry() {
    let mut f = Fixture::new();
    f.import(
        9001,
        &[("集团", "http://existing.example/", "web_alive", "", "[]")],
    );
    f.connection.execute("UPDATE project_assets SET decision='confirmed',note='人工备注',entry_opened_at='original-opened'", []).unwrap();
    let db_path = PathBuf::from(f.connection.path().unwrap());
    let (held_tx, held_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        let mut other = db::open(&db_path).unwrap();
        let tx = other
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        tx.execute("UPDATE projects SET name='并发修改' WHERE id=9001", [])
            .unwrap();
        held_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        tx.commit().unwrap();
    });
    held_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let result = f.import(
        9002,
        &[("集团", "http://existing.example/", "web_alive", "", "[]")],
    );
    writer.join().unwrap();
    assert_eq!(result.inserted, 0);
    let state: (i64,String,String,String) = f.connection.query_row("SELECT (SELECT count(*) FROM assets),decision,note,entry_opened_at FROM project_assets", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        state,
        (
            1,
            "confirmed".into(),
            "人工备注".into(),
            "original-opened".into()
        )
    );
}

#[test]
fn asset_import_failure_rolls_back_entire_file_and_can_reenter() {
    let mut f = Fixture::new();
    f.import(
        9001,
        &[("集团", "http://existing.example/", "web_alive", "", "[]")],
    );
    f.connection.execute("UPDATE project_assets SET decision='confirmed',note='人工备注',entry_opened_at='original-opened'", []).unwrap();
    let path = f.root.join("batch.csv");
    let mut csv = String::from("company,asset_key,link,protocol\n");
    for id in 0..2002 {
        csv.push_str(&format!(
            "集团,http://batch-{id}.example/,http://batch-{id}.example/,http\n"
        ));
    }
    fs::write(&path, &csv).unwrap();
    f.connection.execute_batch("CREATE TRIGGER fail_last_asset BEFORE INSERT ON assets WHEN NEW.link='http://batch-2001.example/' BEGIN SELECT RAISE(ABORT,'fixture_late_failure'); END;").unwrap();
    let mut progress = Vec::new();
    let error =
        import_csv_with_progress(&mut f.connection, 9002, 9001, &path, |n| progress.push(n))
            .unwrap_err();
    assert!(error.contains("fixture_late_failure"), "{error}");
    assert!(
        !progress.is_empty(),
        "failure must occur after a progress event"
    );
    assert!(f.connection.is_autocommit());
    let counts: (i64,i64,i64) = f.connection.query_row("SELECT (SELECT count(*) FROM assets),(SELECT count(*) FROM project_assets),(SELECT count(*) FROM asset_events WHERE run_id=9002)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(counts, (1, 1, 0));
    assert_eq!(fs::read_to_string(&path).unwrap(), csv);
    f.connection
        .execute_batch("DROP TRIGGER fail_last_asset;")
        .unwrap();
    let result = import_csv_with_progress(&mut f.connection, 9002, 9001, &path, |_| {}).unwrap();
    assert_eq!(result.inserted, 2002);
    let repeat = import_csv_with_progress(&mut f.connection, 9002, 9001, &path, |_| {}).unwrap();
    assert_eq!(repeat.inserted, 0);
    assert_eq!(
        f.connection
            .query_row("SELECT count(*) FROM assets", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2003
    );
    let state: (String, String, String) = f
        .connection
        .query_row(
            "SELECT decision,note,entry_opened_at FROM project_assets WHERE decision='confirmed'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        state,
        (
            "confirmed".into(),
            "人工备注".into(),
            "original-opened".into()
        )
    );
}
