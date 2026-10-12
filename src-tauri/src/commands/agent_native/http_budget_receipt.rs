//! Unwired private HTTP transport adapter; no file/IPC signing or backfill API.
//! The caller must keep the SAME private Connection from committed pre-send
//! claim capture through the actual headers writer. Never reconstruct a fresh
//! witness from a saved received claim. Receipt persistence/schema is separate.
use crate::agent_runtime::{
    multi_agent::{budget, lease::CoordinatorLease},
    web_mode::{http_budget_receipt as receipt, VerifiedMode},
};
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HttpClaimIdentity {
    run_id: String,
    invocation_id: String,
    request_index: i64,
    ordinal: i64,
    budget_attempt: i64,
    request_hash: String,
}
impl HttpClaimIdentity {
    fn source(&self) -> String {
        format!(
            "http:{}:{}:{}:{}",
            self.run_id, self.invocation_id, self.request_index, self.request_hash
        )
    }
    fn validate(&self) -> Result<(), String> {
        if self.run_id.is_empty()
            || self.invocation_id.is_empty()
            || self.request_index <= 0
            || self.ordinal <= 0
            || self.budget_attempt <= 0
            || self.request_hash.len() != 64
            || !self.request_hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("http_budget_receipt_claim_invalid".into());
        }
        Ok(())
    }
}

pub(crate) struct FreshHttpBudgetClaim {
    connection: *const Connection,
    actor: CoordinatorLease,
    original: VerifiedMode,
    identity: HttpClaimIdentity,
    pre_send_rows: Vec<super::NativeCoordinatorFrozenRows>,
}

impl FreshHttpBudgetClaim {
    // Call only from the actual claim writer, after original claim+reserve+
    // forfeit exist and before sending. AgentHttpClaim has no public constructor.
    // No received JSON/value or WAF witness is accepted by this constructor.
    pub(super) fn before_send_on(
        tx: &Transaction<'_>,
        context: &super::AgentRunContext,
        claim: &super::AgentHttpClaim,
    ) -> Result<Self, String> {
        if claim.root_target.is_some() {
            return Err("http_budget_receipt_multi_required".into());
        }
        let (actor, _) = budget::target::child_owner(tx, &claim.run_id)?
            .ok_or("http_budget_receipt_multi_required")?;
        let original = super::original_root_capture_mode(tx, &actor.root_run_id)?;
        receipt::require_protocol_on(tx, &actor.root_run_id, &original)?;
        let identity = HttpClaimIdentity {
            run_id: claim.run_id.clone(),
            invocation_id: claim.invocation_id.clone(),
            request_index: claim.request_index,
            ordinal: claim.ordinal,
            budget_attempt: claim.budget_attempt,
            request_hash: claim.request_hash.clone(),
        };
        identity.validate()?;
        let tool: String = tx
            .query_row(
                "SELECT tool_name FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2
             AND budget_attempt=?3 AND ordinal=?4 AND run_id=?5 AND invocation_id=?6
             AND request_index=?7 AND request_hash=?8 AND attempt_number=?9
             AND response_status=0 AND received_at=''",
                params![
                    actor.scan_id,
                    actor.target_key,
                    identity.budget_attempt,
                    identity.ordinal,
                    identity.run_id,
                    identity.invocation_id,
                    identity.request_index,
                    identity.request_hash,
                    actor.attempt_number
                ],
                |row| row.get(0),
            )
            .map_err(|_| "http_budget_receipt_fresh_claim_required")?;
        super::agent_authorize_tool_on(tx, context, &tool).map_err(str::to_string)?;
        if actor.scan_id != context.scan_id
            || actor.attempt_number != context.attempt_number
            || actor.target_key != context.target_url
            || context
                .run
                .as_ref()
                .is_none_or(|run| run.run_id != identity.run_id)
        {
            return Err("http_budget_receipt_scope_invalid".into());
        }
        let pre_send_rows = source_rows(tx, &actor, &identity, false)?;
        Ok(Self {
            connection: &**tx,
            actor,
            original,
            identity,
            pre_send_rows,
        })
    }

