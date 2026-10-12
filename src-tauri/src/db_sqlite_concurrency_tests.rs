// SQLite's default memory statistics take one process-global mutex on every
// sqlite3_malloc/free. Each `open` re-parses the full schema, so concurrent
// connections serialized on that mutex and wall-time/lease budgets expired
// under parallel load. The bundled build disables it (repo .cargo/config.toml).
#[test]
fn bundled_sqlite_has_no_process_global_memstatus_mutex() {
    let root = std::env::temp_dir().join(format!("oviraptor-memstatus-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let objects: i64 = connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
        .unwrap();
    assert!(objects > 0, "the real schema must have been parsed");
    let option = std::ffi::CString::new("DEFAULT_MEMSTATUS=0").unwrap();
    assert_eq!(
        unsafe { rusqlite::ffi::sqlite3_compileoption_used(option.as_ptr()) },
        1,
        "bundled SQLite must be built with SQLITE_DEFAULT_MEMSTATUS=0"
    );
    assert_eq!(
        unsafe { rusqlite::ffi::sqlite3_memory_used() },
        0,
        "memory statistics (and their global mutex) must stay disabled"
    );
    drop(connection);
    let _ = fs::remove_dir_all(root);
}
