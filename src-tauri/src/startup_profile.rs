//! Opt-in desktop acceptance profile. Normal launches continue to use the
//! user's existing Oviraptor directory and legacy migration path.

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

const ACCEPTANCE_ENV: &str = "OVIRAPTOR_ACCEPTANCE_ROOT";

pub(crate) fn acceptance_root() -> Result<Option<PathBuf>, String> {
    resolve_acceptance_root(std::env::var_os(ACCEPTANCE_ENV), &std::env::temp_dir())
}

/// Reject an invalid opt-in profile before AppKit/Tauri starts. A setup error
/// after the native event loop begins is treated as a crash by macOS.
pub(crate) fn preflight(identifier: &str) -> Result<(), String> {
    let root = acceptance_root()?;
    require_bundle_root(identifier, root.as_deref())?;
    if let Some(root) = root {
        isolated_directories(&root)?;
    }
    Ok(())
}

fn require_bundle_root(identifier: &str, root: Option<&Path>) -> Result<(), String> {
    if identifier == "com.oviraptor.nest.acceptance" && root.is_none() {
        return Err("acceptance_bundle_requires_isolated_root".into());
    }
    Ok(())
}

fn resolve_acceptance_root(
    value: Option<OsString>,
    temp_dir: &Path,
) -> Result<Option<PathBuf>, String> {
    let Some(value) = value else { return Ok(None) };
    let requested = PathBuf::from(value);
    if !requested.is_absolute() {
        return Err("acceptance_profile_requires_absolute_path".into());
    }
    let metadata = fs::symlink_metadata(&requested).map_err(|_| "acceptance_profile_missing")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("acceptance_profile_not_private_directory".into());
    }
    let root = requested
        .canonicalize()
        .map_err(|_| "acceptance_profile_missing")?;
    let temp = temp_dir
        .canonicalize()
        .map_err(|_| "acceptance_profile_temp_unavailable")?;
    if root.parent() != Some(temp.as_path())
        || !root
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("oviraptor-acceptance-"))
    {
        return Err("acceptance_profile_must_be_direct_private_temp_child".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("acceptance_profile_permissions_not_private".into());
        }
    }
    Ok(Some(root))
}

fn private_child(root: &Path, name: &str) -> Result<PathBuf, String> {
    let child = root.join(name);
    match fs::symlink_metadata(&child) {
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err("acceptance_profile_child_not_directory".into())
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err("acceptance_profile_child_unavailable".into())
        }
        _ => {}
    }
    fs::create_dir_all(&child).map_err(|_| "acceptance_profile_child_unavailable")?;
    if child
        .canonicalize()
        .map_err(|_| "acceptance_profile_child_unavailable")?
        .parent()
        != Some(root)
    {
        return Err("acceptance_profile_child_escaped".into());
    }
    Ok(child)
}

pub(crate) fn isolated_directories(root: &Path) -> Result<(PathBuf, PathBuf), String> {
    let data = private_child(root, "oviraptor")?;
    // Do not let a reused profile redirect SQLite writes to a live user DB.
    for name in [
        "oviraptor.sqlite3",
        "oviraptor.sqlite3-wal",
        "oviraptor.sqlite3-shm",
    ] {
        match fs::symlink_metadata(data.join(name)) {
            Ok(entry) => {
                if entry.file_type().is_symlink() {
                    return Err("acceptance_profile_database_symlink".into());
                }
                if !entry.is_file() {
                    return Err("acceptance_profile_database_not_file".into());
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if entry.nlink() != 1 {
                        return Err("acceptance_profile_database_hardlink".into());
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("acceptance_profile_database_unavailable".into()),
        }
    }
    let exports = private_child(root, "exports")?;
    Ok((data, exports))
}

#[cfg(test)]
#[path = "startup_profile_tests.rs"]
mod tests;
