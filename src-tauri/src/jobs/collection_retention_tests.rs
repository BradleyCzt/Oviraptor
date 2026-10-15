use super::*;

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "oviraptor-retention-fixture-{}-{}",
            std::process::id(),
            TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self {
            root: root.canonicalize().unwrap(),
        }
    }
    fn output(&self, run: i64) -> PathBuf {
        let dir = self.root.join(format!("run-{run}"));
        fs::create_dir_all(&dir).unwrap();
        dir.join("collection")
    }
    fn open(&self, run: i64, project: i64, now: i64, limits: Limits) -> CollectionStorage {
        CollectionStorage::open(&self.root, &self.output(run), project, run, now, limits).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn limits() -> Limits {
    Limits {
        global_bytes: 512 * 1024,
        project_bytes: 384 * 1024,
        run_bytes: 128 * 1024,
        pages: 4,
        age_seconds: 100,
    }
}
fn page(number: u8) -> PathBuf {
    PathBuf::from(format!("collection-checkpoint-v1/pages/{number:064x}.json"))
}

#[test]
fn collection_retention_defaults_bound_only_explicit_owned_collection_scope() {
    let value = Limits::default();
    assert_eq!(value.global_bytes, 2 * 1024 * 1024 * 1024);
    assert_eq!(value.run_bytes, 512 * 1024 * 1024);
    assert_eq!(value.pages, 4096);
    assert_eq!(value.age_seconds, 7 * 86400);
    let fixture = Fixture::new();
    let storage = fixture.open(1, 10, 1000, limits());
    assert_eq!(
        storage.report().unwrap().as_json()["analysisSnapshotPinsIntegrated"],
        false
    );
    assert!(storage.report().unwrap().as_json()["scope"]
        .as_str()
        .unwrap()
        .contains("owned_collection"));
}
#[test]
fn collection_retention_rejects_payload_before_touching_existing_file() {
    let fixture = Fixture::new();
    let storage = fixture.open(1, 10, 1000, limits());
    let path = page(1);
    storage
        .write(&path, b"original", ArtifactKind::Page, Some(1000), 1000)
        .unwrap();
    let before = fs::read(storage.output.join(OWNER)).unwrap();
    assert!(storage
        .write(
            &path,
            &vec![0; 128 * 1024],
            ArtifactKind::Page,
            Some(1000),
            1000
        )
        .unwrap_err()
        .starts_with("resource_limited:"));
    assert_eq!(fs::read(storage.output.join(path)).unwrap(), b"original");
    assert_eq!(fs::read(storage.output.join(OWNER)).unwrap(), before);
    assert!(!inventory(&storage.output)
        .unwrap()
        .iter()
        .any(|r| r.path.to_string_lossy().contains(".retention-tmp-")));
}
#[test]
fn collection_retention_page_admission_refuses_next_request_and_write() {
    let fixture = Fixture::new();
    let mut policy = limits();
    policy.pages = 1;
    let storage = fixture.open(1, 10, 1000, policy);
    storage.preflight_page(1000).unwrap();
    storage
        .write(&page(1), b"one", ArtifactKind::Page, Some(1000), 1000)
        .unwrap();
    assert!(storage
        .preflight_page(1000)
        .unwrap_err()
        .contains("resource_limited"));
    assert!(storage
        .write(&page(2), b"two", ArtifactKind::Page, Some(1000), 1000)
        .unwrap_err()
        .contains("resource_limited"));
    assert!(!storage.output.join(page(2)).exists());
}
#[test]
fn collection_retention_project_and_global_admission_include_other_run_files() {
    let fixture = Fixture::new();
    let mut policy = limits();
    policy.global_bytes = 180 * 1024;
    policy.project_bytes = 180 * 1024;
    let first = fixture.open(1, 10, 1000, policy);
    first
        .write(
            &page(1),
            &vec![0; 80 * 1024],
            ArtifactKind::Page,
            Some(1000),
            1000,
        )
        .unwrap();
    let second = fixture.open(2, 10, 1000, policy);
    assert!(second
        .write(
            &page(2),
            &vec![0; 80 * 1024],
            ArtifactKind::Page,
            Some(1000),
            1000
        )
        .unwrap_err()
        .contains("resource_limited"));
    assert!(second.report().unwrap().project_bytes >= 80 * 1024);
    let third = fixture.open(3, 20, 1000, policy);
    assert!(third
        .write(
            &page(3),
            &vec![0; 80 * 1024],
            ArtifactKind::Page,
            Some(1000),
            1000
        )
        .unwrap_err()
        .contains("resource_limited"));
}
#[test]
fn collection_retention_expired_closed_raw_has_durable_tombstone_but_metadata_remains() {
    let fixture = Fixture::new();
    let first = fixture.open(1, 10, 1000, limits());
    first
        .write(&page(1), b"receipt", ArtifactKind::Page, Some(1000), 1000)
        .unwrap();
    first
        .write(
            Path::new("collection_state.json"),
            b"{}",
            ArtifactKind::Metadata,
            Some(1000),
            1000,
        )
        .unwrap();
    first.finish(1000).unwrap();
    let old = first.output.clone();
    drop(first);
    let _second = fixture.open(2, 20, 1200, limits());
    assert!(!old.join(page(1)).exists());
    assert!(old.join("collection_state.json").exists());
    let manifest = read_manifest(&old).unwrap();
    assert_eq!(manifest["expiredFiles"], 1);
    assert_eq!(manifest["expiredBytes"], 7);
    assert_eq!(manifest["tombstones"][0]["hash"], hash(b"receipt"));
    assert!(manifest["entries"].get(page(1).to_str().unwrap()).is_none());
}
#[test]
fn collection_retention_active_writer_and_checkpoint_lease_prevent_expiry() {
    let fixture = Fixture::new();
    let first = fixture.open(1, 10, 1000, limits());
    first
        .write(&page(1), b"active", ArtifactKind::Page, Some(1000), 1000)
        .unwrap();
    first.finish(1000).unwrap();
    let _second = fixture.open(2, 20, 1200, limits());
    assert!(first.output.join(page(1)).exists());
    let old = first.output.clone();
    drop(first);
    let _checkpoint = lock(&old.join("collection-checkpoint-v1/active.lock"), false)
        .unwrap()
        .unwrap();
    let _third = fixture.open(3, 20, 1200, limits());
    assert!(old.join(page(1)).exists());
}
#[test]
fn collection_retention_pin_unknown_age_changed_hash_and_unknown_files_are_preserved() {
    let fixture = Fixture::new();
    let first = fixture.open(1, 10, 1000, limits());
    for index in 1..=3 {
        first
            .write(
                &page(index),
                b"original",
                ArtifactKind::Page,
                if index == 2 { None } else { Some(1000) },
                1000,
            )
            .unwrap();
    }
    let mut manifest = read_manifest(&first.output).unwrap();
    manifest["entries"][page(2).to_str().unwrap()]["storedAt"] = Value::Null;
    first.publish_control(&first.output, &manifest).unwrap();
    first.set_pinned(&page(1), true).unwrap();
    fs::write(first.output.join(page(3)), b"externally changed").unwrap();
    fs::write(first.output.join("unknown.user"), b"never clean this").unwrap();
    first.finish(1000).unwrap();
    let old = first.output.clone();
    drop(first);
    let _second = fixture.open(2, 20, 1200, limits());
    for index in 1..=3 {
        assert!(old.join(page(index)).exists());
    }
    assert!(old.join("unknown.user").exists());
}
#[test]
fn collection_retention_unknown_legacy_run_is_not_adopted_or_cleaned_by_sweep() {
    let fixture = Fixture::new();
    let legacy = fixture.output(1);
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("candidates.csv"), b"old user data").unwrap();
    assert!(CollectionStorage::open(&fixture.root, &legacy, 10, 1, 1000, limits()).is_err());
    assert!(!legacy.join(OWNER).exists());
    let _second = fixture.open(2, 20, 99999, limits());
    assert_eq!(
        fs::read(legacy.join("candidates.csv")).unwrap(),
        b"old user data"
    );
    let adopted =
        CollectionStorage::open_adopting_current(&fixture.root, &legacy, 10, 1, 1000, limits())
            .unwrap();
    assert!(read_manifest(&legacy).unwrap()["entries"]["candidates.csv"]["receivedAt"].is_null());
    adopted.finish(1000).unwrap();
    drop(adopted);
    let _third = fixture.open(3, 20, 99999, limits());
    assert!(legacy.join("candidates.csv").exists());
}
#[test]
fn collection_retention_rejected_new_owner_cannot_publish_metadata_over_budget() {
    let fixture = Fixture::new();
    let output = fixture.output(1);
    let policy = Limits {
        global_bytes: 100,
        project_bytes: 100,
        run_bytes: 100,
        ..limits()
    };
    assert!(
        CollectionStorage::open(&fixture.root, &output, 10, 1, 1000, policy)
            .err()
            .unwrap()
            .contains("resource_limited")
    );
    assert!(!output.join(OWNER).exists());
}
#[test]
fn collection_retention_removal_recovers_tombstoned_raw_after_crash() {
    let fixture = Fixture::new();
    let first = fixture.open(1, 10, 1000, limits());
    let path = page(1);
    first
        .write(
            &path,
            b"crash receipt",
            ArtifactKind::Page,
            Some(1000),
            1000,
        )
        .unwrap();
    let mut manifest = read_manifest(&first.output).unwrap();
    let key = path.to_str().unwrap();
    let entry = manifest["entries"][key].clone();
    record_removal(&mut manifest, key, &entry, "receipt_expired", 1000);
    first.publish_control(&first.output, &manifest).unwrap();
    first.finish(1000).unwrap();
    let old = first.output.clone();
    drop(first);
    let _second = fixture.open(2, 20, 1200, limits());
    assert!(!old.join(&path).exists());
    assert_eq!(read_manifest(&old).unwrap()["expiredFiles"], 1);
}
#[test]
fn collection_retention_explicit_remove_checks_hash_pin_and_records_summary() {
    let fixture = Fixture::new();
    let storage = fixture.open(1, 10, 1000, limits());
    let path = page(1);
    storage
        .write(&path, b"payload", ArtifactKind::Page, Some(1000), 1000)
        .unwrap();
    storage.set_pinned(&path, true).unwrap();
    assert!(storage.remove(&path, "receipt_expired", 1200).is_err());
    storage.set_pinned(&path, false).unwrap();
    assert!(storage.remove(&path, "delete_anything", 1200).is_err());
    let report = storage.remove(&path, "receipt_expired", 1200).unwrap();
    assert_eq!(report.expired_files, 1);
    assert!(!storage.output.join(path).exists());
}
#[test]
fn collection_retention_refuses_traversal_other_run_and_duplicate_writer() {
    let fixture = Fixture::new();
    let storage = fixture.open(1, 10, 1000, limits());
    assert!(storage
        .write(
            Path::new("../outside.csv"),
            b"bad",
            ArtifactKind::Csv,
            Some(1000),
            1000
        )
        .is_err());
    assert!(
        CollectionStorage::open(&fixture.root, &storage.output, 10, 1, 1000, limits()).is_err()
    );
    assert!(CollectionStorage::open(
        &fixture.root,
        &fixture.root.join("other"),
        10,
        1,
        1000,
        limits()
    )
    .is_err());
}
#[test]
fn collection_retention_finish_after_payload_limit_keeps_durable_close_marker() {
    let fixture = Fixture::new();
    let storage = fixture.open(1, 10, 1000, limits());
    storage
        .write(
            &page(1),
            &vec![1; 80 * 1024],
            ArtifactKind::Page,
            Some(1000),
            1000,
        )
        .unwrap();
    assert!(storage
        .write(
            &page(2),
            &vec![2; 80 * 1024],
            ArtifactKind::Page,
            Some(1000),
            1000
        )
        .is_err());
    storage.finish(1001).unwrap();
    assert_eq!(read_manifest(&storage.output).unwrap()["closedAt"], 1001);
}
#[cfg(unix)]
#[test]
fn collection_retention_never_follows_symlink_payload_or_owned_root() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = fixture.root.join("user-file");
    fs::write(&outside, b"user").unwrap();
    let storage = fixture.open(1, 10, 1000, limits());
    symlink(&outside, storage.output.join("candidates.csv")).unwrap();
    assert!(storage
        .write(
            Path::new("candidates.csv"),
            b"overwrite",
            ArtifactKind::Csv,
            Some(1000),
            1000
        )
        .is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"user");
}

