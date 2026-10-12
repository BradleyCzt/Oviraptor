// Production Web entry: original v9 birth, actual provider and local HTTP only.
fn controlled_executor_web_actual_fixture(
    invalid_first: bool,
) -> (BudgetTriggerFixture, AgentTargetOutcome) {
    controlled_executor_web_actual_fixture_fault(invalid_first, None)
}

fn controlled_executor_web_actual_fixture_fault(
    invalid_first: bool,
    fault: Option<&'static str>,
) -> (BudgetTriggerFixture, AgentTargetOutcome) {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let mut h = fresh_multi_production_harness_with_site("controlled-web-v9", |wire| {
        assert!(wire
            .lines()
            .next()
            .unwrap()
            .starts_with("GET /controlled HTTP/"));
        (
            200,
            "application/json",
            json!({"controlledNonce":"original-web-response-20261009"}).to_string(),
        )
    });
    let target = h.context.target_url.clone();
    let site = h.site_seen.clone();
    let rounds = AtomicUsize::new(0);
    let fault_db = h.db_path.clone();
    let (port, seen, stop) = spawn_endpoint(Arc::new(move |wire| {
        let body: JsonValue = serde_json::from_str(wire.split_once("\r\n\r\n").unwrap().1).unwrap();
        let response = if wire.contains("You are the Root Coordinator") {
            let input: JsonValue =
                serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
            let step = match input["basis"]["phase"].as_str().unwrap() {
                "bootstrap" => "dispatch:spa_api_mapper",
                "mapper-output" => "dispatch:web_executor",
                "budget-allocation" => "assess:budget_allocation",
                "budget-consumption" => "assess:budget_consumption",
                "http-budget-consumption" => "assess:http_budget_consumption",
                "capability-changed" => "assess:capability_changed",
                "new-evidence" => "assess:new_evidence",
                "protection-changed" => "assess:protection_changed",
                other => panic!("unexpected original Root phase: {other}"),
            };
            proposal_model_response(&json!({"schemaVersion":1,"observed":["original paid execution fact"],"missing":[],"suggestions":[step],"costNotes":[],"risks":[]}).to_string())
        } else if wire.contains("SPA/API Mapper") {
            proposal_model_response(
                r#"{"summary":"original frozen Mapper","priorityContracts":[],"risks":[]}"#,
            )
        } else {
            let round = rounds.fetch_add(1, Ordering::SeqCst);
            if round == 0 {
                if fault == Some("503") {
                    return (
                        503,
                        "application/json",
                        json!({"error":{"message":"actual Web provider unavailable"}}).to_string(),
                    );
                }
                if matches!(fault, Some("withdraw" | "withdraw-text")) {
                    let db = db::open(&fault_db).unwrap();
                    assert_eq!(db.execute("UPDATE agent_coordinator_leases SET lease_epoch=lease_epoch+1,fencing_token=?1",[Uuid::new_v4().to_string()]).unwrap(),1);
                }
            }

            if fault == Some("withdraw-text") && round == 0 {
                return (
                    200,
                    "application/json",
                    sdk_exact_model_body(
                        "actual paid text-only response after original C withdrawal",
                    ),
                );
            }
            if invalid_first && round == 0 {
                model_round(
                    &[(
                        "replay_http",
                        json!({"identity":"anonymous","method":"GET","url":42,"family":"authorization"}),
                    )],
                    10,
                )
            } else if round == usize::from(invalid_first) {
                if invalid_first {
                    assert!(
                        site.lock().unwrap().is_empty(),
                        "invalid tool arguments cannot dispatch HTTP"
                    );
                    let errors: Vec<JsonValue> = body["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|m| m["role"] == "tool")
                        .map(|m| serde_json::from_str(m["content"].as_str().unwrap()).unwrap())
                        .collect();
                    assert!(
                        errors.iter().any(|e| e["code"] == "invalid_arguments"
                            && e["errorFingerprint"]
                                .as_str()
                                .is_some_and(|s| !s.is_empty())
                            && e["schemaErrors"]
                                .as_array()
                                .is_some_and(|items| items
                                    .iter()
                                    .any(|item| item["path"] == "$.url"
                                        && item["keyword"] == "type"
                                        && item["actual"] == "integer:42"))),
                        "second actual SDK must receive the deterministic rejection: {errors:?}"
                    );
                }
                model_round(
                    &[(
                        "replay_http",
                        json!({"identity":"anonymous","method":"GET","url":format!("{target}/controlled"),"family":"authorization"}),
                    )],
                    10,
                )
            } else {
                assert_eq!(site.lock().unwrap().len(), 1);
                let observations: Vec<&JsonValue> = body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|m| m["role"] == "tool" && m["name"] == "replay_http")
                    .collect();
                assert!(observations.iter().any(|m|m["content"].as_str().is_some_and(|s|s.contains("original-web-response-20261009"))),"later actual SDK must observe the original response artifact, not a synthetic outcome");
                model_round(&[], 10)
            }
        };
        (200, "application/json", response)
    }));
    h.context.environment.api_base = format!("http://127.0.0.1:{port}/v1");
    h.model_seen = seen;
    h.context.execution_plan.hard_total_tokens = 60000;
    h.context.execution_plan.soft_uncached_tokens = 30000;
    h.context.execution_plan.hard_model_requests = 20;
    h.context.execution_plan.soft_model_requests = 10;
    h.context.execution_plan.max_turns = 8;
    freeze_fresh_multi_production_harness_web_expiry_v9(&mut h);
    let session = multi_agent_prepare(&mut h.context).unwrap();
    let f = BudgetTriggerFixture {
        h,
        session: Some(session),
        boundary: Arc::new(std::sync::Mutex::new(None)),
        stop,
    };
    let outcome = run_native_agent(&f.h.context);
    (f, outcome)
}

