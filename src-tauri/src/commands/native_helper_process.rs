// Private machine capture stays separate from durable diagnostic text.
#[cfg_attr(not(test), allow(dead_code))]
struct NativeHelperCapture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stderr_truncated: bool,
}
fn run_native_json_helper_raw(
    helper: &Path,
    payload: &JsonValue,
    runtime_path: &OsString,
    proxy: Option<&str>,
    no_proxy: &str,
    timeout: Duration,
    control: &NativeReconControl,
) -> Result<Vec<u8>, String> {
    run_native_json_helper_capture(
        helper,
        payload,
        runtime_path,
        proxy,
        no_proxy,
        timeout,
        control,
    )
    .map(|v| v.stdout)
}
fn run_native_json_helper_capture(
    helper: &Path,
    payload: &JsonValue,
    runtime_path: &OsString,
    proxy: Option<&str>,
    no_proxy: &str,
    timeout: Duration,
    control: &NativeReconControl,
) -> Result<NativeHelperCapture, String> {
    control.check()?;
    // Unit tests use a synthetic fixture worker for cancellation/pipe tests.
    // Every shipped worker, including test invocations of its real filename,
    // takes the same integrity gate before a process is spawned.
    #[cfg(test)]
    let synthetic_fixture = matches!(
        helper.file_name().and_then(|name| name.to_str()),
        Some("fixture.cjs" | "fake_runtime_probe.cjs")
    );
    #[cfg(not(test))]
    let synthetic_fixture = false;
    if !synthetic_fixture {
        verify_bundled_worker(helper)?;
    }
    let input = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
    let node = resolve_helper_node(runtime_path)?;
    let mut observer = control
        .log_owner
        .as_ref()
        .map(|owner| {
            NativeHelperObserver::begin(
                owner,
                helper,
                &input,
                &serde_json::json!([
                    format!("{:x}", Sha256::digest(node.as_os_str().as_encoded_bytes())),
                    format!("{:x}", Sha256::digest(runtime_path.as_encoded_bytes())),
                    proxy,
                    no_proxy
                ]),
            )
        })
        .transpose()?;
    let mut command = Command::new(&node);
    configure_child_command(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .arg(helper)
        .env("PATH", runtime_path)
        .env("NO_PROXY", no_proxy)
        .env("no_proxy", no_proxy);
    // Same FD_CLOEXEC-atomic capture pipes as native_pipeline::process::run: a
    // concurrent spawn elsewhere can never inherit (and hold open) these ends.
    #[cfg(unix)]
    let (mut stdin, stdout, stderr) = {
        use crate::native_pipeline::process::capture_pipe;
        let unavailable = |_| "helper_output_pipe_unavailable".to_string();
        let (in_read, in_write) = capture_pipe().map_err(unavailable)?;
        let (out_read, out_write) = capture_pipe().map_err(unavailable)?;
        let (err_read, err_write) = capture_pipe().map_err(unavailable)?;
        command.stdin(in_read).stdout(out_write).stderr(err_write);
        (in_write, out_read, err_read)
    };
    #[cfg(not(unix))]
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(proxy) = proxy.filter(|value| !value.trim().is_empty()) {
        command.env("HTTPS_PROXY", proxy).env("HTTP_PROXY", proxy);
    }
    control.check()?;
    let spawned = command.spawn();
    // The parent's copies of the child ends close here, before any read/write.
    drop(command);
    let child = spawned.map_err(|error| format!("无法启动 {}：{error}", helper.display()))?;
    let mut child = crate::native_pipeline::process::OwnedChild::new(child);
    #[cfg(not(unix))]
    let (mut stdin, stdout, stderr) = (
        child.0.stdin.take().expect("piped stdin"),
        child.0.stdout.take().expect("piped stdout"),
        child.0.stderr.take().expect("piped stderr"),
    );
    crate::native_pipeline::process::interruptible(&stdout)
        .map_err(|_| "helper_output_pipe_unavailable")?;
    crate::native_pipeline::process::interruptible(&stderr)
        .map_err(|_| "helper_output_pipe_unavailable")?;
    // The three pipes must be drained concurrently; otherwise a large JSON
    // capture can fill stdout and deadlock a parent waiting for process exit.
    let writer = thread::spawn(move || stdin.write_all(&input));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (out_send, out_read) = mpsc::channel();
    let (err_send, err_read) = mpsc::channel();
    let stdout_stop = stop.clone();
    let stderr_stop = stop.clone();
    // Machine JSON stdout is NEVER sent to diagnostic framing/journal.
    thread::spawn(move || {
        crate::native_pipeline::process::output::pump(
            stdout,
            32 * 1024 * 1024,
            stdout_stop,
            out_send,
            None,
        )
    });
    let diagnostic = observer.as_ref().map(|o| o.channel.pipe("stderr"));
    thread::spawn(move || {
        crate::native_pipeline::process::output::pump(
            stderr,
            64 * 1024,
            stderr_stop,
            err_send,
            diagnostic,
        )
    });
    let started = Instant::now();
    let status = loop {
        if let Err(error) = control.check() {
            break Err(error);
        }
        if started.elapsed() >= timeout {
            break Err(format!(
                "{} 超过 {} 秒",
                helper.display(),
                timeout.as_secs()
            ));
        }
        if let Some(o) = &mut observer {
            o.deliver();
        }
        match child.0.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => break Err(error.to_string()),
            Ok(None) => thread::sleep(Duration::from_millis(5)),
        }
    };
    // Reuse the actual process group owner even on normal exit. Descendants
    // cannot hold inherited pipes open or publish a late effect after return.
    #[cfg(not(unix))]
    force_stop_sentinel_process(child.0.id() as i64);
    let cleanup = child.stop();
    stop.store(true, std::sync::atomic::Ordering::Release);
    let written = writer.join().map_err(|_| "helper 输入线程退出".to_string());
    let (output, truncated) = out_read
        .recv_timeout(Duration::from_millis(250))
        .map_err(|_| "helper_output_drain_incomplete")?;
    let (error_output, stderr_truncated) = err_read
        .recv_timeout(Duration::from_millis(250))
        .map_err(|_| "helper_output_drain_incomplete")?;
    if let Some(o) = &mut observer {
        o.drain();
    }
    let terminal = if control.cancelled.load(std::sync::atomic::Ordering::Acquire) {
        "cancelled"
    } else if started.elapsed() >= timeout || Instant::now() >= control.deadline {
        "timeout"
    } else if status.as_ref().is_ok_and(|s| s.success())
        && !truncated
        && written.as_ref().is_ok_and(|w| w.is_ok())
        && cleanup.is_none()
        && serde_json::from_slice::<JsonValue>(&output).is_ok()
    {
        "completed"
    } else {
        "failed"
    };
    let log_failure = observer
        .as_mut()
        .and_then(|o| o.finish(terminal, cleanup.is_some()).err());
    if cleanup.is_some() {
        return Err("helper_process_cleanup_incomplete".into());
    }
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            return Err(format!(
                "{error}；辅助程序 stderr：{}；日志状态：{}",
                native_helper_stderr_note(&error_output),
                log_failure.as_deref().unwrap_or("committed")
            ));
        }
    };
    if let Some(error) = log_failure {
        return Err(error);
    }
    written?.map_err(|error| error.to_string())?;
    control.check()?;
    if !status.success() {
        return Err(format!(
            "{} {}；辅助程序 stderr：{}",
            helper.display(),
            native_helper_exit_label(&status),
            native_helper_stderr_note(&error_output)
        ));
    }
    if truncated {
        return Err("helper JSON 超过 32 MiB，未将截断数据当作完整证据".into());
    }
    serde_json::from_slice::<JsonValue>(&output)
        .map_err(|error| format!("{} JSON 无效：{error}", helper.display()))?;
    Ok(NativeHelperCapture {
        stdout: output,
        stderr: error_output,
        stderr_truncated,
    })
}

#[cfg(all(test, unix))]
mod native_helper_live_tests {
    use super::*;
    include!("native_helper_live_tests.rs");
}
