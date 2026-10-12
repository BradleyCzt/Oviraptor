// Phase 1.1: use the product target entry, original frozen Root, and a loopback
// provider. The provider only returns decisions; all writes and Worker execution
// belong to the production paid tick / apply / mailbox drain path.
fn homogeneous_product_envelope(
    round: usize,
    task: &str,
    action: &str,
    payload: JsonValue,
    refs: JsonValue,
) -> JsonValue {
    json!({
        "decision_id": format!("dec-{round:016x}"), "parent_task_id": task,
        "action": action, "payload": payload, "allowlist_refs": refs
    })
}

fn homogeneous_product_refs() -> JsonValue {
    json!({"slice_ids":[],"worker_ids":[],"tool_ids":[],
        "target_keys":[],"directive_ids":[],"assignment_ids":[]})
}

fn homogeneous_product_execute(h: &AgentHarness) -> OwnedAgentTargetOutcome {
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: h.context.route.clone(),
        target_dir: h.context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    let settings = json!({"agentBackendPolicy":"native"});
    run_agent_target(
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
    .expect("actual product target entry")
}

fn homogeneous_product_freeze(h: &mut AgentHarness) {
    // Declare opt-in before activation. This scan-level plan is separate from
    // the signed per-target original financial plan, which stays untouched.
    let plan = json!({"scheduling_model":"homogeneous_workers"}).to_string();
    db::open(&h.db_path).unwrap().execute(
        "INSERT INTO native_scan_plans(scan_id,attempt_number,scan_type,backend,plan_json,plan_hash)
         VALUES(?1,1,'web','native',?2,?3)",
        params![h.context.scan_id, plan, crate::agent_runtime::store::stable_hash(&plan)],
    ).unwrap();
    freeze_fresh_multi_production_harness(h);
}

#[test]
fn homogeneous_product_original_headroom_denial_is_limited_without_model_io() {
    use crate::agent_runtime::multi_agent::homogeneous::{intent, store};
    let mut h = fresh_multi_production_harness("homogeneous-product-last-request");
    h.context.execution_plan.hard_model_requests = 1;
    h.context.execution_plan.soft_model_requests = 1;
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
        panic!("original headroom denial must occur before provider I/O")
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    homogeneous_product_freeze(&mut h);
    // Exercise the same prepare / product Root loop with the genuinely frozen
    // one-request plan. The new-target UI planner intentionally has higher floors.
    match multi_agent_prepare(&mut h.context) {
        Err(error) => assert_eq!(error, "homogeneous_scan_legacy_bootstrap_stepped_aside"),
        Ok(_) => panic!("frozen homogeneous plan must step aside from legacy bootstrap"),
    }
    let outcome = run_homogeneous_agent_target(&mut h.context);
    assert!(
        matches!(outcome, AgentTargetOutcome::Limited(_)),
        "{}",
        outcome.detail()
    );
    assert!(outcome
        .detail()
        .contains("child_budget_reservation_exceeded_or_stale"));
    let db = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    let task = store::load_task_by_root_run(&db, root).unwrap().unwrap();
    assert_eq!(task.state, "created");
    assert_eq!(
        intent::latest_stop_reason(&db, &task.task_id)
            .unwrap()
            .as_deref(),
        None
    );
    assert!(store::load_slices(&db, &task.task_id).unwrap().is_empty());
    assert!(h.model_seen.lock().unwrap().is_empty());
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='publication'", [root], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "model_requests")
            .unwrap()
            .consumed,
        0
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, Some(""), "model_requests")
            .unwrap()
            .reserved,
        0
    );
    drop(db);
    let _ = fs::remove_dir_all(&h.root);
}

