// Legacy Native finish renews its own Coordinator timestamps. Preserve every
// other column and row, including the exact lease identity and original rowid.
fn root_v2_slot_legacy_replay_snapshot(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Vec<(String, Vec<String>)> {
    let mut snapshot = super::tests::application_table_snapshot(db);
    let mut query = db.prepare(
        "SELECT rowid,*,scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4 AND lease_epoch=?5 AND fencing_token=?6 AS normalize_legacy_lease FROM agent_coordinator_leases ORDER BY rowid"
    ).unwrap();
    let width = query.column_count() - 1;
    let timestamp_columns: Vec<usize> = query.column_names()[..width].iter().enumerate()
        .filter_map(|(index, name)| matches!(*name, "lease_expires_at" | "heartbeat_at" | "updated_at").then_some(index))
        .collect();
    assert_eq!(timestamp_columns.len(), 3);
    let rows: Vec<(bool, String)> = query.query_map(
        params![lease.scan_id, lease.attempt_number, lease.target_key, lease.root_run_id,
            lease.lease_epoch, lease.fencing_token],
        |row| {
            let selected: bool = row.get(width)?;
            let mut values = (0..width).map(|index| row.get::<_, rusqlite::types::Value>(index))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if selected {
                for index in &timestamp_columns { values[*index] = rusqlite::types::Value::Null; }
            }
            Ok((selected, format!("{values:?}")))
        },
    ).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert_eq!(rows.iter().filter(|(selected, _)| *selected).count(), 1,
        "only the original exact Coordinator lease may refresh its timestamps");
    snapshot.iter_mut().find(|(name, _)| name == "agent_coordinator_leases").unwrap().1 =
        rows.into_iter().map(|(_, row)| row).collect();
    snapshot
}

fn root_v2_slot_legacy_none_has_no_http(h: &AgentHarness) {
    assert!(h.site_seen.lock().unwrap().is_empty());
    let db = db::open(&h.db_path).unwrap();
    for table in ["agent_http_request_claims", "agent_http_budget_origins", "agent_http_budget_receipts"] {
        assert_eq!(db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get::<_, i64>(0)).unwrap(), 0, "{table}");
    }
}

