// Two real initial SDK receipts fund the fixture. Timer tests make no later SDK
// or source business publication, and never alter immutable origin/ceilings.
type SourceTimerFixture = (
    PathBuf,
    rusqlite::Connection,
    WorkbenchStartRecord,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
);

fn source_timer_fixture(schema: i64) -> SourceTimerFixture {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{scheduler, source},
    };
    let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(std::sync::Arc::new(
        |_| {
            (200,"application/json",
        json!({"choices":[{"message":{"role":"assistant","content":"Original paid Source initial assessment"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
        },
    ));
    let env = source_specialist_test_environment(port);
    let (root, db, record) = source_dispatch_fixture_configure(&env, None, |db, record| {
        record.policy["webModeCeiling"] = json!("quick");
        db.execute("UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentQuickTimeout',30)",[]).unwrap();
    });
    analysis_view_run(&root, &record, |_, engine, _, _, scratch, _| {
        Ok(source_regression_outcome(engine, scratch))
    })
    .unwrap();
    let work = root.join("attempt-0001");
    let coordinator = prepare_native_source_coordinator_fresh_schema_for_test(
        &db,
        &record.scan_id,
        1,
        &work,
        schema,
    )
    .unwrap();
    let actor =
        native_source_fresh_finance::original_for_execution(&db, &coordinator.run_id).unwrap();
    source_reviewer_checkpoint_fixture(&root, &db, &record, &actor, "after_initial_assessments");
    assert_eq!(seen.lock().unwrap().len(), 2);
    crate::agent_runtime::multi_agent::directive::source_guidance::freeze(
        &db,
        &actor,
        AgentRole::RepoMapper,
        true,
    )
    .unwrap();
    let revision: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(revision),1) FROM agent_evidence_revisions WHERE root_run_id=?1",
            [&actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let slice = source::tool_task_slice(&db, &actor, AgentRole::RepoMapper, revision).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &actor,
        AgentRole::RepoMapper,
        AgentLane::ReadOnlyAnalysis,
        "source_tools_ready",
        &slice,
        revision,
        &source::tool_capabilities(AgentRole::RepoMapper).unwrap(),
        1000,
        3,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &actor, &child).unwrap();
    let wall:i64=db.query_row("SELECT hard_limit FROM agent_budget_limits WHERE root_run_id=?1 AND dimension='wall_time_ms'",[&actor.root_run_id],|r|r.get(0)).unwrap();
    assert_eq!(
        wall, 30_000,
        "timeout frozen before Root INSERT, no old-row rewrite"
    );
    (root, db, record, actor, child, seen, stop)
}
fn source_timer_authorizers(
    root: &Path,
    db: &rusqlite::Connection,
    record: &WorkbenchStartRecord,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> (Result<(), String>, Result<(), String>) {
    let path = root.join("oviraptor.sqlite3");
    let work = root.join("attempt-0001");
    let (model, runtime, _) =
        verify_source_runtime_contract(db, &record.scan_id, 1, &work).unwrap();
    let proxy = source_runtime_proxy(&runtime).unwrap();
    let mut context =
        crate::scanning::agent_tests::test_context(&path, &actor.target_key, Vec::new());
    context.scan_id = record.scan_id.clone();
    context.run = Some(AgentRunLedger {
        db_path: path.clone(),
        run_id: child.run_id.clone(),
    });
    let native = agent_native_source_tool_authority(db, &context, "repo.inventory")
        .map(|_| ())
        .map_err(str::to_string);
    let scoped = SpecialistTransportContext {
        supervision: None,
        db_path: &path,
        scan_id: &record.scan_id,
        attempt_number: 1,
        target_key: &actor.target_key,
        run_id: &actor.root_run_id,
        environment: &model,
        proxy,
        usage_dir: &work,
        deadline: None,
    };
    let tx = db.unchecked_transaction().unwrap();
    let phase = authorize_source_tool_phase(&tx, &scoped, actor, child);
    tx.rollback().unwrap();
    (native, phase)
}
#[test]
fn source_original_timer_both_live_authorizers_use_born_clock_not_mutable_started_at() {
    for schema in [3, 4] {
        let (root, db, record, actor, child, seen, stop) = source_timer_fixture(schema);
        db.execute(
            "UPDATE agent_runs SET started_at='2000-01-01 00:00:00' WHERE id=?1",
            [&actor.root_run_id],
        )
        .unwrap();
        assert_eq!(
            source_model_remaining_seconds(&db, &actor.root_run_id, 30).unwrap(),
            0,
            "old helper is deliberately inconsistent"
        );
        assert!(crate::agent_runtime::multi_agent::budget::clock::remaining(
            &db,
            &actor.root_run_id
        )
        .is_ok());
        let before = crate::scanning::web_mode_test_rows(&db);
        let (native, phase) = source_timer_authorizers(&root, &db, &record, &actor, &child);
        assert!(
            native.is_ok(),
            "real Native Source Broker used mutable started_at: {native:?}"
        );
        assert!(
            phase.is_ok(),
            "real phase admission used mutable started_at: {phase:?}"
        );
        crate::scanning::web_mode_assert_rows(&db, &before);
        assert_eq!(seen.lock().unwrap().len(), 2);
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn source_original_timer_natural_expiry_rejects_work_and_reentry_keeps_paid_original_rows() {
    let (root, db, record, actor, child, seen, stop) = source_timer_fixture(4);
    let cutoff = std::time::Instant::now() + Duration::from_secs(36);
    loop {
        match crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &actor.root_run_id) {
            Err(code) if code == "budget_wall_time_exhausted" => break,
            Ok(_) => {
                assert!(std::time::Instant::now() < cutoff);
                std::thread::sleep(Duration::from_millis(20));
            }
            other => panic!("natural original clock must expire without mutation: {other:?}"),
        }
    }
    let before = source_exit_snapshot(&db);
    let (native, phase) = source_timer_authorizers(&root, &db, &record, &actor, &child);
    assert!(native.is_err() && phase.is_err());
    assert!(run_native_source_assessments(
        &root.join("oviraptor.sqlite3"),
        &record.scan_id,
        1,
        &root.join("attempt-0001")
    )
    .is_err());
    source_exit_assert_snapshot(&db, &actor, &before);
    source_exit_require_fact(&db, &actor, true);
    assert_eq!(seen.lock().unwrap().len(), 2);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &actor.root_run_id,
            None,
            "model_requests"
        )
        .unwrap()
        .consumed,
        2
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_specialist_calls WHERE root_run_id=?1 AND state='received'",
            [&actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    let owner = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        &db,
        &actor.root_run_id,
    )
    .unwrap();
    owner.require_original_coordinator(&db, &actor).unwrap();
    assert!(owner.require_live(&db).is_err());
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
