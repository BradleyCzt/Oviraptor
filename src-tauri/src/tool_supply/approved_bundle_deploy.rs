//! Task-external, exclusive publication of a verified new-format regular-file
//! tree. This grants no worker capability and exposes no executable pathname.
use super::{approval::ToolAdministratorConfiguration, approved_bundle};
use rusqlite::{Connection, Transaction};

pub(crate) struct VerifiedDeployedBundle {
    original: approved_bundle::ApprovedFlatBundle,
    content_key: String,
    #[cfg(target_os = "macos")]
    held: platform::HeldDeployment,
}
impl VerifiedDeployedBundle {
    pub(crate) fn approval_id(&self) -> &str {
        self.original.approval_id()
    }
    pub(crate) fn content_key(&self) -> &str {
        &self.content_key
    }
    pub(crate) fn metadata(&self) -> &super::package_format::VerifiedFlatBundle {
        self.original.metadata()
    }
    pub(crate) fn require_on(
        &mut self,
        db: &Connection,
        configuration: &ToolAdministratorConfiguration,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
            return self.require_on(&tx, configuration);
        }
        self.original.require_on(db, configuration)?;
        #[cfg(target_os = "macos")]
        {
            self.held.require(configuration, self.original.metadata())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err("unsupported_tool_bundle_exclusive_deploy_platform".into())
        }
    }
}
/// Pure current-origin tree proof. Absence of the fixed deployment destination
/// is the only None result; malformed or replaced entities are never repaired.
pub(crate) fn read_original_on(
    db: &Connection,
    configuration: &ToolAdministratorConfiguration,
    approval_id: &str,
) -> Result<Option<VerifiedDeployedBundle>, String> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        return read_original_on(&tx, configuration, approval_id);
    }
    let mut original = approved_bundle::open_on(db, configuration, approval_id)?;
    let content_key = super::candidate_cache_key(original.manifest())?;
    #[cfg(target_os = "macos")]
    {
        let Some(held) = platform::read_existing(configuration, &content_key, &original)? else {
            return Ok(None);
        };
        original.require_on(db, configuration)?;
        let mut proof = VerifiedDeployedBundle {
            original,
            content_key,
            held,
        };
        proof.require_on(db, configuration)?;
        Ok(Some(proof))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&mut original, content_key);
        Err("unsupported_tool_bundle_exclusive_deploy_platform".into())
    }
}
pub(crate) fn deploy_on(
    original_database: &Connection,
    preparation: &crate::db::EnvironmentPreparationLease,
    configuration: &ToolAdministratorConfiguration,
    approval_id: &str,
) -> Result<VerifiedDeployedBundle, String> {
    if !original_database.is_autocommit() {
        return Err("tool_bundle_deploy_outer_transaction_forbidden".into());
    }
    require_original_database(original_database)?;
    let path = original_database
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("tool_bundle_deploy_existing_database_required")?;
    let db = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|e| format!("tool_bundle_deploy_existing_database_open:{e}"))?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    db.authorizer(Some(|context: rusqlite::hooks::AuthContext<'_>| {
        use rusqlite::hooks::{AuthAction, Authorization};
        if context.accessor.is_some() {
            return Authorization::Deny;
        }
        let allow = match context.action {
            AuthAction::Read { table_name, .. } => {
                context.database_name == Some("main")
                    || (context.database_name == Some("temp") && table_name == "sqlite_temp_master")
            }
            AuthAction::Select
            | AuthAction::Function { .. }
            | AuthAction::Transaction { .. }
            | AuthAction::Savepoint { .. } => true,
            _ => false,
        };
        if allow {
            Authorization::Allow
        } else {
            Authorization::Deny
        }
    }))
    .map_err(|e| e.to_string())?;
    let changes = db.total_changes();
    let tx = Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        require_original_database(original_database)?;
        require_original_database(&tx)?;
        let deployed = deploy_in(&tx, preparation, configuration, approval_id)?;
        require_original_database(original_database)?;
        require_original_database(&tx)?;
        preparation.require_tool_supply_on(&tx)?;
        if tx.total_changes() != changes {
            return Err("tool_bundle_deploy_unexpected_database_write".into());
        }
        Ok(deployed)
    })();
    match result {
        Ok(deployed) => {
            tx.commit()
                .map_err(|e| format!("tool_bundle_deploy_commit:{e}"))?;
            Ok(deployed)
        }
        Err(error) => match tx.rollback() {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!(
                "{error};tool_bundle_deploy_rollback_failed:{rollback}"
            )),
        },
    }
}
fn require_original_database(db: &Connection) -> Result<(), String> {
    let mut moved: std::os::raw::c_int = 0;
    let result = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            db.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
            (&mut moved as *mut std::os::raw::c_int).cast(),
        )
    };
    if result != rusqlite::ffi::SQLITE_OK || moved != 0 {
        return Err("tool_bundle_deploy_original_database_changed".into());
    }
    Ok(())
}
fn deploy_in(
    tx: &Transaction<'_>,
    preparation: &crate::db::EnvironmentPreparationLease,
    configuration: &ToolAdministratorConfiguration,
    approval_id: &str,
) -> Result<VerifiedDeployedBundle, String> {
    preparation.require_tool_supply_on(tx)?;
    let mut original = approved_bundle::open_on(tx, configuration, approval_id)?;
    let content_key = super::candidate_cache_key(original.manifest())?;
    #[cfg(target_os = "macos")]
    {
        let held = platform::deploy(configuration, &content_key, &original)?;
        original.require_on(tx, configuration)?;
        preparation.require_tool_supply_on(tx)?;
        let mut result = VerifiedDeployedBundle {
            original,
            content_key,
            held,
        };
        result.require_on(tx, configuration)?;
        Ok(result)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (&mut original, content_key);
        Err("unsupported_tool_bundle_exclusive_deploy_platform".into())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use crate::tool_supply::package_format::{RegularFileMode, VerifiedFlatBundle};
    use sha2::{Digest, Sha256};
    use std::{
        collections::{BTreeMap, BTreeSet},
        ffi::{CStr, CString},
        fs::File,
        io::{Read, Seek, SeekFrom, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::MetadataExt,
        },
        path::{Component, Path},
    };
    #[derive(Clone, PartialEq, Eq)]
    struct Identity {
        device: u64,
        inode: u64,
        owner: u32,
        mode: u32,
        length: u64,
        links: u64,
        modified: (i64, i64),
        changed: (i64, i64),
    }
    fn identity(file: &File) -> Result<Identity, String> {
        let m = file.metadata().map_err(|e| e.to_string())?;
        Ok(Identity {
            device: m.dev(),
            inode: m.ino(),
            owner: m.uid(),
            mode: m.mode(),
            length: m.len(),
            links: m.nlink(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
    fn same_dir(a: &File, b: &File) -> Result<(), String> {
        let a = identity(a)?;
        let b = identity(b)?;
        if a.device != b.device || a.inode != b.inode {
            return Err("tool_bundle_deploy_directory_changed".into());
        }
        Ok(())
    }
    fn name(value: &str) -> Result<CString, String> {
        if value.is_empty() || matches!(value, "." | "..") || value.contains('/') {
            return Err("tool_bundle_deploy_name_invalid".into());
        }
        CString::new(value).map_err(|_| "tool_bundle_deploy_name_invalid".into())
    }
    fn open(parent: &File, value: &str, flags: i32, mode: u16) -> Result<File, String> {
        let name = name(value)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                u32::from(mode),
            )
        };
        if fd < 0 {
            return Err(format!(
                "tool_bundle_deploy_open:{}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    fn directory(file: &File, mode: Option<u32>) -> Result<(), String> {
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_dir()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o077 != 0
            || mode.is_some_and(|mode| m.mode() & 0o777 != mode)
        {
            return Err("tool_bundle_deploy_directory_permissions".into());
        }
        Ok(())
    }
    fn child_dir(parent: &File, value: &str, create: bool) -> Result<File, String> {
        if create {
            let n = name(value)?;
            if unsafe { libc::mkdirat(parent.as_raw_fd(), n.as_ptr(), 0o700) } != 0 {
                let e = std::io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::EEXIST) {
                    return Err(format!("tool_bundle_deploy_mkdir:{e}"));
                }
            } else {
                parent.sync_all().map_err(|e| e.to_string())?;
            }
        }
        let file = open(parent, value, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        directory(&file, None)?;
        Ok(file)
    }
    struct Anchor {
        components: Vec<String>,
        directories: Vec<File>,
    }
    impl Anchor {
        fn capture(path: &Path, create: bool) -> Result<Self, String> {
            if !path.is_absolute() {
                return Err("tool_bundle_deploy_root_invalid".into());
            }
            let mut components = Vec::new();
            let mut directories = vec![File::open("/").map_err(|e| e.to_string())?];
            let paths: Vec<_> = path.components().collect();
            for (i, c) in paths.iter().enumerate() {
                match c {
                    Component::RootDir => (),
                    Component::Normal(value) => {
                        let value = value.to_str().ok_or("tool_bundle_deploy_root_invalid")?;
                        let parent = directories
                            .last()
                            .ok_or("tool_bundle_deploy_root_invalid")?;
                        let next = if i + 1 == paths.len() {
                            child_dir(parent, value, create)?
                        } else {
                            open(parent, value, libc::O_RDONLY | libc::O_DIRECTORY, 0)?
                        };
                        components.push(value.into());
                        directories.push(next);
                    }
                    _ => return Err("tool_bundle_deploy_root_invalid".into()),
                }
            }
            directory(
                directories
                    .last()
                    .ok_or("tool_bundle_deploy_root_invalid")?,
                None,
            )?;
            let anchor = Self {
                components,
                directories,
            };
            anchor.require()?;
            Ok(anchor)
        }
        fn last(&self) -> Result<&File, String> {
            self.directories
                .last()
                .ok_or_else(|| "tool_bundle_deploy_root_invalid".into())
        }
        fn require(&self) -> Result<(), String> {
            let mut located = File::open("/").map_err(|e| e.to_string())?;
            same_dir(&located, &self.directories[0])?;
            for (i, value) in self.components.iter().enumerate() {
                located = open(&located, value, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
                same_dir(&located, &self.directories[i + 1])?;
            }
            directory(&located, None)
        }
    }
    fn names(dir: &File) -> Result<BTreeSet<String>, String> {
        let fd = unsafe { libc::dup(dir.as_raw_fd()) };
        if fd < 0 {
            return Err("tool_bundle_deploy_list_failed".into());
        }
        let stream = unsafe { libc::fdopendir(fd) };
        if stream.is_null() {
            unsafe { libc::close(fd) };
            return Err("tool_bundle_deploy_list_failed".into());
        }
        unsafe { libc::rewinddir(stream) };
        let result = (|| {
            let mut names = BTreeSet::new();
            loop {
                unsafe { *libc::__error() = 0 };
                let entry = unsafe { libc::readdir(stream) };
                if entry.is_null() {
                    if std::io::Error::last_os_error().raw_os_error() != Some(0) {
                        return Err("tool_bundle_deploy_list_failed".into());
                    }
                    break;
                }
                let value = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                    .to_str()
                    .map_err(|_| "tool_bundle_deploy_name_invalid")?;
                if matches!(value, "." | "..") {
                    continue;
                }
                if names.len() >= 128 {
                    return Err("tool_bundle_deploy_extra_entry".into());
                }
                names.insert(value.into());
            }
            Ok(names)
        })();
        if unsafe { libc::closedir(stream) } != 0 {
            return Err("tool_bundle_deploy_list_failed".into());
        }
        result
    }
    fn expected_tree(bundle: &VerifiedFlatBundle) -> BTreeMap<String, BTreeSet<String>> {
        let mut tree = BTreeMap::new();
        tree.insert(String::new(), BTreeSet::new());
        for entry in bundle.entries() {
            let mut parent = String::new();
            let parts: Vec<_> = entry.path().split('/').collect();
            for (i, part) in parts.iter().enumerate() {
                tree.entry(parent.clone())
                    .or_insert_with(BTreeSet::new)
                    .insert((*part).into());
                if i + 1 < parts.len() {
                    parent = if parent.is_empty() {
                        (*part).into()
                    } else {
                        format!("{parent}/{part}")
                    };
                    tree.entry(parent.clone()).or_insert_with(BTreeSet::new);
                }
            }
        }
        tree
    }
    fn relative_dir(root: &File, path: &str) -> Result<File, String> {
        let mut dir = root.try_clone().map_err(|e| e.to_string())?;
        if !path.is_empty() {
            for part in path.split('/') {
                dir = open(&dir, part, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
            }
        }
        Ok(dir)
    }
    fn final_mode(mode: RegularFileMode) -> u32 {
        match mode {
            RegularFileMode::ReadOnlyData => 0o400,
            RegularFileMode::EntryCode => 0o500,
        }
    }
    fn check_file(
        file: &mut File,
        entry: &crate::tool_supply::package_format::FlatBundleEntry,
    ) -> Result<Identity, String> {
        let before = identity(file)?;
        let m = file.metadata().map_err(|e| e.to_string())?;
        if !m.is_file()
            || before.owner != unsafe { libc::geteuid() }
            || before.links != 1
            || before.mode & 0o777 != final_mode(entry.mode())
            || before.length != entry.content_bytes()
        {
            return Err("tool_bundle_deploy_file_invalid".into());
        }
        file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            total = total
                .checked_add(u64::try_from(n).map_err(|_| "tool_bundle_deploy_size_invalid")?)
                .ok_or("tool_bundle_deploy_size_invalid")?;
            if total > entry.content_bytes() {
                return Err("tool_bundle_deploy_file_changed".into());
            }
            hash.update(&buffer[..n]);
        }
        let digest: [u8; 32] = hash.finalize().into();
        if total != entry.content_bytes()
            || digest != *entry.content_sha256()
            || identity(file)? != before
        {
            return Err("tool_bundle_deploy_file_changed".into());
        }
        Ok(before)
    }
    pub(super) struct HeldDeployment {
        anchor: Anchor,
        release: File,
        tree: Vec<(String, File)>,
        files: Vec<(String, File, Identity)>,
        id: String,
        key: String,
    }
    impl HeldDeployment {
        fn capture(
            anchor: Anchor,
            release: File,
            id: &str,
            key: &str,
            bundle: &VerifiedFlatBundle,
        ) -> Result<Self, String> {
            let root = child_dir(&release, key, false)?;
            let expected = expected_tree(bundle);
            let mut tree = Vec::new();
            for (path, children) in &expected {
                let dir = relative_dir(&root, path)?;
                directory(&dir, Some(0o500))?;
                if names(&dir)? != *children {
                    return Err("tool_bundle_deploy_tree_changed".into());
                }
                tree.push((path.clone(), dir));
            }
            let mut files = Vec::new();
            for entry in bundle.entries() {
                let (parent, leaf) = entry.path().rsplit_once('/').unwrap_or(("", entry.path()));
                let dir = relative_dir(&root, parent)?;
                let mut file = open(&dir, leaf, libc::O_RDONLY, 0)?;
                let original = check_file(&mut file, entry)?;
                files.push((entry.path().into(), file, original));
            }
            let result = Self {
                anchor,
                release,
                tree,
                files,
                id: id.into(),
                key: key.into(),
            };
            result.require_tree(bundle)?;
            Ok(result)
        }
        fn require_tree(&self, bundle: &VerifiedFlatBundle) -> Result<(), String> {
            self.anchor.require()?;
            let release = child_dir(self.anchor.last()?, &self.id, false)?;
            same_dir(&release, &self.release)?;
            let root = child_dir(&release, &self.key, false)?;
            let expected = expected_tree(bundle);
            if self.tree.len() != expected.len() || self.files.len() != bundle.entries().len() {
                return Err("tool_bundle_deploy_tree_changed".into());
            }
            for (path, held) in &self.tree {
                let located = relative_dir(&root, path)?;
                same_dir(held, &located)?;
                directory(&located, Some(0o500))?;
                if names(&located)?
                    != *expected
                        .get(path)
                        .ok_or("tool_bundle_deploy_tree_changed")?
                {
                    return Err("tool_bundle_deploy_tree_changed".into());
                }
            }
            for ((path, held, original), entry) in self.files.iter().zip(bundle.entries()) {
                if path != entry.path() || identity(held)? != *original {
                    return Err("tool_bundle_deploy_file_changed".into());
                }
                let (parent, leaf) = path.rsplit_once('/').unwrap_or(("", path));
                let dir = relative_dir(&root, parent)?;
                let mut located = open(&dir, leaf, libc::O_RDONLY, 0)?;
                if check_file(&mut located, entry)? != *original {
                    return Err("tool_bundle_deploy_file_changed".into());
                }
            }
            Ok(())
        }
        pub(super) fn require(
            &self,
            configuration: &ToolAdministratorConfiguration,
            bundle: &VerifiedFlatBundle,
        ) -> Result<(), String> {
            let configured =
                Anchor::capture(&configuration.original_deployed_package_root(), false)?;
            same_dir(configured.last()?, self.anchor.last()?)?;
            self.require_tree(bundle)
        }
    }
    fn chmod(file: &File, mode: u16) -> Result<(), String> {
        if unsafe { libc::fchmod(file.as_raw_fd(), libc::mode_t::from(mode)) } != 0 {
            return Err("tool_bundle_deploy_chmod_failed".into());
        }
        file.sync_all().map_err(|e| e.to_string())
    }
    fn exists(parent: &File, value: &str) -> Result<bool, String> {
        let n = name(value)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                n.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            return Ok(true);
        }
        if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            Ok(false)
        } else {
            Err("tool_bundle_deploy_destination_invalid".into())
        }
    }
    pub(super) fn read_existing(
        configuration: &ToolAdministratorConfiguration,
        key: &str,
        original: &approved_bundle::ApprovedFlatBundle,
    ) -> Result<Option<HeldDeployment>, String> {
        let path = configuration.original_deployed_package_root();
        let parent_path = path.parent().ok_or("tool_bundle_deploy_root_invalid")?;
        let parent = Anchor::capture(parent_path, false)?;
        let leaf = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("tool_bundle_deploy_root_invalid")?;
        if !exists(parent.last()?, leaf)? {
            parent.require()?;
            return Ok(None);
        }
        let anchor = Anchor::capture(&path, false)?;
        if !exists(anchor.last()?, &original.manifest().id)? {
            anchor.require()?;
            return Ok(None);
        }
        let release = child_dir(anchor.last()?, &original.manifest().id, false)?;
        if !exists(&release, key)? {
            anchor.require()?;
            let located = child_dir(anchor.last()?, &original.manifest().id, false)?;
            same_dir(&release, &located)?;
            return Ok(None);
        }
        HeldDeployment::capture(
            anchor,
            release,
            &original.manifest().id,
            key,
            original.metadata(),
        )
        .map(Some)
    }
    fn verify_staging(root: &File, bundle: &VerifiedFlatBundle) -> Result<(), String> {
        for (path, children) in expected_tree(bundle) {
            let directory = relative_dir(root, &path)?;
            self::directory(&directory, Some(0o500))?;
            if names(&directory)? != children {
                return Err("tool_bundle_deploy_tree_changed".into());
            }
        }
        for entry in bundle.entries() {
            let (parent, leaf) = entry.path().rsplit_once('/').unwrap_or(("", entry.path()));
            let directory = relative_dir(root, parent)?;
            let mut file = open(&directory, leaf, libc::O_RDONLY, 0)?;
            check_file(&mut file, entry)?;
        }
        Ok(())
    }
    pub(super) fn deploy(
        configuration: &ToolAdministratorConfiguration,
        key: &str,
        original: &approved_bundle::ApprovedFlatBundle,
    ) -> Result<HeldDeployment, String> {
        let anchor = Anchor::capture(&configuration.original_deployed_package_root(), true)?;
        let release = child_dir(anchor.last()?, &original.manifest().id, true)?;
        anchor.require()?;
        if exists(&release, key)? {
            return HeldDeployment::capture(
                anchor,
                release,
                &original.manifest().id,
                key,
                original.metadata(),
            );
        }
        let staged = format!(".staging-{}", uuid::Uuid::new_v4());
        let stage_name = name(&staged)?;
        if unsafe { libc::mkdirat(release.as_raw_fd(), stage_name.as_ptr(), 0o700) } != 0 {
            return Err("tool_bundle_deploy_stage_create_failed".into());
        }
        release.sync_all().map_err(|e| e.to_string())?;
        let staging = child_dir(&release, &staged, false)?;
        // Any error retains only this fresh owned staging tree and the original
        // preparation fence for investigation; no recursive foreign cleanup.
        let expected = expected_tree(original.metadata());
        let mut created = BTreeMap::<String, File>::new();
        created.insert(
            String::new(),
            staging.try_clone().map_err(|e| e.to_string())?,
        );
        for path in expected.keys().filter(|p| !p.is_empty()) {
            let (parent, leaf) = path.rsplit_once('/').unwrap_or(("", path));
            let parent = created
                .get(parent)
                .ok_or("tool_bundle_deploy_tree_changed")?;
            let n = name(leaf)?;
            if unsafe { libc::mkdirat(parent.as_raw_fd(), n.as_ptr(), 0o700) } != 0 {
                return Err("tool_bundle_deploy_directory_conflict".into());
            }
            parent.sync_all().map_err(|e| e.to_string())?;
            let dir = child_dir(parent, leaf, false)?;
            created.insert(path.clone(), dir);
        }
        for entry in original.metadata().entries() {
            let (parent, leaf) = entry.path().rsplit_once('/').unwrap_or(("", entry.path()));
            let parent = created
                .get(parent)
                .ok_or("tool_bundle_deploy_tree_changed")?;
            let mut file = open(
                parent,
                leaf,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            let start = usize::try_from(entry.content_offset())
                .map_err(|_| "tool_bundle_deploy_size_invalid")?;
            let end = start
                .checked_add(
                    usize::try_from(entry.content_bytes())
                        .map_err(|_| "tool_bundle_deploy_size_invalid")?,
                )
                .ok_or("tool_bundle_deploy_size_invalid")?;
            let bytes = original
                .original_bytes()
                .get(start..end)
                .ok_or("tool_bundle_deploy_size_invalid")?;
            let digest: [u8; 32] = Sha256::digest(bytes).into();
            if digest != *entry.content_sha256() {
                return Err("tool_bundle_deploy_file_changed".into());
            }
            file.write_all(bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            chmod(
                &file,
                u16::try_from(final_mode(entry.mode()))
                    .map_err(|_| "tool_bundle_deploy_mode_invalid")?,
            )?;
            parent.sync_all().map_err(|e| e.to_string())?;
        }
        for dir in created.values().rev() {
            chmod(dir, 0o500)?;
        }
        verify_staging(&staging, original.metadata())?;
        anchor.require()?;
        let located = child_dir(anchor.last()?, &original.manifest().id, false)?;
        same_dir(&release, &located)?;
        let original_stage = child_dir(&release, &staged, false)?;
        same_dir(&staging, &original_stage)?;
        let key_name = name(key)?;
        if unsafe {
            libc::renameatx_np(
                release.as_raw_fd(),
                stage_name.as_ptr(),
                release.as_raw_fd(),
                key_name.as_ptr(),
                libc::RENAME_EXCL,
            )
        } != 0
        {
            if std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) {
                return HeldDeployment::capture(
                    anchor,
                    release,
                    &original.manifest().id,
                    key,
                    original.metadata(),
                );
            }
            return Err(format!(
                "tool_bundle_deploy_exclusive_publish_failed:{}",
                std::io::Error::last_os_error()
            ));
        }
        release.sync_all().map_err(|e| e.to_string())?;
        HeldDeployment::capture(
            anchor,
            release,
            &original.manifest().id,
            key,
            original.metadata(),
        )
    }
}

#[cfg(all(test, target_os = "macos"))]
#[path = "approved_bundle_deploy/tests.rs"]
mod tests;
