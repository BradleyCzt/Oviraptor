#[derive(Debug, PartialEq)]
struct ValidatedReviewDecision {
    verdict: String,
    reason_codes: JsonValue,
    missing_evidence: JsonValue,
    confidence: f64,
    summary: String,
    gap_assessment: Option<JsonValue>,
}

fn validated_review_decision(text: &str) -> Result<ValidatedReviewDecision, String> {
    let value: JsonValue = serde_json::from_str(text)
        .map_err(|_| "review_decision_invalid_json".to_string())?;
    let object = value.as_object().ok_or("review_decision_invalid_shape")?;
    if object.len() != 5 + usize::from(object.contains_key("gapAssessment")) || !["verdict", "reasonCodes", "missingEvidence", "confidence", "summary"]
        .iter().all(|key| object.contains_key(*key))
    {
        return Err("review_decision_invalid_shape".into());
    }
    let verdict = value["verdict"].as_str()
        .filter(|verdict| matches!(*verdict, "confirmed" | "rejected" | "insufficient_evidence"))
        .ok_or("review_decision_invalid_verdict")?;
    let reason_codes = value["reasonCodes"].as_array()
        .filter(|items| items.len() <= 20 && items.iter().all(|item| item.as_str()
            .is_some_and(|code| !code.is_empty() && code.len() <= 80
                && code.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'))))
        .ok_or("review_decision_invalid_reasons")?;
    let missing_evidence = value["missingEvidence"].as_array()
        .filter(|items| items.len() <= 20 && items.iter().all(|item| item.as_str()
            .is_some_and(|description| !description.trim().is_empty() && description.chars().count() <= 300)))
        .ok_or("review_decision_invalid_missing_evidence")?;
    if verdict == "confirmed" && !missing_evidence.is_empty() {
        return Err("review_decision_confirmed_with_missing_evidence".into());
    }
    let confidence = value["confidence"].as_f64()
        .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
        .ok_or("review_decision_invalid_confidence")?;
    let summary = value["summary"].as_str()
        .filter(|summary| !summary.trim().is_empty() && summary.chars().count() <= 500)
        .ok_or("review_decision_invalid_summary")?;
    Ok(ValidatedReviewDecision {
        verdict: verdict.to_string(),
        reason_codes: JsonValue::Array(reason_codes.clone()),
        missing_evidence: JsonValue::Array(missing_evidence.clone()),
        confidence,
        summary: summary.to_string(),
        gap_assessment: object.get("gapAssessment").cloned(),
    })
}

fn replay_frozen_review(
    context: &AgentRunContext,
    session: &MultiAgentSession,
    candidate_id: &str,
    revision: i64,
    candidate_text: &str,
    outcome: AgentTargetOutcome,
) -> Result<AgentTargetOutcome, String> {
    let connection = db::open(&context.db_path)?;
    {
        let tx=connection.unchecked_transaction().map_err(|e|e.to_string())?;
        let frozen:JsonValue=serde_json::from_str(candidate_text).map_err(|_|"review_authorization_original_candidate_invalid")?;
        native_authorization_original_candidate_on(&tx,&session.lease,&context.target_dir,&frozen)?;
    }
    let failed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_review_requests WHERE root_run_id=?1 \
         AND candidate_id=?2 AND candidate_revision=?3 AND candidate_json=?4 AND status='failed')",
        params![session.lease.root_run_id, candidate_id, revision, candidate_text], |row| row.get(0),
    ).map_err(|error| format!("review_replay_recovery_lookup:{error}"))?;
    let recovery_error = if failed {
        recover_received_review(&connection, &session.lease, candidate_id, revision, candidate_text, &context.target_dir).err()
    } else { None };
    // Keep decision, manifest, candidate and published finding reads in one
    // snapshot. Release it before the Investigator opens its write transaction.
    let transaction = connection.unchecked_transaction()
        .map_err(|error| format!("review_replay_snapshot:{error}"))?;
    let connection = &transaction;
    let frozen:JsonValue=serde_json::from_str(candidate_text).map_err(|_|"review_authorization_original_candidate_invalid")?;
    native_authorization_original_candidate_on(connection,&session.lease,&context.target_dir,&frozen)?;
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(connection, &session.lease)?;
    let prior: Option<(String, String, String, String, String, f64)> = connection.query_row(
        "SELECT d.verdict,d.missing_evidence_json,m.payload_json,q.reviewer_run_id,d.reason_codes_json,d.confidence \
         FROM agent_review_requests q \
         JOIN agent_review_decisions d ON d.id=q.decision_id AND d.root_run_id=q.root_run_id \
           AND d.candidate_id=q.candidate_id AND d.candidate_revision=q.candidate_revision \
           AND d.reviewer_run_id=q.reviewer_run_id AND d.verdict=q.status \
         JOIN agent_assignments a ON a.id=q.assignment_id AND a.child_run_id=q.reviewer_run_id \
           AND a.coordinator_run_id=q.root_run_id AND a.state='completed' \
         JOIN agent_runs r ON r.id=q.reviewer_run_id AND r.root_run_id=q.root_run_id \
           AND r.role='evidence_reviewer' AND r.lane='review' \
           AND r.status='terminal' AND r.terminal_state='completed' \
         JOIN agent_messages m ON m.root_run_id=q.root_run_id AND m.from_run_id=q.reviewer_run_id \
           AND m.to_run_id=q.root_run_id AND m.kind='review_decision' \
           AND m.assignment_id=q.assignment_id AND m.evidence_revision=q.candidate_revision \
           AND m.correlation_id=q.id AND m.delivered_at<>'' AND m.acknowledged_at<>'' \
         WHERE q.root_run_id=?1 AND q.candidate_id=?2 AND q.candidate_revision=?3 \
           AND q.candidate_json=?4 LIMIT 1",
        params![session.lease.root_run_id, candidate_id, revision, candidate_text],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    ).optional().map_err(|error| format!("review_replay_lookup_failed:{error}"))?;
    // Another local reader may have completed the same receipt between the
    // failed-state lookup and our recovery lock. Validate its completed result
    // normally; never schedule a replacement model call on this race.
    let (verdict, missing_text, payload_text, reviewer_run_id, reasons_text, confidence) = prior.ok_or_else(||
        recovery_error.unwrap_or_else(|| "review_replay_incomplete_requires_recovery".into()))?;
    if !matches!(verdict.as_str(), "confirmed" | "rejected" | "insufficient_evidence") {
        return Err("review_replay_invalid_verdict".into());
    }
    if let Err(error) = verify_review_snapshot(
        connection, &session.lease.root_run_id, candidate_id, revision, &context.target_dir,
    ) {
        // A previous insufficiency can never publish anything. An unrelated
        // graph insertion must not turn it into a false confirmation or an
        // unbounded retry; keep it incomplete and require a new verified fact
        // before scheduling another Reviewer revision.
        if verdict == "insufficient_evidence" && error == "review_manifest_changed_requires_new_revision" {
            return Ok(AgentTargetOutcome::incomplete(
                "review_gate_insufficient_evidence:evidence_manifest_changed",
            ));
        }
        return Err(error);
    }
    let payload: JsonValue = serde_json::from_str(&payload_text)
        .map_err(|_| "review_replay_invalid_message")?;
    let missing: JsonValue = serde_json::from_str(&missing_text)
        .map_err(|_| "review_replay_invalid_missing_evidence")?;
    if payload["candidateId"] != candidate_id || payload["candidateRevision"] != revision
        || payload["verdict"] != verdict || !missing.is_array()
    {
        return Err("review_replay_decision_mismatch".into());
    }
    // A completed replay consumes the original paid decision; mutable mailbox
    // fields cannot replace its summary or add executable payload fields.
    let (assignment_id, message_id): (String, String) = connection.query_row(
        "SELECT q.assignment_id,m.id FROM agent_review_requests q JOIN agent_messages m ON m.assignment_id=q.assignment_id AND m.correlation_id=q.id AND m.kind='review_decision' AND m.root_run_id=q.root_run_id WHERE q.root_run_id=?1 AND q.candidate_id=?2 AND q.candidate_revision=?3 AND q.reviewer_run_id=?4",
        params![session.lease.root_run_id,candidate_id,revision,reviewer_run_id],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).map_err(|error| format!("review_replay_original_lookup:{error}"))?;
    let reviewer = crate::agent_runtime::multi_agent::scheduler::ScheduledChild {
        assignment_id,
        run_id: reviewer_run_id.clone(),
        role: crate::agent_runtime::contract::AgentRole::EvidenceReviewer,
    };
    let receipt = crate::agent_runtime::multi_agent::specialist::received_for_local_delivery(
        connection, &session.lease, &reviewer,
    )?;
    let original = validated_review_decision(&receipt.text)?;
    let message = crate::agent_runtime::multi_agent::mailbox::verified_completed_output(
        connection, &session.lease, &message_id,
    )?;
    let expected = crate::agent_runtime::secrets::redact_json(&json!({
        "verdict":original.verdict,"summary":original.summary,
        "candidateId":candidate_id,"candidateRevision":revision,
    }));
    let reasons: JsonValue = serde_json::from_str(&reasons_text)
        .map_err(|_| "review_replay_invalid_reasons")?;
    if !receipt.rejection.is_empty() || original.verdict != verdict
        || original.missing_evidence != missing || original.reason_codes != reasons
        || !confidence.is_finite() || (original.confidence - confidence).abs() > f64::EPSILON
        || message.payload != expected
        || payload != expected
    {
        return Err("review_replay_original_decision_conflict".into());
    }
    let frozen: JsonValue = serde_json::from_str(candidate_text)
        .map_err(|_| "review_replay_invalid_candidate")?;
    if frozen.get("gapFollowup").is_some() {
        let request: String = connection.query_row("SELECT id FROM agent_review_requests WHERE root_run_id=?1 AND candidate_id=?2 AND candidate_revision=?3",
            params![session.lease.root_run_id,candidate_id,revision], |r|r.get(0)).map_err(|e|e.to_string())?;
        verified_gap_review_receipt(&transaction,&request)?;
    }
    let findings = frozen["findingCandidates"].as_array()
        .ok_or("review_replay_invalid_candidate")?;
    let expected_status = match verdict.as_str() {
        "confirmed" => "published",
        "rejected" => "rejected",
        _ => "insufficient_evidence",
    };
    for finding in findings {
        let id = finding["id"].as_str().ok_or("review_replay_invalid_candidate")?;
        let settled: Option<(JsonValue, bool)> = connection.query_row(
            "SELECT c.stage,c.kind,c.record_key,c.title,c.severity,c.record_json, \
             ((?5='published' AND c.published_at<>'' AND EXISTS(SELECT 1 FROM sentinel_findings f \
                WHERE f.scan_id=c.scan_id AND f.target_url=c.target_url AND f.stage=c.stage \
                  AND f.kind=c.kind AND f.record_key=c.record_key AND f.title=c.title \
                  AND f.severity=c.severity AND f.record_json=c.record_json)) \
               OR (?5<>'published' AND c.published_at='')) \
             FROM agent_finding_candidates c WHERE c.id=?1 AND c.root_run_id=?2 AND c.candidate_revision=?3 \
             AND c.reviewer_run_id=?4 AND c.status=?5 AND c.scan_id=?6 AND c.target_url=?7",
            params![id, session.lease.root_run_id, revision, reviewer_run_id, expected_status,
                session.lease.scan_id, session.lease.target_key],
            |row| {
                let record: String = row.get(5)?;
                let record: JsonValue = serde_json::from_str(&record).map_err(|error|
                    rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error)))?;
                Ok((serde_json::json!({"id":id,"stage":row.get::<_,String>(0)?,
                    "kind":row.get::<_,String>(1)?,"recordKey":row.get::<_,String>(2)?,
                    "title":row.get::<_,String>(3)?,"severity":row.get::<_,String>(4)?,"record":record}), row.get(6)?))
            },
        ).optional().map_err(|error| format!("review_replay_candidates_unavailable:{error}"))?;
        if !settled.is_some_and(|(stored, published)| stored == *finding && published) {
            return Err("review_replay_publication_incomplete_requires_recovery".into());
        }
    }
    transaction.commit().map_err(|error| format!("review_replay_snapshot_commit:{error}"))?;
    native_coordinator_original_review_expired_feedback(
        context, &session.lease, candidate_id, revision,
    )?;
    if verdict == "insufficient_evidence" && !outcome_requires_manual_execution_resolution(&outcome) {
        multi_agent_investigate_review_gap(context, session, candidate_id, revision, &missing)?;
    }
    if verdict == "confirmed" {
        Ok(outcome)
    } else {
        Ok(AgentTargetOutcome::incomplete(format!(
            "review_gate_{verdict}:{}",
            agent_text_truncated(payload["summary"].as_str().unwrap_or(""), 500),
        )))
    }
}

