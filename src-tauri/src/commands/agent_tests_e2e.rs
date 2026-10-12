    /// Case 1 + 12 end to end: an anonymous SPA gets a real model tool loop,
    /// HTTP verification, coverage close-out and persistence — without Strix.
    #[test]
    fn e2e_anonymous_spa_completes_without_strix() {
        let mut harness = fresh_single_production_harness("e2e-anon", mock_site);
        freeze_fresh_single_production_harness(&mut harness);
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[("inspect_evidence", serde_json::json!({"kind": "api"}))],
                    800,
                ),
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders"),"family":"authorization"}),
                    )],
                    1_200,
                ),
                model_round(
                    &[(
                        "record_hypothesis_result",
                        serde_json::json!({"hypothesisKey":"h-anon-orders","status":"insufficient_evidence","family":"authorization"}),
                    )],
                    1_400,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({
                            "coverage": [
                                {"family":"authorization","status":"covered","reason":"已比较匿名访问"},
                                {"family":"business_flow","status":"not_applicable","reason":"缺少登录身份，未执行下单流程"},
                                {"family":"information_disclosure","status":"not_applicable","reason":"本轮未覆盖该族"},
                                {"family":"error_handling","status":"not_applicable","reason":"本轮未覆盖该族"},
                                {"family":"authentication_session","status":"not_applicable","reason":"本轮未覆盖该族"},
                                {"family":"input_reflection_xss","status":"not_applicable","reason":"本轮未覆盖该族"},
                                {"family":"hidden_interface_discovery","status":"not_applicable","reason":"本轮未覆盖该族"}
                            ],
                            "stopReason":"覆盖队列已收口"
                        }),
                    )],
                    1_600,
                ),
            ],
        );
        let owned = execute_fresh_single_production_harness(&mut harness);
        let outcome = &owned.outcome;
        // Early finish_target is refused while families and budget remain. The
        // script then goes quiet, so the attempt ends on the no-progress window
        // as bounded work, not as a pause waiting for a person to click resume.
        assert!(
            matches!(outcome, AgentTargetOutcome::BoundedCompleted(_)),
            "unexpected outcome: {:?}",
            outcome.detail()
        );
        assert!(
            !outcome.detail().contains("可继续同一尝试"),
            "{:?}",
            outcome.detail()
        );
        assert!(!findings_for(&harness.db_path, AGENT_EVIDENCE_STAGE).is_empty());
        let resumed = NativeAgentState::read(&harness.db_path, "agent-scan", &target_url)
            .expect("the checkpoint must survive");
        assert!(!resumed.terminal_reason.is_empty(), "a bounded stop records why it ended");
        assert_eq!(resumed.target_requests, 1, "the spent HTTP request is kept");
        // §9.3: one anonymous request is work on the authorization family, it is
        // not coverage. The ledger must say so instead of closing the family.
        assert!(
            !resumed.covered_families.contains(&"authorization".to_string()),
            "an anonymous-only request cannot cover authorization"
        );
        assert!(
            resumed.coverage_evidence.iter().any(|entry| {
                entry.family == "authorization"
                    && entry.result == "partial"
                    && entry.reason_code == "anonymous_only"
                    && !entry.request_record_ids.is_empty()
            }),
            "{:?}",
            resumed.coverage_evidence
        );
        let coverage = findings_for(&harness.db_path, AGENT_COVERAGE_STAGE);
        assert_eq!(coverage.len(), 1);
        let ledger = json(coverage[0].1.clone());
        let gaps = ledger
            .get("uncoveredFamilies")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        let business_flow = gaps
            .iter()
            .find(|row| value_first(row, &["family"]) == "business_flow")
            .expect("the reported partial family must stay visible");
        assert_eq!(
            value_first(business_flow, &["reason"]),
            "缺少登录身份，未执行下单流程"
        );
        // The actual user entry registers the original Single Root before SDK
        // dispatch. Its four model dimensions must match the checkpoint and
        // the provider calls, checked by assert_fresh_single_surface below.
        let connection = db::open(&harness.db_path).unwrap();
        let total_tokens: i64 = connection
            .query_row(
                "SELECT total_tokens FROM sentinel_scans WHERE id='agent-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            total_tokens > 4_000,
            "usage must be ledger-backed: {total_tokens}"
        );
        assert!(
            harness.model_seen.lock().unwrap().len() > 4,
            "提前收口被拒绝后必须继续请求模型"
        );
        let requests = harness.site_seen.lock().unwrap().join("\n");
        assert!(requests.contains("GET /api/orders"), "{requests}");
        assert!(harness.context.target_dir.join("agent-http").is_dir());
        assert!(
            harness.context.target_dir.starts_with(&harness.root),
            "each run owns scratch space under its own attempt directory (§7)"
        );
        let model_requests = harness.model_seen.lock().unwrap();
        assert!(model_requests.len() > 4, "every scripted round must be served, then the loop continues");
        assert!(model_requests
            .iter()
            .all(|row| row.contains("tool_choice") && row.contains("\"tools\"")));
        assert!(model_requests[3].contains("finish_target"));
        drop(model_requests);
        let mut tally = AgentPipelineTally::default();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path,
            &harness.context.scan_id,
            &harness.context.route,
            &owned,
            &mut tally,
        ));
        // §14.1: an anonymous SPA ends as resumable work with zero findings, and
        // every result surface agrees on that.
        assert_fresh_single_surface(&harness, &target_url, "completed_with_gaps", 1, 0);
    }

    include!("agent_tests_native_policy_original.rs");

    /// Case 3 end to end: nothing found is still a normal terminal state.
    #[test]
    fn e2e_no_finding_closes_normally() {
        let mut harness = fresh_single_production_harness_with_sessions(
            "e2e-nofind",
            mock_site,
            &[("session-a", "cookie-alpha", "Bearer alpha")],
        );
        freeze_fresh_single_production_harness(&mut harness);
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "compare_identities",
                        serde_json::json!({"leftIdentity":"session-a","rightIdentity":"anonymous","method":"GET","path":"/api/orders","family":"authorization"}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "finish_target",
                        serde_json::json!({
                            "coverage": closing_ledger(&["authorization"]),
                            "stopReason":"证据已穷尽"
                        }),
                    )],
                    1_000,
                ),
            ],
        );
        let owned = execute_fresh_single_production_harness(&mut harness);
        let outcome = &owned.outcome;
        assert!(
            matches!(outcome, AgentTargetOutcome::Completed(_)),
            "{:?}",
            outcome.detail()
        );
        let completion = outcome.completion().unwrap();
        assert_eq!(completion.confirmed_findings, 0);
        assert!(completion.uncovered_families.is_empty());
        assert!(completion.covered_families.contains(&"authorization".to_string()));
        assert!(findings_for(&harness.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
        let requests = harness.site_seen.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|wire| wire.starts_with("GET /api/orders HTTP/")));
        let authenticated: Vec<_> = requests.iter().filter(|wire| wire.contains("cookie-alpha")).collect();
        assert_eq!(authenticated.len(), 1);
        assert!(authenticated[0].contains("Bearer alpha"));
        let anonymous: Vec<_> = requests.iter().filter(|wire| !wire.contains("cookie-alpha")).collect();
        assert_eq!(anonymous.len(), 1);
        assert!(!anonymous[0].to_ascii_lowercase().contains("cookie:"));
        assert!(!anonymous[0].to_ascii_lowercase().contains("authorization:"));
        drop(requests);
        let models = harness.model_seen.lock().unwrap().join("\n");
        assert!(!models.contains("cookie-alpha") && !models.contains("Bearer alpha"));
        let mut tally = AgentPipelineTally::default();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path,
            &harness.context.scan_id,
            &harness.context.route,
            &owned,
            &mut tally
        ));
        assert_eq!(tally.completed, 1);
        assert_fresh_single_surface(&harness, &target_url, "completed", 2, 0);
        let connection = db::open(&harness.db_path).unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM sentinel_targets WHERE scan_id='agent-scan'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "completed");
        let before = single_finally_physical(&connection);
        let calls = harness.model_seen.lock().unwrap().len();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path, &harness.context.scan_id, &harness.context.route,
            &owned, &mut tally,
        ));
        assert_eq!(single_finally_physical(&connection), before);
        assert_eq!(tally.counted(), 1);
        assert_eq!(harness.model_seen.lock().unwrap().len(), calls);
        assert_eq!(harness.site_seen.lock().unwrap().len(), 2);
    }

    /// Case 6/7 end to end: identities never share credentials, and a
    /// single-identity task cannot invent an A/B pair.
    #[test]
    fn e2e_single_identity_and_pair_isolation() {
        let (_root, db_path) = temp_database("agent-e2e-identities");
        seed_scan(&db_path, "agent-scan", "scanning");
        seed_session(&db_path, "session-a", "cookie-alpha", "Bearer alpha");
        seed_session(&db_path, "session-b", "cookie-beta", "Bearer beta");
        let (site_port, site_seen, _site_stop) =
            spawn_endpoint(std::sync::Arc::new(mock_site));
        let target_url = format!("http://127.0.0.1:{site_port}");
        seed_target(&db_path, &target_url);

        let mut single =
            test_context(&db_path, &target_url, vec![AgentIdentity::scoped("session-a")]);
        single.target_dir = std::env::temp_dir().canonicalize().unwrap()
            .join(format!("oviraptor-single-{}", Uuid::new_v4()));
        single.log_path = single.target_dir.join("runner.log");
        fs::create_dir_all(&single.target_dir).unwrap();
        let (model_port, _stop, _seen) = spawn_model(vec![model_round(
            &[(
                "compare_identities",
                serde_json::json!({"leftIdentity":"session-a","rightIdentity":"session-a","method":"GET","path":"/api/orders"}),
            )],
            700,
        )]);
        single.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        let outcome = NativeAgentBackend.execute(&single);
        assert!(
            !matches!(outcome, AgentTargetOutcome::Failed(_)),
            "{:?}",
            outcome.detail()
        );
        assert!(NativeAgentState::read(&db_path, "agent-scan", &target_url).is_some());

        let pair = vec![
            AgentIdentity::scoped("session-a"),
            AgentIdentity::scoped("session-b"),
        ];
        let mut both = test_context(&db_path, &target_url, pair.clone());
        both.target_dir = std::env::temp_dir().canonicalize().unwrap()
            .join(format!("oviraptor-pair-{}", Uuid::new_v4()));
        both.log_path = both.target_dir.join("runner.log");
        fs::create_dir_all(&both.target_dir).unwrap();
        let (model_port, _stop, _seen) = spawn_model(vec![model_round(
            &[(
                "compare_identities",
                serde_json::json!({"leftIdentity":"session-a","rightIdentity":"session-b","method":"GET","path":"/api/orders","family":"authorization"}),
            )],
            700,
        )]);
        both.environment.api_base = format!("http://127.0.0.1:{model_port}/v1");
        let outcome = NativeAgentBackend.execute(&both);
        // The script stops after one comparison. The loop keeps going until the
        // no-progress window, then closes with the gaps still named.
        assert!(
            matches!(outcome, AgentTargetOutcome::BoundedCompleted(_)),
            "identity comparison must not fail hard: {:?}",
            outcome.detail()
        );
        assert!(outcome.detail().contains("未覆盖"), "{:?}", outcome.detail());
        let seen = site_seen.lock().unwrap().join("\n");
        assert!(seen.contains("cookie-alpha") && seen.contains("cookie-beta"));
        assert!(
            !seen.contains("cookie-alpha; session=cookie-beta")
                && !seen.contains("cookie-beta; session=cookie-alpha"),
            "credentials must never be merged: {seen}"
        );
        let diff = findings_for(&db_path, AGENT_EVIDENCE_STAGE)
            .into_iter()
            .map(|(_, row)| row)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            diff.contains("身份 A") && diff.contains("身份 B"),
            "two-identity comparison must label both sides: {diff}"
        );
    }

    /// Case 5 end to end: a confirmed challenge stops the loop and fuses the
    /// target, and no further request is allowed.
    #[test]
    fn e2e_confirmed_challenge_fuses_target() {
        let mut harness = fresh_single_production_harness("e2e-waf", mock_site);
        freeze_fresh_single_production_harness(&mut harness);
        let target_url = harness.context.target_url.clone();
        retarget_model(
            &mut harness,
            vec![
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/gated")}),
                    )],
                    900,
                ),
                model_round(
                    &[(
                        "replay_http",
                        serde_json::json!({"identity":"anonymous","method":"GET","url":format!("{target_url}/api/orders")}),
                    )],
                    900,
                ),
            ],
        );
        let owned = execute_fresh_single_production_harness(&mut harness);
        let outcome = &owned.outcome;
        let AgentTargetOutcome::Limited(stop) = &outcome else {
            panic!("challenge must fuse the target, got {:?}", outcome.detail());
        };
        assert_eq!(stop.code, AGENT_STOP_WAF);
        assert!(stop.requires_fuse());
        let mut tally = AgentPipelineTally::default();
        assert!(record_owned_agent_target_outcome(
            &harness.db_path,
            &harness.context.scan_id,
            &harness.context.route,
            &owned,
            &mut tally,
        ));
        assert_eq!(tally.limited, 1);
        assert_fresh_single_surface(&harness, &target_url, "protected_stop", 1, 0);
        let connection = db::open(&harness.db_path).unwrap();
        let fused: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sentinel_fuse_zone WHERE project_id=9001 AND archived=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(fused >= 1, "the fuse zone must record the protected target");
        assert_eq!(
            harness.site_seen.lock().unwrap().len(),
            1,
            "only the challenging request may reach the site"
        );
        // Durable denial survives a new DB connection and cleared runtime
        // memory. It is neither a signed source claim nor a resume grant.
        drop(connection);
        let reopened = db::open(&harness.db_path).unwrap();
        let original = super::tests::application_table_snapshot(&reopened);
        let sdk_before = harness.model_seen.lock().unwrap().len();
        let target_before = harness.site_seen.lock().unwrap().clone();
        let run = harness.context.run.as_ref().unwrap();
        let invocation: String = reopened.query_row(
            "SELECT invocation_id FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND run_id=?3 AND response_status<>0 AND received_at<>''",
            params![harness.context.scan_id,target_url,run.run_id], |row| row.get(0),
        ).unwrap();
        let mut runtime = AgentToolRuntime {
            protected_stop: Some(stop.clone()),
            current_invocation: invocation,
            target_requests: 1,
            ..Default::default()
        };
        runtime.protected_stop = None;
        assert!(runtime.protected_stop.is_none());
        let persisted = agent_protection_stop_on(&reopened, &harness.context)
            .unwrap().expect("the original actual challenge must remain a durable denial");
        assert_eq!(persisted.code, AGENT_STOP_WAF);
        assert!(persisted.requires_fuse());
        let mut other_target = harness.context.clone();
        other_target.target_url = format!("{target_url}/different-protection-target");
        assert!(agent_protection_stop_on(&reopened, &other_target).unwrap().is_none());
        let mut other_scan = harness.context.clone();
        other_scan.scan_id = "different-protection-scan".into();
        assert!(agent_protection_stop_on(&reopened, &other_scan).unwrap().is_none());
        let mut later_attempt = harness.context.clone();
        later_attempt.attempt_number = later_attempt.attempt_number.checked_add(1).unwrap();
        assert_eq!(agent_protection_stop_on(&reopened, &later_attempt)
            .unwrap().expect("a new attempt must not clear the target denial").code, AGENT_STOP_WAF);
        let request = AgentHttpRequest {
            identity: AgentIdentity::anonymous(),
            method: "GET".into(),
            url: format!("{target_url}/api/orders"),
            extra_headers: vec![],
            body: None,
            content_type: None,
            contract_key: String::new(),
            family: "authorization".into(),
            source: ScopeSource::HttpReplay,
            tool: "replay_http".into(),
            timeout_seconds: 5,
        };
        for context in [&harness.context, &later_attempt] {
            let tx = reopened.unchecked_transaction().unwrap();
            let error = claim_agent_http_request_on(
                &tx, context, &runtime, &request, &request.url, &[],
            ).expect_err("durable protection must deny before auth, invocation or journal writes");
            assert_eq!(error, "http_protection_stop_active");
            assert!(super::tests::application_table_snapshot(&tx) == original);
            tx.commit().unwrap();
            assert!(super::tests::application_table_snapshot(&reopened) == original);
            assert_eq!(harness.model_seen.lock().unwrap().len(), sdk_before);
            assert_eq!(*harness.site_seen.lock().unwrap(), target_before);
        }
        let tx = reopened.unchecked_transaction().unwrap();
        let target_request = tx.query_row(
            "SELECT scan_id,attempt_number,target_url,budget_attempt,ordinal,invocation_id,request_index,request_hash FROM agent_http_request_claims WHERE run_id=?1 AND invocation_id=?2 AND response_status<>0 AND received_at<>''",
            params![run.run_id,runtime.current_invocation], |row| Ok(
                crate::agent_runtime::multi_agent::budget::root::target::RootTargetRequest {
                    scan: row.get(0)?, attempt: row.get(1)?, target: row.get(2)?,
                    budget_attempt: row.get(3)?, ordinal: row.get(4)?,
                    invocation: row.get(5)?, request_index: row.get(6)?, request_hash: row.get(7)?,
                }),
        ).unwrap();
        let source = format!("http:{}:{}:{}:{}",run.run_id,target_request.invocation,
            target_request.request_index,target_request.request_hash);
        // Single has no multi assignment or Coordinator grant. This value is
        // only a scope carrier proving the first deny guard; empty C fields
        // cannot authorize execution, and no lease/grant is inserted or renewed.
        let scope_only = crate::agent_runtime::multi_agent::lease::CoordinatorLease {
            scan_id: target_request.scan.clone(), attempt_number: target_request.attempt,
            target_key: target_request.target.clone(), root_run_id: run.run_id.clone(),
            lease_epoch: 0, fencing_token: String::new(), lease_expires_at: String::new(),
        };
        assert_eq!(crate::agent_runtime::multi_agent::budget::target::claim(
            &tx,&scope_only,"",&source,
        ).unwrap_err(),"target_protection_stop_active");
        assert!(super::tests::application_table_snapshot(&tx) == original);
        // Historical owner loading is pure read even after protected closure.
        // Reusing the actual received claim tuple tests denial before live-owner
        // admission; it does not assert that a received call is new work.
        let owner = crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(
            &tx,&run.run_id,
        ).unwrap();
        let denial = match crate::agent_runtime::multi_agent::budget::root::target::RootTargetCall::claim(
            &tx,owner,target_request,false,0,
        ) {
            Err(error) => error,
            Ok(_) => panic!("shared protection must deny before original Root admission"),
        };
        assert_eq!(denial, "target_protection_stop_active");
        assert!(super::tests::application_table_snapshot(&tx) == original);
        tx.commit().unwrap();
        assert!(super::tests::application_table_snapshot(&reopened) == original);
        assert_eq!(harness.model_seen.lock().unwrap().len(), sdk_before);
        assert_eq!(*harness.site_seen.lock().unwrap(), target_before);
        let usage = agent_request_accounting(&reopened, &harness.context.scan_id,
            harness.context.attempt_number, &target_url).unwrap();
        assert_eq!((usage.executor_recorded, usage.executor_unresolved), (1, 0));
        let target_cost = crate::agent_runtime::multi_agent::budget::balance(
            &reopened, &run.run_id, Some(""), "target_requests",
        ).unwrap();
        assert_eq!((target_cost.consumed, target_cost.reserved, target_cost.indeterminate), (1, 0, 0));
        // The actual protected response is retained privately as an unsealed
        // transport observation. Its public digest does not prove signed origin.
        let artifact_dir = harness.context.target_dir.join(AGENT_HTTP_DIRECTORY);
        let directory = open_agent_artifact_directory(&artifact_dir).unwrap();
        let mut names = fs::read_dir(&artifact_dir).unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["0001.body", "0001.json"]);
        let (record_bytes, record_len) = read_agent_artifact_file(
            &directory,"0001.json",1_048_576,
        ).unwrap();
        assert_eq!(record_len,record_bytes.len() as u64);
        let record: JsonValue = serde_json::from_slice(&record_bytes).unwrap();
        assert_eq!(record_bytes,record.to_string().as_bytes());
        let (body, body_len) = read_agent_artifact_file(
            &directory,"0001.body",AGENT_MAX_RESPONSE_BYTES as u64,
        ).unwrap();
        let (expected_status, expected_type, expected_body) = mock_site(
            "GET /api/gated HTTP/1.1\r\n\r\n".into(),
        );
        assert_eq!(body,expected_body.as_bytes());
        assert_eq!(body_len,expected_body.len() as u64);
        assert_eq!(record["payloadFile"],"0001.body");
        assert_eq!(record["payloadBytes"],body_len);
        assert!(record["requestPayloadFile"].is_null());
        assert_eq!(record["request"]["method"],"GET");
        assert_eq!(record["request"]["identity"],"anonymous");
        assert_eq!(record["request"]["url"],format!("{target_url}/api/gated"));
        let response = &record["response"];
        assert_eq!(response["schemaVersion"],1);
        assert_eq!(response["status"],expected_status);
        assert_eq!(response["contentType"],expected_type);
        assert_eq!(response["truncated"],false);
        assert_eq!(response["bodySha256"],format!("{:x}",sha2::Sha256::digest(&body)));
        assert_eq!(response["protectionCode"],AGENT_STOP_WAF);
        assert_eq!(response["provenance"],"transport_observed_unsealed");
        assert!(response.get("originalArtifactSealed").is_none());
        let original_claim: JsonValue = reopened.query_row(
            "SELECT run_id,invocation_id,request_index,budget_attempt,ordinal,request_hash,response_status FROM agent_http_request_claims WHERE run_id=?1 AND invocation_id=?2 AND received_at<>''",
            params![run.run_id,runtime.current_invocation], |row| {
                assert_eq!(row.get::<_,i64>(6)?,i64::from(expected_status));
                Ok(json!({"runId":row.get::<_,String>(0)?,"invocationId":row.get::<_,String>(1)?,
                    "requestIndex":row.get::<_,i64>(2)?,"budgetAttempt":row.get::<_,i64>(3)?,
                    "ordinal":row.get::<_,i64>(4)?,"requestHash":row.get::<_,String>(5)?}))
            },
        ).unwrap();
        for key in ["runId","invocationId","requestIndex","budgetAttempt","ordinal","requestHash"] {
            assert_eq!(response["httpClaim"][key],original_claim[key]);
        }
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(directory.metadata().unwrap().permissions().mode() & 0o777,0o700);
            for name in &names {
                let metadata = fs::symlink_metadata(artifact_dir.join(name)).unwrap();
                assert!(metadata.is_file());
                assert_eq!(metadata.permissions().mode() & 0o777,0o600);
            }
        }
        assert_eq!(harness.model_seen.lock().unwrap().len(),sdk_before);
        assert_eq!(*harness.site_seen.lock().unwrap(),target_before);
        assert!(super::tests::application_table_snapshot(&reopened) == original);
    }

    // ------------------------------------------------------------------
    // Review fixes: queue consumption, resume budget and identity diffs
    // ------------------------------------------------------------------
