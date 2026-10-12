// Real creator/startup/role SDK/Broker/owned consumption, shared with the
// production ownership regressions. No positive Root or capability SQL issuer.
fn native_policy_original_team(
    h: &AgentHarness,
    owned: &OwnedAgentTargetOutcome,
    expected_roles: &[&str],
) {
    assert!(
        matches!(
            owned.outcome,
            AgentTargetOutcome::Completed(_) | AgentTargetOutcome::BoundedCompleted(_)
        ),
        "{}",
        owned.outcome.detail()
    );
    let db = db::open(&h.db_path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(&db, root).unwrap();
    let mode = crate::agent_runtime::web_mode::root::read(&db, root)
        .unwrap()
        .unwrap();
    assert_eq!(mode.mode(), crate::agent_runtime::web_mode::WebMode::Multi);
    assert_eq!(
        mode.fact(),
        private_web_mode_on(&db, &h.context.scan_id, h.context.attempt_number)
            .unwrap()
            .fact()
    );
    let mut expected = expected_roles
        .iter()
        .map(|s| (*s).to_string())
        .collect::<Vec<_>>();
    expected.sort();
    let roles = db
        .prepare("SELECT role FROM agent_runs WHERE root_run_id=?1 ORDER BY role")
        .unwrap()
        .query_map([root], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        roles, expected,
        "only actually executed independent roles exist"
    );
    let children = roles.len() - 1;
    let count = |sql: &str| db.query_row(sql, [root], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count("SELECT count(*) FROM agent_runs WHERE id=?1 AND parent_run_id IS NULL AND root_run_id=id AND role='coordinator' AND backend='native' AND lane='read_only_analysis' AND status='terminal' AND terminal_state<>'' AND terminal_code<>'' AND capability_lease_json NOT LIKE '%replay_http%' AND capability_lease_json NOT LIKE '%browser_action%'"),1);
    assert_eq!(count("SELECT count(*) FROM agent_runs r JOIN agent_assignments a ON a.id=r.assignment_id WHERE r.root_run_id=?1 AND r.id<>?1 AND r.parent_run_id=?1 AND r.backend='native' AND r.orchestration_policy='multi' AND a.coordinator_run_id=?1 AND a.child_run_id=r.id AND a.role=r.role AND a.lane=r.lane AND a.state='completed' AND a.budget_settled_at<>''"),i64::try_from(children).unwrap());
    for sql in [
        "SELECT count(DISTINCT r.id) FROM agent_events e JOIN agent_runs r ON r.id=e.run_id WHERE r.root_run_id=?1 AND r.id<>?1 AND e.event_type='model_round_completed'",
        "SELECT count(DISTINCT r.id) FROM agent_snapshots s JOIN agent_runs r ON r.id=s.run_id WHERE r.root_run_id=?1 AND r.id<>?1",
    ] {assert_eq!(count(sql),i64::try_from(children).unwrap(),"{sql}");}
    assert_eq!(
        count("SELECT count(*) FROM tool_invocations WHERE run_id=?1"),
        0,
        "Root owns no target tools"
    );
    assert!(count("SELECT count(*) FROM tool_invocations t JOIN agent_runs r ON r.id=t.run_id WHERE r.root_run_id=?1 AND r.role='web_executor'")>0);
    assert_eq!(count("SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND role='web_executor' AND lane='target_touching' AND capability_lease_json LIKE '%replay_http%'"),1);
    assert_eq!(count("SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND role='spa_api_mapper' AND lane='read_only_analysis'"),1);
    assert_eq!(count("SELECT count(*) FROM agent_external_surface_captures c JOIN agent_runs r ON r.id=c.child_run_id JOIN agent_assignments a ON a.id=c.assignment_id WHERE r.root_run_id=?1 AND r.role='external_surface' AND r.lane='target_touching' AND c.state='received' AND a.state='completed'"),1);
    for table in [
        "agent_finding_candidates",
        "agent_review_requests",
        "agent_review_decisions",
    ] {
        assert_eq!(
            count(&format!(
                "SELECT count(*) FROM {table} WHERE root_run_id=?1"
            )),
            0,
            "{table}"
        );
    }
    assert_eq!(
        count("SELECT count(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'"),
        0,
        "no candidate cannot invent a Reviewer"
    );
    assert_eq!(count("SELECT count(DISTINCT kind) FROM agent_messages WHERE root_run_id=?1 AND kind IN ('evidence_summary','execution_assignment','execution_result') AND delivered_at<>'' AND acknowledged_at<>''"),3);
    assert!(count("SELECT count(*) FROM agent_messages WHERE root_run_id=?1 AND kind IN ('evidence_summary','execution_assignment','execution_result') AND delivered_at<>'' AND acknowledged_at<>''")>=5);
    assert_eq!(
        count("SELECT count(*) FROM agent_multi_exit_receipts WHERE root_run_id=?1"),
        1
    );
    let calls = i64::try_from(h.model_seen.lock().unwrap().len()).unwrap();
    let fee = crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "model_requests")
        .unwrap();
    assert_eq!(fee.consumed, calls);
    for dimension in crate::agent_runtime::multi_agent::budget::DIMENSIONS {
        let b =
            crate::agent_runtime::multi_agent::budget::balance(&db, root, None, dimension).unwrap();
        assert_eq!((b.reserved, b.indeterminate), (0, 0), "{dimension}");
    }
    assert!(fs::read_to_string(&h.context.log_path)
        .unwrap()
        .contains("native backend selected"));
    assert_eq!(
        agent_select_backend(
            &h.db_path,
            &h.context.scan_id,
            1,
            &h.context.target_url,
            &json!({}),
            true
        ),
        AgentBackendKind::Native
    );
    let mut tally = AgentPipelineTally::default();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert_eq!(tally.failed, 0);
    let status = native_scan_status(&db, &h.context.scan_id).unwrap();
    assert_eq!(status["findingCandidateCount"], 0);
    assert_eq!(status["multiAgentReady"], true, "{status}");
    let before = single_finally_physical(&db);
    let http = h.site_seen.lock().unwrap().len();
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 1);
    assert_eq!(single_finally_physical(&db), before);
    assert_eq!(
        h.model_seen.lock().unwrap().len(),
        usize::try_from(calls).unwrap()
    );
    assert_eq!(h.site_seen.lock().unwrap().len(), http);
}

