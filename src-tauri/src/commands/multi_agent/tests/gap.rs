#[test]
fn reviewer_insufficiency_starts_one_read_only_gap_run_and_assessment() {
    // Actual frozen Web Root/C/worker (original 60_000 tokens / 20 requests).
    let (port,_seen,_stop)=specialist_original_endpoint(std::sync::Arc::new(|wire| {
        if specialist_original_is_web_sdk(&wire) {
            (200,"application/json",proposal_model_response("actual bounded Web exit before sealed gap"))
        } else { multi_agent_original_fake_response(wire) }
    }));
    let mut f=root_tick_fixture_from_web_mode_turns("review-gap",
        &format!("http://127.0.0.1:{port}/v1"),true,(60_000,20),None,
        web_mode_fixture("multi","https://authorized.example.test"),Some(1));
    drop(f.parent.take());
    let lease=f.actor.clone();
    let context=&mut f.context;
    let root_run_id = lease.root_run_id.clone();
    let connection = db::open(&context.db_path).unwrap();
    let mut session = multi_agent_original_prepare(context);
    let _real=RealSpecialistTransport::enter();
    let outcome=run_native_agent(context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    assert_eq!(_seen.lock().unwrap().iter().filter(|wire|specialist_original_is_web_sdk(wire)).count(),1);
    multi_agent_finish_execution(context, &mut session, &outcome).unwrap();
    let usage:(i64,i64)=connection.query_row("SELECT used_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(usage,(20,1));
    // Actual independent Reviewer seal (candidate-gap / revision 2 /
    // ["missing control"]) with its received receipt, as the born Root requires.
    seal_specialist_original_gap_review(&connection, &session.lease, context);
    context.evidence["volatileOnly"] = serde_json::json!("not in the sealed Reviewer request");
    let prior_target_runs: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND lane='target_touching'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    // The born Root compares the live context with its frozen evidence, so a
    // volatile-only change is refused before any Investigator is scheduled.
    assert_eq!(multi_agent_investigate_review_gap(
        context, &session, "candidate-gap", 2, &serde_json::json!(["missing control"]),
    ).unwrap_err(), "root_tick_original_evidence_conflict");
    let refused: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(refused, 0);
    context.evidence.as_object_mut().unwrap().remove("volatileOnly");
    let _real = RealSpecialistTransport::enter();
    multi_agent_investigate_review_gap(
        context, &session, "candidate-gap", 2, &serde_json::json!(["missing control"]),
    ).unwrap();
    // The actual Investigator input is the sealed Reviewer request only.
    let investigator_requests: Vec<String> = _seen.lock().unwrap().iter()
        .filter(|r| r.contains("Deep Investigator") && !r.contains("You are the Root Coordinator"))
        .cloned().collect();
    assert_eq!(investigator_requests.len(), 1);
    assert!(!investigator_requests[0].contains("volatileOnly"));
    assert!(!investigator_requests[0].contains("frozenEvidence"));
    // A pre-upgrade task slice lacks missingEvidence. The acknowledged
    // proposal, not the new optional field, remains the replay source.
    connection.execute(
        "UPDATE agent_assignments SET task_slice_json=?1 WHERE coordinator_run_id=?2 AND role='deep_investigator'",
        rusqlite::params![serde_json::json!({"candidateId":"candidate-gap","revision":2}).to_string(), root_run_id],
    ).unwrap();
    multi_agent_investigate_review_gap(
        context, &session, "candidate-gap", 2, &serde_json::json!(["missing control"]),
    ).unwrap();
    let gap: (i64, String, String) = connection.query_row(
        "SELECT COUNT(*),MAX(a.lane),MAX(r.status) FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id \
         WHERE a.coordinator_run_id=?1 AND a.role='deep_investigator'",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(gap, (1, "read_only_analysis".into(), "terminal".into()));
    let messages: (i64, i64) = connection.query_row(
        "SELECT SUM(CASE WHEN kind='gap_proposed' AND acknowledged_at<>'' THEN 1 ELSE 0 END), \
         SUM(CASE WHEN kind='proposal_assessed' AND acknowledged_at<>'' THEN 1 ELSE 0 END) \
         FROM agent_messages WHERE root_run_id=?1 AND evidence_revision=2",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(messages, (1, 1));
    let after_target_runs: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_runs WHERE root_run_id=?1 AND lane='target_touching'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(after_target_runs, prior_target_runs);
    drop(connection);
    drop(session);
}

#[test]
fn gap_refuses_missing_or_changed_review_seal_before_scheduling() {
    // Actual frozen Web Root/C/worker (original 60_000 tokens / 20 requests).
    let (port,_seen,_stop)=specialist_original_endpoint(std::sync::Arc::new(|wire| {
        if specialist_original_is_web_sdk(&wire) {
            (200,"application/json",proposal_model_response("actual bounded Web exit before sealed gap"))
        } else { multi_agent_original_fake_response(wire) }
    }));
    let mut f=root_tick_fixture_from_web_mode_turns("gap-seal-required",
        &format!("http://127.0.0.1:{port}/v1"),true,(60_000,20),None,
        web_mode_fixture("multi","https://authorized.example.test"),Some(1));
    drop(f.parent.take());
    let lease=f.actor.clone();
    let context=&mut f.context;
    let root_run_id = lease.root_run_id.clone();
    let connection = db::open(&context.db_path).unwrap();
    let mut session = multi_agent_original_prepare(context);
    let _real=RealSpecialistTransport::enter();
    let outcome=run_native_agent(context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    assert_eq!(_seen.lock().unwrap().iter().filter(|wire|specialist_original_is_web_sdk(wire)).count(),1);
    multi_agent_finish_execution(context, &mut session, &outcome).unwrap();
    let usage:(i64,i64)=connection.query_row("SELECT used_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(usage,(20,1));
    let original_calls=_seen.lock().unwrap().len();
    let gap = serde_json::json!(["missing control"]);
    assert_eq!(multi_agent_investigate_review_gap(context, &session, "candidate-gap", 2, &gap).unwrap_err(),
        "gap_review_request_unavailable");
    seal_gap_review_fixture(&connection, &session.lease, context, "candidate-gap", 2, &gap);
    assert_eq!(multi_agent_investigate_review_gap(context, &session, "candidate-gap", 2,
        &serde_json::json!(["different gap"])).unwrap_err(), "gap_review_missing_evidence_mismatch");
    connection.execute("UPDATE agent_review_requests SET candidate_json='{}' WHERE candidate_id='candidate-gap'", []).unwrap();
    assert_eq!(multi_agent_investigate_review_gap(context, &session, "candidate-gap", 2, &gap).unwrap_err(),
        "review_manifest_changed_requires_new_revision");
    let scheduled: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(scheduled, 0);
    assert_eq!(_seen.lock().unwrap().len(),original_calls,"missing or changed seals cannot dispatch an SDK");
    drop(connection);
    drop(session);
}

#[test]
fn gap_proposals_require_attributable_facts_and_bounded_non_executable_contracts() {
    let proposal = serde_json::json!({
        "summary":"check the missing control", "nextStep":"request_new_contract",
        "gapCode":"missing_owner_control", "supportingFactRefs":["ev-trusted"],
        "missingEvidence":["owner X response"], "prerequisites":["operator control group"],
        "proposedContracts":["new_attempt_control_group_request"],
        "expectedInformationGain":0.5, "impactCeiling":"medium",
        "estimatedCost":{"modelTokens":1000,"modelRequests":1,"targetRequests":3},
        "sideEffectClass":"read_only", "overlapKeys":["owner-control"],
        "falsificationCondition":"owner X cannot access", "stopCondition":"429 or scope rejection"
    });
    let missing = serde_json::json!(["owner X response"]);
    assert!(parse_gap_proposal(&proposal, &["ev-trusted".into()], &missing).is_ok());
    let mut prematurely_approved = proposal.clone();
    prematurely_approved["proposedContracts"] = serde_json::json!(["operator_approved_control_group"]);
    assert_eq!(parse_gap_proposal(&prematurely_approved, &["ev-trusted".into()], &missing).unwrap_err(),
        "gap_proposal_contract_cost_mismatch");
    assert!(parse_gap_proposal_versioned(&prematurely_approved, &["ev-trusted".into()], &missing, 2).is_ok());
    let mut unknown = proposal.clone();
    unknown["supportingFactRefs"] = serde_json::json!(["ev-other-run"]);
    assert_eq!(parse_gap_proposal(&unknown, &["ev-trusted".into()], &missing).unwrap_err(),
        "gap_proposal_unattributed_fact");
    let mut unsafe_contract = proposal.clone();
    unsafe_contract["proposedContracts"] = serde_json::json!(["shell curl https://other.test"]);
    assert_eq!(parse_gap_proposal(&unsafe_contract, &["ev-trusted".into()], &missing).unwrap_err(),
        "gap_proposal_unknown_contract");
    let mut unbounded = proposal.clone();
    unbounded["estimatedCost"]["targetRequests"] = serde_json::json!(999);
    assert!(parse_gap_proposal(&unbounded, &["ev-trusted".into()], &missing).is_err());
    let mut side_effect = proposal.clone();
    side_effect["sideEffectClass"] = serde_json::json!("write");
    assert!(parse_gap_proposal(&side_effect, &["ev-trusted".into()], &missing).is_err());
    let mut substituted_gap = proposal.clone();
    substituted_gap["missingEvidence"] = serde_json::json!(["some unrelated gap"]);
    assert_eq!(parse_gap_proposal(&substituted_gap, &["ev-trusted".into()], &missing).unwrap_err(),
        "gap_proposal_reviewer_gap_mismatch");
    let mut zero_request_contract = proposal.clone();
    zero_request_contract["estimatedCost"]["targetRequests"] = serde_json::json!(0);
    assert_eq!(parse_gap_proposal(&zero_request_contract, &["ev-trusted".into()], &missing).unwrap_err(),
        "gap_proposal_contract_cost_mismatch");
    let mut false_manual = proposal.clone();
    false_manual["nextStep"] = serde_json::json!("manual_review");
    assert_eq!(parse_gap_proposal(&false_manual, &["ev-trusted".into()], &missing).unwrap_err(),
        "gap_proposal_contract_cost_mismatch");
    let mut existing_evidence = proposal;
    existing_evidence["nextStep"] = serde_json::json!("observe_existing_evidence");
    existing_evidence["proposedContracts"] = serde_json::json!(["existing_evidence_review"]);
    existing_evidence["estimatedCost"]["targetRequests"] = serde_json::json!(0);
    assert!(parse_gap_proposal(&existing_evidence, &["ev-trusted".into()], &missing).is_ok());
}

#[test]
fn completed_gap_replay_rejects_tampered_or_unacknowledged_assessment() {
    // Actual creator, immutable Web Root/C, one original paid Web SDK/exit,
    // and a separately received independent Reviewer seal. Same 60_000/20 cap.
    let (port,seen,stop)=specialist_original_endpoint(std::sync::Arc::new(multi_agent_original_fake_response));
    let (root,context,session)=specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
    let original_web=specialist_original_take_web_seen(&session.lease.root_run_id);
    assert_eq!(original_web.count(),1);
    let root_run_id=session.lease.root_run_id.clone();
    let connection=db::open(&context.db_path).unwrap();
    let _real=RealSpecialistTransport::enter();
    let gap=serde_json::json!(["missing control"]);
    multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap();
    let original_provider_calls=(seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len());
    let (original_id, original_payload): (String, String) = connection.query_row(
        "SELECT id,payload_json FROM agent_messages WHERE root_run_id=?1 AND kind='proposal_assessed'",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let mut improperly_granted: serde_json::Value = serde_json::from_str(&original_payload).unwrap();
    assert_eq!(improperly_granted["schemaVersion"], 3);
    assert_eq!(improperly_granted["newAttemptRequired"], true);
    assert_eq!(improperly_granted["targetRequestsGranted"], 0);
    improperly_granted["targetRequestsGranted"] = serde_json::json!(3);
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![improperly_granted.to_string(), original_id]).unwrap();
    assert_eq!(multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap_err(),
        "gap_assessment_incomplete_requires_recovery");
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![original_payload, original_id]).unwrap();
    multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap();
    let mut wrong_next_attempt: serde_json::Value = serde_json::from_str(&original_payload).unwrap();
    wrong_next_attempt["newAttemptRequired"] = serde_json::json!(false);
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![wrong_next_attempt.to_string(), original_id]).unwrap();
    assert_eq!(multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap_err(),
        "gap_assessment_incomplete_requires_recovery");
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![original_payload, original_id]).unwrap();
    let assessment_id: String = connection.query_row(
        "SELECT id FROM agent_messages WHERE root_run_id=?1 AND kind='proposal_assessed'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![serde_json::json!({"candidateId":"other","evidenceRevision":2}).to_string(), assessment_id]).unwrap();
    assert_eq!(multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap_err(),
        "gap_assessment_incomplete_requires_recovery");
    connection.execute("UPDATE agent_messages SET payload_json=?1,acknowledged_at='' WHERE id=?2",
        rusqlite::params![serde_json::json!({"candidateId":"candidate-gap","evidenceRevision":2,
            "summary":"deferred","decision":"deferred_requires_new_evidence_revision",
            "reasonCode":"proposal_is_not_a_verified_execution_contract"}).to_string(), assessment_id]).unwrap();
    assert_eq!(multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap_err(),
        "gap_assessment_incomplete_requires_recovery");
    connection.execute("UPDATE agent_messages SET acknowledged_at=datetime('now','localtime') WHERE id=?1",
        [&assessment_id]).unwrap();
    let gap_child: String = connection.query_row(
        "SELECT child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    connection.execute("UPDATE agent_runs SET terminal_state='failed' WHERE id=?1",
        [&gap_child]).unwrap();
    assert_eq!(multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap_err(),
        "gap_assessment_incomplete_requires_recovery");
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
    assert_eq!((seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len()),original_provider_calls,
        "tamper/replay never sends another child, Root or Web SDK request");
    assert_eq!(original_web.count(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn gap_v2_semantic_read_validation_accepts_acknowledged_payload_without_dispatch() {
    // Historical JSON compatibility is a read-only semantic check. This test
    // does not claim rewritten v2 payloads satisfy original receipt provenance
    // or authorize execution; the paired protocol replay test must reject them.
    let (port,seen,stop)=specialist_original_endpoint(std::sync::Arc::new(multi_agent_original_fake_response));
    let (root,context,session)=specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
    let original_web=specialist_original_take_web_seen(&session.lease.root_run_id);
    assert_eq!(original_web.count(),1);
    let root_run_id=session.lease.root_run_id.clone();
    let connection=db::open(&context.db_path).unwrap();
    let _real=RealSpecialistTransport::enter();
    let gap=serde_json::json!(["missing control"]);
    multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap();
    let original_provider_calls=(seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len());
    let (proposal_id, proposal_text): (String, String) = connection.query_row(
        "SELECT id,payload_json FROM agent_messages WHERE root_run_id=?1 AND kind='gap_proposed'",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let mut proposal: serde_json::Value = serde_json::from_str(&proposal_text).unwrap();
    proposal["schemaVersion"] = serde_json::json!(2);
    proposal["proposal"]["proposedContracts"] = serde_json::json!(["operator_approved_control_group"]);
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![proposal.to_string(), proposal_id]).unwrap();
    let (assessment_id, assessment_text): (String, String) = connection.query_row(
        "SELECT id,payload_json FROM agent_messages WHERE root_run_id=?1 AND kind='proposal_assessed'",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let mut assessment: serde_json::Value = serde_json::from_str(&assessment_text).unwrap();
    assessment["schemaVersion"] = serde_json::json!(2);
    assessment["reasonCode"] = serde_json::json!("proposal_is_not_a_verified_execution_contract");
    assessment.as_object_mut().unwrap().remove("newAttemptRequired");
    assessment.as_object_mut().unwrap().remove("targetRequestsGranted");
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![assessment.to_string(), assessment_id]).unwrap();
    let original_rows=web_mode_test_rows(&connection);
    let (assignment,child):(String,String)=connection.query_row(
        "SELECT id,child_run_id FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",
        [&root_run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert!(completed_gap_round_valid(&connection,&root_run_id,&assignment,&child,
        "candidate-gap",2,&gap,&context.target_dir).unwrap());
    web_mode_assert_rows(&connection,&original_rows);
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
    assert_eq!((seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len()),original_provider_calls,
        "tamper/replay never sends another child, Root or Web SDK request");
    assert_eq!(original_web.count(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_gap_replay_new_protocol_rejects_legacy_rewrite_without_reissuing_requests() {
    // Actual creator, immutable Web Root/C, one original paid Web SDK/exit,
    // and a separately received independent Reviewer seal. Same 60_000/20 cap.
    let (port,seen,stop)=specialist_original_endpoint(std::sync::Arc::new(multi_agent_original_fake_response));
    let (root,context,session)=specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
    let original_web=specialist_original_take_web_seen(&session.lease.root_run_id);
    assert_eq!(original_web.count(),1);
    let root_run_id=session.lease.root_run_id.clone();
    let connection=db::open(&context.db_path).unwrap();
    let _real=RealSpecialistTransport::enter();
    let gap=serde_json::json!(["missing control"]);
    multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap();
    let original_provider_calls=(seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len());
    let (proposal_id, proposal_text): (String, String) = connection.query_row(
        "SELECT id,payload_json FROM agent_messages WHERE root_run_id=?1 AND kind='gap_proposed'",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let mut proposal: serde_json::Value = serde_json::from_str(&proposal_text).unwrap();
    proposal["schemaVersion"] = serde_json::json!(2);
    proposal["proposal"]["proposedContracts"] = serde_json::json!(["operator_approved_control_group"]);
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![proposal.to_string(), proposal_id]).unwrap();
    let (assessment_id, assessment_text): (String, String) = connection.query_row(
        "SELECT id,payload_json FROM agent_messages WHERE root_run_id=?1 AND kind='proposal_assessed'",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let mut assessment: serde_json::Value = serde_json::from_str(&assessment_text).unwrap();
    assessment["schemaVersion"] = serde_json::json!(2);
    assessment["reasonCode"] = serde_json::json!("proposal_is_not_a_verified_execution_contract");
    assessment.as_object_mut().unwrap().remove("newAttemptRequired");
    assessment.as_object_mut().unwrap().remove("targetRequestsGranted");
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![assessment.to_string(), assessment_id]).unwrap();
    let original_rows=web_mode_test_rows(&connection);
    assert_eq!(multi_agent_investigate_review_gap(&context,&session,"candidate-gap",2,&gap).unwrap_err(),
        "mailbox_gap_assessment_original_conflict");
    web_mode_assert_rows(&connection,&original_rows);
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
    assert_eq!((seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len()),original_provider_calls,
        "tamper/replay never sends another child, Root or Web SDK request");
    assert_eq!(original_web.count(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_gap_replay_rejects_unattributed_proposal_reference() {
    // Actual creator, immutable Web Root/C, one original paid Web SDK/exit,
    // and a separately received independent Reviewer seal. Same 60_000/20 cap.
    let (port,seen,stop)=specialist_original_endpoint(std::sync::Arc::new(multi_agent_original_fake_response));
    let (root,context,session)=specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
    let original_web=specialist_original_take_web_seen(&session.lease.root_run_id);
    assert_eq!(original_web.count(),1);
    let root_run_id=session.lease.root_run_id.clone();
    let connection=db::open(&context.db_path).unwrap();
    let _real=RealSpecialistTransport::enter();
    let gap=serde_json::json!(["missing control"]);
    multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap();
    let original_provider_calls=(seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len());
    let (id, raw): (String, String) = connection.query_row(
        "SELECT id,payload_json FROM agent_messages WHERE root_run_id=?1 AND kind='gap_proposed'",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let mut payload: serde_json::Value = serde_json::from_str(&raw).unwrap();
    payload["proposal"]["supportingFactRefs"] = serde_json::json!(["ev-from-other-run"]);
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![payload.to_string(), id]).unwrap();
    assert_eq!(multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap_err(),
        "gap_assessment_incomplete_requires_recovery");
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
    assert_eq!((seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len()),original_provider_calls,
        "tamper/replay never sends another child, Root or Web SDK request");
    assert_eq!(original_web.count(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn completed_gap_replay_rejects_proposal_that_replaces_the_reviewers_gap() {
    // Actual creator, immutable Web Root/C, one original paid Web SDK/exit,
    // and a separately received independent Reviewer seal. Same 60_000/20 cap.
    let (port,seen,stop)=specialist_original_endpoint(std::sync::Arc::new(multi_agent_original_fake_response));
    let (root,context,session)=specialist_original_gap_fixture(&format!("http://127.0.0.1:{port}/v1"));
    let original_web=specialist_original_take_web_seen(&session.lease.root_run_id);
    assert_eq!(original_web.count(),1);
    let root_run_id=session.lease.root_run_id.clone();
    let connection=db::open(&context.db_path).unwrap();
    let _real=RealSpecialistTransport::enter();
    let gap=serde_json::json!(["missing control"]);
    multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap();
    let original_provider_calls=(seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len());
    let (id, raw): (String, String) = connection.query_row(
        "SELECT id,payload_json FROM agent_messages WHERE root_run_id=?1 AND kind='gap_proposed'",
        [&root_run_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    let mut payload: serde_json::Value = serde_json::from_str(&raw).unwrap();
    // The outer envelope still matches the sealed Reviewer decision. Only
    // the untrusted proposal has silently substituted an easier evidence gap.
    payload["proposal"]["missingEvidence"] = serde_json::json!(["other gap"]);
    connection.execute("UPDATE agent_messages SET payload_json=?1 WHERE id=?2",
        rusqlite::params![payload.to_string(), id]).unwrap();
    assert_eq!(multi_agent_investigate_review_gap(&context, &session, "candidate-gap", 2, &gap).unwrap_err(),
        "gap_assessment_incomplete_requires_recovery");
    assert_eq!((seen.lock().unwrap().len(),original_web.seen.lock().unwrap().len()),original_provider_calls,
        "tamper/replay never sends another child, Root or Web SDK request");
    assert_eq!(original_web.count(),1);
    stop.store(true,std::sync::atomic::Ordering::SeqCst);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_gap_round_is_not_mistaken_for_an_assessed_revision() {
    use crate::agent_runtime::{contract::{AgentLane, AgentRole}, multi_agent::scheduler};
    // Actual frozen Web Root/C/worker (original 60_000 tokens / 20 requests).
    let (port,seen,_stop)=specialist_original_endpoint(std::sync::Arc::new(|wire| {
        if specialist_original_is_web_sdk(&wire) {
            (200,"application/json",proposal_model_response("actual bounded Web exit before failed gap"))
        } else { multi_agent_original_fake_response(wire) }
    }));
    let mut f=root_tick_fixture_from_web_mode_turns("gap-failed-reentry",
        &format!("http://127.0.0.1:{port}/v1"),true,(60_000,20),None,
        web_mode_fixture("multi","https://authorized.example.test"),Some(1));
    drop(f.parent.take());
    let lease=f.actor.clone();
    let context=&mut f.context;
    let root_run_id = lease.root_run_id.clone();
    let connection = db::open(&context.db_path).unwrap();
    let mut session = multi_agent_original_prepare(context);
    let _real=RealSpecialistTransport::enter();
    let outcome=run_native_agent(context);
    assert!(matches!(outcome,AgentTargetOutcome::BoundedCompleted(_) | AgentTargetOutcome::Incomplete(_)),"{outcome:?}");
    assert_eq!(seen.lock().unwrap().iter().filter(|wire|specialist_original_is_web_sdk(wire)).count(),1);
    multi_agent_finish_execution(context, &mut session, &outcome).unwrap();
    let usage:(i64,i64)=connection.query_row("SELECT used_tokens,used_requests FROM agent_runs WHERE id=?1",
        [&session.executor.run_id],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(usage,(20,1));
    let original_calls=seen.lock().unwrap().len();
    seal_gap_review_fixture(&connection, &session.lease, context, "candidate-gap", 2,
        &serde_json::json!(["missing control"]));
    let child = scheduler::schedule_child(
        &connection, &session.lease, AgentRole::DeepInvestigator, AgentLane::ReadOnlyAnalysis,
        "reviewer_insufficient_evidence", &serde_json::json!({"candidateId":"candidate-gap","revision":2}),
        2, &["evidence.read".into(), "mailbox.write".into()], 4_000, 1,
    ).unwrap();
    scheduler::start_child_or_release(&connection, &session.lease, &child).unwrap();
    scheduler::finish_child(&connection, &session.lease, &child, false, "simulated crash").unwrap();
    let before = review_delivery_snapshot(&connection);
    let error = multi_agent_investigate_review_gap(
        context, &session, "candidate-gap", 2, &serde_json::json!(["missing control"]),
    ).unwrap_err();
    assert_eq!(error, "gap_assessment_incomplete_requires_recovery:specialist_received_receipt_required");
    assert_eq!(before, review_delivery_snapshot(&connection));
    assert_eq!(seen.lock().unwrap().len(),original_calls,"failed gap reentry must not dispatch another SDK");
    let count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1 AND role='deep_investigator'",
        [&root_run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 1);
    drop(connection);
    drop(session);
}

