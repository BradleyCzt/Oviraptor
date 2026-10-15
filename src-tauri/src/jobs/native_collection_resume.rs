use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(super) struct CollectionBinding {
    version: u32,
    pub(super) seeds_sha256: String,
    pub(super) plan_sha256: String,
    pub(super) api_url: String,
    pub(super) credential_sha256: String,
    pub(super) fields: Vec<String>,
    pub(super) page_size: usize,
    pub(super) max_pages: usize,
    pub(super) full: bool,
    pub(super) enable_cidr24: bool,
    pub(super) profile: String,
}

impl CollectionBinding {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        seeds: &[u8],
        plan: &Value,
        api_url: &str,
        credential: &str,
        fields: Vec<String>,
        page_size: usize,
        max_pages: usize,
        full: bool,
        enable_cidr24: bool,
        profile: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            version: FORMAT_VERSION,
            seeds_sha256: digest(seeds),
            plan_sha256: digest(&serde_json::to_vec(plan).map_err(|error| error.to_string())?),
            api_url: api_url.into(),
            credential_sha256: digest(credential.as_bytes()),
            fields,
            page_size,
            max_pages,
            full,
            enable_cidr24,
            profile: profile.into(),
        })
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("采集检查点路径没有父目录")?;
    fs::create_dir_all(parent).map_err(|error| format!("无法创建采集检查点：{error}"))?;
    let temporary = parent.join(format!(".collection-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("无法创建采集检查点暂存文件：{error}"))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("无法保存采集检查点：{error}"))?;
        fs::rename(&temporary, path).map_err(|error| format!("无法发布采集检查点：{error}"))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("无法同步采集检查点目录：{error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn publish(
    storage: Option<&crate::jobs::collection_retention::CollectionStorage>,
    output: &Path,
    path: &Path,
    bytes: &[u8],
    kind: crate::jobs::collection_retention::ArtifactKind,
    received_at: Option<&str>,
) -> Result<(), String> {
    if let Some(storage) = storage {
        let relative = path
            .strip_prefix(output)
            .map_err(|_| "收据路径超出当前任务")?;
        let received = received_at
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|time| time.timestamp());
        storage.write(
            relative,
            bytes,
            kind,
            received,
            chrono::Utc::now().timestamp(),
        )?;
        Ok(())
    } else {
        atomic_write(path, bytes)
    }
}

pub(super) const RECEIPT_RULE_VERSION: &str = "fofa-receipt-v2";
pub(super) const PAYLOAD_HASH_SEMANTICS: &str = "sha256-redacted-json-v1";
pub(super) const MAX_RECEIPT_AGE_SECONDS: i64 = 7 * 24 * 60 * 60;
pub(super) const MAX_RECEIPT_BYTES: u64 = 512 * 1024 * 1024;
pub(super) const MAX_RECEIPT_PAGES: usize = 4096;

#[derive(Clone, Default, Deserialize, Serialize)]
pub(super) struct RetentionReport {
    pub(super) max_age_seconds: i64,
    pub(super) max_bytes: u64,
    pub(super) max_pages: usize,
    pub(super) stored_bytes: u64,
    pub(super) stored_pages: usize,
    pub(super) expired_pages: usize,
    pub(super) legacy_unknown_pages: usize,
    pub(super) history_truncated: bool,
    pub(super) limit_reached: bool,
}

#[derive(Clone, Deserialize, Serialize)]
struct SavedPage {
    version: u32,
    binding_sha256: String,
    request_sha256: String,
    payload_sha256: String,
    fields: Vec<String>,
    payload: Value,
    #[serde(default)]
    received_at: Option<String>,
    #[serde(default)]
    rule_version: String,
    #[serde(default)]
    payload_redacted: bool,
    #[serde(default)]
    payload_hash_semantics: String,
    #[serde(default)]
    redaction_rule_version: String,
    #[serde(default)]
    redacted_locator_rows: Vec<usize>,
}

#[derive(Clone)]
pub(super) struct PageEvidence {
    pub(super) payload: Value,
    pub(super) received_at: Option<String>,
    pub(super) payload_sha256: String,
    pub(super) request_sha256: String,
    pub(super) payload_redacted: bool,
    pub(super) redacted_locator_rows: Vec<usize>,
}

pub(super) struct CollectionCheckpoint<'a> {
    storage: Option<&'a crate::jobs::collection_retention::CollectionStorage>,
    directory: PathBuf,
    output: PathBuf,
    binding_sha256: String,
    key: String,
    proxy: String,
    retention: std::cell::RefCell<RetentionReport>,
    legacy_noted: std::cell::RefCell<std::collections::BTreeSet<String>>,
    _lock: File,
}

