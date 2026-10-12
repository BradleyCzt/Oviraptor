// Actual ordinary creator/HMAC and original Root/C/control/limits at birth.
// This fixture adds genuine Root SDK calls; child counts exclude them explicitly.
fn specialist_original_context(
    tag: &str,
    endpoint: &str,
) -> (
    PathBuf,
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    specialist_original_context_limits(tag, endpoint, 60_000, 20, &json!({}))
}
// Same actual creator/HMAC/frozen Web Root path, with the caller's original
// hard token/request ceilings passed through unchanged.
// `evidence_patch` keys are part of the original frontend evidence frozen
// before the Root's first tick (never edited afterwards).
fn specialist_original_context_limits(
    tag: &str,
    endpoint: &str,
    hard_tokens: i64,
    hard_requests: i64,
    evidence_patch: &JsonValue,
) -> (
    PathBuf,
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) {
    let (root, path, run_id, actor) = multi_agent_new_task_root(tag, hard_tokens, hard_requests);
    let db = db::open(&path).unwrap();
    let mut context = test_context(&path, &actor.target_key, vec![AgentIdentity::anonymous()]);
    context.scan_id = actor.scan_id.clone();
    context.execution_plan =
        web_mode_positive_plan_or(&path, &actor.target_key, context.execution_plan);
    for (key, value) in evidence_patch.as_object().into_iter().flatten() {
        context.evidence[key] = value.clone();
    }
    let work: String = db
        .query_row(
            "SELECT work_dir FROM sentinel_scan_attempts WHERE scan_id=?1 AND attempt_number=1",
            [&actor.scan_id],
            |r| r.get(0),
        )
        .unwrap();
    context.target_dir = PathBuf::from(work).join("url-pipeline/target-00001");
    fs::create_dir_all(&context.target_dir).unwrap();
    fs::write(
        context.target_dir.join(".oviraptor-scan-id"),
        &actor.scan_id,
    )
    .unwrap();
    fs::write(
        context.target_dir.join("frontend-evidence.json"),
        context.evidence.to_string(),
    )
    .unwrap();
    context.environment.api_base = endpoint.into();
    context.run = Some(AgentRunLedger {
        db_path: path,
        run_id,
    });
    bind_agent_evidence_location(&context).unwrap();
    native_frozen_web_root_mode(&context).unwrap();
    (root, context, actor)
}
type SpecialistTestHandler =
    std::sync::Arc<dyn Fn(String) -> (u16, &'static str, String) + Send + Sync>;
type SpecialistTestSeen = std::sync::Arc<std::sync::Mutex<Vec<String>>>;
fn specialist_original_endpoint(
    handler: SpecialistTestHandler,
) -> (
    u16,
    SpecialistTestSeen,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    spawn_endpoint(std::sync::Arc::new(move |request| {
        if request.contains("You are the Root Coordinator") {
            (
                200,
                "application/json",
                changed_fact_reviewer_response(&request, true),
            )
        } else if request.contains("SPA/API Mapper") {
            (
                200,
                "application/json",
                proposal_model_response(
                    r#"{"summary":"actual paid Mapper receipt","priorityContracts":[],"risks":[]}"#,
                ),
            )
        } else if request.contains("Original fixture Evidence Reviewer") {
            (
                200,
                "application/json",
                proposal_model_response(
                    r#"{"verdict":"insufficient_evidence","reasonCodes":["missing_control"],"missingEvidence":["missing control"],"confidence":0.2,"summary":"reviewer needs control"}"#,
                ),
            )
        } else {
            handler(request)
        }
    }))
}
// Requests the wrapped test handler itself answered: everything except the
// born Root, the fixture Mapper and the fixture sealing Reviewer that
// `specialist_original_endpoint` routes. Keeps a migrated test's original
// per-work SDK count (e.g. exactly one Investigator request) unchanged.
fn specialist_handler_call_count(seen: &SpecialistTestSeen) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|request| {
            !request.contains("You are the Root Coordinator")
                && !request.contains("SPA/API Mapper")
                && !request.contains("Original fixture Evidence Reviewer")
        })
        .count()
}
fn specialist_child_call_count(seen: &SpecialistTestSeen) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|request| !request.contains("You are the Root Coordinator"))
        .count()
}

