use super::*;

/// The run context carries the scan's validated captured accounts plus, when
/// any account exists, the single fixed credential-free control that
/// `multi_agent_prepare` / `agent_backend` append (`AgentIdentity::anonymous`).
/// That control is not a captured account, never changes the identity policy
/// and can never be an Authorization side. Everything else stays exact: the
/// captured identities must equal the scan's validated set, the side identity
/// must be one of them, and any other "anonymous"-looking or extra identity is
/// denied.
pub(super) fn authorization_identity_binding(
    bound: &[AgentIdentity],
    validated: Vec<String>,
    identity: &str,
) -> Result<(), String> {
    let denied = || "authorization_identity_binding_denied".to_string();
    let mut controls = 0usize;
    let mut captured = Vec::with_capacity(bound.len());
    for item in bound {
        if item.anonymous || item.key == "anonymous" {
            if !(item.anonymous && item.key == "anonymous" && item.session_id.is_none()) {
                return Err(denied());
            }
            controls += 1;
        } else if item.session_id.as_deref() != Some(item.key.as_str()) {
            return Err(denied());
        } else {
            captured.push(item.key.clone());
        }
    }
    let mut validated = validated;
    validated.sort();
    captured.sort();
    if controls > 1
        || (controls == 1 && validated.is_empty())
        || captured != validated
        || !validated.iter().any(|known| known == identity)
    {
        return Err(denied());
    }
    Ok(())
}

// Construction remains inside the actual broker. Existing claims cannot be
// read back into this token after a crash or an attempted side replay.
struct ClaimedAuthorizationSide {
    rowid: i64,
    created_at: String,
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    worker: String,
    originals: Vec<NativeCoordinatorFrozenRows>,
}

// Only the actual fresh claim + completed original writer can construct this
// envelope, inside the final response transaction. No reader/backfill API.
pub(crate) struct FreshAuthorizationSideReceipt {
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    raw: String,
    connection: *const rusqlite::Connection,
    originals: Vec<NativeCoordinatorFrozenRows>,
}
impl FreshAuthorizationSideReceipt {
    pub(crate) fn require_on<'a>(&'a self, db: &rusqlite::Connection)
        -> Result<(&'a crate::agent_runtime::multi_agent::lease::CoordinatorLease, &'a str), String> {
        if db.is_autocommit() || !std::ptr::eq(self.connection, db) {
            return Err("authorization_fresh_receipt_writer_required".into());
        }
        for row in &self.originals { row.verify(db)?; }
        Ok((&self.actor, &self.raw))
    }
}

#[cfg(test)]
pub(super) fn claim_authorization_probe(
    context: &AgentRunContext,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control: &SaveAuthorizationControlInput,
    side: &str, identity: &str, url: &str,
) -> Result<(), String> {
    claim_authorization_probe_fresh(context, child, control, side, identity, url).map(|_| ())
}