fn controlled_executor_web_assert_actual_evidence(
    f: &BudgetTriggerFixture,
    outcome: &AgentTargetOutcome,
    minimum_rounds: i64,
) {
    use crate::agent_runtime::multi_agent::budget;
    assert!(
        matches!(
            outcome,
            AgentTargetOutcome::Incomplete(_) | AgentTargetOutcome::BoundedCompleted(_)
        ),
        "actual original bounded loop: {}",
        outcome.detail()
    );
    assert_eq!(
        f.h.site_seen.lock().unwrap().len(),
        1,
        "actual outcome: {}; SDK calls: {}",
        outcome.detail(),
        f.h.model_seen.lock().unwrap().len()
    );
    let session = f.session.as_ref().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let rounds:i64=db.query_row("SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'",[&session.executor.run_id],|r|r.get(0)).unwrap();
    assert!(
        rounds >= minimum_rounds,
        "original paid Web rounds={rounds}, required={minimum_rounds}"
    );
    let b = budget::balance(
        &db,
        &session.lease.root_run_id,
        Some(&session.executor.assignment_id),
        "target_requests",
    )
    .unwrap();
    assert_eq!((b.consumed, b.reserved, b.indeterminate), (1, 0, 0));
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1 AND response_status=200 AND received_at<>''",
            [&session.executor.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let source:String=db.query_row("SELECT source_id FROM agent_http_budget_receipts WHERE root_run_id=?1 AND run_id=?2 AND response_status=200",params![session.lease.root_run_id,session.executor.run_id],|r|r.get(0)).unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    let receipt = crate::agent_runtime::web_mode::http_budget_storage::read_original_on(
        &tx,
        &session.lease,
        &source,
    )
    .unwrap()
    .unwrap();
    assert_eq!(receipt.fact()["status"], 200);
    assert_eq!(receipt.fact()["targetRequests"], 1);
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_evidence_nodes WHERE root_run_id=?1 AND created_by_run_id=?2 AND kind='request_record'",params![session.lease.root_run_id,session.executor.run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
}

#[test]
fn controlled_executor_web_actual_sdk_http_observation_then_bounded_exit() {
    let _real = RealSpecialistTransport::enter();
    let (f, outcome) = controlled_executor_web_actual_fixture(false);
    controlled_executor_web_assert_actual_evidence(&f, &outcome, 2);
}

#[test]
fn controlled_executor_web_actual_invalid_arguments_observed_then_corrected_http() {
    let _real = RealSpecialistTransport::enter();
    let (f, outcome) = controlled_executor_web_actual_fixture(true);
    controlled_executor_web_assert_actual_evidence(&f, &outcome, 3);
}

#[test]
fn controlled_executor_web_actual_coordinator_withdrawn_after_paid_sdk_cannot_send_http() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let (f, outcome) = controlled_executor_web_actual_fixture_fault(false, Some("withdraw"));
    assert!(
        !matches!(
            outcome,
            AgentTargetOutcome::Completed(_) | AgentTargetOutcome::BoundedCompleted(_)
        ),
        "{}",
        outcome.detail()
    );
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    let session = f.session.as_ref().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let model = budget::balance(
        &db,
        &session.lease.root_run_id,
        Some(&session.executor.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        model.consumed, 1,
        "the real received model bill survives withdrawal"
    );
    assert_eq!(model.indeterminate, 0);
    let target = budget::balance(
        &db,
        &session.lease.root_run_id,
        Some(&session.executor.assignment_id),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (target.consumed, target.reserved, target.indeterminate),
        (0, 0, 0)
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='received'",[&session.executor.run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1",
            [&session.executor.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn controlled_executor_web_actual_provider_503_preserves_unknown_bill_and_entry_does_not_retry() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let (f, outcome) = controlled_executor_web_actual_fixture_fault(false, Some("503"));
    assert!(
        !matches!(
            outcome,
            AgentTargetOutcome::Completed(_) | AgentTargetOutcome::BoundedCompleted(_)
        ),
        "{}",
        outcome.detail()
    );
    let session = f.session.as_ref().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let before = budget::balance(
        &db,
        &session.lease.root_run_id,
        Some(&session.executor.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!((before.consumed, before.indeterminate), (0, 1));
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    let sdk = f.h.model_seen.lock().unwrap().len();
    let dispatches:i64=db.query_row("SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='dispatch'",[&session.executor.run_id],|r|r.get(0)).unwrap();
    assert_eq!(dispatches, 1);
    let retry = run_native_agent(&f.h.context);
    assert!(
        !matches!(
            retry,
            AgentTargetOutcome::Completed(_) | AgentTargetOutcome::BoundedCompleted(_)
        ),
        "{}",
        retry.detail()
    );
    let AgentTargetOutcome::Incomplete(stop) = &retry else {
        panic!(
            "original unknown cost must stop before SDK: {}",
            retry.detail()
        );
    };
    assert_eq!(stop.code, terminal_code::REQUEST_RECONCILIATION_REQUIRED);
    // The machine category is stop.code. stop.reason/detail is the original
    // localized manual-reconciliation guidance, not the lower-level SQL guard.
    assert!(stop.reason.contains("预算保留，不自动重放"));
    let after = budget::balance(
        &db,
        &session.lease.root_run_id,
        Some(&session.executor.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (after.consumed, after.reserved, after.indeterminate),
        (before.consumed, before.reserved, before.indeterminate)
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase='dispatch'",[&session.executor.run_id],|r|r.get::<_,i64>(0)).unwrap(),dispatches);
    assert_eq!(f.h.model_seen.lock().unwrap().len(), sdk);
    assert!(f.h.site_seen.lock().unwrap().is_empty());
}

#[test]
fn controlled_executor_web_actual_paid_text_only_after_coordinator_withdrawal_cannot_continue() {
    use crate::agent_runtime::multi_agent::budget;
    let _real = RealSpecialistTransport::enter();
    let (f, outcome) = controlled_executor_web_actual_fixture_fault(false, Some("withdraw-text"));
    let AgentTargetOutcome::ResumeIncompatible(stop) = &outcome else {
        panic!("actual withdrawn text-only stop: {}", outcome.detail());
    };
    assert_eq!(stop.code, terminal_code::RESUME_INCOMPATIBLE);
    assert_eq!(
        stop.reason,
        "原模型账单已保存；此执行分支的权限已失效，停止使用该响应及派发后续动作"
    );
    assert!(f.h.site_seen.lock().unwrap().is_empty());
    let session = f.session.as_ref().unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let model = budget::balance(
        &db,
        &session.lease.root_run_id,
        Some(&session.executor.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!((model.consumed, model.indeterminate), (1, 0));
    let target = budget::balance(
        &db,
        &session.lease.root_run_id,
        Some(&session.executor.assignment_id),
        "target_requests",
    )
    .unwrap();
    assert_eq!(
        (target.consumed, target.reserved, target.indeterminate),
        (0, 0, 0)
    );
    for phase in ["dispatch", "received"] {
        assert_eq!(
            db.query_row(
                "SELECT count(*) FROM agent_web_model_journal WHERE child_run_id=?1 AND phase=?2",
                params![session.executor.run_id, phase],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_http_request_claims WHERE run_id=?1",
            [&session.executor.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM tool_invocations WHERE run_id=?1",
            [&session.executor.run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_receipts q WHERE q.root_run_id=?1 AND q.phase='publication' AND q.round>(SELECT COALESCE(max(x.round),0) FROM agent_root_tick_receipts x WHERE x.root_run_id=q.root_run_id AND x.phase='request' AND json_extract(x.fact_json,'$.request.basis.phase')='budget-allocation')",[&session.lease.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),0,"no business publication after the paid revoked Web response");
}
