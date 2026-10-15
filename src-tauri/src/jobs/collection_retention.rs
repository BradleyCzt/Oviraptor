//! Owned collection files only, never a general directory cleaner.
//!
//! The caller supplies the DB-verified run/project and keeps this writer alive
//! for the collection. All producers must publish through `write`; an admission
//! check followed by an unrelated write would not serialize the global budget.
//! Unregistered legacy runs, user data, analysis/audit tables and other job
//! stages are outside this policy. No analysis-snapshot pin business is implied.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const OWNER: &str = ".collection-retention-v1.json";
const WRITER: &str = ".collection-writer-v1.lock";
const GLOBAL: &str = ".collection-retention-v1.lock";
const MAX_METADATA: u64 = 8 * 1024 * 1024;
const MAX_FILES: usize = 16_384;
const MAX_HISTORY: usize = 4096;
static TEMPORARY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub global_bytes: u64,
    pub project_bytes: u64,
    pub run_bytes: u64,
    pub pages: usize,
    pub age_seconds: i64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            global_bytes: 2 * 1024 * 1024 * 1024,
            project_bytes: 1024 * 1024 * 1024,
            run_bytes: 512 * 1024 * 1024,
            pages: 4096,
            age_seconds: 7 * 24 * 60 * 60,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArtifactKind {
    Page,
    LegacyPage,
    Cache,
    Csv,
    Metadata,
    Inventory,
}
impl ArtifactKind {
    fn name(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::LegacyPage => "legacy_page",
            Self::Cache => "cache",
            Self::Csv => "csv",
            Self::Metadata => "metadata",
            Self::Inventory => "inventory",
        }
    }
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "page" => Ok(Self::Page),
            "legacy_page" => Ok(Self::LegacyPage),
            "cache" => Ok(Self::Cache),
            "csv" => Ok(Self::Csv),
            "metadata" => Ok(Self::Metadata),
            "inventory" => Ok(Self::Inventory),
            _ => Err("collection_retention_unknown_kind".into()),
        }
    }
    fn page(self) -> bool {
        matches!(self, Self::Page | Self::LegacyPage | Self::Cache)
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct RetentionReport {
    pub run_bytes: u64,
    pub project_bytes: u64,
    pub global_bytes: u64,
    pub stored_pages: usize,
    pub expired_files: u64,
    pub expired_bytes: u64,
    pub global_expired_files: u64,
    pub global_expired_bytes: u64,
    pub unmanaged_runs: usize,
    pub unknown_files: usize,
    pub history_truncated: bool,
}
impl RetentionReport {
    pub(crate) fn as_json(&self) -> Value {
        json!({"schemaVersion":1,"scope":"owned_collection_files_and_control_metadata_only",
            "runBytes":self.run_bytes,"projectBytes":self.project_bytes,"globalBytes":self.global_bytes,
            "storedPages":self.stored_pages,"expiredFiles":self.expired_files,"expiredBytes":self.expired_bytes,
            "globalExpiredFiles":self.global_expired_files,"globalExpiredBytes":self.global_expired_bytes,"removalCounterMeaning":"durably_tombstoned_artifacts",
            "unmanagedRuns":self.unmanaged_runs,"unknownFiles":self.unknown_files,"historyTruncated":self.history_truncated,
            "analysisSnapshotPinsIntegrated":false})
    }
}
pub(crate) struct CollectionStorage {
    root: PathBuf,
    output: PathBuf,
    project: i64,
    run: i64,
    limits: Limits,
    _writer: File,
}
struct Stored {
    path: PathBuf,
    bytes: u64,
}
fn fail(code: &str) -> String {
    format!("resource_limited:collection_retention:{code}")
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn file_hash(path: &Path) -> Result<String, String> {
    regular(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn directory(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink()
        || !meta.is_dir()
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("collection_retention_unsafe_directory".into());
    }
    Ok(())
}
fn regular(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => {
            Err("collection_retention_unsafe_file".into())
        }
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}
fn options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options
}
fn lock(path: &Path, wait: bool) -> Result<Option<File>, String> {
    directory(path.parent().ok_or("collection_retention_no_parent")?)?;
    regular(path)?;
    let file = options()
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    if wait {
        file.lock().map_err(|e| e.to_string())?;
        Ok(Some(file))
    } else {
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
}
fn parents(base: &Path, relative: &Path, create: bool) -> Result<PathBuf, String> {
    directory(base)?;
    let mut path = base.to_path_buf();
    let parts = relative.components().collect::<Vec<_>>();
    if parts.is_empty() || parts.iter().any(|p| !matches!(p, Component::Normal(_))) {
        return Err("collection_retention_relative_path_required".into());
    }
    for part in &parts[..parts.len() - 1] {
        path.push(part.as_os_str());
        if !path.exists() && create {
            fs::create_dir(&path).map_err(|e| e.to_string())?;
        }
        directory(&path)?;
    }
    path.push(parts.last().unwrap().as_os_str());
    regular(&path)?;
    Ok(path)
}
fn valid_path(relative: &Path, kind: ArtifactKind) -> Result<String, String> {
    let value = relative
        .to_str()
        .ok_or("collection_retention_non_utf8_path")?;
    if value.is_empty()
        || value.len() > 220
        || value
            .bytes()
            .any(|b| !(b.is_ascii_alphanumeric() || b"/_-.".contains(&b)))
        || relative
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err("collection_retention_invalid_artifact_path".into());
    }
    let filename = relative.file_name().and_then(|v| v.to_str()).unwrap_or("");
    let page = filename
        .strip_suffix(".json")
        .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()));
    let ok = match kind {
        ArtifactKind::Page => {
            page && relative.parent() == Some(Path::new("collection-checkpoint-v1/pages"))
        }
        ArtifactKind::LegacyPage => {
            page && relative.parent() == Some(Path::new("collection-checkpoint-v1/legacy-pages"))
        }
        ArtifactKind::Cache => page && relative.parent() == Some(Path::new("native-cache")),
        ArtifactKind::Inventory => {
            relative.components().count() == 1
                && matches!(
                    filename,
                    "url_inventory.csv"
                        | "confirmed_urls.txt"
                        | "pending_urls.txt"
                        | "rejected_urls.txt"
                )
        }
        ArtifactKind::Csv => {
            relative.components().count() == 1
                && filename.ends_with(".csv")
                && !filename.starts_with('.')
        }
        ArtifactKind::Metadata => {
            filename.ends_with(".json")
                && !filename.starts_with('.')
                && (relative.components().count() == 1
                    || relative.parent() == Some(Path::new("collection-checkpoint-v1")))
        }
    };
    if !ok {
        return Err("collection_retention_kind_path_mismatch".into());
    }
    Ok(value.into())
}
fn inventory(base: &Path) -> Result<Vec<Stored>, String> {
    fn visit(path: &Path, rows: &mut Vec<Stored>, depth: usize) -> Result<(), String> {
        if depth > 4 || rows.len() > MAX_FILES {
            return Err(fail("inventory_limit"));
        }
        directory(path)?;
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if meta.file_type().is_symlink() {
                return Err("collection_retention_symlink_input".into());
            }
            if meta.is_dir() {
                visit(&path, rows, depth + 1)?;
            } else if meta.is_file() {
                rows.push(Stored {
                    path,
                    bytes: meta.len(),
                });
                if rows.len() > MAX_FILES {
                    return Err(fail("inventory_limit"));
                }
            } else {
                return Err("collection_retention_special_input".into());
            }
        }
        Ok(())
    }
    let mut rows = Vec::new();
    visit(base, &mut rows, 0)?;
    Ok(rows)
}
fn read_manifest(output: &Path) -> Result<Value, String> {
    let path = parents(output, Path::new(OWNER), false)?;
    if !regular(&path)? || fs::metadata(&path).map_err(|e| e.to_string())?.len() > MAX_METADATA {
        return Err(fail("owner_metadata_missing_or_oversized"));
    }
    let value: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "collection_retention_invalid_owner")?;
    if value["owner"] != "oviraptor-collection"
        || value["version"] != 1
        || value["projectId"].as_i64().is_none_or(|n| n < 1)
        || value["runId"].as_i64().is_none_or(|n| n < 1)
        || !value["entries"].is_object()
        || !value["tombstones"].is_array()
    {
        return Err("collection_retention_invalid_owner".into());
    }
    for (path, entry) in value["entries"].as_object().unwrap() {
        let kind = ArtifactKind::parse(
            entry["kind"]
                .as_str()
                .ok_or("collection_retention_invalid_entry")?,
        )?;
        valid_path(Path::new(path), kind)?;
        if entry["bytes"].as_u64().is_none()
            || entry["hash"]
                .as_str()
                .is_none_or(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
            || !matches!(entry["phase"].as_str(), Some("available" | "writing"))
            || !entry["pinned"].is_boolean()
            || !(entry["receivedAt"].is_null()
                || entry["receivedAt"].as_i64().is_some_and(|n| n >= 0))
        {
            return Err("collection_retention_invalid_entry".into());
        }
    }
    Ok(value)
}
fn encode(value: &Value) -> Result<Vec<u8>, String> {
    let raw = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if raw.len() as u64 > MAX_METADATA {
        return Err(fail("metadata_limit"));
    }
    Ok(raw)
}
fn atomic(base: &Path, relative: &Path, raw: &[u8]) -> Result<(), String> {
    let path = parents(base, relative, true)?;
    let parent = path.parent().unwrap();
    let temporary = parent.join(format!(
        ".retention-tmp-{}-{}",
        std::process::id(),
        TEMPORARY.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = options()
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(raw)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        directory(parent)?;
        regular(&path)?;
        fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() && regular(&temporary).unwrap_or(false) {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn record_removal(manifest: &mut Value, path: &str, entry: &Value, reason: &str, now: i64) {
    let rows = manifest["tombstones"].as_array_mut().unwrap();
    rows.push(json!({"path":path,"kind":entry["kind"],"hash":entry["hash"],"bytes":entry["bytes"],"receivedAt":entry["receivedAt"],"storedAt":entry["storedAt"],"reason":reason,"removedAt":now}));
    if rows.len() > MAX_HISTORY {
        rows.drain(..rows.len() - MAX_HISTORY);
        manifest["historyTruncated"] = json!(true);
    }
    manifest["expiredFiles"] = json!(manifest["expiredFiles"]
        .as_u64()
        .unwrap_or(0)
        .saturating_add(1));
    manifest["expiredBytes"] = json!(manifest["expiredBytes"]
        .as_u64()
        .unwrap_or(0)
        .saturating_add(entry["bytes"].as_u64().unwrap_or(0)));
    manifest["entries"].as_object_mut().unwrap().remove(path);
}
impl CollectionStorage {
    pub(crate) fn open(
        root: &Path,
        output: &Path,
        project: i64,
        run: i64,
        now: i64,
        limits: Limits,
    ) -> Result<Self, String> {
        Self::open_in(root, output, project, run, now, limits, false)
    }
    /// Explicitly called for a DB-verified current legacy run, never from a sweep.
    /// Existing recognized files have unknown age and are not silently expired.
    pub(crate) fn open_adopting_current(
        root: &Path,
        output: &Path,
        project: i64,
        run: i64,
        now: i64,
        limits: Limits,
    ) -> Result<Self, String> {
        Self::open_in(root, output, project, run, now, limits, true)
    }
    fn open_in(
        root: &Path,
        output: &Path,
        project: i64,
        run: i64,
        now: i64,
        limits: Limits,
        adopt: bool,
    ) -> Result<Self, String> {
        directory(root)?;
        let relative = output
            .strip_prefix(root)
            .map_err(|_| "collection_retention_outside_jobs_root")?;
        if project < 1
            || run < 1
            || now < 0
            || limits.age_seconds < 1
            || limits.pages < 1
            || limits.run_bytes > limits.project_bytes
            || limits.project_bytes > limits.global_bytes
            || relative.components().count() != 2
            || relative.file_name() != Some(std::ffi::OsStr::new("collection"))
            || relative
                .components()
                .any(|p| !matches!(p, Component::Normal(_)))
        {
            return Err("collection_retention_invalid_scope".into());
        }
        directory(output.parent().ok_or("collection_retention_no_run")?)?;
        if !output.exists() {
            fs::create_dir(output).map_err(|e| e.to_string())?;
        }
        directory(output)?;
        let global = lock(&root.join(GLOBAL), true)?.unwrap();
        let writer =
            lock(&output.join(WRITER), false)?.ok_or("collection_retention_writer_active")?;
        let owner = output.join(OWNER);
        let storage = Self {
            root: root.into(),
            output: output.into(),
            project,
            run,
            limits,
            _writer: writer,
        };
        let mut manifest = if !regular(&owner)? {
            let existing = inventory(output)?;
            if !adopt && existing.iter().any(|row| row.path != output.join(WRITER)) {
                return Err("collection_retention_unowned_existing_directory".into());
            }
            let mut entries = serde_json::Map::new();
            for row in existing
                .into_iter()
                .filter(|r| r.path != output.join(WRITER))
            {
                let path = row.path.strip_prefix(output).map_err(|e| e.to_string())?;
                let kind = if path.starts_with("collection-checkpoint-v1/pages") {
                    Some(ArtifactKind::Page)
                } else if path.starts_with("collection-checkpoint-v1/legacy-pages") {
                    Some(ArtifactKind::LegacyPage)
                } else if path.starts_with("native-cache") {
                    Some(ArtifactKind::Cache)
                } else if path.components().count() == 1
                    && matches!(
                        path.to_str(),
                        Some(
                            "url_inventory.csv"
                                | "confirmed_urls.txt"
                                | "pending_urls.txt"
                                | "rejected_urls.txt"
                        )
                    )
                {
                    Some(ArtifactKind::Inventory)
                } else if path.components().count() == 1
                    && path.extension().is_some_and(|e| e == "csv")
                {
                    Some(ArtifactKind::Csv)
                } else if path.extension().is_some_and(|e| e == "json") {
                    Some(ArtifactKind::Metadata)
                } else {
                    None
                };
                if let Some(kind) = kind.filter(|kind| valid_path(path, *kind).is_ok()) {
                    entries.insert(path.to_string_lossy().into_owned(),json!({"kind":kind.name(),"hash":file_hash(&row.path)?,"bytes":row.bytes,"receivedAt":null,"storedAt":null,"phase":"available","pinned":false}));
                }
            }
            let value = json!({"owner":"oviraptor-collection","version":1,"projectId":project,"runId":run,"createdAt":now,"closedAt":null,
                "entries":entries,"tombstones":[],"expiredFiles":0,"expiredBytes":0,"historyTruncated":false});
            value
        } else {
            read_manifest(output)?
        };
        if manifest["projectId"] != project || manifest["runId"] != run {
            return Err("collection_retention_owner_conflict".into());
        }
        Self::reconcile(&mut manifest, output, now, false)?;
        manifest["closedAt"] = Value::Null;
        // New markers must pass admission before publication, including adopted
        // files and temporary metadata. Rejected opens cannot leak owner bytes.
        storage.prune_locked(now)?;
        let raw = storage.control_bytes(&manifest)?;
        let report = storage.report_scope(output, &manifest)?;
        storage.budget(
            &report,
            (raw.len() as u64).saturating_add(storage.headroom()),
            report.stored_pages,
        )?;
        atomic(output, Path::new(OWNER), &raw)?;
        drop(global);
        storage.preflight(now)?;
        Ok(storage)
    }
    fn owned(&self) -> Result<Vec<PathBuf>, String> {
        directory(&self.root)?;
        let mut out = Vec::new();
        for item in fs::read_dir(&self.root).map_err(|e| e.to_string())? {
            let path = item.map_err(|e| e.to_string())?.path();
            let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if !meta.is_dir() || meta.file_type().is_symlink() {
                continue;
            }
            let collection = path.join("collection");
            match fs::symlink_metadata(&collection) {
                Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
                _ => continue,
            }
            if regular(&collection.join(OWNER))? {
                read_manifest(&collection)?;
                out.push(collection);
            }
        }
        Ok(out)
    }
    fn report_locked(&self) -> Result<RetentionReport, String> {
        let manifest = read_manifest(&self.output)?;
        if manifest["projectId"] != self.project || manifest["runId"] != self.run {
            return Err("collection_retention_owner_conflict".into());
        }
        self.report_scope(&self.output, &manifest)
    }
    fn report_scope(&self, current: &Path, own: &Value) -> Result<RetentionReport, String> {
        let mut report = RetentionReport::default();
        report.expired_files = own["expiredFiles"].as_u64().unwrap_or(0);
        report.expired_bytes = own["expiredBytes"].as_u64().unwrap_or(0);
        report.history_truncated = own["historyTruncated"].as_bool().unwrap_or(false);
        let mut owned = self.owned()?;
        if !owned.iter().any(|path| path == current) {
            owned.push(current.into());
        }
        report.unmanaged_runs = fs::read_dir(&self.root)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .count()
            .saturating_sub(owned.len());
        for output in owned {
            let manifest = if output == current {
                own.clone()
            } else {
                read_manifest(&output)?
            };
            let files = inventory(&output)?;
            let bytes = files
                .iter()
                .map(|r| r.bytes)
                .fold(0u64, u64::saturating_add);
            report.global_bytes = report.global_bytes.saturating_add(bytes);
            report.global_expired_files = report
                .global_expired_files
                .saturating_add(manifest["expiredFiles"].as_u64().unwrap_or(0));
            report.global_expired_bytes = report
                .global_expired_bytes
                .saturating_add(manifest["expiredBytes"].as_u64().unwrap_or(0));
            if manifest["projectId"] == own["projectId"] {
                report.project_bytes = report.project_bytes.saturating_add(bytes);
            }
            if output == current {
                report.run_bytes = bytes;
                for row in files {
                    let rel = row
                        .path
                        .strip_prefix(&output)
                        .map_err(|e| e.to_string())?
                        .to_string_lossy();
                    if let Some(entry) = manifest["entries"].get(rel.as_ref()) {
                        if ArtifactKind::parse(entry["kind"].as_str().unwrap())?.page() {
                            report.stored_pages += 1;
                        }
                    } else if rel != OWNER
                        && rel != WRITER
                        && rel != "collection-checkpoint-v1/active.lock"
                    {
                        report.unknown_files += 1;
                    }
                }
            }
        }
        Ok(report)
    }
    fn reconcile(
        manifest: &mut Value,
        output: &Path,
        now: i64,
        close_abandoned: bool,
    ) -> Result<bool, String> {
        let mut changed = false;
        let entries = manifest["entries"].as_object().unwrap().clone();
        for (relative, entry) in entries {
            if entry["phase"] != "writing" {
                continue;
            }
            let path = parents(output, Path::new(&relative), false)?;
            if regular(&path)?
                && fs::metadata(&path).map_err(|e| e.to_string())?.len()
                    == entry["bytes"].as_u64().unwrap()
                && file_hash(&path)? == entry["hash"].as_str().unwrap()
            {
                manifest["entries"][&relative]["phase"] = json!("available");
                manifest["entries"][&relative]
                    .as_object_mut()
                    .unwrap()
                    .remove("recoveryConflict");
                changed = true;
            } else if entry["recoveryConflict"] != true {
                // A failed overwrite may have left the previous payload. Do not
                // infer its identity, erase it, or pretend publication succeeded.
                manifest["entries"][&relative]["recoveryConflict"] = json!(true);
                changed = true;
            }
        }
        if close_abandoned && manifest["closedAt"].is_null() {
            manifest["closedAt"] = json!(now);
            manifest["recoveredClosedAt"] = json!(now);
            changed = true;
        }
        Ok(changed)
    }
    fn prune_locked(&self, now: i64) -> Result<(), String> {
        for output in self.owned()? {
            if output == self.output {
                continue;
            }
            let mut manifest = read_manifest(&output)?;
            let Some(_writer) = lock(&output.join(WRITER), false)? else {
                continue;
            };
            let checkpoint = output.join("collection-checkpoint-v1");
            let _checkpoint_lock = if checkpoint.exists() {
                directory(&checkpoint)?;
                match lock(&checkpoint.join("active.lock"), false)? {
                    Some(lock) => Some(lock),
                    None => continue,
                }
            } else {
                None
            };
            if Self::reconcile(&mut manifest, &output, now, true)? {
                self.publish_control(&output, &manifest)?;
            }
            if manifest["closedAt"]
                .as_i64()
                .is_none_or(|t| now.saturating_sub(t) < self.limits.age_seconds)
            {
                continue;
            }
            // Recover a crash after durable tombstoning but before unlink. Never
            // unlink a path that has since acquired a live catalog reference.
            for tombstone in manifest["tombstones"].as_array().unwrap() {
                let relative = tombstone["path"]
                    .as_str()
                    .ok_or("collection_retention_invalid_tombstone")?;
                let kind = ArtifactKind::parse(
                    tombstone["kind"]
                        .as_str()
                        .ok_or("collection_retention_invalid_tombstone")?,
                )?;
                valid_path(Path::new(relative), kind)?;
                if manifest["entries"].get(relative).is_some() {
                    continue;
                }
                let path = parents(&output, Path::new(relative), false)?;
                if regular(&path)? && file_hash(&path)? == tombstone["hash"].as_str().unwrap_or("")
                {
                    fs::remove_file(path).map_err(|e| e.to_string())?;
                }
            }
            let candidates = manifest["entries"].as_object().unwrap().clone();
            for (relative, entry) in candidates {
                if entry["phase"] != "available"
                    || entry["pinned"] != false
                    || entry["kind"] == "metadata"
                    || entry["receivedAt"]
                        .as_i64()
                        .or_else(|| entry["storedAt"].as_i64())
                        .is_none_or(|t| now.saturating_sub(t) < self.limits.age_seconds)
                {
                    continue;
                }
                let path = parents(&output, Path::new(&relative), false)?;
                if !regular(&path)? {
                    continue;
                }
                if file_hash(&path)? != entry["hash"].as_str().unwrap() {
                    continue;
                }
                // Durable summary/tombstone precedes removal. A crash may leave
                // an extra raw file, never a live reference to erased bytes.
                record_removal(
                    &mut manifest,
                    &relative,
                    &entry,
                    "expired_owned_collection",
                    now,
                );
                self.publish_control(&output, &manifest)?;
                regular(&path)?;
                fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    fn headroom(&self) -> u64 {
        MAX_METADATA.min(self.limits.run_bytes / 4)
    }
    fn control_bytes(&self, value: &Value) -> Result<Vec<u8>, String> {
        let raw = encode(value)?;
        if raw.len() as u64 > self.headroom() {
            return Err(fail("control_metadata_limit"));
        }
        Ok(raw)
    }
    fn publish_control(&self, output: &Path, value: &Value) -> Result<(), String> {
        if output == self.output
            && (value["projectId"] != self.project || value["runId"] != self.run)
        {
            return Err("collection_retention_owner_conflict".into());
        }
        let raw = self.control_bytes(value)?;
        self.budget(&self.report_scope(output, value)?, raw.len() as u64, 0)?;
        atomic(output, Path::new(OWNER), &raw)
    }
    fn budget(&self, report: &RetentionReport, incoming: u64, pages: usize) -> Result<(), String> {
        if report.run_bytes.saturating_add(incoming) > self.limits.run_bytes
            || report.project_bytes.saturating_add(incoming) > self.limits.project_bytes
            || report.global_bytes.saturating_add(incoming) > self.limits.global_bytes
            || pages > self.limits.pages
        {
            return Err(fail("budget_exhausted"));
        }
        Ok(())
    }
    pub(crate) fn preflight(&self, now: i64) -> Result<RetentionReport, String> {
        let _global = lock(&self.root.join(GLOBAL), true)?.unwrap();
        self.prune_locked(now)?;
        let report = self.report_locked()?;
        self.budget(&report, 0, report.stored_pages)?;
        Ok(report)
    }
    /// Cache misses call this before making a page request. The writer lease
    /// serializes this run; publication still repeats exact byte admission.
    pub(crate) fn preflight_page(&self, now: i64) -> Result<RetentionReport, String> {
        let _global = lock(&self.root.join(GLOBAL), true)?.unwrap();
        self.prune_locked(now)?;
        let report = self.report_locked()?;
        self.budget(
            &report,
            self.headroom().saturating_add(1),
            report.stored_pages.saturating_add(1),
        )?;
        Ok(report)
    }
    pub(crate) fn report(&self) -> Result<RetentionReport, String> {
        let _global = lock(&self.root.join(GLOBAL), true)?.unwrap();
        self.report_locked()
    }
    pub(crate) fn write(
        &self,
        relative: &Path,
        bytes: &[u8],
        kind: ArtifactKind,
        received_at: Option<i64>,
        now: i64,
    ) -> Result<RetentionReport, String> {
        let key = valid_path(relative, kind)?;
        if now < 0
            || received_at.is_some_and(|t| t < 0)
            || kind == ArtifactKind::Metadata && bytes.len() as u64 > MAX_METADATA
        {
            return Err(fail("invalid_time_or_metadata_limit"));
        }
        let _global = lock(&self.root.join(GLOBAL), true)?.unwrap();
        self.prune_locked(now)?;
        let mut manifest = read_manifest(&self.output)?;
        let report = self.report_locked()?;
        let old = manifest["entries"].get(&key).cloned();
        if old.as_ref().is_some_and(|entry| entry["pinned"] == true) {
            return Err("collection_retention_pinned_artifact".into());
        }
        let entry = json!({"kind":kind.name(),"hash":hash(bytes),"bytes":bytes.len(),"receivedAt":received_at,"storedAt":now,"phase":"available","pinned":false});
        manifest["entries"][&key] = entry;
        let raw = self.control_bytes(&manifest)?;
        let pages = report.stored_pages
            + usize::from(
                kind.page()
                    && old.as_ref().is_none_or(|e| {
                        !ArtifactKind::parse(e["kind"].as_str().unwrap())
                            .is_ok_and(ArtifactKind::page)
                    }),
            );
        // Include both temporary files before either old file is replaced. This
        // deliberately rejects overwrites without spare physical headroom.
        self.budget(
            &report,
            (bytes.len() as u64)
                .saturating_add(raw.len() as u64)
                .saturating_add(self.headroom()),
            pages,
        )?;
        manifest["entries"][&key]["phase"] = json!("writing");
        self.publish_control(&self.output, &manifest)?;
        atomic(&self.output, relative, bytes)?;
        manifest["entries"][&key]["phase"] = json!("available");
        self.publish_control(&self.output, &manifest)?;
        self.report_locked()
    }
    pub(crate) fn remove(
        &self,
        relative: &Path,
        reason: &str,
        now: i64,
    ) -> Result<RetentionReport, String> {
        let _global = lock(&self.root.join(GLOBAL), true)?.unwrap();
        let mut manifest = read_manifest(&self.output)?;
        let key = relative
            .to_str()
            .ok_or("collection_retention_invalid_path")?;
        let entry = manifest["entries"]
            .get(key)
            .cloned()
            .ok_or("collection_retention_unregistered_removal")?;
        valid_path(
            relative,
            ArtifactKind::parse(entry["kind"].as_str().unwrap())?,
        )?;
        if entry["pinned"] != false
            || entry["phase"] != "available"
            || !matches!(
                reason,
                "receipt_expired" | "legacy_relocated" | "redacted_replaced"
            )
        {
            return Err("collection_retention_removal_refused".into());
        }
        let path = parents(&self.output, relative, false)?;
        if !regular(&path)? {
            return Err("collection_retention_removal_missing".into());
        }
        if file_hash(&path)? != entry["hash"].as_str().unwrap() {
            return Err("collection_retention_removal_hash_changed".into());
        }
        record_removal(&mut manifest, key, &entry, reason, now);
        self.publish_control(&self.output, &manifest)?;
        regular(&path)?;
        fs::remove_file(path).map_err(|e| e.to_string())?;
        self.report_locked()
    }
    /// Low-level retention reference only; no analysis approval/pin lifecycle is
    /// wired here. A caller must explicitly own and release any such reference.
    pub(crate) fn set_pinned(&self, relative: &Path, pinned: bool) -> Result<(), String> {
        let _global = lock(&self.root.join(GLOBAL), true)?.unwrap();
        let mut manifest = read_manifest(&self.output)?;
        let key = relative
            .to_str()
            .ok_or("collection_retention_invalid_path")?;
        let entry = manifest["entries"]
            .get_mut(key)
            .ok_or("collection_retention_pin_unregistered")?;
        entry["pinned"] = json!(pinned);
        self.publish_control(&self.output, &manifest)
    }
    pub(crate) fn finish(&self, now: i64) -> Result<RetentionReport, String> {
        let _global = lock(&self.root.join(GLOBAL), true)?.unwrap();
        let mut manifest = read_manifest(&self.output)?;
        if now < 0 {
            return Err("collection_retention_invalid_time".into());
        }
        manifest["closedAt"] = json!(now);
        self.publish_control(&self.output, &manifest)?;
        self.report_locked()
    }
}

#[cfg(test)]
#[path = "collection_retention_tests.rs"]
mod tests;
