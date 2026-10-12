use super::*;

fn private_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("oviraptor-acceptance-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    }
    root
}

#[test]
fn normal_launch_has_no_acceptance_profile() {
    assert_eq!(
        resolve_acceptance_root(None, &std::env::temp_dir()).unwrap(),
        None
    );
}

#[test]
fn acceptance_bundle_requires_isolation_before_creating_data_directories() {
    assert_eq!(
        require_bundle_root("com.oviraptor.nest.acceptance", None).unwrap_err(),
        "acceptance_bundle_requires_isolated_root"
    );
    assert!(require_bundle_root("com.oviraptor.desktop", None).is_ok());
    let root = private_root();
    let verified = resolve_acceptance_root(
        Some(root.clone().into_os_string()),
        &std::env::temp_dir(),
    )
    .unwrap()
    .unwrap();
    assert!(require_bundle_root("com.oviraptor.nest.acceptance", Some(&verified)).is_ok());
    assert!(!root.join("oviraptor").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn private_temp_profile_separates_database_exports_and_legacy_sources() {
    let root = private_root();
    let selected =
        resolve_acceptance_root(Some(root.clone().into_os_string()), &std::env::temp_dir())
            .unwrap()
            .unwrap();
    let (data, exports) = isolated_directories(&selected).unwrap();
    assert_eq!(data, selected.join("oviraptor"));
    assert_eq!(exports, selected.join("exports"));
    assert_eq!(
        isolated_directories(&selected).unwrap(),
        (data.clone(), exports)
    );
    assert!(!data.join("oviraptor.sqlite3").exists());
    fs::write(data.join("oviraptor.sqlite3"), b"existing private database").unwrap();
    assert_eq!(
        isolated_directories(&selected).unwrap(),
        (data, selected.join("exports"))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn profile_rejects_relative_outside_or_shared_directory_without_writes() {
    let root = private_root();
    assert!(resolve_acceptance_root(
        Some(OsString::from("oviraptor-acceptance-relative")),
        &std::env::temp_dir()
    )
    .is_err());
    let outside = root.parent().unwrap().parent().unwrap();
    assert!(resolve_acceptance_root(
        Some(outside.as_os_str().to_os_string()),
        &std::env::temp_dir()
    )
    .is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            resolve_acceptance_root(Some(root.clone().into_os_string()), &std::env::temp_dir())
                .unwrap_err(),
            "acceptance_profile_permissions_not_private"
        );
    }
    assert!(!root.join("oviraptor").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn profile_rejects_redirected_root_database_and_export_directory() {
    use std::os::unix::fs::symlink;
    let root = private_root();
    let selected =
        resolve_acceptance_root(Some(root.clone().into_os_string()), &std::env::temp_dir())
            .unwrap()
            .unwrap();
    let alias = root
        .parent()
        .unwrap()
        .join(format!("oviraptor-acceptance-{}", uuid::Uuid::new_v4()));
    symlink(&root, &alias).unwrap();
    assert!(
        resolve_acceptance_root(Some(alias.clone().into_os_string()), &std::env::temp_dir())
            .is_err()
    );
    fs::remove_file(alias).unwrap();

    let data = root.join("oviraptor");
    fs::create_dir(&data).unwrap();
    let victim = root.join("victim.db");
    fs::write(&victim, b"original").unwrap();
    symlink(&victim, data.join("oviraptor.sqlite3")).unwrap();
    assert_eq!(
        isolated_directories(&selected).unwrap_err(),
        "acceptance_profile_database_symlink"
    );
    assert_eq!(fs::read(&victim).unwrap(), b"original");
    fs::remove_file(data.join("oviraptor.sqlite3")).unwrap();
    symlink(&data, root.join("exports")).unwrap();
    assert_eq!(
        isolated_directories(&selected).unwrap_err(),
        "acceptance_profile_child_not_directory"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn profile_rejects_hardlinked_database_without_touching_its_source() {
    let root = private_root();
    let source_root = private_root();
    let selected =
        resolve_acceptance_root(Some(root.clone().into_os_string()), &std::env::temp_dir())
            .unwrap()
            .unwrap();
    let (data, _) = isolated_directories(&selected).unwrap();
    let source = source_root.join("user.sqlite3");
    fs::write(&source, b"original user database").unwrap();
    fs::hard_link(&source, data.join("oviraptor.sqlite3")).unwrap();
    assert_eq!(
        isolated_directories(&selected).unwrap_err(),
        "acceptance_profile_database_hardlink"
    );
    assert_eq!(fs::read(source).unwrap(), b"original user database");
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(source_root).unwrap();
}