#[test]
fn policy_native_coverage_only_is_ready_without_a_reviewer() {
    let (h, owned, stop) = multi_terminal_owned_dispatch();
    native_policy_original_team(
        &h,
        &owned,
        &[
            "coordinator",
            "spa_api_mapper",
            "external_surface",
            "web_executor",
            "client_side",
        ],
    );
    assert_eq!(h.site_seen.lock().unwrap().len(), 2);
    let state =
        NativeAgentState::read(&h.db_path, &h.context.scan_id, &h.context.target_url).unwrap();
    assert_eq!(state.target_requests, 1);
    assert!(
        !state
            .covered_families
            .contains(&"authorization".to_string()),
        "anonymous HTTP is not completed authorization coverage"
    );
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(owned);
    fs::remove_dir_all(h.root).unwrap();
}

#[test]
fn policy_native_routes_without_runtime_adapter() {
    let (h, owned, _) = fresh_multi_anonymous_control_dispatch(false);
    native_policy_original_team(
        &h,
        &owned,
        &[
            "coordinator",
            "spa_api_mapper",
            "identity_session",
            "external_surface",
            "web_executor",
            "client_side",
        ],
    );
    let requests = h.site_seen.lock().unwrap();
    assert_eq!(requests.len(), 4, "public surface capture plus three original Web Broker requests");
    let entries = requests
        .iter()
        .filter(|w| w.starts_with("GET / HTTP/"))
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 2);
    for entry in entries {
        let wire = entry.to_ascii_lowercase();
        assert!(!wire.contains("cookie:") && !wire.contains("authorization:"));
    }
    let profiles = requests
        .iter()
        .filter(|w| w.starts_with("GET /api/profile HTTP/"))
        .collect::<Vec<_>>();
    assert_eq!(profiles.len(), 2);
    assert_eq!(
        profiles
            .iter()
            .filter(|w| w.contains("cookie-alpha") && w.contains("Bearer alpha"))
            .count(),
        1
    );
    assert_eq!(
        profiles
            .iter()
            .filter(|w| !w.to_ascii_lowercase().contains("cookie:")
                && !w.to_ascii_lowercase().contains("authorization:"))
            .count(),
        1
    );
    drop(requests);
    let models = h.model_seen.lock().unwrap().join("\n");
    assert!(!models.contains("cookie-alpha") && !models.contains("Bearer alpha"));
    native_policy_original_readiness_damage(&h, &owned);
    drop(owned);
    fs::remove_dir_all(h.root).unwrap();
}