/// Atomic broker decision and durable, non-replayable side reservation. The
/// model cannot invoke this function, and a request is never sent before its
/// exact method, URL, identity, contract, lease, lane and budget are checked.
fn claim_authorization_probe_fresh(
    context: &AgentRunContext,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control: &SaveAuthorizationControlInput,
    side: &str,
    identity: &str,
    url: &str,
) -> Result<ClaimedAuthorizationSide, String> {
    let expected = match side {
        "owner" => (&control.owner_identity, &control.owner_object_url),
        "cross" => (&control.tester_identity, &control.owner_object_url),
        "tester" => (&control.tester_identity, &control.tester_control_url),
        _ => return Err("authorization_side_invalid".into()),
    };
    if (identity, url) != (expected.0.as_str(), expected.1.as_str())
        || control.scan_id != context.scan_id || control.target_url != context.target_url
        || control.attempt_number != context.attempt_number
        || child.role != crate::agent_runtime::contract::AgentRole::Authorization
        || context.run.as_ref().map(|run| run.run_id.as_str()) != Some(child.run_id.as_str())
    {
        return Err("authorization_side_binding_invalid".into());
    }
    let mut connection = db::open(&context.db_path)?;
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "authorization_claim_transaction_failed".to_string())?;
    // A dedicated side probe is still a target-touching Web request. Validate
    // the same authoritative attempt plan as the generic tool broker before
    // reserving a side or charging any of its three request slots.
    agent_require_frozen_web_plan(&tx, context).map_err(str::to_string)?;
    let (_, identities) = crate::auth_session::validated_scan_identities(
        &tx, &context.scan_id, &context.target_url,
    ).map_err(|_| "authorization_identity_binding_denied".to_string())?;
    authorization_identity_binding(&context.identities, identities, identity)?;
    let authorized=authorization_probe_authority(&tx,context,child,control,side,identity,url,false)?;
    let Some(budget) = authorized else { return Err("authorization_capability_or_fencing_denied".into()) };
    let spent: i64 = tx.query_row(
        "SELECT COUNT(*) FROM agent_authorization_probe_claims WHERE child_run_id=?1",
        [&child.run_id], |row| row.get(0),
    ).map_err(|_| "authorization_budget_unavailable".to_string())?;
    if spent >= budget || budget <= 0 {
        return Err("authorization_request_budget_exhausted".into());
    }
    let usage = agent_request_accounting(&tx,&context.scan_id,context.attempt_number,&context.target_url)?;
    let ceiling = context.execution_plan.hard_model_requests.max(1).saturating_mul(4).min(400);
    if usage.budget_committed >= ceiling {
        return Err("authorization_request_budget_exhausted".into());
    }
    // A damaged/deleted mutable claim is never permission to send again.
    // Original financial dispatch and first-response seals survive separately.
    let previously_dispatched:bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM main.agent_authorization_side_receipts WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND contract_key=?4 AND side=?5)
         OR EXISTS(SELECT 1 FROM agent_budget_entries e JOIN agent_runs r ON r.assignment_id=e.assignment_id AND r.root_run_id=e.root_run_id
            WHERE r.scan_id=?1 AND r.attempt_number=?2 AND r.target_url=?3 AND r.role='authorization' AND e.dimension='target_requests'
              AND e.source_id='authorization:'||e.assignment_id||':'||?6||':'||?5)",
        params![context.scan_id,context.attempt_number,context.target_url,control.contract_key,side,
            crate::agent_runtime::store::stable_hash(&control.contract_key)],|r|r.get(0),
    ).map_err(|_|"authorization_original_dispatch_unavailable")?;
    if previously_dispatched {return Err("authorization_side_already_claimed".into());}
    let (lease,assignment)=crate::agent_runtime::multi_agent::budget::target::child_owner(&tx,&child.run_id)?
        .ok_or("authorization_budget_owner_missing")?;
    crate::agent_runtime::multi_agent::budget::supervision_capacity::require_authorization_probe_headroom_on(&tx, &lease)?;
    let changed=tx.execute(
        "INSERT INTO agent_authorization_probe_claims(scan_id,attempt_number,target_url,contract_key,side,child_run_id,assignment_id) \
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![context.scan_id, context.attempt_number, context.target_url, control.contract_key,
            side, child.run_id, child.assignment_id],
    ).map_err(|_| "authorization_side_already_claimed".to_string())?;
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2
        AND target_url=?3 AND contract_key=?4 AND side=?5 AND child_run_id=?6 AND assignment_id=?7 AND artifact_id='')",
        params![context.scan_id,context.attempt_number,context.target_url,control.contract_key,side,child.run_id,child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if changed!=1 || !exact { return Err("authorization_claim_postcondition".into()); }

    crate::agent_runtime::multi_agent::budget::target::claim(&tx,&lease,&assignment,
        &authorization_budget_source(child,control,side))?;
    agent_require_frozen_web_plan(&tx,context).map_err(str::to_string)?;
    if authorization_probe_authority(&tx,context,child,control,side,identity,url,true)?!=Some(budget) {
        return Err("authorization_capability_or_fencing_denied".into());
    }
    let (rowid, created_at): (i64, String) = tx.query_row(
        "SELECT rowid,created_at FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2 AND target_url=?3 AND contract_key=?4 AND side=?5 AND child_run_id=?6 AND assignment_id=?7 AND artifact_id=''",
        params![context.scan_id,context.attempt_number,context.target_url,control.contract_key,side,child.run_id,child.assignment_id],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).map_err(|_| "authorization_fresh_claim_missing")?;
    let worker: String = tx.query_row("SELECT id FROM agent_assignment_attempts WHERE child_run_id=?1 AND assignment_id=?2 AND root_run_id=?3 AND coordinator_epoch=?4 AND coordinator_fencing_token=?5 AND state='running'",params![child.run_id,child.assignment_id,lease.root_run_id,lease.lease_epoch,lease.fencing_token],|r|r.get(0)).map_err(|_|"authorization_fresh_worker_missing")?;
    let attempt = context.attempt_number.to_string();
    let source = authorization_budget_source(child,control,side);
    let mut originals = Vec::new();
    for (sql, keys) in [
        ("SELECT rowid,* FROM agent_authorization_controls WHERE scan_id=?1 AND attempt_number=CAST(?2 AS INTEGER) AND target_url=?3 AND contract_key=?4",vec![context.scan_id.as_str(),attempt.as_str(),context.target_url.as_str(),control.contract_key.as_str()]),
        ("SELECT rowid,id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,leased_at FROM agent_assignment_attempts WHERE id=?1",vec![worker.as_str()]),
        ("SELECT rowid,id,coordinator_run_id,child_run_id,role,lane,target_key,task_slice_json,evidence_revision,lease_epoch,fencing_token,reserved_tokens,reserved_requests,capability_lease_json,contract_keys_json,identity_handles_json FROM agent_assignments WHERE id=?1",vec![child.assignment_id.as_str()]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 AND dimension='target_requests' AND kind IN ('reserve','forfeit') ORDER BY rowid",vec![lease.root_run_id.as_str(),child.assignment_id.as_str(),worker.as_str(),source.as_str()]),
    ] { originals.push(NativeCoordinatorFrozenRows::capture(&tx,sql,&keys)?); }
    tx.commit().map_err(|_| "authorization_claim_commit_failed".to_string())?;
    Ok(ClaimedAuthorizationSide { rowid, created_at, actor: lease, worker, originals })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn authorization_probe_authority(connection: &rusqlite::Connection, context: &AgentRunContext,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control: &SaveAuthorizationControlInput, side: &str, identity: &str, url: &str,
    during_own_claim: bool)
    -> Result<Option<i64>,String> {
    crate::agent_runtime::multi_agent::attempts::require_live_for_run(connection,&child.run_id)
        .map_err(|_| "authorization_capability_or_fencing_denied".to_string())?;
    let homogeneous:bool=connection.query_row("SELECT scheduling_model='homogeneous_workers' FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if homogeneous {
        let root:String=connection.query_row("SELECT coordinator_run_id FROM agent_assignments WHERE id=?1",[&child.assignment_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        let actor=crate::commands::homogeneous_execution::actor_for_root(connection,&root)?;
        // Before the claim the budget is fully determinate (strict gate). After
        // the probe claims its side, and again inside the response transaction
        // before reconciliation, the probe's own single target request is the
        // exactly-one indeterminate entry the probe-tolerant gate admits.
        if during_own_claim {
            crate::commands::homogeneous_execution::check_target_scope_during_own_probe(connection,&actor,&child.assignment_id,&control.contract_key)?;
        } else {
            crate::commands::homogeneous_execution::check_target_scope(connection,&actor,&child.assignment_id,&control.contract_key)?;
        }
    }
    connection.query_row(
        "SELECT (SELECT COUNT(*)*3 FROM agent_authorization_controls scope \
           WHERE scope.scan_id=r.scan_id AND scope.attempt_number=r.attempt_number AND scope.target_url=r.target_url) FROM agent_runs r \
         JOIN sentinel_scans s ON s.id=r.scan_id AND s.attempt_count=r.attempt_number AND s.status='scanning' \
         JOIN agent_assignments a ON a.id=r.assignment_id AND a.child_run_id=r.id \
         JOIN agent_coordinator_leases c ON c.root_run_id=a.coordinator_run_id \
           AND c.scan_id=r.scan_id AND c.attempt_number=r.attempt_number AND c.target_key=r.target_url \
         JOIN agent_runs root ON root.id=c.root_run_id AND root.root_run_id=root.id AND root.role='coordinator' \
           AND root.scan_id=c.scan_id AND root.attempt_number=c.attempt_number AND root.target_url=c.target_key \
           AND root.status IN ('prepared','running') AND r.root_run_id=root.id \
         JOIN agent_lane_leases l ON l.assignment_id=a.id AND l.scan_id=r.scan_id \
           AND l.attempt_number=r.attempt_number AND l.target_key=r.target_url AND l.lane='target_touching' \
         JOIN agent_capability_leases p ON p.assignment_id=a.id AND p.child_run_id=r.id \
           AND p.root_run_id=a.coordinator_run_id AND p.capability='authorization_probe' \
         JOIN agent_authorization_controls k ON k.scan_id=r.scan_id AND k.attempt_number=r.attempt_number \
           AND k.target_url=r.target_url AND k.contract_key=?4 AND k.method='GET' \
           AND k.owner_object_value=?10 AND k.tester_object_value=?11 AND k.response_object_pointer=?12 \
         WHERE r.id=?1 AND r.scan_id=?2 AND r.attempt_number=?3 AND r.target_url=?5 \
           AND r.orchestration_policy='multi' AND r.role='authorization' AND r.lane='target_touching' AND r.status='running' \
           AND a.id=?6 AND a.role='authorization' AND a.lane='target_touching' AND a.state='running' \
           AND a.target_key=r.target_url \
           AND a.lease_epoch=c.lease_epoch AND a.fencing_token=c.fencing_token \
           AND p.lease_epoch=c.lease_epoch AND p.fencing_token=c.fencing_token AND p.revoked_at='' \
           AND a.lease_expires_at>datetime('now','localtime') AND p.lease_expires_at>datetime('now','localtime') \
           AND c.lease_expires_at>datetime('now','localtime') \
           AND ((?7='owner' AND k.owner_identity=?8 AND k.owner_object_url=?9) \
             OR (?7='cross' AND k.tester_identity=?8 AND k.owner_object_url=?9) \
             OR (?7='tester' AND k.tester_identity=?8 AND k.tester_control_url=?9))",
        params![child.run_id, context.scan_id, context.attempt_number, control.contract_key,
            context.target_url, child.assignment_id, side, identity, url,
            control.owner_object_value, control.tester_object_value, control.response_object_pointer],
        |row| row.get(0),
    ).optional().map_err(|_| "authorization_broker_unavailable".to_string())
}

fn authorization_budget_source(child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control: &SaveAuthorizationControlInput, side: &str) -> String {
    format!("authorization:{}:{}:{}",child.assignment_id,
        crate::agent_runtime::store::stable_hash(&control.contract_key),side)
}

/// The Authorization child has no model-controlled URL or HTTP tool. Only this
/// three-side driver can reach the dedicated broker above. It intentionally
/// does not run the generic replay/discovery pipeline (CORS and enumeration
/// probes would violate the exact three-request control group).
pub(super) fn execute_authorization_side(
    context: &AgentRunContext,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    control: &SaveAuthorizationControlInput,
    side: &str,
    identity_key: &str,
    url: &str,
) -> Result<(AuthorizationProbeResponse, String), String> {
    let cancel = agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number);
    if cancel.is_cancelled() { return Err("authorization_attempt_cancelled".into()); }
    let identity = agent_identity_of(context, identity_key)
        .ok_or_else(|| "authorization_identity_unbound".to_string())?;
    let (normalized_url, _) = agent_scope_check(context, url, "GET", ScopeSource::IdentityComparison)
        .map_err(|_| "authorization_frozen_scope_denied".to_string())?;
    if normalized_url != url { return Err("authorization_url_not_canonical".into()); }
    let headers = agent_identity_headers(context, &identity)
        .map_err(|_| "authorization_identity_unavailable".to_string())?;
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(20));
    if let Some(proxy) = &context.proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)
            .map_err(|_| "authorization_proxy_invalid".to_string())?);
    }
    let client = builder.build().map_err(|_| "authorization_client_unavailable".to_string())?;
    let mut request = client.get(url)
        .header(reqwest::header::USER_AGENT, "oviraptor-authorization-control/1.0")
        .header(reqwest::header::CACHE_CONTROL, "no-store");
    for (name, value) in &headers {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| "authorization_session_header_invalid".to_string())?;
        request = request.header(name, value);
    }
    if cancel.is_cancelled() { return Err("authorization_attempt_cancelled".into()); }
    let fresh_claim = claim_authorization_probe_fresh(context, child, control, side, identity_key, url)?;
    let connection = db::open(&context.db_path)?;
    agent_require_frozen_web_plan(&connection, context).map_err(str::to_string)?;
    if cancel.is_cancelled() { return Err("authorization_attempt_cancelled".into()); }
    if authorization_probe_authority(&connection,context,child,control,side,identity_key,url,true)?.is_none() {
        return Err("authorization_capability_or_fencing_denied".into());
    }
    request=request.timeout(crate::agent_runtime::multi_agent::budget::target::transport_timeout(
        &connection,&child.run_id,Duration::from_secs(20))?);
    let mut response = request.send().map_err(|_| "authorization_request_failed".to_string())?;
    let status = response.status().as_u16();
    let content_type = response.headers().get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok()).unwrap_or_default().to_string();
    let response_headers = response.headers().iter().filter_map(|(key, value)| {
        Some((key.as_str().to_string(), value.to_str().ok()?.to_string()))
    }).collect::<Vec<_>>();
    let cache_state = agent_cache_state(&response_headers).to_string();
    let redirect_code = if (300..400).contains(&status) { "redirect" } else { "" }.to_string();
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut response).take(AGENT_MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut bytes).map_err(|_| "authorization_response_read_failed".to_string())?;
    let truncated = bytes.len() > AGENT_MAX_RESPONSE_BYTES;
    bytes.truncate(AGENT_MAX_RESPONSE_BYTES);
    let body = String::from_utf8(bytes).map_err(|_| "authorization_response_not_utf8".to_string())?;
    // A scan can be paused while the blocking GET is in flight. A claimed
    // request is still charged, but its response must not become proof after
    // the operator has stopped this attempt.
    if cancel.is_cancelled() { return Err("authorization_attempt_cancelled".into()); }
    let record = AuthorizationProbeResponse {
        attempt_number: context.attempt_number,
        contract_key: control.contract_key.clone(),
        identity: identity_key.to_string(), url: url.to_string(), status,
        content_type: content_type.clone(), redirect_code: redirect_code.clone(),
        cache_state: cache_state.clone(), truncated, body,
    };
    let written = agent_write_http_record_fresh(
        context, 1,
        &serde_json::json!({"identity": identity_key,"method":"GET","url":url,
            "headers":headers.iter().map(|(name,value)| serde_json::json!({"name":name,"value":value})).collect::<Vec<_>>(),
            "contractKey":control.contract_key}),
        &serde_json::json!({"identity":identity_key,"status":status,"contentType":content_type,
            "redirectCode":redirect_code,"cacheState":cache_state,"truncated":truncated,
            "authorizationSide":side,"rawBodyPrivate":true}),
        record.body.as_bytes(),
    ).map_err(|_| "authorization_evidence_write_failed".to_string())?;
    if written.scan_id() != context.scan_id || written.attempt_number() != context.attempt_number
        || written.target_url() != context.target_url || written.run_id() != Some(child.run_id.as_str())
        || written.directory() != context.target_dir.join(AGENT_HTTP_DIRECTORY).as_path()
    { return Err("authorization_fresh_artifact_scope_invalid".into()); }
    let artifact_id = written.artifact_id().to_string();
    let connection=db::open(&context.db_path)?;
    let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Immediate).map_err(|e|e.to_string())?;
    for row in &fresh_claim.originals { row.verify(&tx)?; }
    if authorization_probe_authority(&tx,context,child,control,side,identity_key,url,true)?.is_none() {
        return Err("authorization_capability_or_fencing_denied".into());
    }
    let updated = tx.execute(
        "UPDATE agent_authorization_probe_claims SET artifact_id=?1 WHERE scan_id=?2 AND attempt_number=?3 \
         AND target_url=?4 AND contract_key=?5 AND side=?6 AND child_run_id=?7 AND artifact_id='' AND rowid=?8 AND created_at=?9",
        params![artifact_id, context.scan_id, context.attempt_number, context.target_url,
            control.contract_key, side, child.run_id, fresh_claim.rowid, fresh_claim.created_at],
    ).map_err(|_| "authorization_claim_artifact_failed".to_string())?;
    if updated != 1 { return Err("authorization_claim_artifact_fenced".into()); }
    let exact:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=?2
        AND target_url=?3 AND contract_key=?4 AND side=?5 AND child_run_id=?6 AND assignment_id=?7 AND artifact_id=?8)",
        params![context.scan_id,context.attempt_number,context.target_url,control.contract_key,side,child.run_id,child.assignment_id,artifact_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact { return Err("authorization_claim_artifact_fenced".into()); }
    let (lease,assignment)=crate::agent_runtime::multi_agent::budget::target::child_owner(&tx,&child.run_id)?
        .ok_or("authorization_budget_owner_missing")?;
    crate::agent_runtime::multi_agent::budget::target::receive(&tx,&lease,&assignment,
        &authorization_budget_source(child,control,side))?;
    if lease.root_run_id != fresh_claim.actor.root_run_id || lease.lease_epoch != fresh_claim.actor.lease_epoch
        || lease.fencing_token != fresh_claim.actor.fencing_token { return Err("authorization_original_actor_changed".into()); }
    crate::agent_runtime::multi_agent::budget::target::original_target_receipt(&tx,&lease,&child.run_id,
        &authorization_budget_source(child,control,side))?;
    if let Some(issuer) = original_withdrawal_issuer_on(&tx,&lease)? {
        let storage_id: i64 = tx.query_row("SELECT coalesce(max(storage_id),0)+1 FROM agent_authorization_side_receipts",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        let created: String = tx.query_row("SELECT datetime('now','localtime')",[],|r|r.get(0)).map_err(|e|e.to_string())?;
        let receipt_id = uuid::Uuid::new_v4().to_string();
        let mut originals = fresh_claim.originals;
        let attempt=context.attempt_number.to_string();
        let source=authorization_budget_source(child,control,side);
        for (sql,keys) in [
            ("SELECT rowid,* FROM agent_authorization_probe_claims WHERE scan_id=?1 AND attempt_number=CAST(?2 AS INTEGER) AND target_url=?3 AND contract_key=?4 AND side=?5",vec![context.scan_id.as_str(),attempt.as_str(),context.target_url.as_str(),control.contract_key.as_str(),side]),
            ("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 AND source_id=?4 AND dimension='target_requests' ORDER BY rowid",vec![lease.root_run_id.as_str(),child.assignment_id.as_str(),fresh_claim.worker.as_str(),source.as_str()]),
        ] { originals.push(NativeCoordinatorFrozenRows::capture(&tx,sql,&keys)?); }
        let original_rows: Vec<JsonValue> = originals.iter().map(|r|serde_json::json!({"sql":r.sql,"keys":r.keys,"hash":r.hash})).collect();
        let raw=serde_json::json!({"receiptId":receipt_id,"storageId":storage_id,"createdAt":created,
            "rootRunId":lease.root_run_id,"scanId":context.scan_id,"attemptNumber":context.attempt_number,
            "targetUrl":context.target_url,"contractKey":control.contract_key,"side":side,
            "claimRowid":fresh_claim.rowid,"assignmentId":child.assignment_id,"childRunId":child.run_id,
            "workerAttemptId":fresh_claim.worker,"artifactId":artifact_id,
            "coordinatorEpoch":lease.lease_epoch,"coordinatorFence":lease.fencing_token,
            "originalRows":original_rows,"recordSha256":written.record_sha256(),"bodySha256":written.body_sha256(),
            "recordBytes":written.record_bytes(),"bodyBytes":written.body_bytes()}).to_string();
        let fresh=FreshAuthorizationSideReceipt {actor:lease,raw,connection:&*tx,originals};
        issuer.issue_authorization_side_on(&tx,&fresh)?;
    }
    tx.commit().map_err(|_|"authorization_claim_artifact_commit_failed")?;
    Ok((record, artifact_id))
}
