// Private to the actual transport. This freezes first-send authorization only;
// no response receipt, signing authority or executable recovery is created.
struct ProtectionPreSendWitness {
    claim_binding: String,
    canonical_authorization: String,
    authorization_sha256: String,
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    historical: Vec<NativeCoordinatorFrozenRows>,
}

impl ProtectionPreSendWitness {
    fn require_same_claim(
        &self,
        context: &AgentRunContext,
        claim: &AgentHttpClaim,
    ) -> Result<(), String> {
        self.require_binding(&protection_claim_binding(context, claim)?)
    }

    fn require_binding(&self, binding: &str) -> Result<(), String> {
        if self.claim_binding != binding
            || self.authorization_sha256
                != format!(
                    "{:x}",
                    Sha256::digest(self.canonical_authorization.as_bytes())
                )
        {
            return Err("protection_pre_send_binding_changed".into());
        }
        Ok(())
    }

    fn require_original_on(&self, db: &rusqlite::Connection) -> Result<(), String> {
        if db.is_autocommit() {
            return Err("protection_original_snapshot_required".into());
        }
        for row in &self.historical {
            row.verify(db)?;
        }
        Ok(())
    }

    fn canonical_authorization_sha256(&self) -> &str {
        &self.authorization_sha256
    }
}

fn protection_claim_binding(
    context: &AgentRunContext,
    claim: &AgentHttpClaim,
) -> Result<String, String> {
    if context
        .run
        .as_ref()
        .is_none_or(|run| run.run_id != claim.run_id || run.db_path != context.db_path)
    {
        return Err("protection_pre_send_claim_scope_invalid".into());
    }
    Ok(
        json!({"scanId":context.scan_id,"attemptNumber":context.attempt_number,
        "targetUrl":context.target_url,"dbPath":context.db_path,"targetDir":context.target_dir,
        "runId":claim.run_id,"invocationId":claim.invocation_id,
        "requestIndex":claim.request_index,"budgetAttempt":claim.budget_attempt,
        "ordinal":claim.ordinal,"requestHash":claim.request_hash})
        .to_string(),
    )
}

