use super::*;
use crate::tool_supply::{approved_bundle::tests::Fixture, package_format::MAGIC};
use sha2::{Digest, Sha256};
use std::{
    fs,
    os::unix::{
        fs::symlink,
        fs::{MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};
fn bytes() -> Vec<u8> {
    let mut b = MAGIC.to_vec();
    b.extend(1u16.to_le_bytes());
    b.extend(2u32.to_le_bytes());
    for (path, mode, content) in [
        (
            "bin/parser",
            1u8,
            b"fixed parser deployment bytes".as_slice(),
        ),
        (
            "data/nested/input.json",
            0u8,
            b"{\"readonly\":true}".as_slice(),
        ),
    ] {
        b.extend(u16::try_from(path.len()).unwrap().to_le_bytes());
        b.extend(u64::try_from(content.len()).unwrap().to_le_bytes());
        b.push(mode);
        b.extend(Sha256::digest(content));
        b.extend(path.as_bytes());
        b.extend(content);
    }
    b
}
fn directory(f: &Fixture) -> PathBuf {
    let db = Connection::open(f.database_path()).unwrap();
    let bundle = approved_bundle::open_on(&db, f.configuration(), f.approval_id()).unwrap();
    f.configuration()
        .original_deployed_package_root()
        .join(&bundle.manifest().id)
        .join(crate::tool_supply::candidate_cache_key(bundle.manifest()).unwrap())
}
// Only this fixture's freshly created directory tree is made writable for
// owned teardown. No data/cache belonging to a different task is traversed.
struct OwnedCleanup(PathBuf);
impl Drop for OwnedCleanup {
    fn drop(&mut self) {
        fn writable(path: &Path) {
            if let Ok(m) = fs::symlink_metadata(path) {
                if m.is_dir() && !m.file_type().is_symlink() {
                    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
                    if let Ok(entries) = fs::read_dir(path) {
                        for entry in entries.flatten() {
                            writable(&entry.path());
                        }
                    }
                }
            }
        }
        writable(&self.0);
    }
}
fn deploy(f: &Fixture) -> VerifiedDeployedBundle {
    let db = Connection::open(f.database_path()).unwrap();
    let lease = crate::db::begin_environment_preparation(f.database_path()).unwrap();
    let before = db.total_changes();
    let deployed = deploy_on(&db, &lease, f.configuration(), f.approval_id()).unwrap();
    assert_eq!(db.total_changes(), before);
    lease.complete().unwrap();
    deployed
}
#[test]
fn approved_bundle_deploy_actual_signed_nested_readonly_tree_and_same_inode_reuse() {
    let f = Fixture::new(approved_bundle::ADAPTER, bytes());
    let _cleanup = OwnedCleanup(f.configuration().original_deployed_package_root());
    let mut first = deploy(&f);
    let path = directory(&f);
    let entry = path.join("bin/parser");
    let data = path.join("data/nested/input.json");
    let before = fs::metadata(&entry).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o500);
    assert_eq!(before.mode() & 0o777, 0o500);
    assert_eq!(fs::metadata(&data).unwrap().mode() & 0o777, 0o400);
    assert_eq!(fs::read(&entry).unwrap(), b"fixed parser deployment bytes");
    assert_eq!(fs::read(&data).unwrap(), b"{\"readonly\":true}");
    assert!(fs::OpenOptions::new().write(true).open(&entry).is_err());
    let second = deploy(&f);
    assert_eq!(first.content_key(), second.content_key());
    assert_eq!(first.approval_id(), f.approval_id());
    assert_eq!(first.metadata().entries().len(), 2);
    let after = fs::metadata(&entry).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    first
        .require_on(
            &Connection::open(f.database_path()).unwrap(),
            f.configuration(),
        )
        .unwrap();
    assert!(fs::read_dir(path.parent().unwrap())
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".staging-")));
}
#[test]
fn approved_bundle_deploy_actual_extra_missing_link_and_replacement_never_repair_or_overwrite() {
    for attack in ["extra", "missing", "symlink", "replacement"] {
        let f = Fixture::new(approved_bundle::ADAPTER, bytes());
        let _cleanup = OwnedCleanup(f.configuration().original_deployed_package_root());
        let mut held = deploy(&f);
        let path = directory(&f);
        let entry = path.join("bin/parser");
        let untouched = fs::read(path.join("data/nested/input.json")).unwrap();
        match attack {
            "extra" => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                fs::write(path.join("unexpected"), b"foreign extra bytes").unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
            }
            "missing" => {
                fs::set_permissions(path.join("bin"), fs::Permissions::from_mode(0o700)).unwrap();
                fs::remove_file(&entry).unwrap();
                fs::set_permissions(path.join("bin"), fs::Permissions::from_mode(0o500)).unwrap();
            }
            "symlink" => {
                fs::set_permissions(path.join("bin"), fs::Permissions::from_mode(0o700)).unwrap();
                fs::remove_file(&entry).unwrap();
                symlink("../../foreign", &entry).unwrap();
                fs::set_permissions(path.join("bin"), fs::Permissions::from_mode(0o500)).unwrap();
            }
            _ => {
                fs::set_permissions(path.join("bin"), fs::Permissions::from_mode(0o700)).unwrap();
                fs::remove_file(&entry).unwrap();
                fs::write(&entry, b"changed parser bytes").unwrap();
                fs::set_permissions(&entry, fs::Permissions::from_mode(0o500)).unwrap();
                fs::set_permissions(path.join("bin"), fs::Permissions::from_mode(0o500)).unwrap();
            }
        }
        let db = Connection::open(f.database_path()).unwrap();
        let held_error = held.require_on(&db, f.configuration()).unwrap_err();
        assert!(
            held_error == "tool_bundle_deploy_tree_changed"
                || held_error == "tool_bundle_deploy_file_changed"
                || held_error.starts_with("tool_bundle_deploy_open:"),
            "{attack}: {held_error}"
        );
        let lease = crate::db::begin_environment_preparation(f.database_path()).unwrap();
        let error = deploy_on(&db, &lease, f.configuration(), f.approval_id())
            .err()
            .unwrap();
        assert!(
            error == "tool_bundle_deploy_tree_changed"
                || error == "tool_bundle_deploy_file_changed"
                || error == "tool_bundle_deploy_file_invalid"
                || error.starts_with("tool_bundle_deploy_open:"),
            "{attack}: {error}"
        );
        assert_eq!(
            fs::read(path.join("data/nested/input.json")).unwrap(),
            untouched
        );
        match attack {
            "extra" => assert_eq!(
                fs::read(path.join("unexpected")).unwrap(),
                b"foreign extra bytes"
            ),
            "missing" => assert!(!entry.exists()),
            "symlink" => assert_eq!(
                fs::read_link(&entry).unwrap(),
                PathBuf::from("../../foreign")
            ),
            _ => assert_eq!(fs::read(&entry).unwrap(), b"changed parser bytes"),
        };
        lease.complete().unwrap();
    }
}
#[test]
fn approved_bundle_deploy_actual_outer_snapshot_foreign_owner_and_stage_fault_are_rejected() {
    let f = Fixture::new(approved_bundle::ADAPTER, bytes());
    let _cleanup = OwnedCleanup(f.configuration().original_deployed_package_root());
    let db = Connection::open(f.database_path()).unwrap();
    let lease = crate::db::begin_environment_preparation(f.database_path()).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    assert_eq!(
        deploy_on(&snapshot, &lease, f.configuration(), f.approval_id())
            .err()
            .unwrap(),
        "tool_bundle_deploy_outer_transaction_forbidden"
    );
    snapshot.rollback().unwrap();
    let other = Fixture::new(approved_bundle::ADAPTER, bytes());
    let otherdb = Connection::open(other.database_path()).unwrap();
    assert_eq!(
        deploy_on(&otherdb, &lease, other.configuration(), other.approval_id())
            .err()
            .unwrap(),
        "tool_supply_preparation_owner_mismatch"
    );
    assert!(!other
        .configuration()
        .original_deployed_package_root()
        .exists());
    lease.complete().unwrap();
    // A real filesystem permission failure blocks stage creation; no published
    // release is invented and the existing unrelated file remains untouched.
    let destination = directory(&f);
    let parent = destination.parent().unwrap();
    fs::create_dir_all(parent).unwrap();
    fs::set_permissions(
        f.configuration().original_deployed_package_root(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::write(parent.join("unrelated"), b"preserve").unwrap();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o500)).unwrap();
    let lease = crate::db::begin_environment_preparation(f.database_path()).unwrap();
    assert_eq!(
        deploy_on(&db, &lease, f.configuration(), f.approval_id())
            .err()
            .unwrap(),
        "tool_bundle_deploy_stage_create_failed"
    );
    assert!(!destination.exists());
    assert_eq!(fs::read(parent.join("unrelated")).unwrap(), b"preserve");
    lease.complete().unwrap();
}

