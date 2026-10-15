use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
};

fn animator_owners() -> &'static Mutex<HashMap<usize, Weak<()>>> {
    static OWNERS: OnceLock<Mutex<HashMap<usize, Weak<()>>>> = OnceLock::new();
    OWNERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// There is at most one animator for an AppState's activity counter, including
/// a previous animator that has not woken from its frame delay yet.
pub(super) struct AnimatorLease {
    active_jobs: Arc<AtomicUsize>,
    token: Arc<()>,
}

impl AnimatorLease {
    pub(super) fn acquire(active_jobs: Arc<AtomicUsize>) -> Option<Self> {
        let mut owners = animator_owners()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = Arc::as_ptr(&active_jobs) as usize;
        if owners.get(&key).and_then(Weak::upgrade).is_some() {
            return None;
        }
        let token = Arc::new(());
        owners.insert(key, Arc::downgrade(&token));
        Some(Self { active_jobs, token })
    }

    pub(super) fn should_run(&self) -> bool {
        let mut owners = animator_owners()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.owns(&owners) {
            return false;
        }
        if self.active_jobs.load(Ordering::SeqCst) > 0 {
            return true;
        }
        // The zero check and release share the acquire lock. A new job either
        // keeps this animator alive or acquires its own after this release.
        self.release(&mut owners);
        false
    }

    fn release(&self, owners: &mut HashMap<usize, Weak<()>>) {
        let key = Arc::as_ptr(&self.active_jobs) as usize;
        if self.owns(owners) {
            owners.remove(&key);
        }
    }

    fn owns(&self, owners: &HashMap<usize, Weak<()>>) -> bool {
        owners
            .get(&(Arc::as_ptr(&self.active_jobs) as usize))
            .is_some_and(|owner| owner.as_ptr() == Arc::as_ptr(&self.token))
    }
}

impl Drop for AnimatorLease {
    fn drop(&mut self) {
        let mut owners = animator_owners()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.release(&mut owners);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TrayActivity {
    Idle,
    Updating,
}

impl TrayActivity {
    /// Read when the UI callback runs, not when an animation frame is queued.
    pub(super) fn current(active_jobs: &AtomicUsize) -> Self {
        if active_jobs.load(Ordering::SeqCst) == 0 {
            Self::Idle
        } else {
            Self::Updating
        }
    }

    pub(super) fn title(self) -> &'static str {
        match self {
            // tray-icon's macOS implementation leaves the old title untouched
            // for None. An explicit empty string clears the NSStatusItem title.
            Self::Idle => "",
            Self::Updating => "资产更新中",
        }
    }

    pub(super) fn tooltip(self) -> &'static str {
        match self {
            Self::Idle => "Oviraptor · 后台运行",
            Self::Updating => "Oviraptor · 正在更新资产",
        }
    }
}

/// The ownership of a running job, including a job whose thread cannot start.
/// Dropping it releases activity on success, failure, cancellation and unwind.
pub(super) struct JobActivityGuard {
    active_jobs: Arc<AtomicUsize>,
    on_finished: Option<Box<dyn FnOnce(bool) + Send>>,
}

impl JobActivityGuard {
    pub(super) fn begin(
        active_jobs: Arc<AtomicUsize>,
        on_finished: impl FnOnce(bool) + Send + 'static,
    ) -> (Self, bool) {
        let first_job = active_jobs.fetch_add(1, Ordering::SeqCst) == 0;
        (
            Self {
                active_jobs,
                on_finished: Some(Box::new(on_finished)),
            },
            first_job,
        )
    }
}

impl Drop for JobActivityGuard {
    fn drop(&mut self) {
        let last_job = self.active_jobs.fetch_sub(1, Ordering::SeqCst) == 1;
        if let Some(on_finished) = self.on_finished.take() {
            on_finished(last_job);
        }
    }
}

