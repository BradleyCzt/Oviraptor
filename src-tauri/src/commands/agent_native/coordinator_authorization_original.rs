// Pure reconstruction of original three-side Authorization work. Each side
// requires its first-response private birth-bound issuance, original physical
// facts and exact original artifact bytes. Never repair or backfill a seal.
struct NativeAuthorizationOriginalFinding {
    rows: Vec<NativeCoordinatorFrozenRows>,
    bounded_facts: JsonValue,
}

fn native_authorization_original_finding_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    target_dir: &Path,
    finding: &JsonValue,
) -> Result<NativeAuthorizationOriginalFinding, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{attempts, budget, scheduler},
    };
    if db.is_autocommit() {
        return Err("authorization_original_snapshot_required".into());
    }
    crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(db, actor)?;
    crate::agent_runtime::multi_agent::lease::require_executable_coordinator(db, actor)?;
    let mode = private_web_mode_on(db, &actor.scan_id, actor.attempt_number)?;
    let directory =
        checked_evidence_directory(Path::new(&mode.fact().work_dir), target_dir, &actor.scan_id)?;
    if directory != target_dir
        || resolve_agent_evidence_location(db, &actor.root_run_id)? != directory
    {
        return Err("authorization_original_directory_conflict".into());
    }
    let record = &finding["record"];
    let key = record["contractKey"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("authorization_original_finding_invalid")?;
    let finding_id = finding["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("authorization_original_finding_invalid")?;
    let controls = authorization_original_controls(db, actor)?;
    let control = controls
        .iter()
        .find(|c| c.contract_key == key)
        .ok_or("authorization_original_control_missing")?;
    let sides = record["sides"]
        .as_array()
        .filter(|s| s.len() == 3)
        .ok_or("authorization_original_finding_invalid")?;
    let claims: Vec<(String, String, String, String)> = {
        let mut q=db.prepare("SELECT side,child_run_id,assignment_id,artifact_id FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND contract_key=?4 ORDER BY side").map_err(|e|e.to_string())?;
        let mapped = q
            .query_map(
                params![actor.scan_id, actor.attempt_number, actor.target_key, key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(|e| e.to_string())?;
        let mut found = Vec::new();
        for row in mapped {
            found.push(row.map_err(|e| e.to_string())?);
        }
        found
    };
    if claims.len() != 3
        || claims
            .iter()
            .any(|c| c.1 != claims[0].1 || c.2 != claims[0].2 || c.3.is_empty())
    {
        return Err("authorization_original_claims_conflict".into());
    }
    let child = scheduler::ScheduledChild {
        run_id: claims[0].1.clone(),
        assignment_id: claims[0].2.clone(),
        role: AgentRole::Authorization,
    };
    let worker = attempts::current(db, actor, &child.assignment_id)?;
    let (task,revision):(String,i64)=db.query_row(r#"SELECT a.task_slice_json,a.evidence_revision FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND a.target_key=?4 AND a.role='authorization' AND a.lane='target_touching' AND a.trigger_code='operator_authorization_control_ready' AND a.state='completed' AND a.budget_settled_at<>'' AND a.finished_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.lease_epoch=?5 AND a.fencing_token=?6 AND a.capability_lease_json='["authorization_probe"]' AND a.contract_keys_json='[]' AND a.identity_handles_json='[]' AND r.backend='native' AND r.orchestration_policy='multi' AND r.role=a.role AND r.lane=a.lane AND r.scan_id=?7 AND r.attempt_number=?8 AND r.target_url=?4 AND r.root_run_id=?3 AND r.parent_run_id=?3 AND r.assignment_id=a.id AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>'' AND r.used_tokens=0 AND r.used_cached_tokens=0 AND r.used_requests=0 AND r.cancel_requested_at=''"#,
        params![child.assignment_id,child.run_id,actor.root_run_id,actor.target_key,actor.lease_epoch,actor.fencing_token,actor.scan_id,actor.attempt_number],|r|Ok((r.get(0)?,r.get(1)?)))
        .map_err(|_|"authorization_original_child_conflict")?;
    let expected_task = crate::agent_runtime::secrets::redact_json(
        &json!({"contractKeys":controls.iter().map(|c|&c.contract_key).collect::<Vec<_>>(),"objective":"仅执行已登记的 A→X、B→X、B→Y GET 控制组"}),
    );
    let expected_task_text = expected_task.to_string();
    if task != expected_task_text
        || revision != 1
        || worker.child_run_id != child.run_id
        || worker.state != "completed"
        || worker.finished_at.is_empty()
    {
        return Err("authorization_original_task_conflict".into());
    }
    let released:bool=db.query_row("SELECT NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='') AND NOT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1)",params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let original_cap:bool=db.query_row("SELECT count(*)=1 FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND capability='authorization_probe' AND lease_epoch=?4 AND fencing_token=?5 AND revoked_at<>''",params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !released || !original_cap {
        return Err("authorization_original_child_not_closed".into());
    }
    let claim_count:i64=db.query_row("SELECT count(*) FROM agent_authorization_probe_claims WHERE child_run_id=?1 AND assignment_id=?2 AND scan_id=?3 AND attempt_number=?4 AND target_url=?5 AND artifact_id<>'' AND contract_key IN(SELECT value FROM json_each(?6,'$.contractKeys'))",params![child.run_id,child.assignment_id,actor.scan_id,actor.attempt_number,actor.target_key,task],|r|r.get(0)).map_err(|e|e.to_string())?;
    let all_count:i64=db.query_row("SELECT count(*) FROM agent_authorization_probe_claims WHERE child_run_id=?1 OR assignment_id=?2",params![child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if claim_count != all_count || claim_count != 3 * controls.len() as i64 {
        return Err("authorization_original_claims_conflict".into());
    }
    for dimension in budget::DIMENSIONS {
        let b = budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            &worker.id,
            dimension,
        )?;
        if b.reserved != 0
            || b.indeterminate != 0
            || dimension == "target_requests" && b.consumed != claim_count
            || dimension != "target_requests" && dimension != "wall_time_ms" && b.consumed != 0
        {
            return Err("authorization_original_cost_conflict".into());
        }
    }
    let mut responses = Vec::new();
    let mut hashes = Vec::new();
    let mut sealed_rows = Vec::new();
    let issuer=original_withdrawal_issuer_on(db,actor)?.ok_or("authorization_original_issuer_missing")?;
    let seal_count:i64=db.query_row("SELECT count(*) FROM main.agent_authorization_side_receipts WHERE child_run_id=?1 OR assignment_id=?2",params![child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if seal_count != claim_count {return Err("authorization_original_seal_count_conflict".into());}
    let mut artifacts = std::collections::BTreeSet::new();
    for (index, side) in ["owner", "cross", "tester"].into_iter().enumerate() {
        let claim = claims
            .iter()
            .find(|c| c.0 == side)
            .ok_or("authorization_original_claims_conflict")?;
        if !artifacts.insert(claim.3.clone())
            || sides[index] != json!({"name":side,"artifactId":claim.3})
        {
            return Err("authorization_original_finding_claim_conflict".into());
        }
        let source = format!(
            "authorization:{}:{}:{side}",
            child.assignment_id,
            crate::agent_runtime::store::stable_hash(key)
        );
        let owner = budget::target::original_target_receipt(db, actor, &child.run_id, &source)?;
        if owner != (child.assignment_id.clone(), worker.id.clone()) {
            return Err("authorization_original_cost_conflict".into());
        }
        let (response, hash) =
            authorization_original_artifact(target_dir, control, side, &claim.3)?;
        let receipt:String=db.query_row("SELECT receipt_id FROM main.agent_authorization_side_receipts WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND contract_key=?5 AND side=?6 AND child_run_id=?7 AND assignment_id=?8 AND worker_attempt_id=?9 AND artifact_id=?10",
            params![actor.root_run_id,actor.scan_id,actor.attempt_number,actor.target_key,key,side,child.run_id,child.assignment_id,worker.id,claim.3],|r|r.get(0)).map_err(|_|"authorization_original_side_seal_missing")?;
        let raw=issuer.verify_authorization_side_issued(db,&receipt)?;
        let fact:JsonValue=serde_json::from_str(&raw).map_err(|_|"authorization_original_side_seal_invalid")?;
        if fact["coordinatorEpoch"] != actor.lease_epoch || fact["coordinatorFence"] != actor.fencing_token
            || fact["recordSha256"] != hash["recordSha256"] || fact["bodySha256"] != hash["bodySha256"]
            || fact["recordBytes"] != hash["recordBytes"] || fact["bodyBytes"] != hash["bodyBytes"] {
            return Err("authorization_original_signed_artifact_changed".into());
        }
        let original_rows=fact["originalRows"].as_array().filter(|r|r.len()==6).ok_or("authorization_original_side_facts_invalid")?;
        for row in original_rows {
            let sql=row["sql"].as_str().filter(|s|s.starts_with("SELECT ") && !s.contains(';')).ok_or("authorization_original_side_facts_invalid")?;
            let frozen=NativeCoordinatorFrozenRows{sql:sql.into(),keys:serde_json::from_value(row["keys"].clone()).map_err(|_|"authorization_original_side_facts_invalid")?,hash:row["hash"].as_str().ok_or("authorization_original_side_facts_invalid")?.into(),allow_empty:false};
            frozen.verify(db)?;
            sealed_rows.push(frozen);
        }
        sealed_rows.push(NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,* FROM main.agent_authorization_side_receipts WHERE receipt_id=?1",&[&receipt])?);
        responses.push(response);
        hashes.push(hash);
    }
    verify_authorization_control_group(control, [&responses[0], &responses[1], &responses[2]])
        .map_err(|_| "authorization_original_control_group_invalid")?;
    let expected_record = json!({"kind":"authorization_cross_object_read","contractKey":key,
        "verification":"operator_bound_three_side_native_control","responseObjectPointer":control.response_object_pointer,
        "method":"GET","sides":sides,"attemptNumber":actor.attempt_number});
    let expected_key = format!(
        "authorization-control:{}",
        &crate::agent_runtime::store::stable_hash(&format!(
            "{}:{}:{key}",
            actor.scan_id, actor.attempt_number
        ))[..24]
    );
    let bound:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_finding_candidates WHERE id=?1 AND root_run_id=?2 AND scan_id=?3 AND target_url=?4 AND stage=?5 AND kind='vulnerability' AND record_key=?6 AND record_json=?7)",params![finding_id,actor.root_run_id,actor.scan_id,actor.target_key,AGENT_VULNERABILITY_STAGE,expected_key,expected_record.to_string()],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !bound
        || *record != expected_record
        || finding["recordKey"] != expected_key
        || finding["stage"] != AGENT_VULNERABILITY_STAGE
        || finding["kind"] != "vulnerability"
    {
        return Err("authorization_original_finding_conflict".into());
    }
    let message =
        authorization_original_delivery(db, actor, &child, controls.len() as i64, claim_count)?;
    let mut rows = sealed_rows;
    let attempt = actor.attempt_number.to_string();
    for (sql,keys) in [
        ("SELECT rowid,* FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=CAST(?2 AS INTEGER) AND target_url=?3 ORDER BY contract_key",vec![actor.scan_id.as_str(),attempt.as_str(),actor.target_key.as_str()]),
        ("SELECT rowid,* FROM agent_authorization_probe_claims WHERE child_run_id=?1 OR assignment_id=?2 ORDER BY rowid",vec![child.run_id.as_str(),child.assignment_id.as_str()]),
        ("SELECT rowid,* FROM agent_evidence_locations WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_assignments WHERE id=?1",vec![child.assignment_id.as_str()]),
        ("SELECT rowid,* FROM agent_runs WHERE id=?1",vec![child.run_id.as_str()]),
        ("SELECT rowid,* FROM agent_assignment_attempts WHERE id=?1",vec![worker.id.as_str()]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE lease_attempt_id=?1 ORDER BY rowid",vec![worker.id.as_str()]),
        ("SELECT rowid,* FROM agent_messages WHERE id=?1",vec![message.as_str()]),
        ("SELECT rowid,* FROM agent_capability_leases WHERE assignment_id=?1 ORDER BY rowid",vec![child.assignment_id.as_str()]),
        ("SELECT rowid,* FROM agent_collaboration_events WHERE entity_id IN(?1,?2,?3) ORDER BY sequence",vec![child.assignment_id.as_str(),child.run_id.as_str(),message.as_str()]),
    ] {
        rows.push(NativeCoordinatorFrozenRows::capture(db,sql,&keys)?);
    }
    rows.push(NativeCoordinatorFrozenRows::capture_optional(
        db,
        "SELECT rowid,* FROM agent_lane_leases WHERE assignment_id=?1 ORDER BY rowid",
        &[&child.assignment_id],
    )?);
    rows.push(NativeCoordinatorFrozenRows::capture(db,"SELECT name,type,tbl_name,sql FROM sqlite_master WHERE tbl_name IN('agent_authorization_controls','agent_authorization_probe_claims','agent_authorization_side_receipts','agent_budget_entries','agent_messages','agent_collaboration_events') ORDER BY name",&[])?);
    // Current bytes match their verified first-write private receipts.
    Ok(NativeAuthorizationOriginalFinding {
        rows,
        bounded_facts: json!({"schema":1,
        "rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,
        "childRunId":child.run_id,"assignmentId":child.assignment_id,"workerAttemptId":worker.id,
        "contractKey":key,"originalArtifactSealed":true,"artifactProvenance":"original_signed_receipt",
        "artifacts":hashes,"dispatchGranted":false,"targetRequestsGranted":0}),
    })
}

fn authorization_original_controls(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
) -> Result<Vec<SaveAuthorizationControlInput>, String> {
    let mut q=db.prepare("SELECT contract_key,owner_object_url,tester_control_url,object_query_key,owner_object_value,tester_object_value,response_object_pointer,owner_identity,tester_identity FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND method='GET' ORDER BY contract_key").map_err(|e|e.to_string())?;
    let controls = q
        .query_map(
            params![actor.scan_id, actor.attempt_number, actor.target_key],
            |r| {
                Ok(SaveAuthorizationControlInput {
                    scan_id: actor.scan_id.clone(),
                    attempt_number: actor.attempt_number,
                    target_url: actor.target_key.clone(),
                    contract_key: r.get(0)?,
                    owner_object_url: r.get(1)?,
                    tester_control_url: r.get(2)?,
                    object_query_key: r.get(3)?,
                    owner_object_value: r.get(4)?,
                    tester_object_value: r.get(5)?,
                    response_object_pointer: r.get(6)?,
                    owner_identity: r.get(7)?,
                    tester_identity: r.get(8)?,
                })
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if controls.is_empty() || controls.len() > 4 {
        return Err("authorization_original_control_invalid".into());
    }
    for c in &controls {
        if c.owner_identity.is_empty()
            || c.tester_identity.is_empty()
            || c.owner_identity == c.tester_identity
            || c.contract_key.trim().is_empty()
            || c.object_query_key.trim().is_empty()
            || c.owner_object_value.trim().is_empty()
            || c.tester_object_value.trim().is_empty()
            || c.owner_object_value == c.tester_object_value
            || !valid_authorization_pointer(&c.response_object_pointer)
            || authorization_control_urls(c)?
                != (c.owner_object_url.clone(), c.tester_control_url.clone())
        {
            return Err("authorization_original_control_invalid".into());
        }
    }
    Ok(controls)
}

fn authorization_original_delivery(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control_count: i64,
    claim_count: i64,
) -> Result<String, String> {
    let correlation = format!("authorization:{}", child.assignment_id);
    let dedup = format!(
        "{}:authorization_result:{correlation}:1",
        child.assignment_id
    );
    let mut q=db.prepare("SELECT id FROM agent_messages WHERE root_run_id=?1 AND run_id=?1 AND assignment_id=?2 AND from_run_id=?3 AND to_run_id=?1 AND from_agent='authorization' AND to_agent='coordinator' AND kind='authorization_result' AND correlation_id=?4 AND evidence_revision=1 AND dedup_key=?5 AND artifact_refs_json='[]'").map_err(|e|e.to_string())?;
    let ids = q
        .query_map(
            params![
                actor.root_run_id,
                child.assignment_id,
                child.run_id,
                correlation,
                dedup
            ],
            |r| r.get::<_, String>(0),
        )
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    if ids.len() != 1 {
        return Err("authorization_original_delivery_conflict".into());
    }
    let message =
        crate::agent_runtime::multi_agent::mailbox::verified_completed_output(db, actor, &ids[0])?;
    let verified = message.payload["verified"]
        .as_i64()
        .filter(|v| *v > 0 && *v <= control_count)
        .ok_or("authorization_original_delivery_conflict")?;
    let expected = crate::agent_runtime::secrets::redact_json(
        &json!({"summary":format!("authorization controls verified {verified}/{control_count}"),"verified":verified,"requestCount":claim_count}),
    );
    if message.payload != expected {
        return Err("authorization_original_delivery_conflict".into());
    }
    crate::collaboration_events::client_delivery_schema::verify(db)?;
    let (events,created,ack):(i64,i64,i64)=db.query_row("SELECT count(*),sum(e.scan_id=?2 AND e.attempt_number=?3 AND e.entity_type='mailbox_message' AND e.event_type=e.entity_type AND e.payload_json=json_object('kind',m.kind,'deliveredAt','','acknowledgedAt','') AND e.created_at=m.created_at),sum(e.scan_id=?2 AND e.attempt_number=?3 AND e.entity_type='mailbox_message' AND e.event_type=e.entity_type AND e.payload_json=json_object('kind',m.kind,'deliveredAt',m.delivered_at,'acknowledgedAt',m.acknowledged_at) AND e.created_at>=m.acknowledged_at) FROM agent_collaboration_events e JOIN agent_messages m ON m.id=e.entity_id WHERE e.entity_id=?1",params![ids[0],actor.scan_id,actor.attempt_number],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|e|e.to_string())?;
    let (closures,kinds):(i64,i64)=db.query_row("SELECT count(*),count(DISTINCT entity_type) FROM agent_collaboration_events WHERE scan_id=?1 AND attempt_number=?2 AND entity_type=event_type AND ((entity_type='assignment' AND entity_id=?3 AND payload_json=json_object('role','authorization','state','completed')) OR (entity_type='agent_run' AND entity_id=?4 AND payload_json=json_object('role','authorization','status','terminal','terminalState','completed')))",params![actor.scan_id,actor.attempt_number,child.assignment_id,child.run_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?;
    if (events, created, ack) != (2, 1, 1) || (closures, kinds) != (2, 2) {
        return Err("authorization_original_events_conflict".into());
    }
    Ok(ids[0].clone())
}

fn authorization_original_artifact(
    target_dir: &Path,
    control: &SaveAuthorizationControlInput,
    side: &str,
    artifact: &str,
) -> Result<(AuthorizationProbeResponse, JsonValue), String> {
    use sha2::Digest;
    let slot = artifact
        .strip_suffix(".json")
        .filter(|s| s.len() >= 4 && s.bytes().all(|b| b.is_ascii_digit()))
        .ok_or("authorization_original_artifact_invalid")?;
    let index = slot
        .parse::<u64>()
        .map_err(|_| "authorization_original_artifact_invalid")?;
    if index == 0 || format!("{index:04}.json") != artifact {
        return Err("authorization_original_artifact_invalid".into());
    }
    let directory = open_agent_artifact_directory(&target_dir.join(AGENT_HTTP_DIRECTORY))
        .ok_or("authorization_original_artifact_missing")?;
    let (bytes, _) = read_agent_artifact_file(&directory, artifact, 1_048_576)
        .ok_or("authorization_original_artifact_missing")?;
    let record: JsonValue =
        serde_json::from_slice(&bytes).map_err(|_| "authorization_original_artifact_invalid")?;
    if record.to_string().as_bytes() != bytes {
        return Err("authorization_original_artifact_invalid".into());
    }
    let (body, body_len) = read_agent_artifact_file(
        &directory,
        &format!("{slot}.body"),
        AGENT_MAX_RESPONSE_BYTES as u64,
    )
    .ok_or("authorization_original_artifact_missing")?;
    let (identity, url) = match side {
        "owner" => (&control.owner_identity, &control.owner_object_url),
        "cross" => (&control.tester_identity, &control.owner_object_url),
        "tester" => (&control.tester_identity, &control.tester_control_url),
        _ => return Err("authorization_original_artifact_invalid".into()),
    };
    let header_names = record
        .pointer("/request/headerNames")
        .and_then(JsonValue::as_array)
        .filter(|a| {
            a.len() <= 128
                && a.iter()
                    .all(|v| v.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256))
        })
        .ok_or("authorization_original_artifact_invalid")?;
    let request = crate::agent_runtime::secrets::redact_json(
        &json!({"identity":identity,"method":"GET","url":url,"headerNames":header_names,"contractKey":control.contract_key,"bodyBytes":null}),
    );
    let response = &record["response"];
    let status = response["status"]
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .ok_or("authorization_original_artifact_invalid")?;
    let content_type = response["contentType"]
        .as_str()
        .ok_or("authorization_original_artifact_invalid")?;
    let redirect = response["redirectCode"]
        .as_str()
        .ok_or("authorization_original_artifact_invalid")?;
    let cache = response["cacheState"]
        .as_str()
        .ok_or("authorization_original_artifact_invalid")?;
    let truncated = response["truncated"]
        .as_bool()
        .ok_or("authorization_original_artifact_invalid")?;
    let expected = json!({"index":index,"request":request,"response":{"identity":identity,"status":status,"contentType":content_type,"redirectCode":redirect,"cacheState":cache,"truncated":truncated,"authorizationSide":side,"rawBodyPrivate":true},"payloadFile":format!("{slot}.body"),"payloadBytes":body_len,"requestPayloadFile":null});
    if record != expected {
        return Err("authorization_original_artifact_scope_conflict".into());
    }
    let text =
        String::from_utf8(body.clone()).map_err(|_| "authorization_original_artifact_invalid")?;
    Ok((
        AuthorizationProbeResponse {
            attempt_number: control.attempt_number,
            contract_key: control.contract_key.clone(),
            identity: identity.clone(),
            url: url.clone(),
            status,
            content_type: content_type.into(),
            redirect_code: redirect.into(),
            cache_state: cache.into(),
            truncated,
            body: text,
        },
        json!({"side":side,"artifactId":artifact,"recordSha256":format!("{:x}",sha2::Sha256::digest(&bytes)),"bodySha256":format!("{:x}",sha2::Sha256::digest(&body)),"recordBytes":bytes.len(),"bodyBytes":body_len}),
    ))
}

// The legacy unsigned contract is distinct. Ordinary birth-bound candidates
// require original signed Authorization facts before SDK, delivery or replay.
fn native_authorization_original_candidate_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    target_dir: &Path,
    candidate: &JsonValue,
) -> Result<Vec<NativeAuthorizationOriginalFinding>, String> {
    if db.is_autocommit() { return Err("authorization_original_snapshot_required".into()); }
    if original_withdrawal_issuer_on(db,actor)?.is_none() { return Ok(Vec::new()); }
    let findings=candidate["findingCandidates"].as_array().ok_or("authorization_original_candidate_invalid")?;
    let mut originals=Vec::new();
    for finding in findings {
        if finding["record"]["kind"] == "authorization_cross_object_read"
            || finding["recordKey"].as_str().is_some_and(|key|key.starts_with("authorization-control:")) {
            originals.push(native_authorization_original_finding_on(db,actor,target_dir,finding)?);
        }
    }
    Ok(originals)
}
