#[test]
fn source_specialists_cancel_probe_is_assignment_scoped_and_fails_closed() {
    use crate::agent_runtime::{
        contract::{AgentBackendKind, AgentRole, MultiAgentPolicy},
        multi_agent::{lease as leases, scheduler, source},
        store::{self, AgentRunRow},
    };
    for mutation in [
        "child_cancel",
        "child_pause",
        "root_cancel",
        "root_terminal",
        "assignment_pause",
        "fence",
        "lease_expired",
        "capability_revoked",
        "capability_expired",
        "lane_lost",
        "attempt",
        "scan_pause",
    ] {
        let (root, connection, record, lease) = source_specialist_fixture();
        let slice = source::task_slice(&connection, &lease, AgentRole::RepoMapper).unwrap();
        let children = [scheduler::prepare_readonly_child(
            &connection,
            &lease,
            AgentRole::RepoMapper,
            "source_results_ready",
            &slice,
            8_000,
        )
        .unwrap()];
        // The same target has one readonly lane. Use another target's genuine
        // root/assignment to prove local cancellation leaves scan siblings live.
        let sibling_id = format!("{}-sibling", lease.root_run_id);
        let sibling_target = "https://sibling.example.invalid";
        let route = FrontendRoute {
            url: sibling_target.into(),
            score: 70,
            mode: "standard".into(),
            surface: "framework_application".into(),
            reasons: Vec::new(),
        };
        let environment = ModelRuntimeEnv {
            llm: "openai/test-model".into(),
            api_key: "fixture-only".into(),
            api_base: "https://model.invalid/v1".into(),
            deployment: "cloud".into(),
            full_power: false,
            prompt_audit_mode: "off".into(),
        };
        let plan = build_agent_execution_plan(
            &AgentBudgetSettings::from_json(&json!({})),
            &route,
            &environment,
            AgentBackendKind::Native,
            Path::new("/nonexistent/oviraptor.sqlite3"),
            &record.scan_id,
        )
        .with_attempt(1);
        let mut row = AgentRunRow::new(
            &sibling_id,
            &record.scan_id,
            1,
            sibling_target,
            AgentBackendKind::Native,
            AgentRole::Coordinator,
            plan.hash(),
            "sibling-evidence",
        )
        .with_budget(
            plan.soft_uncached_tokens,
            plan.hard_total_tokens,
            plan.soft_model_requests,
            plan.hard_model_requests,
        );
        row.root_run_id = sibling_id.clone();
        row.orchestration_policy = MultiAgentPolicy::Multi;
        store::create_run(&connection, &row).unwrap();
        connection
            .execute(
                "UPDATE agent_runs SET plan_json=?2 WHERE id=?1",
                params![sibling_id, plan.as_json().to_string()],
            )
            .unwrap();
        let sibling_lease = leases::acquire_coordinator_lease(
            &connection,
            &record.scan_id,
            1,
            sibling_target,
            &sibling_id,
            600,
        )
        .unwrap();
        let sibling_child = scheduler::prepare_readonly_child(
            &connection,
            &sibling_lease,
            AgentRole::SpaApiMapper,
            "evidence_ready",
            &json!({"objective":"readonly sibling"}),
            8_000,
        )
        .unwrap();
        let path = root.join("oviraptor.sqlite3");
        let first = specialist_model_cancel_token(&path, &lease, &children[0], None);
        let sibling = specialist_model_cancel_token(&path, &sibling_lease, &sibling_child, None);
        assert!(!first.is_cancelled(), "{mutation}");
        assert!(!sibling.is_cancelled());
        match mutation {
            "child_cancel" => {
                connection.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",[&children[0].run_id]).unwrap();
            }
            "child_pause" => {
                connection
                    .execute(
                        "UPDATE agent_runs SET status='paused' WHERE id=?1",
                        [&children[0].run_id],
                    )
                    .unwrap();
            }
            "root_cancel" => {
                connection.execute("UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",[&lease.root_run_id]).unwrap();
            }
            "root_terminal" => {
                connection
                    .execute(
                        "UPDATE agent_runs SET status='terminal' WHERE id=?1",
                        [&lease.root_run_id],
                    )
                    .unwrap();
            }
            "assignment_pause" => {
                connection
                    .execute(
                        "UPDATE agent_assignments SET state='paused' WHERE id=?1",
                        [&children[0].assignment_id],
                    )
                    .unwrap();
            }
            "fence" => {
                connection.execute("UPDATE agent_coordinator_leases SET fencing_token='replaced' WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
            }
            "lease_expired" => {
                connection.execute("UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01' WHERE root_run_id=?1",[&lease.root_run_id]).unwrap();
            }
            "capability_revoked" => {
                connection.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE assignment_id=?1",[&children[0].assignment_id]).unwrap();
            }
            "capability_expired" => {
                connection.execute("UPDATE agent_capability_leases SET lease_expires_at='2000-01-01' WHERE assignment_id=?1",[&children[0].assignment_id]).unwrap();
            }
            "lane_lost" => {
                connection
                    .execute(
                        "DELETE FROM agent_lane_leases WHERE assignment_id=?1",
                        [&children[0].assignment_id],
                    )
                    .unwrap();
            }
            "attempt" => {
                connection
                    .execute(
                        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
                        [&record.scan_id],
                    )
                    .unwrap();
            }
            "scan_pause" => {
                connection
                    .execute(
                        "UPDATE sentinel_scans SET status='paused' WHERE id=?1",
                        [&record.scan_id],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(first.is_cancelled(), "{mutation}");
        assert_eq!(
            sibling.is_cancelled(),
            matches!(mutation, "attempt" | "scan_pause"),
            "{mutation}"
        );
        assert!(specialist_model_cancel_token(
            &root.join("missing-parent/db"),
            &lease,
            &children[0],
            None
        )
        .is_cancelled());
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_specialists_inflight_child_cancel_aborts_transport_without_completing_or_retrying() {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{scheduler, source},
    };
    use std::sync::{atomic::Ordering, mpsc, Arc, Mutex};
    use std::time::Duration;
    let (arrived_tx, arrived_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release = Arc::new(Mutex::new(release_rx));
    let (port, _seen, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(move |_| {
        let _ = arrived_tx.send(());
        let _ = release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10));
        (200,"application/json",json!({"choices":[{"message":{"content":"late response","role":"assistant"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}).to_string())
    }));
    let environment = source_specialist_test_environment(port);
    let (root, connection, record, lease) = source_specialist_true_born_model_fixture(&environment);
    let role = AgentRole::RepoMapper;
    let slice = source::task_slice(&connection, &lease, role).unwrap();
    let input = source::assessment_input(&connection, &lease, role).unwrap();
    let (tokens, _) = source_assessment_budget(
        &source_assessment_messages("source assessment", &input),
        &agent_model_profile(&environment, None).unwrap(),
    )
    .unwrap();
    let child = scheduler::prepare_readonly_child(
        &connection,
        &lease,
        role,
        "source_results_ready",
        &slice,
        tokens,
    )
    .unwrap();
    let db_path = root.join("oviraptor.sqlite3");
    let context = SpecialistTransportContext { supervision: None,
        db_path: &db_path,
        scan_id: &record.scan_id,
        attempt_number: 1,
        target_key: &lease.target_key,
        run_id: &lease.root_run_id,
        environment: &environment,
        proxy: None,
        usage_dir: &root,
        deadline: None,
    };
    std::thread::scope(|scope| {
        let (done_tx, done_rx) = mpsc::channel();
        let (context_ref, lease_ref, child_ref, input_ref) = (&context, &lease, &child, &input);
        scope.spawn(move || {
            let _ = done_tx.send(specialist_round_transport(
                context_ref,
                lease_ref,
                child_ref,
                "source assessment",
                input_ref.clone(),
            ));
        });
        arrived_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the actual localhost model request must arrive");
        connection
            .execute(
                "UPDATE agent_runs SET cancel_requested_at=datetime('now','localtime') WHERE id=?1",
                [&child.run_id],
            )
            .unwrap();
        let result = done_rx.recv_timeout(Duration::from_secs(3));
        let _ = release_tx.send(());
        assert!(result
            .expect("child cancellation must abort before the provider responds")
            .is_err());
    });
    assert!(
        native_source_attempt_active(&connection, &record.scan_id, 1),
        "the parent scan is still active"
    );
    let receipt:(String,i64,i64)=connection.query_row("SELECT state,(SELECT count(*) FROM agent_messages WHERE assignment_id=?1),(SELECT count(*) FROM agent_events WHERE run_id=?2 AND event_type='model_round_completed') FROM agent_specialist_calls WHERE assignment_id=?1",params![child.assignment_id,child.run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    // A revoked source contract cannot append a received/uncertain receipt under
    // old authority. Keep the original executing claim as unresolved, not done.
    assert_eq!(receipt, ("executing".into(), 0, 0));
    connection
        .execute(
            "UPDATE agent_runs SET cancel_requested_at='' WHERE id=?1",
            [&child.run_id],
        )
        .unwrap();
    assert!(
        specialist_round_transport(&context, &lease, &child, "source assessment", input).is_err()
    );
    assert!(
        arrived_rx.try_recv().is_err(),
        "an unresolved model call must never be automatically resent"
    );
    stop.store(true, Ordering::SeqCst);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}
