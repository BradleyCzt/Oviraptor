// The real exclusive publisher is macOS-only. Other Unix hosts must reject
// valid, owned requests without inserting a decision or creating cache bytes.
fn owned_package_all_table_snapshot(db: &Connection) -> Vec<(String, Vec<String>)> {
    let tables = db.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap().query_map([], |row| row.get::<_, String>(0)).unwrap()
        .collect::<rusqlite::Result<Vec<_>>>().unwrap();
    let quote = |name: &str| format!("\"{}\"", name.replace('"', "\"\""));
    tables.into_iter().map(|table| {
        let without_rowid: bool = db.query_row(
            "SELECT wr FROM pragma_table_list WHERE schema='main' AND name=?1", [&table],
            |row| row.get(0),
        ).unwrap();
        let sql = if without_rowid {
            let keys = db.prepare("SELECT name FROM pragma_table_info(?1,'main') WHERE pk>0 ORDER BY pk")
                .unwrap().query_map([&table], |row| row.get::<_, String>(0)).unwrap()
                .collect::<rusqlite::Result<Vec<_>>>().unwrap();
            assert!(!keys.is_empty());
            format!("SELECT * FROM main.{} ORDER BY {}", quote(&table),
                keys.iter().map(|key| quote(key)).collect::<Vec<_>>().join(","))
        } else {
            format!("SELECT rowid,* FROM main.{} ORDER BY rowid", quote(&table))
        };
        let mut statement = db.prepare(&sql).unwrap();
        let width = statement.column_count();
        let rows = statement.query_map([], |row| {
            let values = (0..width).map(|index| row.get::<_, rusqlite::types::Value>(index))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(format!("{values:?}"))
        }).unwrap().collect::<rusqlite::Result<Vec<_>>>().unwrap();
        (table, rows)
    }).collect()
}

fn owned_package_configuration_snapshot(root: &std::path::Path) -> Vec<(PathBuf, String, Vec<u8>)> {
    use std::os::unix::fs::MetadataExt;
    fn visit(root: &std::path::Path, path: &std::path::Path, rows: &mut Vec<(PathBuf, String, Vec<u8>)>) {
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(!metadata.file_type().is_symlink());
        let identity = format!("{:?}", (metadata.dev(), metadata.ino(), metadata.uid(), metadata.mode(),
            metadata.nlink(), metadata.len(), metadata.mtime(), metadata.mtime_nsec(),
            metadata.ctime(), metadata.ctime_nsec()));
        let bytes = if metadata.is_dir() { Vec::new() } else {
            assert!(metadata.is_file());
            fs::read(path).unwrap()
        };
        rows.push((path.strip_prefix(root).unwrap().to_path_buf(), identity, bytes));
        if metadata.is_dir() {
            let mut children = fs::read_dir(path).unwrap().map(|entry| entry.unwrap().path()).collect::<Vec<_>>();
            children.sort();
            for child in children { visit(root, &child, rows); }
        }
    }
    let mut rows = Vec::new();
    visit(root, root, &mut rows);
    rows
}

fn owned_package_unsupported_publication_is_readonly(f: &Fixture, decision: &[u8], existing: bool) {
    let candidate = approval::preflight_approve(&f.config, decision, &f.manifest, &f.package).unwrap();
    let approval_id = candidate.id();
    let db = Connection::open(&f.path).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM tool_administrator_decisions WHERE decision_id=?1",
        [approval_id], |row| row.get::<_, i64>(0)).unwrap(), if existing { 1 } else { 0 });
    assert!(!f.config.original_package_cache_root().exists());
    let preparation = crate::db::begin_environment_preparation(&f.path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    preparation.require_tool_supply_on(&tx).unwrap();
    drop(tx);
    // Lease creation is an explicit precondition. Only the actual publication
    // phase is measured; its failure must retain this exact, still-valid fence.
    let before = owned_package_all_table_snapshot(&db);
    let configuration = f.config.directory_for_test();
    let files = owned_package_configuration_snapshot(&configuration);
    let changes = db.total_changes();
    for _ in 0..2 {
        let error = approval_writer::approve_with_package(&db, &preparation, &f.config,
            decision, &f.manifest, &f.package).err().unwrap();
        assert_eq!(error, "unsupported_tool_package_exclusive_publish_platform");
        assert_eq!(owned_package_all_table_snapshot(&db), before);
        assert_eq!(db.total_changes(), changes);
        assert_eq!(owned_package_configuration_snapshot(&configuration), files);
        assert!(!f.config.original_package_cache_root().exists());
        let tx = db.unchecked_transaction().unwrap();
        preparation.require_tool_supply_on(&tx).unwrap();
        let source_error = approved_package_source::ApprovedPackageSource::open_on(
            &tx, &f.config, approval_id).err().unwrap();
        assert_eq!(source_error, if existing { "approved_package_source_missing" }
            else { "approved_package_source_approval_missing" });
        assert_eq!(owned_package_all_table_snapshot(&tx), before);
        assert_eq!(tx.total_changes(), changes);
        drop(tx);
    }
    assert_eq!(owned_package_configuration_snapshot(&configuration), files);
    // No unknown filesystem outcome remains. Explicit test cleanup occurs
    // after all rejection and original-fence preservation assertions.
    preparation.complete().unwrap();
}

#[test]
fn approved_package_owned_writer_unsupported_new_decision_has_no_publication_side_effects() {
    let f = Fixture::new();
    owned_package_unsupported_publication_is_readonly(&f, &f.new_signed_decision(), false);
}

#[test]
fn approved_package_owned_writer_unsupported_original_missing_cache_preserves_decision_and_fence() {
    let f = Fixture::new();
    owned_package_unsupported_publication_is_readonly(&f, &f.decision, true);
}
