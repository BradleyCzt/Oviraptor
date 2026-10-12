// Regression for the parallel-only failure of
// `environment_preparation_and_scan_activation_are_atomic_across_connections`:
// a completed preparation's released flock was still referenced by children
// that other tests were spawning, so the next begin was refused.
#[cfg(unix)]
fn with_concurrent_spawners<T>(body: impl FnOnce() -> T) -> T {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let stop = Arc::new(AtomicBool::new(false));
    let spawners: Vec<_> = (0..4)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let _ = std::process::Command::new("/usr/bin/true").status();
                }
            })
        })
        .collect();
    let result = body();
    stop.store(true, Ordering::Relaxed);
    for spawner in spawners {
        spawner.join().unwrap();
    }
    result
}

#[cfg(unix)]
#[test]
fn completed_environment_preparation_can_begin_again_while_children_are_spawned() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-preparation-spawn-{}",
        uuid::Uuid::new_v4()
    ));
    let db_path = initialize(&root).unwrap();
    let refused = with_concurrent_spawners(|| {
        let mut refused = Vec::new();
        for _ in 0..150 {
            match begin_environment_preparation(&db_path) {
                Ok(preparation) => preparation.complete().unwrap(),
                Err(error) => refused.push(error),
            }
        }
        refused
    });
    assert!(
        refused.is_empty(),
        "a released preparation lock was refused {} times: {:?}",
        refused.len(),
        refused.first()
    );
    assert_eq!(
        environment_preparation_status(&db_path).unwrap().state,
        "idle"
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_live_environment_preparation_is_still_refused_after_the_spawn_settle_grace() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-preparation-held-{}",
        uuid::Uuid::new_v4()
    ));
    let db_path = initialize(&root).unwrap();
    let live = begin_environment_preparation(&db_path).unwrap();
    let started = Instant::now();
    let error = begin_environment_preparation(&db_path).err().unwrap();
    let waited = started.elapsed();
    assert!(error.contains("已有环境准备正在进行"), "{error}");
    assert!(
        waited >= PREPARATION_LOCK_SPAWN_SETTLE,
        "refused before the grace: {waited:?}"
    );
    assert!(
        waited < Duration::from_secs(2),
        "refusal must stay bounded: {waited:?}"
    );
    assert_eq!(
        environment_preparation_status(&db_path).unwrap().state,
        "installing"
    );
    assert!(
        recover_environment_preparation(&db_path, &live.owner, "已确认安装子进程停止").is_err()
    );
    live.complete().unwrap();
    assert_eq!(
        environment_preparation_status(&db_path).unwrap().state,
        "idle"
    );
    fs::remove_dir_all(root).unwrap();
}
