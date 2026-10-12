// Execute only these fixed SELECTs and locally derived keys. A signed query
// string is compared with the contract; it is never itself executed.
fn native_protection_original_rows(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    fact: &JsonValue,
) -> Result<(Vec<NativeCoordinatorFrozenRows>, JsonValue), String> {
    let pre = &fact["preSendAuthorization"];
    native_protection_exact_keys(
        pre,
        &[
            "schemaVersion",
            "provenance",
            "claim",
            "rootRunId",
            "assignmentId",
            "workerAttemptId",
            "coordinatorEpoch",
            "coordinatorFence",
            "scanId",
            "attemptNumber",
            "originalFacts",
            "historicalFacts",
        ],
    )?;
    let claim = &pre["claim"];
    native_protection_exact_keys(
        claim,
        &[
            "scanId",
            "attemptNumber",
            "targetUrl",
            "dbPath",
            "targetDir",
            "runId",
            "invocationId",
            "requestIndex",
            "budgetAttempt",
            "ordinal",
            "requestHash",
        ],
    )?;
    if pre["schemaVersion"] != 1
        || pre["provenance"] != "original_send_authorization_unsealed"
        || pre["rootRunId"] != actor.root_run_id
        || pre["scanId"] != actor.scan_id
        || pre["attemptNumber"] != actor.attempt_number
        || pre["coordinatorEpoch"] != actor.lease_epoch
        || pre["coordinatorFence"] != actor.fencing_token
        || pre["assignmentId"] != fact["assignmentId"]
        || pre["workerAttemptId"] != fact["workerAttemptId"]
        || claim["runId"] != fact["runId"]
        || claim["scanId"] != actor.scan_id
        || claim["attemptNumber"] != actor.attempt_number
        || claim["targetUrl"] != actor.target_key
    {
        return Err("protection_original_pre_send_scope_changed".into());
    }
    let run = native_protection_text(fact, "runId")?;
    let assignment = native_protection_text(fact, "assignmentId")?;
    let worker = native_protection_text(fact, "workerAttemptId")?;
    let invocation = native_protection_text(claim, "invocationId")?;
    let hash = native_protection_text(claim, "requestHash")?;
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("protection_original_claim_invalid".into());
    }
    let index = native_protection_positive(claim, "requestIndex")?;
    let budget = native_protection_positive(claim, "budgetAttempt")?.to_string();
    let ordinal = native_protection_positive(claim, "ordinal")?.to_string();
    let source = format!("http:{run}:{invocation}:{index}:{hash}");
    let root = actor.root_run_id.as_str();
    let scan = actor.scan_id.as_str();
    let target = actor.target_key.as_str();
    let specs: Vec<(&str,Vec<&str>)> = vec![
        ("SELECT rowid,* FROM agent_http_budget_origins WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=CAST(?3 AS INTEGER)",vec![scan,target,&budget]),
        ("SELECT rowid,id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,leased_at FROM agent_assignment_attempts WHERE id=?1",vec![worker]),
        ("SELECT rowid,id,coordinator_run_id,role,lane,target_key,dedup_key,trigger_code,task_slice_json,contract_keys_json,identity_handles_json,capability_lease_json,created_at FROM agent_assignments WHERE id=?1",vec![assignment]),
        ("SELECT rowid,id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,created_at FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 ORDER BY rowid",vec![assignment,run]),
        ("SELECT rowid,id,scan_id,attempt_number,target_url,backend,role,parent_run_id,root_run_id,assignment_id,lane,orchestration_policy,plan_hash,plan_json,created_at FROM agent_runs WHERE id=?1",vec![run]),
        ("SELECT rowid,root_run_id,binding_json,signature FROM agent_root_mode_birth_seals WHERE root_run_id=?1",vec![root]),
        ("SELECT id,run_id,tool_name,tool_version,contract_key,identity_handle,input_hash,policy_decision,started_at FROM tool_invocations WHERE run_id=?1 AND invocation_id=?2 AND policy_decision='allow'",vec![run,invocation]),
        ("SELECT rowid,scan_id,target_url,budget_attempt,ordinal,attempt_number,run_id,invocation_id,request_index,tool_name,identity_handle,request_hash,created_at FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=CAST(?3 AS INTEGER) AND ordinal=CAST(?4 AS INTEGER)",vec![scan,target,&budget,&ordinal]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4 AND kind IN ('reserve','forfeit') ORDER BY rowid",vec![root,assignment,worker,&source]),
        ("SELECT rowid,* FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=CAST(?3 AS INTEGER) AND ordinal=CAST(?4 AS INTEGER)",vec![scan,target,&budget,&ordinal]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4 ORDER BY rowid",vec![root,assignment,worker,&source]),
        ("SELECT rowid,* FROM agent_protection_stops WHERE scan_id=?1 AND target_url=?2",vec![scan,target]),
    ];
    let originals = fact["originalFacts"]
        .as_array()
        .filter(|a| a.len() == 12)
        .ok_or("protection_original_rows_shape_invalid")?;
    if pre["historicalFacts"] != json!(&originals[..9]) {
        return Err("protection_original_historical_rows_changed".into());
    }
    let mut rows = Vec::new();
    for ((sql, keys), signed) in specs.iter().zip(originals) {
        native_protection_exact_keys(signed, &["query", "keys", "physicalHash"])?;
        if signed["query"] != *sql || signed["keys"] != json!(keys) {
            return Err("protection_original_query_contract_invalid".into());
        }
        let row = NativeCoordinatorFrozenRows::capture(db, sql, keys)?;
        if row.fact() != *signed {
            return Err("protection_original_physical_fact_changed".into());
        }
        rows.push(row);
    }
    // The old pending/full-cost snapshots are signed provenance, not current
    // state queries. Check their fixed shape/keys; never replay pending SQL.
    let pending = pre["originalFacts"]
        .as_array()
        .filter(|a| a.len() == 9)
        .ok_or("protection_original_pre_send_rows_invalid")?;
    for (i, signed) in pending.iter().enumerate() {
        native_protection_exact_keys(signed, &["query", "keys", "physicalHash"])?;
        let expected=match i { 0 => (
            "SELECT c.rowid,c.* FROM agent_http_request_claims c WHERE c.scan_id=?1 AND c.target_url=?2 AND c.budget_attempt=CAST(?3 AS INTEGER) AND c.ordinal=CAST(?4 AS INTEGER) AND c.run_id=?5 AND c.invocation_id=?6 AND c.request_hash=?7 AND c.response_status=0 AND c.received_at=''",
            vec![scan,target,&budget,&ordinal,run,invocation,hash]),
            6 => specs[10].clone(),
            n => specs[if n<6 {n-1} else {n-2}].clone(),
        };
        if signed["query"] != expected.0
            || signed["keys"] != json!(expected.1)
            || native_protection_text(signed, "physicalHash")?.len() != 64
            || (i != 0
                && signed["physicalHash"]
                    != originals[if i == 6 {
                        8
                    } else if i < 6 {
                        i - 1
                    } else {
                        i - 2
                    }]["physicalHash"])
        {
            return Err("protection_original_pre_send_rows_changed".into());
        }
    }
    let exact: bool=db.query_row(
        "SELECT count(*)=1 FROM agent_http_request_claims c JOIN agent_runs r ON r.id=c.run_id
         JOIN agent_assignments a ON a.id=r.assignment_id AND a.coordinator_run_id=r.root_run_id
         JOIN agent_assignment_attempts x ON x.id=?11 AND x.child_run_id=r.id AND x.assignment_id=a.id AND x.root_run_id=r.root_run_id
         JOIN agent_http_budget_origins o ON o.scan_id=c.scan_id AND o.target_url=c.target_url AND o.budget_attempt=c.budget_attempt
         WHERE c.rowid=?1 AND c.scan_id=?2 AND c.target_url=?3 AND c.attempt_number=?4 AND c.run_id=?5
          AND c.invocation_id=?6 AND c.request_index=?7 AND c.budget_attempt=?8 AND c.ordinal=?9 AND c.request_hash=?10
          AND c.response_status=?12 AND c.received_at<>'' AND r.root_run_id=?13 AND r.parent_run_id=?13
          AND r.scan_id=c.scan_id AND r.attempt_number=c.attempt_number AND r.target_url=c.target_url
          AND r.assignment_id=?14 AND r.role='web_executor' AND r.lane='target_touching' AND r.backend='native'
          AND r.orchestration_policy='multi' AND a.role=r.role AND a.lane=r.lane AND a.target_key=r.target_url
          AND x.coordinator_epoch=?15 AND x.coordinator_fencing_token=?16 AND o.schema_version=1 AND o.baseline_requests>=0
          AND EXISTS(SELECT 1 FROM tool_invocations i WHERE i.run_id=c.run_id AND i.invocation_id=c.invocation_id
            AND i.tool_name=c.tool_name AND i.policy_decision='allow')
          AND (SELECT count(*) FROM agent_capability_leases p WHERE p.root_run_id=r.root_run_id AND p.assignment_id=a.id
            AND p.child_run_id=r.id AND p.capability=c.tool_name AND p.lease_epoch=?15 AND p.fencing_token=?16)=1",
        params![native_protection_positive(fact,"claimRowid")?,scan,target,actor.attempt_number,run,invocation,index,
            native_protection_positive(claim,"budgetAttempt")?,native_protection_positive(claim,"ordinal")?,hash,worker,
            native_protection_positive(fact,"status")?,root,assignment,actor.lease_epoch,actor.fencing_token],|r|r.get(0),
    ).map_err(|_|"protection_original_claim_unavailable")?;
    if !exact {
        return Err("protection_original_claim_scope_invalid".into());
    }
    if crate::agent_runtime::multi_agent::budget::target::original_target_receipt(
        db, actor, run, &source,
    )? != (assignment.to_string(), worker.to_string())
    {
        return Err("protection_original_target_cost_changed".into());
    }
    let stop: bool=db.query_row(
        "SELECT count(*)=1 FROM agent_protection_stops s JOIN agent_http_request_claims c ON c.rowid=?1
         WHERE s.scan_id=?2 AND s.target_url=?3 AND s.source_attempt_number=?4 AND s.source_run_id=?5
          AND s.stop_code='confirmed_waf_or_challenge' AND json_extract(s.claim_json,'$.rowid')=c.rowid
          AND json_extract(s.claim_json,'$.requestHash')=c.request_hash AND json_extract(s.claim_json,'$.responseStatus')=c.response_status",
        params![native_protection_positive(fact,"claimRowid")?,scan,target,actor.attempt_number,run],|r|r.get(0),
    ).map_err(|_|"protection_original_stop_unavailable")?;
    if !stop {
        return Err("protection_original_stop_changed".into());
    }
    Ok((rows, claim.clone()))
}
