/// Bind a candidate to the exact persisted graph and revalidated Broker facts.
/// Candidate revision counts review attempts; evidence revision is a separate
/// ancestry head and must never be inferred from the review attempt number.
fn review_snapshot_manifest(
    connection: &rusqlite::Connection,
    root_run_id: &str,
    target_dir: &Path,
    candidate_text: &str,
) -> Result<(i64, String), String> {
    let candidate: JsonValue = serde_json::from_str(candidate_text)
        .map_err(|_| "review_manifest_invalid_candidate")?;
    let facts = review_fact_refs(connection, root_run_id, target_dir)?;
    let supplied = candidate.get("persistedFactRefs");
    if supplied.is_some_and(|value| *value != serde_json::json!(facts))
        || (supplied.is_none() && !facts.is_empty())
    {
        return Err("review_manifest_fact_refs_changed".into());
    }
    let head: i64 = connection.query_row(
        "SELECT COALESCE(MAX(revision),0) FROM agent_evidence_revisions WHERE root_run_id=?1",
        [root_run_id], |row| row.get(0),
    ).map_err(|error| format!("review_manifest_head_unavailable:{error}"))?;
    let mut revisions = Vec::new();
    let mut statement = connection.prepare(
        "SELECT revision,parent_revision,cause_event_id FROM agent_evidence_revisions \
         WHERE root_run_id=?1 ORDER BY revision",
    ).map_err(|error| format!("review_manifest_revisions_unavailable:{error}"))?;
    let rows = statement.query_map([root_run_id], |row| Ok(serde_json::json!([
        row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, String>(2)?,
    ]))).map_err(|error| format!("review_manifest_revisions_unavailable:{error}"))?;
    for row in rows { revisions.push(row.map_err(|error| format!("review_manifest_revision_invalid:{error}"))?); }
    let mut nodes = Vec::new();
    let mut statement = connection.prepare(
        "SELECT id,revision,kind,provenance,natural_key_hash,payload_json,artifact_refs_json,created_by_run_id,supersedes_id \
         FROM agent_evidence_nodes WHERE root_run_id=?1 ORDER BY id",
    ).map_err(|error| format!("review_manifest_nodes_unavailable:{error}"))?;
    let rows = statement.query_map([root_run_id], |row| Ok(serde_json::json!([
        row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?,
        row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
        row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?,
    ]))).map_err(|error| format!("review_manifest_nodes_unavailable:{error}"))?;
    for row in rows { nodes.push(row.map_err(|error| format!("review_manifest_node_invalid:{error}"))?); }
    let mut edges = Vec::new();
    let mut statement = connection.prepare(
        "SELECT id,revision,from_node_id,to_node_id,kind,payload_json,created_by_run_id \
         FROM agent_evidence_edges WHERE root_run_id=?1 ORDER BY id",
    ).map_err(|error| format!("review_manifest_edges_unavailable:{error}"))?;
    let rows = statement.query_map([root_run_id], |row| Ok(serde_json::json!([
        row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?,
        row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, String>(5)?,
        row.get::<_, String>(6)?,
    ]))).map_err(|error| format!("review_manifest_edges_unavailable:{error}"))?;
    for row in rows { edges.push(row.map_err(|error| format!("review_manifest_edge_invalid:{error}"))?); }
    let digest = crate::agent_runtime::store::stable_hash(&serde_json::json!({
        "schemaVersion":1, "rootRunId":root_run_id, "candidate":candidate_text,
        "evidenceRevision":head, "revisions":revisions, "nodes":nodes,
        "edges":edges, "verifiedFactRefs":facts,
    }).to_string());
    Ok((head, digest))
}