// Migrated positive fixture: every caller unwraps prepare and later executes
// gap/investigator work, so it needs the actual creator/HMAC frozen Web Root,
// its real C/worker and the real Reviewer seal (`specialist_original_gap_fixture`)
// instead of a hand-rolled partial root. Original 60,000 tokens / 20 requests
// unchanged. The born Root's own bootstrap/changed-fact ticks, Mapper and the
// sealing Reviewer use this fixture-owned endpoint; callers attach their own
// endpoint for the work under test afterwards.
fn specialist_gap_fixture() -> (std::path::PathBuf, AgentRunContext, MultiAgentSession) {
    let (port, _seen, _stop) = specialist_original_endpoint(std::sync::Arc::new(|request| {
        (200, "application/json", changed_fact_reviewer_response(&request, true))
    }));
    specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"))
}

// Positive-fixture migration for former `multi_agent_test_root` callers that
// unwrap prepare: the actual frozen Web Root/C/worker from
// `specialist_original_context_limits` with the caller's original ceilings,
// plus a fixture endpoint (Root/Mapper/Reviewer routed by
// `specialist_original_endpoint`, other roles via the shared changed-fact
// responder). The returned Seen/stop keep that frozen endpoint alive.
fn multi_agent_original_test_root(
    tag: &str,
    hard_tokens: i64,
    hard_requests: i64,
) -> (
    PathBuf,
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    SpecialistTestSeen,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    multi_agent_original_test_root_with(tag, hard_tokens, hard_requests, &json!({}))
}
fn multi_agent_original_test_root_with(
    tag: &str,
    hard_tokens: i64,
    hard_requests: i64,
    evidence_patch: &JsonValue,
) -> (
    PathBuf,
    AgentRunContext,
    crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    SpecialistTestSeen,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let (port, seen, stop) =
        specialist_original_endpoint(std::sync::Arc::new(multi_agent_original_fake_response));
    let (root, context, lease) = specialist_original_context_limits(
        tag,
        &format!("http://127.0.0.1:{port}/v1"),
        hard_tokens,
        hard_requests,
        evidence_patch,
    );
    (root, context, lease, seen, stop)
}
// The former in-process test transport's Investigator/Reviewer replies, now
// served through the real gateway for born frozen Web Roots. Every other role
// (IdentitySession etc.) uses the shared changed-fact responder.
fn multi_agent_original_fake_response(request: String) -> (u16, &'static str, String) {
    if request.contains("独立的只读 Deep Investigator") {
        // Byte-for-byte the same Investigator proposal the former
        // in-process test transport returned (1000 tokens / 1 request /
        // 3 target requests), now delivered through the real gateway.
        let body: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input: JsonValue =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        return (200, "application/json", proposal_model_response(&json!({
            "summary": "test investigator assessed the reviewer's frozen evidence gap",
            "nextStep": "request_new_contract",
            "gapCode":"missing_owner_control", "supportingFactRefs":[],
            "missingEvidence":input["reviewerMissingEvidence"], "prerequisites":["operator control group"],
            "proposedContracts":["new_attempt_control_group_request"],
            "expectedInformationGain":0.5, "impactCeiling":"medium",
            "estimatedCost":{"modelTokens":1000,"modelRequests":1,"targetRequests":3},
            "sideEffectClass":"read_only", "overlapKeys":["owner-control"],
            "falsificationCondition":"owner access absent", "stopCondition":"scope rejection"
        }).to_string()));
    }
    if request.contains("你是独立 Evidence Reviewer。你没有网络") {
        // Same verdicts the former in-process test transport derived from
        // the frozen CandidateBundle, now through the real gateway.
        let body: JsonValue =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        let input: JsonValue =
            serde_json::from_str(body["messages"][1]["content"].as_str().unwrap()).unwrap();
        let decision = if input.pointer("/evidence/forceReviewerInsufficientForTest") == Some(&JsonValue::Bool(true)) {
            json!({"verdict":"insufficient_evidence","reasonCodes":["fixture_missing_control"],
                "missingEvidence":["ownership control"],"confidence":0.0,"summary":"test reviewer requested a verified control"})
        } else if input.pointer("/evidence/forceReviewerRejectedForTest") == Some(&JsonValue::Bool(true)) {
            json!({"verdict":"rejected","reasonCodes":["fixture_counterevidence"],"missingEvidence":[],
                "confidence":0.9,"summary":"test reviewer rejected the frozen candidate bundle"})
        } else {
            json!({"verdict":"confirmed","reasonCodes":["test_fixture_confirmed"],"missingEvidence":[],
                "confidence":1.0,"summary":"test reviewer confirmed the frozen candidate bundle"})
        };
        return (200, "application/json", proposal_model_response(&decision.to_string()));
    }
    (200, "application/json", changed_fact_reviewer_response(&request, true))
}
fn multi_agent_original_prepare(context: &mut AgentRunContext) -> MultiAgentSession {
    let _real = RealSpecialistTransport::enter();
    multi_agent_prepare(context).unwrap()
}

