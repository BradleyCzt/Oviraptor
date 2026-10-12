// The first case tests the SQLite admission/counting boundary with explicitly
// synthetic journal rows. It does not claim target I/O or parallel model SDKs.
fn supervision_capacity_test_pending(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    run: &str,
    ordinal: i64,
) {
    db.execute(
        "INSERT INTO agent_http_budget_origins(scan_id,target_url,budget_attempt,baseline_requests)
         VALUES(?1,?2,?3,0) ON CONFLICT DO NOTHING",
        params![lease.scan_id, lease.target_key, lease.attempt_number],
    )
    .unwrap();
    db.execute(
        "INSERT INTO agent_http_request_claims(scan_id,target_url,budget_attempt,ordinal,
         attempt_number,run_id,invocation_id,request_index,tool_name,identity_handle,request_hash)
         VALUES(?1,?2,?3,?4,?3,?5,?6,1,'replay_http','anonymous',?6)",
        params![
            lease.scan_id,
            lease.target_key,
            lease.attempt_number,
            ordinal,
            run,
            format!("synthetic-capacity-boundary-{ordinal}")
        ],
    )
    .unwrap();
}

#[test]
fn supervision_capacity_concurrent_immediate_transactions_cannot_spend_last_slot_twice() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(0);
    let s = f.session.as_ref().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let available = budget::root::remaining_child_capacity(&db, &s.lease.root_run_id)
        .unwrap()
        .1
        .unwrap();
    assert!(available >= 7);
    let prefix = available - 7;
    for ordinal in 1..=prefix {
        supervision_capacity_test_pending(&db, &s.lease, &s.executor.run_id, ordinal);
    }
    let before = budget::root::remaining_child_capacity(&db, &s.lease.root_run_id).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        // Open both connections before the barrier. Real IMMEDIATE transactions
        // serialize the count/check/insert; no sleep-based assumed ordering.
        let worker = db::open(&f.h.db_path).unwrap();
        let lease = s.lease.clone();
        let executor = s.executor.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            let tx = rusqlite::Transaction::new_unchecked(
                &worker,
                rusqlite::TransactionBehavior::Immediate,
            )
            .unwrap();
            match budget::supervision_capacity::require_http_headroom(
                &tx,
                &lease,
                &executor.assignment_id,
                &executor.run_id,
            ) {
                Ok(()) => {
                    supervision_capacity_test_pending(&tx, &lease, &executor.run_id, prefix + 1);
                    tx.commit().unwrap();
                    Ok(())
                }
                Err(error) => {
                    assert_eq!(tx.total_changes(), 0, "losing guard must not write");
                    tx.rollback().unwrap();
                    Err(error)
                }
            }
        }));
    }
    barrier.wait();
    let outcomes: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter_map(|r| r.as_ref().err())
            .collect::<Vec<_>>(),
        vec!["root_supervision_request_capacity_exhausted"]
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE scan_id=?1",
            [&s.lease.scan_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        prefix + 1
    );
    assert_eq!(
        budget::root::remaining_child_capacity(&db, &s.lease.root_run_id).unwrap(),
        before,
        "counted obligations are not fabricated financial reserves or payments"
    );
    let changes = db.total_changes();
    let snapshot = db.unchecked_transaction().unwrap();
    let report = serde_json::to_value(
        budget::supervision_capacity::diagnostic(&snapshot, &s.lease.root_run_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["pendingPhysicalRequests"], prefix + 1);
    assert_eq!(report["nextHttpRequiredModelRequests"], available + 1);
    assert_eq!(report["nextHttpRequestBalanceSufficient"], false);
    assert_eq!(report["authoritative"], false);
    assert_eq!(report["executionAllowed"], false);
    assert_eq!(report["futureTokenCapacityGuaranteed"], false);
    assert_eq!(report["usdCapacityCoverage"], "not_enforced_by_this_ledger");
    snapshot.commit().unwrap();
    assert_eq!(
        db.total_changes(),
        changes,
        "report must never create reserves or fees"
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 3);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

fn supervision_capacity_test_http(
    context: &AgentRunContext,
) -> (AgentToolRuntime, AgentHttpRequest) {
    let mut runtime = AgentToolRuntime::default();
    let invocation = runtime.begin_invocation(context.attempt_number);
    context
        .run
        .as_ref()
        .unwrap()
        .begin_tool("replay_http", &json!({}), &invocation)
        .unwrap();
    let request = AgentHttpRequest {
        identity: AgentIdentity::anonymous(),
        method: "GET".into(),
        url: context.target_url.clone(),
        extra_headers: vec![],
        body: None,
        content_type: None,
        contract_key: String::new(),
        family: "authorization".into(),
        source: ScopeSource::IdentityComparison,
        tool: "replay_http".into(),
        timeout_seconds: 5,
    };
    (runtime, request)
}

#[test]
fn supervision_capacity_original_unknown_claim_blocks_fresh_work_without_refund_or_io() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(0);
    let s = f.session.as_ref().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let (mut runtime, request) = supervision_capacity_test_http(&f.h.context);
    let claim = claim_agent_http_request(&f.h.context, &runtime, &request, &request.url)
        .unwrap()
        .unwrap();
    assert_eq!(claim.ordinal, 1);
    let debt = budget::balance(&db, &s.lease.root_run_id, None, "target_requests").unwrap();
    assert_eq!(debt.indeterminate, 1);
    let snapshot = db.unchecked_transaction().unwrap();
    let report = serde_json::to_value(
        budget::supervision_capacity::diagnostic(&snapshot, &s.lease.root_run_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(report["pendingPhysicalRequests"], 1);
    assert_eq!(report["freshWorkBlocked"], "cost_reconciliation_required");
    assert_eq!(
        report["nextHttpRequestBalanceSufficient"], true,
        "numeric balance does not override unknown-cost refusal"
    );
    snapshot.commit().unwrap();
    runtime.target_requests = 1;
    let before = super::tests::application_table_snapshot(&db);
    let mut fresh = request;
    fresh
        .extra_headers
        .push(("X-Capacity-Test".into(), "distinct-request".into()));
    let error = claim_agent_http_request(&f.h.context, &runtime, &fresh, &fresh.url).unwrap_err();
    assert_eq!(error, "budget_indeterminate_requires_reconciliation");
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(
        budget::balance(&db, &s.lease.root_run_id, None, "target_requests")
            .unwrap()
            .indeterminate,
        1
    );
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 3);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn supervision_capacity_report_requires_one_snapshot_and_omits_legacy_protocol() {
    let (_root, context, _runtime, _request) = http_journal_fixture("http://127.0.0.1:9/", 0);
    let db = db::open(&context.db_path).unwrap();
    let root: String = db
        .query_row(
            "SELECT root_run_id FROM agent_runs WHERE id=?1",
            [&context.run.as_ref().unwrap().run_id],
            |r| r.get(0),
        )
        .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&db, &root,)
            .unwrap_err(),
        "supervision_capacity_snapshot_required"
    );
    let snapshot = db.unchecked_transaction().unwrap();
    assert!(
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(
            &snapshot, &root,
        )
        .unwrap()
        .is_none()
    );
    snapshot.commit().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == before);
}

