#[test]
fn homogeneous_model_workers_real_transport_parallel_receipts_and_chat_catchup() {
    homogeneous_model_case(0);
}
#[test]
fn homogeneous_model_paid_invalid_output_retries_with_new_original_attempt() {
    homogeneous_model_case(1);
}
#[test]
fn homogeneous_model_unknown_cost_stops_without_replay_or_fake_report() {
    homogeneous_model_case(2);
}
#[test]
fn homogeneous_model_guidance_during_paid_tick_is_queued_then_acknowledged_by_next_worker() {
    homogeneous_model_case(3);
}
#[test]
fn homogeneous_model_guidance_confirmation_rejects_trigger_financial_writes_atomically() {
    homogeneous_model_case(4);
}
#[test]
fn homogeneous_plaintext_guidance_reaches_real_transport_and_original_report_unchanged() {
    homogeneous_model_case(5);
}
#[test]
fn homogeneous_invalid_root_output_degrades_to_real_worker_dispatch_and_closure() {
    homogeneous_model_case(6);
}
fn homogeneous_model_case(failure: usize) {
    use crate::agent_runtime::multi_agent::homogeneous::{model_worker, store};
    let mut h = fresh_multi_production_harness("homogeneous-model-workers");
    let root_path = h.db_path.clone();
    let worker_active = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let active = worker_active.clone();
    let seen_peak = peak.clone();
    let worker_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sdk_calls = worker_calls.clone();
    let rounds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = rounds.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let req: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input: JsonValue =
            serde_json::from_str(req["messages"][1]["content"].as_str().unwrap()).unwrap();
        if input["slice_id"].is_string() {
            assert_eq!(input["goal"], "按现有观测拆分并核对分派范围，累计验收后收官");
            assert_eq!(req["messages"][0]["content"], model_worker::SYSTEM);
            assert!(req
                .get("tools")
                .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)));
            assert_eq!(req["max_tokens"], 512);
            let worker_call = sdk_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if failure == 2 {
                return (
                    503,
                    "application/json",
                    "{\"error\":\"fixture outage\"}".into(),
                );
            }
            let n = active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            seen_peak.fetch_max(n, std::sync::atomic::Ordering::SeqCst);
            if worker_call < 2 {
                let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while sdk_calls.load(std::sync::atomic::Ordering::SeqCst) < 2
                    && std::time::Instant::now() < until
                {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            let refs: Vec<JsonValue> = input["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["evidence_ref"].clone())
                .collect();
            assert!(!refs.is_empty());
            if failure == 1 && worker_call == 0 {
                return (
                    200,
                    "application/json",
                    proposal_model_response("{\"invented_tool\":\"shell.exec\"}"),
                );
            }
            let summary = if failure == 5 { format!("已完成 {} 的只读分析；Cookie: session=fixture-account-a",input["scope_value"].as_str().unwrap()) } else { format!("已完成 {} 的只读分析",input["scope_value"].as_str().unwrap()) };
            return (200,"application/json",proposal_model_response(&json!({"output":{"summary_zh":summary,"findings":[],"coverage_notes":["已核对本切片观测材料"]},"evidence_refs":refs,"terminal_hint":"completed"}).to_string()));
        }
        let round = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        assert!(round < 35, "must reach a bounded terminal state");
        if failure == 6 && round <= 2 {
            return (200, "application/json", proposal_model_response("{}"));
        }
        let task = &input["basis"]["homogeneousTask"];
        assert_eq!(task["goal"], "按现有观测拆分并核对分派范围，累计验收后收官");
        let cards = task["slices"].as_array().unwrap();
        let mut refs = homogeneous_product_refs();
        if matches!(failure, 3 | 4 | 5) && round == 2 {
            // Confirm while the original paid frame is already frozen. The paid
            // action stays immutable and the new guidance enters the next tick.
            let conn = db::open(&root_path).unwrap();
            let draft = draft_scan_directive_in(
                &conn,
                input["basis"]["scan"].as_str().unwrap(),
                if failure == 5 { "优先核对现有证据的覆盖边界；比较 Cookie: session=fixture-account-a 和 Token=fixture-account-b" } else { "优先核对现有证据的覆盖边界" },
                "team",
            )
            .unwrap();
            if failure == 4 {
                conn.execute_batch("CREATE TRIGGER fixture_illegal_guidance_write AFTER INSERT ON agent_human_directive_gated BEGIN UPDATE agent_budget_ledger SET spent_tokens=0 WHERE root_run_id=(SELECT root_run_id FROM agent_homogeneous_tasks WHERE task_id=NEW.parent_task_id); END;").unwrap();
                let result = crate::agent_runtime::multi_agent::directive::confirm_bound_draft(
                    &conn,
                    draft["scanId"].as_str().unwrap(),
                    draft["id"].as_str().unwrap(),
                    draft["revision"].as_i64().unwrap(),
                    draft["draftHash"].as_str().unwrap(),
                );
                assert!(
                    result.is_err(),
                    "a trigger cannot inherit approval or Root write authority"
                );
                assert_eq!(
                    conn.query_row(
                        "SELECT status FROM agent_directive_drafts WHERE id=?1",
                        [draft["id"].as_str().unwrap()],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                    "drafted"
                );
                for table in [
                    "agent_human_directive_gated",
                    "agent_user_directives",
                    "agent_directive_human_reviews",
                ] {
                    assert_eq!(
                        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                            .get::<_, i64>(0))
                            .unwrap(),
                        0
                    );
                }
                conn.execute_batch("DROP TRIGGER fixture_illegal_guidance_write;")
                    .unwrap();
            }
            let directive = crate::agent_runtime::multi_agent::directive::confirm_bound_draft(
                &conn,
                draft["scanId"].as_str().unwrap(),
                draft["id"].as_str().unwrap(),
                draft["revision"].as_i64().unwrap(),
                draft["draftHash"].as_str().unwrap(),
            )
            .unwrap();
            assert_eq!(directive.status, "pending");
        }
        let (action, payload) = if let Some(directive) =
            task["directives"].as_array().unwrap().first()
        {
            refs["directive_ids"] = json!([directive["directive_id"]]);
            (
                "relay_directive",
                json!({"directive_id":directive["directive_id"],"mode":"relay","rewritten_text":null,"target_worker_id":null,"broadcast_constraint":if failure==5 {"优先核对现有证据的覆盖边界；比较 Cookie: session=fixture-account-a 和 Token=fixture-account-b"} else {"优先核对现有证据的覆盖边界"},"user_feedback_zh":"已采纳，只在冻结范围内分析现有材料"}),
            )
        } else if cards.is_empty() {
            let drafts: Vec<JsonValue> = task["decompositionCandidates"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s["scope_kind"] == "path_prefix")
                .cloned()
                .collect();
            assert_eq!(drafts.len(), 2);
            refs["target_keys"] = json!([task["targetKey"]]);
            (
                "decompose",
                json!({"slices":drafts,"decomposition_basis_zh":"按两个观测路径拆分"}),
            )
        } else if let Some(slice) = cards
            .iter()
            .find(|s| s["state"] == "pending" || s["state"] == "rejected")
        {
            let worker = task["workers"]
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["state"] == "idle")
                .unwrap();
            refs["slice_ids"] = json!([slice["slice_id"]]);
            refs["worker_ids"] = json!([worker["worker_id"]]);
            refs["tool_ids"] = json!(["evidence.read"]);
            (
                "assign",
                json!({"slice_id":slice["slice_id"],"worker_id":worker["worker_id"],"reserved_tokens":slice["minimum_reserved_tokens"],"reserved_requests":1,"capability_lease":["evidence.read"],"lane":"read_only_analysis","attempt":slice["next_attempt"]}),
            )
        } else if let Some(report) = task["reports"].as_array().unwrap().first() {
            refs["slice_ids"] = json!([report["slice_id"]]);
            refs["assignment_ids"] = json!([report["assignment_id"]]);
            let failed = report["terminal_hint"] == "failed";
            let mut body = json!({"assignment_id":report["assignment_id"],"slice_id":report["slice_id"],"report_id":report["report_id"],"verdict":if failed {"retry"} else {"accept"},"machine_checks":report["system_machine_checks"],"model_judgement":{"adequate":!failed,"question_zh":"切片是否有可核验依据？","answer_zh":"按真实汇报裁决"},"reason_codes":if failed {json!(["output_schema_invalid"])} else {json!(["goal_reached"])},"evidence_refs":report["evidence_refs"]});
            if failed
                && report["system_machine_checks"]
                    .as_object()
                    .unwrap()
                    .values()
                    .any(|v| v == false)
            {
                body["model_judgement"] = JsonValue::Null;
            }
            if failed {
                body["retry_hint_zh"] = json!("只交付符合契约的证据汇报");
            }
            (if failed { "reject_retry" } else { "accept" }, body)
        } else if cards.iter().all(|s| s["state"] == "accepted") {
            let accepted: Vec<JsonValue> = cards.iter().map(|s| s["slice_id"].clone()).collect();
            refs["slice_ids"] = json!(accepted);
            (
                "stop",
                json!({"stop_reason":"goal_reached","accepted_slice_ids":accepted,"rejected_slice_ids":[],"summary_zh":"两个只读切片已验收","publish_partial":false}),
            )
        } else {
            (
                "decompose",
                json!({"slices":[],"decomposition_basis_zh":"等待已派发汇报"}),
            )
        };
        let mut decision = homogeneous_product_envelope(
            round,
            task["taskId"].as_str().unwrap(),
            action,
            payload,
            refs,
        );
        if action == "relay_directive"
            || action == "accept"
            || action == "reject_retry"
            || decision["payload"]["slices"]
                .as_array()
                .is_some_and(Vec::is_empty)
        {
            let conn = db::open(&root_path).unwrap();
            let ids = store::list_assignment_ids_in_state(
                &conn,
                task["intentSnapshot"]["root_run_id"]
                    .as_str()
                    .unwrap_or_else(|| task["taskId"].as_str().unwrap()),
                &["assigned", "running", "reported"],
            )
            .unwrap();
            let ids = if ids.is_empty() {
                task["candidates"]["assignmentIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|id| {
                        store::load_assignment(&conn, id.as_str().unwrap())
                            .unwrap()
                            .filter(|a| {
                                matches!(a.state.as_str(), "assigned" | "running" | "reported")
                            })
                            .map(|a| a.assignment_id)
                    })
                    .collect::<Vec<_>>()
            } else {
                ids
            };
            decision["allowlist_refs"]["assignment_ids"] = json!(ids);
            decision["zero_intent"] = json!({"reason":"blocked_on_inflight_output","evidence_assignment_ids":ids,"note_zh":"本轮等待或验收已在途任务"});
        }
        (
            200,
            "application/json",
            proposal_model_response(&decision.to_string()),
        )
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    let conn = db::open(&h.db_path).unwrap();
    let mut plan=json!({"schemaVersion":1,"scanId":h.context.scan_id,"attemptNumber":1,"scheduling_model":"homogeneous_workers","workerExecution":"model_read_only_v1","allowedParallel":3,"lane":"read_only_analysis","goal":"按现有观测拆分并核对分派范围，累计验收后收官"});
    if failure == 5 { plan["credentialVisibility"] = json!("plaintext"); }
    let plan = plan.to_string();
    conn.execute("INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash) VALUES(?1,1,'web','native',?2,?3)",params![h.context.scan_id,plan,crate::agent_runtime::store::stable_hash(&plan)]).unwrap();
    h.context.evidence["apiCandidates"] =
        json!([{"path":"/api/one","unusedLargeHeaders":"x".repeat(8000)},{"path":"/api/two"}]);
    fs::write(
        h.context.target_dir.join("frontend-evidence.json"),
        h.context.evidence.to_string(),
    )
    .unwrap();
    freeze_fresh_multi_production_harness(&mut h);
    let root = h.context.run.as_ref().unwrap().run_id.clone();
    let outcome = homogeneous_product_execute(&h);
    if failure == 2 {
        assert!(
            matches!(outcome.outcome, AgentTargetOutcome::Incomplete(_)),
            "{}",
            outcome.outcome.detail()
        );
        assert!(worker_calls.load(std::sync::atomic::Ordering::SeqCst) <= 2);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM agent_worker_reports", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(
            crate::agent_runtime::multi_agent::budget::balance(
                &conn,
                &root,
                None,
                "model_input_tokens"
            )
            .unwrap()
            .indeterminate
                > 0
        );
        assert_eq!(conn.query_row("SELECT count(*) FROM agent_runs WHERE role='deep_investigator' AND status='running'",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        drop(conn);
        let _ = fs::remove_dir_all(h.root);
        return;
    }
    if matches!(failure, 3 | 4 | 5) {
        if failure == 5 {
            assert!(conn.query_row("SELECT count(*) FROM agent_specialist_calls WHERE request_json LIKE '%fixture-account-a%' AND response_json LIKE '%fixture-account-a%'", [], |r| r.get::<_, i64>(0)).unwrap() >= 1);
            let status = native_scan_status_after(&conn, &h.context.scan_id, None).unwrap();
            assert!(status.to_string().contains("fixture-account-a"), "operator chat must preserve task identity material");
        }
        assert_eq!(
            conn.query_row("SELECT state FROM agent_human_directive_gated", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
            "applied"
        );
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM agent_directive_human_reviews WHERE kind='approve'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        assert!(conn.query_row("SELECT count(*) FROM agent_messages WHERE kind='homogeneous_guidance' AND delivered_at<>'' AND acknowledged_at<>''",[],|r|r.get::<_,i64>(0)).unwrap()>=1);
        assert!(
            conn.query_row(
                "SELECT count(*) FROM agent_specialist_calls WHERE request_json LIKE '%覆盖边界%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
                >= 1
        );
    }
    let expected_workers = if failure == 1 { 3 } else if failure == 6 { 1 } else { 2 };
    assert!(
        outcome.outcome.completion().is_some(),
        "{}",
        outcome.outcome.detail()
    );
    assert_eq!(
        peak.load(std::sync::atomic::Ordering::SeqCst),
        if failure == 6 { 1 } else { 2 },
        "independent concurrent model calls"
    );
    assert_eq!(conn.query_row("SELECT count(*) FROM agent_worker_reports WHERE consumed_requests=1 AND terminal_hint='completed'",[],|r|r.get::<_,i64>(0)).unwrap(), if failure == 6 { 1 } else { 2 });
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM agent_specialist_calls WHERE state='received'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        expected_workers
    );
    assert_eq!(
        outcome.outcome.completion().unwrap().model_requests,
        rounds.load(std::sync::atomic::Ordering::SeqCst) as i64 + expected_workers
    );
    let status = native_scan_status_after(&conn, &h.context.scan_id, None).unwrap();
    assert_eq!(
        status["llmRequests"],
        rounds.load(std::sync::atomic::Ordering::SeqCst) as i64 + expected_workers
    );
    assert_eq!(
        status["totalTokens"],
        (rounds.load(std::sync::atomic::Ordering::SeqCst) as i64 + expected_workers) * 20
    );
    let tx = conn.unchecked_transaction().unwrap();
    let window = TimelineWindow::latest(&tx, &h.context.scan_id, 1, None).unwrap();
    let (timeline, _) =
        native_status_timeline(&tx, &h.context.scan_id, 1, &window, &json!({}), None).unwrap();
    let reports: Vec<_> = timeline
        .iter()
        .filter(|i| i["eventType"] == "worker_report")
        .collect();
    if matches!(failure, 3 | 4 | 5) {
        let original:String=conn.query_row("SELECT id FROM agent_user_directives",[],|r|r.get(0)).unwrap();
        let guide=timeline.iter().find(|i|i["eventType"]=="homogeneous_guidance").unwrap();
        assert_eq!(guide["sourceUserDirectiveId"],original);
        assert_eq!(guide["status"],"applied");
        assert!(timeline.iter().any(|i|i["messageKind"]=="homogeneous_guidance" && i["eventType"]=="mailbox_message" && i["summary"].as_str().unwrap().contains("覆盖边界") && i["summary"].as_str().unwrap().contains("Worker 已读取约束")));
        let source=timeline.iter().find(|i|i["eventType"]=="user_directive").unwrap();
        assert_eq!(source["taskClosure"]["disposition"],"not_applied","the original independent action receipt is not rewritten as a guidance ACK");
    }
    assert_eq!(reports.len(), expected_workers as usize);
    assert!(reports
        .iter()
        .all(|i| i["sequence"].as_i64().unwrap() > 0 && i["modelRequests"] == 1));
    let first = reports
        .iter()
        .map(|i| i["sequence"].as_i64().unwrap())
        .min()
        .unwrap();
    let catchup = TimelineWindow::latest(&tx, &h.context.scan_id, 1, Some(first - 1)).unwrap();
    assert_eq!(
        native_status_timeline(&tx, &h.context.scan_id, 1, &catchup, &json!({}), None)
            .unwrap()
            .0
            .iter()
            .filter(|i| i["eventType"] == "worker_report")
            .count(),
        expected_workers as usize
    );
    tx.commit().unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM agent_assignments WHERE scheduling_model='legacy_role_pipeline'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    let _ = fs::remove_dir_all(h.root);
}
