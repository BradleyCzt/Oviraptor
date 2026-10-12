// Shared production terminal entry while an original ordinary SDK is in flight.
#[test]
fn specialist_inflight_original_mapper_blocks_root_terminal_until_actual_sdk_exit() {
    use std::sync::{Arc, Mutex, mpsc};
    let (root, mut context, actor, child) = specialist_financial_fixture();
    let (arrive_tx, arrive) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let release = Mutex::new(release);
    let (port, seen, _stop) = spawn_endpoint(Arc::new(move |_| {
        let _ = arrive_tx.send(());
        let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(8));
        (200, "application/json", proposal_model_response("original Mapper response"))
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let db = db::open(&context.db_path).unwrap();
    let original_status: String = db.query_row("SELECT status FROM agent_runs WHERE id=?1", [&actor.root_run_id], |r|r.get(0)).unwrap();
    let outcome = AgentTargetOutcome::incomplete("ordinary original SDK must exit before terminal publication");
    let (closing, unchanged, returned, status) = std::thread::scope(|scope| {
        let (done_tx, done) = mpsc::channel();
        let (c, a, child_ref) = (&context, &actor, &child);
        scope.spawn(move || { let _ = done_tx.send(multi_agent_child_round_transport(c, a, child_ref, "readonly", json!({}))); });
        arrive.recv_timeout(Duration::from_secs(5)).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let closing = finish_coordinator_run(&db, &actor, &outcome);
        let unchanged = super::tests::application_table_snapshot(&db) == before;
        let status: String = db.query_row("SELECT status FROM agent_runs WHERE id=?1", [&actor.root_run_id], |r|r.get(0)).unwrap();
        db.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1", [&child.run_id]).unwrap();
        let early = done.recv_timeout(Duration::from_secs(3));
        release_tx.send(()).unwrap();
        let returned = match early { Ok(result) => Some(result), Err(_) => { let _ = done.recv_timeout(Duration::from_secs(5)); None } };
        (closing, unchanged, returned, status)
    });
    assert!(closing.is_err(), "shared Root finisher published terminal before the ordinary original SDK returned");
    assert!(unchanged, "busy SDK must roll back all application rows");
    assert_eq!(status, original_status);
    assert!(returned.is_some_and(|r| r.is_err()), "original cancellation must return before provider release");
    let fee = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid", [&child.assignment_id]).unwrap();
    finish_coordinator_run(&db, &actor, &outcome).unwrap();
    assert!(crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(&db,
        "SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 ORDER BY rowid", [&child.assignment_id]).unwrap() == fee);
    let replay = super::tests::application_table_snapshot(&db);
    finish_coordinator_run(&db, &actor, &outcome).unwrap();
    assert!(super::tests::application_table_snapshot(&db)==replay);
    let deadline = std::time::Instant::now()+Duration::from_secs(1);
    while seen.lock().unwrap().is_empty() && std::time::Instant::now()<deadline { std::thread::yield_now(); }
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(db); drop(context); fs::remove_dir_all(root).unwrap();
}

type SpecialistLifetimeFixture = (PathBuf, AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild, Seen);
fn specialist_inflight_finished_original() -> SpecialistLifetimeFixture {
    let (root, mut context, actor, child) = specialist_financial_fixture();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| (503, "application/json", r#"{"error":{"message":"fixture unavailable"}}"#.into())));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    assert!(multi_agent_child_round_transport(&context, &actor, &child, "readonly", json!({})).is_err());
    (root, context, actor, child, seen)
}
fn specialist_inflight_original_path(context: &AgentRunContext, actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease, child: &str) -> PathBuf {
    use sha2::Digest;
    let key = serde_json::to_vec(&(&actor.scan_id, actor.attempt_number, "specialist-sdk", child)).unwrap();
    let db = fs::canonicalize(&context.db_path).unwrap();
    let mut name = db.file_name().unwrap().to_os_string(); name.push(".invocations");
    db.with_file_name(name).join(format!("{:x}.lock", sha2::Sha256::digest(key)))
}
#[test]
fn specialist_inflight_missing_original_or_foreign_lock_cannot_close_or_change_unknown_bill() {
    for foreign in [false, true] {
        let (root, context, actor, child, seen) = specialist_inflight_finished_original();
        let path = specialist_inflight_original_path(&context, &actor, &child.run_id);
        assert!(path.is_file());
        fs::remove_file(&path).unwrap(); // Temporary corruption after actual SDK exit.
        if foreign {
            drop(crate::agent_runtime::execution_owner::claim_native_invocation(&context.db_path,
                &actor.scan_id, actor.attempt_number, "specialist-sdk", "foreign-child").unwrap());
        }
        let db = db::open(&context.db_path).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        let error = finish_coordinator_run(&db, &actor, &AgentTargetOutcome::incomplete("lost original physical proof")).unwrap_err();
        assert!(error.contains("specialist_transport_original_exit_proof_missing"), "{error}");
        assert!(super::tests::application_table_snapshot(&db)==before);
        assert!(!path.exists(), "must not create proof of an original SDK exit");
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(db); drop(context); fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn specialist_inflight_changed_original_call_scope_role_or_hash_keeps_every_prior_row() {
    for damage in [
        "UPDATE agent_specialist_calls SET request_hash='bad-original-hash'",
        "UPDATE agent_specialist_calls SET role='evidence_reviewer'",
        "UPDATE agent_specialist_calls SET lease_epoch=lease_epoch+1",
    ] {
        let (root, context, actor, _child, seen) = specialist_inflight_finished_original();
        let db = db::open(&context.db_path).unwrap();
        db.execute_batch("DROP TRIGGER agent_specialist_call_immutable").unwrap();
        db.execute_batch(damage).unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(finish_coordinator_run(&db, &actor, &AgentTargetOutcome::incomplete("damaged original call")).is_err(), "{damage}");
        assert!(super::tests::application_table_snapshot(&db)==before, "{damage}");
        assert_eq!(seen.lock().unwrap().len(), 1);
        drop(db); drop(context); fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn specialist_inflight_later_probe_failure_releases_prior_idle_lock_without_refund_or_dispatch() {
    use crate::agent_runtime::{contract::{AgentLane,AgentRole},multi_agent::{scheduler,lease::CoordinatorLease}};
    let (root, mut context, actor, first) = specialist_financial_fixture();
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst)==0 {
            (200, "application/json", proposal_model_response("first original paid response"))
        } else { (503, "application/json", r#"{"error":{"message":"fixture unavailable"}}"#.into()) }
    }));
    context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    let (_, usage) = multi_agent_child_round_transport(&context, &actor, &first, "readonly", json!({})).unwrap();
    let db = db::open(&context.db_path).unwrap();
    settle_child_usage(&db, &actor, &first, &usage).unwrap();
    scheduler::finish_child(&db, &actor, &first, true, "original paid assessment completed").unwrap();
    let second = scheduler::schedule_child(&db, &actor, AgentRole::DeepInvestigator, AgentLane::ReadOnlyAnalysis,
        "original-contradiction", &json!({"contradiction":"frozen original evidence"}), 2,
        &["evidence.read".into(),"mailbox.write".into()], 8000, 1).unwrap();
    scheduler::start_child_or_release(&db, &actor, &second).unwrap();
    assert!(multi_agent_child_round_transport(&context, &actor, &second, "readonly", json!({})).is_err());
    fs::remove_file(specialist_inflight_original_path(&context, &actor, &second.run_id)).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert!(finish_coordinator_run(&db, &actor, &AgentTargetOutcome::incomplete("second original proof missing")).is_err());
    assert!(super::tests::application_table_snapshot(&db)==before);
    let c: &CoordinatorLease = &actor;
    let probe = crate::agent_runtime::execution_owner::probe_native_invocation(&context.db_path,
        &c.scan_id,c.attempt_number,"specialist-sdk",&first.run_id).unwrap();
    assert!(probe.is_some(), "failure must release its earlier idle probes");
    drop(probe);
    assert_eq!(seen.lock().unwrap().len(), 2);
    drop(db); drop(context); fs::remove_dir_all(root).unwrap();
}

// The Web cancellation positive case needs its own current signed Root/worker
// and live parent. It does not upgrade the shared historical HTTP fixtures.
fn specialist_inflight_original_executor_context() -> (PathBuf, AgentRunContext,
    crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor) {
    specialist_inflight_original_executor_context_with_tokens(8000)
}
fn specialist_inflight_original_executor_context_with_tokens(tokens:i64) -> (PathBuf, AgentRunContext,
    crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor) {
    specialist_inflight_original_executor_context_at(tokens,"http://127.0.0.1:9")
}
fn specialist_inflight_original_executor_context_at(tokens:i64,target:&str)->(PathBuf,AgentRunContext,
    crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor) {
    use crate::agent_runtime::{contract::{AgentRole,AgentLane},multi_agent::scheduler};
    let (root, path, run, actor) = multi_agent_new_task_root_for_target("original-web-cancel", 60000, 20, target);
    let db = db::open(&path).unwrap();
    let child = scheduler::schedule_child(&db,&actor,AgentRole::WebExecutor,AgentLane::TargetTouching,
        "original-cancellation-contract", &json!({}), 1, &["replay_http".into()],tokens,1).unwrap();
    scheduler::start_child_or_release(&db,&actor,&child).unwrap();
    let work: String = db.query_row("SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=1",
        [&actor.scan_id],|r|r.get(0)).unwrap();
    let mut context = test_context(&path,target,vec![AgentIdentity::anonymous()]);
    context.scan_id = actor.scan_id.clone();
    context.execution_plan = web_mode_positive_plan_or(&path,target,context.execution_plan);
    context.target_dir = PathBuf::from(work).join("url-pipeline/target-00001");
    context.log_path = root.join("runner.log");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(context.target_dir.join(".oviraptor-scan-id"),&actor.scan_id).unwrap();
    fs::write(context.target_dir.join("frontend-evidence.json"),context.evidence.to_string()).unwrap();
    context.run = Some(AgentRunLedger {db_path:path.clone(),run_id:run});
    bind_agent_evidence_location(&context).unwrap();
    context.run = Some(AgentRunLedger {db_path:path.clone(),run_id:child.run_id});
    let parent = start_original_fixture_supervisor(&path,&actor).unwrap();
    context.supervision = Some(parent.ticket());
    (root,context,parent)
}