#[test]
fn supervision_capacity_actual_paid_publications_remove_only_their_original_http_obligations() {
    let (h, owned, _) = fresh_multi_anonymous_control_dispatch_with_extra(false, 1);
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    let db = db::open(&h.db_path).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    let claims: i64 = snapshot
        .query_row(
            "SELECT count(*) FROM agent_http_request_claims c
        JOIN agent_runs r ON r.id=c.run_id WHERE r.root_run_id=?1 AND r.role='web_executor'",
            [root],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(claims, 3, "three independently observed HTTP facts");
    let before = super::tests::application_table_snapshot(&snapshot);
    let calls = h.model_seen.lock().unwrap().len();
    let gaps = crate::agent_runtime::multi_agent::budget_gaps::budget_ledger_gaps_in_snapshot(
        &snapshot, root,
    )
    .unwrap();
    let report = serde_json::to_value(gaps).unwrap();
    let capacity = &report["supervisionRequestCapacity"];
    assert_eq!(
        capacity["pendingPhysicalRequests"], 0,
        "only verified paid original publications discharge obligations"
    );
    assert_eq!(capacity["nextHttpRequiredModelRequests"], 7);
    assert_eq!(capacity["executionAllowed"], false);
    assert_eq!(
        report["appendJournal"][8]["coverage"],
        "race_batch_dispatch_unimplemented"
    );
    assert!(super::tests::application_table_snapshot(&snapshot) == before);
    snapshot.commit().unwrap();
    assert_eq!(h.model_seen.lock().unwrap().len(), calls);
    assert_eq!(h.site_seen.lock().unwrap().len(), 4);
}

#[test]
fn supervision_capacity_single_report_keeps_actual_model_and_target_costs_without_v6_grant() {
    let mut h = fresh_single_production_harness("single-budget-inventory", |_| {
        (200, "text/plain", "one original target observation".into())
    });
    freeze_fresh_single_production_harness(&mut h);
    let target = h.context.target_url.clone();
    retarget_model(
        &mut h,
        vec![
            model_round(
                &[(
                    "replay_http",
                    json!({"identity":"anonymous","method":"GET",
            "url":target,"family":"authentication_session"}),
                )],
                100,
            ),
            model_round(
                &[(
                    "finish_target",
                    json!({"coverage":closing_ledger(&["authentication_session"]),
            "stopReason":"bounded original observation"}),
                )],
                100,
            ),
        ],
    );
    let owned = execute_fresh_single_production_harness(&mut h);
    assert!(
        matches!(
            owned.outcome,
            AgentTargetOutcome::Completed(_) | AgentTargetOutcome::BoundedCompleted(_)
        ),
        "{}",
        owned.outcome.detail()
    );
    let root = owned.original_terminal.root_run_id.as_ref().unwrap();
    let db = db::open(&h.db_path).unwrap();
    let snapshot = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&snapshot);
    let gaps = crate::agent_runtime::multi_agent::budget_gaps::budget_ledger_gaps_in_snapshot(
        &snapshot, root,
    )
    .unwrap();
    let report = serde_json::to_value(gaps).unwrap();
    assert!(
        report.get("supervisionRequestCapacity").is_none(),
        "Single is not v6 Multi"
    );
    let rows = report["appendJournal"].as_array().unwrap();
    let physical_models = h.model_seen.lock().unwrap().len();
    assert!(physical_models >= 2);
    // A rejected finish proposal may require additional original SDK rounds.
    // Reconcile the real provider arrivals, rather than assuming proposal count
    // is the number of physically paid requests.
    for (dimension, actual) in [("model_requests", physical_models as i64), ("target_requests", 1)] {
        let row = rows.iter().find(|r| r["dimension"] == dimension).unwrap();
        assert_eq!(row["consumed"], actual);
        assert_eq!(row["reserved"], 0);
        assert_eq!(row["indeterminate"], 0);
    }
    assert!(super::tests::application_table_snapshot(&snapshot) == before);
    snapshot.commit().unwrap();
    assert_eq!(h.model_seen.lock().unwrap().len(), physical_models);
    assert_eq!(h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn supervision_capacity_local_publication_keeps_obligation_until_final_decision() {
    struct ResetBoundary;
    impl Drop for ResetBoundary {
        fn drop(&mut self) {
            ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot| { slot.borrow_mut().take(); });
        }
    }
    let _reset = ResetBoundary;
    let _real = RealSpecialistTransport::enter();
    let f = new_evidence_closed_original_fixture("capacity-local-step");
    let db = db::open(&f.h.db_path).unwrap();
    let (id, _) = f.record(&db);
    let calls = f.calls();
    let mut rounds = 0;
    *f.new_evidence_reply.lock().unwrap() = Some(Box::new(move |_| {
        rounds += 1;
        if rounds == 1 {
            root_local_wire(json!([root_local_call("capacity-snapshot", "snapshot.read", json!({}))]), "", true)
        } else {
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original local snapshot checked"],
                "missing":[],"suggestions":["assess:new_evidence"],"costNotes":[],"risks":[]}).to_string())
        }
    }));
    let intermediate = std::sync::Arc::new(std::sync::Mutex::new(None));
    let captured = intermediate.clone();
    let path = f.h.db_path.clone();
    let root = f.root.clone();
    // The first actual paid checkpoint precedes its local-step publication.
    // Observe at the second paid checkpoint: the first local publication exists,
    // both SDK calls have original receipts, but the final decision is unpublished.
    ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            ROOT_TICK_AFTER_PAID_TEST_BOUNDARY.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    let db = db::open(&path).unwrap();
                    let tx = db.unchecked_transaction().unwrap();
                    let before = super::tests::application_table_snapshot(&tx);
                    let report = crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, &root)
                        .unwrap().unwrap();
                    *captured.lock().unwrap() = Some(serde_json::to_value(report).unwrap());
                    assert!(super::tests::application_table_snapshot(&tx) == before);
                }));
            });
        }));
    });
    let decision = native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &id).unwrap();
    assert!(native_root_paid_suggestion_selected(&decision, "assess:new_evidence"));
    assert_eq!(f.calls(), calls + 2);
    let intermediate = intermediate.lock().unwrap().take().expect("second actual paid checkpoint");
    assert_eq!(intermediate["pendingPhysicalRequests"], 1,
        "paid local tools are intermediate reasoning, not a completed supervision decision");
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let final_report = crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, &f.root)
        .unwrap().unwrap();
    assert_eq!(serde_json::to_value(final_report).unwrap()["pendingPhysicalRequests"], 0);
    assert!(super::tests::application_table_snapshot(&tx) == before);
    drop(tx);
    let calls = f.calls();
    assert!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &id).unwrap().replayed);
    assert_eq!(f.calls(), calls, "replay does not buy another local or final SDK call");
}

