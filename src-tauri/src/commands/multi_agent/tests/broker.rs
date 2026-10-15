#[test]
fn broker_checks_role_capability_revocation_expiry_and_fencing() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    let (root, path, root_run_id, lease) = multi_agent_test_root("broker-auth", 100, 10);
    let connection = db::open(&path).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger { db_path: path.clone(), run_id: root_run_id });
    assert_eq!(agent_authorize_tool(&context, "replay_http"), Err("tool_capability_or_fencing_denied"));
    let mapper = scheduler::schedule_child(&connection, &lease, AgentRole::SpaApiMapper, AgentLane::ReadOnlyAnalysis,
        "mapper-auth", &serde_json::json!({}), 1, &["evidence.read".into()], 10, 1).unwrap();
    scheduler::mark_child_running(&connection, &lease, &mapper).unwrap();
    context.run.as_mut().unwrap().run_id = mapper.run_id.clone();
    assert_eq!(agent_authorize_tool(&context, "replay_http"), Err("tool_capability_or_fencing_denied"));
    scheduler::finish_child(&connection, &lease, &mapper, true, "done").unwrap();

    let child = scheduler::schedule_child(&connection, &lease, AgentRole::WebExecutor, AgentLane::TargetTouching,
        "executor-auth", &serde_json::json!({}), 1, &["inspect_evidence".into()], 10, 1).unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    context.run.as_mut().unwrap().run_id = child.run_id.clone();
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Ok(()));
    context.identities = vec![AgentIdentity::scoped("session-not-bound")];
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("tool_identity_binding_denied"));
    context.identities = vec![AgentIdentity::anonymous()];
    connection.execute(
        "INSERT INTO sentinel_scan_contexts(scan_id,policy_json) VALUES(?1,?2) \
         ON CONFLICT(scan_id) DO UPDATE SET policy_json=excluded.policy_json",
        rusqlite::params![lease.scan_id, r#"{"authSessionIds":["missing-session"]}"#],
    ).unwrap();
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("tool_identity_binding_denied"));
    connection.execute(
        "UPDATE sentinel_scan_contexts SET policy_json='{}' WHERE scan_id=?1",
        [&lease.scan_id],
    ).unwrap();
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Ok(()));
    assert_eq!(agent_authorize_tool(&context, "replay_http"), Err("tool_capability_or_fencing_denied"));
    let read_fences = |connection: &rusqlite::Connection| {
        let mut statement = connection.prepare(
            "SELECT 'coordinator',lease_epoch,fencing_token FROM agent_coordinator_leases WHERE root_run_id=?1 \
             UNION ALL SELECT 'assignment',lease_epoch,fencing_token FROM agent_assignments WHERE child_run_id=?2 \
             UNION ALL SELECT 'capability',lease_epoch,fencing_token FROM agent_capability_leases WHERE child_run_id=?2 \
             ORDER BY 1,2,3",
        ).unwrap();
        let rows = statement.query_map(rusqlite::params![lease.root_run_id,child.run_id], |row| {
            Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,row.get::<_,String>(2)?))
        }).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        rows
    };
    let original_fences = read_fences(&connection);
    assert_eq!(original_fences.len(), 3);
    // This positive case needs a live lease, not a race with the next rounded
    // second. The separate negative case below still uses a real past expiry.
    let transaction = rusqlite::Transaction::new_unchecked(
        &connection, rusqlite::TransactionBehavior::Immediate,
    ).unwrap();
    let near_expiry: String = transaction.query_row(
        "SELECT datetime('now','+120 seconds','localtime')", [], |row| row.get(0),
    ).unwrap();
    for table in ["agent_coordinator_leases", "agent_assignments", "agent_capability_leases"] {
        let key = if table == "agent_coordinator_leases" { "root_run_id" } else { "child_run_id" };
        let owner = if table == "agent_coordinator_leases" { &lease.root_run_id } else { &child.run_id };
        assert_eq!(transaction.execute(&format!("UPDATE {table} SET lease_expires_at=?1 WHERE {key}=?2"), rusqlite::params![near_expiry,owner]).unwrap(), 1);
    }
    transaction.commit().unwrap();
    let seconds_before: i64 = connection.query_row(
        "SELECT CAST(strftime('%s',lease_expires_at) AS INTEGER)-CAST(strftime('%s','now','localtime') AS INTEGER) FROM agent_capability_leases WHERE child_run_id=?1",
        [&child.run_id], |row| row.get(0),
    ).unwrap();
    assert!(seconds_before > 0 && seconds_before <= 120);
    scheduler::refresh_running_executor_leases(&connection, &child.run_id).unwrap();
    let seconds_remaining: i64 = connection.query_row(
        "SELECT CAST(strftime('%s',lease_expires_at) AS INTEGER)-CAST(strftime('%s','now') AS INTEGER) FROM agent_capability_leases WHERE child_run_id=?1",
        [&child.run_id], |row| row.get(0),
    ).unwrap();
    assert!(seconds_remaining > 300);
    let renewed_deadlines = {
        let mut statement = connection.prepare(
            "SELECT lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1 \
             UNION ALL SELECT lease_expires_at FROM agent_assignments WHERE child_run_id=?2 \
             UNION ALL SELECT lease_expires_at FROM agent_capability_leases WHERE child_run_id=?2 \
             UNION ALL SELECT lease_expires_at FROM agent_runs WHERE id=?2",
        ).unwrap();
        let rows = statement.query_map(rusqlite::params![lease.root_run_id,child.run_id], |row| row.get::<_,String>(0))
            .unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        rows
    };
    assert_eq!(renewed_deadlines.len(), 4);
    assert!(renewed_deadlines.iter().all(|deadline| deadline == &renewed_deadlines[0]));
    assert!(renewed_deadlines[0] > near_expiry);
    assert_eq!(read_fences(&connection), original_fences);
    let denied = agent_execute_tool(&context, &mut AgentToolRuntime::default(), "replay_http",
        &serde_json::json!({"identity":"anonymous","method":"GET","url":lease.target_key}));
    assert_eq!(denied.model_view["code"], "tool_capability_or_fencing_denied");
    let mut runtime = AgentToolRuntime::default();
    let allocated = runtime.begin_invocation(context.attempt_number);
    let retained = agent_execute_tool_at(&context, &mut runtime, "replay_http",
        &serde_json::json!({"identity":"anonymous","method":"GET","url":lease.target_key}), &allocated);
    assert_eq!(retained.invocation_id, allocated);
    assert_eq!(runtime.invocations, 1);
    connection.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE child_run_id=?1", [&child.run_id]).unwrap();
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("tool_capability_or_fencing_denied"));
    let lease_snapshot = |connection: &rusqlite::Connection| {
        [
            ("agent_coordinator_leases", "scan_id,attempt_number,target_key"),
            ("agent_assignments", "id"),
            ("agent_capability_leases", "id"),
            ("agent_runs", "id"),
        ].into_iter().map(|(table,order)| {
            let mut statement = connection.prepare(&format!("SELECT * FROM {table} ORDER BY {order}")).unwrap();
            let columns = statement.column_count();
            let rows = statement.query_map([], |row| {
                (0..columns).map(|column| row.get::<_,rusqlite::types::Value>(column)).collect::<rusqlite::Result<Vec<_>>>()
            }).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
            (table,rows)
        }).collect::<Vec<_>>()
    };
    let revoked_state = lease_snapshot(&connection);
    let revoked_changes = connection.total_changes();
    assert_eq!(scheduler::refresh_running_executor_leases(&connection, &child.run_id).unwrap_err(), "executor_lease_expired_or_fenced");
    assert_eq!(lease_snapshot(&connection), revoked_state);
    assert_eq!(connection.total_changes(), revoked_changes);
    connection.execute("UPDATE agent_capability_leases SET revoked_at='',lease_expires_at=datetime('now','-1 second','localtime') WHERE child_run_id=?1", [&child.run_id]).unwrap();
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("tool_capability_or_fencing_denied"));
    let expired_state = lease_snapshot(&connection);
    let expired_changes = connection.total_changes();
    assert_eq!(scheduler::refresh_running_executor_leases(&connection, &child.run_id).unwrap_err(), "executor_lease_expired_or_fenced");
    assert_eq!(lease_snapshot(&connection), expired_state);
    assert_eq!(connection.total_changes(), expired_changes);
    connection.execute("UPDATE agent_capability_leases SET lease_expires_at=?1 WHERE child_run_id=?2", rusqlite::params![lease.lease_expires_at, child.run_id]).unwrap();
    connection.execute("UPDATE agent_coordinator_leases SET fencing_token='superseded' WHERE root_run_id=?1", [&lease.root_run_id]).unwrap();
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("tool_capability_or_fencing_denied"));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn broker_refuses_a_changed_or_missing_frozen_web_surface() {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::scheduler};

    let (root, path, _, lease) = multi_agent_test_root("surface-fence", 100, 10);
    let connection = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::WebExecutor, AgentLane::TargetTouching,
        "surface-check", &serde_json::json!({}), 1,
        &["inspect_evidence".into(), "replay_http".into()], 10, 1,
    ).unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.run = Some(AgentRunLedger { db_path: path.clone(), run_id: child.run_id });
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Ok(()));

    connection.execute(
        "UPDATE agent_runs SET plan_json=json_set(plan_json,'$.executionSurface','host_linux') WHERE id=?1",
        [&lease.root_run_id],
    ).unwrap();
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("tool_execution_surface_denied"));
    let request = AgentHttpRequest {
        identity: AgentIdentity::anonymous(), method: "GET".into(), url: lease.target_key.clone(),
        extra_headers: Vec::new(), body: None, content_type: None,
        contract_key: String::new(), family: "authorization".into(),
        source: ScopeSource::IdentityComparison, tool: "replay_http".into(), timeout_seconds: 5,
    };
    let mut runtime = AgentToolRuntime::default();
    let rejected = agent_http_exchange(&context, &mut runtime, &request).unwrap_err();
    assert_eq!(rejected["code"], "tool_execution_surface_denied");
    assert_eq!(runtime.target_requests, 0, "even a direct send must spend no request");

    connection.execute(
        "UPDATE agent_runs SET plan_json=json_set(json_remove(plan_json,'$.executionSurface'),'$.executionSurface','web_only','$.mode','deep') WHERE id=?1",
        [&lease.root_run_id],
    ).unwrap();
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("tool_execution_surface_denied"));
    connection.execute(
        "UPDATE agent_runs SET plan_json='{}' WHERE id=?1",
        [&lease.root_run_id],
    ).unwrap();
    assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("tool_execution_plan_unavailable"));

    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn paused_attempt_does_not_renew_running_executor_capabilities() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    let (root, db_path, _, lease) = multi_agent_test_root("paused-executor-heartbeat", 100, 10);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection, &lease, AgentRole::WebExecutor, AgentLane::TargetTouching,
        "heartbeat", &serde_json::json!({}), 1, &["inspect_evidence".into()], 10, 1,
    ).unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    let before: String = connection.query_row(
        "SELECT lease_expires_at FROM agent_capability_leases WHERE child_run_id=?1",
        [&child.run_id], |row| row.get(0),
    ).unwrap();
    connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&lease.scan_id]).unwrap();
    assert_eq!(
        scheduler::refresh_running_executor_leases(&connection, &child.run_id).unwrap_err(),
        "agent_attempt_not_active",
    );
    let after: String = connection.query_row(
        "SELECT lease_expires_at FROM agent_capability_leases WHERE child_run_id=?1",
        [&child.run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(before, after);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn broker_refuses_stale_attempt_even_with_unexpired_capability() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    for stopped in ["paused", "replaced", "deleted"] {
        let (root, path, root_run_id, lease) = multi_agent_test_root(stopped, 100, 10);
        let connection = db::open(&path).unwrap();
        let child = scheduler::schedule_child(
            &connection, &lease, AgentRole::WebExecutor, AgentLane::TargetTouching,
            "stale-broker", &serde_json::json!({}), 1, &["inspect_evidence".into()], 10, 1,
        ).unwrap();
        scheduler::mark_child_running(&connection, &lease, &child).unwrap();
        let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
        context.scan_id = lease.scan_id.clone();
        context.run = Some(AgentRunLedger { db_path: path.clone(), run_id: child.run_id.clone() });
        assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Ok(()));

        match stopped {
            "paused" => { connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&lease.scan_id]).unwrap(); }
            "replaced" => { connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1", [&lease.scan_id]).unwrap(); }
            "deleted" => { connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)", [&lease.scan_id]).unwrap(); }
            _ => unreachable!(),
        }
        assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("agent_attempt_not_active"), "{stopped}");
        context.run.as_mut().unwrap().run_id = root_run_id.clone();
        connection.execute("UPDATE agent_runs SET orchestration_policy='single' WHERE id=?1", [&root_run_id]).unwrap();
        assert_eq!(agent_authorize_tool(&context, "inspect_evidence"), Err("agent_attempt_not_active"), "single {stopped}");
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn stale_attempt_cannot_promote_broker_artifact_to_review_fact() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    for stopped in ["paused", "replaced", "deleted"] {
        let (root, path, _, lease) = multi_agent_test_root(stopped, 100, 10);
        let connection = db::open(&path).unwrap();
        let child = scheduler::schedule_child(
            &connection, &lease, AgentRole::WebExecutor, AgentLane::TargetTouching,
            "stale-fact", &serde_json::json!({}), 1, &["replay_http".into()], 10, 1,
        ).unwrap();
        scheduler::mark_child_running(&connection, &lease, &child).unwrap();
        let mut context = test_context(&path, &lease.target_key, vec![AgentIdentity::anonymous()]);
        context.scan_id = lease.scan_id.clone();
        context.run = Some(AgentRunLedger { db_path: path.clone(), run_id: child.run_id.clone() });
        let view = serde_json::json!({
            "requestId":"req-0001", "status":200,
            "bodySha256":format!("{:x}", Sha256::digest(b"safe body")),
        });
        let artifact = agent_write_http_record(&context, 1,
            &serde_json::json!({"method":"GET","url":lease.target_key,"identity":"anonymous"}),
            &view, b"safe body",
        ).unwrap();
        match stopped {
            "paused" => { connection.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&lease.scan_id]).unwrap(); }
            "replaced" => { connection.execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1", [&lease.scan_id]).unwrap(); }
            "deleted" => { connection.execute("INSERT INTO sentinel_deleted_scans(scan_id) VALUES(?1)", [&lease.scan_id]).unwrap(); }
            _ => unreachable!(),
        }
        let before = single_finally_physical(&connection);
        assert_eq!(agent_record_http_observation(&context, "replay_http", &artifact, &view, "req-0001").unwrap_err(),
            "agent_attempt_not_active", "{stopped}");
        assert_eq!(single_finally_physical(&connection), before, "{stopped}");
        let count: i64 = connection.query_row("SELECT COUNT(*) FROM agent_evidence_nodes", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0, "{stopped}");
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(context.target_dir).unwrap();
    }
}

