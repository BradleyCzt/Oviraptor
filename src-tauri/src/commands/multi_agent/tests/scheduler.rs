#[test]
fn failed_child_start_rolls_back_assignment_and_releases_its_lease() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, db_path, root_run_id, lease) =
        multi_agent_test_root("child-start-rollback", 60_000, 20);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "start-rollback-test",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        8_000,
        1,
    ).unwrap();
    connection.execute_batch(
        "CREATE TRIGGER reject_mapper_run_start BEFORE UPDATE OF status ON agent_runs \
         WHEN NEW.id LIKE 'run-asg-%' AND NEW.status='running' \
         BEGIN SELECT RAISE(ABORT,'run start unavailable'); END;",
    ).unwrap();
    let error = scheduler::start_child_or_release(&connection, &lease, &child).unwrap_err();
    assert!(error.contains("run start unavailable"), "{error}");
    let state: (String, String, String) = connection.query_row(
        "SELECT a.state,r.status,r.terminal_state FROM agent_assignments a \
         JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1",
        [&child.assignment_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(state, ("failed".into(), "terminal".into(), "failed".into()));
    let lanes: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1",
        [&child.assignment_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(lanes, 0);
    let budget: (i64, i64) = connection.query_row(
        "SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(budget, (0, 0));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn paused_attempt_cannot_schedule_new_child_even_with_live_coordinator_lease() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    let (root, db_path, root_run_id, lease) =
        multi_agent_test_root("paused-scheduling", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&lease.scan_id]).unwrap();
    let error = scheduler::schedule_child(
        &connection, &lease, AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis,
        "paused", &serde_json::json!({}), 1, &["evidence.read".into()], 100, 1,
    ).unwrap_err();
    assert_eq!(error, "agent_attempt_not_active");
    let assignments: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(assignments, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn paused_attempt_cannot_start_prepared_child_and_releases_its_resources() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    let (root, db_path, root_run_id, lease) =
        multi_agent_test_root("paused-child-start", 20_000, 10);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis,
        "before-pause", &serde_json::json!({}), 1, &["evidence.read".into()], 100, 1,
    ).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&lease.scan_id]).unwrap();
    let error = scheduler::start_child_or_release(&connection, &lease, &child).unwrap_err();
    assert_eq!(error, "agent_attempt_not_active");
    let state: (String, String) = connection.query_row(
        "SELECT a.state,r.terminal_state FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1",
        [&child.assignment_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(state, ("failed".into(), "failed".into()));
    let lanes: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1",
        [&child.assignment_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(lanes, 0);
    let budget: (i64, i64) = connection.query_row(
        "SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(budget, (0, 0));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn replaced_or_deleted_attempt_cannot_schedule_child() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    for reason in ["replaced", "deleted"] {
        let (root, db_path, root_run_id, lease) =
            multi_agent_test_root(reason, 20_000, 10);
        let connection = db::open(&db_path).unwrap();
        if reason == "replaced" {
            connection.execute(
                "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1", [&lease.scan_id],
            ).unwrap();
        } else {
            connection.execute(
                "INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)", [&lease.scan_id],
            ).unwrap();
        }
        let error = scheduler::schedule_child(
            &connection, &lease, AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis,
            reason, &serde_json::json!({}), 1, &["evidence.read".into()], 100, 1,
        ).unwrap_err();
        assert_eq!(error, "agent_attempt_not_active", "{reason}");
        let assignments: i64 = connection.query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1",
            [&root_run_id], |row| row.get(0),
        ).unwrap();
        assert_eq!(assignments, 0, "{reason}");
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn executor_handoff_failure_releases_target_lane_and_reservation() {
    let _real=RealSpecialistTransport::enter();
    let (mut h,seen)=web_closure_prepare_harness("executor-handoff-failure");
    let root_run_id=h.context.run.as_ref().unwrap().run_id.clone();
    let scan=h.context.scan_id.clone();
    let connection=db::open(&h.db_path).unwrap();
    let context=&mut h.context;
    connection.execute_batch(
        "CREATE TRIGGER reject_executor_handoff BEFORE INSERT ON agent_messages \
         WHEN NEW.kind='execution_assignment' BEGIN SELECT RAISE(ABORT,'handoff unavailable'); END;",
    ).unwrap();
    let error = match multi_agent_prepare(context) {
        Ok(_) => panic!("an unacknowledged executor assignment must not start"),
        Err(error) => error,
    };
    assert!(error.contains("handoff unavailable"), "{error}");
    let assignment: (String, i64, i64) = connection.query_row(
        "SELECT state,reserved_tokens,reserved_requests FROM agent_assignments \
         WHERE coordinator_run_id=?1 AND role='web_executor'",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(assignment, ("failed".into(), 0, 0));
    let lanes: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_lane_leases WHERE scan_id=?1 AND lane='target_touching'",
        [&scan], |row| row.get(0),
    ).unwrap();
    assert_eq!(lanes, 0);
    let budget: (i64, i64) = connection.query_row(
        "SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(budget, (0, 0));
    assert_eq!(context.run.as_ref().map(|run| run.run_id.as_str()), Some(root_run_id.as_str()));
    drop(connection);
    assert_eq!(seen.lock().unwrap().len(),3,"actual Root, Mapper, changed-fact Root reached the handoff boundary");
    let directory=h.root.clone();drop(h);fs::remove_dir_all(directory).unwrap();
}

#[test]
fn executor_mailbox_failure_releases_target_lane_and_reservation() {
    let _real=RealSpecialistTransport::enter();
    let (port,seen,_stop)=specialist_original_endpoint(std::sync::Arc::new(|wire| {
        if specialist_original_is_web_sdk(&wire) {
            (200,"application/json",proposal_model_response("actual bounded Web exit before mailbox fault"))
        } else { multi_agent_original_fake_response(wire) }
    }));
    let mut f=root_tick_fixture_from_web_mode_turns("executor-mailbox-failure",
        &format!("http://127.0.0.1:{port}/v1"),true,(60_000,20),None,
        web_mode_fixture("multi","https://authorized.example.test"),Some(1));
    drop(f.parent.take());
    let root_run_id=f.actor.root_run_id.clone();
    let connection=db::open(&f.context.db_path).unwrap();
    let context=&mut f.context;
    let mut session=multi_agent_prepare(context).unwrap();
    assert_eq!(seen.lock().unwrap().len(),3,"actual Root, Mapper, changed-fact Root reached the mailbox boundary");
    let outcome=run_native_agent(context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    assert_eq!(seen.lock().unwrap().iter().filter(|wire|specialist_original_is_web_sdk(wire)).count(),1);
    connection.execute_batch(
        "CREATE TRIGGER reject_executor_result BEFORE INSERT ON agent_messages \
         WHEN NEW.kind='execution_result' BEGIN SELECT RAISE(ABORT,'mailbox unavailable'); END;"
    ).unwrap();
    let error = multi_agent_finish_execution(
        context, &mut session, &outcome,
    ).unwrap_err();
    assert!(error.contains("mailbox unavailable"), "{error}");
    let state: String = connection.query_row(
        "SELECT state FROM agent_assignments WHERE id=?1", [&session.executor.assignment_id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(state, "failed");
    let lanes: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_lane_leases WHERE assignment_id=?1", [&session.executor.assignment_id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(lanes, 0);
    let reserved: i64 = connection.query_row(
        "SELECT reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1", [&root_run_id],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(reserved, 0);
    drop(connection);
    drop(session);
}
