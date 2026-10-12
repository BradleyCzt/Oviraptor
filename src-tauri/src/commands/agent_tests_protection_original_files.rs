#[test]
fn protection_original_file_bytes_lengths_and_directory_move_are_read_only_failures() {
    let f = ProtectionOriginalFixture::new("protection-original-files");
    let db = db::open(&f.h.db_path).unwrap();
    let directory = f.h.context.target_dir.join(AGENT_HTTP_DIRECTORY);
    f.assert_original_read_only(&db, true);
    for name in ["0001.json", "0001.body"] {
        let path = directory.join(name);
        let original = fs::read(&path).unwrap();
        for changed in [
            [original.as_slice(), b" "].concat(),
            original[..original.len() - 1].to_vec(),
        ] {
            fs::write(&path, &changed).unwrap();
            f.assert_original_read_only(&db, false);
            f.assert_original_read_only(&db, false);
            assert_eq!(fs::read(&path).unwrap(), changed);
            fs::write(&path, &original).unwrap();
            f.assert_original_read_only(&db, true);
        }
    }
    let moved = f.h.context.target_dir.join("original-http-moved");
    fs::rename(&directory, &moved).unwrap();
    f.assert_original_read_only(&db, false);
    assert!(!directory.exists());
    fs::rename(&moved, &directory).unwrap();
    f.assert_original_read_only(&db, true);
}

#[cfg(unix)]
#[test]
fn protection_original_soft_hard_links_and_public_permissions_cannot_replace_original_bytes() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let f = ProtectionOriginalFixture::new("protection-original-links");
    let db = db::open(&f.h.db_path).unwrap();
    let directory = f.h.context.target_dir.join(AGENT_HTTP_DIRECTORY);
    f.assert_original_read_only(&db, true);
    for name in ["0001.json", "0001.body"] {
        let path = directory.join(name);
        let saved = directory.join(format!("original-{name}"));
        fs::rename(&path, &saved).unwrap();
        symlink(&saved, &path).unwrap();
        f.assert_original_read_only(&db, false);
        assert!(fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_file(&path).unwrap();
        fs::rename(&saved, &path).unwrap();
        f.assert_original_read_only(&db, true);
        let alias = directory.join(format!("duplicate-{name}"));
        fs::hard_link(&path, &alias).unwrap();
        f.assert_original_read_only(&db, false);
        assert!(alias.exists());
        fs::remove_file(&alias).unwrap();
        f.assert_original_read_only(&db, true);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        f.assert_original_read_only(&db, false);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        f.assert_original_read_only(&db, true);
    }
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    f.assert_original_read_only(&db, false);
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    f.assert_original_read_only(&db, true);
}

#[test]
fn protection_original_private_marker_and_task_damage_do_not_trigger_repair() {
    let f = ProtectionOriginalFixture::new("protection-original-private-binding");
    let db = db::open(&f.h.db_path).unwrap();
    let attempt: String = db
        .query_row(
            "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=?2",
            params![f.actor.scan_id, f.actor.attempt_number],
            |r| r.get(0),
        )
        .unwrap();
    f.assert_original_read_only(&db, true);
    for name in [".oviraptor-scan-id", "task.json"] {
        let path = Path::new(&attempt).join(name);
        let original = fs::read(&path).unwrap();
        fs::write(&path, b"cold-storage-damage").unwrap();
        f.assert_original_read_only(&db, false);
        f.assert_original_read_only(&db, false);
        assert_eq!(fs::read(&path).unwrap(), b"cold-storage-damage");
        fs::write(&path, original).unwrap();
        f.assert_original_read_only(&db, true);
    }
}
