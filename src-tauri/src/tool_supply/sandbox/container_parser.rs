//! Fixed, offline container mechanism. This leaf grants no Broker authority.
//! All caller code runs in the container; no host paths or credentials enter it.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) const ADAPTER: &str = "offline-parser-container-v1";
// Task-time pulls and caller-selected images are deliberately unavailable.
pub(crate) const IMAGE: &str =
    "sha256:114c7a4425406451c2866b6aafe69fe29b1b298832db1277d411ac73c82d04d6";
const MAX_INPUT: usize = 64 * 1024;
const MAX_CODE: usize = 16 * 1024;
const CLI: &str = "/Applications/Docker.app/Contents/Resources/bin/docker";
const OWN_LABEL: &str = "com.oviraptor.offline-parser-owner";
const GUARDIAN_ARGUMENT: &str = "--nest-private-parser-guardian-v1";
#[path = "container_parser/orphans.rs"]
mod orphans;
pub(crate) use orphans::recover_orphans_once;

/// Called before Tauri/config/DB initialization. The private carrier has no
/// host execution or target tools, only the fixed offline container protocol.
pub(crate) fn guardian_entry() -> Option<i32> {
    if std::env::args().nth(1).as_deref() == Some("--nest-private-parser-cleanup-v1") {
        return Some(
            match Runtime::inspect().and_then(|runtime| runtime.recover_orphans()) {
                Ok(receipts) => {
                    println!("{}", json!({"recovered":receipts}));
                    0
                }
                Err(error) => {
                    eprintln!("{error}");
                    2
                }
            },
        );
    }
    if std::env::args().nth(1).as_deref() == Some("--nest-private-parser-runtime-v1") {
        return Some(match Runtime::inspect() {
            Ok(runtime) => {
                println!(
                    "{}",
                    json!({"adapter":ADAPTER,"runtimeDigest":runtime.digest(),"image":IMAGE})
                );
                0
            }
            Err(error) => {
                eprintln!("{error}");
                2
            }
        });
    }
    if std::env::args().nth(1).as_deref() != Some(GUARDIAN_ARGUMENT) {
        return None;
    }
    let result = (|| -> Result<(), String> {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let mut input = std::io::BufReader::new(std::io::stdin());
        let request = read_guardian_request(&mut input)?;
        let closed = Arc::new(AtomicBool::new(false));
        let marker = closed.clone();
        std::thread::spawn(move || {
            let mut extra = [0u8; 1];
            let _ = input.read(&mut extra);
            marker.store(true, Ordering::Release);
        });
        let runtime = Runtime::inspect()?;
        let receipt = runtime.execute(&request, &|| closed.load(Ordering::Acquire))?;
        let mut stdout = std::io::stdout().lock();
        serde_json::to_writer(&mut stdout, &receipt).map_err(|e| e.to_string())?;
        stdout.write_all(b"\n").map_err(|e| e.to_string())?;
        Ok(())
    })();
    Some(match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            2
        }
    })
}

fn read_guardian_request(input: &mut impl std::io::BufRead) -> Result<Request, String> {
    let mut line = Vec::new();
    loop {
        let available = input.fill_buf().map_err(|e| e.to_string())?;
        if available.is_empty() {
            return Err("container_parser_guardian_request_incomplete".into());
        }
        let n = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        if line.len() + n > 512 * 1024 {
            return Err("container_parser_guardian_request_too_large".into());
        }
        let done = available[n - 1] == b'\n';
        line.extend_from_slice(&available[..n]);
        input.consume(n);
        if done {
            break;
        }
    }
    let request: Request =
        serde_json::from_slice(&line).map_err(|_| "container_parser_guardian_request_invalid")?;
    request.require()?;
    Ok(request)
}

