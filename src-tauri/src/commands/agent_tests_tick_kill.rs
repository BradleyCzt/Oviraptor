// Actual process death after paid commit, with original C and zero SDK replay.
#[test]
fn coordinator_tick_paid_process_probe() {
    let Some(path) = std::env::var_os("OVIRAPTOR_ROOT_TICK_INPUT") else {
        return;
    };
    let (mut context, actor) = root_tick_reload_probe(Path::new(&path));
    let parent = start_original_fixture_supervisor(
        &context.db_path,
        &actor,
    )
    .unwrap();
    context.supervision = Some(parent.ticket());
    let _ = native_coordinator_tick(&context, &actor);
    panic!("test parent must kill the paid process before this returns");
}

#[cfg(unix)]
#[test]
fn coordinator_tick_actual_sigkill_recovers_original_decision_without_new_sdk() {
    use std::{os::unix::process::ExitStatusExt, process::Stdio};
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
        (
            200,
            "application/json",
            proposal_model_response(&root_tick_valid_text("paid before SIGKILL")),
        )
    }));
    let mut f = root_tick_fixture("tick-paid-kill", &format!("http://127.0.0.1:{port}/v1"));
    let input = f.f.root.join("original-tick-input.json");
    let ready = f.f.root.join("paid-tick.ready");
    root_tick_write_probe(&f, &input);
    drop(f.parent.take());
    let mut process = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "scanning::agent_tests::coordinator_tick_paid_process_probe",
            "--nocapture",
        ])
        .env("OVIRAPTOR_ROOT_TICK_INPUT", &input)
        .env("OVIRAPTOR_ROOT_TICK_PAID_READY_FILE", &ready)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = process.stdin.take().unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let paid = loop {
        if ready.is_file() {
            break true;
        }
        if Instant::now() >= deadline || process.try_wait().unwrap().is_some() {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let killed = process.kill();
    let exit = process.wait().unwrap();
    drop(stdin);
    let mut child_error = String::new();
    if !paid {
        if let Some(stderr) = process.stderr.take() {
            let mut bounded = std::io::Read::take(stderr, 8192);
            std::io::Read::read_to_string(&mut bounded, &mut child_error).unwrap();
        }
    }
    assert!(
        paid,
        "actual production paid SDK checkpoint was not reached: {exit}; {child_error}"
    );
    assert!(killed.is_ok());
    assert_eq!(exit.signal(), Some(libc::SIGKILL));
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_model_journal WHERE phase='received'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let (mut context, original) = root_tick_reload_probe(&input);
    assert_eq!(original.lease_epoch, f.actor.lease_epoch);
    assert_eq!(original.fencing_token, f.actor.fencing_token);
    let parent = start_original_fixture_supervisor(
        &context.db_path,
        &original,
    )
    .unwrap();
    context.supervision = Some(parent.ticket());
    let restored = native_coordinator_tick(&context, &original).unwrap_or_else(|e| {
        panic!("actual killed paid tick must recover same semantic result: {e}")
    });
    assert!(restored.replayed);
    assert!(restored.summary.as_json()["observed"]
        .to_string()
        .contains("paid before SIGKILL"));
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(root_tick_count(&db, &original.root_run_id, "request"), 1);
    assert_eq!(root_tick_count(&db, &original.root_run_id, "decision"), 1);
    assert_eq!(
        root_tick_count(&db, &original.root_run_id, "publication"),
        1
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &original.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        1
    );
    parent.check().unwrap();
    drop(parent);
}