#[test]
fn collection_retention_new_csv_and_inventory_use_storage_clock_without_inventing_observation_time()
{
    let fixture = Fixture::new();
    let first = fixture.open(1, 10, 1000, limits());
    first
        .write(
            Path::new("candidates.csv"),
            b"url\n",
            ArtifactKind::Csv,
            None,
            1000,
        )
        .unwrap();
    first
        .write(
            Path::new("confirmed_urls.txt"),
            b"https://example.invalid\n",
            ArtifactKind::Inventory,
            None,
            1000,
        )
        .unwrap();
    let before = read_manifest(&first.output).unwrap();
    assert!(before["entries"]["candidates.csv"]["receivedAt"].is_null());
    assert_eq!(before["entries"]["candidates.csv"]["storedAt"], 1000);
    first.finish(1000).unwrap();
    let old = first.output.clone();
    drop(first);
    let second = fixture.open(2, 20, 1200, limits());
    assert!(!old.join("candidates.csv").exists());
    assert!(!old.join("confirmed_urls.txt").exists());
    assert_eq!(second.report().unwrap().global_expired_files, 2);
}

#[test]
fn collection_retention_abandoned_writer_recovers_exact_publication_and_conservative_close_clock() {
    let fixture = Fixture::new();
    let first = fixture.open(1, 10, 1000, limits());
    let path = page(1);
    first
        .write(
            &path,
            b"published-before-crash",
            ArtifactKind::Page,
            Some(1000),
            1000,
        )
        .unwrap();
    let mut manifest = read_manifest(&first.output).unwrap();
    manifest["entries"][path.to_str().unwrap()]["phase"] = json!("writing");
    first.publish_control(&first.output, &manifest).unwrap();
    let old = first.output.clone();
    drop(first);
    let second = fixture.open(2, 20, 1200, limits());
    let recovered = read_manifest(&old).unwrap();
    assert_eq!(recovered["closedAt"], 1200);
    assert_eq!(recovered["recoveredClosedAt"], 1200);
    assert_eq!(
        recovered["entries"][path.to_str().unwrap()]["phase"],
        "available"
    );
    assert!(old.join(&path).exists());
    second.preflight(1301).unwrap();
    assert!(!old.join(&path).exists());
}
#[test]
fn collection_retention_crashed_overwrite_with_different_payload_is_never_guessed_or_removed() {
    let fixture = Fixture::new();
    let first = fixture.open(1, 10, 1000, limits());
    let path = page(1);
    first
        .write(&path, b"old", ArtifactKind::Page, Some(1000), 1000)
        .unwrap();
    let mut manifest = read_manifest(&first.output).unwrap();
    manifest["entries"][path.to_str().unwrap()]["phase"] = json!("writing");
    manifest["entries"][path.to_str().unwrap()]["hash"] = json!(hash(b"not-published"));
    first.publish_control(&first.output, &manifest).unwrap();
    let old = first.output.clone();
    drop(first);
    let second = fixture.open(2, 20, 1200, limits());
    second.preflight(1500).unwrap();
    assert_eq!(fs::read(old.join(&path)).unwrap(), b"old");
    let value = read_manifest(&old).unwrap();
    assert_eq!(
        value["entries"][path.to_str().unwrap()]["recoveryConflict"],
        true
    );
}

