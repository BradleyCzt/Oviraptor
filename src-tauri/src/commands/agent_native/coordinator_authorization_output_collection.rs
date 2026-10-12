// Pure original Authorization collection. This is not an issuer, live grant,
// output mailbox producer, or model-confirmed vulnerability verdict.
pub(crate) struct VerifiedAuthorizationOutputCollection {
    fact: JsonValue,
}
impl VerifiedAuthorizationOutputCollection {
    pub(crate) fn fact(&self) -> &JsonValue {
        &self.fact
    }
}

pub(crate) fn native_authorization_output_collection_on(
    db: &rusqlite::Connection,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    target_dir: &Path,
) -> Result<VerifiedAuthorizationOutputCollection, String> {
    use crate::agent_runtime::{
        contract::AgentRole,
        multi_agent::{attempts, budget, contract_owner},
    };
    use std::collections::BTreeSet;
    if db.is_autocommit() {
        return Err("authorization_output_collection_snapshot_required".into());
    }
    if child.role != AgentRole::Authorization {
        return Err("authorization_output_collection_role_invalid".into());
    }
    budget::root::RootOwner::load_original(db, &actor.root_run_id)?
        .require_original_coordinator(db, actor)?;
    let original = original_root_capture_mode(db, &actor.root_run_id)?;
    let directory = checked_evidence_directory(
        Path::new(&original.fact().work_dir),
        target_dir,
        &actor.scan_id,
    )?;
    if directory != target_dir
        || resolve_agent_evidence_location(db, &actor.root_run_id)? != directory
    {
        return Err("authorization_output_collection_directory_conflict".into());
    }
    let issuer = crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer::from_verified_birth_on(
        db, &actor.root_run_id, original.clone(),
    )?;
    issuer.require_actor(db, actor)?;
    let worker = attempts::current(db, actor, &child.assignment_id)?;
    let (task,revision,state):(String,i64,String)=db.query_row(r#"SELECT a.task_slice_json,a.evidence_revision,a.state FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.target_key=?4 AND a.role='authorization' AND a.lane='target_touching' AND a.trigger_code='operator_authorization_control_ready' AND a.lease_epoch=?5 AND a.fencing_token=?6 AND a.capability_lease_json='["authorization_probe"]' AND a.contract_keys_json='[]' AND a.identity_handles_json='[]' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.failure_class='' AND r.backend='native' AND r.orchestration_policy='multi' AND r.role=a.role AND r.lane=a.lane AND r.assignment_id=a.id AND r.root_run_id=?3 AND r.parent_run_id=?3 AND r.scan_id=?7 AND r.attempt_number=?8 AND r.target_url=?4 AND r.used_tokens=0 AND r.used_cached_tokens=0 AND r.used_requests=0 AND r.cancel_requested_at='' AND ((a.state='running' AND a.finished_at='' AND r.status='running' AND r.terminal_state='' AND r.finished_at='') OR (a.state='completed' AND a.finished_at<>'' AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>''))"#,params![child.assignment_id,child.run_id,actor.root_run_id,actor.target_key,actor.lease_epoch,actor.fencing_token,actor.scan_id,actor.attempt_number],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"authorization_output_collection_original_child_conflict")?;
    let completed = state == "completed";
    if revision != 1
        || worker.child_run_id != child.run_id
        || worker.lease_epoch != 1
        || (completed
            && (worker.state != "completed"
                || worker.finished_at.is_empty()
                || !worker.failure_class.is_empty()))
        || (!completed && (worker.state != "running" || !worker.finished_at.is_empty()))
    {
        return Err("authorization_output_collection_original_worker_conflict".into());
    }
    let contract_key = contract_owner::schedule_contract_key(
        actor.attempt_number,
        &actor.target_key,
        "authorization",
        "operator_authorization_control_ready",
        revision,
    );
    let owned:bool=db.query_row("SELECT count(*)=1 FROM agent_contract_owners WHERE assignment_id=?1 AND root_run_id=?2 AND contract_key=?3 AND lease_epoch=?4 AND fencing_token=?5",params![child.assignment_id,actor.root_run_id,contract_key,actor.lease_epoch,actor.fencing_token],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !owned {
        return Err("authorization_output_collection_original_contract_owner_changed".into());
    }
    let controls = authorization_original_controls(db, actor)?;
    let control_keys: Vec<_> = controls.iter().map(|c| c.contract_key.as_str()).collect();
    let unique_keys: BTreeSet<_> = control_keys.iter().copied().collect();
    let expected_task = crate::agent_runtime::secrets::redact_json(
        &json!({"contractKeys":control_keys,"objective":"仅执行已登记的 A→X、B→X、B→Y GET 控制组"}),
    )
    .to_string();
    if task != expected_task || unique_keys.len() != controls.len() {
        return Err("authorization_output_collection_original_task_changed".into());
    }
    let capability_count:i64=db.query_row("SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2",params![child.assignment_id,child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let cap:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND capability='authorization_probe' AND lease_epoch=?4 AND fencing_token=?5 AND ((?6 AND revoked_at<>'') OR (NOT ?6 AND revoked_at='')))",params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token,completed],|r|r.get(0)).map_err(|e|e.to_string())?;
    let lane_count: i64 = db
        .query_row(
            "SELECT count(*) FROM agent_lane_leases WHERE assignment_id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let lane:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_lane_leases WHERE assignment_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_key=?4 AND lane='target_touching')",params![child.assignment_id,actor.scan_id,actor.attempt_number,actor.target_key],|r|r.get(0)).map_err(|e|e.to_string())?;
    if capability_count != 1
        || !cap
        || (completed && lane_count != 0)
        || (!completed && (lane_count != 1 || !lane))
    {
        return Err("authorization_output_collection_original_resources_changed".into());
    }
    let expected_count = i64::try_from(controls.len())
        .map_err(|_| "budget_amount_overflow")?
        .checked_mul(3)
        .ok_or("budget_amount_overflow")?;
    let all_claims:i64=db.query_row("SELECT count(*) FROM agent_authorization_probe_claims WHERE child_run_id=?1 OR assignment_id=?2",params![child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let scoped_claims:i64=db.query_row("SELECT count(*) FROM agent_authorization_probe_claims WHERE child_run_id=?1 AND assignment_id=?2 AND scan_id=?3 AND attempt_number=?4 AND target_url=?5 AND artifact_id<>'' AND contract_key IN(SELECT value FROM json_each(?6,'$.contractKeys'))",params![child.run_id,child.assignment_id,actor.scan_id,actor.attempt_number,actor.target_key,task],|r|r.get(0)).map_err(|e|e.to_string())?;
    let all_seals:i64=db.query_row("SELECT count(*) FROM main.agent_authorization_side_receipts WHERE child_run_id=?1 OR assignment_id=?2",params![child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if all_claims != expected_count
        || scoped_claims != expected_count
        || all_seals != expected_count
    {
        return Err("authorization_output_collection_original_inventory_changed".into());
    }
    // Authorization never made a child SDK call; do not invent a zero invoice
    // if a foreign or still-running execution record actually exists.
    let foreign_execution:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_web_model_journal WHERE child_run_id=?1 OR assignment_id=?2) OR EXISTS(SELECT 1 FROM agent_specialist_calls WHERE child_run_id=?1 OR assignment_id=?2) OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1) OR EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1)",params![child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if foreign_execution {
        return Err("authorization_output_collection_unexpected_execution".into());
    }
    let foreign_budget: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM agent_budget_entries WHERE (assignment_id=?1 OR lease_attempt_id=?2) AND (root_run_id<>?3 OR assignment_id<>?1 OR lease_attempt_id<>?2))", params![child.assignment_id,worker.id,actor.root_run_id], |r|r.get(0)).map_err(|e|e.to_string())?;
    if foreign_budget {
        return Err("authorization_output_collection_foreign_budget_namespace".into());
    }
    let clock = budget::clock::original_worker_samples::original_worker_consumed_on(
        db, actor, child, &worker.id,
    )?;
    for (index, dimension) in budget::DIMENSIONS.iter().enumerate() {
        let b = budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            &worker.id,
            dimension,
        )?;
        if index == 8 {
            if b.indeterminate != 0
                || b.consumed != 0
                || (completed && b.reserved != 0)
                || (!completed && !matches!(b.reserved, 0 | 1))
            {
                return Err("authorization_output_collection_original_slot_changed".into());
            }
            continue;
        }
        let expected = match index {
            4 => expected_count,
            9 => clock,
            _ => 0,
        };
        if b.reserved != 0 || b.indeterminate != 0 || b.consumed != expected {
            return Err(format!(
                "authorization_output_collection_original_budget_changed:{dimension}"
            ));
        }
    }
    let mut groups = Vec::new();
    let mut artifacts = BTreeSet::new();
    let mut receipts = BTreeSet::new();
    let mut finding_ids = BTreeSet::new();
    let mut frozen_rows = Vec::new();
    let mut verified = 0_i64;
    for control in &controls {
        let key = &control.contract_key;
        let claims:Vec<(String,String)>=db.prepare("SELECT side,artifact_id FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND contract_key=?4 AND child_run_id=?5 AND assignment_id=?6 ORDER BY side").map_err(|e|e.to_string())?.query_map(params![actor.scan_id,actor.attempt_number,actor.target_key,key,child.run_id,child.assignment_id],|r|Ok((r.get(0)?,r.get(1)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
        if claims.len() != 3
            || claims.iter().map(|c| c.0.as_str()).collect::<BTreeSet<_>>()
                != BTreeSet::from(["owner", "cross", "tester"])
        {
            return Err("authorization_output_collection_original_side_set_changed".into());
        }
        let mut responses = Vec::new();
        let mut sides = Vec::new();
        let mut hashes = Vec::new();
        for side in ["owner", "cross", "tester"] {
            let artifact = &claims
                .iter()
                .find(|c| c.0 == side)
                .ok_or("authorization_output_collection_original_side_missing")?
                .1;
            if !artifacts.insert(artifact.clone()) {
                return Err("authorization_output_collection_duplicate_artifact".into());
            }
            let source = format!(
                "authorization:{}:{}:{side}",
                child.assignment_id,
                crate::agent_runtime::store::stable_hash(key)
            );
            if budget::target::original_target_receipt(db, actor, &child.run_id, &source)?
                != (child.assignment_id.clone(), worker.id.clone())
            {
                return Err("authorization_output_collection_original_cost_scope_changed".into());
            }
            let (response, hash) =
                authorization_original_artifact(target_dir, control, side, artifact)?;
            let receipt:String=db.query_row("SELECT receipt_id FROM main.agent_authorization_side_receipts WHERE root_run_id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND contract_key=?5 AND side=?6 AND child_run_id=?7 AND assignment_id=?8 AND worker_attempt_id=?9 AND artifact_id=?10",params![actor.root_run_id,actor.scan_id,actor.attempt_number,actor.target_key,key,side,child.run_id,child.assignment_id,worker.id,artifact],|r|r.get(0)).map_err(|_|"authorization_output_collection_original_side_seal_missing")?;
            if !receipts.insert(receipt.clone()) {
                return Err("authorization_output_collection_duplicate_receipt".into());
            }
            let raw = issuer.verify_authorization_side_issued(db, &receipt)?;
            let fact: JsonValue = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            if fact["coordinatorEpoch"] != actor.lease_epoch
                || fact["coordinatorFence"] != actor.fencing_token
                || fact["rootRunId"] != actor.root_run_id
                || fact["scanId"] != actor.scan_id
                || fact["attemptNumber"] != actor.attempt_number
                || fact["targetUrl"] != actor.target_key
                || fact["assignmentId"] != child.assignment_id
                || fact["childRunId"] != child.run_id
                || fact["workerAttemptId"] != worker.id
                || fact["contractKey"] != *key
                || fact["side"] != side
                || fact["artifactId"] != *artifact
                || fact["recordSha256"] != hash["recordSha256"]
                || fact["bodySha256"] != hash["bodySha256"]
                || fact["recordBytes"] != hash["recordBytes"]
                || fact["bodyBytes"] != hash["bodyBytes"]
            {
                return Err(
                    "authorization_output_collection_original_signed_source_changed".into(),
                );
            }
            let rows = fact["originalRows"]
                .as_array()
                .filter(|r| r.len() == 6)
                .ok_or("authorization_output_collection_original_side_rows_invalid")?;
            for row in rows {
                let sql = row["sql"]
                    .as_str()
                    .filter(|s| s.starts_with("SELECT ") && !s.contains(';'))
                    .ok_or("authorization_output_collection_original_side_rows_invalid")?;
                let frozen = NativeCoordinatorFrozenRows {
                    sql: sql.into(),
                    keys: serde_json::from_value(row["keys"].clone()).map_err(|e| e.to_string())?,
                    hash: row["hash"]
                        .as_str()
                        .ok_or("authorization_output_collection_original_side_rows_invalid")?
                        .into(),
                    allow_empty: false,
                };
                frozen.verify(db)?;
                frozen_rows.push(frozen);
            }
            frozen_rows.push(NativeCoordinatorFrozenRows::capture(
                db,
                "SELECT rowid,* FROM main.agent_authorization_side_receipts WHERE receipt_id=?1",
                &[&receipt],
            )?);
            sides.push(json!({"name":side,"artifactId":artifact,"receiptId":receipt}));
            hashes.push(hash);
            responses.push(response);
        }
        let verdict = verify_authorization_control_group(
            control,
            [&responses[0], &responses[1], &responses[2]],
        );
        let confirmed = verdict.is_ok();
        if confirmed {
            verified = verified.checked_add(1).ok_or("budget_amount_overflow")?;
        }
        let record_sides: Vec<_> = sides
            .iter()
            .map(|s| json!({"name":s["name"],"artifactId":s["artifactId"]}))
            .collect();
        let record = json!({"kind":"authorization_cross_object_read","contractKey":key,"verification":"operator_bound_three_side_native_control","responseObjectPointer":control.response_object_pointer,"method":"GET","sides":record_sides,"attemptNumber":actor.attempt_number});
        let finding_key = format!(
            "authorization-control:{}",
            &crate::agent_runtime::store::stable_hash(&format!(
                "{}:{}:{key}",
                actor.scan_id, actor.attempt_number
            ))[..24]
        );
        let findings:Vec<(String,String,String,String,String,String)>=db.prepare("SELECT id,stage,kind,title,severity,record_json FROM agent_finding_candidates WHERE root_run_id=?1 AND scan_id=?2 AND target_url=?3 AND (record_key=?4 OR (json_extract(record_json,'$.kind')='authorization_cross_object_read' AND json_extract(record_json,'$.contractKey')=?5)) ORDER BY id").map_err(|e|e.to_string())?.query_map(params![actor.root_run_id,actor.scan_id,actor.target_key,finding_key,key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
        let canonical_record = record.to_string();
        if (confirmed
            && (findings.len() != 1
                || findings[0].1 != AGENT_VULNERABILITY_STAGE
                || findings[0].2 != "vulnerability"
                || findings[0].3 != "跨账号访问已登记的他人对象"
                || findings[0].4 != "high"
                || findings[0].5 != canonical_record))
            || (!confirmed && !findings.is_empty())
        {
            return Err("authorization_output_collection_original_finding_changed".into());
        }
        let finding_id = if confirmed {
            let id = &findings[0].0;
            if !finding_ids.insert(id.clone()) {
                return Err("authorization_output_collection_duplicate_finding".into());
            }
            frozen_rows.push(NativeCoordinatorFrozenRows::capture(db,"SELECT rowid,id,root_run_id,scan_id,target_url,stage,kind,record_key,title,severity,record_json,created_at FROM agent_finding_candidates WHERE id=?1 AND record_key=?2",&[id,&finding_key])?);
            Some(id.clone())
        } else {
            None
        };
        groups.push(json!({"contractKey":key,"verified":i64::from(confirmed),"reason":verdict.err(),"sideReceipts":sides,"artifacts":hashes,"findingId":finding_id}));
    }
    let attempt = actor.attempt_number.to_string();
    // Exclude legitimate running->completed and slot release mutations. These
    // stable physical sources must remain identical across the producer's TX.
    for (sql,keys) in [
        ("SELECT rowid,* FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=CAST(?2 AS INTEGER) AND target_url=?3 ORDER BY contract_key",vec![actor.scan_id.as_str(),attempt.as_str(),actor.target_key.as_str()]),
        ("SELECT rowid,* FROM agent_authorization_probe_claims WHERE child_run_id=?1 OR assignment_id=?2 ORDER BY rowid",vec![child.run_id.as_str(),child.assignment_id.as_str()]),
        ("SELECT rowid,* FROM agent_evidence_locations WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE lease_attempt_id=?1 AND dimension<>'concurrency_batches' ORDER BY rowid",vec![worker.id.as_str()]),
        ("SELECT rowid,id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id FROM agent_assignment_attempts WHERE id=?1",vec![worker.id.as_str()]),
        ("SELECT rowid,id,child_run_id,coordinator_run_id,role,lane,target_key,dedup_key,trigger_code,task_slice_json,evidence_revision,capability_lease_json,contract_keys_json,identity_handles_json,lease_epoch,fencing_token,budget_settled_at FROM agent_assignments WHERE id=?1",vec![child.assignment_id.as_str()]),
        ("SELECT rowid,root_run_id,contract_key,assignment_id,lease_epoch,fencing_token,acquired_at FROM agent_contract_owners WHERE assignment_id=?1",vec![child.assignment_id.as_str()]),
    ] {frozen_rows.push(NativeCoordinatorFrozenRows::capture(db,sql,&keys)?);}
    let physical: Vec<_> = frozen_rows
        .iter()
        .map(|r| json!({"sql":r.sql,"keys":r.keys,"hash":r.hash}))
        .collect();
    Ok(VerifiedAuthorizationOutputCollection {
        fact: json!({"schemaVersion":1,"purpose":"original_complete_authorization_output_collection","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":worker.id,"contractKeys":control_keys,"controlCount":controls.len(),"requestCount":expected_count,"verified":verified,"groups":groups,"wallTimeMs":clock,"originalPhysicalFacts":physical,"executionAllowed":false,"recoveryImplemented":false}),
    })
}