fn protection_pre_send_witness(
    context: &AgentRunContext,
    request: &AgentHttpRequest,
    claim: Option<&AgentHttpClaim>,
) -> Result<Option<ProtectionPreSendWitness>, String> {
    let Some(claim) = claim else {
        return Ok(None);
    };
    let binding = protection_claim_binding(context, claim)?;
    let connection = db::open(&context.db_path)?;
    let tx = connection
        .unchecked_transaction()
        .map_err(|_| "protection_pre_send_snapshot_failed")?;
    let (policy, root): (String, String) = tx.query_row(
        "SELECT orchestration_policy,CASE WHEN root_run_id='' THEN id ELSE root_run_id END
         FROM agent_runs WHERE id=?1 AND scan_id=?2 AND attempt_number=?3 AND target_url=?4 AND backend='native'",
        params![claim.run_id,context.scan_id,context.attempt_number,context.target_url],
        |r| Ok((r.get(0)?,r.get(1)?)),
    ).map_err(|_| "protection_pre_send_run_missing")?;
    // Old Single/Root declarations retain their original transport behavior.
    if policy == "single" {
        return Ok(None);
    }
    // Authenticate a signed original birth before treating a missing protocol
    // as a historical contract. Cold field deletion must not downgrade it.
    let (actor, assignment) =
        crate::agent_runtime::multi_agent::budget::target::child_owner(&tx, &claim.run_id)?
            .ok_or("protection_pre_send_original_owner_missing")?;
    let issuer = original_withdrawal_issuer_on(&tx, &actor)?;
    let declaration = crate::agent_runtime::web_mode::root::read(&tx, &root)?;
    if declaration
        .as_ref()
        .is_none_or(|d| d.protection_supervision().is_none())
    {
        return Ok(None);
    }
    if policy != "multi" {
        return Err("protection_pre_send_ordinary_multi_required".into());
    }
    let exact_claim: bool = tx.query_row(
        "SELECT count(*)=1 FROM agent_http_request_claims c
         JOIN agent_http_budget_origins o ON o.scan_id=c.scan_id AND o.target_url=c.target_url AND o.budget_attempt=c.budget_attempt
         WHERE c.scan_id=?1 AND c.target_url=?2 AND c.attempt_number=?3 AND c.run_id=?4
          AND c.invocation_id=?5 AND c.request_index=?6 AND c.budget_attempt=?7 AND c.ordinal=?8
          AND c.request_hash=?9 AND c.tool_name=?10 AND c.identity_handle=?11
          AND c.response_status=0 AND c.received_at='' AND o.schema_version=1 AND o.baseline_requests>=0",
        params![context.scan_id,context.target_url,context.attempt_number,claim.run_id,
            claim.invocation_id,claim.request_index,claim.budget_attempt,claim.ordinal,
            claim.request_hash,request.tool,request.identity.key],|r|r.get(0),
    ).map_err(|_| "protection_pre_send_claim_unavailable")?;
    if !exact_claim {
        return Err("protection_pre_send_original_claim_changed".into());
    }
    agent_authorize_tool_on(&tx, context, &request.tool).map_err(str::to_string)?;
    if actor.root_run_id != root
        || actor.scan_id != context.scan_id
        || actor.attempt_number != context.attempt_number
        || actor.target_key != context.target_url
    {
        return Err("protection_pre_send_original_owner_conflict".into());
    }
    // Verifies the private whole birth, so adding a protocol to an old Root
    // cannot manufacture first-send authorization for this new contract.
    let issuer = issuer.ok_or("protection_pre_send_private_birth_required")?;
    issuer.require_actor(&tx, &actor)?;
    let worker: String = tx.query_row(
        "SELECT id FROM agent_assignment_attempts WHERE root_run_id=?1 AND assignment_id=?2
         AND child_run_id=?3 AND coordinator_epoch=?4 AND coordinator_fencing_token=?5 AND state='running'",
        params![root,assignment,claim.run_id,actor.lease_epoch,actor.fencing_token],|r|r.get(0),
    ).map_err(|_| "protection_pre_send_worker_missing")?;
    let exact_cap: bool = tx.query_row(
        "SELECT count(*)=1 FROM agent_capability_leases WHERE root_run_id=?1 AND assignment_id=?2
         AND child_run_id=?3 AND capability=?4 AND lease_epoch=?5 AND fencing_token=?6
         AND revoked_at='' AND lease_expires_at>datetime('now','localtime')",
        params![root,assignment,claim.run_id,request.tool,actor.lease_epoch,actor.fencing_token],|r|r.get(0),
    ).map_err(|_| "protection_pre_send_capability_unavailable")?;
    if !exact_cap {
        return Err("protection_pre_send_capability_conflict".into());
    }
    let source = agent_http_budget_source(claim);
    for (kind, suffix) in [("reserve", "reserve"), ("forfeit", "dispatch")] {
        let exact: bool = tx.query_row(
            "SELECT count(*)=1 FROM agent_budget_entries e JOIN agent_assignment_attempts x ON x.id=e.lease_attempt_id
             WHERE e.root_run_id=?1 AND e.assignment_id=?2 AND e.lease_attempt_id=?3 AND e.dimension='target_requests'
              AND e.source_id=?4 AND e.kind=?5 AND e.amount=1 AND x.lease_epoch>0
              AND e.idempotency_key=(CASE WHEN x.lease_epoch=1 THEN '' ELSE 'worker:'||x.id||':' END)||'target:'||?4||':'||?6",
            params![root,assignment,worker,source,kind,suffix],|r|r.get(0),
        ).map_err(|_| "protection_pre_send_target_cost_unavailable")?;
        if !exact {
            return Err("protection_pre_send_target_cost_invalid".into());
        }
    }
    let count: i64 = tx
        .query_row(
            "SELECT count(*) FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2
         AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4",
            params![root, assignment, worker, source],
            |r| r.get(0),
        )
        .map_err(|_| "protection_pre_send_target_cost_unavailable")?;
    if count != 2 {
        return Err("protection_pre_send_target_cost_invalid".into());
    }
    let budget = claim.budget_attempt.to_string();
    let ordinal = claim.ordinal.to_string();
    let mut facts = Vec::new();
    let mut historical = Vec::new();
    for (sql, keys) in [
        ("SELECT c.rowid,c.* FROM agent_http_request_claims c WHERE c.scan_id=?1 AND c.target_url=?2 AND c.budget_attempt=CAST(?3 AS INTEGER) AND c.ordinal=CAST(?4 AS INTEGER) AND c.run_id=?5 AND c.invocation_id=?6 AND c.request_hash=?7 AND c.response_status=0 AND c.received_at=''",vec![context.scan_id.as_str(),context.target_url.as_str(),budget.as_str(),ordinal.as_str(),claim.run_id.as_str(),claim.invocation_id.as_str(),claim.request_hash.as_str()]),
        ("SELECT rowid,* FROM agent_http_budget_origins WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=CAST(?3 AS INTEGER)",vec![context.scan_id.as_str(),context.target_url.as_str(),budget.as_str()]),
        ("SELECT rowid,id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,leased_at FROM agent_assignment_attempts WHERE id=?1",vec![worker.as_str()]),
        ("SELECT rowid,id,coordinator_run_id,role,lane,target_key,dedup_key,trigger_code,task_slice_json,contract_keys_json,identity_handles_json,capability_lease_json,created_at FROM agent_assignments WHERE id=?1",vec![assignment.as_str()]),
        ("SELECT rowid,id,root_run_id,assignment_id,child_run_id,capability,lease_epoch,fencing_token,created_at FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 ORDER BY rowid",vec![assignment.as_str(),claim.run_id.as_str()]),
        ("SELECT rowid,id,scan_id,attempt_number,target_url,backend,role,parent_run_id,root_run_id,assignment_id,lane,orchestration_policy,plan_hash,plan_json,created_at FROM agent_runs WHERE id=?1",vec![claim.run_id.as_str()]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4 ORDER BY rowid",vec![root.as_str(),assignment.as_str(),worker.as_str(),source.as_str()]),
        ("SELECT rowid,root_run_id,binding_json,signature FROM agent_root_mode_birth_seals WHERE root_run_id=?1",vec![root.as_str()]),
        ("SELECT id,run_id,tool_name,tool_version,contract_key,identity_handle,input_hash,policy_decision,started_at FROM tool_invocations WHERE run_id=?1 AND invocation_id=?2 AND policy_decision='allow'",vec![claim.run_id.as_str(),claim.invocation_id.as_str()]),
    ] {
        let row = NativeCoordinatorFrozenRows::capture(&tx,sql,&keys)?;
        if !matches!(facts.len(), 0 | 6) { historical.push(row.clone()); }
        facts.push(row.fact());
    }
    historical.push(NativeCoordinatorFrozenRows::capture(&tx,
        "SELECT rowid,scan_id,target_url,budget_attempt,ordinal,attempt_number,run_id,invocation_id,request_index,tool_name,identity_handle,request_hash,created_at FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=CAST(?3 AS INTEGER) AND ordinal=CAST(?4 AS INTEGER)",
        &[context.scan_id.as_str(),context.target_url.as_str(),budget.as_str(),ordinal.as_str()])?);
    // Headers add reconciliation, but cannot replace the original two
    // pre-send physical charges with semantically similar rows.
    historical.push(NativeCoordinatorFrozenRows::capture(&tx,
        "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND dimension='target_requests' AND source_id=?4 AND kind IN ('reserve','forfeit') ORDER BY rowid",
        &[root.as_str(),assignment.as_str(),worker.as_str(),source.as_str()])?);
    let canonical = json!({"schemaVersion":1,"provenance":"original_send_authorization_unsealed",
        "claim":serde_json::from_str::<JsonValue>(&binding).map_err(|_|"protection_pre_send_binding_invalid")?,
        "rootRunId":root,"assignmentId":assignment,"workerAttemptId":worker,
        "coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,
        "scanId":context.scan_id,"attemptNumber":context.attempt_number,"originalFacts":facts,
        "historicalFacts":historical.iter().map(NativeCoordinatorFrozenRows::fact).collect::<Vec<_>>()}).to_string();
    let witness = ProtectionPreSendWitness {
        claim_binding: binding,
        authorization_sha256: format!("{:x}", Sha256::digest(canonical.as_bytes())),
        canonical_authorization: canonical,
        actor,
        historical,
    };
    witness.require_same_claim(context, claim)?;
    Ok(Some(witness))
}