#[test]
fn approved_bundle_deploy_actual_pure_absence_and_held_fence_replay_have_zero_writes() {
    let f = Fixture::new(approved_bundle::ADAPTER, bytes());
    let _cleanup = OwnedCleanup(f.configuration().original_deployed_package_root());
    let db = Connection::open(f.database_path()).unwrap();
    assert!(read_original_on(&db, f.configuration(), f.approval_id())
        .unwrap()
        .is_none());
    assert!(!f.configuration().original_deployed_package_root().exists());
    let original = deploy(&f);
    let lease = crate::db::begin_environment_preparation(f.database_path()).unwrap();
    let owner: String = db
        .query_row(
            "SELECT owner FROM environment_preparation_lease WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let before = db.total_changes();
    let count_before: i64 = db
        .query_row(
            "SELECT count(*) FROM environment_preparation_events",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut repeated = read_original_on(&db, f.configuration(), f.approval_id())
        .unwrap()
        .unwrap();
    assert_eq!(repeated.content_key(), original.content_key());
    repeated.require_on(&db, f.configuration()).unwrap();
    assert_eq!(db.total_changes(), before);
    assert_eq!(
        db.query_row(
            "SELECT owner FROM environment_preparation_lease WHERE singleton=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        owner
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM environment_preparation_events",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        count_before
    );
    let path = directory(&f);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(path.join("unexpected"), b"unchanged attacker bytes").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();
    assert_eq!(
        read_original_on(&db, f.configuration(), f.approval_id())
            .err()
            .unwrap(),
        "tool_bundle_deploy_tree_changed"
    );
    assert_eq!(
        fs::read(path.join("unexpected")).unwrap(),
        b"unchanged attacker bytes"
    );
    assert_eq!(db.total_changes(), before);
    lease.complete().unwrap();
}
