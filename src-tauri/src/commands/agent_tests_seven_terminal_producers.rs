// Original producers only. Localhost responses exercise the actual gateway;
// disposable history/withdrawal are negative inputs, never Root or fee issuers.
fn seven_terminal_single(kind: &str) -> (AgentHarness, OwnedAgentTargetOutcome) {
    let mut h = if kind == "completed" {
        fresh_single_production_harness_with_sessions(
            "seven-completed",
            mock_site,
            &[("session-a", "cookie-alpha", "Bearer alpha")],
        )
    } else {
        fresh_single_production_harness(&format!("seven-{kind}"), mock_site)
    };
    if kind == "failed" {
        h.context.environment.llm.clear();
        h.context.execution_plan = build_agent_execution_plan(
            &AgentBudgetSettings::from_json(&json!({})),
            &h.context.route,
            &h.context.environment,
            AgentBackendKind::Native,
            &h.db_path,
            &h.context.scan_id,
        )
        .with_attempt(1);
    }
    freeze_fresh_single_production_harness(&mut h);
    let url = &h.context.target_url;
    let script = match kind {
        "completed" => vec![
            model_round(
                &[(
                    "compare_identities",
                    json!({"leftIdentity":"session-a","rightIdentity":"anonymous","method":"GET","path":"/api/orders","family":"authorization"}),
                )],
                900,
            ),
            model_round(
                &[(
                    "finish_target",
                    json!({"coverage":closing_ledger(&["authorization"]),"stopReason":"证据已穷尽"}),
                )],
                1000,
            ),
        ],
        "completed_with_gaps" => vec![
            model_round(
                &[(
                    "replay_http",
                    json!({"identity":"anonymous","method":"GET","url":format!("{url}/api/orders"),"family":"authorization"}),
                )],
                900,
            ),
            model_round(
                &[(
                    "finish_target",
                    json!({"coverage":closing_ledger(&[]),"stopReason":"匿名材料仍有命名缺口"}),
                )],
                1000,
            ),
        ],
        "protected_stop" => vec![model_round(
            &[(
                "replay_http",
                json!({"identity":"anonymous","method":"GET","url":format!("{url}/api/gated"),"family":"authorization"}),
            )],
            900,
        )],
        "failed" => vec![],
        _ => unreachable!(),
    };
    retarget_model(&mut h, script);
    let owned = execute_fresh_single_production_harness(&mut h);
    (h, owned)
}

fn seven_terminal_resume() -> (AgentHarness, OwnedAgentTargetOutcome) {
    let mut h = fresh_multi_production_harness("seven-resume");
    freeze_fresh_multi_production_harness(&mut h);
    retarget_model(
        &mut h,
        vec![proposal_model_response(&root_tick_valid_text(
            "paid original recovery refusal",
        ))],
    );
    let actor = bootstrap_original_actor(&h);
    let parent = start_original_fixture_supervisor(&h.db_path, &actor).unwrap();
    h.context.supervision = Some(parent.ticket());
    assert!(
        !native_coordinator_tick(&h.context, &actor)
            .unwrap()
            .replayed
    );
    drop(parent);
    h.context.supervision = None;
    let db = db::open(&h.db_path).unwrap();
    let original = bootstrap_original_sources(&db);
    bootstrap_original_history(&db, "target", &h.context);
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
    assert_eq!(
        owned.outcome.detail(),
        "target_execution_recovery_requires_fresh_attempt"
    );
    assert_eq!(bootstrap_original_sources(&db), original);
    (h, owned)
}

// Queue a real production request behind this harness's cloud admission gate
// (process-wide in production; per fake-provider origin under cfg(test)). The
// temporary scan status withdraws execution before transport. This is not an
// active-cancel IPC or remote cancellation acceptance test.
fn seven_terminal_queued_stop(status: &str) -> (AgentHarness, OwnedAgentTargetOutcome) {
    use crate::agent_runtime::model::{admission, CancelToken};
    use std::time::{Duration, Instant};
    let mut h = fresh_single_production_harness(&format!("seven-{status}"), mock_site);
    freeze_fresh_single_production_harness(&mut h);
    assert_eq!(h.context.environment.deployment, "cloud");
    let gate = admission::gate_for(false, &h.context.environment.api_base);
    let permits: Vec<_> = (0..admission::CLOUD_MAX_IN_FLIGHT)
        .map(|_| gate.acquire(&CancelToken::new()).unwrap())
        .collect();
    let path = h.db_path.clone();
    let scan = h.context.scan_id.clone();
    let (owned, queued, stopped) = std::thread::scope(|scope| {
        let thread = scope.spawn(|| execute_fresh_single_production_harness(&mut h));
        let deadline = Instant::now() + Duration::from_secs(5);
        while gate.queued() == 0 && Instant::now() < deadline && !thread.is_finished() {
            std::thread::sleep(Duration::from_millis(5));
        }
        let queued = gate.queued() == 1;
        let db = db::open(&path).unwrap();
        let stopped = db.execute(
            "UPDATE sentinel_scans SET status=?1 WHERE id=?2 AND status='scanning'",
            params![status, scan],
        );
        // Always release permits before joining, including timeout/error cases.
        // On the success path wait for removal so releasing capacity cannot
        // race a provider dispatch before the cancellation checker runs.
        let deadline = Instant::now() + Duration::from_secs(5);
        while queued && gate.queued() != 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let removed = gate.queued() == 0;
        drop(permits);
        (thread.join().unwrap(), queued && removed, stopped)
    });
    assert!(
        queued,
        "the actual request must queue and withdraw before transport"
    );
    assert_eq!(stopped.unwrap(), 1);
    assert!(
        matches!(owned.outcome, AgentTargetOutcome::Cancelled),
        "{:?}",
        owned.outcome
    );
    assert!(h.model_seen.lock().unwrap().is_empty());
    assert!(h.site_seen.lock().unwrap().is_empty());
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let phases: Vec<String> = db
        .prepare("SELECT phase FROM agent_root_model_journal WHERE root_run_id=?1 ORDER BY rowid")
        .unwrap()
        .query_map([root], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        phases,
        vec!["dispatch", "unsent"],
        "original unsent receipt, no uncertain or paid call"
    );
    (h, owned)
}
