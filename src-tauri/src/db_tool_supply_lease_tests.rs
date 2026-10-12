// Disposable initialized databases only; tests never operate on app data.
struct ToolSupplyLeaseTestDirectory(PathBuf);
impl ToolSupplyLeaseTestDirectory {
    fn new() -> (Self, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("oviraptor-tool-lease-{}", uuid::Uuid::new_v4()));
        let path = initialize(&root).unwrap();
        (Self(root), path)
    }
}
impl Drop for ToolSupplyLeaseTestDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
#[test]
fn tool_supply_lease_original_immediate_transaction_and_complete_remain_valid() {
    let (_directory, path) = ToolSupplyLeaseTestDirectory::new();
    let guard = begin_environment_preparation(&path).unwrap();
    let mut db = open(&path).unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    guard.require_tool_supply_on(&tx).unwrap();
    tx.rollback().unwrap();
    drop(db);
    guard.complete().unwrap();
    assert_eq!(environment_preparation_status(&path).unwrap().state, "idle");
}
#[test]
fn tool_supply_lease_temp_owner_cannot_hide_original_owner_replacement() {
    let (_directory, path) = ToolSupplyLeaseTestDirectory::new();
    let guard = begin_environment_preparation(&path).unwrap();
    let mut db = open(&path).unwrap();
    db.execute_batch("CREATE TEMP TABLE environment_preparation_lease AS SELECT * FROM main.environment_preparation_lease; UPDATE main.environment_preparation_lease SET owner='replacement-owner' WHERE singleton=1;").unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(
        guard.require_tool_supply_on(&tx).unwrap_err(),
        "tool_supply_preparation_owner_mismatch"
    );
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT owner FROM main.environment_preparation_lease",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "replacement-owner"
    );
}
#[test]
fn tool_supply_lease_temp_scan_inventory_is_never_authoritative() {
    let (_directory, path) = ToolSupplyLeaseTestDirectory::new();
    let guard = begin_environment_preparation(&path).unwrap();
    let mut db = open(&path).unwrap();
    // Emulate a cold damaged main activation while restoring the exact
    // production interlock before asking the lease to authorize anything.
    let trigger: String = db.query_row("SELECT sql FROM main.sqlite_master WHERE name='environment_preparation_refuse_scan_insert'", [], |r|r.get(0)).unwrap();
    db.execute_batch("DROP TRIGGER environment_preparation_refuse_scan_insert; INSERT INTO main.sentinel_scans(id,status) VALUES('cold-active-scan','scanning');").unwrap();
    db.execute_batch(&trigger).unwrap();
    db.execute_batch(
        "CREATE TEMP TABLE sentinel_scans AS SELECT * FROM main.sentinel_scans WHERE 0;",
    )
    .unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(
        guard.require_tool_supply_on(&tx).unwrap_err(),
        "tool_supply_preparation_owner_mismatch"
    );
    tx.rollback().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT status FROM main.sentinel_scans WHERE id='cold-active-scan'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "scanning"
    );
}
#[cfg(unix)]
#[test]
fn tool_supply_lease_replaced_held_lock_path_cannot_authorize_or_complete() {
    let (_directory, path) = ToolSupplyLeaseTestDirectory::new();
    let guard = begin_environment_preparation(&path).unwrap();
    let lock = path.with_extension("preparation.lock");
    fs::rename(&lock, lock.with_extension("original-lock")).unwrap();
    fs::write(&lock, []).unwrap();
    let mut db = open(&path).unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(
        guard.require_tool_supply_on(&tx).unwrap_err(),
        "tool_supply_preparation_owner_mismatch"
    );
    tx.rollback().unwrap();
    assert_eq!(
        guard.complete().unwrap_err(),
        "tool_supply_preparation_owner_mismatch"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM environment_preparation_lease",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}
#[cfg(unix)]
#[test]
fn tool_supply_lease_same_owner_copied_database_is_a_different_entity() {
    let (_directory, path) = ToolSupplyLeaseTestDirectory::new();
    let guard = begin_environment_preparation(&path).unwrap();
    let db = open(&path).unwrap();
    db.pragma_update(None, "journal_mode", "DELETE").unwrap();
    drop(db);
    let original = path.with_extension("original-db");
    fs::rename(&path, &original).unwrap();
    fs::copy(&original, &path).unwrap();
    let mut db = open(&path).unwrap();
    assert_eq!(
        db.query_row(
            "SELECT owner FROM main.environment_preparation_lease",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        guard.owner
    );
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    assert_eq!(
        guard.require_tool_supply_on(&tx).unwrap_err(),
        "tool_supply_preparation_owner_mismatch"
    );
    tx.rollback().unwrap();
    assert_eq!(
        guard.complete().unwrap_err(),
        "tool_supply_preparation_owner_mismatch"
    );
}
#[cfg(unix)]
#[test]
fn tool_supply_lease_old_open_sqlite_entity_cannot_follow_rebuilt_path() {
    let (_directory, path) = ToolSupplyLeaseTestDirectory::new();
    let guard = begin_environment_preparation(&path).unwrap();
    let mut db = open(&path).unwrap();
    db.pragma_update(None, "journal_mode", "DELETE").unwrap();
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let original = path.with_extension("original-db");
    fs::rename(&path, &original).unwrap();
    fs::copy(&original, &path).unwrap();
    assert_eq!(
        guard.require_tool_supply_on(&tx).unwrap_err(),
        "tool_supply_preparation_owner_mismatch"
    );
    tx.rollback().unwrap();
}
