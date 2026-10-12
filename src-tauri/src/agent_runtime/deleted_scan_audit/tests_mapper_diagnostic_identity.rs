//! Real compiled sidecar DDL and lossless private arenas, not paid SDK proofs.
use super::{
    rows::{Cell, RowIdentity, Table},
    Bundle,
};
use rusqlite::{params, Connection};

fn original_diagnostic_tables(with_row: bool) -> (Table, Table) {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!("../model/diagnostics/schema.sql"))
        .unwrap();
    if with_row {
        db.execute("INSERT INTO native_sdk_log_owners(rowid,owner_id,domain,dispatch_key,scan_id,attempt_number,root_run_id,run_id,assignment_id,lease_attempt_id,worker_id,round_number,request_hash,identity_hash,created_at)
            VALUES(7,'original-owner','specialist','original-dispatch','scan',1,'root','child','assignment','worker-attempt','worker',1,?1,?2,'2026-10-09 20:00:00')",params!["a".repeat(64),"b".repeat(64)]).unwrap();
        db.execute("INSERT INTO native_sdk_log_mapper_owners(owner_id,logical_domain,identity_hash) VALUES('original-owner','mapper_loop',?1)",["c".repeat(64)]).unwrap();
    }
    (
        Table::read(
            &db,
            "native_sdk_log_mapper_owners",
            "?1 IS NOT NULL",
            "scope",
        )
        .unwrap(),
        Table::read(&db, "native_sdk_log_owners", "?1 IS NOT NULL", "scope").unwrap(),
    )
}

#[test]
fn deleted_audit_mapper_diagnostic_actual_pk_and_private_restore_preserve_all_values() {
    let (table, parent) = original_diagnostic_tables(true);
    assert!(
        matches!(&table.identity,Some(RowIdentity::PrimaryKey{columns}) if columns==&vec!["owner_id".to_string()])
    );
    assert_eq!(
        table.columns,
        vec!["owner_id", "logical_domain", "identity_hash"]
    );
    assert_eq!(table.rows.len(), 1);
    let arena = Connection::open_in_memory().unwrap();
    arena.pragma_update(None, "foreign_keys", false).unwrap();
    table.restore(&arena).unwrap();
    assert_eq!(arena.query_row("SELECT count(*) FROM main.sqlite_master WHERE type='table' AND name='native_sdk_log_owners'",[],|r|r.get::<_,i64>(0)).unwrap(),0,
        "sidecar restoration must not create the later column-only parent");
    parent.restore(&arena).unwrap();
    assert_eq!(
        Table::read(
            &arena,
            "native_sdk_log_mapper_owners",
            "?1 IS NOT NULL",
            "scope"
        )
        .unwrap()
        .canonical()
        .unwrap(),
        table.canonical().unwrap()
    );
    assert_eq!(
        Table::read(&arena, "native_sdk_log_owners", "?1 IS NOT NULL", "scope")
            .unwrap()
            .canonical()
            .unwrap(),
        parent.canonical().unwrap()
    );
    assert_eq!(
        arena
            .query_row(
                "SELECT rowid FROM native_sdk_log_owners WHERE owner_id='original-owner'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        7
    );
}

#[test]
fn deleted_audit_mapper_diagnostic_empty_sidecar_and_cold_pk_definition_are_bounded() {
    let (empty, _) = original_diagnostic_tables(false);
    let arena = Connection::open_in_memory().unwrap();
    empty.restore(&arena).unwrap();
    assert_eq!(
        Table::read(
            &arena,
            "native_sdk_log_mapper_owners",
            "?1 IS NOT NULL",
            "scope"
        )
        .unwrap()
        .canonical()
        .unwrap(),
        empty.canonical().unwrap()
    );
    let (mut altered, _) = original_diagnostic_tables(true);
    altered.identity = Some(RowIdentity::PrimaryKey {
        columns: vec!["identity_hash".into()],
    });
    let arena = Connection::open_in_memory().unwrap();
    assert_eq!(
        altered.restore(&arena).unwrap_err(),
        "deleted_audit_original_schema_identity_changed"
    );
    assert_eq!(
        arena
            .query_row(
                "SELECT count(*) FROM native_sdk_log_mapper_owners",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(arena.query_row("SELECT count(*) FROM main.sqlite_master WHERE type='table' AND name='native_sdk_log_owners'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
}

#[test]
fn deleted_audit_mapper_additive_sources_remain_optional_for_original_bundle_bytes() {
    // This tests the retained-name decision only, not a signed closed worker.
    let marker = Table {
        name: "agent_multi_exit_receipts".into(),
        columns: vec!["rowid".into(), "id".into()],
        rows: vec![vec![
            Cell::Integer(1),
            Cell::Text("structural-marker".into()),
        ]],
        identity: None,
    };
    for version in [1, 2] {
        let mut bundle = Bundle {
            version,
            scan: "scan".into(),
            roots: vec!["root".into()],
            tables: vec![marker.clone()],
        };
        let original = serde_json::to_string(&bundle).unwrap();
        assert!(!bundle.retained().any(|name| matches!(
            *name,
            "native_sdk_log_mapper_owners" | "agent_mapper_loop_journal"
        )));
        assert_eq!(
            serde_json::to_string(&bundle).unwrap(),
            original,
            "no old archive backfill"
        );
        let (sidecar, _) = original_diagnostic_tables(false);
        bundle.tables.push(sidecar);
        assert!(bundle
            .retained()
            .any(|name| *name == "native_sdk_log_mapper_owners"));
        bundle.tables.push(Table {
            name: "agent_mapper_loop_journal".into(),
            columns: vec!["rowid".into(), "call_id".into()],
            rows: vec![],
            identity: None,
        });
        assert!(bundle
            .retained()
            .any(|name| *name == "agent_mapper_loop_journal"));
    }
}
