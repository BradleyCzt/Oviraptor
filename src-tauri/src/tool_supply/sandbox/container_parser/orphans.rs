//! The private durable journal owns only one labelled container, never a DB or
//! another live guardian. Kernel file locks survive neither exit nor SIGKILL.
use super::*;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    name: String,
    nonce: String,
    container_id: Option<String>,
    runtime_digest: String,
}

impl OwnedContainer<'_> {
    pub(super) fn save_journal(&self) -> Result<(), String> {
        let value = Journal {
            schema_version: 1,
            name: self.name.clone(),
            nonce: self.nonce.clone(),
            container_id: self.created_id.clone(),
            runtime_digest: self.runtime.digest.clone(),
        };
        let next = self.scratch.path.join("container.next");
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&next).map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut file, &value).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&next, self.scratch.path.join("container.json")).map_err(|e| e.to_string())?;
        fs::File::open(&self.scratch.path)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

pub(super) fn create_lock(path: &std::path::Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path.join("owner.lock"))
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("container_parser_owner_lock_failed".into());
        }
    }
    Ok(file)
}
fn private_file(path: &std::path::Path) -> Result<fs::File, String> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|_| "container_parser_orphan_journal_unreadable")?;
    let meta = file
        .metadata()
        .map_err(|_| "container_parser_orphan_journal_unreadable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if !meta.is_file()
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.nlink() != 1
            || meta.mode() & 0o077 != 0
            || meta.len() > 4096
        {
            return Err("container_parser_orphan_journal_not_private".into());
        }
    }
    Ok(file)
}

impl Runtime {
    pub(crate) fn recover_orphans(&self) -> Result<Vec<Value>, String> {
        #[cfg(not(unix))]
        {
            return Err("unsupported_sandbox:container_parser_platform_unverified".into());
        }
        #[cfg(unix)]
        {
            use std::os::{fd::AsRawFd, unix::fs::MetadataExt};
            let mut receipts = Vec::new();
            for entry in fs::read_dir(std::env::temp_dir()).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let Some(suffix) = name.strip_prefix("oviraptor-parser-runtime-") else {
                    continue;
                };
                if !uuid::Uuid::parse_str(suffix).is_ok_and(|u| u.to_string() == suffix) {
                    continue;
                }
                let path = entry.path();
                if !path.join("container.json").exists() {
                    continue;
                }
                let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if !meta.is_dir()
                    || meta.file_type().is_symlink()
                    || meta.uid() != unsafe { libc::geteuid() }
                    || meta.mode() & 0o077 != 0
                {
                    return Err("container_parser_orphan_directory_not_private".into());
                }
                let lock = private_file(&path.join("owner.lock"))?;
                if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == std::io::ErrorKind::WouldBlock {
                        continue;
                    }
                    return Err("container_parser_orphan_lock_failed".into());
                }
                let mut file = private_file(&path.join("container.json"))?;
                let mut bytes = Vec::new();
                Read::by_ref(&mut file)
                    .take(4097)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 4096 {
                    return Err("container_parser_orphan_journal_too_large".into());
                }
                let journal: Journal = serde_json::from_slice(&bytes)
                    .map_err(|_| "container_parser_orphan_journal_invalid")?;
                if journal.schema_version != 1
                    || !uuid::Uuid::parse_str(&journal.nonce)
                        .is_ok_and(|u| u.to_string() == journal.nonce)
                    || journal.name != format!("oviraptor-parser-{}", journal.nonce)
                    || !journal
                        .runtime_digest
                        .strip_prefix("sha256:")
                        .is_some_and(|s| {
                            s.len() == 64
                                && s.bytes()
                                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                        })
                    || journal.container_id.as_ref().is_none_or(|id| {
                        id.len() != 64
                            || !id
                                .bytes()
                                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                    })
                {
                    // The create outcome was never acknowledged: preserve this
                    // record and report uncertainty; never retry its code.
                    return Err("container_parser_orphan_creation_or_runtime_unverified".into());
                }
                let scratch = Scratch { path, _lock: lock };
                let id = journal.container_id.clone().unwrap();
                let mut owned = OwnedContainer {
                    runtime: self,
                    scratch: &scratch,
                    name: journal.name,
                    nonce: journal.nonce,
                    created_id: journal.container_id,
                    cleaned: false,
                };
                let present = self.command(
                    &scratch,
                    &["container", "inspect", &id],
                    None,
                    8192,
                    Duration::from_secs(5),
                    &|| false,
                )?;
                if !present.success
                    && present.stop_reason.is_empty()
                    && String::from_utf8_lossy(&present.stderr).contains("No such container:")
                {
                    fs::remove_file(scratch.path.join("container.json"))
                        .map_err(|e| e.to_string())?;
                    owned.cleaned = true;
                } else {
                    owned.kill()?;
                    owned.cleanup()?;
                }
                receipts.push(json!({"containerId":id,"originalRuntimeDigest":journal.runtime_digest,"cleanupVerified":true,"executionReplayed":false,"reason":"orphaned_guardian"}));
            }
            Ok(receipts)
        }
    }
}
