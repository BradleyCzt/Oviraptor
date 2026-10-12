//! §10.2 item 7 — one bounded way to run an external program. Arguments are always an
//! array (never a shell string), output is capped, and both a deadline and a cancel
//! check are mandatory, so no caller can invent a fourth variant of this.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

#[path = "process_log/mod.rs"]
pub(crate) mod log;
#[path = "process_output.rs"]
pub(crate) mod output;
use output::pump;

pub(crate) struct ObservedRun {
    pub run: BoundedRun,
    pub log_error: Option<String>,
}

type Observer<'a> = dyn FnMut(&log::Event) -> Result<(), String> + 'a;

pub(crate) fn run_observed(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    limits: &ProcessLimits,
    cancelled: &dyn Fn() -> bool,
    observe: &mut dyn FnMut(&log::Event) -> Result<(), String>,
) -> Result<ObservedRun, ProcessError> {
    run_inner(program, args, cwd, limits, cancelled, Some(observe))
}

#[derive(Clone, Debug)]
pub struct ProcessLimits {
    pub timeout: Duration,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub poll: Duration,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            stdout_bytes: 4 * 1024 * 1024,
            stderr_bytes: 1024 * 1024,
            poll: Duration::from_millis(20),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BoundedRun {
    pub exit: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    pub duration: Duration,
    /// An incomplete drain/cleanup must never be reported as successful execution.
    pub cleanup_error: Option<String>,
}

impl BoundedRun {
    pub fn succeeded(&self) -> bool {
        !self.timed_out && !self.cancelled && self.cleanup_error.is_none() && self.exit == Some(0)
    }

    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    pub fn stderr_text(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }
}

/// A missing program is reported apart from a failed one, because the caller has to
/// record a coverage gap rather than a scan failure (§10.2 item 9).
#[derive(Clone, Debug)]
pub enum ProcessError {
    Missing { program: String, reason: String },
    Launch { program: String, reason: String },
}

impl ProcessError {
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            Self::Missing { program, reason } => format!("缺少可执行文件 {program}：{reason}"),
            Self::Launch { program, reason } => format!("无法启动 {program}：{reason}"),
        }
    }
}

#[cfg(unix)]
pub(crate) fn interruptible(pipe: &impl std::os::fd::AsRawFd) -> std::io::Result<()> {
    let fd = pipe.as_raw_fd();
    // SAFETY: fd is borrowed from a live, exclusively owned child pipe. Neither
    // call consumes it; flags are changed only for this owned read endpoint.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn interruptible<T>(_pipe: &T) -> std::io::Result<()> {
    Ok(())
}

/// One captured stream as (parent read end, child write end), both carrying
/// FD_CLOEXEC from the instant they exist. `Stdio::piped()` on macOS creates the
/// pair with `pipe()` and only then sets FD_CLOEXEC; a process spawned by any
/// other thread inside that window inherits the write end (observed holder: an
/// unrelated `/bin/sh` and its child at fd 50) and EOF waits for that stranger to
/// exit. `pipe2(O_CLOEXEC)` closes the window atomically, with no lock and no
/// serialization. A FIFO opened with O_CLOEXEC is atomic too but holds 8 KiB and
/// loses poll wakeups under a blocked writer (32 MiB of helper JSON took >10 s),
/// so it is not used.
#[cfg(unix)]
pub(crate) fn capture_pipe() -> std::io::Result<(std::fs::File, std::fs::File)> {
    capture_pipe_observed(&mut |_| ())
}

/// `born` runs the instant each descriptor exists (tests spawn a stranger there).
/// macOS exports `pipe2` only from 27.0, so it is resolved at run time. Older
/// macOS keeps std's own `pipe()` + FD_CLOEXEC sequence (no regression, window
/// remains) and `born` then runs inside that window so the test exposes it.
#[cfg(target_os = "macos")]
fn capture_pipe_observed(
    born: &mut dyn FnMut(std::os::fd::RawFd),
) -> std::io::Result<(std::fs::File, std::fs::File)> {
    use std::os::fd::{FromRawFd, OwnedFd};
    type Pipe2 = unsafe extern "C" fn(*mut libc::c_int, libc::c_int) -> libc::c_int;
    static PIPE2: std::sync::OnceLock<Option<Pipe2>> = std::sync::OnceLock::new();
    let pipe2 = *PIPE2.get_or_init(|| {
        // SAFETY: NUL-terminated name looked up in the default namespace.
        let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"pipe2".as_ptr()) };
        // SAFETY: a non-null `pipe2` is libSystem's POSIX 2024 pipe2(int[2], int).
        (!symbol.is_null()).then(|| unsafe { std::mem::transmute::<*mut libc::c_void, Pipe2>(symbol) })
    });
    let mut fds = [0; 2];
    // SAFETY: both calls fill two new descriptors that are owned immediately below.
    let created = match pipe2 {
        Some(pipe2) => unsafe { pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) },
        None => unsafe { libc::pipe(fds.as_mut_ptr()) },
    };
    if created != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    born(fds[0]);
    born(fds[1]);
    if pipe2.is_none() {
        for fd in fds {
            // SAFETY: F_SETFD on a descriptor owned above.
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
    }
    Ok((std::fs::File::from(read), std::fs::File::from(write)))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn capture_pipe_observed(
    born: &mut dyn FnMut(std::os::fd::RawFd),
) -> std::io::Result<(std::fs::File, std::fs::File)> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let mut fds = [0; 2];
    // SAFETY: pipe2 fills two new descriptors that are owned immediately below.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    born(fds[0]);
    born(fds[1]);
    Ok((std::fs::File::from(read), std::fs::File::from(write)))
}

