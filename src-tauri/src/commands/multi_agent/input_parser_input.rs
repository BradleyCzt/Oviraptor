// U178: build and enforce the InputParser readonly SDK input surface.
//
// The surface is derived only from the minted child's frozen task slice (U177),
// whose `factBindings` are exactly the U175 frame's verified Broker records. Each
// cited fact is re-loaded and re-verified here: it must be this Root's own,
// observed request_record, its hashes must still match the frozen Broker artifact,
// and the hash-verified record is read for redacted metadata only — method, path,
// observed query parameter names, body presence. Raw request/body files are never
// opened. A tampered record, a fact re-scoped to another run, a missing binding or
// any drift fails closed before a single provider byte, writing nothing.
fn input_parser_assignment_slice(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
) -> Result<(JsonValue, String), String> {
    use crate::agent_runtime::contract::AgentRole;
    let (text, caps): (String, String) = db
        .query_row(
            "SELECT task_slice_json,capability_lease_json FROM agent_assignments \
             WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND role='input_parser' \
               AND lane='read_only_analysis' AND target_key=?4 AND lease_epoch=?5 AND fencing_token=?6",
            params![
                child.assignment_id,
                child.run_id,
                lease.root_run_id,
                lease.target_key,
                lease.lease_epoch,
                lease.fencing_token
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| "input_parser_assignment_invalid")?;
    let caps: Vec<String> =
        serde_json::from_str(&caps).map_err(|_| "input_parser_assignment_invalid")?;
    if caps != ["evidence.read", "mailbox.write"] {
        return Err("input_parser_capability_invalid".into());
    }
    let slice: JsonValue = serde_json::from_str(&text).map_err(|_| "input_parser_task_invalid")?;
    if slice["role"] != AgentRole::InputParser.as_str() {
        return Err("input_parser_task_scope_invalid".into());
    }
    let decision_hash = slice["frame"]["decisionHash"]
        .as_str()
        .ok_or("input_parser_task_scope_invalid")?
        .to_string();
    Ok((slice, decision_hash))
}

fn input_parser_build_surface(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    target_dir: &Path,
) -> Result<crate::agent_runtime::multi_agent::input_parser::Task, String> {
    use crate::agent_runtime::{
        evidence_graph::{
            contract::{EvidenceNodeKind, EvidenceProvenance},
            store::load_evidence_node,
        },
        multi_agent::input_parser::{self, Parameter, Surface},
    };
    let (slice, decision_hash) = input_parser_assignment_slice(db, lease, child)?;
    let bindings = slice["frame"]["factBindings"]
        .as_array()
        .filter(|b| !b.is_empty())
        .ok_or("input_parser_task_scope_invalid")?;
    let mut surfaces = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let evidence_ref = binding["evidenceRef"]
            .as_str()
            .ok_or("input_parser_surface_invalid")?;
        let node = load_evidence_node(db, evidence_ref)?.ok_or("input_parser_surface_fact_missing")?;
        if node.root_run_id != lease.root_run_id
            || node.kind != EvidenceNodeKind::RequestRecord
            || node.provenance != EvidenceProvenance::Observed
            || node.revision <= 0
            || node.payload["recordSha256"] != binding["recordSha256"]
            || node.payload["bodySha256"] != binding["bodySha256"]
            || node.payload["requestId"] != binding["requestId"]
            || serde_json::to_value(&node.artifact_refs).unwrap_or(JsonValue::Null)
                != binding["artifactRefs"]
            || !verified_web_http_review_fact(target_dir, &node.artifact_refs, &node.payload)
        {
            return Err("input_parser_surface_fact_invalid".into());
        }
        let [artifact] = node.artifact_refs.as_slice() else {
            return Err("input_parser_surface_fact_invalid".into());
        };
        let name = artifact
            .strip_prefix("agent-http/")
            .ok_or("input_parser_surface_fact_invalid")?;
        let directory = open_agent_artifact_directory(&target_dir.join(AGENT_HTTP_DIRECTORY))
            .ok_or("input_parser_artifact_root_invalid")?;
        let (bytes, _) = read_agent_artifact_file(&directory, name, 1_048_576)
            .ok_or("input_parser_record_missing")?;
        let record: JsonValue =
            serde_json::from_slice(&bytes).map_err(|_| "input_parser_record_invalid")?;
        let request = &record["request"];
        let method = request["method"]
            .as_str()
            .ok_or("input_parser_record_invalid")?
            .to_string();
        let url = request["url"].as_str().ok_or("input_parser_record_invalid")?;
        let parsed = reqwest::Url::parse(url).map_err(|_| "input_parser_record_invalid")?;
        let mut names: Vec<String> = parsed
            .query_pairs()
            .map(|(k, _)| k.to_string())
            .filter(|k| !k.trim().is_empty())
            .collect();
        names.sort();
        names.dedup();
        let parameters: Vec<Parameter> = names
            .into_iter()
            .map(|name| Parameter {
                name,
                location: "query".into(),
            })
            .collect();
        let body_present = request["bodyBytes"].as_u64().is_some_and(|n| n > 0);
        if parameters.is_empty() && !body_present {
            return Err("input_parser_surface_fact_invalid".into());
        }
        surfaces.push(Surface {
            evidence_ref: evidence_ref.to_string(),
            request_id: binding["requestId"].as_str().unwrap_or_default().to_string(),
            record_sha256: binding["recordSha256"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            body_sha256: binding["bodySha256"].as_str().map(str::to_string),
            method,
            path: parsed.path().to_string(),
            parameters,
            body_present,
        });
    }
    let task = input_parser::Task {
        schema_version: 1,
        phase: input_parser::PHASE.into(),
        root_run_id: lease.root_run_id.clone(),
        scan_id: lease.scan_id.clone(),
        attempt_number: lease.attempt_number,
        target: lease.target_key.clone(),
        coordinator_epoch: lease.lease_epoch,
        coordinator_fence: lease.fencing_token.clone(),
        decision_hash: decision_hash.clone(),
        target_requests_granted: 0,
        browser_actions_granted: 0,
        tools_granted: Vec::new(),
        surfaces,
    };
    // Round-trip the built surface through the strict validator: it binds to the
    // lease and the exact paid frame decision hash, and rejects any out-of-scope
    // grant, redaction leak or oversized surface.
    let value = serde_json::to_value(&task).map_err(|e| e.to_string())?;
    input_parser::Task::from_value(&value, lease, &decision_hash)
}

// The SDK input the child actually receives must equal the authorized surface
// rebuilt under the dispatch transaction; a swapped or loosened input is refused.
fn input_parser_verify_input(
    context: &SpecialistTransportContext<'_>,
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    input: &JsonValue,
) -> Result<(), String> {
    let task = input_parser_build_surface(db, lease, child, context.usage_dir)?;
    if serde_json::to_value(&task).map_err(|e| e.to_string())? != *input {
        return Err("input_parser_dispatch_surface_changed".into());
    }
    Ok(())
}

fn input_parser_model_budget(
    messages: &[JsonValue],
    profile: &AgentModelProfile,
) -> Result<i64, String> {
    let bytes = serde_json::to_vec(messages)
        .map_err(|_| "input_parser_input_invalid")?
        .len();
    let tokens = i64::try_from(bytes)
        .ok()
        .and_then(|n| n.checked_add(512 + 256))
        .filter(|n| *n <= 4000)
        .ok_or("input_parser_input_oversized")?;
    if profile.max_context_tokens > 0 && tokens as u64 > profile.max_context_tokens {
        return Err("input_parser_context_budget_exceeded".into());
    }
    Ok(tokens)
}

// Delivery-time re-verification: the readonly surface is unchanged, the receipt
// is the one the provider actually returned, and the output stays inside the
// strict parameter schema. Runs inside the delivery transaction, so any failure
// rolls back with zero writes.
fn input_parser_validate_received(
    db: &rusqlite::Connection,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    usage: &AgentTokenUsage,
    payload: &JsonValue,
    target_dir: Option<&Path>,
) -> Result<(), String> {
    use crate::agent_runtime::multi_agent::{input_parser, specialist};
    let task = input_parser_build_surface(
        db,
        lease,
        child,
        target_dir.ok_or("input_parser_artifact_root_required")?,
    )?;
    if payload["inputParserTask"] != serde_json::to_value(&task).map_err(|e| e.to_string())? {
        return Err("input_parser_delivery_task_changed".into());
    }
    let receipt = specialist::received_for_local_delivery(db, lease, child)?;
    let text = payload["summary"]
        .as_str()
        .ok_or("input_parser_response_missing")?;
    if receipt.text != text || receipt.usage != *usage || !receipt.rejection.is_empty() {
        return Err("input_parser_delivery_receipt_mismatch".into());
    }
    input_parser::validate_assessment(text, &task)
}