#[test]
fn root_v2_slot_legacy_none_finishes_without_http_advisory_and_replays_without_new_work() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let mut h = root_v2_slot_harness("legacy-none-http-finish", 1);
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    let mut session = multi_agent_prepare(&mut h.context).unwrap();
    let db = db::open(&h.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let mode = crate::agent_runtime::web_mode::root::read(&tx, &root).unwrap().unwrap();
    assert!(mode.bootstrap_dispatch().is_none());
    let before_gate = super::tests::application_table_snapshot(&tx);
    let changes = tx.total_changes();
    assert!(!native_http_budget_protocol_enabled_on(&tx, &root).unwrap());
    assert_eq!(super::tests::application_table_snapshot(&tx), before_gate);
    assert_eq!(tx.total_changes(), changes);
    drop(tx);
    assert_eq!(h.model_seen.lock().unwrap().len(), 3);
    root_v2_slot_legacy_none_has_no_http(&h);
    let outcome = AgentTargetOutcome::incomplete("legacy original zero HTTP close");
    multi_agent_finish_execution(&h.context, &mut session, &outcome).unwrap();
    assert_eq!(h.model_seen.lock().unwrap().len(), 3, "closing cannot invent a new paid advisory");
    root_v2_slot_legacy_none_has_no_http(&h);
    assert_eq!(budget::balance(&db, &root, None, "model_requests").unwrap().consumed, 3);
    assert_eq!(budget::balance(&db, &root, None, "target_requests").unwrap(), budget::Balance::default());
    assert_eq!(budget::balance(&db, &root, None, "concurrency_batches").unwrap(), budget::Balance::default());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_lane_leases WHERE assignment_id=?1", [&session.executor.assignment_id], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_messages WHERE assignment_id=?1 AND kind='execution_result' AND acknowledged_at<>'' AND delivery_attempts=1", [&session.executor.assignment_id], |row| row.get::<_, i64>(0)).unwrap(), 1);

    // Force the old metadata refresh to be observable without relying on a
    // same-second timestamp coincidence. Keep the original future expiry.
    assert_eq!(db.execute("UPDATE agent_coordinator_leases SET heartbeat_at='2000-01-01 00:00:00',updated_at='2000-01-01 00:00:00' WHERE scan_id=?1 AND attempt_number=?2 AND target_key=?3 AND root_run_id=?4 AND lease_epoch=?5 AND fencing_token=?6",
        params![session.lease.scan_id, session.lease.attempt_number, session.lease.target_key,
            session.lease.root_run_id, session.lease.lease_epoch, session.lease.fencing_token]).unwrap(), 1);
    let before = root_v2_slot_legacy_replay_snapshot(&db, &session.lease);
    multi_agent_finish_execution(&h.context, &mut session, &outcome).unwrap();
    assert_eq!(root_v2_slot_legacy_replay_snapshot(&db, &session.lease), before,
        "full replay cannot change business, financial, scope or other lease rows");
    let refreshed: (String, String) = db.query_row("SELECT heartbeat_at,updated_at FROM agent_coordinator_leases WHERE root_run_id=?1", [&root], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    assert_ne!(refreshed.0, "2000-01-01 00:00:00");
    assert_ne!(refreshed.1, "2000-01-01 00:00:00");
    assert_eq!(h.model_seen.lock().unwrap().len(), 3);
    root_v2_slot_legacy_none_has_no_http(&h);
    drop(session);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn root_v2_slot_legacy_none_actual_exchange_still_refuses_before_claim_fee_or_network() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let mut h = root_v2_slot_harness("legacy-none-actual-http-refusal", 1);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let db = db::open(&h.db_path).unwrap();
    let run = h.context.run.as_ref().unwrap();
    assert_eq!(run.run_id, session.executor.run_id);
    assert_eq!(agent_authorize_tool_on(&db, &h.context, "replay_http"), Ok(()),
        "the refusal must not be caused by an invalid executor or grant");
    let arguments = json!({"identity":"anonymous","method":"GET",
        "url":format!("{}/", h.context.target_url),"family":"authorization"});
    let mut runtime = AgentToolRuntime::default();
    let invocation = runtime.begin_invocation(h.context.attempt_number);
    run.begin_tool("replay_http", &arguments, &invocation).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM tool_invocations WHERE run_id=?1 AND invocation_id=?2 AND tool_name='replay_http' AND policy_decision='allow' AND status='running'",
        params![run.run_id, invocation], |row| row.get::<_, i64>(0)).unwrap(), 1);
    let request = AgentHttpRequest {
        identity: AgentIdentity::anonymous(), method: "GET".into(),
        url: format!("{}/", h.context.target_url), extra_headers: vec![], body: None,
        content_type: None, contract_key: String::new(), family: "authorization".into(),
        source: ScopeSource::IdentityComparison, tool: "replay_http".into(), timeout_seconds: 5,
    };
    let before = super::tests::application_table_snapshot(&db);
    let changes = db.total_changes();
    let error = agent_http_exchange(&h.context, &mut runtime, &request).unwrap_err();
    assert_eq!(error["code"], "evidence_write_failed");
    assert_eq!(error["error"], "http_budget_receipt_original_v8_protocol_required",
        "a valid running executor must reach the unchanged physical protocol gate");
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(db.total_changes(), changes);
    assert_eq!(runtime.target_requests, 0);
    assert_eq!(runtime.current_request_index, 0);
    assert_eq!(runtime.invocations, 1);
    assert_eq!(runtime.current_invocation, invocation);
    assert!(runtime.requests.is_empty());
    assert!(runtime.last_artifact_id.is_empty());
    assert_eq!(h.model_seen.lock().unwrap().len(), 3);
    root_v2_slot_legacy_none_has_no_http(&h);
    assert_eq!(budget::balance(&db, &session.lease.root_run_id, None, "target_requests").unwrap(), budget::Balance::default());
    drop(session);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}

include!("agent_tests_root_v2_slot_http_legacy_birth.rs");
