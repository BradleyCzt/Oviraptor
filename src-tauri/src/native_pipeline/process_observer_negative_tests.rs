#[test]
fn native_process_observer_cancellation_strong_kill_flushes_tail_and_stops_descendants() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let result = f.run(
        "printf 'ready\\n'; printf 'password=partial-synthetic-secret' >&2;
        (sleep 0.2; printf escaped > escaped) & printf ready > cancel; wait",
        &|| f.root.join("cancel").exists(),
        &mut |event| journal.append(event, &mut |_| Ok(())).map(|_| ()),
    );
    assert!(result.run.cancelled);
    assert!(!result.run.succeeded());
    let page = f.read(0, 300);
    assert!(page.rows.iter().any(|row| row.message == "ready"));
    assert!(page
        .rows
        .iter()
        .any(|row| row.hint.stream == "stderr" && row.message.contains("<redacted:")));
    assert!(!format!("{:?}", page.rows).contains("partial-synthetic-secret"));
    let count = page.rows.len();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!f.root.join("escaped").exists());
    assert_eq!(
        f.read(0, 300).rows.len(),
        count,
        "no late observer append after return"
    );
}

#[test]
fn native_process_observer_oversized_record_is_omitted_whole_and_flood_is_reported_as_gap() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let script = "printf 'password=oversize-secret'; i=0; while [ $i -lt 17000 ]; do printf x; i=$((i+1)); done;
        printf '\\n'; i=0; while [ $i -lt 8192 ]; do printf 'flood-line\\n'; i=$((i+1)); done";
    let result = f.run(script, &|| false, &mut |event| {
        std::thread::sleep(Duration::from_millis(4));
        journal.append(event, &mut |_| Ok(())).map(|_| ())
    });
    assert!(
        result.log_error.is_some(),
        "queue loss must never claim complete logs"
    );
    let page = f.read(0, 300);
    assert!(!format!("{:?}", page.rows).contains("oversize-secret"));
    assert!(
        page.rows.iter().any(|row| row.gap),
        "overflow or oversize must persist a visible gap"
    );
    assert!(
        page.rows
            .iter()
            .any(|row| row.hint.stream == "gap" && row.message.contains("queue")),
        "a queue overflow must have its own durable gap marker"
    );
}