fn native_policy_original_readiness_damage(h: &AgentHarness, owned: &OwnedAgentTargetOutcome) {
    let source = db::open(&h.db_path).unwrap();
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    let original = single_finally_physical(&source);
    for (index, damage) in [
        Some("UPDATE agent_messages SET acknowledged_at='' WHERE kind='execution_result'"),
        Some("DELETE FROM tool_invocations WHERE run_id IN (SELECT id FROM agent_runs WHERE role='web_executor')"),
        None,
    ].into_iter().enumerate() {
        // SQLite makes a consistent disposable copy. Verify every typed row
        // and rowid before damage; never restore or reissue production authority.
        let path = h.root.join(format!("readiness-negative-{index}.sqlite3"));
        source.execute("VACUUM INTO ?1", [path.to_string_lossy()]).unwrap();
        let copy = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(single_finally_physical(&copy), original);
        assert_eq!(native_scan_status(&copy, &h.context.scan_id).unwrap()["multiAgentReady"], true);
        if let Some(sql) = damage {
            copy.execute_batch(sql).unwrap();
        } else {
            let legacy = crate::agent_runtime::multi_agent::assignment::AgentAssignment::new(
                "legacy-executor-sentinel", root,
                crate::agent_runtime::contract::AgentRole::DeepInvestigator,
                crate::agent_runtime::contract::AgentLane::TargetTouching,
                &h.context.target_url, "legacy-executor-sentinel",
            );
            crate::agent_runtime::multi_agent::assignment::insert_assignment(&copy, &legacy).unwrap();
        }
        let damaged = single_finally_physical(&copy);
        assert_eq!(native_scan_status(&copy, &h.context.scan_id).unwrap()["multiAgentReady"], false, "damage {index}");
        assert_eq!(single_finally_physical(&copy), damaged, "reader repaired damaged data");
        assert_eq!(single_finally_physical(&source), original);
        assert_eq!(native_scan_status(&source, &h.context.scan_id).unwrap()["multiAgentReady"], true);
    }
}

#[test]
fn native_policy_original_retired_frozen_matrix_cannot_execute_or_publish() {
    let mut h = fresh_web_production_harness_with_sessions(
        "retired-matrix-original",
        mock_site,
        &[],
        "multi",
    );
    freeze_fresh_multi_production_harness(&mut h);
    let db = db::open(&h.db_path).unwrap();
    db.execute("UPDATE sentinel_scan_attempts SET backend_plan_json=json_set(backend_plan_json,'$.targets[0].backend','strix') WHERE scan_id=?1 AND attempt_number=1",[&h.context.scan_id]).unwrap();
    let damaged = single_finally_physical(&db);
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: h.context.route.clone(),
        target_dir: h.context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    let settings = json!({});
    let owned = run_agent_target(
        &prepared,
        AgentTargetExecution {
            db_path: &h.db_path,
            scan_id: &h.context.scan_id,
            attempt_number: 1,
            settings: &settings,
            environment: &h.context.environment,
            adaptive: &AgentBudgetSettings::from_json(&settings),
            log_path: &h.context.log_path,
        },
    )
    .unwrap();
    assert!(matches!(owned.outcome, AgentTargetOutcome::Failed(_)));
    assert_eq!(owned.outcome.terminal_code(), AGENT_STOP_CONFIGURATION);
    assert!(owned.outcome.detail().contains("backend_retired"));
    assert!(h.model_seen.lock().unwrap().is_empty());
    assert!(h.site_seen.lock().unwrap().is_empty());
    assert_eq!(single_finally_physical(&db), damaged);
    let mut tally = AgentPipelineTally::default();
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(tally.counted(), 0);
    assert_eq!(single_finally_physical(&db), damaged);
    drop(owned);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
