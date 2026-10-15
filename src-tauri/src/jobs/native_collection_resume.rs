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

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    atomic_write(path, &bytes)
}

#[derive(Deserialize, Serialize)]
struct SavedPage {
    version: u32,
    binding_sha256: String,
    request_sha256: String,
    payload_sha256: String,
    fields: Vec<String>,
    payload: Value,
}

pub(super) struct CollectionCheckpoint {
    directory: PathBuf,
    binding_sha256: String,
    // On macOS/Unix, a process crash releases flock without a stale sentinel
    // file blocking the next recovery. The App job registry guards other OSes.
    _lock: File,
}

impl CollectionCheckpoint {
    pub(super) fn open(output: &Path, binding: CollectionBinding) -> Result<Self, String> {
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
            let locked = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if locked != 0 {
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
            write_json(&path, &binding)?;
        }
        let binding_sha256 =
            digest(&serde_json::to_vec(&binding).map_err(|error| error.to_string())?);
        fs::create_dir_all(directory.join("pages")).map_err(|error| error.to_string())?;
        Ok(Self {
            directory,
            binding_sha256,
            _lock: lock,
        })
    }

    pub(super) fn read_page(&self, request: &str, fields: &[String]) -> Option<Value> {
        let page: SavedPage = serde_json::from_slice(
            &fs::read(self.directory.join("pages").join(format!("{request}.json"))).ok()?,
        )
        .ok()?;
        let payload_bytes = serde_json::to_vec(&page.payload).ok()?;
        (page.version == FORMAT_VERSION
            && page.binding_sha256 == self.binding_sha256
            && page.request_sha256 == request
            && page.fields == fields
            && page.payload_sha256 == digest(&payload_bytes)
            && valid_page(&page.payload))
        .then_some(page.payload)
    }

    pub(super) fn save_page(
        &self,
        request: &str,
        fields: &[String],
        payload: &Value,
    ) -> Result<(), String> {
        if !valid_page(payload) {
            return Err("FOFA 页面没有有效 results，不能保存为已完成证据".into());
        }
        write_json(
            &self.directory.join("pages").join(format!("{request}.json")),
            &SavedPage {
                version: FORMAT_VERSION,
                binding_sha256: self.binding_sha256.clone(),
                request_sha256: request.into(),
                payload_sha256: digest(
                    &serde_json::to_vec(payload).map_err(|error| error.to_string())?,
                ),
                fields: fields.to_vec(),
                payload: payload.clone(),
            },
        )
    }

    pub(super) fn write_state(&self, output: &Path, state: &Value) -> Result<(), String> {
        let mut state = state.clone();
        state["version"] = FORMAT_VERSION.into();
        state["binding_sha256"] = self.binding_sha256.clone().into();
        write_json(&output.join("collection_state.json"), &state)
    }
}

pub(super) fn valid_page(payload: &Value) -> bool {
    payload.is_object()
        && payload
            .get("error")
            .is_none_or(|error| error.as_bool() == Some(false))
        && payload
            .get("results")
            .and_then(Value::as_array)
            .is_some_and(|results| results.iter().all(|row| row.is_object() || row.is_array()))
}
