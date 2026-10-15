use super::*;
use crate::{db, jobs::tray_activity};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Barrier, Mutex,
    },
    thread,
};

struct Fixture {
    root: PathBuf,
    path: PathBuf,
    conn: rusqlite::Connection,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("oviraptor-job-lifecycle-{}", uuid::Uuid::new_v4()));
        let path = db::initialize(&root).unwrap();
        let conn = db::open(&path).unwrap();
        conn.execute(
            "INSERT INTO projects(id,name) VALUES(9001,'Synthetic lifecycle')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO runs(id,project_id,name,status,progress) VALUES(9001,9001,'Synthetic run','running',92)", []).unwrap();
        Self { root, path, conn }
    }
    fn state(&self) -> (String, String, String, f64) {
        self.conn
            .query_row(
                "SELECT status,error,COALESCE(finished_at,''),progress FROM runs WHERE id=9001",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn late_cancel_cannot_replace_any_persisted_terminal_state() {
    let f = Fixture::new();
    for terminal in ["completed", "failed", "cancelled", "interrupted"] {
        f.conn.execute("UPDATE runs SET status=?1,error='original detail',finished_at='2026-01-01 00:00:00' WHERE id=9001", [terminal]).unwrap();
        let before = f.state();
        assert!(request_cancel_on(&f.conn, 9001)
            .unwrap_err()
            .contains("已结束"));
        assert_eq!(f.state(), before);
    }
    assert!(request_cancel_on(&f.conn, 9999).is_err());
}

#[test]
fn successful_last_import_still_honors_cancellation_before_terminal_commit() {
    let mut f = Fixture::new();
    let event = finish_with_fallback(&mut f.conn, 9001, Ok(()), true).unwrap();
    assert_eq!(event.status, "cancelled");
    assert_eq!(f.state().0, "cancelled");
    assert_eq!(f.state().3, 92.0);
    f.conn
        .execute(
            "UPDATE runs SET status='running',finished_at=NULL WHERE id=9001",
            [],
        )
        .unwrap();
    request_cancel_on(&f.conn, 9001).unwrap();
    let event = finish_with_fallback(&mut f.conn, 9001, Ok(()), false).unwrap();
    assert_eq!(event.status, "cancelled");
    assert!(!f.state().2.is_empty());
}

#[test]
fn simultaneous_finish_and_cancel_have_one_durable_winner_and_release_registry() {
    let f = Fixture::new();
    let registry = Arc::new(Mutex::new(HashMap::new()));
    let flag = Arc::new(AtomicBool::new(false));
    registry.lock().unwrap().insert(9001i64, flag.clone());
    let count = Arc::new(AtomicUsize::new(0));
    let owned_registry = registry.clone();
    let (activity, _) = tray_activity::JobActivityGuard::begin(count.clone(), move |_| {
        owned_registry.lock().unwrap().remove(&9001);
    });
    let barrier = Arc::new(Barrier::new(2));
    let worker_barrier = barrier.clone();
    let path = f.path.clone();
    let worker_flag = flag.clone();
    let worker = thread::spawn(move || {
        let _activity = activity;
        let mut conn = db::open(&path).unwrap();
        worker_barrier.wait();
        finish_with_fallback(&mut conn, 9001, Ok(()), worker_flag.load(Ordering::Relaxed))
            .unwrap()
            .status
    });
    barrier.wait();
    let cancellation = {
        let active = registry.lock().unwrap();
        if let Some(flag) = active.get(&9001) {
            flag.store(true, Ordering::Relaxed);
            request_cancel_on(&f.conn, 9001)
        } else {
            Err("worker already released".into())
        }
    };
    let terminal = worker.join().unwrap();
    assert!(matches!(terminal.as_str(), "completed" | "cancelled"));
    if cancellation.is_ok() {
        assert_eq!(terminal, "cancelled");
    }
    assert_eq!(f.state().0, terminal);
    assert!(registry.lock().unwrap().is_empty());
    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn failed_completion_write_persists_failure_instead_of_emitting_success() {
    let mut f = Fixture::new();
    f.conn.execute_batch("CREATE TRIGGER fixture_no_complete BEFORE UPDATE OF status ON runs WHEN NEW.status='completed' BEGIN SELECT RAISE(ABORT,'fixture_completion_write_failure'); END;").unwrap();
    let event = finish_with_fallback(&mut f.conn, 9001, Ok(()), false).unwrap();
    assert_eq!(event.status, "failed");
    assert!(event.message.contains("fixture_completion_write_failure"));
    let state = f.state();
    assert_eq!(state.0, "failed");
    assert!(state.1.contains("fixture_completion_write_failure"));
    assert!(!state.2.is_empty());
    assert_eq!(state.3, 92.0);
}

#[test]
fn unavailable_terminal_storage_returns_error_and_retains_recoverable_run() {
    let mut f = Fixture::new();
    f.conn.execute_batch("CREATE TRIGGER fixture_no_terminal BEFORE UPDATE OF status ON runs BEGIN SELECT RAISE(ABORT,'fixture_all_terminal_writes_fail'); END;").unwrap();
    let before = f.state();
    let error = finish_with_fallback(
        &mut f.conn,
        9001,
        Err("original collection failure".into()),
        false,
    )
    .unwrap_err();
    assert!(error.contains("original collection failure"));
    assert!(error.contains("fixture_all_terminal_writes_fail"));
    assert_eq!(f.state(), before);
}

#[test]
fn worker_panic_is_persisted_as_failure_and_never_overwrites_an_existing_terminal() {
    let mut f = Fixture::new();
    let result = tray_activity::catch_job_panic(|| panic!("synthetic worker unwind"));
    let event = finish_with_fallback(&mut f.conn, 9001, result, false).unwrap();
    assert_eq!(event.status, "failed");
    assert!(f.state().1.contains("线程异常退出"));
    let before = f.state();
    assert!(finish_with_fallback(&mut f.conn, 9001, Ok(()), false).is_err());
    assert_eq!(f.state(), before);
}

#[test]
fn silently_ignored_completion_write_falls_back_to_durable_failure() {
    let mut f = Fixture::new();
    f.conn.execute_batch("CREATE TRIGGER fixture_ignore_complete BEFORE UPDATE OF status ON runs WHEN NEW.status='completed' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let event = finish_with_fallback(&mut f.conn, 9001, Ok(()), false).unwrap();
    assert_eq!(event.status, "failed");
    assert!(f.state().1.contains("终态更新未生效"));
    assert_eq!(f.state().0, "failed");
}

#[test]
fn silently_ignored_terminal_writes_return_error_and_keep_recoverable_run() {
    let mut f = Fixture::new();
    f.conn.execute_batch("CREATE TRIGGER fixture_ignore_terminal BEFORE UPDATE OF status ON runs BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let before = f.state();
    let error = finish_with_fallback(&mut f.conn, 9001, Ok(()), false).unwrap_err();
    assert!(error.contains("终态更新未生效"));
    assert_eq!(f.state(), before);
}

#[test]
fn terminal_readback_rejects_after_trigger_rewrite_before_success_publication() {
    let mut f = Fixture::new();
    f.conn.execute_batch("CREATE TRIGGER fixture_rewrite_complete AFTER UPDATE OF status ON runs WHEN NEW.status='completed' BEGIN UPDATE runs SET status='running' WHERE id=NEW.id; END;").unwrap();
    let event = finish_with_fallback(&mut f.conn, 9001, Ok(()), false).unwrap();
    assert_eq!(event.status, "failed");
    assert_eq!(f.state().0, "failed");
    assert!(f.state().1.contains("终态校验失败"));
}