    // Runtime's private typed receipt constructor calls this only after the
    // actual headers writer has saved status and reconciled the original cost.
    pub(crate) fn received_fact_on(
        &self,
        db: &Connection,
        status: u16,
    ) -> Result<(VerifiedMode, Value), String> {
        if db.is_autocommit() || !std::ptr::eq(self.connection, db) {
            return Err("http_budget_receipt_original_writer_required".into());
        }
        if !(100..=599).contains(&status) {
            return Err("http_budget_receipt_status_invalid".into());
        }
        receipt::require_protocol_on(db, &self.actor.root_run_id, &self.original)?;
        for row in &self.pre_send_rows {
            row.verify(db)?;
        }
        require_received_on(db, &self.actor, &self.identity, status)?;
        let rows = source_rows(db, &self.actor, &self.identity, true)?;
        let binding: String = db
            .query_row(
                "SELECT binding_json FROM main.agent_root_mode_birth_seals WHERE root_run_id=?1",
                [&self.actor.root_run_id],
                |row| row.get(0),
            )
            .map_err(|_| "http_budget_receipt_birth_missing")?;
        let value = json!({"schemaVersion":1,"purpose":"original_received_http_target_cost",
            "rootRunId":self.actor.root_run_id,"scanId":self.actor.scan_id,
            "attemptNumber":self.actor.attempt_number,"targetUrl":self.actor.target_key,
            "coordinatorEpoch":self.actor.lease_epoch,"coordinatorFence":self.actor.fencing_token,
            "modeReceiptId":self.original.fact().receipt_id,
            "birthBindingHash":crate::agent_runtime::store::stable_hash(&binding),
            "claim":self.identity,"sourceId":self.identity.source(),"status":status,
            "targetRequests":1,"originalFacts":rows.iter().map(|row|row.fact()).collect::<Vec<_>>()});
        Ok((self.original.clone(), value))
    }
}

pub(crate) fn verify_saved_on(
    db: &Connection,
    actor: &CoordinatorLease,
    value: &Value,
    signature: &[u8],
) -> Result<(), String> {
    let original = super::original_root_capture_mode(db, &actor.root_run_id)?;
    receipt::verify_saved_on(db, &original, value, signature)?;
    if value["rootRunId"] != actor.root_run_id
        || value["scanId"] != actor.scan_id
        || value["attemptNumber"] != actor.attempt_number
        || value["targetUrl"] != actor.target_key
        || value["coordinatorEpoch"] != actor.lease_epoch
        || value["coordinatorFence"] != actor.fencing_token
    {
        return Err("http_budget_receipt_original_actor_changed".into());
    }
    let identity: HttpClaimIdentity = serde_json::from_value(value["claim"].clone())
        .map_err(|_| "http_budget_receipt_claim_invalid")?;
    identity.validate()?;
    let status = value["status"]
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .filter(|n| (100..=599).contains(n))
        .ok_or("http_budget_receipt_status_invalid")?;
    if value["sourceId"] != identity.source() {
        return Err("http_budget_receipt_source_changed".into());
    }
    require_received_on(db, actor, &identity, status)?;
    let rows = source_rows(db, actor, &identity, true)?;
    if value["originalFacts"] != json!(rows.iter().map(|row| row.fact()).collect::<Vec<_>>()) {
        return Err("http_budget_receipt_original_fact_changed".into());
    }
    Ok(())
}

