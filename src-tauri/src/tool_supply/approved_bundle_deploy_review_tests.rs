//! Actual owned deploy boundary against stale snapshots and caller-local state.
use super::{approved_bundle, approved_bundle_deploy, package_format};
use approved_bundle::tests::Fixture;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
fn package() -> Vec<u8> {
    let content = b"fixed deployment review parser, never executed";
    let mut b = package_format::MAGIC.to_vec();
    b.extend(1u16.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(10u16.to_le_bytes());
    b.extend(u64::try_from(content.len()).unwrap().to_le_bytes());
    b.push(1);
    b.extend(Sha256::digest(content));
    b.extend(b"bin/parser");
    b.extend(content);
    b
}
#[test]
fn approved_bundle_deploy_review_actual_revoke_after_old_snapshot_refuses_without_creating_root() {
    let f = Fixture::new(approved_bundle::ADAPTER, package());
    let db = Connection::open(f.database_path()).unwrap();
    let old = db.unchecked_transaction().unwrap();
    let mut bundle = approved_bundle::open_on(&old, f.configuration(), f.approval_id()).unwrap();
    f.revoke();
    // This is deliberately only a historical snapshot: the old read still
    // verifies, but it is never passed to the owned deployment authority.
    bundle.require_on(&old, f.configuration()).unwrap();
    drop(bundle);
    old.rollback().unwrap();
    let lease = crate::db::begin_environment_preparation(f.database_path()).unwrap();
    let before = db.total_changes();
    let error = approved_bundle_deploy::deploy_on(&db, &lease, f.configuration(), f.approval_id())
        .err()
        .unwrap();
    assert_eq!(error, "tool_administrator_approval_revoked");
    assert_eq!(db.total_changes(), before);
    assert!(!f.configuration().original_deployed_package_root().exists());
    lease.complete().unwrap();
}
#[test]
fn approved_bundle_deploy_review_caller_temp_approval_cannot_hide_actual_revocation() {
    let f = Fixture::new(approved_bundle::ADAPTER, package());
    let db = Connection::open(f.database_path()).unwrap();
    db.execute_batch("CREATE TEMP TABLE tool_administrator_decisions AS SELECT * FROM main.tool_administrator_decisions WHERE kind='approve'").unwrap();
    f.revoke();
    // Caller-local shadows are not a source for the independently opened writer.
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM temp.tool_administrator_decisions WHERE kind='revoke'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    let lease = crate::db::begin_environment_preparation(f.database_path()).unwrap();
    let error = approved_bundle_deploy::deploy_on(&db, &lease, f.configuration(), f.approval_id())
        .err()
        .unwrap();
    assert_eq!(error, "tool_administrator_approval_revoked");
    assert!(!f.configuration().original_deployed_package_root().exists());
    lease.complete().unwrap();
}
