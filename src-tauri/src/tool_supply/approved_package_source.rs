//! Current approved cache bytes, held by their original filesystem entities.
//! Approval currently stores no package: absent sources fail, without staging,
//! repair, extraction, execution, or accepting a caller-supplied path.
use super::{
    approval, candidate_cache_key, ToolCapabilityManifest, MAX_MANIFEST_BYTES,
    MAX_OPAQUE_PACKAGE_BYTES, MAX_STREAMED_PACKAGE_BYTES,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub(crate) struct ApprovedPackageSource {
    approval_id: String,
    approval_hash: String,
    manifest: ToolCapabilityManifest,
    canonical_manifest: String,
    configuration_digest: String,
    directories: Vec<HeldDirectory>,
    manifest_file: File,
    manifest_identity: FileIdentity,
    package_file: File,
    package_identity: FileIdentity,
}
struct HeldDirectory {
    path: PathBuf,
    file: File,
    identity: FileIdentity,
    private: bool,
}
#[derive(Clone, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    mode: u32,
    links: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
#[cfg(unix)]
fn identity(metadata: &fs::Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
        mode: metadata.mode(),
        links: metadata.nlink(),
        length: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    }
}
#[cfg(not(unix))]
fn identity(_: &fs::Metadata) -> FileIdentity {
    unreachable!("non-Unix admission always fails")
}
fn check(metadata: &fs::Metadata, directory: bool, private: bool) -> Result<(), String> {
    #[cfg(not(unix))]
    {
        let _ = (metadata, directory, private);
        return Err("unsupported_approved_package_source_platform".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & if private { 0o077 } else { 0o022 } != 0
            || metadata.mode() & if directory { 0o500 } else { 0o400 }
                != if directory { 0o500 } else { 0o400 }
            || if directory {
                !metadata.is_dir()
            } else {
                !metadata.is_file() || metadata.nlink() != 1
            }
        {
            return Err("approved_package_source_permissions".into());
        }
        Ok(())
    }
}
fn require_no_symlink_ancestors(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("approved_package_source_directory_invalid".into());
    }
    let mut prefix = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::RootDir | std::path::Component::Normal(_) => {
                prefix.push(component.as_os_str())
            }
            _ => return Err("approved_package_source_directory_invalid".into()),
        }
        let metadata =
            fs::symlink_metadata(&prefix).map_err(|error| match error.raw_os_error() {
                Some(libc::ENOENT) => {
                    "approved_package_source_configuration_directory_missing".into()
                }
                Some(code) => format!("approved_package_source_open_failed:{code}"),
                None => "approved_package_source_open_failed:unknown".into(),
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("approved_package_source_directory_changed".into());
        }
    }
    Ok(())
}
fn open_base(path: &Path) -> Result<File, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)
            .map_err(|error| match error.raw_os_error() {
                Some(libc::ENOENT) => {
                    "approved_package_source_configuration_directory_missing".into()
                }
                Some(code) => format!("approved_package_source_open_failed:{code}"),
                None => "approved_package_source_open_failed:unknown".into(),
            })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("unsupported_approved_package_source_platform".into())
    }
}
fn open_child(parent: &File, name: &str, directory: bool) -> Result<File, String> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let name =
            std::ffi::CString::new(name).map_err(|_| "approved_package_source_name_invalid")?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if directory { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOENT) => "approved_package_source_missing".into(),
                Some(code) => format!("approved_package_source_open_failed:{code}"),
                None => "approved_package_source_open_failed:unknown".into(),
            });
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    #[cfg(not(unix))]
    {
        let _ = (parent, name, directory);
        Err("unsupported_approved_package_source_platform".into())
    }
}
fn same_directory(a: &FileIdentity, b: &FileIdentity) -> bool {
    a.device == b.device && a.inode == b.inode && a.owner == b.owner && a.mode == b.mode
}
fn read_manifest(
    file: &mut File,
    configuration: &approval::ToolAdministratorConfiguration,
    expected: &ToolCapabilityManifest,
) -> Result<(String, String), String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "approved_package_source_read_failed")?;
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "approved_package_source_read_failed")?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err("approved_package_source_manifest_too_large".into());
    }
    let value: super::SignedToolManifest =
        serde_json::from_slice(&bytes).map_err(|_| "approved_package_source_manifest_invalid")?;
    let (verified, digest) =
        approval::verify_package_manifest_on(configuration, &bytes, &expected.platform)?;
    if &verified != expected {
        return Err("approved_package_source_manifest_changed".into());
    }
    Ok((
        serde_json::to_string(&value).map_err(|_| "approved_package_source_manifest_invalid")?,
        digest,
    ))
}
fn package_hash(file: &mut File) -> Result<(String, u64), String> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "approved_package_source_read_failed")?;
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = file
            .read(&mut buffer)
            .map_err(|_| "approved_package_source_read_failed")?;
        if size == 0 {
            break;
        }
        total = total
            .checked_add(size as u64)
            .ok_or("approved_package_source_too_large")?;
        if total > MAX_STREAMED_PACKAGE_BYTES {
            return Err("approved_package_source_too_large".into());
        }
        hash.update(&buffer[..size]);
    }
    Ok((format!("sha256:{:x}", hash.finalize()), total))
}
impl ApprovedPackageSource {
    pub(crate) fn open_on(
        db: &Connection,
        configuration: &approval::ToolAdministratorConfiguration,
        approval_id: &str,
    ) -> Result<Self, String> {
        if cfg!(not(unix)) {
            return Err("unsupported_approved_package_source_platform".into());
        }
        if db.is_autocommit() {
            let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return Self::open_on(&snapshot, configuration, approval_id);
        }
        let approved = approval::read_original_on(db, configuration, approval_id)?
            .ok_or("approved_package_source_approval_missing")?;
        let canonical_manifest:String=db.query_row("SELECT canonical_manifest FROM main.tool_administrator_decisions WHERE decision_id=?1 AND kind='approve'",[approval_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        let root = configuration.original_package_cache_root();
        let base = root
            .parent()
            .ok_or("approved_package_source_directory_invalid")?;
        require_no_symlink_ancestors(base)?;
        let base_file = open_base(base)?;
        let metadata = base_file
            .metadata()
            .map_err(|_| "approved_package_source_metadata_failed")?;
        check(&metadata, true, false)?;
        let mut directories = vec![HeldDirectory {
            path: base.into(),
            file: base_file,
            identity: identity(&metadata),
            private: false,
        }];
        let key = candidate_cache_key(approved.manifest())?;
        for name in [
            "tool-supply-cache",
            approved.manifest().id.as_str(),
            key.as_str(),
        ] {
            let parent = directories
                .last()
                .ok_or("approved_package_source_directory_invalid")?;
            let file = open_child(&parent.file, name, true)?;
            let metadata = file
                .metadata()
                .map_err(|_| "approved_package_source_metadata_failed")?;
            check(&metadata, true, true)?;
            directories.push(HeldDirectory {
                path: parent.path.join(name),
                file,
                identity: identity(&metadata),
                private: true,
            });
        }
        let parent = &directories
            .last()
            .ok_or("approved_package_source_directory_invalid")?
            .file;
        let mut manifest_file = open_child(parent, "manifest.json", false)?;
        let package_file = open_child(parent, "package.blob", false)?;
        let metadata = manifest_file
            .metadata()
            .map_err(|_| "approved_package_source_metadata_failed")?;
        check(&metadata, false, false)?;
        let manifest_identity = identity(&metadata);
        if manifest_identity.length > MAX_MANIFEST_BYTES as u64 {
            return Err("approved_package_source_manifest_too_large".into());
        }
        let (cached_manifest, configuration_digest) =
            read_manifest(&mut manifest_file, configuration, approved.manifest())?;
        if cached_manifest != canonical_manifest {
            return Err("approved_package_source_manifest_changed".into());
        }
        let metadata = package_file
            .metadata()
            .map_err(|_| "approved_package_source_metadata_failed")?;
        check(&metadata, false, false)?;
        let package_identity = identity(&metadata);
        if package_identity.length > MAX_STREAMED_PACKAGE_BYTES {
            return Err("approved_package_source_too_large".into());
        }
        let mut result = Self {
            approval_id: approval_id.into(),
            approval_hash: approved.envelope_hash().into(),
            manifest: approved.manifest().clone(),
            canonical_manifest,
            configuration_digest,
            directories,
            manifest_file,
            manifest_identity,
            package_file,
            package_identity,
        };
        result.require_on(db, configuration)?;
        Ok(result)
    }
    pub(crate) fn manifest(&self) -> &ToolCapabilityManifest {
        &self.manifest
    }
    pub(crate) fn approval_id(&self) -> &str {
        &self.approval_id
    }
    pub(crate) fn package_size(&self) -> u64 {
        self.package_identity.length
    }
    fn require_entities(&self) -> Result<(), String> {
        require_no_symlink_ancestors(
            &self
                .directories
                .first()
                .ok_or("approved_package_source_directory_invalid")?
                .path,
        )?;
        for directory in &self.directories {
            let held = directory
                .file
                .metadata()
                .map_err(|_| "approved_package_source_metadata_failed")?;
            let located = fs::symlink_metadata(&directory.path)
                .map_err(|_| "approved_package_source_directory_changed")?;
            check(&held, true, directory.private)?;
            check(&located, true, directory.private)?;
            if !same_directory(&identity(&held), &directory.identity)
                || !same_directory(&identity(&located), &directory.identity)
            {
                return Err("approved_package_source_directory_changed".into());
            }
        }
        let parent = self
            .directories
            .last()
            .ok_or("approved_package_source_directory_invalid")?;
        let mut names = fs::read_dir(&parent.path)
            .map_err(|_| "approved_package_source_directory_changed")?
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "approved_package_source_directory_changed")?;
        names.sort();
        if names != ["manifest.json", "package.blob"] {
            return Err("approved_package_source_entries_changed".into());
        }
        for (name, file, original) in [
            (
                "manifest.json",
                &self.manifest_file,
                &self.manifest_identity,
            ),
            ("package.blob", &self.package_file, &self.package_identity),
        ] {
            let held = file
                .metadata()
                .map_err(|_| "approved_package_source_metadata_failed")?;
            let located = open_child(&parent.file, name, false)?
                .metadata()
                .map_err(|_| "approved_package_source_metadata_failed")?;
            check(&held, false, false)?;
            check(&located, false, false)?;
            if identity(&held) != *original || identity(&located) != *original {
                return Err("approved_package_source_file_changed".into());
            }
        }
        Ok(())
    }
    pub(crate) fn require_on(
        &mut self,
        db: &Connection,
        configuration: &approval::ToolAdministratorConfiguration,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.require_on(&snapshot, configuration);
        }
        let approved = approval::read_original_on(db, configuration, &self.approval_id)?
            .ok_or("approved_package_source_approval_missing")?;
        if approved.envelope_hash() != self.approval_hash
            || approved.manifest() != &self.manifest
            || configuration.original_package_cache_root()
                != self
                    .directories
                    .get(1)
                    .ok_or("approved_package_source_directory_invalid")?
                    .path
        {
            return Err("approved_package_source_approval_changed".into());
        }
        self.require_entities()?;
        let (cached_manifest, configuration_digest) =
            read_manifest(&mut self.manifest_file, configuration, &self.manifest)?;
        if cached_manifest != self.canonical_manifest
            || configuration_digest != self.configuration_digest
        {
            return Err("approved_package_source_manifest_changed".into());
        }
        let (digest, length) = package_hash(&mut self.package_file)?;
        if digest != self.manifest.digest || length != self.package_identity.length {
            return Err("approved_package_source_digest_changed".into());
        }
        self.require_entities()?;
        let confirmed = approval::read_original_on(db, configuration, &self.approval_id)?
            .ok_or("approved_package_source_approval_missing")?;
        if confirmed.envelope_hash() != self.approval_hash || confirmed.manifest() != &self.manifest
        {
            return Err("approved_package_source_approval_changed".into());
        }
        Ok(())
    }
    /// Parsing consumes bytes from this held verified entity, never a path or
    /// independently supplied byte vector. A bounded copy is checked again
    /// after reading; even a concurrent in-place writer cannot return new bytes.
    pub(crate) fn read_verified_bytes_on(
        &mut self,
        db: &Connection,
        configuration: &approval::ToolAdministratorConfiguration,
    ) -> Result<Vec<u8>, String> {
        if db.is_autocommit() {
            let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.read_verified_bytes_on(&snapshot, configuration);
        }
        self.require_on(db, configuration)?;
        if self.package_size() > MAX_OPAQUE_PACKAGE_BYTES as u64 {
            return Err("approved_package_source_parser_input_too_large".into());
        }
        self.package_file
            .seek(SeekFrom::Start(0))
            .map_err(|_| "approved_package_source_read_failed")?;
        let mut bytes = Vec::new();
        (&mut self.package_file)
            .take(MAX_OPAQUE_PACKAGE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "approved_package_source_read_failed")?;
        if bytes.len() > MAX_OPAQUE_PACKAGE_BYTES
            || bytes.len() as u64 != self.package_size()
            || format!("sha256:{:x}", Sha256::digest(&bytes)) != self.manifest.digest
        {
            return Err("approved_package_source_digest_changed".into());
        }
        self.require_on(db, configuration)?;
        Ok(bytes)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::tool_supply::frozen_plan::tests::TestSupplyFixture;
    use std::os::unix::fs::PermissionsExt;
    const PACKAGE: &[u8] = b"fixed offline parsing package";
    struct Fixture {
        root: PathBuf,
        path: PathBuf,
        supply: TestSupplyFixture,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // Only this fixture's inert cache is made removable; production
            // source readers never chmod or clean up a caller's package.
            fn make_removable(path: &Path) {
                if let Ok(metadata) = fs::symlink_metadata(path) {
                    if metadata.is_dir() && !metadata.file_type().is_symlink() {
                        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
                        if let Ok(entries) = fs::read_dir(path) {
                            for entry in entries.flatten() {
                                make_removable(&entry.path());
                            }
                        }
                    }
                }
            }
            make_removable(&self.supply.configuration().original_package_cache_root());
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn fixture(publish: bool) -> Fixture {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "oviraptor-approved-source-{}",
            uuid::Uuid::new_v4()
        ));
        let path = crate::db::initialize(&root).unwrap();
        let supply = TestSupplyFixture::approve_on(&path).unwrap();
        if publish {
            let db = crate::db::open(&path).unwrap();
            let (decision,manifest):(String,String)=db.query_row("SELECT canonical_envelope,canonical_manifest FROM tool_administrator_decisions WHERE decision_id=?1",[supply.approval_id()],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            let preparation = crate::db::begin_environment_preparation(&path).unwrap();
            let tx = db.unchecked_transaction().unwrap();
            crate::tool_supply::approved_package_publish::publish_on(
                &tx,
                &preparation,
                supply.configuration(),
                decision.as_bytes(),
                manifest.as_bytes(),
                PACKAGE,
            )
            .unwrap();
            tx.commit().unwrap();
            preparation.complete().unwrap();
        }
        Fixture { root, path, supply }
    }
    fn snapshot(db: &Connection) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
        let mut query = db
            .prepare("SELECT name FROM main.sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let names = query
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        names
            .into_iter()
            .map(|name| {
                let quoted = format!("\"{}\"", name.replace('"', "\"\""));
                let mut q = db.prepare(&format!("SELECT * FROM main.{quoted}")).unwrap();
                let count = q.column_count();
                let mut rows = q
                    .query_map([], |r| {
                        (0..count)
                            .map(|i| r.get(i))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                rows.sort_by_cached_key(|row| format!("{row:?}"));
                (name, rows)
            })
            .collect()
    }
    fn source(f: &Fixture, db: &Connection) -> ApprovedPackageSource {
        ApprovedPackageSource::open_on(db, f.supply.configuration(), f.supply.approval_id())
            .unwrap()
    }
    fn candidate(s: &ApprovedPackageSource) -> PathBuf {
        s.directories.last().unwrap().path.clone()
    }
    fn writable(directory: &Path) {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn readonly(directory: &Path) {
        fs::set_permissions(directory, fs::Permissions::from_mode(0o500)).unwrap();
    }
    #[test]
    fn approved_package_source_missing_cached_bytes_do_not_follow_an_approval() {
        let f = fixture(false);
        let db = crate::db::open(&f.path).unwrap();
        let before = snapshot(&db);
        assert_eq!(
            ApprovedPackageSource::open_on(&db, f.supply.configuration(), f.supply.approval_id())
                .err()
                .unwrap(),
            "approved_package_source_missing"
        );
        assert_eq!(snapshot(&db), before);
    }
    #[test]
    fn approved_package_source_actual_publication_reads_original_whole_bytes_without_writes() {
        let f = fixture(true);
        let db = crate::db::open(&f.path).unwrap();
        let before = snapshot(&db);
        let mut s = source(&f, &db);
        assert_eq!(s.approval_id(), f.supply.approval_id());
        assert_eq!(s.package_size(), PACKAGE.len() as u64);
        assert_eq!(
            s.manifest().digest,
            format!("sha256:{:x}", Sha256::digest(PACKAGE))
        );
        assert_eq!(
            s.read_verified_bytes_on(&db, f.supply.configuration())
                .unwrap(),
            PACKAGE
        );
        assert_eq!(snapshot(&db), before);
    }
    #[test]
    fn approved_package_source_replacement_extra_symlink_and_hardlink_fail_closed() {
        for fault in ["replacement", "extra", "symlink", "hardlink"] {
            let f = fixture(true);
            let db = crate::db::open(&f.path).unwrap();
            let mut s = source(&f, &db);
            let directory = candidate(&s);
            writable(&directory);
            match fault {
                "extra" => fs::write(directory.join("unexpected-entry"), b"inert").unwrap(),
                "replacement" => {
                    fs::remove_file(directory.join("package.blob")).unwrap();
                    fs::write(directory.join("package.blob"), PACKAGE).unwrap();
                    fs::set_permissions(
                        directory.join("package.blob"),
                        fs::Permissions::from_mode(0o400),
                    )
                    .unwrap();
                }
                "symlink" => {
                    let backup = f.root.join("original-package");
                    fs::copy(directory.join("package.blob"), &backup).unwrap();
                    fs::remove_file(directory.join("package.blob")).unwrap();
                    std::os::unix::fs::symlink(&backup, directory.join("package.blob")).unwrap();
                }
                "hardlink" => {
                    let backup = f.root.join("original-hardlink");
                    fs::hard_link(directory.join("package.blob"), &backup).unwrap();
                }
                _ => unreachable!(),
            }
            readonly(&directory);
            let before = snapshot(&db);
            assert!(
                s.read_verified_bytes_on(&db, f.supply.configuration())
                    .is_err(),
                "original held entity must reject {fault}"
            );
            if fault != "replacement" {
                assert!(
                    ApprovedPackageSource::open_on(
                        &db,
                        f.supply.configuration(),
                        f.supply.approval_id()
                    )
                    .is_err(),
                    "new source must reject {fault}"
                );
            }
            // New same-digest inode can be verified by a new independent
            // factory; it may never impersonate the original held source.
            assert_eq!(snapshot(&db), before);
        }
    }
    #[test]
    fn approved_package_source_cached_publisher_signature_damage_is_reverified() {
        let f = fixture(true);
        let db = crate::db::open(&f.path).unwrap();
        let s = source(&f, &db);
        let directory = candidate(&s);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.join("manifest.json")).unwrap()).unwrap();
        manifest["signatureBase64"] = serde_json::json!(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            [0u8; 64]
        ));
        writable(&directory);
        fs::remove_file(directory.join("manifest.json")).unwrap();
        fs::write(
            directory.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::set_permissions(
            directory.join("manifest.json"),
            fs::Permissions::from_mode(0o400),
        )
        .unwrap();
        readonly(&directory);
        let before = snapshot(&db);
        assert_eq!(
            ApprovedPackageSource::open_on(&db, f.supply.configuration(), f.supply.approval_id())
                .err()
                .unwrap(),
            "tool_signature_invalid"
        );
        assert_eq!(snapshot(&db), before);
    }
    #[test]
    fn approved_package_source_actual_admin_revocation_invalidates_held_source() {
        let f = fixture(true);
        let db = crate::db::open(&f.path).unwrap();
        let mut s = source(&f, &db);
        f.supply.revoke_on(&f.path).unwrap();
        let before = snapshot(&db);
        assert_eq!(
            s.read_verified_bytes_on(&db, f.supply.configuration())
                .err()
                .unwrap(),
            "tool_administrator_approval_revoked"
        );
        assert_eq!(snapshot(&db), before);
    }
    #[test]
    fn approved_package_source_symlink_and_unreadable_errors_are_not_cache_missing() {
        for fault in ["symlink", "unreadable"] {
            let f = fixture(true);
            let db = crate::db::open(&f.path).unwrap();
            let s = source(&f, &db);
            let directory = candidate(&s);
            let package = directory.join("package.blob");
            if fault == "symlink" {
                writable(&directory);
                let backup = f.root.join("actual-original-blob");
                fs::copy(&package, &backup).unwrap();
                fs::remove_file(&package).unwrap();
                std::os::unix::fs::symlink(&backup, &package).unwrap();
                readonly(&directory);
            } else {
                fs::set_permissions(&package, fs::Permissions::from_mode(0o000)).unwrap();
            }
            let before = snapshot(&db);
            let error = ApprovedPackageSource::open_on(
                &db,
                f.supply.configuration(),
                f.supply.approval_id(),
            )
            .err()
            .unwrap();
            assert_ne!(error, "approved_package_source_missing");
            if fault == "symlink" {
                assert_eq!(
                    error,
                    format!("approved_package_source_open_failed:{}", libc::ELOOP)
                );
            } else if unsafe { libc::geteuid() } == 0 {
                assert_eq!(error, "approved_package_source_permissions");
            } else {
                assert_eq!(
                    error,
                    format!("approved_package_source_open_failed:{}", libc::EACCES)
                );
            }
            assert_eq!(snapshot(&db), before);
        }
    }
    #[test]
    fn approved_package_source_parent_rebuilt_with_identical_bytes_rejects_held_entity() {
        let f = fixture(true);
        let db = crate::db::open(&f.path).unwrap();
        let before = snapshot(&db);
        let mut held = source(&f, &db);
        assert_eq!(
            held.read_verified_bytes_on(&db, f.supply.configuration())
                .unwrap(),
            PACKAGE
        );
        let directory = candidate(&held);
        let parent = directory.parent().unwrap();
        let backup = parent.join("original-held-candidate");
        writable(parent);
        fs::rename(&directory, &backup).unwrap();
        fs::create_dir(&directory).unwrap();
        for name in ["manifest.json", "package.blob"] {
            fs::copy(backup.join(name), directory.join(name)).unwrap();
            fs::set_permissions(directory.join(name), fs::Permissions::from_mode(0o400)).unwrap();
        }
        readonly(&directory);
        readonly(parent);
        assert_eq!(
            held.read_verified_bytes_on(&db, f.supply.configuration())
                .err()
                .unwrap(),
            "approved_package_source_directory_changed"
        );
        // A newly verified source can accept the original signed bytes, but
        // cannot make the previously held directory identify the new entity.
        let mut fresh = source(&f, &db);
        assert_eq!(
            fresh
                .read_verified_bytes_on(&db, f.supply.configuration())
                .unwrap(),
            PACKAGE
        );
        assert_eq!(snapshot(&db), before);
    }

    #[test]
    fn approved_package_source_in_place_bytes_restored_still_rejects_original_ctime() {
        let f = fixture(true);
        let db = crate::db::open(&f.path).unwrap();
        let before = snapshot(&db);
        let mut held = source(&f, &db);
        let package = candidate(&held).join("package.blob");
        let original = held.package_identity.clone();
        fs::set_permissions(&package, fs::Permissions::from_mode(0o600)).unwrap();
        let mut altered = PACKAGE.to_vec();
        altered[0] ^= 1;
        fs::write(&package, &altered).unwrap();
        fs::write(&package, PACKAGE).unwrap();
        fs::set_permissions(&package, fs::Permissions::from_mode(0o400)).unwrap();
        let restored = identity(&fs::metadata(&package).unwrap());
        assert_eq!(restored.device, original.device);
        assert_eq!(restored.inode, original.inode);
        assert_ne!(restored.changed, original.changed);
        assert_eq!(fs::read(&package).unwrap(), PACKAGE);
        assert_eq!(
            held.read_verified_bytes_on(&db, f.supply.configuration())
                .err()
                .unwrap(),
            "approved_package_source_file_changed"
        );
        let mut fresh = source(&f, &db);
        assert_eq!(
            fresh
                .read_verified_bytes_on(&db, f.supply.configuration())
                .unwrap(),
            PACKAGE
        );
        assert_eq!(snapshot(&db), before);
    }

    #[test]
    fn approved_package_source_temp_inventory_shadow_rejects_without_main_writes() {
        let f = fixture(true);
        let db = crate::db::open(&f.path).unwrap();
        let before = snapshot(&db);
        let mut held = source(&f, &db);
        db.execute_batch("CREATE TEMP TABLE tool_administrator_decisions AS SELECT * FROM main.tool_administrator_decisions").unwrap();
        assert_eq!(
            held.read_verified_bytes_on(&db, f.supply.configuration())
                .err()
                .unwrap(),
            "tool_administrator_inventory_schema_changed"
        );
        assert_eq!(
            ApprovedPackageSource::open_on(&db, f.supply.configuration(), f.supply.approval_id())
                .err()
                .unwrap(),
            "tool_administrator_inventory_schema_changed"
        );
        assert_eq!(snapshot(&db), before);
        db.execute_batch("DROP TABLE temp.tool_administrator_decisions")
            .unwrap();
        assert_eq!(
            held.read_verified_bytes_on(&db, f.supply.configuration())
                .unwrap(),
            PACKAGE
        );
        assert_eq!(snapshot(&db), before);
    }
}