fn candidate_diff_only_settled_findings(previous: &str, current: &JsonValue) -> bool {
    let Ok(mut previous): Result<JsonValue, _> = serde_json::from_str(previous) else {
        return false;
    };
    let mut current = current.clone();
    let Some(previous_findings) = previous.get("findingCandidates").and_then(JsonValue::as_array) else {
        return false;
    };
    if previous_findings.is_empty()
        || current.get("findingCandidates").and_then(JsonValue::as_array)
            .is_none_or(|findings| !findings.is_empty())
    {
        return false;
    }
    previous.as_object_mut().unwrap().remove("findingCandidates");
    current.as_object_mut().unwrap().remove("findingCandidates");
    previous == current
}

/// A model's prose is never a new fact. Freeze references to attributable,
/// persisted observations so an insufficient review cannot be retried merely
/// by editing the in-memory candidate bundle. The Reviewer still determines
/// whether any of these facts actually closes its stated gap.
fn review_fact_refs(
    connection: &rusqlite::Connection,
    root_run_id: &str,
    target_dir: &Path,
) -> Result<Vec<String>, String> {
    let mut statement = connection.prepare(
        "WITH RECURSIVE ancestry(revision) AS ( \
           SELECT revision FROM agent_evidence_revisions \
           WHERE root_run_id=?1 AND revision=( \
             SELECT MAX(revision) FROM agent_evidence_revisions WHERE root_run_id=?1) \
           UNION ALL \
           SELECT r.parent_revision FROM agent_evidence_revisions r \
           JOIN ancestry a ON r.root_run_id=?1 AND r.revision=a.revision \
           WHERE r.parent_revision IS NOT NULL \
         ) \
         SELECT n.id,n.natural_key_hash,n.created_by_run_id,n.artifact_refs_json,n.payload_json,n.supersedes_id,n.revision,r.role \
         FROM agent_evidence_nodes n \
         JOIN agent_runs root ON root.id=n.root_run_id AND root.role='coordinator' \
         JOIN agent_runs r ON r.id=n.created_by_run_id AND r.root_run_id=root.id \
           AND r.scan_id=root.scan_id AND r.attempt_number=root.attempt_number \
           AND r.target_url=root.target_url AND r.orchestration_policy='multi' \
         JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id \
           AND a.coordinator_run_id=root.id AND a.role=r.role AND a.lane=r.lane \
           AND a.target_key=root.target_url AND a.evidence_revision=n.revision \
         WHERE n.root_run_id=?1 AND n.revision IN (SELECT revision FROM ancestry) \
           AND n.kind='request_record' \
           AND n.provenance='observed' AND r.role IN ('web_executor','external_surface') AND r.lane='target_touching' \
         ORDER BY n.id",
    ).map_err(|error| format!("无法读取审查事实索引：{error}"))?;
    let rows = statement.query_map([root_run_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?,
            row.get::<_, String>(2)?, row.get::<_, String>(3)?,
            row.get::<_, String>(4)?, row.get::<_, String>(5)?, row.get::<_, i64>(6)?,row.get::<_, String>(7)?))
    }).map_err(|error| format!("无法遍历审查事实索引：{error}"))?;
    let mut verified = Vec::new();
    for row in rows {
        let (id, natural_key, author_run, text, payload, supersedes_id, revision, role) =
            row.map_err(|error| format!("无法解码审查事实索引：{error}"))?;
        let artifacts: Vec<String> = serde_json::from_str(&text)
            .map_err(|_| "review_fact_invalid_artifact_refs")?;
        let [artifact] = artifacts.as_slice() else { continue };
        let Some(slot) = artifact.strip_prefix("agent-http/") else { continue };
        let expected_key = crate::agent_runtime::evidence_graph::store::evidence_natural_key(
            root_run_id,
            crate::agent_runtime::evidence_graph::contract::EvidenceNodeKind::RequestRecord,
            &format!("{author_run}:{slot}"),
        );
        if natural_key != expected_key || id != format!("ev-{}", &expected_key[..32]) {
            continue;
        }
        // A graph row or a nonempty artifact reference is not itself an
        // attributable observation. Only the current Broker format has a
        // revalidator; other source kinds stay visible in the graph but cannot
        // reopen an insufficient Reviewer until they have their own verifier.
        let payload: JsonValue = serde_json::from_str(&payload)
            .map_err(|_| "review_fact_invalid_payload")?;
        let valid = if role == "external_surface" {
            verified_public_surface_review_fact(connection,root_run_id,&author_run,slot,target_dir,&payload)
        } else { verified_web_http_review_fact(target_dir, &artifacts, &payload) };
        if valid {
            verified.push((id, supersedes_id, revision));
        }
    }
    // A later verified Broker fact can replace an earlier version. An inferred
    // node, forged child/assignment, missing artifact or same-revision claim
    // cannot retire a fact from the Reviewer's frozen manifest.
    let revisions: std::collections::HashMap<&str, i64> = verified.iter()
        .map(|(id, _, revision)| (id.as_str(), *revision)).collect();
    let mut retired = std::collections::HashSet::new();
    for (_, supersedes, revision) in &verified {
        if let Some(predecessor) = revisions.get(supersedes.as_str()) {
            if *predecessor < *revision
                && crate::agent_runtime::evidence_graph::store::evidence_revision_is_ancestor(
                    connection, root_run_id, *revision, *predecessor,
                )?
            {
                retired.insert(supersedes.clone());
            }
        }
    }
    Ok(verified.into_iter().filter_map(|(id, _, _)| {
        if retired.contains(id.as_str()) { None } else { Some(id) }
    }).collect())
}

fn has_new_review_fact(previous: &str, current: &JsonValue) -> bool {
    let Ok(previous): Result<JsonValue, _> = serde_json::from_str(previous) else {
        return false;
    };
    let Some(old) = previous.get("persistedFactRefs").and_then(JsonValue::as_array) else {
        // Historical snapshots without a fact manifest need explicit recovery.
        return false;
    };
    current.get("persistedFactRefs").and_then(JsonValue::as_array)
        .is_some_and(|facts| facts.iter().any(|fact| !old.contains(fact)))
}