/// The independent guardian owns create/start/kill/remove. Closing the parent
/// pipe (including SIGKILL of the App) cancels it; the App never owns the
/// container's cleanup process. This is mechanism only, not a Broker permit.
pub(crate) fn execute_guarded(
    request: &Request,
    cancelled: &dyn Fn() -> bool,
) -> Result<Receipt, String> {
    request.require()?;
    if cancelled() {
        return Err("container_parser_cancelled_before_create".into());
    }
    let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg(GUARDIAN_ARGUMENT)
        .env_clear()
        .env(
            "HOME",
            std::env::var_os("HOME").ok_or("container_parser_home_unavailable")?,
        )
        .env("TMPDIR", std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut input = child
        .stdin
        .take()
        .ok_or("container_parser_guardian_pipe_missing")?;
    let mut encoded = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    encoded.push(b'\n');
    input.write_all(&encoded).map_err(|e| e.to_string())?;
    let keepalive = std::cell::RefCell::new(Some(input));
    let result = capture(&mut child, 1024 * 1024, Duration::from_secs(120), &|| {
        if cancelled() {
            keepalive.borrow_mut().take();
        }
        false
    })?;
    if !result.success || !result.stop_reason.is_empty() {
        return Err(format!(
            "container_parser_guardian_failed:{}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    let receipt: Receipt = serde_json::from_slice(&result.stdout)
        .map_err(|_| "container_parser_guardian_receipt_invalid")?;
    if !receipt.cleanup_verified
        || receipt.runtime_digest != request.runtime_digest
        || receipt.code_sha256 != hash(request.code.as_bytes())
        || receipt.input_sha256 != hash(request.input.to_string().as_bytes())
    {
        return Err("container_parser_guardian_receipt_changed".into());
    }
    Ok(receipt)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Limits {
    pub memory_mib: u32,
    pub timeout_seconds: u32,
    pub output_bytes: u32,
}
impl Limits {
    fn require(&self) -> Result<(), String> {
        if !(16..=256).contains(&self.memory_mib)
            || !(1..=60).contains(&self.timeout_seconds)
            || !(256..=65536).contains(&self.output_bytes)
        {
            return Err("container_parser_limits_invalid".into());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    pub code: String,
    pub input: Value,
    pub limits: Limits,
    pub runtime_digest: String,
}
impl Request {
    fn require(&self) -> Result<(), String> {
        self.limits.require()?;
        if self.code.is_empty()
            || self.code.len() > MAX_CODE
            || self.input.to_string().len() > MAX_INPUT
        {
            return Err("container_parser_input_size_invalid".into());
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Receipt {
    pub container_id: String,
    pub runtime_digest: String,
    pub code_sha256: String,
    pub input_sha256: String,
    pub exit_code: i64,
    pub oom_killed: bool,
    pub elapsed_millis: u128,
    pub stdout: String,
    pub stderr: String,
    pub stop_reason: String,
    pub cleanup_verified: bool,
}

pub(crate) struct Runtime {
    socket: String,
    cli_hash: String,
    digest: String,
}
fn hash(bytes: &[u8]) -> String {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes);
    format!(
        "sha256:{}",
        digest
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

impl Runtime {
    pub(crate) fn inspect() -> Result<Self, String> {
        #[cfg(not(target_os = "macos"))]
        {
            return Err("unsupported_sandbox:container_parser_platform_unverified".into());
        }
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::fs::{FileTypeExt, MetadataExt};
            let file = fs::symlink_metadata(CLI).map_err(|_| "container_parser_cli_missing")?;
            if !file.is_file() || file.file_type().is_symlink() || file.mode() & 0o022 != 0 {
                return Err("container_parser_cli_identity_invalid".into());
            }
            let home = std::env::var_os("HOME").ok_or("container_parser_home_unavailable")?;
            let socket = PathBuf::from(home).join(".docker/run/docker.sock");
            let metadata =
                fs::symlink_metadata(&socket).map_err(|_| "container_parser_daemon_unavailable")?;
            if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
                return Err("container_parser_local_daemon_required".into());
            }
            let cli_hash = hash(&fs::read(CLI).map_err(|_| "container_parser_cli_unreadable")?);
            let digest = hash(&serde_json::to_vec(&json!({
                "adapter":ADAPTER,"image":IMAGE,"cliSha256":cli_hash,
                "wrapperSha256":hash(WRAPPER.as_bytes()),"seccompSha256":hash(seccomp().to_string().as_bytes()),
                "guardianSourceSha256":hash(include_bytes!("container_parser.rs")),
                "recoverySourceSha256":hash(include_bytes!("container_parser/orphans.rs")),
                "network":"none","uid":65534,"pids":1,"cpuQuota":100000,
                "cpuPeriod":100000,"readOnly":true,"mounts":[],"platform":"linux/arm64"
            })).map_err(|e| e.to_string())?);
            let runtime = Self {
                socket: format!("unix://{}", socket.display()),
                cli_hash,
                digest,
            };
            let scratch = Scratch::new()?;
            let info = runtime.command(
                &scratch,
                &["info", "--format", "{{json .}}"],
                None,
                512 * 1024,
                Duration::from_secs(10),
                &|| false,
            )?;
            let info: Value = serde_json::from_slice(&info.stdout)
                .map_err(|_| "container_parser_daemon_invalid")?;
            if info["OSType"] != "linux"
                || info["CgroupVersion"] != "2"
                || !info["SecurityOptions"].as_array().is_some_and(|options| {
                    options
                        .iter()
                        .any(|v| v.as_str().is_some_and(|s| s.starts_with("name=seccomp")))
                })
            {
                return Err("unsupported_sandbox:container_parser_cgroup_seccomp_required".into());
            }
            let image = runtime.command(
                &scratch,
                &["image", "inspect", IMAGE],
                None,
                512 * 1024,
                Duration::from_secs(10),
                &|| false,
            )?;
            let image: Value = serde_json::from_slice(&image.stdout)
                .map_err(|_| "container_parser_image_missing")?;
            if image.as_array().is_none_or(|a| a.len() != 1)
                || image[0]["Id"] != IMAGE
                || image[0]["Os"] != "linux"
                || image[0]["Architecture"] != "arm64"
                || image[0]["Config"]["Volumes"]
                    .as_object()
                    .is_some_and(|v| !v.is_empty())
            {
                return Err("container_parser_fixed_image_required".into());
            }
            Ok(runtime)
        }
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }

    fn command(
        &self,
        scratch: &Scratch,
        args: &[&str],
        input: Option<Vec<u8>>,
        bound: usize,
        timeout: Duration,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Captured, String> {
        if hash(&fs::read(CLI).map_err(|_| "container_parser_cli_unreadable")?) != self.cli_hash {
            return Err("container_parser_cli_changed".into());
        }
        let mut child = Command::new(CLI)
            .args(args)
            .env_clear()
            .env("DOCKER_HOST", &self.socket)
            .env("DOCKER_CONFIG", &scratch.path)
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("container_parser_cli_spawn:{e}"))?;
        let writer = input.map(|bytes| {
            let mut pipe = child.stdin.take().unwrap();
            std::thread::spawn(move || pipe.write_all(&bytes))
        });
        let result = capture(&mut child, bound, timeout, cancelled);
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        result
    }

    pub(crate) fn execute(
        &self,
        request: &Request,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Receipt, String> {
        request.require()?;
        if request.runtime_digest != self.digest {
            return Err("container_parser_runtime_changed".into());
        }
        if cancelled() {
            return Err("container_parser_cancelled_before_create".into());
        }
        let scratch = Scratch::new()?;
        let profile = scratch.path.join("seccomp.json");
        fs::write(&profile, seccomp().to_string()).map_err(|e| e.to_string())?;
        let nonce = uuid::Uuid::new_v4().to_string();
        let name = format!("oviraptor-parser-{nonce}");
        let label = format!("{OWN_LABEL}={nonce}");
        let security = format!("seccomp={}", profile.display());
        let memory = (u64::from(request.limits.memory_mib) * 1024 * 1024).to_string();
        let mut owned = OwnedContainer {
            runtime: self,
            scratch: &scratch,
            name,
            nonce,
            created_id: None,
            cleaned: false,
        };
        owned.save_journal()?;
        let created=self.command(&scratch,&["create","--pull=never","--interactive","--name",&owned.name,
            "--label",&label,"--network=none","--read-only","--cap-drop=ALL","--security-opt=no-new-privileges",
            "--security-opt",&security,"--user=65534:65534","--pids-limit=1","--cpu-period=100000","--cpu-quota=100000",
            "--memory",&memory,"--memory-swap",&memory,"--restart=no","--log-driver=none",
            "--ipc=private","--cgroupns=private","--workdir=/output",
            "--tmpfs=/output:rw,noexec,nosuid,nodev,size=16777216,mode=0700,uid=65534,gid=65534",
            "--entrypoint=/usr/bin/python3",IMAGE,"-I","-S","-u","-c",WRAPPER],None,8192,Duration::from_secs(15),cancelled)?;
        if !created.success || !created.stop_reason.is_empty() {
            return Err(format!(
                "container_parser_create_failed:{}:{}",
                created.stop_reason,
                String::from_utf8_lossy(&created.stderr)
            ));
        }
        let id = String::from_utf8(created.stdout)
            .map_err(|_| "container_parser_id_invalid")?
            .trim()
            .to_string();
        if id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("container_parser_id_invalid".into());
        }
        owned.created_id = Some(id.clone());
        owned.save_journal()?;
        let original = owned.inspect()?;
        require_container(&original, &owned.nonce, &memory)?;
        if cancelled() {
            return Err("container_parser_cancelled_before_start".into());
        }
        let input = request.input.to_string();
        let envelope = json!({"code":request.code,"input_utf8":input,"code_sha256":hash(request.code.as_bytes()),
            "input_sha256":hash(input.as_bytes()),"memory":memory,"output_limit":request.limits.output_bytes});
        let started = Instant::now();
        let captured = self.command(
            &scratch,
            &["start", "--attach", "--interactive", &id],
            Some(serde_json::to_vec(&envelope).map_err(|e| e.to_string())?),
            request.limits.output_bytes as usize,
            Duration::from_secs(u64::from(request.limits.timeout_seconds)),
            cancelled,
        )?;
        if !captured.stop_reason.is_empty() {
            owned.kill()?;
        }
        let state = owned.inspect()?;
        if state["State"]["Running"] != false {
            return Err("container_parser_exit_unverified".into());
        }
        let exit = state["State"]["ExitCode"]
            .as_i64()
            .ok_or("container_parser_exit_invalid")?;
        let oom = state["State"]["OOMKilled"]
            .as_bool()
            .ok_or("container_parser_exit_invalid")?;
        let mut receipt = Receipt {
            container_id: id,
            runtime_digest: self.digest.clone(),
            code_sha256: hash(request.code.as_bytes()),
            input_sha256: hash(input.as_bytes()),
            exit_code: exit,
            oom_killed: oom,
            elapsed_millis: started.elapsed().as_millis(),
            stdout: String::from_utf8_lossy(&captured.stdout).into(),
            stderr: String::from_utf8_lossy(&captured.stderr).into(),
            stop_reason: captured.stop_reason,
            cleanup_verified: false,
        };
        owned.cleanup()?;
        receipt.cleanup_verified = true;
        Ok(receipt)
    }
}

fn require_container(value: &Value, nonce: &str, memory: &str) -> Result<(), String> {
    let host = &value["HostConfig"];
    if value["Image"] != IMAGE
        || value["Config"]["Labels"][OWN_LABEL] != nonce
        || value["Config"]["User"] != "65534:65534"
        || host["NetworkMode"] != "none"
        || host["ReadonlyRootfs"] != true
        || host["Privileged"] != false
        || host["PidMode"] != ""
        || host["PidsLimit"] != 1
        || host["CpuQuota"] != 100000
        || host["CpuPeriod"] != 100000
        || host["IpcMode"] != "private"
        || host["CgroupnsMode"] != "private"
        || host["Memory"].as_u64().map(|n| n.to_string()).as_deref() != Some(memory)
        || host["MemorySwap"] != host["Memory"]
        || value["Mounts"].as_array().is_none_or(|m| !m.is_empty())
        || host["CapDrop"] != json!(["ALL"])
        || !host["SecurityOpt"].as_array().is_some_and(|s| {
            s.iter().any(|v| v == "no-new-privileges")
                && s.iter().any(|v| {
                    v.as_str()
                        .and_then(|s| s.strip_prefix("seccomp="))
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .is_some_and(|v| v == seccomp())
                })
        })
    {
        return Err("container_parser_runtime_policy_changed".into());
    }
    Ok(())
}
struct OwnedContainer<'a> {
    runtime: &'a Runtime,
    scratch: &'a Scratch,
    name: String,
    nonce: String,
    created_id: Option<String>,
    cleaned: bool,
}
impl OwnedContainer<'_> {
    fn inspect(&self) -> Result<Value, String> {
        let got = self.runtime.command(
            self.scratch,
            &["container", "inspect", &self.name],
            None,
            512 * 1024,
            Duration::from_secs(5),
            &|| false,
        )?;
        let data: Value =
            serde_json::from_slice(&got.stdout).map_err(|_| "container_parser_inspect_failed")?;
        if data.as_array().is_none_or(|v| v.len() != 1)
            || data[0]["Config"]["Labels"][OWN_LABEL] != self.nonce
            || self
                .created_id
                .as_ref()
                .is_some_and(|id| data[0]["Id"] != *id)
        {
            return Err("container_parser_owner_changed".into());
        }
        Ok(data[0].clone())
    }
    fn kill(&self) -> Result<(), String> {
        let value = self.inspect()?;
        if value["State"]["Running"] == true {
            let id = value["Id"].as_str().ok_or("container_parser_id_invalid")?;
            let killed = self.runtime.command(
                self.scratch,
                &["kill", "--signal=KILL", id],
                None,
                8192,
                Duration::from_secs(5),
                &|| false,
            )?;
            if !killed.success {
                return Err("container_parser_kill_failed".into());
            }
        }
        Ok(())
    }
    fn cleanup(&mut self) -> Result<(), String> {
        let original = self.inspect()?;
        let id = original["Id"]
            .as_str()
            .ok_or("container_parser_id_invalid")?;
        let gone = self.runtime.command(
            self.scratch,
            &["rm", "--force", id],
            None,
            8192,
            Duration::from_secs(5),
            &|| false,
        )?;
        if !gone.success {
            return Err("container_parser_cleanup_failed".into());
        }
        let absent = self.runtime.command(
            self.scratch,
            &["container", "inspect", id],
            None,
            8192,
            Duration::from_secs(5),
            &|| false,
        )?;
        let diagnostic = String::from_utf8_lossy(&absent.stderr);
        if absent.success
            || !absent.stop_reason.is_empty()
            || !(diagnostic.contains("No such container:")
                || diagnostic.contains("No such object:"))
        {
            return Err("container_parser_cleanup_unverified".into());
        }
        self.cleaned = true;
        fs::remove_file(self.scratch.path.join("container.json")).map_err(|e| e.to_string())?;
        Ok(())
    }
}
impl Drop for OwnedContainer<'_> {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = self.cleanup();
        }
    }
}
struct Scratch {
    path: PathBuf,
    _lock: fs::File,
}
impl Scratch {
    fn new() -> Result<Self, String> {
        let path =
            std::env::temp_dir().join(format!("oviraptor-parser-runtime-{}", uuid::Uuid::new_v4()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .map_err(|e| e.to_string())?;
        }
        #[cfg(not(unix))]
        {
            return Err("unsupported_sandbox:private_runtime_directory_unverified".into());
        }
        let lock = orphans::create_lock(&path)?;
        Ok(Self { path, _lock: lock })
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // An unverified cleanup is durable work, never silently discarded.
        if !self.path.join("container.json").exists() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
struct Captured {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    success: bool,
    stop_reason: String,
}
#[cfg(unix)]
fn nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("container_parser_pipe_unavailable".into());
    }
    Ok(())
}
fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>, bound: usize) -> Result<bool, String> {
    let mut chunk = [0; 4096];
    for _ in 0..16 {
        match pipe.read(&mut chunk) {
            Ok(0) => return Ok(false),
            Ok(n) => {
                let keep = (bound.saturating_sub(bytes.len())).min(n);
                bytes.extend_from_slice(&chunk[..keep]);
                if keep < n {
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(format!("container_parser_pipe_failed:{e}")),
        }
    }
    Ok(false)
}
fn capture(
    child: &mut Child,
    bound: usize,
    timeout: Duration,
    cancelled: &dyn Fn() -> bool,
) -> Result<Captured, String> {
    #[cfg(not(unix))]
    {
        let _ = (child, bound, timeout, cancelled);
        return Err("unsupported_sandbox:container_parser_platform_unverified".into());
    }
    #[cfg(unix)]
    {
        let mut out = child.stdout.take().ok_or("container_parser_pipe_missing")?;
        let mut err = child.stderr.take().ok_or("container_parser_pipe_missing")?;
        nonblocking(&out)?;
        nonblocking(&err)?;
        let started = Instant::now();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut stop = String::new();
        let status = loop {
            let excess = drain(&mut out, &mut stdout, bound)?
                | drain(&mut err, &mut stderr, bound.saturating_sub(stdout.len()))?;
            if excess {
                stop = "output_limit_exceeded".into();
            } else if cancelled() {
                stop = "cancelled".into();
            } else if started.elapsed() >= timeout {
                stop = "timeout".into();
            }
            if !stop.is_empty() {
                let _ = child.kill();
                break child.wait().map_err(|e| e.to_string())?;
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let excess = drain(&mut out, &mut stdout, bound)?
            | drain(&mut err, &mut stderr, bound.saturating_sub(stdout.len()))?;
        if excess && stop.is_empty() {
            stop = "output_limit_exceeded".into();
        }
        Ok(Captured {
            stdout,
            stderr,
            success: status.success(),
            stop_reason: stop,
        })
    }
}

fn seccomp() -> Value {
    json!({"defaultAction":"SCMP_ACT_ALLOW","syscalls":[{"names":[
    "socket","socketpair","connect","bind","listen","accept","accept4","sendto","sendmsg","sendmmsg",
    "clone","clone3","fork","vfork","unshare","setns","mount","umount2","pivot_root","ptrace","bpf","keyctl",
    "add_key","request_key","userfaultfd","open_by_handle_at","perf_event_open","reboot","kexec_load","kexec_file_load","init_module","finit_module",
    "io_uring_setup","io_uring_enter","io_uring_register","delete_module","process_vm_readv","process_vm_writev"
],"action":"SCMP_ACT_ERRNO","errnoRet":1}]})
}

const WRAPPER: &str = r#"import sys,json,hashlib,os,ctypes,platform,resource
doc=json.load(sys.stdin)
code=doc['code']; input_utf8=doc['input_utf8']; data=json.loads(input_utf8)
digest=lambda b:'sha256:'+hashlib.sha256(b).hexdigest()
if digest(code.encode())!=doc['code_sha256']: raise ValueError('code_digest_mismatch')
if digest(input_utf8.encode())!=doc['input_sha256']: raise ValueError('input_digest_mismatch')
memory=open('/sys/fs/cgroup/memory.max').read().strip()
cpu=open('/sys/fs/cgroup/cpu.max').read().strip()
pids=open('/sys/fs/cgroup/pids.max').read().strip()
if memory!=doc['memory'] or cpu!='100000 100000' or pids!='1': raise ValueError('cgroup_contract_mismatch')
libc=ctypes.CDLL(None,use_errno=True)
class Filter(ctypes.Structure): _fields_=[('code',ctypes.c_ushort),('jt',ctypes.c_ubyte),('jf',ctypes.c_ubyte),('k',ctypes.c_uint)]
class Program(ctypes.Structure): _fields_=[('len',ctypes.c_ushort),('filter',ctypes.POINTER(Filter))]
arch=platform.machine()
calls=[221,281] if arch=='aarch64' else [59,322] if arch=='x86_64' else None
if calls is None: raise ValueError('architecture_unsupported')
audit_arch=0xc00000b7 if arch=='aarch64' else 0xc000003e
rules=[Filter(0x20,0,0,4),Filter(0x15,1,0,audit_arch),Filter(0x06,0,0,0x80000000),Filter(0x20,0,0,0)]
if arch=='x86_64': rules.extend([Filter(0x45,0,1,0x40000000),Filter(0x06,0,0,0x80000000)])
for call in calls: rules.extend([Filter(0x15,0,1,call),Filter(0x06,0,0,0x00050001)])
rules.append(Filter(0x06,0,0,0x7fff0000))
array=(Filter*len(rules))(*rules); program=Program(len(rules),array)
if libc.prctl(38,1,0,0,0)!=0 or libc.prctl(22,2,ctypes.byref(program),0,0)!=0: raise OSError('exec_filter_failed')
resource.setrlimit(resource.RLIMIT_FSIZE,(doc['output_limit'],doc['output_limit']))
os.environ.clear()
before=resource.getrusage(resource.RUSAGE_SELF)
namespace={}; exec(compile(code,'<frozen-helper>','exec'),namespace)
result=namespace['analyze'](data)
after=resource.getrusage(resource.RUSAGE_SELF)
encoded=json.dumps({'schemaVersion':1,'codeSha256':doc['code_sha256'],'inputSha256':doc['input_sha256'],
 'resourceUsage':{'cpuSeconds':after.ru_utime+after.ru_stime-before.ru_utime-before.ru_stime,'maxRssKib':after.ru_maxrss},'result':result},ensure_ascii=False,separators=(',',':'))
if len(encoded.encode())>doc['output_limit']: raise ValueError('output_limit_exceeded')
print(encoded)
"#;

#[cfg(all(test, target_os = "macos"))]
#[path = "container_parser/tests.rs"]
mod tests;
