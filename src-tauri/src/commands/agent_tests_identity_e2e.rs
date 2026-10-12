    /// Two accounts that see exactly the same shape and the same values.
    fn equal_role_site(request: String) -> (u16, &'static str, String) {
        let _ = &request;
        (
            200,
            "application/json",
            r#"{"userId":"self","roleId":"r-member","role":"member","canWrite":false}"#.to_string(),
        )
    }

    /// The same endpoint answering differently for the two accounts: one of them
    /// holds an authorization field the other does not.
    fn unequal_role_site(request: String) -> (u16, &'static str, String) {
        let cookie = request
            .lines()
            .find(|row| row.to_ascii_lowercase().starts_with("cookie:"))
            .unwrap_or_default()
            .to_string();
        let body = if cookie.contains("cookie-alpha") {
            r#"{"userId":"u-7","roleId":"r-admin","role":"admin","permissions":["write"],"tenantId":"t-1"}"#
        } else {
            r#"{"userId":"u-8","roleId":"r-member","role":"member","tenantId":"t-1"}"#
        };
        (200, "application/json", body.to_string())
    }

    /// §14.3: two accounts with equal rights produce a comparison and no finding.
    #[test]
    fn e2e_two_equal_accounts_close_without_a_finding() {
        let mut harness = fresh_single_production_harness_with_sessions(
            "e2e-equal", equal_role_site,
            &[("session-a","cookie-alpha","Bearer alpha"),("session-b","cookie-beta","Bearer beta")],
        );
        freeze_fresh_single_production_harness(&mut harness);
        let url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "compare_identities",
                        serde_json::json!({"leftIdentity":"session-a","rightIdentity":"session-b","method":"GET","path":"/api/profile","family":"authorization","contractKey":"idor|/api/profile"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "record_hypothesis_result",
                        serde_json::json!({"hypothesisKey":"h-equal","status":"insufficient_evidence","family":"authorization","contractKey":"idor|/api/profile"}),
                    )],
                    1_100,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({"coverage": closing_ledger(&["authorization"]), "stopReason": "覆盖队列已收口"}),
                    )],
                    1_300,
                ),
            ],
        );
        let owned = execute_fresh_single_production_harness(&mut harness);
        let outcome = &owned.outcome;
        assert!(
            !matches!(outcome, AgentTargetOutcome::Failed(_)),
            "an equal-rights pair is normal work, never a failure: {:?}",
            outcome.detail()
        );
        let status = match &outcome {
            AgentTargetOutcome::Completed(_) => "completed",
            AgentTargetOutcome::BoundedCompleted(_) => "completed_with_gaps",
            AgentTargetOutcome::Incomplete(_) => "paused",
            other => panic!("unexpected outcome: {:?}", other.detail()),
        };
        let mut tally = AgentPipelineTally::default();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path, &harness.context.scan_id, &harness.context.route,
            &owned, &mut tally,
        ));
        assert_fresh_single_surface(&harness, &url, status, 2, 0);
        assert_fresh_single_identity_wire(&harness);
        let state = NativeAgentState::read(&harness.db_path, "agent-scan", &url).unwrap();
        assert!(
            !state.covered_families.contains(&"authorization".to_string())
                || state.coverage_evidence.iter().any(|entry| {
                    entry.family == "authorization"
                        && entry.evidence_kind == "identity_pair"
                        && entry.result == "partial"
                }),
            "no material difference must not read as a proven boundary: {:?}",
            state.coverage_evidence
        );
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            0,
            "equal rights confirm nothing"
        );
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            2,
            "both sides of the pair reached the target once"
        );
    }

    /// §14.4: different account roles on the same self-profile endpoint do not
    /// establish object ownership or an unauthorized cross-account read.
    #[test]
    fn e2e_personalized_permission_difference_does_not_confirm_idor() {
        let mut harness = fresh_single_production_harness_with_sessions(
            "e2e-diff", unequal_role_site,
            &[("session-a","cookie-alpha","Bearer alpha"),("session-b","cookie-beta","Bearer beta")],
        );
        freeze_fresh_single_production_harness(&mut harness);
        let url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "compare_identities",
                        serde_json::json!({"leftIdentity":"session-a","rightIdentity":"session-b","method":"GET","path":"/api/profile","family":"authorization","contractKey":"idor|/api/profile"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "record_hypothesis_result",
                        serde_json::json!({
                            "hypothesisKey":"h-tenant","status":"confirmed","family":"authorization",
                            "contractKey":"idor|/api/profile","title":"成员账号可见管理员权限字段",
                            "severity":"medium","confidence":0.8,
                            "controlRequestId":"req-0001","testRequestId":"req-0002",
                            "responseDifferenceArtifactId":"diff-0002.json",
                            "impact":"低权账号可读取高权配置","reproductionSteps":"用 session-b 重放同一请求并比较 permissions"
                        }),
                    )],
                    1_100,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({"coverage": closing_ledger(&["authorization"]), "stopReason": "覆盖队列已收口"}),
                    )],
                    1_300,
                ),
            ],
        );
        let owned = execute_fresh_single_production_harness(&mut harness);
        let outcome = &owned.outcome;
        let status = match &outcome {
            AgentTargetOutcome::Completed(_) => "completed",
            AgentTargetOutcome::BoundedCompleted(_) => "completed_with_gaps",
            AgentTargetOutcome::Incomplete(_) => "paused",
            other => panic!("unexpected outcome: {:?}", other.detail()),
        };
        let mut tally = AgentPipelineTally::default();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path, &harness.context.scan_id, &harness.context.route,
            &owned, &mut tally,
        ));
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            0,
            "the bound pair has no verified object ownership control"
        );
        assert_fresh_single_surface(&harness, &url, status, 2, 0);
        assert_fresh_single_identity_wire(&harness);
    }

    include!("agent_tests_missing_side_original.rs");

    /// §14: what every mock case has to leave behind on the result surfaces —
    /// target state, attempt lineage, spent requests, tokens, coverage evidence,
    /// findings and the fields the panel reads. §12 adds the last block: the run
    /// ledger is the authority and the checkpoint row is its serialization, so the
    /// two must agree instead of each keeping its own idea of the state.
    fn assert_agent_surface_with_original_usage(
        harness: &AgentHarness,
        url: &str,
        expected_status: &str,
        expected_target_requests: i64,
        expected_findings: usize,
        original_usage: Option<(i64, i64)>,
    ) {
        let connection = db::open(&harness.db_path).unwrap();
        let stored: String = connection
            .query_row(
                "SELECT status FROM sentinel_targets WHERE scan_id='agent-scan' AND url=?1",
                [url],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, expected_status, "the target has one terminal state");
        let state = NativeAgentState::read(&harness.db_path, "agent-scan", url)
            .expect("the attempt leaves a checkpoint");
        assert_eq!(
            state.target_requests, expected_target_requests,
            "the HTTP budget the run really spent survives"
        );
        assert!(state.token_usage.total_tokens > 0, "tokens must be accounted");
        assert_eq!(
            state.confirmed_findings as usize, expected_findings,
            "the ledger counts exactly the confirmed findings"
        );
        let run = read_run_row(&harness.db_path);
        assert_eq!(run.rows, 1, "one coordinator run per scan");
        let (requests, tokens) = original_usage.unwrap_or((run.used_requests, run.used_tokens));
        assert_eq!(requests, state.token_usage.model_requests);
        assert_eq!(tokens, state.token_usage.total_tokens);
        let coverage = findings_for(&harness.db_path, AGENT_COVERAGE_STAGE);
        assert_eq!(coverage.len(), 1, "one coverage ledger per target");
        let ledger = json(coverage[0].1.clone());
        for key in [
            "coveredFamilies",
            "uncoveredFamilies",
            "coverageEvidence",
            "confirmedFindings",
            "targetRequests",
            "totalTokens",
        ] {
            assert!(ledger.get(key).is_some(), "the summary must carry {key}: {ledger}");
        }
        assert_eq!(
            ledger["confirmedFindings"].as_i64(),
            Some(expected_findings as i64)
        );
        assert_eq!(
            findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).len(),
            expected_findings
        );
        let evidence: Vec<JsonValue> = ledger["coverageEvidence"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for row in evidence
            .iter()
            .filter(|row| row["result"] == "covered")
        {
            assert!(
                !row["requestRecordIds"]
                    .as_array()
                    .map(|ids| ids.is_empty())
                    .unwrap_or(true),
                "a covered entry must cite executed requests: {row}"
            );
        }
        let snapshot: String = connection
            .query_row(
                "SELECT snapshot_json FROM agent_snapshots WHERE run_id=(SELECT id FROM agent_runs WHERE scan_id='agent-scan' ORDER BY rowid DESC LIMIT 1)",
                [],
                |row| row.get(0),
            )
            .unwrap_or_default();
        let snapshot = json(snapshot);
        if !snapshot.is_null() {
            assert_eq!(
                snapshot["targetRequests"].as_i64(),
                Some(state.target_requests),
                "the canonical snapshot and the checkpoint must agree"
            );
            assert_eq!(
                snapshot["usedTokens"].as_i64(),
                Some(state.token_usage.total_tokens)
            );
        }
    }

    include!("agent_tests_seven_terminal_original.rs");

    /// Case 16: the model dies mid-run. The evidence stays, the checkpoint stays
    /// resumable and nothing is reported as a failure of the target itself.
    #[test]
    fn e2e_model_interruption_keeps_the_run_resumable() {
        let harness = agent_harness("e2e-model-down", mock_site, vec![AgentIdentity::anonymous()]);
        let AgentHarness {
            db_path,
            mut context,
            site_seen,
            ..
        } = harness;
        let target_url = context.target_url.clone();
        let (model_port, _seen, _stop) = spawn_endpoint(std::sync::Arc::new(|request| {
            if request.contains("POST /v1/chat/completions") {
                return (
                    500,
                    "application/json",
                    "{\"error\":{\"message\":\"upstream unavailable\"}}".to_string(),
                );
            }
            (404, "text/plain", "unexpected path".to_string())
        }));
        context.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        let outcome = NativeAgentBackend.execute(&context);
        assert!(
            matches!(outcome, AgentTargetOutcome::Incomplete(_)),
            "a transport stall must stay retryable: {:?}",
            outcome.detail()
        );
        let stored = NativeAgentState::read(&db_path, "agent-scan", &target_url).unwrap();
        assert!(
            stored.terminal_reason.is_empty(),
            "an interrupted run must not look finished"
        );
        assert_eq!(stored.target_requests, 0);
        assert_eq!(site_seen.lock().unwrap().len(), 0);
    }
