/// Promote only a broker-written HTTP record into a reviewable observation.
/// The graph never receives raw response bytes, credential headers or a URL
/// with potentially sensitive query parameters. A missing/tampered local record
/// or a stale child cannot manufacture a new review revision.
fn agent_record_http_observation(
    context: &AgentRunContext,
    tool: &str,
    artifact_id: &str,
    view: &JsonValue,
    request_id: &str,
) -> Result<(), String> {
    use crate::agent_runtime::{
        contract::MultiAgentPolicy,
        evidence_graph::{
            contract::{EvidenceNode, EvidenceNodeKind, EvidenceProvenance},
            store::{evidence_natural_key, insert_evidence_node, load_evidence_node},
        },
    };
    use rusqlite::TransactionBehavior;

    let Some(run) = &context.run else {
        // Direct single-agent fixtures do not have a persisted run.
        return Ok(());
    };
    if run.db_path != context.db_path {
        return Err("http_observation_assignment_or_lease_denied".into());
    }
    let connection = db::open(&run.db_path).map_err(|error| error.to_string())?;
    let (policy, role): (String, String) = connection.query_row(
        "SELECT orchestration_policy,role FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4",
        params![run.run_id, context.scan_id, context.attempt_number, context.target_url],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "http_observation_run_not_found".to_string())?;
    if policy != MultiAgentPolicy::Multi.as_str() {
        return if policy == MultiAgentPolicy::Single.as_str() && role == "coordinator" {
            agent_authorize_tool_on(&connection, context, tool).map_err(str::to_string)?;
            agent_single_require_fresh_on(&connection, context)
        } else {
            Err("http_observation_policy_denied".into())
        };
    }
    if request_id.is_empty()
        || view.get("requestId").and_then(JsonValue::as_str) != Some(request_id)
        || !view
            .get("status")
            .and_then(JsonValue::as_u64)
            .is_some_and(|s| (100..=599).contains(&s))
        || view
            .get("bodySha256")
            .and_then(JsonValue::as_str)
            .is_none_or(|s| s.len() != 64)
        || !artifact_id.ends_with(".json")
        || artifact_id.len() < 9
        || !artifact_id[..artifact_id.len() - 5]
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err("http_observation_record_invalid".into());
    }
    let directory = context.target_dir.join(AGENT_HTTP_DIRECTORY);
    let body_name = format!("{}.body", &artifact_id[..artifact_id.len() - 5]);
    let directory =
        open_agent_artifact_directory(&directory).ok_or("http_observation_artifact_missing")?;
    let (bytes, _) = read_agent_artifact_file(&directory, artifact_id, 1_048_576)
        .ok_or("http_observation_artifact_missing")?;
    let (body_bytes, body_len) =
        read_agent_artifact_file(&directory, &body_name, AGENT_MAX_RESPONSE_BYTES as u64 * 3)
            .ok_or("http_observation_payload_missing")?;
    if format!("{:x}", Sha256::digest(&body_bytes))
        != view
            .get("bodySha256")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
    {
        return Err("http_observation_payload_mismatch".into());
    }
    let record: JsonValue =
        serde_json::from_slice(&bytes).map_err(|_| "http_observation_artifact_invalid")?;
    if record.get("response") != Some(view)
        || record.get("payloadFile").and_then(JsonValue::as_str) != Some(body_name.as_str())
        || record.get("payloadBytes").and_then(JsonValue::as_u64) != Some(body_len)
    {
        return Err("http_observation_artifact_mismatch".into());
    }
    let transaction =
        rusqlite::Transaction::new_unchecked(&connection, TransactionBehavior::Immediate)
            .map_err(|error| format!("http_observation_lock_failed:{error}"))?;
    agent_authorize_tool_on(&transaction, context, tool)
        .map_err(str::to_string)?;
    let worker = crate::agent_runtime::multi_agent::attempts::require_live_for_run(
        &transaction,
        &run.run_id,
    )?;
    // The write lock makes the assignment, capability and fencing check atomic
    // with insertion. The earlier send-boundary check alone is not sufficient:
    // cancellation or lease revocation may happen while reading a response.
    let (root_run_id, revision): (String, i64) = transaction.query_row(
        "SELECT a.coordinator_run_id,a.evidence_revision FROM agent_runs r \
         JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id \
         JOIN agent_coordinator_leases c ON c.root_run_id=a.coordinator_run_id \
           AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url \
         JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id \
           AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane=a.lane \
         JOIN agent_capability_leases p ON p.assignment_id=a.id AND p.child_run_id=r.id \
           AND p.root_run_id=a.coordinator_run_id AND p.capability=?5 \
         WHERE r.id=?1 AND r.scan_id=?2 AND r.attempt_number=?3 AND r.target_url=?4 \
           AND EXISTS(SELECT 1 FROM sentinel_scans s WHERE s.id=r.scan_id \
             AND s.attempt_count=r.attempt_number AND s.status='scanning') \
           AND NOT EXISTS(SELECT 1 FROM sentinel_deleted_scans d WHERE d.scan_id=r.scan_id) \
           AND r.orchestration_policy='multi' AND r.status='running' \
           AND r.role='web_executor' AND r.lane='target_touching' AND r.root_run_id=a.coordinator_run_id \
           AND a.role=r.role AND a.lane=r.lane AND a.target_key=r.target_url AND a.state='running' \
           AND a.evidence_revision>0 AND a.lease_epoch=c.lease_epoch AND a.fencing_token=c.fencing_token \
           AND a.lease_expires_at>datetime('now','localtime') AND c.lease_expires_at>datetime('now','localtime') \
           AND p.lease_epoch=c.lease_epoch AND p.fencing_token=c.fencing_token \
           AND p.revoked_at='' AND p.lease_expires_at>datetime('now','localtime')",
        params![run.run_id, context.scan_id, context.attempt_number, context.target_url, tool],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "http_observation_assignment_or_lease_denied".to_string())?;
    let natural_key_hash = evidence_natural_key(
        &root_run_id,
        EvidenceNodeKind::RequestRecord,
        &format!("{}:{artifact_id}", run.run_id),
    );
    let mut observation_payload = json!({"requestId":request_id,"status":view["status"],
        "bodySha256":view["bodySha256"],"recordSha256":format!("{:x}",Sha256::digest(&bytes))});
    // Old artifacts without a transport binding keep their exact original shape.
    if let Some(claim) = view.get("httpClaim") {
        observation_payload["httpClaim"] = claim.clone();
    }
    let node = EvidenceNode {
        id: format!("ev-{}", &natural_key_hash[..32]),
        root_run_id: root_run_id.clone(),
        revision,
        kind: EvidenceNodeKind::RequestRecord,
        provenance: EvidenceProvenance::Observed,
        natural_key_hash,
        payload: observation_payload,
        artifact_refs: vec![format!("{AGENT_HTTP_DIRECTORY}/{artifact_id}")],
        created_by_run_id: run.run_id.clone(),
        supersedes_id: String::new(),
        created_at: String::new(),
    };
    insert_evidence_node(&transaction, &node)?;
    let stored = load_evidence_node(&transaction, &node.id)?
        .ok_or("http_observation_persistence_conflict")?;
    let mut expected = node;
    expected.created_at = stored.created_at.clone();
    if stored.created_at.is_empty() || stored != expected {
        return Err("http_observation_persistence_conflict".into());
    }
    agent_authorize_tool_on(&transaction, context, tool)
        .map_err(str::to_string)?;
    let revision_unchanged: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments
        WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND evidence_revision=?4)",
            params![worker.assignment_id, run.run_id, root_run_id, revision],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !revision_unchanged
        || crate::agent_runtime::multi_agent::attempts::require_live_for_run(
            &transaction,
            &run.run_id,
        )? != worker
    {
        return Err("http_observation_assignment_or_lease_denied".into());
    }
    transaction
        .commit()
        .map_err(|error| format!("http_observation_commit_failed:{error}"))
}