#[test]
fn homogeneous_product_entry_pays_decomposes_drains_accepts_and_stops() {
    use crate::agent_runtime::multi_agent::homogeneous::{decompose, machine, pool, store};
    let mut h = fresh_multi_production_harness("homogeneous-product-complete");
    let path = h.db_path.clone();
    let rounds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = rounds.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |wire| {
        let round = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        assert!(
            round <= 4,
            "valid closure must finish in four paid decisions"
        );
        let request: JsonValue =
            serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input: JsonValue =
            serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(
            input["basis"]["phase"], "homogeneous-decision",
            "first round must skip legacy birth"
        );
        let connection = db::open(&path).unwrap();
        let root: String = connection
            .query_row("SELECT root_run_id FROM agent_homogeneous_tasks", [], |r| {
                r.get(0)
            })
            .unwrap();
        let task = store::load_task_by_root_run(&connection, &root)
            .unwrap()
            .unwrap();
        let mut refs = homogeneous_product_refs();
        let mut decision = match round {
            1 => {
                let slice = decompose::slice_id_for(
                    &task.task_id,
                    "path_prefix",
                    "/api/homogeneous",
                    task.evidence_revision_base,
                );
                refs["target_keys"] = json!([task.target_key]);
                homogeneous_product_envelope(
                    round,
                    &task.task_id,
                    "decompose",
                    json!({
                        "slices":[{"slice_id":slice,"scope_kind":"path_prefix","scope_value":"/api/homogeneous",
                            "lane":"read_only_analysis","evidence_revision_base":task.evidence_revision_base,
                            "worker_capability_profile_id":"homogeneous_v1","max_retries":2}],
                        "decomposition_basis_zh":"按现有证据路径分解"
                    }),
                    refs,
                )
            }
            2 => {
                let slice = store::load_slices(&connection, &task.task_id)
                    .unwrap()
                    .remove(0);
                let worker = pool::worker_ids_in_state(&connection, &root, "idle")
                    .unwrap()
                    .remove(0);
                refs["slice_ids"] = json!([slice.slice_id]);
                refs["worker_ids"] = json!([worker]);
                refs["tool_ids"] = json!(["evidence.read"]);
                homogeneous_product_envelope(
                    round,
                    &task.task_id,
                    "assign",
                    json!({
                        "slice_id":slice.slice_id,"worker_id":worker,"reserved_tokens":100,"reserved_requests":1,
                        "capability_lease":["evidence.read"],"lane":"read_only_analysis","attempt":1
                    }),
                    refs,
                )
            }
            3 => {
                let id = store::list_assignment_ids_in_state(&connection, &root, &["reported"])
                    .unwrap()
                    .remove(0);
                let assignment = store::load_assignment(&connection, &id).unwrap().unwrap();
                let slice = store::load_slice(&connection, &assignment.slice_id)
                    .unwrap()
                    .unwrap();
                let report =
                    store::load_report(&connection, &store::report_id_for(&id, assignment.attempt))
                        .unwrap()
                        .unwrap();
                assert_eq!(
                    report.terminal_hint, "completed",
                    "automatic mailbox consumer must report"
                );
                let checks = machine::recompute(&connection, &task, &assignment, &slice, &report);
                refs["slice_ids"] = json!([slice.slice_id]);
                refs["assignment_ids"] = json!([id]);
                let mut d = homogeneous_product_envelope(
                    round,
                    &task.task_id,
                    "accept",
                    json!({
                        "assignment_id":id,"slice_id":slice.slice_id,"report_id":report.report_id,
                        "verdict":"accept","machine_checks":checks,
                        "model_judgement":{"adequate":true,"question_zh":"证据是否覆盖切片？","answer_zh":"已覆盖"},
                        "reason_codes":["goal_reached"],"evidence_refs":report.evidence_refs
                    }),
                    refs,
                );
                d["zero_intent"] = json!({"reason":"blocked_on_inflight_output",
                    "evidence_assignment_ids":[id],"note_zh":"本轮验收已汇报证据，下一轮收官"});
                d
            }
            _ => {
                let accepted: Vec<String> = store::load_slices(&connection, &task.task_id)
                    .unwrap()
                    .into_iter()
                    .filter(|s| s.state == "accepted")
                    .map(|s| s.slice_id)
                    .collect();
                assert_eq!(accepted.len(), 1);
                refs["slice_ids"] = json!(accepted);
                homogeneous_product_envelope(
                    round,
                    &task.task_id,
                    "stop",
                    json!({
                        "stop_reason":"goal_reached","accepted_slice_ids":accepted,"rejected_slice_ids":[],
                        "summary_zh":"只读证据切片已验收","publish_partial":false
                    }),
                    refs,
                )
            }
        };
        // Keep the source decision strictly snake_case even though the existing
        // paid invoice continues to store its independent camelCase summary.
        decision["model_notes_zh"] = json!("单轮单动作");
        (
            200,
            "application/json",
            proposal_model_response(&decision.to_string()),
        )
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    homogeneous_product_freeze(&mut h);
    let connection = db::open(&h.db_path).unwrap();
    let root = &h.context.run.as_ref().unwrap().run_id;
    connection.execute("INSERT INTO agent_evidence_revisions(root_run_id,revision,parent_revision) VALUES(?1,1,NULL)", [root]).unwrap();
    let node =
        crate::agent_runtime::store::stable_hash(&format!("{root}:homogeneous-product-evidence"));
    connection.execute("INSERT INTO agent_evidence_nodes(id,root_run_id,revision,kind,provenance,natural_key_hash,payload_json,created_by_run_id)
        VALUES(?1,?2,1,'endpoint','observed',?1,?3,?2)",
        params![node, root, json!({"path":"/api/homogeneous","host":"127.0.0.1"}).to_string()]).unwrap();
    let owned = homogeneous_product_execute(&h);
    assert!(
        owned.outcome.completion().is_some(),
        "{}",
        owned.outcome.detail()
    );
    let completion = owned.outcome.completion().unwrap();
    assert_eq!(completion.model_requests, 4, "confirmed Root invoices");
    assert_eq!(completion.total_tokens, 80, "original provider usage");
    assert!(!owned.outcome.detail().contains("bootstrap_failed"));
    assert_eq!(rounds.load(std::sync::atomic::Ordering::SeqCst), 4);
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM agent_task_slices WHERE state='accepted'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &connection,
            root,
            Some(""),
            "model_requests"
        )
        .unwrap()
        .reserved,
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM agent_root_tick_receipts WHERE phase='request'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        4
    );
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE scheduling_model='legacy_role_pipeline'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM agent_lane_leases", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(fs::read_to_string(&h.context.log_path)
        .unwrap()
        .contains("root_homogeneous_workers:1:"));
    assert_eq!(connection.query_row("SELECT reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1", [root], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))).unwrap(), (0, 0));
    assert_eq!(connection.query_row("SELECT COALESCE(SUM(CASE kind WHEN 'reserve' THEN amount WHEN 'consume' THEN -amount WHEN 'release' THEN -amount WHEN 'forfeit' THEN -amount ELSE 0 END),0) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id<>'' AND dimension IN ('model_input_tokens','model_cached_tokens','model_output_tokens','model_requests')", [root], |row| row.get::<_, i64>(0)).unwrap(), 0, "accepted Worker leaves no fine reservation");
    drop(owned);
    drop(connection);
    let _ = fs::remove_dir_all(&h.root);
}