#[test]
fn collection_retention_gc_control_peak_is_admitted_before_tombstone_or_unlink() {
    let fixture = Fixture::new();
    let policy = Limits {
        global_bytes: 128 * 1024,
        project_bytes: 128 * 1024,
        ..limits()
    };
    let first = fixture.open(1, 10, 1000, policy);
    first
        .write(&page(1), b"receipt", ArtifactKind::Page, Some(1000), 1000)
        .unwrap();
    first.finish(1000).unwrap();
    let old = first.output.clone();
    let used = first.report().unwrap().global_bytes;
    // Model a legacy/unknown file discovered inside an explicitly adopted run.
    // Such a file is counted but is never an automatic deletion candidate.
    fs::write(
        old.join("unknown-preserved.bin"),
        vec![0; (policy.global_bytes - used - 100) as usize],
    )
    .unwrap();
    let before = fs::read(old.join(OWNER)).unwrap();
    drop(first);
    let next = fixture.output(2);
    assert!(
        CollectionStorage::open(&fixture.root, &next, 20, 2, 1200, policy)
            .err()
            .unwrap()
            .contains("resource_limited")
    );
    assert_eq!(fs::read(old.join(OWNER)).unwrap(), before);
    assert!(old.join(page(1)).exists());
    assert!(!next.join(OWNER).exists());
}
