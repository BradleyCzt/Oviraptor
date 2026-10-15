fn sdk_source_gate_contract(tool_phase: bool) {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{scheduler, source},
    };
    use std::sync::{mpsc, Arc, Mutex};
    {
        let (arrive, arrived) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let released = Arc::new(Mutex::new(released));
        let (port, seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(
            move |_| {
                let _ = arrive.send(());
                let _ = released
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5));
                let mut body: JsonValue =
                    serde_json::from_str(&super::agent_tests::sdk_exact_model_body(
                        "Source SDK exact assessment; not execution proof",
                    ))
                    .unwrap();
                if tool_phase {
                    body["choices"][0]["message"] = json!({"role":"assistant","tool_calls":[{"id":"sdk-finish","type":"function","function":{"name":"assignment.finish","arguments":json!({"summary":"Source SDK tool assessment","gaps":[]}).to_string()}}]});
                }
                (200, "application/json", body.to_string())
            },
        ));
        let env = source_specialist_test_environment(port);
        let (root, db, record, lease) = source_tool_true_born_fixture_model(Some(&env));
        db.execute("UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();
        let role = AgentRole::RepoMapper;
        let slice = if tool_phase {
            source::tool_task_slice(&db, &lease, role, 1).unwrap()
        } else {
            source::task_slice(&db, &lease, role).unwrap()
        };
        let input = source::assessment_input(&db, &lease, role).unwrap();
        let child = if tool_phase {
            let child = scheduler::schedule_child(
                &db,
                &lease,
                role,
                AgentLane::ReadOnlyAnalysis,
                "source_tools_ready",
                &slice,
                1,
                &source::tool_capabilities(role).unwrap(),
                24_000,
                3,
            )
            .unwrap();
            scheduler::mark_child_running(&db, &lease, &child).unwrap();
            child
        } else {
            let (tokens, _) = source_assessment_budget(
                &source_assessment_messages("source assessment", &input),
                &agent_model_profile(&env, None).unwrap(),
            )
            .unwrap();
            scheduler::prepare_readonly_child(
                &db,
                &lease,
                role,
                "source_results_ready",
                &slice,
                tokens,
            )
            .unwrap()
        };
        let database = root.join("oviraptor.sqlite3");
        let ctx = SpecialistTransportContext {
            supervision: None,
            db_path: &database,
            scan_id: &record.scan_id,
            attempt_number: 1,
            target_key: &lease.target_key,
            run_id: &lease.root_run_id,
            environment: &env,
            proxy: None,
            usage_dir: &root,
            deadline: None,
        };
        let native: String = db
            .query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [&lease.root_run_id],
                |r| r.get(0),
            )
            .unwrap();
        let (live, result) = std::thread::scope(|scope| {
            let (done, rx) = mpsc::channel();
            let context = &ctx;
            let actor = &lease;
            let worker = &child;
            let material = &slice;
            let input = &input;
            scope.spawn(move || {
                let outcome = if tool_phase {
                    let worker_db = db::open(context.db_path).unwrap();
                    execute_source_tool_assignment(&worker_db, context, actor, worker, material)
                        .map(|v| v.to_string())
                } else {
                    specialist_round_transport(
                        context,
                        actor,
                        worker,
                        "source assessment",
                        input.clone(),
                    )
                    .map(|(text, usage)| {
                        assert_eq!(usage.total_tokens, 12);
                        text
                    })
                };
                let _ = done.send(outcome);
            });
            if let Err(gate_error) = arrived.recv_timeout(Duration::from_secs(5)) {
                panic!("Source SDK gate did not arrive: {gate_error:?}; original execution error: {:?}",
                    rx.try_recv().map(|result| result.err()));
            }
            let live = super::agent_tests::sdk_log_rows(&db);
            let public = serde_json::to_value(
                crate::agent_runtime::model::diagnostics::replay::read(
                    &database,
                    &record.scan_id,
                    1,
                    None,
                    None,
                    0,
                    300,
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(public["rows"].as_array().unwrap().len(), 2);
            assert!(public["rows"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["runId"] == child.run_id));

            assert!(
                rx.try_recv().is_err(),
                "real Source model gate is still live"
            );
            let _ = release.send(());
            let result = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            (live, result)
        });
        result.unwrap();
        assert_eq!(
            live.iter()
                .map(|r| r["stage"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["prepared", "sent"]
        );
        let domain = if tool_phase {
            "source_round"
        } else {
            "specialist"
        };
        let worker: (String, String) = db
            .query_row(
                "SELECT id,worker_id FROM agent_assignment_attempts WHERE child_run_id=?1",
                [&child.run_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(live.iter().all(|r| r["domain"] == domain
            && r["runId"] == child.run_id
            && r["rootRunId"] == lease.root_run_id
            && r["assignmentId"] == child.assignment_id
            && r["leaseAttemptId"] == worker.0
            && r["workerId"] == worker.1));
        let rows = super::agent_tests::sdk_log_rows(&db);
        let public = serde_json::to_value(
            crate::agent_runtime::model::diagnostics::replay::read(
                &database,
                &record.scan_id,
                1,
                None,
                None,
                0,
                300,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(public["rows"].as_array().unwrap().len(), 6);
        assert_eq!(public["rows"][5]["terminalState"], "returned");

        assert_eq!(
            rows.iter()
                .map(|r| r["stage"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "prepared",
                "sent",
                "response_received",
                "cost_saved",
                "validated",
                "terminal"
            ]
        );
        assert_eq!(rows.last().unwrap()["terminalState"], "returned");
        assert_eq!(rows.last().unwrap()["costPhase"], "received");
        assert_eq!(
            db.query_row(
                "SELECT plan_json FROM agent_runs WHERE id=?1",
                [&lease.root_run_id],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            native
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(!json!(rows)
            .to_string()
            .contains("Source SDK exact assessment"));
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1",
                [&child.run_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn native_sdk_log_source_initial_actual_gate_binds_original_worker_without_http_grants() {
    sdk_source_gate_contract(false);
}

#[test]
fn native_sdk_log_source_tool_actual_gate_binds_original_worker_without_http_grants() {
    sdk_source_gate_contract(true);
}