fn verify_review_snapshot(
    connection: &rusqlite::Connection,
    root_run_id: &str,
    candidate_id: &str,
    candidate_revision: i64,
    target_dir: &Path,
) -> Result<(), String> {
    let (candidate, evidence_revision, manifest_hash): (String, i64, String) = connection.query_row(
        "SELECT candidate_json,evidence_revision,manifest_hash FROM agent_review_requests \
         WHERE root_run_id=?1 AND candidate_id=?2 AND candidate_revision=?3",
        params![root_run_id, candidate_id, candidate_revision],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(|error| format!("review_manifest_request_unavailable:{error}"))?;
    if manifest_hash.is_empty() { return Err("review_manifest_legacy_unsealed_requires_recovery".into()); }
    let current = review_snapshot_manifest(connection, root_run_id, target_dir, &candidate)?;
    if current != (evidence_revision, manifest_hash) {
        return Err("review_manifest_changed_requires_new_revision".into());
    }
    Ok(())
}

/// Load only the independently completed and acknowledged Reviewer request.
/// The candidate and its fact refs must be the exact manifest-sealed version;
/// a mutable AgentRunContext is never an evidence source for this specialist.
#[cfg(test)]
fn sealed_gap_review_candidate(
    connection: &rusqlite::Connection,
    root_run_id: &str,
    candidate_id: &str,
    revision: i64,
    missing_evidence: &JsonValue,
    target_dir: &Path,
) -> Result<(JsonValue, JsonValue, Vec<String>), String> {
    let transaction = rusqlite::Transaction::new_unchecked(
        connection, rusqlite::TransactionBehavior::Immediate,
    ).map_err(|error| format!("gap_review_lock_unavailable:{error}"))?;
    let sealed = sealed_gap_review_candidate_in_transaction(
        &transaction, root_run_id, candidate_id, revision, missing_evidence, target_dir,
    )?;
    transaction.commit().map_err(|error| format!("gap_review_commit_failed:{error}"))?;
    Ok(sealed)
}

fn sealed_gap_review_candidate_in_transaction(
    transaction: &rusqlite::Connection,
    root_run_id: &str,
    candidate_id: &str,
    revision: i64,
    missing_evidence: &JsonValue,
    target_dir: &Path,
) -> Result<(JsonValue, JsonValue, Vec<String>), String> {
    if transaction.is_autocommit() { return Err("gap_original_snapshot_required".into()); }
    let stored: Option<(String, String, String)> = transaction.query_row(
        "SELECT q.candidate_json,d.missing_evidence_json,m.payload_json \
         FROM agent_review_requests q \
         JOIN agent_review_decisions d ON d.id=q.decision_id AND d.root_run_id=q.root_run_id \
           AND d.candidate_id=q.candidate_id AND d.candidate_revision=q.candidate_revision \
           AND d.reviewer_run_id=q.reviewer_run_id AND d.verdict='insufficient_evidence' \
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
           AND q.status='insufficient_evidence' LIMIT 1",
        params![root_run_id, candidate_id, revision],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional().map_err(|error| format!("gap_review_lookup_failed:{error}"))?;
    let (candidate_text, missing_text, message_text) =
        stored.ok_or("gap_review_request_unavailable")?;
    let stored_missing: JsonValue = serde_json::from_str(&missing_text)
        .map_err(|_| "gap_review_missing_evidence_invalid")?;
    if stored_missing != *missing_evidence {
        return Err("gap_review_missing_evidence_mismatch".into());
    }
    let message: JsonValue = serde_json::from_str(&message_text)
        .map_err(|_| "gap_review_decision_message_invalid")?;
    if message["candidateId"] != candidate_id || message["candidateRevision"] != revision
        || message["verdict"] != "insufficient_evidence"
    {
        return Err("gap_review_decision_message_mismatch".into());
    }
    verify_review_snapshot(transaction, root_run_id, candidate_id, revision, target_dir)?;
    let candidate: JsonValue = serde_json::from_str(&candidate_text)
        .map_err(|_| "gap_review_candidate_invalid")?;
    let trusted_refs: Vec<String> = serde_json::from_value(
        candidate.get("persistedFactRefs").cloned().unwrap_or_else(|| serde_json::json!([])),
    ).map_err(|_| "gap_review_fact_refs_invalid")?;
    Ok((candidate, stored_missing, trusted_refs))
}
