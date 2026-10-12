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

/// Entry wiring shared by the App restart hook and the new homogeneous
/// execution hook. Only journal-attributed containers whose owner lock is
/// free are reclaimed: a guardian still holding its flock is a claimed
/// program and is skipped, never replayed. Every outcome is reported through
/// `log` and the return value; both wired entries treat failure as
/// non-fatal so an unavailable runtime cannot block startup or a run.
pub(crate) fn recover_orphans_once(scope: &str, log: &dyn Fn(&str)) -> Result<Vec<Value>, String> {
    match Runtime::inspect().and_then(|runtime| runtime.recover_orphans()) {
        Ok(receipts) => {
            log(&format!(
                "container_parser_orphan_recovery:{scope}:recovered={}",
                serde_json::to_string(&receipts).unwrap_or_else(|_| "[]".into())
            ));
            Ok(receipts)
        }
        Err(error) => {
            log(&format!(
                "container_parser_orphan_recovery:{scope}:failed:{error}"
            ));
            Err(error)
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod entry_recovery_tests {
    use super::*;

    fn write_private_journal(path: &std::path::Path, bytes: &[u8]) {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .expect("private journal created");
        file.write_all(bytes).expect("private journal written");
    }

    fn journal_bytes(nonce: &str, container_id: Option<&str>, digest: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schemaVersion":1,
            "name":format!("oviraptor-parser-{nonce}"),
            "nonce":nonce,
            "containerId":container_id,
            "runtimeDigest":digest
        }))
        .expect("journal encoded")
    }

    fn create_labelled_container(runtime: &Runtime, scratch: &Scratch, nonce: &str) -> String {
        let name = format!("oviraptor-parser-{nonce}");
        let label = format!("{OWN_LABEL}={nonce}");
        let created = runtime
            .command(
                scratch,
                &["create", "--pull=never", "--name", &name, "--label", &label, IMAGE],
                None,
                8192,
                Duration::from_secs(15),
                &|| false,
            )
            .expect("container created");
        assert!(
            created.success && created.stop_reason.is_empty(),
            "{}",
            String::from_utf8_lossy(&created.stderr)
        );
        String::from_utf8(created.stdout)
            .expect("container id decoded")
            .trim()
            .to_string()
    }

    #[test]
    fn entry_recovery_reclaims_only_unclaimed_journal_orphans() {
        let runtime = Runtime::inspect().unwrap();
        // A claimed program: the live owner keeps its flock, so recovery must
        // not touch its container or its journal.
        let claimed = Scratch::new().unwrap();
        let claimed_nonce = uuid::Uuid::new_v4().to_string();
        let claimed_id = create_labelled_container(&runtime, &claimed, &claimed_nonce);
        write_private_journal(
            &claimed.path.join("container.json"),
            &journal_bytes(&claimed_nonce, Some(&claimed_id), runtime.digest()),
        );
        // A real orphan: the owning guardian is gone, its lock is free.
        let orphan = Scratch::new().unwrap();
        let orphan_nonce = uuid::Uuid::new_v4().to_string();
        let orphan_id = create_labelled_container(&runtime, &orphan, &orphan_nonce);
        let orphan_path = orphan.path.clone();
        write_private_journal(
            &orphan.path.join("container.json"),
            &journal_bytes(&orphan_nonce, Some(&orphan_id), runtime.digest()),
        );
        drop(orphan);
        // Unattributed neighbours are ignored, never treated as errors.
        let stranger = std::env::temp_dir().join("oviraptor-parser-runtime-not-a-uuid");
        let _ = fs::remove_dir_all(&stranger);
        let _ = fs::remove_file(&stranger);
        fs::create_dir(&stranger).unwrap();
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let receipts =
            recover_orphans_once("entry_test", &|line| logs.borrow_mut().push(line.to_string()))
                .expect("entry recovery succeeds");
        assert!(receipts.iter().any(|receipt| {
            receipt["containerId"] == orphan_id.as_str()
                && receipt["executionReplayed"] == false
                && receipt["cleanupVerified"] == true
                && receipt["reason"] == "orphaned_guardian"
        }));
        assert!(!receipts
            .iter()
            .any(|receipt| receipt["containerId"] == claimed_id.as_str()));
        let removed = runtime
            .command(
                &claimed,
                &["container", "inspect", &orphan_id],
                None,
                8192,
                Duration::from_secs(5),
                &|| false,
            )
            .unwrap();
        assert!(!removed.success);
        assert!(
            String::from_utf8_lossy(&removed.stderr).contains("No such container:"),
            "orphan container must be removed"
        );
        assert!(!orphan_path.exists(), "orphan journal directory reclaimed");
        let untouched = runtime
            .command(
                &claimed,
                &["container", "inspect", &claimed_id],
                None,
                512 * 1024,
                Duration::from_secs(5),
                &|| false,
            )
            .unwrap();
        assert!(untouched.success, "claimed container must survive");
        assert!(claimed
            .path
            .join("container.json")
            .exists());
        let messages = logs.borrow();
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0]
                .starts_with("container_parser_orphan_recovery:entry_test:recovered=[")
        );
        // Teardown of the claimed fixture.
        let _ = runtime.command(
            &claimed,
            &["rm", "--force", &claimed_id],
            None,
            8192,
            Duration::from_secs(5),
            &|| false,
        );
        fs::remove_file(claimed.path.join("container.json")).unwrap();
        fs::remove_dir_all(&stranger).unwrap();
    }

    #[test]
    fn entry_recovery_failure_is_reported_and_preserves_the_record() {
        let runtime = Runtime::inspect().unwrap();
        let unacknowledged = Scratch::new().unwrap();
        let preserved = unacknowledged.path.clone();
        // The guardian died between the pre-create and post-create journal
        // writes: the create outcome was never acknowledged.
        write_private_journal(
            &unacknowledged.path.join("container.json"),
            &journal_bytes(&uuid::Uuid::new_v4().to_string(), None, runtime.digest()),
        );
        drop(unacknowledged);
        let logs = std::cell::RefCell::new(Vec::<String>::new());
        let error = recover_orphans_once("entry_failure_test", &|line| {
            logs.borrow_mut().push(line.to_string())
        })
        .expect_err("unacknowledged journal must be reported, not replayed");
        assert_eq!(
            error,
            "container_parser_orphan_creation_or_runtime_unverified"
        );
        let messages = logs.borrow();
        assert_eq!(
            messages[0],
            "container_parser_orphan_recovery:entry_failure_test:failed:container_parser_orphan_creation_or_runtime_unverified"
        );
        // Uncertainty is preserved for reconciliation, never auto-discarded.
        assert!(preserved.join("container.json").is_file());
        fs::remove_file(preserved.join("container.json")).unwrap();
        fs::remove_dir_all(&preserved).unwrap();
    }
}