/// Recheck the exact artifact when the Reviewer freezes its next fact list.
/// An insertion-time check is insufficient if a file is later removed or
/// replaced. This intentionally accepts only the Web Broker's current format.
fn verified_web_http_review_fact(
    target_dir: &Path,
    artifacts: &[String],
    payload: &JsonValue,
) -> bool {
    let [artifact] = artifacts else { return false };
    let Some(name) = artifact.strip_prefix("agent-http/") else {
        return false;
    };
    let Some(slot) = name.strip_suffix(".json") else {
        return false;
    };
    if slot.len() < 4 || !slot.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let Some(expected_hash) = payload.get("recordSha256").and_then(JsonValue::as_str) else {
        return false;
    };
    if expected_hash.len() != 64 || !expected_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let directory = target_dir.join(AGENT_HTTP_DIRECTORY);
    let Some(directory) = open_agent_artifact_directory(&directory) else {
        return false;
    };
    let Some((bytes, _)) = read_agent_artifact_file(&directory, name, 1_048_576) else {
        return false;
    };
    if format!("{:x}", Sha256::digest(&bytes)) != expected_hash {
        return false;
    }
    let Ok(record) = serde_json::from_slice::<JsonValue>(&bytes) else {
        return false;
    };
    let Some((body_bytes, body_len)) = read_agent_artifact_file(
        &directory,
        &format!("{slot}.body"),
        AGENT_MAX_RESPONSE_BYTES as u64 * 3,
    ) else {
        return false;
    };
    if format!("{:x}", Sha256::digest(&body_bytes))
        != payload
            .get("bodySha256")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
    {
        return false;
    }
    record.get("payloadFile").and_then(JsonValue::as_str) == Some(format!("{slot}.body").as_str())
        && record.get("payloadBytes").and_then(JsonValue::as_u64) == Some(body_len)
        && record.pointer("/response/requestId") == payload.get("requestId")
        && record.pointer("/response/status") == payload.get("status")
        && record.pointer("/response/bodySha256") == payload.get("bodySha256")
}
