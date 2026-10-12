use super::*;
use std::{process::{Child, Command, Stdio}, time::{Duration, Instant}};

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait_marker(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "missing barrier: {}", path.display());
}
fn child(path: &Path, marker: &Path, hold: bool) -> ChildGuard {
    ChildGuard(Command::new(std::env::current_exe().unwrap())
        .args(["agent_runtime::execution_owner::tests::recovery_queue_cross_process_release_and_crash", "--exact"])
        .env("OVIRAPTOR_QUEUE_TEST_DB", path)
        .env("OVIRAPTOR_QUEUE_TEST_MARKER", marker)
        .env("OVIRAPTOR_QUEUE_TEST_HOLD", if hold { "1" } else { "0" })
        .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap())
}

#[test]
fn recovery_queue_cross_process_release_and_crash() {
    if let Some(path) = std::env::var_os("OVIRAPTOR_QUEUE_TEST_DB") {
        let marker = std::path::PathBuf::from(std::env::var_os("OVIRAPTOR_QUEUE_TEST_MARKER").unwrap());
        fs::write(marker.with_extension("ready"), b"before queue").unwrap();
        let _owner = queue_recovery_approval(Path::new(&path)).unwrap();
        fs::write(&marker, b"owned").unwrap();
        if std::env::var("OVIRAPTOR_QUEUE_TEST_HOLD").as_deref() == Ok("1") {
            std::thread::sleep(Duration::from_secs(30));
        }
        return;
    }
    let directory = std::env::temp_dir().join(format!("oviraptor-queue-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("fixture.db");
    fs::write(&path, b"disposable queue fixture").unwrap();
    let owner = queue_recovery_approval(&path).unwrap();
    let (_, lock_path) = invocation_path(&path, "", 0, "recovery_approval", "database").unwrap();
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(&lock_path).unwrap().ino()
    };
    let marker = directory.join("waiter.acquired");
    let mut waiter = child(&path, &marker, false);
    wait_marker(&marker.with_extension("ready"));
    std::thread::sleep(Duration::from_millis(100));
    assert!(!marker.exists());
    assert!(waiter.0.try_wait().unwrap().is_none());
    drop(owner);
    wait_marker(&marker);
    assert!(waiter.0.wait().unwrap().success());

    let held = directory.join("holder.acquired");
    let mut holder = child(&path, &held, true);
    wait_marker(&held);
    let after = directory.join("after-crash.acquired");
    let mut next = child(&path, &after, false);
    wait_marker(&after.with_extension("ready"));
    std::thread::sleep(Duration::from_millis(100));
    assert!(!after.exists());
    holder.0.kill().unwrap();
    assert!(!holder.0.wait().unwrap().success());
    wait_marker(&after);
    assert!(next.0.wait().unwrap().success());
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(&lock_path).unwrap().ino(), inode);
    }
    assert_eq!(fs::read(&path).unwrap(), b"disposable queue fixture");
    fs::remove_dir_all(&directory).unwrap();
}