#[test]
fn supervision_capacity_revoked_original_worker_cannot_use_available_headroom() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(0);
    let s = f.session.as_ref().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let (runtime, request) = supervision_capacity_test_http(&f.h.context);
    assert_eq!(
        db.execute(
            "UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime')
        WHERE assignment_id=?1 AND capability='replay_http'",
            [&s.executor.assignment_id]
        )
        .unwrap(),
        1
    );
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        claim_agent_http_request(&f.h.context, &runtime, &request, &request.url).unwrap_err(),
        "tool_capability_or_fencing_denied"
    );
    assert!(super::tests::application_table_snapshot(&db) == before);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 3);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn supervision_capacity_local_step_then_unknown_final_keeps_obligation_and_debt() {
    let _real = RealSpecialistTransport::enter();
    let f = new_evidence_closed_original_fixture("capacity-local-unknown");
    let db = db::open(&f.h.db_path).unwrap();
    let (node, _) = f.record(&db);
    let calls = f.calls();
    let consumed = f.requests(&db).consumed;
    let publications = root_tick_count(&db, &f.root, "publication");
    let mut rounds = 0;
    *f.new_evidence_reply.lock().unwrap() = Some(Box::new(move |_| {
        rounds += 1;
        if rounds == 1 {
            root_local_wire(json!([root_local_call("capacity-unknown-snapshot", "snapshot.read", json!({}))]), "", true)
        } else {
            let mut reply: JsonValue = serde_json::from_str(&proposal_model_response(
                &json!({"schemaVersion":1,"observed":["paid original local snapshot"],
                    "missing":[],"suggestions":["assess:new_evidence"],"costNotes":[],"risks":[]}).to_string(),
            )).unwrap();
            reply.as_object_mut().unwrap().remove("usage");
            reply.to_string()
        }
    }));
    let error = native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).err().unwrap();
    assert_eq!(error, "budget_indeterminate_requires_reconciliation");
    assert_eq!(f.calls(), calls + 2);
    assert_eq!(f.requests(&db).consumed, consumed + 2, "both provider arrivals retain their known request charges");
    assert_eq!(f.requests(&db).indeterminate, 0);
    assert_eq!(root_tick_count(&db, &f.root, "publication"), publications + 1, "only the paid intermediate step is published");
    let unknown = crate::agent_runtime::multi_agent::budget::balance(&db, &f.root, Some(""), "model_input_tokens").unwrap().indeterminate;
    assert!(unknown > 0, "unknown final usage remains an original financial obligation");
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let report = serde_json::to_value(
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, &f.root).unwrap().unwrap()
    ).unwrap();
    assert_eq!(report["pendingPhysicalRequests"], 1);
    assert_eq!(report["freshWorkBlocked"], "cost_reconciliation_required");
    assert_eq!(report["executionAllowed"], false);
    assert!(super::tests::application_table_snapshot(&tx) == before);
    drop(tx);
    let before = client_delivery_all_rows(&db);
    assert!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).is_err());
    assert_eq!(client_delivery_all_rows(&db), before);
    assert_eq!(f.calls(), calls + 2, "unknown final cost never causes an automatic redispatch");
    assert_eq!(crate::agent_runtime::multi_agent::budget::balance(&db, &f.root, Some(""), "model_input_tokens").unwrap().indeterminate, unknown);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn supervision_capacity_report_observes_unsampled_original_deadline_without_writes() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|request| {
        (200, "application/json", changed_fact_response(&request, true))
    }));
    // Freeze the real deadline before the first Root INSERT; never move an
    // origin, expand a timeout or synthesize a terminal elapsed observation.
    let mut f = root_tick_fixture_protocol_limits_timeout(
        "capacity-unsampled-deadline", &format!("http://127.0.0.1:{port}/v1"),
        true, (60_000, 20), Some(8),
    );
    drop(f.parent.take());
    let session = multi_agent_prepare(&mut f.context).unwrap();
    let db = db::open(&f.context.db_path).unwrap();
    let root = &session.lease.root_run_id;
    assert_eq!(seen.lock().unwrap().len(), 3);
    crate::agent_runtime::multi_agent::budget::clock::remaining(&db, root).unwrap();
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match crate::agent_runtime::multi_agent::budget::clock::remaining(&db, root) {
            Ok(_) => {
                assert!(std::time::Instant::now() < limit, "original deadline must expire");
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(error) => {
                assert_eq!(error, "budget_wall_time_exhausted");
                break;
            }
        }
    }
    assert!(crate::agent_runtime::multi_agent::budget::clock::elapsed_fact::read(&db, root).unwrap().is_none());
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let report = serde_json::to_value(
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, root).unwrap().unwrap()
    ).unwrap();
    assert_eq!(report["freshWorkBlocked"], "wall_time_exhausted");
    assert_eq!(report["executionAllowed"], false);
    assert!(super::tests::application_table_snapshot(&tx) == before, "expired-clock inspection cannot append elapsed fees or change authority");
    assert_eq!(seen.lock().unwrap().len(), 3);
}