pub(super) fn catch_job_panic(run: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run))
        .unwrap_or_else(|_| Err("资产任务线程异常退出，请查看操作日志后重试".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        sync::{mpsc, Barrier},
        thread,
    };

    fn begin_registered_job(
        count: Arc<AtomicUsize>,
        active_ids: Arc<Mutex<HashSet<i64>>>,
        id: i64,
        finishes: mpsc::Sender<(i64, bool)>,
    ) -> JobActivityGuard {
        active_ids.lock().unwrap().insert(id);
        JobActivityGuard::begin(count, move |last| {
            active_ids.lock().unwrap().remove(&id);
            finishes.send((id, last)).unwrap();
        })
        .0
    }

    #[test]
    fn tray_activity_idle_explicitly_clears_the_macos_title() {
        // Match the locked macOS provider's Option behavior: None is a no-op.
        fn set_native_title(title: &mut String, requested: Option<&str>) {
            if let Some(requested) = requested {
                *title = requested.into();
            }
        }
        let count = AtomicUsize::new(1);
        let mut displayed = String::new();
        set_native_title(&mut displayed, Some(TrayActivity::current(&count).title()));
        assert_eq!(displayed, "资产更新中");
        set_native_title(&mut displayed, None);
        assert_eq!(displayed, "资产更新中");

        count.store(0, Ordering::SeqCst);
        let idle = TrayActivity::current(&count);
        set_native_title(&mut displayed, Some(idle.title()));
        assert_eq!(displayed, "");
        assert_eq!(idle.tooltip(), "Oviraptor · 后台运行");
    }

    #[test]
    fn tray_activity_overlapping_workers_stay_busy_until_the_last_finishes() {
        let count = Arc::new(AtomicUsize::new(0));
        let ids = Arc::new(Mutex::new(HashSet::new()));
        let (finished_tx, finished_rx) = mpsc::channel();
        let guard_a = begin_registered_job(count.clone(), ids.clone(), 1, finished_tx.clone());
        let guard_b = begin_registered_job(count.clone(), ids.clone(), 2, finished_tx);
        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = barrier.clone();
        let worker = thread::spawn(move || {
            let _guard = guard_b;
            worker_barrier.wait();
        });

        drop(guard_a);
        assert_eq!(finished_rx.recv().unwrap(), (1, false));
        assert_eq!(TrayActivity::current(&count), TrayActivity::Updating);
        assert_eq!(*ids.lock().unwrap(), HashSet::from([2]));
        barrier.wait();
        worker.join().unwrap();
        assert_eq!(finished_rx.recv().unwrap(), (2, true));
        assert_eq!(TrayActivity::current(&count), TrayActivity::Idle);
        assert!(ids.lock().unwrap().is_empty());
    }

    #[test]
    fn tray_activity_queued_frames_use_the_state_when_applied() {
        let count = Arc::new(AtomicUsize::new(0));
        let (first, first_job) = JobActivityGuard::begin(count.clone(), |_| {});
        assert!(first_job);
        let queued_busy_count = count.clone();
        let queued_busy_frame = move || TrayActivity::current(&queued_busy_count);
        drop(first);
        assert_eq!(queued_busy_frame(), TrayActivity::Idle);

        let queued_idle_count = count.clone();
        let queued_idle_restore = move || TrayActivity::current(&queued_idle_count);
        let (second, first_job) = JobActivityGuard::begin(count.clone(), |_| {});
        assert!(first_job);
        assert_eq!(queued_idle_restore(), TrayActivity::Updating);
        drop(second);
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn tray_activity_fast_serial_jobs_reuse_the_sleeping_animator() {
        let count = Arc::new(AtomicUsize::new(0));
        let (first_job, _) = JobActivityGuard::begin(count.clone(), |_| {});
        let animator = AnimatorLease::acquire(count.clone()).unwrap();
        drop(first_job);
        // Each new job starts before the existing animation thread checks its
        // next frame. No additional animation thread should be started.
        for _ in 0..100 {
            let (job, _) = JobActivityGuard::begin(count.clone(), |_| {});
            assert!(AnimatorLease::acquire(count.clone()).is_none());
            assert!(animator.should_run());
            drop(job);
        }
        assert!(!animator.should_run());
        assert_eq!(TrayActivity::current(&count), TrayActivity::Idle);
    }

    #[test]
    fn tray_activity_retired_animator_cannot_release_its_replacement() {
        let count = Arc::new(AtomicUsize::new(0));
        let (first_job, _) = JobActivityGuard::begin(count.clone(), |_| {});
        let old_animator = AnimatorLease::acquire(count.clone()).unwrap();
        drop(first_job);
        assert!(!old_animator.should_run());

        let (new_job, _) = JobActivityGuard::begin(count.clone(), |_| {});
        let new_animator = AnimatorLease::acquire(count.clone()).unwrap();
        assert!(!old_animator.should_run());
        drop(old_animator);
        assert!(AnimatorLease::acquire(count.clone()).is_none());
        assert!(new_animator.should_run());
        drop(new_job);
        assert!(!new_animator.should_run());
    }

    #[test]
    fn tray_activity_concurrent_start_and_animator_retirement_keep_one_owner() {
        for _ in 0..100 {
            let count = Arc::new(AtomicUsize::new(0));
            let (job, _) = JobActivityGuard::begin(count.clone(), |_| {});
            let old_animator = AnimatorLease::acquire(count.clone()).unwrap();
            drop(job);
            let barrier = Arc::new(Barrier::new(2));
            let worker_barrier = barrier.clone();
            let worker_count = count.clone();
            let worker = thread::spawn(move || {
                worker_barrier.wait();
                let (new_job, _) = JobActivityGuard::begin(worker_count.clone(), |_| {});
                let animator = AnimatorLease::acquire(worker_count);
                (new_job, animator)
            });
            barrier.wait();
            let old_keeps_running = old_animator.should_run();
            let (new_job, new_animator) = worker.join().unwrap();
            assert_ne!(old_keeps_running, new_animator.is_some());
            assert!(AnimatorLease::acquire(count.clone()).is_none());
            drop(new_job);
            if let Some(new_animator) = new_animator {
                assert!(!new_animator.should_run());
            } else {
                assert!(!old_animator.should_run());
            }
        }
    }

    #[test]
    fn tray_activity_error_and_cancellation_release_job_ownership() {
        for result in [
            Ok(()),
            Err("request failed".into()),
            Err("__CANCELLED__".into()),
        ] {
            let count = Arc::new(AtomicUsize::new(0));
            let ids = Arc::new(Mutex::new(HashSet::new()));
            let (tx, rx) = mpsc::channel();
            {
                let _guard = begin_registered_job(count.clone(), ids.clone(), 9, tx);
                assert_eq!(TrayActivity::current(&count), TrayActivity::Updating);
                assert_eq!(catch_job_panic(|| result.clone()), result);
            }
            assert_eq!(rx.recv().unwrap(), (9, true));
            assert!(ids.lock().unwrap().is_empty());
            assert_eq!(TrayActivity::current(&count), TrayActivity::Idle);
        }
    }

    #[test]
    fn tray_activity_worker_panic_is_a_failure_and_releases_activity() {
        let count = Arc::new(AtomicUsize::new(0));
        let ids = Arc::new(Mutex::new(HashSet::new()));
        let (tx, rx) = mpsc::channel();
        let guard = begin_registered_job(count.clone(), ids.clone(), 11, tx);
        let worker = thread::spawn(move || {
            let _guard = guard;
            catch_job_panic(|| panic!("probe worker panicked"))
        });
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error, "资产任务线程异常退出，请查看操作日志后重试");
        assert_eq!(rx.recv().unwrap(), (11, true));
        assert!(ids.lock().unwrap().is_empty());
        assert_eq!(TrayActivity::current(&count), TrayActivity::Idle);
    }

    #[test]
    fn tray_activity_unwinding_or_an_unstarted_worker_releases_the_guard() {
        let count = Arc::new(AtomicUsize::new(0));
        let ids = Arc::new(Mutex::new(HashSet::new()));
        let (tx, rx) = mpsc::channel();
        let guard = begin_registered_job(count.clone(), ids.clone(), 13, tx);
        // An unstarted worker closure owns and drops the same guard.
        let unstarted_worker = move || {
            let _guard = guard;
        };
        drop(unstarted_worker);
        assert_eq!(rx.recv().unwrap(), (13, true));
        assert_eq!(TrayActivity::current(&count), TrayActivity::Idle);
        assert!(ids.lock().unwrap().is_empty());

        let (guard, _) = JobActivityGuard::begin(count.clone(), |_| {});
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                let _guard = guard;
                panic!("after executing the job");
            }))
            .is_err()
        );
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