#[test]
fn native_process_observer_failed_append_never_notifies_or_claims_complete_capture() {
    let f = Fixture::new();
    let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
    let admin = Connection::open(&f.db).unwrap();
    admin
        .execute_batch(
            "CREATE TRIGGER deny_fixture_row BEFORE INSERT ON native_process_log_rows
        BEGIN SELECT RAISE(ABORT,'fixture storage denial'); END;",
        )
        .unwrap();
    let mut notified = 0;
    let result = f.run("printf 'plain\\n'", &|| false, &mut |event| {
        journal
            .append(event, &mut |_| {
                notified += 1;
                Ok(())
            })
            .map(|_| ())
    });
    assert!(result.log_error.is_some());
    assert_eq!(notified, 0);
    assert!(f.read(0, 300).rows.is_empty());
    assert_eq!(result.run.stdout, b"plain\n");
    assert_eq!(
        admin
            .query_row(
                "SELECT row_count FROM native_process_log_executions",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

/// A group SIGKILL alone misses members that are mid-fork (observed escapee: the
/// orphaned `sleep 0.2` of the strong-kill test holding stdout/stderr as fd 1/2).
/// `OwnedChild::stop` must leave the process group empty when it returns.
#[test]
fn native_process_stop_leaves_no_group_member_forked_during_the_group_kill() {
    use std::os::unix::process::CommandExt;
    for trial in 0..8 {
        let child = std::process::Command::new("/bin/sh")
            .args(["-c", "i=0; while [ $i -lt 4000 ]; do sleep 2 & i=$((i+1)); done; wait"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let pgid = child.id() as i32;
        let mut child = OwnedChild::new(child);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(child.stop(), None, "trial {trial}");
        // SAFETY: probe-only signal 0 to the group this test created.
        let alive = unsafe { libc::kill(-pgid, 0) } == 0;
        if alive {
            // SAFETY: reap escapees of this test's own group before failing.
            unsafe { libc::kill(-pgid, libc::SIGKILL) };
        }
        assert!(!alive, "trial {trial}: a forked descendant escaped the group kill");
    }
}

#[test]
fn native_process_observer_cancel_during_fork_storm_still_flushes_redacted_tail() {
    for trial in 0..5 {
        let f = Fixture::new();
        let mut journal = log::Journal::begin(&f.db, f.scope.clone()).unwrap();
        let result = f.run(
            "printf 'password=fork-storm-secret' >&2; printf ready > cancel;
            i=0; while [ $i -lt 4000 ]; do sleep 2 & i=$((i+1)); done; wait",
            &|| f.root.join("cancel").exists(),
            &mut |event| journal.append(event, &mut |_| Ok(())).map(|_| ()),
        );
        assert!(result.run.cancelled, "trial {trial}");
        assert_eq!(result.run.cleanup_error, None, "trial {trial}");
        assert_eq!(result.log_error, None, "trial {trial}");
        let page = f.read(0, 300);
        assert!(
            page.rows
                .iter()
                .any(|row| row.hint.stream == "stderr" && row.message.contains("<redacted:")),
            "trial {trial}: escaped descendants must not hold the pipes past the drain grace"
        );
        assert!(!format!("{:?}", page.rows).contains("fork-storm-secret"));
    }
}

/// Darwin answers a group kill with EPERM while the only member is a zombie, so
/// an exited-but-unreaped leader must not be reported as a cleanup failure.
#[test]
fn native_process_stop_of_exited_unreaped_leader_is_not_a_cleanup_failure() {
    use std::os::unix::process::CommandExt;
    for trial in 0..8 {
        let child = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let pgid = child.id() as i32;
        let mut child = OwnedChild::new(child);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(child.stop(), None, "trial {trial}");
        // SAFETY: probe-only signal 0 to the group this test created.
        assert_eq!(unsafe { libc::kill(-pgid, 0) }, -1, "trial {trial}");
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH),
            "trial {trial}"
        );
    }
}

/// Killed orphans linger as zombies until launchd reaps them; meanwhile Darwin
/// answers the group kill with EPERM. The sweep must wait them out, not fail.
#[cfg(target_os = "macos")]
#[test]
fn native_process_group_sweep_waits_out_a_zombie_only_group() {
    use std::os::unix::process::CommandExt;
    for trial in 0..4 {
        let mut leader = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let pgid = leader.id() as i32;
        std::thread::sleep(Duration::from_millis(40));
        // SAFETY: probe-only signal 0; the unreaped zombie leader is the only member.
        assert_eq!(unsafe { libc::kill(-pgid, 0) }, -1, "trial {trial}");
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM),
            "trial {trial}: precondition — zombie-only group answers EPERM"
        );
        let sweep = std::thread::spawn(move || sweep_process_group(pgid, Instant::now()));
        std::thread::sleep(Duration::from_millis(60));
        assert!(!sweep.is_finished(), "trial {trial}: EPERM must be retried");
        leader.wait().unwrap();
        assert_eq!(sweep.join().unwrap(), None, "trial {trial}");
    }
}

/// Observed residual holder: an unrelated `/bin/sh` + child keeping a capture
/// write end at fd 50 (inherited inside std's macOS pipe()->FD_CLOEXEC window).
/// A stranger spawned the instant each capture descriptor exists must not keep it
/// past exec: EOF arrives as soon as our own write end closes.
#[test]
fn native_process_capture_pipe_is_never_inherited_by_a_spawn_at_its_birth() {
    use std::io::Read;
    let mut strangers = Vec::new();
    let (mut read, write) = capture_pipe_observed(&mut |_| {
        strangers.push(
            std::process::Command::new("/bin/sleep")
                .arg("2")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
    })
    .unwrap();
    assert_eq!(strangers.len(), 2, "one stranger per descriptor birth");
    for fd in [std::os::fd::AsRawFd::as_raw_fd(&read), std::os::fd::AsRawFd::as_raw_fd(&write)] {
        // SAFETY: F_GETFD on a descriptor owned by this test.
        assert_ne!(unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
    }
    drop(write);
    std::thread::sleep(Duration::from_millis(100)); // strangers have exec'd `sleep`
    interruptible(&read).unwrap();
    let mut byte = [0u8; 1];
    let eof = read.read(&mut byte);
    for mut stranger in strangers {
        let _ = stranger.kill();
        let _ = stranger.wait();
    }
    assert_eq!(eof.ok(), Some(0), "a stranger still holds the capture write end");
    // The production entry points use only these pipes for captured streams.
    let process = include_str!("../execution_service/host_process.rs");
    assert!(process.contains("let (out_read, out_write) = capture_pipe().map_err(pipe_setup)?;"));
    assert!(process.contains("let (err_read, err_write) = capture_pipe().map_err(pipe_setup)?;"));
    assert!(process.contains("Some(pipe2) => unsafe { pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) },"));
    assert!(process.contains("libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC)"));
    assert!(!process.contains("libc::mkfifo("), "a capture FIFO stalls large output");
    let helper = include_str!("../commands/native_helper_process.rs");
    assert_eq!(helper.matches("capture_pipe().map_err(unavailable)?").count(), 3);
}

#[test]
fn native_process_capture_pipe_buffers_like_an_anonymous_pipe() {
    // A capture FIFO (atomic O_CLOEXEC, but 8 KiB) made the 2 ms WouldBlock tick
    // move one small buffer per tick: 32 MiB of helper JSON overran its 10 s budget.
    use std::io::Write;
    fn capacity(mut write: impl Write + std::os::fd::AsRawFd) -> usize {
        interruptible(&write).unwrap();
        let chunk = [b'x'; 1024];
        let mut total = 0;
        loop {
            match write.write(&chunk) {
                Ok(n) => total += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return total,
                Err(e) => panic!("{e}"),
            }
        }
    }
    let (_capture_read, capture_write) = capture_pipe().unwrap();
    let (_anon_read, anon_write) = std::io::pipe().unwrap();
    let (capture, anonymous) = (capacity(capture_write), capacity(anon_write));
    assert!(capture >= 64 * 1024, "capture pipe holds only {capture} bytes");
    assert_eq!(capture, anonymous);
}