fn known_request(path: &Path) -> bool {
    path.extension().is_some_and(|value| value == "json")
        && path
            .file_stem()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
}

impl<'a> CollectionCheckpoint<'a> {
    #[cfg(test)]
    pub(super) fn open(output: &Path, binding: CollectionBinding) -> Result<Self, String> {
        Self::open_redacted(output, binding, "", "")
    }

    pub(super) fn open_redacted(
        output: &Path,
        binding: CollectionBinding,
        key: &str,
        proxy: &str,
    ) -> Result<Self, String> {
        Self::open_with_storage(output, binding, key, proxy, None)
    }

    pub(super) fn open_with_storage(
        output: &Path,
        binding: CollectionBinding,
        key: &str,
        proxy: &str,
        storage: Option<&'a crate::jobs::collection_retention::CollectionStorage>,
    ) -> Result<Self, String> {
        let directory = output.join("collection-checkpoint-v1");
        fs::create_dir_all(&directory)
            .map_err(|error| format!("无法创建采集检查点目录：{error}"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("active.lock"))
            .map_err(|error| format!("无法打开采集检查点锁：{error}"))?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err("同一采集目录已有任务正在执行，请等待该任务停止后再恢复".into());
            }
        }
        let path = directory.join("binding.json");
        if path.exists() {
            let existing: CollectionBinding = serde_json::from_slice(
                &fs::read(&path).map_err(|error| format!("无法读取采集检查点绑定：{error}"))?,
            )
            .map_err(|_| "采集检查点绑定损坏；请保留原任务并创建新任务".to_string())?;
            if existing != binding {
                if existing.credential_sha256 != binding.credential_sha256 {
                    return Err(
                        "FOFA Key 已改变，不能复用其他凭据的采集检查点；请恢复原 Key，或创建新任务"
                            .into(),
                    );
                }
                return Err("原采集种子、查询计划或数据采集选项已改变；请恢复原任务快照，或创建新任务，不能混用检查点".into());
            }
        } else {
            let pages = directory.join("pages");
            if pages.exists()
                && fs::read_dir(&pages)
                    .map_err(|error| error.to_string())?
                    .next()
                    .is_some()
            {
                return Err(
                    "采集检查点缺少原始绑定，不能信任遗留页面；请保留原任务并创建新任务".into(),
                );
            }
            publish(
                storage,
                output,
                &path,
                &serde_json::to_vec(&binding).map_err(|e| e.to_string())?,
                crate::jobs::collection_retention::ArtifactKind::Metadata,
                None,
            )?;
        }
        let binding_sha256 =
            digest(&serde_json::to_vec(&binding).map_err(|error| error.to_string())?);
        fs::create_dir_all(directory.join("pages")).map_err(|error| error.to_string())?;
        let checkpoint = Self {
            storage,
            directory,
            output: output.into(),
            binding_sha256,
            key: key.into(),
            proxy: proxy.into(),
            _lock: lock,
            legacy_noted: std::cell::RefCell::new(std::collections::BTreeSet::new()),
            retention: std::cell::RefCell::new(RetentionReport {
                max_age_seconds: MAX_RECEIPT_AGE_SECONDS,
                max_bytes: MAX_RECEIPT_BYTES,
                max_pages: MAX_RECEIPT_PAGES,
                ..RetentionReport::default()
            }),
        };
        checkpoint.enforce_retention()?;
        Ok(checkpoint)
    }

    fn receipt_files(&self) -> Result<Vec<PathBuf>, String> {
        let mut files = Vec::new();
        // Only this run's explicitly owned receipt namespaces are inspected.
        // Never walk another run, follow symlinks, or prune arbitrary user files.
        for directory in [
            self.directory.join("pages"),
            self.directory.join("legacy-pages"),
            self.output.join("native-cache"),
        ] {
            if !directory.exists() {
                continue;
            }
            if fs::symlink_metadata(&directory)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("采集收据目录不能是符号链接".into());
            }
            for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                let kind = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if !kind.is_file() || kind.file_type().is_symlink() || !known_request(&path) {
                    return Err("采集收据目录包含未知文件；保留原目录，拒绝自动清理".into());
                }
                files.push(path);
            }
        }
        Ok(files)
    }

    fn append_history(&self, filename: &str, record: Value) -> Result<(), String> {
        let path = self.directory.join(filename);
        let mut records = if path.exists() {
            let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
            if metadata.len() > 8 * 1024 * 1024 {
                return Err("采集兼容记录超过有界大小".into());
            }
            serde_json::from_slice::<Value>(&fs::read(&path).map_err(|e| e.to_string())?)
                .map_err(|_| "采集兼容记录损坏")?
                .get("records")
                .and_then(Value::as_array)
                .cloned()
                .ok_or("采集兼容记录缺少 records")?
        } else {
            Vec::new()
        };
        if !records.contains(&record) {
            records.push(record);
        }
        if records.len() > MAX_RECEIPT_PAGES {
            records.drain(..records.len() - MAX_RECEIPT_PAGES);
            self.retention.borrow_mut().history_truncated = true;
        }
        loop {
            let value = serde_json::json!({"version":1,"records":records});
            let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
            if bytes.len() <= 8 * 1024 * 1024 {
                return self.publish(
                    &path,
                    &bytes,
                    crate::jobs::collection_retention::ArtifactKind::Metadata,
                    None,
                );
            }
            records = value["records"]
                .as_array()
                .cloned()
                .ok_or("采集兼容记录损坏")?;
            if records.len() <= 1 {
                return Err("receipt_metadata_limit: 单条兼容收据超过8MiB".into());
            }
            records.remove(0);
            self.retention.borrow_mut().history_truncated = true;
        }
    }

    fn enforce_retention(&self) -> Result<(), String> {
        let now = chrono::Utc::now();
        for path in self.receipt_files()? {
            let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
            // Filesystem time is used only for bounded storage expiry of old
            // receipts. It is never promoted to an asset observation timestamp.
            let expired = metadata
                .modified()
                .ok()
                .and_then(|time| std::time::SystemTime::now().duration_since(time).ok())
                .is_some_and(|age| age.as_secs() > MAX_RECEIPT_AGE_SECONDS as u64);
            if expired {
                self.append_history(
                    "retention-receipts.json",
                    serde_json::json!({
                        "request_sha256":path.file_stem().and_then(|x| x.to_str()),
                        "received_at":null,"reason":"receipt_expired","expired_at":now.to_rfc3339(),
                        "bytes":metadata.len()
                    }),
                )?;
                self.remove(&path, "receipt_expired")?;
                self.retention.borrow_mut().expired_pages += 1;
            }
        }
        // Keep the writer available to publish a typed resource-limited state.
        // A pre-existing oversized run must not send another request.
        match self.check_budget(None, 0) {
            Err(_) if self.retention.borrow().limit_reached => Ok(()),
            result => result,
        }
    }

    fn check_budget(&self, replacement: Option<&Path>, incoming: u64) -> Result<(), String> {
        let files = self.receipt_files()?;
        let mut bytes = 0u64;
        let mut pages = 0usize;
        let mut actual_bytes = 0u64;
        let mut actual_pages = 0usize;
        for path in files {
            let length = fs::metadata(&path).map_err(|e| e.to_string())?.len();
            actual_bytes = actual_bytes.saturating_add(length);
            actual_pages += 1;
            if replacement == Some(path.as_path()) {
                continue;
            }
            bytes = bytes.saturating_add(length);
            pages += 1;
        }
        if incoming > 0 {
            bytes = bytes.saturating_add(incoming);
            pages += 1;
        }
        let mut report = self.retention.borrow_mut();
        report.stored_bytes = actual_bytes;
        report.stored_pages = actual_pages;
        if bytes > MAX_RECEIPT_BYTES || pages > MAX_RECEIPT_PAGES {
            report.limit_reached = true;
            return Err("receipt_retention_limit: 当前任务收据达到512MiB或4096页上限；保留现有证据，覆盖不完整".into());
        }
        Ok(())
    }

    fn publish(
        &self,
        path: &Path,
        bytes: &[u8],
        kind: crate::jobs::collection_retention::ArtifactKind,
        received_at: Option<&str>,
    ) -> Result<(), String> {
        publish(self.storage, &self.output, path, bytes, kind, received_at)
    }

    fn remove(&self, path: &Path, reason: &str) -> Result<(), String> {
        if let Some(storage) = self.storage {
            storage.remove(
                path.strip_prefix(&self.output)
                    .map_err(|_| "收据路径超出当前任务")?,
                reason,
                chrono::Utc::now().timestamp(),
            )?;
            Ok(())
        } else {
            fs::remove_file(path).map_err(|e| e.to_string())
        }
    }

    pub(super) fn storage_report(&self) -> Result<Option<Value>, String> {
        self.storage
            .map(|storage| storage.report().map(|report| report.as_json()))
            .transpose()
    }

    pub(super) fn ensure_page_capacity(&self) -> Result<(), String> {
        self.check_budget(None, 1)?;
        if let Some(storage) = self.storage {
            storage.preflight_page(chrono::Utc::now().timestamp())?;
        }
        Ok(())
    }

    pub(super) fn retention_report(&self) -> RetentionReport {
        self.retention.borrow().clone()
    }

    pub(super) fn binding_sha256(&self) -> &str {
        &self.binding_sha256
    }

    pub(super) fn sanitize_legacy(&self, path: &Path, payload: &Value) -> Result<(), String> {
        let (payload, redacted) = super::native_collection_resume_transport::redact_payload(
            payload,
            &self.key,
            &self.proxy,
        );
        if redacted {
            let bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
            self.check_budget(Some(path), bytes.len() as u64)?;
            self.publish(
                path,
                &bytes,
                crate::jobs::collection_retention::ArtifactKind::Cache,
                None,
            )?;
        }
        Ok(())
    }

    pub(super) fn note_legacy(
        &self,
        request: &str,
        fields: &[String],
        payload: &Value,
    ) -> Result<(), String> {
        let (payload, redacted) = super::native_collection_resume_transport::redact_payload(
            payload,
            &self.key,
            &self.proxy,
        );
        self.append_history("legacy-receipts.json",serde_json::json!({
            "request_sha256":request,"payload_sha256":digest(&serde_json::to_vec(&payload).map_err(|e|e.to_string())?),
            "fields":fields,"received_at":null,"reason":"legacy_time_unknown",
            "payload_redacted":redacted,"payload_hash_semantics":PAYLOAD_HASH_SEMANTICS
        }))?;
        if self.legacy_noted.borrow_mut().insert(request.into()) {
            self.retention.borrow_mut().legacy_unknown_pages += 1;
        }
        Ok(())
    }

    pub(super) fn read_page(
        &self,
        request: &str,
        fields: &[String],
    ) -> Result<Option<PageEvidence>, String> {
        let path = self.directory.join("pages").join(format!("{request}.json"));
        let Some(raw) = fs::read(&path).ok() else {
            return Ok(None);
        };
        let Some(mut page) = serde_json::from_slice::<SavedPage>(&raw).ok() else {
            return Ok(None);
        };
        let payload_bytes = serde_json::to_vec(&page.payload).map_err(|e| e.to_string())?;
        if !matches!(page.version, 1 | 2)
            || page.binding_sha256 != self.binding_sha256
            || page.request_sha256 != request
            || page.fields != fields
            || page.payload_sha256 != digest(&payload_bytes)
            || !valid_page(&page.payload)
        {
            return Ok(None);
        }
        let locator_proven = !page.payload_redacted
            || page.redaction_rule_version
                == super::native_collection_resume_transport::REDACTION_RULE_VERSION;
        let (payload, redacted, rejected) = super::native_collection_resume_transport::redact_page(
            &page.payload,
            &self.key,
            &self.proxy,
            fields,
        );
        for row in rejected {
            if !page.redacted_locator_rows.contains(&row) {
                page.redacted_locator_rows.push(row);
            }
        }
        page.redaction_rule_version =
            super::native_collection_resume_transport::REDACTION_RULE_VERSION.into();
        page.payload = payload;
        page.payload_redacted |= redacted;
        page.payload_sha256 =
            digest(&serde_json::to_vec(&page.payload).map_err(|e| e.to_string())?);
        page.payload_hash_semantics = PAYLOAD_HASH_SEMANTICS.into();
        let known = page
            .received_at
            .as_deref()
            .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
            .filter(|_| {
                locator_proven
                    && page.version == 2
                    && page.rule_version == RECEIPT_RULE_VERSION
                    && page.payload_hash_semantics == PAYLOAD_HASH_SEMANTICS
            });
        if known.as_ref().is_some_and(|time| {
            chrono::Utc::now()
                .signed_duration_since(*time)
                .num_seconds()
                > MAX_RECEIPT_AGE_SECONDS
        }) {
            self.append_history(
                "retention-receipts.json",
                serde_json::json!({"request_sha256":request,
                "payload_sha256":page.payload_sha256,"received_at":page.received_at,
                "reason":"receipt_expired","payload_hash_semantics":PAYLOAD_HASH_SEMANTICS}),
            )?;
            self.remove(&path, "receipt_expired")?;
            self.retention.borrow_mut().expired_pages += 1;
            return Ok(None);
        }
        if known.is_none() {
            self.note_legacy(request, fields, &page.payload)?;
            let archive = self.directory.join("legacy-pages");
            fs::create_dir_all(&archive).map_err(|e| e.to_string())?;
            let mut archived = archive.join(format!("{request}.json"));
            // Preserve original bytes when safe; sanitizing a legacy receipt
            // must never preserve a credential merely for byte compatibility.
            let bytes = if redacted {
                serde_json::to_vec(&page).map_err(|e| e.to_string())?
            } else {
                raw
            };
            if archived.exists() && fs::read(&archived).map_err(|e| e.to_string())? != bytes {
                // A second undated version must not overwrite the first one.
                archived = archive.join(format!("{}.json", digest(&bytes)));
            }
            if !archived.exists() {
                self.check_budget(Some(&path), bytes.len() as u64)?;
                self.publish(
                    &archived,
                    &bytes,
                    crate::jobs::collection_retention::ArtifactKind::LegacyPage,
                    None,
                )?;
            }
            self.remove(&path, "legacy_relocated")?;
            return Ok(None);
        }
        if redacted {
            self.publish(
                &path,
                &serde_json::to_vec(&page).map_err(|e| e.to_string())?,
                crate::jobs::collection_retention::ArtifactKind::Page,
                page.received_at.as_deref(),
            )?;
        }
        Ok(Some(PageEvidence {
            payload: page.payload,
            received_at: page.received_at,
            payload_sha256: page.payload_sha256,
            request_sha256: request.into(),
            payload_redacted: page.payload_redacted,
            redacted_locator_rows: page.redacted_locator_rows,
        }))
    }

    pub(super) fn save_page(
        &self,
        request: &str,
        fields: &[String],
        payload: &Value,
        received_at: Option<&str>,
        payload_redacted: bool,
        redacted_locator_rows: &[usize],
    ) -> Result<PageEvidence, String> {
        if !valid_page(payload) {
            return Err("FOFA 页面没有有效 results，不能保存为已完成证据".into());
        }
        let (payload, redacted) = super::native_collection_resume_transport::redact_payload(
            payload,
            &self.key,
            &self.proxy,
        );
        let payload_sha256 = digest(&serde_json::to_vec(&payload).map_err(|e| e.to_string())?);
        let page = SavedPage {
            version: 2,
            binding_sha256: self.binding_sha256.clone(),
            request_sha256: request.into(),
            payload_sha256: payload_sha256.clone(),
            fields: fields.into(),
            payload: payload.clone(),
            received_at: received_at.map(str::to_string),
            rule_version: RECEIPT_RULE_VERSION.into(),
            payload_redacted: payload_redacted || redacted,
            payload_hash_semantics: PAYLOAD_HASH_SEMANTICS.into(),
            redaction_rule_version:
                super::native_collection_resume_transport::REDACTION_RULE_VERSION.into(),
            redacted_locator_rows: redacted_locator_rows.into(),
        };
        let path = self.directory.join("pages").join(format!("{request}.json"));
        let bytes = serde_json::to_vec(&page).map_err(|e| e.to_string())?;
        self.check_budget(Some(&path), bytes.len() as u64)?;
        self.publish(
            &path,
            &bytes,
            crate::jobs::collection_retention::ArtifactKind::Page,
            received_at,
        )?;
        self.check_budget(None, 0)?;
        Ok(PageEvidence {
            payload,
            received_at: page.received_at,
            payload_sha256,
            request_sha256: request.into(),
            payload_redacted: page.payload_redacted,
            redacted_locator_rows: page.redacted_locator_rows,
        })
    }

    pub(super) fn write_state(&self, output: &Path, state: &Value) -> Result<(), String> {
        let mut state = state.clone();
        state["version"] = FORMAT_VERSION.into();
        state["binding_sha256"] = self.binding_sha256.clone().into();
        self.publish(
            &output.join("collection_state.json"),
            &serde_json::to_vec(&state).map_err(|e| e.to_string())?,
            crate::jobs::collection_retention::ArtifactKind::Metadata,
            None,
        )
    }
}

pub(super) fn valid_page(payload: &Value) -> bool {
    payload.is_object()
        && payload
            .get("error")
            .is_none_or(|error| error.as_bool() == Some(false))
        && payload.get("results").and_then(Value::as_array).is_some()
}