fn require_received_on(
    db: &Connection,
    actor: &CoordinatorLease,
    identity: &HttpClaimIdentity,
    status: u16,
) -> Result<(), String> {
    let exact: bool = db
        .query_row(
            "SELECT count(*)=1 FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2
         AND budget_attempt=?3 AND ordinal=?4 AND run_id=?5 AND invocation_id=?6
         AND request_index=?7 AND request_hash=?8 AND attempt_number=?9
         AND response_status=?10 AND received_at<>''",
            params![
                actor.scan_id,
                actor.target_key,
                identity.budget_attempt,
                identity.ordinal,
                identity.run_id,
                identity.invocation_id,
                identity.request_index,
                identity.request_hash,
                actor.attempt_number,
                status
            ],
            |row| row.get(0),
        )
        .map_err(|_| "http_budget_receipt_headers_missing")?;
    if !exact {
        return Err("http_budget_receipt_headers_changed".into());
    }
    budget::target::original_target_receipt(db, actor, &identity.run_id, &identity.source())?;
    Ok(())
}

fn source_rows(
    db: &Connection,
    actor: &CoordinatorLease,
    identity: &HttpClaimIdentity,
    received: bool,
) -> Result<Vec<super::NativeCoordinatorFrozenRows>, String> {
    if db.is_autocommit() {
        return Err("http_budget_receipt_snapshot_required".into());
    }
    let budget_attempt = identity.budget_attempt.to_string();
    let ordinal = identity.ordinal.to_string();
    let source = identity.source();
    let mut rows = vec![
        super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,scan_id,target_url,budget_attempt,baseline_requests,schema_version FROM agent_http_budget_origins WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=CAST(?3 AS INTEGER)",
            &[&actor.scan_id,&actor.target_key,&budget_attempt])?,
        super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,scan_id,target_url,budget_attempt,ordinal,attempt_number,run_id,invocation_id,request_index,tool_name,identity_handle,request_hash,created_at FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=CAST(?3 AS INTEGER) AND ordinal=CAST(?4 AS INTEGER)",
            &[&actor.scan_id,&actor.target_key,&budget_attempt,&ordinal])?,
        super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT id,run_id,tool_name,tool_version,contract_key,identity_handle,input_hash,policy_decision,started_at FROM tool_invocations WHERE run_id=?1 AND invocation_id=?2 AND policy_decision='allow'",
            &[&identity.run_id,&identity.invocation_id])?,
        super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id FROM agent_assignment_attempts WHERE child_run_id=?1",
            &[&identity.run_id])?,
        super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT a.rowid,a.id,a.coordinator_run_id,a.child_run_id,a.role,a.lane,a.target_key,a.dedup_key,a.trigger_code,a.task_slice_json,a.contract_keys_json,a.identity_handles_json,a.capability_lease_json,a.created_at FROM agent_assignments a JOIN agent_runs r ON r.assignment_id=a.id AND r.id=a.child_run_id WHERE r.id=?1",
            &[&identity.run_id])?,
        super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,id,scan_id,attempt_number,target_url,backend,role,parent_run_id,root_run_id,assignment_id,lane,orchestration_policy,plan_hash,plan_json,created_at FROM agent_runs WHERE id=?1",
            &[&identity.run_id])?,
        super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,root_run_id,binding_json,signature FROM main.agent_root_mode_birth_seals WHERE root_run_id=?1",
            &[&actor.root_run_id])?,
        super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id=?2 AND dimension='target_requests' AND kind IN ('reserve','forfeit') ORDER BY rowid",
            &[&actor.root_run_id,&source])?,
    ];
    if received {
        rows.push(super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,* FROM agent_http_request_claims WHERE scan_id=?1 AND target_url=?2 AND budget_attempt=CAST(?3 AS INTEGER) AND ordinal=CAST(?4 AS INTEGER)",
            &[&actor.scan_id,&actor.target_key,&budget_attempt,&ordinal])?);
        rows.push(super::NativeCoordinatorFrozenRows::capture(db,
            "SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND source_id=?2 AND dimension='target_requests' ORDER BY rowid",
            &[&actor.root_run_id,&source])?);
    }
    Ok(rows)
}
