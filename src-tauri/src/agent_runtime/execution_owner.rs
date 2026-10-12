//! Shared OS ownership for foreground invocations and background parents.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    path::Path,
};

/// Live caller ownership, separate from the reusable Coordinator capability
/// lease. Never unlink the file: replacing a locked inode would admit two owners.
/// OS release on process death is NOT a receipt that target work can be retried;
/// durable dispatch journals and attempt fencing still govern recovery.
#[derive(Debug)]
pub(crate) struct NativeInvocationOwner(File);

pub(crate) fn claim_native_invocation(
    db_path: &Path,
    scan_id: &str,
    attempt: i64,
    kind: &str,
    target: &str,
) -> Result<NativeInvocationOwner, String> {
    let (directory, path) = invocation_path(db_path, scan_id, attempt, kind, target)?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("native_invocation_directory:{error}"))?;
    open_owner(&directory, &path, true, false)
}

/// Serialize approval writers before they acquire a SQLite write transaction.
/// This database-wide queue also covers independent processes. It grants no
/// execution ownership; the original transaction and receipt checks still run.
pub(crate) fn queue_recovery_approval(db_path: &Path) -> Result<NativeInvocationOwner, String> {
    let (directory, path) = invocation_path(db_path, "", 0, "recovery_approval", "database")?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("native_invocation_directory:{error}"))?;
    open_owner(&directory, &path, true, true)
}

/// Probe only the existing original invocation inode. Absence is not an exit
/// receipt, and this function never creates a directory or a lock file.
pub(crate) fn probe_native_invocation(
    db_path: &Path, scan_id: &str, attempt: i64, kind: &str, target: &str,
) -> Result<Option<NativeInvocationOwner>, String> {
    let (directory, path) = invocation_path(db_path, scan_id, attempt, kind, target)?;
    for entry in [&directory, &path] {
        match fs::symlink_metadata(entry) {
            Ok(metadata) if entry == &directory && (!metadata.is_dir() || metadata.file_type().is_symlink()) => {
                return Err("native_invocation_directory_not_regular".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("native_invocation_probe:{error}")),
        }
    }
    open_owner(&directory, &path, false, false).map(Some)
}

fn invocation_path(
    db_path: &Path, scan_id: &str, attempt: i64, kind: &str, target: &str,
) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    let canonical =
        fs::canonicalize(db_path).map_err(|error| format!("native_invocation_database:{error}"))?;
    // JSON framing prevents ambiguous concatenations; no user-controlled path
    // segment, credential, URL or scan name is written to the lock filename.
    let key =
        serde_json::to_vec(&(scan_id, attempt, kind, target)).map_err(|error| error.to_string())?;
    let digest = format!("{:x}", Sha256::digest(key));
    let mut directory_name = canonical
        .file_name()
        .ok_or("native_invocation_database_name")?
        .to_os_string();
    directory_name.push(".invocations");
    let directory = canonical.with_file_name(directory_name);
    let path = directory.join(format!("{digest}.lock"));
    Ok((directory, path))
}

fn open_owner(directory: &Path, path: &Path, create: bool, wait: bool) -> Result<NativeInvocationOwner, String> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("native_invocation_directory_not_regular".into());
    }
    let mut options = OpenOptions::new();
    options.create(create).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    if fs::symlink_metadata(path)
        .is_ok_and(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
    {
        return Err("native_invocation_lock_not_regular".into());
    }
    let lock = options
        .open(path)
        .map_err(|error| format!("native_invocation_open:{error}"))?;
    if !lock
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("native_invocation_lock_not_regular".into());
    }
    if wait {
        lock.lock().map_err(|error| format!("native_invocation_queue:{error}"))?;
    } else {
        lock.try_lock()
            .map_err(|error| format!("native_invocation_not_owned:{error}"))?;
    }
    Ok(NativeInvocationOwner(lock))
}

impl Drop for NativeInvocationOwner {
    fn drop(&mut self) {
        // Explicit field use documents lifetime and avoids accidentally replacing
        // this RAII type with a path-only marker in a later refactor.
        let _ = self.0.unlock();
    }
}

#[cfg(test)]
#[path = "execution_owner_tests.rs"]
mod tests;
