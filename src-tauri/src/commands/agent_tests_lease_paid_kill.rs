// Actual process exit releases original invocation inodes, not original bills.
#[test]
fn lease_paid_process_probe() {
    let Some(input) = std::env::var_os("OVIRAPTOR_LEASE_PAID_INPUT") else {return;};
    let (mut context,actor) = root_tick_reload_probe(Path::new(&input));
    let run = std::env::var("OVIRAPTOR_LEASE_PAID_CHILD").unwrap();
    let db = db::open(&context.db_path).unwrap();
    let assignment: String = db.query_row("SELECT assignment_id FROM agent_runs WHERE id=?1 AND root_run_id=?2 AND role='spa_api_mapper'",params![run,actor.root_run_id],|r|r.get(0)).unwrap();
    let child = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {assignment_id:assignment,run_id:run,role:crate::agent_runtime::contract::AgentRole::SpaApiMapper};
    let parent = start_original_fixture_supervisor(&context.db_path,&actor).unwrap();
    context.supervision = Some(parent.ticket());
    let _ = native_coordinator_lease_expiry_feedback(&context,&actor,&child);
    panic!("parent must kill actual paid process before publication");
}

#[cfg(unix)]
#[test]
fn lease_paid_actual_sigkill_reopens_original_source_and_publishes_without_resend() {
    lease_paid_actual_lifecycle("process_kill");
}

#[cfg(unix)]
fn lease_paid_kill_and_recover(f: &mut RootTickFixture, child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild, seen: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use std::{os::unix::process::ExitStatusExt,process::Stdio};
    let input = f.f.root.join("original-lease-input.json");
    let ready = f.f.root.join("lease-paid.ready");
    root_tick_write_probe(f,&input);
    drop(f.parent.take());
    let mut process = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","commands::agent_tests::lease_paid_process_probe","--nocapture"])
        .env("OVIRAPTOR_LEASE_PAID_INPUT",&input)
        .env("OVIRAPTOR_LEASE_PAID_CHILD",&child.run_id)
        .env("OVIRAPTOR_ROOT_TICK_PAID_READY_FILE",&ready)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let stdin = process.stdin.take().unwrap();
    let deadline = Instant::now()+Duration::from_secs(20);
    let reached = loop {
        if ready.is_file() {break true;}
        if Instant::now()>=deadline || process.try_wait().unwrap().is_some() {break false;}
        std::thread::sleep(Duration::from_millis(20));
    };
    let killed = process.kill();
    let exit = process.wait().unwrap();
    drop(stdin);
    assert!(reached,"actual lease paid checkpoint was not reached");
    assert!(killed.is_ok());
    assert_eq!(exit.signal(),Some(libc::SIGKILL));
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(seen.lock().unwrap().len(),3);
    assert_eq!(root_tick_count(&db,&f.actor.root_run_id,"publication"),1);
    let fees = client_delivery_rows(&db,"agent_budget_entries");
    let (mut context,original) = root_tick_reload_probe(&input);
    let parent = start_original_fixture_supervisor(&context.db_path,&original).unwrap();
    context.supervision = Some(parent.ticket());
    let recovered = native_coordinator_lease_expiry_feedback(&context,&original,child).unwrap();
    assert!(recovered.replayed);
    assert_eq!(seen.lock().unwrap().len(),3);
    assert_eq!(root_tick_count(&db,&original.root_run_id,"publication"),2);
    assert_eq!(client_delivery_rows(&db,"agent_budget_entries"),fees);
    let published = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_lease_expiry_feedback(&context,&original,child).unwrap();
    assert_eq!(replay.event_sequence,recovered.event_sequence);
    assert_eq!(seen.lock().unwrap().len(),3);
    assert_eq!(super::tests::application_table_snapshot(&db),published);
    parent.check().unwrap();
}