fn specialist_original_mapper_failure_endpoint(
    handler: SpecialistTestHandler,
) -> (
    u16,
    SpecialistTestSeen,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    spawn_endpoint(std::sync::Arc::new(move |request| {
        if request.contains("You are the Root Coordinator") {
            (
                200,
                "application/json",
                changed_fact_reviewer_response(&request, true),
            )
        } else {
            handler(request)
        }
    }))
}
fn seal_specialist_original_gap_review(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    context: &AgentRunContext,
) {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{mailbox, scheduler},
    };
    let child = scheduler::schedule_child(
        db,
        actor,
        AgentRole::EvidenceReviewer,
        AgentLane::Review,
        "candidate_ready",
        &json!({"candidateId":"candidate-gap","candidateRevision":2}),
        2,
        &["evidence.read".into(), "review.write".into()],
        8000,
        1,
    )
    .unwrap();
    scheduler::start_child_or_release(db, actor, &child).unwrap();
    let candidate = json!({"evidence":context.evidence,"persistedFactRefs":review_fact_refs(db,&actor.root_run_id,&context.target_dir).unwrap(),"findingCandidates":[]});
    open_review_request(
        db,
        actor,
        &child,
        "review-candidate-gap-2",
        "candidate-gap",
        2,
        &candidate.to_string(),
        &context.target_dir,
    )
    .unwrap();
    let (text, usage) = multi_agent_child_round_transport(
        context,
        actor,
        &child,
        "Original fixture Evidence Reviewer: inspect frozen evidence only",
        candidate,
    )
    .unwrap();
    let decision = validated_review_decision(&text).unwrap();
    assert_eq!(decision.verdict, "insufficient_evidence");
    assert_eq!(decision.missing_evidence, json!(["missing control"]));
    settle_child_usage(db, actor, &child, &usage).unwrap();
    let message = persist_review_decision(
        db,
        actor,
        &child,
        "review-candidate-gap-2",
        "candidate-gap",
        2,
        "insufficient_evidence",
        &json!(["missing_control"]),
        &decision.missing_evidence,
        0.2,
        &decision.summary,
        &context.target_dir,
    )
    .unwrap();
    mailbox::deliver_expected(db, actor, &actor.root_run_id, &message).unwrap();
    mailbox::acknowledge(db, actor, &actor.root_run_id, &message).unwrap();
    scheduler::finish_child(db, actor, &child, true, "review completed").unwrap();
}
