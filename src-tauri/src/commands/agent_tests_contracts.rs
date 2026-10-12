    /// The attempt row the app writes for "继续未完成阶段" (resume) or
    /// "重新执行" (fresh). Lineage is read from this row, never from UI wording.
    fn seed_attempt_row(db_path: &Path, attempt_number: i64, execution_mode: &str) {
        let connection = db::open(db_path).unwrap();
        connection
            .execute(
                "INSERT OR REPLACE INTO sentinel_scan_attempts(scan_id,attempt_number,execution_mode) VALUES('agent-scan',?1,?2)",
                params![attempt_number, execution_mode],
            )
            .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn difference_record_does_not_follow_replaced_directory_or_hardlinked_file() {
        use std::os::unix::fs::symlink;

        let (_root, db_path) = temp_database("agent-diff-file-boundary");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let artifact = agent_write_diff_record(
            &context,
            1,
            &serde_json::json!({"left":{"requestId":"req-a"},"right":{"requestId":"req-b"}}),
        )
        .unwrap();
        let directory = context.target_dir.join(AGENT_DIFF_DIRECTORY);
        assert!(agent_read_diff_record(&context, &artifact).is_some());

        let other_link = context.target_dir.join("second-hardlink.json");
        std::fs::hard_link(directory.join(&artifact), &other_link).unwrap();
        assert!(agent_read_diff_record(&context, &artifact).is_none());
        std::fs::remove_file(&other_link).unwrap();

        let original = context.target_dir.join("original-diff");
        std::fs::rename(&directory, &original).unwrap();
        symlink(&original, &directory).unwrap();
        assert!(agent_read_diff_record(&context, &artifact).is_none());
        std::fs::remove_file(&directory).unwrap();
        std::fs::rename(&original, &directory).unwrap();
        assert!(agent_read_diff_record(&context, &artifact).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn broker_artifact_writes_refuse_symlinked_target_and_child_directories() {
        use std::os::unix::fs::symlink;

        let (root, db_path) = temp_database("agent-artifact-write-boundary");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        std::fs::create_dir_all(&context.target_dir).unwrap();
        let outside = root.join("outside-artifacts");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, context.target_dir.join(AGENT_DIFF_DIRECTORY)).unwrap();
        symlink(&outside, context.target_dir.join(AGENT_HTTP_DIRECTORY)).unwrap();

        let request = serde_json::json!({"method":"GET","url":"https://app.example.invalid"});
        let response = serde_json::json!({"status":200});
        assert!(agent_write_diff_record(&context, 1, &serde_json::json!({"left":1})).is_err());
        assert!(agent_write_http_record(&context, 1, &request, &response, b"private body").is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);

        let original = root.join("original-target");
        std::fs::rename(&context.target_dir, &original).unwrap();
        symlink(&outside, &context.target_dir).unwrap();
        assert!(agent_write_diff_record(&context, 2, &serde_json::json!({"left":2})).is_err());
        assert!(agent_write_http_record(&context, 2, &request, &response, b"private body").is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn broker_artifacts_refuse_symlinked_ancestor_even_when_target_is_real() {
        use std::os::unix::fs::symlink;

        let (root, db_path) = temp_database("agent-artifact-ancestor-boundary");
        let mut context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let actual = root.join("real-parent");
        std::fs::create_dir(&actual).unwrap();
        let alias = root.join("alias-parent");
        symlink(&actual, &alias).unwrap();
        context.target_dir = alias.join("target");

        let request = serde_json::json!({"method":"GET","url":"https://app.example.invalid"});
        let response = serde_json::json!({"status":200});
        assert!(agent_write_diff_record(&context, 1, &serde_json::json!({"left":1})).is_err());
        assert!(agent_write_http_record(&context, 1, &request, &response, b"private body").is_err());
        assert!(!actual.join("target").exists());

        let stable = root.join("stable-target");
        context.target_dir = stable.clone();
        let artifact = agent_write_diff_record(&context, 1, &serde_json::json!({"left":1})).unwrap();
        assert!(agent_read_diff_record(&context, &artifact).is_some());
        let replacement = root.join("moved-target");
        std::fs::rename(&stable, &replacement).unwrap();
        symlink(&replacement, &stable).unwrap();
        assert!(agent_read_diff_record(&context, &artifact).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_http_payload_reserves_its_slot_without_publishing_metadata() {
        let (_root, db_path) = temp_database("agent-artifact-orphan-slot");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let directory = context.target_dir.join(AGENT_HTTP_DIRECTORY);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("0001.body"), b"old partial body").unwrap();

        let request = serde_json::json!({"method":"GET","url":"https://app.example.invalid"});
        let response = serde_json::json!({"status":200});
        let artifact = agent_write_http_record(&context, 1, &request, &response, b"new body")
            .unwrap();
        assert_eq!(artifact, "0002.json");
        assert!(!directory.join("0001.json").exists());
        assert_eq!(std::fs::read(directory.join("0001.body")).unwrap(), b"old partial body");
        assert_eq!(std::fs::read(directory.join("0002.body")).unwrap(), b"new body");
    }

    fn frozen_plan_for(
        db_path: &Path,
        context: &AgentRunContext,
        attempt_number: i64,
    ) -> AgentExecutionPlan {
        let adaptive = AgentBudgetSettings::from_json(&serde_json::json!({}));
        agent_frozen_plan(
            db_path,
            "agent-scan",
            &context.route.url,
            &context.route,
            &context.environment,
            &adaptive,
            AgentBackendKind::Native,
            attempt_number,
        )
        .expect("a plan must resolve for this attempt")
    }

    /// Regression for the queue bug: a call that carries no path, url, actionKey
    /// or family must not consume anything, and a full URL must still match the
    /// queued `/path`.
    #[test]
    fn queue_entries_are_consumed_only_by_matching_work() {
        let (_root, db_path) = temp_database("agent-queue");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let queue = agent_initial_queue(&context);
        assert!(!queue.is_empty());
        let runtime = AgentToolRuntime::default();
        assert!(
            queue
                .iter()
                .filter(|key| runtime.queue_key_satisfied(&context, key))
                .count()
                == 0,
            "an untouched runtime must consume nothing"
        );

        let mut runtime = AgentToolRuntime::default();
        runtime
            .requests
            .push(trace_of("GET", "/api/orders", "authorization", ""));
        assert!(runtime.queue_key_satisfied(&context, "api:GET|/api/orders"));
        assert!(runtime.queue_key_satisfied(&context, "family:authorization"));
        assert!(
            !runtime.queue_key_satisfied(&context, "family:business_flow"),
            "an unrelated family must stay queued"
        );
        assert!(!runtime.queue_key_satisfied(&context, "api:POST|/api/orders"));
    }

    /// A contract is closed by a verdict or by spending its own attempt cap —
    /// never by a tool call that was rejected before it reached the target.
    #[test]
    fn refused_write_never_spends_or_closes_a_contract() {
        let (_root, db_path) = temp_database("agent-refused");
        seed_scan(&db_path, "agent-scan", "scanning");
        let mut context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        context.evidence["opportunities"] = serde_json::json!([
            {"category": "idor", "endpoint": "/api/orders", "maxAttempts": 2}
        ]);
        let mut runtime = AgentToolRuntime::default();
        let result = agent_execute_tool(
            &context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({
                "identity":"anonymous","method":"POST","url":"https://app.example.invalid/api/orders",
                "contractKey":"idor|/api/orders","attempt":1,
                "cleanup":"删除测试订单","recoveryCondition":"订单不再存在"
            }),
        ).model_view;
        assert_eq!(value_first(&result, &["code"]), "mutation_not_approved");
        assert_eq!(runtime.target_requests, 0, "a refused call costs no HTTP budget");
        assert!(runtime.contract_attempts.is_empty());
        assert!(runtime.contract_outcomes.is_empty());

        let mut state = NativeAgentState::fresh(1, AgentBackendKind::Native, "e", "p", Vec::new());
        native_sync(
            &context,
            &mut state,
            &runtime,
            &["contract:idor|/api/orders".to_string()],
        );
        assert!(state.completed_contract_keys.is_empty());
        assert!(state.exhausted_contract_keys.is_empty());
        assert_eq!(state.pending_queue.len(), 1, "the contract must still be owed");
    }

    /// A verdict cannot close a contract the run never executed.
    #[test]
    fn verdict_requires_executed_contract() {
        let (_root, db_path) = temp_database("agent-verdict");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let mut runtime = AgentToolRuntime::default();
        let result = agent_execute_tool(
            &context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-1","status":"rejected","contractKey":"idor|/api/orders"
            }),
        ).model_view;
        assert_eq!(value_first(&result, &["code"]), "contract_not_executed");
        assert!(runtime.contract_outcomes.is_empty());

        runtime
            .requests
            .push(trace_of("GET", "/api/orders", "authorization", "idor|/api/orders"));
        runtime.contract_attempts.insert("idor|/api/orders".into(), 1);
        let result = agent_execute_tool(
            &context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-1","status":"rejected","contractKey":"idor|/api/orders"
            }),
        ).model_view;
        assert_eq!(value_first(&result, &["status"]), "rejected");
        assert_eq!(
            runtime.contract_outcomes.get("idor|/api/orders").map(String::as_str),
            Some("rejected")
        );
    }

    /// §13.9: a pair of real responses is necessary but not sufficient for an
    /// authorization finding. Two personalized profile responses do not prove
    /// which business object belongs to whom.
    #[test]
    fn confirmed_finding_must_bind_two_executed_requests() {
        let harness = agent_harness(
            "confirm-binding",
            |request: String| {
                let cookie = request
                    .lines()
                    .find(|row| row.to_ascii_lowercase().starts_with("cookie:"))
                    .unwrap_or_default()
                    .to_string();
                let body = if cookie.contains("cookie-alpha") {
                    r#"{"userId":"u-7","roleId":"r-admin","role":"admin","canWrite":true}"#
                } else {
                    r#"{"userId":"u-8","roleId":"r-member","role":"member","canWrite":false}"#
                };
                (200, "application/json", body.to_string())
            },
            vec![
                AgentIdentity::scoped("session-a"),
                AgentIdentity::scoped("session-b"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&harness.db_path, "session-b", "cookie-beta", "Bearer beta");
        let mut runtime = AgentToolRuntime::default();
        let pair = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "compare_identities",
            &serde_json::json!({
                "leftIdentity":"session-a","rightIdentity":"session-b",
                "method":"GET","path":"/api/profile","family":"authorization","contractKey":"idor|/api/profile"
            }),
        )
        .model_view;
        assert_eq!(
            pair["materialDifference"],
            serde_json::json!(true),
            "roleId/role differ between the two accounts: {pair}"
        );
        let control = pair["left"]["requestId"].as_str().unwrap_or_default().to_string();
        let test = pair["right"]["requestId"].as_str().unwrap_or_default().to_string();
        let artifact = value_first(&pair, &["responseDifferenceArtifactId"]);
        assert!(!control.is_empty() && !test.is_empty() && !artifact.is_empty());
        assert_eq!(
            agent_family_sufficiency(&runtime, "authorization"),
            ("covered", "verified_identity_pair")
        );

        let finding = |control: &str, test: &str, artifact: &str| {
            serde_json::json!({
                "hypothesisKey":"h-idor","status":"confirmed","family":"authorization",
                "contractKey":"idor|/api/profile",
                "title":"越权读取他人配置","severity":"high","confidence":0.9,
                "controlRequestId":control,"testRequestId":test,"responseDifferenceArtifactId":artifact,
                "impact":"跨账号读取","reproductionSteps":"1. 用 B 身份重放 2. 观察 A 的字段",
                "counterEvidenceCheck":"已排除缓存与登录跳转"
            })
        };
        let proposed = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &finding(&control, &test, &artifact),
        )
        .model_view;
        assert_eq!(proposed["status"].as_str(), Some("insufficient_evidence"), "{proposed}");
        assert_eq!(proposed["missingEvidence"][0], "authorization_object_control_missing");
        assert_eq!(runtime.confirmed_findings, 0);
        let stored = findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE);
        assert!(stored.is_empty());

        for (label, mut relabelled) in [
            ("missing", finding(&control, &test, &artifact)),
            ("changed", finding(&control, &test, &artifact)),
        ] {
            relabelled["hypothesisKey"] = serde_json::json!(format!("h-family-{label}"));
            if label == "missing" {
                relabelled.as_object_mut().unwrap().remove("family");
            } else {
                relabelled["family"] = serde_json::json!("information_disclosure");
            }
            let refused = agent_execute_tool(
                &harness.context,
                &mut runtime,
                "record_hypothesis_result",
                &relabelled,
            )
            .model_view;
            assert_eq!(refused["status"], "insufficient_evidence", "{label}: {refused}");
            assert_eq!(refused["missingEvidence"][0], "confirmed_family_mismatch");
            assert_eq!(runtime.confirmed_findings, 0);
        }

        // Sharing the same coverage family must not let a model reuse this
        // profile pair as proof for a different business-object contract.
        runtime
            .contract_attempts
            .insert("idor|/api/orders/other".into(), 1);
        let mut wrong_contract = finding(&control, &test, &artifact);
        wrong_contract["hypothesisKey"] = serde_json::json!("h-other-object");
        wrong_contract["contractKey"] = serde_json::json!("idor|/api/orders/other");
        let refused = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &wrong_contract,
        )
        .model_view;
        assert_eq!(refused["status"].as_str(), Some("insufficient_evidence"));
        assert_eq!(
            refused["missingEvidence"][0].as_str(),
            Some("confirmed_request_unlinked")
        );
        assert_eq!(runtime.confirmed_findings, 0);

        // The old shape — three paragraphs of prose — is no longer accepted at all.
        let prose = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-prose","status":"confirmed","family":"authorization",
                "controlRequest":"GET /api/profile 身份 A","testRequest":"GET /api/profile 身份 B",
                "responseDifference":"B 看到了 A 的配置","impact":"跨账号","reproductionSteps":"重放"
            }),
        )
        .model_view;
        assert_eq!(prose["code"].as_str(), Some("invalid_arguments"), "{prose}");
        assert_eq!(runtime.confirmed_findings, 0, "the prose claim added nothing");

        for (label, control_id, test_id, artifact_id, expected) in [
            ("same request twice", control.clone(), control.clone(), artifact.clone(), "confirmed_requires_distinct_requests"),
            ("invented ids", "req-9999".to_string(), test.clone(), artifact.clone(), "confirmed_request_not_executed"),
            ("artifact from another pair", control.clone(), test.clone(), "diff-0009.json".to_string(), "difference_artifact_missing"),
        ] {
            let refused = agent_execute_tool(
                &harness.context,
                &mut runtime,
                "record_hypothesis_result",
                &finding(&control_id, &test_id, &artifact_id),
            )
            .model_view;
            assert_eq!(
                refused["status"].as_str(),
                Some("insufficient_evidence"),
                "{label} must downgrade: {refused}"
            );
            assert_eq!(
                refused["missingEvidence"][0].as_str(),
                Some(expected),
                "{label}: {refused}"
            );
            assert_eq!(refused["downgradedFrom"].as_str(), Some("confirmed"));
            assert_eq!(runtime.confirmed_findings, 0, "{label} must not count");
        }
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            0,
            "a profile contrast without object ownership must not be published"
        );
    }

    /// A valid contrast with a broken candidate store must not change the
    /// hypothesis to validated or count a verdict in the in-memory ledger.
    #[test]
    fn failed_finding_write_does_not_validate_graph_or_credit_verdict() {
        let harness = agent_harness(
            "finding-write-failure",
            |request: String| {
                let account = if request.contains("cookie-alpha") { "a" } else { "b" };
                (200, "application/json", format!(r#"{{"account":"{account}"}}"#))
            },
            vec![
                AgentIdentity::scoped("session-a"),
                AgentIdentity::scoped("session-b"),
            ],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "");
        seed_session(&harness.db_path, "session-b", "cookie-beta", "");
        let mut runtime = AgentToolRuntime::default();
        let pair = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "compare_identities",
            &serde_json::json!({
                "leftIdentity":"session-a", "rightIdentity":"session-b",
                "method":"GET", "path":"/api/status",
                "family":"information_disclosure", "contractKey":"disclosure|/api/status"
            }),
        )
        .model_view;
        assert_eq!(pair["materialDifference"], true, "{pair}");
        let connection = db::open(&harness.db_path).unwrap();
        connection.execute(
            "INSERT INTO investigation_hypotheses(project_id,scan_id,target_url,hypothesis_key,status) VALUES(9001,'agent-scan',?1,'h-write-fail','ready')",
            [&harness.context.target_url],
        ).unwrap();
        // A directly constructed test context writes the legacy result surface;
        // fail only that write while keeping the hypothesis table writable.
        connection.execute_batch(
            "CREATE TRIGGER reject_finding_write BEFORE INSERT ON sentinel_findings \
             BEGIN SELECT RAISE(ABORT, 'candidate store unavailable'); END;",
        ).unwrap();
        let verdict = agent_tool_record_hypothesis_result(
            &harness.context,
            &mut runtime,
            &serde_json::json!({
                "hypothesisKey":"h-write-fail", "status":"confirmed",
                "family":"information_disclosure", "contractKey":"disclosure|/api/status",
                "controlRequestId":pair["left"]["requestId"],
                "testRequestId":pair["right"]["requestId"],
                "responseDifferenceArtifactId":pair["responseDifferenceArtifactId"],
                "impact":"cross-account disclosure", "reproductionSteps":"compare the same endpoint"
            }),
        );
        assert_eq!(verdict["code"], "finding_persist_failed", "{verdict}");
        assert!(runtime.verdict_keys.is_empty());
        assert_eq!(runtime.last_progress.new_verdicts, 0);
        assert_eq!(runtime.confirmed_findings, 0);
        let graph_status: String = connection.query_row(
            "SELECT status FROM investigation_hypotheses WHERE scan_id='agent-scan' AND hypothesis_key='h-write-fail'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(graph_status, "ready", "a failed finding write cannot validate the hypothesis");
    }

    /// §10.2: a pair with no identity or parameter contrast, a cached reply or a
    /// login redirect cannot support a confirmation even when both ids are real.
    #[test]
    fn confirmed_finding_refuses_a_pair_without_a_contrast() {
        let harness = agent_harness(
            "confirm-contrast",
            coverage_site,
            vec![AgentIdentity::anonymous()],
        );
        let base = harness.context.target_url.clone();
        let mut runtime = AgentToolRuntime::default();
        let first = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{base}/api/orders"),"family":"authorization","contractKey":"idor|/api/orders"}),
        )
        .model_view;
        let second = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{base}/api/orders"),"family":"authorization","contractKey":"idor|/api/orders"}),
        )
        .model_view;
        let control = value_first(&first, &["requestId"]);
        let test = value_first(&second, &["requestId"]);
        assert!(!control.is_empty() && control != test, "{first} / {second}");
        // A difference record has to exist, so build one from the two responses.
        let record = agent_response_field_difference(&first, &second);
        let artifact = agent_write_diff_record(
            &harness.context,
            runtime.target_requests,
            &serde_json::json!({
                "left": {"requestId": control},
                "right": {"requestId": test},
                "materialDifference": agent_difference_is_material(&record, 200, 200),
                "fieldDifference": record,
            }),
        )
        .unwrap();
        let refused = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-flat","status":"confirmed","family":"authorization",
                "contractKey":"idor|/api/orders",
                "controlRequestId":control,"testRequestId":test,"responseDifferenceArtifactId":artifact,
                "impact":"读别人的单","reproductionSteps":"重放两次"
            }),
        )
        .model_view;
        assert_eq!(
            refused["status"].as_str(),
            Some("insufficient_evidence"),
            "two identical anonymous requests prove nothing: {refused}"
        );
        assert_eq!(runtime.confirmed_findings, 0);
    }

    #[test]
    fn identity_pair_rejects_different_object_values_even_with_same_parameter_names() {
        let mut runtime = AgentToolRuntime::default();
        for (id, identity) in [("req-a", "session-a"), ("req-b", "session-b")] {
            runtime.record_request(AgentRequestTrace {
                id: id.into(),
                method: "GET".into(),
                origin: "https://example.test".into(),
                path: "/api/orders".into(),
                identity: identity.into(),
                status: 200,
                artifact_id: format!("{id}.json"),
                structure_hash: format!("hash-{id}"),
                ..Default::default()
            });
        }
        let left = serde_json::json!({"requestId":"req-a","url":"https://example.test/api/orders?id=owned&view=full"});
        let wrong_object = serde_json::json!({"requestId":"req-b","url":"https://example.test/api/orders?view=full&id=other"});
        assert!(agent_ab_pair_evidence(&runtime, &left, &wrong_object)
            .unwrap_err()
            .contains("参数值不一致"));
        let same_object = serde_json::json!({"requestId":"req-b","url":"https://example.test/api/orders?view=full&id=owned"});
        assert!(agent_ab_pair_evidence(&runtime, &left, &same_object).is_ok());
    }

    #[test]
    fn authorization_cannot_confirm_two_direct_replays_as_an_identity_pair() {
        let harness = agent_harness(
            "auth-unbound-replay",
            coverage_site,
            vec![AgentIdentity::scoped("session-a"), AgentIdentity::scoped("session-b")],
        );
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&harness.db_path, "session-b", "cookie-beta", "Bearer beta");
        let mut runtime = AgentToolRuntime::default();
        let url = format!("{}/api/orders?id=42", harness.context.target_url);
        let left = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"session-a","method":"GET","url":url,"family":"authorization","contractKey":"idor|/api/orders"}),
        ).model_view;
        let right = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "replay_http",
            &serde_json::json!({"identity":"session-b","method":"GET","url":url,"family":"authorization","contractKey":"idor|/api/orders"}),
        ).model_view;
        let control = value_first(&left, &["requestId"]);
        let test = value_first(&right, &["requestId"]);
        assert!(!control.is_empty() && !test.is_empty(), "{left} / {right}");
        let artifact = agent_write_diff_record(
            &harness.context,
            runtime.target_requests,
            &serde_json::json!({
                "left":{"requestId":control}, "right":{"requestId":test},
                "contractKey":"idor|/api/orders", "method":"GET", "materialDifference":true,
            }),
        ).unwrap();
        let verdict = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "record_hypothesis_result",
            &serde_json::json!({
                "hypothesisKey":"h-replay","status":"confirmed","family":"authorization",
                "contractKey":"idor|/api/orders","controlRequestId":control,"testRequestId":test,
                "responseDifferenceArtifactId":artifact,"impact":"cross-account access",
                "reproductionSteps":"compare responses",
            }),
        ).model_view;
        assert_eq!(verdict["status"], "insufficient_evidence", "{verdict}");
        assert_eq!(verdict["missingEvidence"][0], "authorization_pair_not_comparable");
        assert_eq!(runtime.confirmed_findings, 0);
    }


    /// Resume must not hand the run a second HTTP budget.
    #[test]
    fn restore_keeps_spent_budget_and_closed_contracts() {
        let (_root, db_path) = temp_database("agent-restore");
        seed_scan(&db_path, "agent-scan", "scanning");
        let context = test_context(
            &db_path,
            "https://app.example.invalid",
            vec![AgentIdentity::anonymous()],
        );
        let seeded = NativeAgentState {
            target_requests: 7,
            discovery_rounds: 2,
            observed_requests: vec![trace_of("GET", "/api/orders", "authorization", "idor|/api/orders")],
            contract_attempts: vec![("idor|/api/orders".to_string(), 1)],
            contract_outcomes: vec![("idor|/api/orders".to_string(), "rejected".to_string())],
            confirmed_findings: 3,
            verdict_keys: vec!["h-1:rejected".to_string()],
            ..state(4, 5_000, 1)
        };
        let runtime = AgentToolRuntime::restore(&seeded);
        assert_eq!(runtime.target_requests, 7);
        assert_eq!(runtime.discovery_rounds, 2);
        assert_eq!(runtime.confirmed_findings, 3);
        assert_eq!(runtime.attempts_for("idor|/api/orders"), 1);
        assert!(runtime.families.contains("authorization"));
        assert!(runtime.queue_key_satisfied(&context, "contract:idor|/api/orders"));
        assert!(runtime.queue_key_satisfied(&context, "api:GET|/api/orders"));
        assert!(runtime.touched("get", "/api/orders"));

        let mut round_trip = seeded.clone();
        round_trip.completed_contract_keys = round_trip.completed_contracts();
        let text = round_trip.as_json();
        let read = NativeAgentState::from_json(&text).expect("the ledger must round-trip");
        assert_eq!(read.target_requests, 7);
        assert_eq!(read.observed_requests, seeded.observed_requests);
        assert_eq!(read.completed_contract_keys, vec!["idor|/api/orders".to_string()]);
        assert_eq!(read.attempts_for("idor|/api/orders"), 1);
    }

    /// Rotating ids, timestamps and per-account objects are not a difference.
    #[test]
    fn identity_noise_is_not_a_material_difference() {
        let left = field_summary(
            r#"{"requestId":"a1","createdAt":"2026-09-19T00:00:00","token":"eyJhbGciOiJI.eyJzdWIiOiIx.SflKxwRJ","code":0,"items":[{"id":1,"role":"member"}]}"#,
        );
        let right = field_summary(
            r#"{"requestId":"b2","createdAt":"2026-09-19T00:00:07","token":"eyJhbGciOiJI.eyJzdWIiOiIy.XkL9m2Qz","code":0,"items":[{"id":9,"role":"member"}]}"#,
        );
        let diff = agent_response_field_difference(&left, &right);
        assert!(
            !agent_difference_is_material(&diff, 200, 200),
            "only volatile and ownership fields differ: {diff}"
        );
        assert!(diff
            .get("valueDiffers")
            .and_then(JsonValue::as_array)
            .map(|rows| rows.is_empty())
            .unwrap_or(false));
    }

    /// A field one identity cannot see, a business code split or a status split
    /// is a real difference.
    #[test]
    fn identity_privilege_change_is_a_material_difference() {
        let member = field_summary(r#"{"code":0,"role":"member"}"#);
        let admin = field_summary(r#"{"code":403,"role":"admin","permissions":["write"]}"#);
        let diff = agent_response_field_difference(&member, &admin);
        assert!(agent_difference_is_material(&diff, 200, 200), "{diff}");
        assert!(agent_difference_is_material(&JsonValue::Null, 200, 403));
        assert!(!agent_difference_is_material(&JsonValue::Null, 200, 200));
    }

    /// §13.7: a field whose name ends in `id` is not automatically ownership noise.
    /// `roleId`/`tenantId`/`departmentId`/`permissionId` are the authorization
    /// decision itself; `userId`/`accountId` stay in the ownership bucket that
    /// only confirms whose object each response is.
    #[test]
    fn authorization_fields_are_material_and_ownership_stays_separate() {
        let member = field_summary(
            r#"{"userId":"7","accountId":"a-7","roleId":"r-member","role":"member","departmentId":"d1","tenantId":"t1","permissionId":"p-read"}"#,
        );
        let admin = field_summary(
            r#"{"userId":"8","accountId":"a-8","roleId":"r-admin","role":"admin","departmentId":"d2","tenantId":"t1","permissionId":"p-write"}"#,
        );
        let diff = agent_response_field_difference(&member, &admin);
        let paths = |key: &str| -> Vec<String> {
            diff.get(key)
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|row| row.get("path").and_then(JsonValue::as_str))
                .map(str::to_string)
                .collect()
        };
        let authorization = paths("materialAuthorizationDiffs");
        for field in ["roleId", "role", "departmentId", "permissionId"] {
            assert!(
                authorization.iter().any(|path| path == field),
                "{field} must be a material authorization difference, got {authorization:?}"
            );
        }
        let ownership = paths("subjectDiffs");
        assert!(
            ownership.contains(&"userId".to_string())
                && ownership.contains(&"accountId".to_string()),
            "per-account ids stay visible as ownership, got {ownership:?}"
        );
        assert!(
            paths("valueDiffers").is_empty(),
            "no business field changed here: {diff}"
        );
        assert_eq!(
            diff.get("ownershipOnly").and_then(JsonValue::as_array),
            None,
            "the old catch-all bucket is gone: {diff}"
        );
        assert!(
            !authorization.contains(&"tenantId".to_string()),
            "an unchanged authorization field must not be reported as a diff"
        );
        assert!(agent_difference_is_material(&diff, 200, 200));
        // Ownership alone is never material.
        let only_owner_left = field_summary(r#"{"userId":"7","amount":"num:10"}"#);
        let only_owner_right = field_summary(r#"{"userId":"8","amount":"num:10"}"#);
        let owner_diff =
            agent_response_field_difference(&only_owner_left, &only_owner_right);
        assert!(
            !agent_difference_is_material(&owner_diff, 200, 200),
            "a different subject id is not an authorization difference: {owner_diff}"
        );
        // A field only one identity can see, when it carries permissions, is the
        // strongest form of the same signal.
        let no_perms = field_summary(r#"{"role":"member"}"#);
        let with_perms = field_summary(r#"{"role":"member","permissions":["write"]}"#);
        let presence = agent_response_field_difference(&no_perms, &with_perms);
        assert!(
            presence["materialAuthorizationDiffs"]
                .as_array()
                .map(|rows| rows.iter().any(|row| {
                    row["path"]
                        .as_str()
                        .unwrap_or_default()
                        .starts_with("permissions")
                        && row["left"] == "absent"
                }))
                .unwrap_or(false),
            "a permission field only one side can see must be material: {presence}"
        );
    }

    /// §8: an A/B claim needs two complete records from this chain. A side that
    /// was never collected is `insufficient_evidence`, not a difference.
    #[test]
    fn ab_comparison_refuses_an_incomplete_pair() {
        let harness = agent_harness("ab-pair", mock_site, vec![
            AgentIdentity::scoped("session-a"),
            AgentIdentity::scoped("session-b"),
        ]);
        seed_session(&harness.db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&harness.db_path, "session-b", "cookie-beta", "Bearer beta");
        let mut runtime = AgentToolRuntime::default();
        let missing = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "compare_identities",
            &serde_json::json!({
                "leftIdentity":"session-a","rightIdentity":"ghost","method":"GET","path":"/api/orders","family":"authorization"
            }),
        );
        assert_eq!(
            missing.model_view["status"].as_str(),
            Some("insufficient_evidence"),
            "{}",
            missing.model_view
        );
        assert!(!runtime.families.contains("authorization"));
        assert_eq!(
            runtime.target_requests, 0,
            "an unresolvable identity must not spend either side's request"
        );

        let complete = agent_execute_tool(
            &harness.context,
            &mut runtime,
            "compare_identities",
            &serde_json::json!({
                "leftIdentity":"session-a","rightIdentity":"session-b","method":"GET","path":"/api/orders","family":"authorization"
            }),
        );
        assert_eq!(
            complete.model_view["code"].as_str(),
            None,
            "a real pair must compare: {}",
            complete.model_view
        );
        let left = &complete.model_view["left"];
        let right = &complete.model_view["right"];
        for (side, row) in [("left", left), ("right", right)] {
            assert_ne!(row["requestId"].as_str().unwrap_or_default(), "", "{side}");
            assert_ne!(row["artifactId"].as_str().unwrap_or_default(), "", "{side}");
            assert_ne!(row["structureHash"].as_str().unwrap_or_default(), "", "{side}");
            assert!(row["status"].as_i64().unwrap_or_default() > 0, "{side}");
        }
        assert_ne!(left["requestId"], right["requestId"]);
        assert_ne!(left["artifactId"], right["artifactId"]);
        assert_eq!(runtime.target_requests, 2, "the pair spent exactly two requests");
    }