pub(crate) struct OwnedChild(pub(crate) Child, bool);

impl OwnedChild {
    pub(crate) fn new(child: Child) -> Self {
        Self(child, false)
    }
    pub(crate) fn stop(&mut self) -> Option<String> {
        if self.1 {
            return None;
        }
        self.1 = true;
        let mut error = None;
        #[cfg(unix)]
        {
            // SAFETY: process_group(0) created a new group with our child's PID.
            // Only that group is targeted, never the parent/application group.
            // The result is not final: `sweep_process_group` below decides it once
            // the leader is reaped (a zombie leader alone makes Darwin say EPERM).
            unsafe { libc::kill(-(self.0.id() as i32), libc::SIGKILL) };
        }
        let _ = self.0.kill();
        // Never block indefinitely on a child which the OS could not terminate.
        let started = Instant::now();
        loop {
            match self.0.try_wait() {
                Ok(Some(_)) => break,
                Err(failure) => {
                    error = Some(format!("process_reap:{failure}"));
                    break;
                }
                Ok(None) if started.elapsed() >= Duration::from_secs(1) => {
                    error = Some("process_cleanup_deadline".into());
                    break;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
        #[cfg(unix)]
        if let Some(failure) = sweep_process_group(self.0.id() as i32, started) {
            error.get_or_insert(failure);
        }
        error
    }
}

/// A single group SIGKILL races with members that are mid-fork: a child
/// created while the signal is being delivered is not signalled, survives as an
/// orphan in the same group and keeps the inherited stdout/stderr write ends
/// open (delaying EOF past the drain grace). Repeat the group kill until the
/// kernel reports the group empty. The group ID cannot be recycled while any
/// member exists, so after the leader is reaped `kill(-pgid)` reaches only our
/// own escaped descendants or fails with ESRCH.
///
/// Darwin answers EPERM while the only remaining members are zombies (killed
/// orphans not yet reaped by launchd); they hold no descriptors, so EPERM is
/// retried until ESRCH and reported only if it outlives the cleanup deadline.
#[cfg(unix)]
fn sweep_process_group(pgid: i32, started: Instant) -> Option<String> {
    loop {
        // SAFETY: targets only the group created by process_group(0) for our child.
        let pending = if unsafe { libc::kill(-pgid, libc::SIGKILL) } == 0 {
            "process_group_cleanup_deadline".to_string()
        } else {
            let failure = std::io::Error::last_os_error();
            match failure.raw_os_error() {
                Some(libc::ESRCH) => return None,
                Some(libc::EPERM) => format!("process_group_cleanup:{failure}"),
                _ => return Some(format!("process_group_cleanup:{failure}")),
            }
        };
        if started.elapsed() >= Duration::from_secs(1) {
            return Some(pending);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        // The caller disarms the guard after explicit cleanup. This handles early
        // errors (including configuring pipes) and unwind while the child is alive.
        let _ = self.stop();
    }
}

pub fn run(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    limits: &ProcessLimits,
    cancelled: &dyn Fn() -> bool,
) -> Result<BoundedRun, ProcessError> {
    run_inner(program, args, cwd, limits, cancelled, None).map(|observed| observed.run)
}

fn run_inner(
    program: &Path,
    args: &[String],
    cwd: Option<&Path>,
    limits: &ProcessLimits,
    cancelled: &dyn Fn() -> bool,
    mut observe: Option<&mut Observer<'_>>,
) -> Result<ObservedRun, ProcessError> {
    let channel = observe.as_ref().map(|_| output::Channel::new());
    let mut log_error = None;
    let started = Instant::now();
    let already_cancelled = cancelled();
    if already_cancelled || limits.timeout.is_zero() {
        return Ok(ObservedRun {
            log_error: None,
            run: BoundedRun {
                exit: None,
                stdout: vec![],
                stderr: vec![],
                truncated: false,
                timed_out: !already_cancelled,
                cancelled: already_cancelled,
                duration: started.elapsed(),
                cleanup_error: None,
            },
        });
    }
    let pipe_setup = |error: std::io::Error| ProcessError::Launch {
        program: program.display().to_string(),
        reason: format!("pipe_setup:{error}"),
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd.unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::null());
    #[cfg(unix)]
    let captured = {
        let (out_read, out_write) = capture_pipe().map_err(pipe_setup)?;
        let (err_read, err_write) = capture_pipe().map_err(pipe_setup)?;
        command.stdout(out_write).stderr(err_write);
        (out_read, err_read)
    };
    #[cfg(not(unix))]
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let spawned = command.spawn();
    // The parent's copies of the child write ends close here, before any read.
    drop(command);
    let mut child = OwnedChild(
        spawned.map_err(|error| {
            let text = error.to_string();
            let name = program.display().to_string();
            if error.kind() == std::io::ErrorKind::NotFound {
                ProcessError::Missing {
                    program: name,
                    reason: text,
                }
            } else {
                ProcessError::Launch {
                    program: name,
                    reason: text,
                }
            }
        })?,
        false,
    );
    #[cfg(unix)]
    let (stdout_pipe, stderr_pipe) = {
        interruptible(&captured.0).map_err(pipe_setup)?;
        interruptible(&captured.1).map_err(pipe_setup)?;
        (Some(captured.0), Some(captured.1))
    };
    #[cfg(not(unix))]
    let (stdout_pipe, stderr_pipe) = (child.0.stdout.take(), child.0.stderr.take());
    let (out_sender, out_receiver) = mpsc::channel();
    let (err_sender, err_receiver) = mpsc::channel();
    // The reader threads only ever need the two caps, so they take copies rather than
    // borrowing this call's limits.
    let stdout_cap = limits.stdout_bytes;
    let stderr_cap = limits.stderr_bytes;
    let stop_readers = Arc::new(AtomicBool::new(false));
    if let Some(pipe) = stdout_pipe {
        let stop = Arc::clone(&stop_readers);
        let observer = channel.as_ref().map(|channel| channel.pipe("stdout"));
        std::thread::spawn(move || pump(pipe, stdout_cap, stop, out_sender, observer));
    } else {
        let _ = out_sender.send((Vec::new(), false));
    }
    if let Some(pipe) = stderr_pipe {
        let stop = Arc::clone(&stop_readers);
        let observer = channel.as_ref().map(|channel| channel.pipe("stderr"));
        std::thread::spawn(move || pump(pipe, stderr_cap, stop, err_sender, observer));
    } else {
        let _ = err_sender.send((Vec::new(), false));
    }
    let mut timed_out = false;
    let mut was_cancelled = false;
    let mut status = None;
    loop {
        // Cancellation wins before observing a quick success or spawning a next step.
        if cancelled() {
            was_cancelled = true;
            break;
        }
        if started.elapsed() >= limits.timeout {
            timed_out = true;
            break;
        }
        if let (Some(channel), Some(observer)) = (&channel, observe.as_deref_mut()) {
            channel.deliver(observer, &mut log_error, false);
        }
        match child.0.try_wait() {
            Ok(Some(exit)) => {
                status = Some(exit);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                stop_readers.store(true, Ordering::Release);
                return Err(ProcessError::Launch {
                    program: program.display().to_string(),
                    reason: error.to_string(),
                });
            }
        }
        std::thread::sleep(
            limits
                .poll
                .clamp(Duration::from_millis(1), Duration::from_millis(20))
                .min(limits.timeout.saturating_sub(started.elapsed())),
        );
    }
    // Even normal exit must not leave descendants running with inherited pipes.
    let mut cleanup_error = child.stop();
    stop_readers.store(true, Ordering::Release);
    let mut collect = |receiver: mpsc::Receiver<(Vec<u8>, bool)>| {
        receiver
            .recv_timeout(Duration::from_millis(250))
            .unwrap_or_else(|_| {
                cleanup_error = Some("process_output_drain_incomplete".into());
                (vec![], true)
            })
    };
    let (stdout, out_cut) = collect(out_receiver);
    let (stderr, err_cut) = collect(err_receiver);
    if let (Some(channel), Some(observer)) = (&channel, observe) {
        if cleanup_error.is_some() {
            log_error.get_or_insert_with(|| "native_process_log_drain_incomplete".into());
        }
        channel.finish(observer, &mut log_error);
    }
    Ok(ObservedRun {
        log_error,
        run: BoundedRun {
            exit: status.and_then(|exit| exit.code()),
            stdout,
            stderr,
            truncated: out_cut || err_cut,
            timed_out,
            cancelled: was_cancelled,
            duration: started.elapsed(),
            cleanup_error,
        },
    })
}

#[cfg(all(test, unix))]
#[path = "process_observer_tests.rs"]
mod observer_tests;