fn homogeneous_product_refusal_case(mode: &'static str) {
    use crate::agent_runtime::multi_agent::homogeneous::{decompose, store};
    let mut h = fresh_multi_production_harness(&format!("homogeneous-product-{mode}"));
    let path = h.db_path.clone();
    let rounds = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = rounds.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_wire| {
        let round = count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        assert!(round <= 2, "refusal must not keep paying without work");
        let connection = db::open(&path).unwrap();
        let root: String = connection
            .query_row("SELECT root_run_id FROM agent_homogeneous_tasks", [], |r| {
                r.get(0)
            })
            .unwrap();
        let task = store::load_task_by_root_run(&connection, &root)
            .unwrap()
            .unwrap();
        let mut refs = homogeneous_product_refs();
        if round == 1 {
            if mode == "local-tool" {
                return (
                    200,
                    "application/json",
                    model_round(&[("snapshot.read", json!({}))], 10),
                );
            }
            let draft = decompose::deterministic_candidates(&connection, &task)
                .unwrap()
                .remove(0);
            refs["target_keys"] = json!([task.target_key]);
            let mut d = homogeneous_product_envelope(
                round,
                &task.task_id,
                "decompose",
                json!({
                    "slices":[{"slice_id":draft.slice_id,"scope_kind":draft.scope_kind,
                        "scope_value":draft.scope_value,"lane":draft.lane,
                        "evidence_revision_base":draft.evidence_revision_base,
                        "worker_capability_profile_id":draft.worker_capability_profile_id,
                        "max_retries":draft.max_retries}],"decomposition_basis_zh":"按冻结主机分解"
                }),
                refs,
            );
            if mode == "invalid-zero" {
                d["zero_intent"] = json!({"reason":"just_wait",
                    "evidence_assignment_ids":["asg-1111111111111111"],"note_zh":"非法理由"});
            }
            return (
                200,
                "application/json",
                proposal_model_response(&d.to_string()),
            );
        }
        assert!(
            task.degraded,
            "same paid refusal must mark deterministic fallback"
        );
        let slices = store::load_slices(&connection, &task.task_id).unwrap();
        assert!(
            !slices.is_empty(),
            "first refused round must already replenish an intent"
        );
        assert!(slices.iter().all(|slice| slice.state == "pending"));
        let ids: Vec<_> = slices.into_iter().map(|slice| slice.slice_id).collect();
        refs["slice_ids"] = json!(ids);
        let d = homogeneous_product_envelope(
            round,
            &task.task_id,
            "stop",
            json!({
                "stop_reason":"degraded_path_used","accepted_slice_ids":[],"rejected_slice_ids":ids,
                "summary_zh":"降级事实保留并正式停止","publish_partial":false
            }),
            refs,
        );
        (
            200,
            "application/json",
            proposal_model_response(&d.to_string()),
        )
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    homogeneous_product_freeze(&mut h);
    let connection = db::open(&h.db_path).unwrap();
    if mode == "receipt-fault" {
        connection
            .execute_batch(
                "CREATE TRIGGER product_ignore_decision BEFORE INSERT ON agent_root_tick_receipts
            WHEN NEW.phase='decision' BEGIN SELECT RAISE(IGNORE); END;",
            )
            .unwrap();
    }
    let owned = homogeneous_product_execute(&h);
    assert!(
        owned.outcome.completion().is_none(),
        "a refusal cannot claim goal completion"
    );
    assert!(!owned.outcome.detail().contains("bootstrap_failed"));
    let calls = rounds.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        calls,
        if mode == "receipt-fault" { 1 } else { 2 },
        "{}",
        owned.outcome.detail()
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM agent_root_model_journal WHERE phase='received'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        calls as i64,
        "every provider call must keep exactly one original received invoice"
    );
    if mode == "receipt-fault" {
        assert!(
            owned
                .outcome
                .detail()
                .contains("root_tick_decision_receipt_unconfirmed"),
            "{}",
            owned.outcome.detail()
        );
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM agent_task_slices", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "unconfirmed financial decision must block apply and fallback"
        );
    } else {
        assert!(
            matches!(&owned.outcome, AgentTargetOutcome::Incomplete(_)),
            "{}",
            owned.outcome.detail()
        );
        assert!(fs::read_to_string(&h.context.log_path)
            .unwrap()
            .contains("root_homogeneous_degraded:deterministic"));
        let first: String = connection
            .query_row(
                "SELECT fact_json FROM agent_root_tick_receipts WHERE round=1 AND phase='decision'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let first: JsonValue = serde_json::from_str(&first).unwrap();
        let expected = if mode == "local-tool" {
            "refused:tool_not_authorized"
        } else {
            "refused:zero_intent_illegal"
        };
        assert_eq!(first["summary"]["observed"], json!([expected]));
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM agent_root_tick_receipts WHERE phase='publication'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            2,
            "a bounded refusal keeps original history executable without another invoice"
        );
        assert_eq!(connection.query_row("SELECT count(*) FROM agent_task_slices WHERE decomposition_model='deterministic'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }
    assert_eq!(connection.query_row("SELECT count(*) FROM agent_assignments WHERE scheduling_model='legacy_role_pipeline'", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    drop(owned);
    drop(connection);
    let _ = fs::remove_dir_all(&h.root);
}

#[test]
fn homogeneous_product_invalid_zero_is_refused_after_one_invoice_and_replenished() {
    homogeneous_product_refusal_case("invalid-zero");
}

#[test]
fn homogeneous_product_local_tools_are_refused_and_never_executed() {
    homogeneous_product_refusal_case("local-tool");
}

#[test]
fn homogeneous_product_unconfirmed_paid_decision_blocks_all_control_side_effects() {
    homogeneous_product_refusal_case("receipt-fault");
}