#[test]
fn supervision_capacity_three_paid_local_steps_keep_unfinished_obligation_on_replay() {
    let _real = RealSpecialistTransport::enter();
    let f = new_evidence_closed_original_fixture("capacity-three-local-steps");
    let db = db::open(&f.h.db_path).unwrap();
    let (node, _) = f.record(&db);
    let calls = f.calls();
    let consumed = f.requests(&db).consumed;
    let publications = root_tick_count(&db, &f.root, "publication");
    let mut rounds = 0;
    *f.new_evidence_reply.lock().unwrap() = Some(Box::new(move |_| {
        rounds += 1;
        root_local_wire(json!([root_local_call(&format!("capacity-bounded-snapshot-{rounds}"), "snapshot.read", json!({}))]), "", true)
    }));
    let error = native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).err().unwrap();
    assert_eq!(error, "root_local_react_round_limit");
    assert_eq!(f.calls(), calls + 3);
    assert_eq!(f.requests(&db).consumed, consumed + 3);
    assert_eq!(root_tick_count(&db, &f.root, "publication"), publications + 3);
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let report = serde_json::to_value(
        crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, &f.root).unwrap().unwrap()
    ).unwrap();
    assert_eq!(report["pendingPhysicalRequests"], 1, "three paid intermediate steps cannot fake a completed supervision decision");
    assert_eq!(report["executionAllowed"], false);
    assert_eq!(report["futureTokenCapacityGuaranteed"], false);
    assert!(super::tests::application_table_snapshot(&tx) == before);
    drop(tx);
    let before = client_delivery_all_rows(&db);
    assert_eq!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).err().unwrap(), error);
    assert_eq!(client_delivery_all_rows(&db), before);
    assert_eq!(f.calls(), calls + 3, "original round bound remains frozen on reentry");
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
}

