    /// Phase 2 §2.3 cases 1 and 2: a fresh re-run uses Native even when the
    /// previous attempt is a frozen retired plan or settings contain a legacy
    /// backend alias. The previous attempt's plan remains readable.
    #[test]
    fn fresh_rerun_uses_native_without_rewriting_historical_plans() {
        let (_root, db_path) = temp_database("phase2-fresh-switch");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "fresh");
        let url = "https://app.example.invalid";
        let native = serde_json::json!({"agentBackendPolicy": "native"});
        let strix = serde_json::json!({"agentBackendPolicy": "strix"});

        // Attempt 1 is a historical run and keeps the retired backend marker.
        seed_retired_attempt_plan(
            &db_path,
            "agent-scan",
            1,
            url,
            &AgentExecutionPlan {
                backend: AgentBackendKind::LegacyRemoved,
                ..test_plan_for("standard", url)
            },
        );
        assert_eq!(
            agent_select_backend(&db_path, "agent-scan", 1, url, &native, true),
            AgentBackendKind::LegacyRemoved,
            "the attempt that already ran stays pinned"
        );
        assert_eq!(
            agent_select_backend(&db_path, "agent-scan", 2, url, &native, true),
            AgentBackendKind::Native,
            "a fresh attempt must follow the policy the user just changed"
        );
        // A historical attempt must never be rewritten into Native just to
        // satisfy a new setting; only a fresh attempt can select its backend.
        assert_eq!(persist_agent_execution_plan(
            &db_path,
            "agent-scan",
            1,
            url,
            &AgentExecutionPlan {
                backend: AgentBackendKind::Native,
                ..test_plan_for("standard", url)
            },
        ).unwrap_err(), "agent_runs_backend_unsupported");
        let switched = temp_database("phase2-fresh-switch-back");
        let _ = switched.0;
        seed_scan(&switched.1, "agent-scan", "scanning");
        seed_attempt_row(&switched.1, 1, "initial");
        seed_attempt_row(&switched.1, 2, "fresh");
        persist_agent_execution_plan(
            &switched.1,
            "agent-scan",
            1,
            url,
            &AgentExecutionPlan {
                backend: AgentBackendKind::Native,
                ..test_plan_for("standard", url)
            },
        ).unwrap();
        assert_eq!(
            agent_select_backend(&switched.1, "agent-scan", 2, url, &strix, true),
            AgentBackendKind::Native,
            "a fresh eligible attempt ignores a retired strix policy"
        );
        // §2.2: switching is never achieved by wiping history.
        let connection = db::open(&db_path).unwrap();
        let pinned: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_runs WHERE scan_id='agent-scan' AND attempt_number=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(pinned >= 1, "attempt 1 keeps its own plan row");
    }

    #[test]
    fn one_attempt_plan_is_immutable_and_replay_is_idempotent() {
        let (_root, db_path) = temp_database("attempt-plan-immutable");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        let url = "https://app.example.invalid";
        let first = test_plan_for("standard", url).with_attempt(1);
        persist_agent_execution_plan(&db_path, "agent-scan", 1, url, &first).unwrap();
        persist_agent_execution_plan(&db_path, "agent-scan", 1, url, &first).unwrap();

        let mut changed = first.clone();
        changed.mode = "deep".into();
        assert_eq!(persist_agent_execution_plan(&db_path, "agent-scan", 1, url, &changed).unwrap_err(),
            "agent_attempt_plan_frozen_conflict");
        let connection = db::open(&db_path).unwrap();
        assert_eq!(crate::agent_runtime::store::attempt_plan(&connection, "agent-scan", 1, url), Some(first.as_json()));
        assert_eq!(read_agent_checkpoint(&db_path, "agent-scan", url, "agent_execution_plan"), first.as_json());
    }

    #[test]
    fn web_only_plan_is_frozen_without_rewriting_legacy_v1_history() {
        let new_plan = test_plan("standard").with_attempt(1);
        let current = new_plan.as_json();
        assert_eq!(current["schemaVersion"], 2);
        assert_eq!(current["executionSurface"], "web_only");
        assert_eq!(AgentExecutionPlan::from_json(&current), Some(new_plan.clone()));

        let mut historical = current.clone();
        historical["schemaVersion"] = serde_json::json!(1);
        historical.as_object_mut().unwrap().remove("executionSurface");
        let inherited = AgentExecutionPlan::from_json(&historical).unwrap();
        assert_eq!(inherited.as_json(), historical, "legacy JSON and its hash stay stable");
        assert_ne!(inherited.hash(), new_plan.hash());
        assert_eq!(inherited.with_attempt(2).frozen_json(), inherited.frozen_json());

        let mut forged = historical.clone();
        forged["executionSurface"] = serde_json::json!("host_linux");
        assert!(AgentExecutionPlan::from_json(&forged).is_none());
        for surface in ["host_linux", "auto", "web_only+host"] {
            let mut forged = current.clone();
            forged["executionSurface"] = serde_json::json!(surface);
            assert!(AgentExecutionPlan::from_json(&forged).is_none(), "{surface}");
        }
        let mut future = current;
        future["schemaVersion"] = serde_json::json!(3);
        assert!(AgentExecutionPlan::from_json(&future).is_none());
    }

    #[test]
    fn concurrent_plan_freeze_has_one_winner_and_one_projection() {
        use std::sync::{Arc, Barrier};

        let (_root, db_path) = temp_database("attempt-plan-race");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        let url = "https://app.example.invalid";
        let first = test_plan_for("standard", url).with_attempt(1);
        let second = test_plan_for("deep", url).with_attempt(1);
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            let jobs: Vec<_> = [first.clone(), second.clone()]
                .into_iter()
                .map(|plan| {
                    let barrier = Arc::clone(&barrier);
                    let db_path = &db_path;
                    scope.spawn(move || {
                        barrier.wait();
                        persist_agent_execution_plan(db_path, "agent-scan", 1, url, &plan)
                    })
                })
                .collect();
            jobs.into_iter().map(|job| job.join().unwrap()).collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.as_ref().err().is_some_and(|error| error == "agent_attempt_plan_frozen_conflict")).count(), 1);
        let winner = if results[0].is_ok() { first } else { second };
        let connection = db::open(&db_path).unwrap();
        assert_eq!(crate::agent_runtime::store::attempt_plan(&connection, "agent-scan", 1, url), Some(winner.as_json()));
        assert_eq!(read_agent_checkpoint(&db_path, "agent-scan", url, "agent_execution_plan"), winner.as_json());
    }

    #[test]
    fn missing_attempt_plan_never_inherits_another_attempts_url_checkpoint() {
        let (_root, db_path) = temp_database("attempt-plan-cross-inheritance");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "fresh");
        let url = "https://app.example.invalid";
        let newer = test_plan_for("deep", url).with_attempt(2);
        persist_agent_execution_plan(&db_path, "agent-scan", 2, url, &newer).unwrap();
        assert_eq!(frozen_plan_of(&db_path, "agent-scan", 1, url), serde_json::Value::Null);
        assert_eq!(frozen_plan_of(&db_path, "agent-scan", 2, url), newer.as_json());
    }

    #[test]
    fn old_attempt_replay_never_rewinds_latest_url_projection() {
        let (_root, db_path) = temp_database("attempt-plan-projection-order");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "fresh");
        let url = "https://app.example.invalid";
        let older = test_plan_for("standard", url).with_attempt(1);
        let newer = test_plan_for("deep", url).with_attempt(2);
        persist_agent_execution_plan(&db_path, "agent-scan", 1, url, &older).unwrap();
        persist_agent_execution_plan(&db_path, "agent-scan", 2, url, &newer).unwrap();
        persist_agent_execution_plan(&db_path, "agent-scan", 1, url, &older).unwrap();
        assert_eq!(read_agent_checkpoint(&db_path, "agent-scan", url, "agent_execution_plan"), newer.as_json());
    }

    #[test]
    fn failed_detail_projection_rolls_back_the_new_authoritative_plan() {
        let (_root, db_path) = temp_database("attempt-plan-atomic-projection");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        let url = "https://app.example.invalid";
        let connection = db::open(&db_path).unwrap();
        connection.execute_batch(
            "CREATE TRIGGER refuse_plan_projection BEFORE INSERT ON sentinel_checkpoints \
             WHEN NEW.stage='agent_execution_plan' BEGIN SELECT RAISE(ABORT,'projection unavailable'); END;",
        ).unwrap();
        let plan = test_plan_for("standard", url).with_attempt(1);
        assert!(persist_agent_execution_plan(&db_path, "agent-scan", 1, url, &plan)
            .unwrap_err().contains("projection unavailable"));
        assert!(crate::agent_runtime::store::attempt_plan(&connection, "agent-scan", 1, url).is_none());
        assert_eq!(read_agent_checkpoint(&db_path, "agent-scan", url, "agent_execution_plan"), serde_json::Value::Null);
    }

    #[test]
    fn failed_attempt_plan_storage_does_not_advance_checkpoint_projection() {
        let (_root, db_path) = temp_database("attempt-plan-write-failure");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "fresh");
        let url = "https://app.example.invalid";
        let first = test_plan_for("standard", url).with_attempt(1);
        persist_agent_execution_plan(&db_path, "agent-scan", 1, url, &first).unwrap();
        let connection = db::open(&db_path).unwrap();
        connection.execute_batch(
            "CREATE TRIGGER refuse_second_plan BEFORE INSERT ON agent_runs \
             WHEN NEW.attempt_number=2 BEGIN SELECT RAISE(ABORT,'plan write unavailable'); END;",
        ).unwrap();
        let next = test_plan_for("deep", url).with_attempt(2);
        assert!(persist_agent_execution_plan(&db_path, "agent-scan", 2, url, &next)
            .unwrap_err().contains("plan write unavailable"));
        assert!(crate::agent_runtime::store::attempt_plan(&connection, "agent-scan", 2, url).is_none());
        assert_eq!(read_agent_checkpoint(&db_path, "agent-scan", url, "agent_execution_plan"), first.as_json());
    }

    /// Phase 2 §5.3: a durable write that cannot be made ends the attempt. No
    /// further model round and no further target request may be spent on state the
    /// runtime cannot record.
    #[test]
    fn persistence_failure_stops_before_the_next_model_round() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        let mut harness = agent_harness(
            "p05-usage-write",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        let target_url = harness.context.target_url.clone();
        let scripted = vec![
            model_round(
                &[(
                    "replay_http",
                    serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"business_flow"}),
                )],
                900,
            ),
            model_round(&[("finish_target", serde_json::json!({"coverage": closing_ledger(&["business_flow"]), "stopReason":"收口"}))], 900),
        ];
        // The first round is served normally; the checkpoint table then disappears,
        // so the commit that must follow the tool result cannot be made. The rows
        // the round already earned stay, which is exactly why the loop may not spend
        // another round on state it cannot record (§5.2).
        let database = harness.db_path.clone();
        let counter = std::sync::Arc::new(AtomicUsize::new(0));
        let rows = scripted.clone();
        let (model_port, model_seen, _model_stop) =
            spawn_endpoint(std::sync::Arc::new(move |request| {
                if !request.contains("POST /v1/chat/completions") {
                    return (404, "text/plain", "unexpected path".to_string());
                }
                let index = counter.fetch_add(1, AtomicOrdering::SeqCst);
                if index == 0 {
                    let connection = db::open(&database).unwrap();
                    connection
                        .execute("DROP TABLE sentinel_checkpoints", [])
                        .unwrap();
                }
                (
                    200,
                    "application/json",
                    rows.get(index)
                        .cloned()
                        .unwrap_or_else(|| model_round(&[], 10)),
                )
            }));
        harness.context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        harness.model_seen = model_seen;
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert_eq!(
            outcome.terminal_status(),
            "persistence_failure",
            "an unwritable ledger is its own end state: {:?}",
            outcome.detail()
        );
        assert_eq!(
            outcome.terminal_code(),
            crate::agent_runtime::contract::terminal_code::PERSISTENCE_FAILURE
        );
        assert!(
            outcome
                .detail()
                .contains("本地记录失败，已停止以避免重复消耗"),
            "the reason names the local record failure: {:?}",
            outcome.detail()
        );
        let prompts = harness.model_seen.lock().unwrap().join("\n");
        assert_eq!(
            prompts.matches("POST /v1/chat/completions").count(),
            1,
            "the attempt must stop before a second model round"
        );
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            1,
            "and never spend a second target request"
        );
    }

    /// Phase 2 §5.3: the opposite half — the database is fine but the raw artifact
    /// cannot be written, so the tool refuses and the loop stops.
    #[test]
    fn artifact_write_failure_refuses_the_tool_and_stops_the_run() {
        let mut harness = agent_harness(
            "p05-artifact",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"business_flow"}),
                    )],
                    900,
                ),
                model_round(&[("finish_target", serde_json::json!({"coverage": closing_ledger(&[]), "stopReason":"收口"}))], 900),
            ],
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&harness.context.target_dir, fs::Permissions::from_mode(0o500))
                .unwrap();
        }
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert_eq!(
            outcome.terminal_status(),
            "persistence_failure",
            "{:?}",
            outcome.detail()
        );
        assert_eq!(
            harness.model_seen.lock().unwrap().len(),
            1,
            "no evidence, no second round"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(
                &harness.context.target_dir,
                fs::Permissions::from_mode(0o700),
            );
        }
    }

    /// Phase 2 §5.2: a locked database gets a short, bounded retry; anything else
    /// fails immediately instead of waiting forever.
    #[test]
    fn locked_writes_retry_bounded_and_other_errors_do_not() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let attempts = AtomicUsize::new(0);
        let ok = with_db_retry("检查点写入", || {
            let seen = attempts.fetch_add(1, Ordering::SeqCst);
            if seen < 2 {
                return Err("database is locked".to_string());
            }
            Ok(())
        });
        assert!(ok.is_ok(), "a lock that clears inside the budget still commits");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);

        let disk = AtomicUsize::new(0);
        let failed: Result<(), String> = with_db_retry("检查点写入", || {
            disk.fetch_add(1, Ordering::SeqCst);
            Err("attempt to write a readonly database".to_string())
        });
        assert!(failed.is_err());
        assert_eq!(
            disk.load(Ordering::SeqCst),
            1,
            "a non-lock failure is reported at once, never retried"
        );
    }

    /// §11: the new terminal state is spelled one way everywhere, is never called a
    /// model failure, and is not picked up by an automatic resume.
    #[test]
    fn persistence_failure_is_its_own_terminal_state() {
        let outcome = AgentTargetOutcome::persistence_failure("本地记录写入失败");
        assert_eq!(outcome.terminal_status(), "persistence_failure");
        assert_ne!(outcome.terminal_status(), "failed");
        assert!(
            !outcome.stop().is_some_and(|stop| stop.requires_fuse()),
            "a local write failure never fuses the target"
        );
        let (_root, db_path) = temp_database("p05-resume-scope");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_target(&db_path, "https://app.example.invalid");
        let connection = db::open(&db_path).unwrap();
        connection
            .execute(
                "UPDATE sentinel_targets SET status='persistence_failure' WHERE scan_id='agent-scan'",
                [],
            )
            .unwrap();
        let resumable: i64 = connection
            .query_row(SENTINEL_RESUME_COUNT_SQL, ["agent-scan"], |row| row.get(0))
            .unwrap();
        assert_eq!(resumable, 0, "only 重新执行 may pick it up again");
    }

    /// Phase 2 §6.2 and §6.3: a stop that lands while a generation is in flight has
    /// to settle the transport first, say so in the ledger with real timestamps, and
    /// never bill or continue a round the runtime never read.
    #[test]
    fn a_stop_during_a_model_round_settles_the_transport_and_is_never_billed() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        // The task comes from the real creator/startup/HMAC and the original
        // frozen Single Root (plan + budget declaration), as Native requires.
        let mut harness = fresh_single_production_harness("p06-cancel", mock_site);
        freeze_fresh_single_production_harness(&mut harness);
        let AgentHarness {
            db_path,
            mut context,
            site_seen,
            ..
        } = harness;
        let ledger = context.run.clone().expect("run row");
        let pause_db = db_path.clone();
        let served = std::sync::Arc::new(AtomicUsize::new(0));
        let served_counter = served.clone();
        let (model_port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(move |_request| {
            if served_counter.fetch_add(1, AtomicOrdering::SeqCst) == 0 {
                // The user stops the task while this generation is still running; the
                // provider answers five seconds later, which is exactly the request
                // that used to keep running after the UI said "stopped". A running
                // task is stopped through the real pause request (cancel only
                // removes drafts), so drive that production entry here.
                request_sentinel_pause(&pause_db, "agent-scan").unwrap();
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
            (
                200,
                "application/json",
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({"coverage": closing_ledger(&[]), "stopReason":"收口"}),
                    )],
                    900,
                ),
            )
        }));
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");

        let started = std::time::Instant::now();
        let outcome = NativeAgentBackend.execute(&context);
        let elapsed = started.elapsed();
        assert!(
            matches!(outcome, AgentTargetOutcome::Cancelled),
            "{:?}",
            outcome.detail()
        );
        assert!(
            elapsed < std::time::Duration::from_secs(4),
            "the stop waited for the provider instead of closing it: {elapsed:?}"
        );
        assert_eq!(
            site_seen.lock().unwrap().len(),
            0,
            "a round that never returned may not spend a target request"
        );

        let connection = db::open(&db_path).unwrap();
        let settled: Option<(i64, String, String)> = connection
            .query_row(
                "SELECT sequence,json_extract(payload_json,'$.cancelRequestedAt'),json_extract(payload_json,'$.transportClosedAt') FROM agent_events WHERE run_id=?1 AND event_type='transport_cancelled' ORDER BY sequence LIMIT 1",
                [&ledger.run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .ok();
        let (cancel_sequence, requested, closed) =
            settled.expect("the ledger holds when the stop settled");
        assert!(!requested.is_empty() && !closed.is_empty());
        let later_rounds: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM agent_events WHERE run_id=?1 AND sequence>?2 AND event_type='model_round_completed'",
                params![&ledger.run_id, cancel_sequence],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            later_rounds, 0,
            "the transport must settle before the round is closed out"
        );
        let (run_status, terminal_state): (String, String) = connection
            .query_row(
                "SELECT status,terminal_state FROM agent_runs WHERE id=?1",
                [&ledger.run_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_ne!(
            run_status, "terminal",
            "a stop keeps the attempt resumable instead of inventing an end"
        );
        assert!(terminal_state.is_empty(), "{terminal_state}");
        assert_eq!(
            connection
                .query_row(
                    "SELECT used_tokens FROM agent_runs WHERE id=?1",
                    [&ledger.run_id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0,
            "an unread round is not billed"
        );
        // Original Root budget: the in-flight request is never consumed, and its
        // unknown provider outcome stays held (not refunded) for reconciliation.
        let root_requests = crate::agent_runtime::multi_agent::budget::balance(
            &connection,
            &ledger.run_id,
            Some(""),
            "model_requests",
        )
        .unwrap();
        assert_eq!((root_requests.consumed, root_requests.indeterminate), (0, 1));
    }

    /// Phase 2 §3.2: the matrix classifies **every** target before any dependency is
    /// resolved, so a mixed scan is known up front and an all-native one needs no
    /// Strix, Docker or Python at all.
    #[test]
    fn backend_matrix_covers_every_target_before_dependencies() {
        let (_root, db_path) = temp_database("phase2-matrix");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        let urls = vec![
            "https://a.example.invalid".to_string(),
            "https://b.example.invalid".to_string(),
            "https://c.example.invalid".to_string(),
        ];
        let native = serde_json::json!({"agentBackendPolicy": "native"});
        let matrix = plan_scan_backends(&db_path, "agent-scan", 1, &urls, &native, true).unwrap();
        assert_eq!(matrix.targets.len(), 3, "one entry per queued target");
        assert!(
            matrix
                .targets
                .iter()
                .all(|target| target.backend == AgentBackendKind::Native),
            "{matrix:?}"
        );
        assert!(matrix.as_json().get("requiresStrix").is_none());
        assert!(matrix.as_json().get("requiresDocker").is_none());
        assert!(matrix.requires_node, "web recon still needs the Node probe");
        assert!(matrix.requires_browser);

        // A matrix is frozen once built, so re-running it cannot change a target.
        let rebuilt = plan_scan_backends(&db_path, "agent-scan", 1, &urls, &serde_json::json!({"agentBackendPolicy":"strix"}), true).unwrap();
        assert_eq!(
            rebuilt, matrix,
            "replanning the same attempt must be a no-op"
        );

        // Mixed case: one target already pinned Strix for this attempt before the
        // matrix is first built. The matrix must report the whole dependency set,
        // not just what the first URL says.
        let (_mixed_root, mixed_db) = temp_database("phase2-matrix-mixed");
        seed_scan(&mixed_db, "agent-scan", "scanning");
        seed_attempt_row(&mixed_db, 1, "initial");
        seed_retired_attempt_plan(
            &mixed_db,
            "agent-scan",
            1,
            &urls[1],
            &AgentExecutionPlan {
                backend: AgentBackendKind::LegacyRemoved,
                ..test_plan_for("standard", &urls[1])
            },
        );
        let rebuilt = plan_scan_backends(&mixed_db, "agent-scan", 1, &urls, &native, true).unwrap();
        assert_eq!(rebuilt.targets[1].backend, AgentBackendKind::LegacyRemoved);
        assert_eq!(rebuilt.targets[0].backend, AgentBackendKind::Native);
        assert!(prepare_scan_dependencies(&rebuilt)
            .unwrap_err()
            .starts_with("backend_retired:"));
        let blocked: Vec<&str> = rebuilt
            .targets
            .iter()
            .filter(|target| target.backend == AgentBackendKind::LegacyRemoved)
            .map(|target| target.url.as_str())
            .collect();
        assert_eq!(blocked, vec!["https://b.example.invalid"]);
    }

    /// Phase 2 §3.2: a settings edit in the middle of an attempt cannot move a target
    /// that the frozen matrix already classified.
    #[test]
    fn backend_matrix_is_frozen_for_the_attempt() {
        let (_root, db_path) = temp_database("phase2-matrix-frozen");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        let urls = vec!["https://a.example.invalid".to_string()];
        let matrix = plan_scan_backends(
            &db_path,
            "agent-scan",
            1,
            &urls,
            &serde_json::json!({"agentBackendPolicy": "native"}),
            true,
        ).unwrap();
        assert_eq!(matrix.targets[0].backend, AgentBackendKind::Native);
        // The user switches the policy to Strix while the attempt is running.
        let (backend, reason) = agent_backend_choice(
            &db_path,
            "agent-scan",
            1,
            &urls[0],
            &serde_json::json!({"agentBackendPolicy": "strix"}),
            true,
        );
        assert_eq!(
            backend,
            AgentBackendKind::Native,
            "the frozen matrix must win over today's settings: {reason}"
        );
        // A different attempt is a different matrix and does follow the new policy.
        seed_attempt_row(&db_path, 2, "fresh");
        assert_eq!(
            agent_select_backend(
                &db_path,
                "agent-scan",
                2,
                &urls[0],
                &serde_json::json!({"agentBackendPolicy": "strix"}),
                true
            ),
            AgentBackendKind::Native,
            "a new attempt does not revive the retired strix policy"
        );
    }

    /// Phase 2 §2.3 case 3: a continuation inherits the parent's frozen backend even
    /// when the settings now say otherwise.
    #[test]
    fn resume_inherits_the_parents_backend_and_never_the_new_settings() {
        let (_root, db_path) = temp_database("phase2-resume-backend");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "resume");
        let url = "https://app.example.invalid";
        persist_agent_execution_plan(
            &db_path,
            "agent-scan",
            1,
            url,
            &AgentExecutionPlan {
                backend: AgentBackendKind::Native,
                ..test_plan_for("standard", url)
            },
        ).unwrap();
        assert_eq!(
            agent_select_backend(&db_path, "agent-scan", 2, url, &serde_json::json!({"agentBackendPolicy":"strix"}), true),
            AgentBackendKind::Native,
            "a continuation cannot change backend mid-chain"
        );
    }

    /// Phase 2 §2.3 case 4: a continuation with no parent plan stops as
    /// `resume_incompatible` before spending a model or target request.
    #[test]
    fn resume_without_a_parent_plan_spends_nothing() {
        let mut harness = agent_harness(
            "phase2-no-parent-plan",
            mock_site,
            vec![AgentIdentity::anonymous()],
        );
        let url = harness.context.target_url.clone();
        seed_attempt_row(&harness.db_path, 1, "initial");
        seed_attempt_row(&harness.db_path, 2, "resume");
        harness.context.attempt_number = 2;
        db::open(&harness.db_path).unwrap().execute("UPDATE sentinel_scans SET attempt_count=2 WHERE id='agent-scan'", []).unwrap();
        harness.context.resume = true;
        let outcome = NativeAgentBackend.execute(&harness.context);
        assert_eq!(outcome.terminal_status(), "resume_incompatible", "{:?}", outcome.detail());
        assert!(harness.site_seen.lock().unwrap().is_empty(), "the target was never touched");
        assert!(harness.model_seen.lock().unwrap().is_empty(), "the model was never asked");
        assert_eq!(
            NativeAgentState::read(&harness.db_path, "agent-scan", &url)
                .map(|state| state.attempt_number),
            None,
            "no state may be created by a refused continuation"
        );
    }

    /// Phase 2 §2.3 case 5: fresh means nothing carries over — not tokens, queue,
    /// terminal reason, no-progress streak or exhausted contracts.
    #[test]
    fn fresh_attempt_inherits_nothing_from_its_predecessor() {
        let (_root, db_path) = temp_database("phase2-fresh-nothing");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_attempt_row(&db_path, 1, "initial");
        seed_attempt_row(&db_path, 2, "fresh");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let evidence_hash = agent_stable_hash(&context.evidence);
        let plan_hash = context.execution_plan.hash();
        let mut parent = NativeAgentState::fresh(
            1,
            AgentBackendKind::Native,
            &evidence_hash,
            &plan_hash,
            vec!["family:authorization".to_string()],
        );
        parent.target_requests = 7;
        parent.discovery_rounds = 3;
        parent.turns = 9;
        parent.no_progress_streak = 4;
        parent.terminal_reason = "连续多轮没有新增证据".to_string();
        parent.exhausted_contract_keys = vec!["idor|/api/orders".to_string()];
        parent.contract_outcomes = vec![("idor|/api/orders".to_string(), "exhausted".to_string())];
        parent.token_usage.total_tokens = 120_000;
        parent.covered_families = vec!["authorization".to_string()];
        parent.coverage_evidence = vec![CoverageEvidence {
            id: "cov-0001".to_string(),
            family: "authorization".to_string(),
            evidence_kind: "request".to_string(),
            result: "covered".to_string(),
            request_record_ids: vec!["req-0001".to_string()],
            ..Default::default()
        }];
        parent.persist(&db_path, "agent-scan", &context.target_url).unwrap();

        let mut next = context.clone();
        next.attempt_number = 2;
        next.resume = false;
        let started = NativeAgentState::for_attempt(&db_path, "agent-scan", &next, &evidence_hash, &plan_hash)
            .expect("a fresh attempt always starts");
        assert_eq!(started.resume_kind, AgentResumeKind::Fresh);
        assert_eq!(started.parent_attempt_number, None, "fresh records no parent state");
        assert_eq!(started.target_requests, 0);
        assert_eq!(started.discovery_rounds, 0);
        assert_eq!(started.turns, 0);
        assert_eq!(started.no_progress_streak, 0);
        assert!(started.terminal_reason.is_empty());
        assert!(started.exhausted_contract_keys.is_empty());
        assert!(started.contract_outcomes.is_empty());
        assert!(started.covered_families.is_empty());
        assert!(started.coverage_evidence.is_empty());
        assert_eq!(started.token_usage.total_tokens, 0);
        assert!(
            started.pending_queue.is_empty(),
            "a fresh attempt inherits no queue; the loop rebuilds it from the plan"
        );
        // §2.2: switching the backend is never achieved by destroying history. Only
        // the live working row is reset; both attempts keep their own plan rows.
        assert_eq!(started.attempt_number, 2);
        let connection = db::open(&db_path).unwrap();
        let attempts: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_scan_attempts WHERE scan_id='agent-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 2, "both attempt rows survive");
        assert_eq!(
            NativeAgentState::read(&db_path, "agent-scan", &context.target_url)
                .map(|state| state.attempt_number),
            None,
            "only the live working row is reset; it is rewritten when the attempt commits"
        );
    }
