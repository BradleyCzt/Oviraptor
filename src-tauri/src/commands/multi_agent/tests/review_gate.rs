#[test]
fn review_gate_insufficiency_triggers_gap_run_without_releasing_target_capabilities() {
    // Same original 60,000/20 ceilings; one real Web turn at birth preserves
    // capacity for the actual Reviewer and independent gap SDKs.
    let (port,_seen,stop)=spawn_endpoint(std::sync::Arc::new(|wire| {
        if wire.contains("独立 Evidence Reviewer") {
            return (200,"application/json",proposal_model_response(&json!({"verdict":"insufficient_evidence","reasonCodes":["fixture_missing_control"],"missingEvidence":["ownership control"],"confidence":0.0,"summary":"test reviewer requested a verified control"}).to_string()));
        }
        if wire.contains("You are the Root Coordinator") || wire.contains("SPA/API Mapper") || wire.contains("独立的只读 Deep Investigator") {
            multi_agent_original_fake_response(wire)
        } else {(200,"application/json",model_round(&[],10))}
    }));
    let fixture=web_mode_fixture("multi","https://authorized.example.test");
    let mut f=root_tick_fixture_from_web_mode_turns("review-gap-trigger",&format!("http://127.0.0.1:{port}/v1"),true,(60_000,20),None,fixture,Some(1));
    drop(f.parent.take());
    let context=&mut f.context;
    let root_run_id = f.actor.root_run_id.clone();
    let connection = db::open(&context.db_path).unwrap();
    stage_agent_finding(context, "web", "evidence", "gap-trigger", "candidate", "info",
        &serde_json::json!({"fact":"fixture"})).unwrap();
    let mut session = multi_agent_original_prepare(context);
    let _real = RealSpecialistTransport::enter();
    let outcome = run_native_agent(context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{}",outcome.detail());
    multi_agent_finish_execution(context, &mut session, &outcome).unwrap();
    let reviewed = multi_agent_review(context, &mut session, outcome);
    assert!(matches!(reviewed, AgentTargetOutcome::Incomplete(_)), "{reviewed:?}");
    let gap_runs: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='deep_investigator' AND lane='read_only_analysis'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(gap_runs, 1,"{}",reviewed.detail());
    let target_leases: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_capability_leases WHERE root_run_id=?1 AND capability IN ('replay_http','compare_identities') AND revoked_at='' AND lease_expires_at>datetime('now','localtime')",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(target_leases, 0);
    drop(connection);
    drop(session);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn reviewer_and_replay_preserve_manual_execution_stop_without_gap_autostart() {
    for code in [terminal_code::REQUEST_RECONCILIATION_REQUIRED, terminal_code::EXECUTION_AUTHORIZATION_DENIED] {
        // Actual frozen Web Root/C/worker (original 60,000 tokens / 20 requests).
        let (port,seen,_stop) = specialist_original_endpoint(std::sync::Arc::new(|wire| {
            if specialist_original_is_web_sdk(&wire) {
                (200,"application/json",proposal_model_response("actual bounded Web exit"))
            } else { multi_agent_original_fake_response(wire) }
        }));
        let mut f = root_tick_fixture_from_web_mode_evidence(
            "review-execution-stop", &format!("http://127.0.0.1:{port}/v1"), true,(60_000,20),None,
            web_mode_fixture("multi","https://authorized.example.test"), RootFixtureEvidence {
                max_turns: Some(1), evidence_patch: Some(&serde_json::json!({"forceReviewerInsufficientForTest":true})),
                ..Default::default()
            });
        drop(f.parent.take());
        let lease = f.actor.clone();
        let context = &mut f.context;
        let root_run_id = lease.root_run_id.clone();
        let connection = db::open(&context.db_path).unwrap();
        stage_agent_finding(context, "web", "evidence", "manual-stop", "candidate", "info",
            &serde_json::json!({"fact":"fixture"})).unwrap();
        let mut session = multi_agent_original_prepare(context);
        let _real = RealSpecialistTransport::enter();
        let executed = run_native_agent(context);
        assert!(matches!(executed,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{executed:?}");
        assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
        let outcome = AgentTargetOutcome::Incomplete(AgentStop::new(code, "manual resolution required"));
        multi_agent_finish_execution(context, &mut session, &outcome).unwrap();
        for _ in 0..2 {
            let reviewed = multi_agent_review(context, &mut session, outcome.clone());
            assert_eq!(reviewed.terminal_code(), code, "{reviewed:?}");
        }
        let (decisions,gaps):(i64,i64)=connection.query_row(
            "SELECT (SELECT COUNT(*) FROM agent_review_decisions WHERE root_run_id=?1), \
             (SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='deep_investigator')",
            [&root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!((decisions,gaps),(1,0), "the Reviewer records its decision, but cannot auto-resolve an execution stop");
    }
}

#[test]
fn coverage_only_result_never_fabricates_a_reviewer_or_a_confirmed_decision() {
    // Actual frozen Web Root/C/worker (original 60,000 tokens / 20 requests).
    let (port,seen,_stop) = specialist_original_endpoint(std::sync::Arc::new(|wire| {
        if specialist_original_is_web_sdk(&wire) {
            (200,"application/json",proposal_model_response("actual bounded Web exit; no target tool"))
        } else { multi_agent_original_fake_response(wire) }
    }));
    let mut f = root_tick_fixture_from_web_mode_turns(
        "coverage-no-reviewer", &format!("http://127.0.0.1:{port}/v1"),
        true,(60_000,20),None,web_mode_fixture("multi","https://authorized.example.test"),Some(1),
    );
    drop(f.parent.take());
    let lease = f.actor.clone();
    let context = &mut f.context;
    let root_run_id = lease.root_run_id.clone();
    let connection = db::open(&context.db_path).unwrap();
    let mut session = multi_agent_original_prepare(context);
    let _real = RealSpecialistTransport::enter();
    let outcome = run_native_agent(context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
    multi_agent_finish_execution(context, &mut session, &outcome).unwrap();
    let usage: (i64,i64) = connection.query_row("SELECT used_tokens,used_requests FROM agent_runs WHERE id=?1", [&session.executor.run_id], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(usage,(20,1));
    assert!(matches!(multi_agent_review(context, &mut session, outcome),
        AgentTargetOutcome::Incomplete(_)));
    let (reviewers, requests, decisions): (i64, i64, i64) = connection.query_row(
        "SELECT (SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'), \
         (SELECT COUNT(*) FROM agent_review_requests WHERE root_run_id=?1), \
         (SELECT COUNT(*) FROM agent_review_decisions WHERE root_run_id=?1)",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!((reviewers, requests, decisions), (0, 0, 0));
    drop(connection);
    drop(session);
}

#[test]
fn insufficient_finding_remains_reviewable_only_after_a_new_candidate_revision() {
    // Actual frozen Web Root/C/worker (original 80,000 tokens / 30 requests).
    // The Reviewer-insufficient marker the first Review sees is frozen with the
    // original evidence before the Root's first tick instead of patched later.
    let (port,seen,_stop) = specialist_original_endpoint(std::sync::Arc::new(|wire| {
            if specialist_original_is_web_sdk(&wire) {
                (200,"application/json",proposal_model_response("actual bounded Web exit"))
            } else { multi_agent_original_fake_response(wire) }
        }));
        let mut f = root_tick_fixture_from_web_mode_evidence(
            "finding-gap-revision", &format!("http://127.0.0.1:{port}/v1"), true,(80_000,30),None,
            web_mode_fixture("multi","https://authorized.example.test"), RootFixtureEvidence {
                max_turns: Some(1), evidence_patch: Some(&serde_json::json!({"forceReviewerInsufficientForTest":true})),
                ..Default::default()
            });
        drop(f.parent.take());
        let lease = f.actor.clone();
        let context = &mut f.context;
    let root_run_id = lease.root_run_id.clone();
    let connection = db::open(&context.db_path).unwrap();
    stage_agent_finding(
        context, "web", "evidence", "gap", "candidate", "info",
        &serde_json::json!({"fact":"frozen candidate"}),
    ).unwrap();
    let mut session = multi_agent_original_prepare(context);
    let _real = RealSpecialistTransport::enter();
    let outcome = run_native_agent(context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    assert_eq!(seen.lock().unwrap().iter().filter(|wire| specialist_original_is_web_sdk(wire)).count(),1);
    multi_agent_finish_execution(context, &mut session, &outcome).unwrap();
    assert!(matches!(multi_agent_review(context, &mut session, outcome.clone()),
        AgentTargetOutcome::Incomplete(_)));
    let first: (String, i64) = connection.query_row(
        "SELECT status,candidate_revision FROM agent_finding_candidates WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(first, ("insufficient_evidence".into(), 1));
    assert_eq!(pending_agent_finding_candidates(&connection, &root_run_id).unwrap().len(), 1);
    let published: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND stage<>'native-agent-coverage'", [&lease.scan_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(published, 0);
    let coverage: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND stage='native-agent-coverage'",
        [&lease.scan_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(coverage, 1, "the actual Web exit persists coverage separately from candidate publication");

    let replayed = multi_agent_review(context, &mut session, outcome.clone());
    assert!(matches!(replayed, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.starts_with("review_gate_insufficient_evidence:")), "{replayed:?}");
    let reviewers: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(reviewers, 1, "a frozen revision must not spend another Reviewer call");
    connection.execute(
        "UPDATE agent_finding_candidates SET status='pending' WHERE root_run_id=?1",
        [&root_run_id],
    ).unwrap();
    let broken = multi_agent_review(context, &mut session, outcome.clone());
    assert!(matches!(broken, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.contains("review_replay_publication_incomplete_requires_recovery")), "{broken:?}");
    connection.execute(
        "UPDATE agent_finding_candidates SET status='insufficient_evidence' WHERE root_run_id=?1",
        [&root_run_id],
    ).unwrap();

    // Changing only model-visible prose cannot manufacture a new fact or spend
    // another Reviewer call after an insufficient decision. The born Root
    // compares live context with its frozen evidence, so the change is refused
    // by original lease authority before replay/Reviewer/decision/revision.
    let frozen_evidence = context.evidence.clone();
    let decisions_before: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_review_decisions WHERE root_run_id=?1", [&root_run_id], |row| row.get(0),
    ).unwrap();
    context.evidence.as_object_mut().unwrap().remove("forceReviewerInsufficientForTest");
    context.evidence["newVerifiedEvidence"] = serde_json::json!({"factRef":"fixture:2"});
    let unchanged = multi_agent_review(context, &mut session, outcome.clone());
    assert!(matches!(unchanged, AgentTargetOutcome::Failed(ref reason)
        if reason.reason == "review_gate_lease:root_tick_original_evidence_conflict"), "{unchanged:?}");
    let reviewers: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND role='evidence_reviewer'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(reviewers, 1);
    let decisions_after: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_review_decisions WHERE root_run_id=?1", [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(decisions_after, decisions_before);
    let unchanged_candidate: (String, i64) = connection.query_row(
        "SELECT status,candidate_revision FROM agent_finding_candidates WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(unchanged_candidate, ("insufficient_evidence".into(), 1));
    // Continue with the original frozen evidence for the fact-graph checks.
    context.evidence = frozen_evidence;
    let unchanged = multi_agent_review(context, &mut session, outcome.clone());
    assert!(matches!(unchanged, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.starts_with("review_gate_insufficient_evidence:")), "{unchanged:?}");

    use crate::agent_runtime::evidence_graph::{
        contract::{EvidenceNode, EvidenceNodeKind, EvidenceProvenance},
        store::insert_evidence_node,
    };
    let mut fact = EvidenceNode {
        id: "fixture-gap-fact".into(),
        root_run_id: root_run_id.clone(),
        revision: 2,
        kind: EvidenceNodeKind::RequestRecord,
        provenance: EvidenceProvenance::Inferred,
        natural_key_hash: crate::agent_runtime::store::stable_hash("fixture-gap-fact"),
        payload: serde_json::json!({"request":"fixture:2"}),
        artifact_refs: vec!["fixture-artifact:2".into()],
        created_by_run_id: session.executor.run_id.clone(),
        supersedes_id: String::new(),
        created_at: String::new(),
    };
    insert_evidence_node(&connection, &fact).unwrap();
    let inferred = multi_agent_review(context, &mut session, outcome.clone());
    assert!(matches!(inferred, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.starts_with("review_gate_insufficient_evidence:")), "{inferred:?}");
    fact.id = "fixture-gap-unlinked-observation".into();
    fact.natural_key_hash = crate::agent_runtime::store::stable_hash(&fact.id);
    fact.provenance = EvidenceProvenance::Observed;
    fact.artifact_refs.clear();
    insert_evidence_node(&connection, &fact).unwrap();
    let unlinked = multi_agent_review(context, &mut session, outcome.clone());
    assert!(matches!(unlinked, AgentTargetOutcome::Incomplete(ref reason)
        if reason.reason.starts_with("review_gate_insufficient_evidence:")), "{unlinked:?}");
    fact.id = "fixture-gap-observation".into();
    fact.natural_key_hash = crate::agent_runtime::store::stable_hash(&fact.id);
    fact.artifact_refs = vec!["fixture-artifact:2".into()];
    insert_evidence_node(&connection, &fact).unwrap();
    // The Mapper can store source-derived graph nodes, but a nonempty string
    // is not proof that a source artifact exists or belongs to this attempt.
    // It must not reopen an insufficient Review without a source verifier.
    fact.id = "fixture-gap-mapper-unverified".into();
    fact.natural_key_hash = crate::agent_runtime::store::stable_hash(&fact.id);
    fact.kind = EvidenceNodeKind::Endpoint;
    fact.provenance = EvidenceProvenance::SourceDerived;
    fact.created_by_run_id = session.mapper.run_id.clone();
    insert_evidence_node(&connection, &fact).unwrap();
    // An arbitrary nonempty artifact reference is not a Broker observation.
    // The former test incorrectly let this fixture open a second Reviewer
    // revision and even publish a finding without new verified HTTP evidence.
    assert!(matches!(multi_agent_review(context, &mut session, outcome),
        AgentTargetOutcome::Incomplete(_)));
    let second: (String, i64) = connection.query_row(
        "SELECT status,candidate_revision FROM agent_finding_candidates WHERE root_run_id=?1",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(second, ("insufficient_evidence".into(), 1));
    let decisions: Vec<(i64, String)> = connection.prepare(
        "SELECT candidate_revision,verdict FROM agent_review_decisions WHERE root_run_id=?1 ORDER BY candidate_revision",
    ).unwrap().query_map([&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap().map(Result::unwrap).collect();
    assert_eq!(decisions, vec![(1, "insufficient_evidence".into())]);
    let published: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sentinel_findings WHERE scan_id=?1 AND stage<>'native-agent-coverage'", [&lease.scan_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(published, 0);
    drop(connection);
    drop(session);
}

#[test]
fn reviewer_model_decision_requires_a_bounded_consistent_schema() {
    let valid = r#"{"verdict":"confirmed","reasonCodes":["verified_fact"],"missingEvidence":[],"confidence":0.9,"summary":"Evidence checked"}"#;
    let decision = validated_review_decision(valid).unwrap();
    assert_eq!(decision.verdict, "confirmed");
    let invalid = [
        ("not json", "review_decision_invalid_json"),
        (r#"{"verdict":"confirmed"}"#, "review_decision_invalid_shape"),
        (r#"{"verdict":"maybe","reasonCodes":[],"missingEvidence":[],"confidence":0.9,"summary":"x"}"#, "review_decision_invalid_verdict"),
        (r#"{"verdict":"confirmed","reasonCodes":[{"bad":1}],"missingEvidence":[],"confidence":0.9,"summary":"x"}"#, "review_decision_invalid_reasons"),
        (r#"{"verdict":"confirmed","reasonCodes":[],"missingEvidence":["missing control"],"confidence":0.9,"summary":"x"}"#, "review_decision_confirmed_with_missing_evidence"),
        (r#"{"verdict":"rejected","reasonCodes":[],"missingEvidence":[],"confidence":1.5,"summary":"x"}"#, "review_decision_invalid_confidence"),
        (r#"{"verdict":"rejected","reasonCodes":[],"missingEvidence":[],"confidence":0.5,"summary":""}"#, "review_decision_invalid_summary"),
        (r#"{"verdict":"rejected","reasonCodes":[],"missingEvidence":[],"confidence":0.5,"summary":"x","extra":true}"#, "review_decision_invalid_shape"),
    ];
    for (text, expected) in invalid {
        assert_eq!(validated_review_decision(text).err().as_deref(), Some(expected), "{text}");
    }
}
