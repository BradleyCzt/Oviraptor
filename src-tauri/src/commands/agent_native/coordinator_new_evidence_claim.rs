// A graph/artifact writer is insufficient: require the original transport claim
// and the original target budget receipt before this fact may cost Root tokens.
fn native_new_evidence_claim_rows(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    target_dir: &Path,
    node: &crate::agent_runtime::evidence_graph::contract::EvidenceNode,
) -> Result<Vec<NativeCoordinatorFrozenRows>, String> {
    let bound = &node.payload["httpClaim"];
    if bound.as_object().is_none_or(|o| o.len() != 7)
        || bound["schemaVersion"] != 1
        || bound["runId"] != node.created_by_run_id
        || ["requestIndex", "budgetAttempt", "ordinal"]
            .iter()
            .any(|key| bound[*key].as_i64().is_none_or(|n| n <= 0))
        || bound["invocationId"].as_str().is_none_or(str::is_empty)
        || bound["requestHash"]
            .as_str()
            .is_none_or(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("root_new_evidence_http_claim_missing".into());
    }
    let name = node.artifact_refs[0]
        .strip_prefix("agent-http/")
        .ok_or("root_new_evidence_source_invalid")?;
    let directory = open_agent_artifact_directory(&target_dir.join(AGENT_HTTP_DIRECTORY))
        .ok_or("root_new_evidence_source_invalid")?;
    let (bytes, _) = read_agent_artifact_file(&directory, name, 1_048_576)
        .ok_or("root_new_evidence_source_invalid")?;
    let record: JsonValue =
        serde_json::from_slice(&bytes).map_err(|_| "root_new_evidence_source_invalid")?;
    if record["response"]["httpClaim"] != *bound {
        return Err("root_new_evidence_http_claim_conflict".into());
    }
    let invocation = bound["invocationId"].as_str().unwrap();
    let index = bound["requestIndex"].as_i64().unwrap().to_string();
    let budget_attempt = bound["budgetAttempt"].as_i64().unwrap().to_string();
    let ordinal = bound["ordinal"].as_i64().unwrap().to_string();
    let attempt = actor.attempt_number.to_string();
    let status = node.payload["status"]
        .as_i64()
        .ok_or("root_new_evidence_source_invalid")?
        .to_string();
    let hash = bound["requestHash"].as_str().unwrap();
    let source = format!(
        "http:{}:{invocation}:{index}:{hash}",
        node.created_by_run_id
    );
    // Freeze only immutable claim/receipt rows, never live balances or C expiry.
    let claim_row = NativeCoordinatorFrozenRows::capture(
        db,
        "SELECT c.* FROM agent_http_request_claims c
         JOIN tool_invocations i ON i.run_id=c.run_id AND i.invocation_id=c.invocation_id
           AND i.tool_name=c.tool_name AND i.policy_decision='allow'
         JOIN agent_runs r ON r.id=c.run_id
         JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id
         JOIN agent_assignment_attempts x ON x.assignment_id=a.id AND x.child_run_id=r.id
           AND x.root_run_id=a.coordinator_run_id
           AND x.coordinator_epoch=a.lease_epoch AND x.coordinator_fencing_token=a.fencing_token
         WHERE c.scan_id=?1 AND c.target_url=?2 AND c.attempt_number=CAST(?3 AS INTEGER)
           AND c.run_id=?4 AND c.invocation_id=?5 AND c.request_index=CAST(?6 AS INTEGER)
           AND c.budget_attempt=CAST(?7 AS INTEGER) AND c.ordinal=CAST(?8 AS INTEGER)
           AND c.request_hash=?9 AND c.response_status=CAST(?10 AS INTEGER) AND c.received_at<>''
           AND a.coordinator_run_id=?11
           AND EXISTS(SELECT 1 FROM agent_budget_entries b WHERE b.root_run_id=?11
             AND b.assignment_id=a.id AND b.lease_attempt_id=x.id AND b.dimension='target_requests'
             AND b.kind='forfeit' AND b.amount=1 AND b.source_id=?12)
           AND EXISTS(SELECT 1 FROM agent_budget_entries b WHERE b.root_run_id=?11
             AND b.assignment_id=a.id AND b.lease_attempt_id=x.id AND b.dimension='target_requests'
             AND b.kind='reconcile' AND b.amount=1 AND b.source_id=?12)",
        &[
            &actor.scan_id,
            &actor.target_key,
            &attempt,
            &node.created_by_run_id,
            invocation,
            &index,
            &budget_attempt,
            &ordinal,
            hash,
            &status,
            &actor.root_run_id,
            &source,
        ],
    )
    .map_err(|_| "root_new_evidence_http_receipt_invalid".to_string())?;
    let (assignment, attempt) =
        crate::agent_runtime::multi_agent::budget::target::original_target_receipt(
            db,
            actor,
            &node.created_by_run_id,
            &source,
        )?;
    Ok(vec![claim_row,
        NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2
             AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4 ORDER BY rowid",
            &[&actor.root_run_id,&assignment,&attempt,&source])?,
        NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,* FROM tool_invocations WHERE run_id=?1 AND invocation_id=?2",
            &[&node.created_by_run_id,invocation])?])
}