#[test]
fn supervision_capacity_v6_owner_refuses_valid_old_sidecar_instead_of_omitting_guard() {
    let _real = RealSpecialistTransport::enter();
    let f = budget_trigger_fixture(0);
    let session = f.session.as_ref().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let root = &session.lease.root_run_id;
    let original = super::tests::application_table_snapshot(&db);
    let original_birth = original_root_capture_mode(&db, root).unwrap();
    let birth_row: (String, Vec<u8>) = db.query_row("SELECT binding_json,signature FROM agent_root_mode_birth_seals WHERE root_run_id=?1", [root], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    let mode = crate::agent_runtime::web_mode::root::read(&db, root).unwrap().unwrap();
    let mut raw = serde_json::to_value(mode).unwrap();
    // One intentionally cold sidecar uses the complete canonical historical
    // v5 contract, with no later optional rules accidentally left attached.
    raw["bootstrapDispatch"] = json!({
        "schemaVersion":5,"permittedStep":"dispatch:spa_api_mapper",
        "reservedModelTokens":8000,"reservedModelRequests":1,
        "allocationRule":"current_balance_after_reviewer_floor",
        "executorAllocationRule":"current_balance_after_reviewer_floor_and_ordered_proposals",
        "rootSupervisionTokenFloor":16000,"rootSupervisionRequestFloor":1,
        "modelCostRule":"finite_grant_share_native_unlimited_no_scarcity"
    });
    let old: crate::agent_runtime::web_mode::root::NewRootModeDeclaration = serde_json::from_value(raw).unwrap();
    let old_text = serde_json::to_string(&old).unwrap();
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate).unwrap();
    // Cold corruption only in a disposable rollback transaction. The private
    // original financial owner and birth seal stay untouched; this is never a legitimate downgrade.
    tx.execute_batch("DROP TRIGGER root_mode_immutable_update").unwrap();
    assert_eq!(tx.execute("UPDATE agent_root_mode_definitions SET definition_json=?2 WHERE root_run_id=?1", params![root,old_text]).unwrap(), 1);
    assert_eq!(crate::agent_runtime::web_mode::root::read(&tx, root).unwrap().unwrap().bootstrap_dispatch().unwrap()["schemaVersion"], 5,
        "the old sidecar is internally valid but cannot change the original v6 owner");
    let unchanged_birth: (String, Vec<u8>) = tx.query_row("SELECT binding_json,signature FROM agent_root_mode_birth_seals WHERE root_run_id=?1", [root], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(unchanged_birth, birth_row, "cold sidecar never changes the original signed birth");
    assert!(crate::agent_runtime::web_mode::root::verify_birth(&tx, root, &original_birth).is_err(), "a valid v5 declaration cannot replace this Root's original signed contract");
    let broken = super::tests::application_table_snapshot(&tx);
    assert_eq!(crate::agent_runtime::multi_agent::budget::supervision_capacity::require_http_headroom(
        &tx, &session.lease, &session.executor.assignment_id, &session.executor.run_id,
    ).unwrap_err(), "root_supervision_request_contract_changed");
    assert_eq!(crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, root).unwrap_err(), "root_supervision_request_contract_changed");
    assert!(super::tests::application_table_snapshot(&tx) == broken);
    tx.rollback().unwrap();
    assert!(super::tests::application_table_snapshot(&db) == original);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), 3);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn supervision_capacity_ambiguous_paid_usage_keeps_original_obligation_and_blocks_redispatch() {
    let _real = RealSpecialistTransport::enter();
    for kind in ["aliases", "duplicate-field", "duplicate-object"] {
    let f = new_evidence_closed_original_fixture(&format!("capacity-ambiguous-usage-{kind}"));
    let db = db::open(&f.h.db_path).unwrap();
    let (node, _) = f.record(&db);
    let calls = f.calls();
    let consumed = f.requests(&db).consumed;
    let publications = root_tick_count(&db, &f.root, "publication");
    *f.new_evidence_reply.lock().unwrap() = Some(Box::new(move |_| {
        let mut reply: JsonValue = serde_json::from_str(&proposal_model_response(
            &json!({"schemaVersion":1,"observed":["original physical response"],
                "missing":[],"suggestions":["assess:new_evidence"],"costNotes":[],"risks":[]}).to_string(),
        )).unwrap();
        let original = reply["usage"]["prompt_tokens"].as_i64().unwrap();
        if kind == "aliases" {
            reply["usage"]["input_tokens"] = json!(original + 1);
            reply.to_string()
        } else if kind == "duplicate-field" {
            reply.to_string().replacen(
                &format!("\"prompt_tokens\":{original}"),
                &format!("\"prompt_tokens\":{},\"prompt_tokens\":{original}", original + 1), 1)
        } else {
            reply.to_string().replacen("\"usage\":", "\"usage\":{},\"usage\":", 1)
        }
    }));
    assert_eq!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).err().unwrap(),
        "budget_indeterminate_requires_reconciliation");
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(f.requests(&db).consumed, consumed + 1);
    assert_eq!(root_tick_count(&db, &f.root, "publication"), publications,
        "ambiguous financial response must not publish its model conclusion");
    let unknown = crate::agent_runtime::multi_agent::budget::balance(&db, &f.root, Some(""), "model_input_tokens").unwrap().indeterminate;
    assert!(unknown > 0);
    let tx = db.unchecked_transaction().unwrap();
    let before = super::tests::application_table_snapshot(&tx);
    let report = serde_json::to_value(crate::agent_runtime::multi_agent::budget::supervision_capacity::diagnostic(&tx, &f.root).unwrap().unwrap()).unwrap();
    assert_eq!(report["pendingPhysicalRequests"], 1);
    assert_eq!(report["freshWorkBlocked"], "cost_reconciliation_required");
    assert!(super::tests::application_table_snapshot(&tx) == before);
    drop(tx);
    let original = client_delivery_all_rows(&db);
    assert!(native_coordinator_new_evidence_feedback(&f.h.context, f.lease(), &node).is_err());
    assert_eq!(client_delivery_all_rows(&db), original);
    assert_eq!(f.calls(), calls + 1);
    assert_eq!(f.h.site_seen.lock().unwrap().len(), 1);
    }
}
