//! Paid original output after signed worker expiry; no SDK or completion issuer.
//! Only capture may establish this opaque source. Saved JSON cannot recreate it.
use super::*;
use crate::agent_runtime::web_mode::withdrawal_issuer::WithdrawalIssuer;
use rusqlite::types::Value as SqlValue;

pub(crate) struct OriginalExpiredMapperOutput<'tx, 'conn> {
    transaction: &'tx Transaction<'conn>,
    actor: CoordinatorLease,
    child: ScheduledChild,
    worker: attempts::AssignmentAttempt,
    output: VerifiedMapperLoopOutput,
    idle: VerifiedMapperIdle,
    fact: Value,
    _sdk_exit: NativeInvocationOwner,
}
impl OriginalExpiredMapperOutput<'_, '_> {
    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
    pub(crate) fn output(&self) -> &VerifiedMapperLoopOutput {
        &self.output
    }
    pub(crate) fn worker_attempt_id(&self) -> &str {
        &self.worker.id
    }
    pub(crate) fn require_before_on(
        &self,
        db: &Transaction<'_>,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        self.binding(db, actor, child)?;
        require_paused_live_on(db, actor, child, &self.worker)?;
        self.idle.require_on(db, actor, child)?;
        self.require_original_on(db, actor, child)
    }
    /// After the dedicated writer closes the original output, check its exact
    /// settled projection. This does not issue a receipt or acknowledge mail.
    pub(crate) fn require_closed_original_on(
        &self,
        db: &Transaction<'_>,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        self.binding(db, actor, child)?;
        self.require_original_on(db, actor, child)?;
        require_expired_settled_projection_on(db, actor, child, &self.worker, &self.output)
    }

    fn binding(
        &self,
        db: &Transaction<'_>,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        if db.is_autocommit()
            || !std::ptr::eq(self.transaction, db)
            || actor != &self.actor
            || child != &self.child
        {
            return Err("mapper_loop_expired_proof_binding_changed".into());
        }
        Ok(())
    }
    fn require_original_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        let worker = require_original_expiry_on(db, actor, child)?;
        let output = verified_original_output_on(db, actor, child)?;
        if worker != self.worker
            || original_fact_on(db, actor, child, &worker, &output)? != self.fact
        {
            return Err("mapper_loop_expired_original_source_changed".into());
        }
        Ok(())
    }
}

pub(crate) fn capture_expired_output_on<'tx, 'conn>(
    db: &'tx Transaction<'conn>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<OriginalExpiredMapperOutput<'tx, 'conn>, String> {
    let worker = require_original_expiry_on(db, actor, child)?;
    require_paused_live_on(db, actor, child, &worker)?;
    let output = verified_original_output_on(db, actor, child)?;
    let idle = verify_for_idle_on(db, actor, child)?;
    if !idle.has_transport() {
        return Err("mapper_loop_expired_original_transport_missing".into());
    }
    let path = db
        .path()
        .filter(|p| !p.is_empty())
        .ok_or("mapper_loop_expired_database_missing")?;
    // Caller must first release the original provider owner. An in-flight SDK
    // must never be treated as a completed original transport.
    let sdk_exit = execution_owner::probe_native_invocation(
        std::path::Path::new(path),
        &actor.scan_id,
        actor.attempt_number,
        "mapper-loop-sdk",
        &child.run_id,
    )
    .map_err(|e| format!("mapper_loop_expired_sdk_not_idle:{e}"))?
    .ok_or("mapper_loop_expired_original_sdk_exit_missing")?;
    idle.require_on(db, actor, child)?;
    let fact = original_fact_on(db, actor, child, &worker, &output)?;
    let proof = OriginalExpiredMapperOutput {
        transaction: db,
        actor: actor.clone(),
        child: child.clone(),
        worker,
        output,
        idle,
        fact,
        _sdk_exit: sdk_exit,
    };
    proof.require_before_on(db, actor, child)?;
    Ok(proof)
}

fn require_original_expiry_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<attempts::AssignmentAttempt, String> {
    budget::model::mapper_loop_contract::require_protocol_on(db, actor)?;
    if child.role != AgentRole::SpaApiMapper {
        return Err("mapper_loop_expired_role_invalid".into());
    }
    let mode = crate::agent_runtime::web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("mapper_loop_expired_original_mode_missing")?;
    let expected = json!({"schemaVersion":1,"source":"original_signed_worker_expiry_receipt","closure":"original_cost_settled_saved_and_acknowledged","deduplication":"original_receipt_physical_fact","permittedStep":"assess:lease_expired","executionGrant":false});
    if mode.lease_expiry_supervision().as_ref() != Some(&expected) {
        return Err("mapper_loop_expired_original_protocol_missing".into());
    }
    let worker = attempts::current(db, actor, &child.assignment_id)?;
    if worker.state != "expired"
        || worker.failure_class != "worker_lease_expired"
        || worker.child_run_id != child.run_id
        || worker.lease_epoch != 1
    {
        return Err("mapper_loop_expired_original_worker_required".into());
    }
    let valid:bool=db.query_row("SELECT datetime(?1) IS NOT NULL AND datetime(?2) IS NOT NULL AND datetime(?1)<=datetime(?2) AND datetime(?2)<=datetime('now','localtime')",params![worker.expires_at,worker.finished_at],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !valid {
        return Err("mapper_loop_expired_original_deadline_invalid".into());
    }
    let original = crate::commands::original_root_capture_mode(db, &actor.root_run_id)?;
    let issuer = WithdrawalIssuer::from_verified_birth_on(db, &actor.root_run_id, original)?;
    issuer.verify_issued(db, &worker.id)?;
    attempts::withdrawal_receipt::verify_original(db, &worker.id)?;
    Ok(worker)
}
fn require_paused_live_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &attempts::AssignmentAttempt,
) -> Result<(), String> {
    lease::validate_coordinator_lease(db, actor)?;
    lease::require_executable_coordinator(db, actor)?;
    let exact:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_lane_leases l ON l.assignment_id=a.id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.role='spa_api_mapper' AND a.lane='read_only_analysis' AND a.state='paused' AND a.failure_class='worker_lease_expired' AND a.budget_settled_at='' AND a.finished_at='' AND r.root_run_id=?3 AND r.parent_run_id=?3 AND r.assignment_id=a.id AND r.role=a.role AND r.lane=a.lane AND r.status='paused' AND r.finished_at='' AND r.terminal_state='' AND r.cancel_requested_at='' AND l.scan_id=?4 AND l.attempt_number=?5 AND l.target_key=?6 AND l.lane=a.lane) AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=?1 OR child_run_id=?2) AND revoked_at='')",params![child.assignment_id,child.run_id,actor.root_run_id,actor.scan_id,actor.attempt_number,actor.target_key],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !exact || attempts::current(db, actor, &child.assignment_id)? != *worker {
        return Err("mapper_loop_expired_paused_projection_required".into());
    }
    Ok(())
}
fn original_fact_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &attempts::AssignmentAttempt,
    output: &VerifiedMapperLoopOutput,
) -> Result<Value, String> {
    let clock = budget::clock::original_worker_samples::original_worker_consumed_on(
        db, actor, child, &worker.id,
    )?;
    for dimension in &budget::DIMENSIONS {
        let balance = budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            &worker.id,
            dimension,
        )?;
        let expected = match *dimension {
            "model_input_tokens" => output.usage().input_tokens,
            "model_cached_tokens" => output.usage().cached_input_tokens,
            "model_output_tokens" => output.usage().output_tokens,
            "model_requests" => output.usage().model_requests,
            "wall_time_ms" => clock,
            _ => 0,
        };
        // Explicit v15 uses the original physical lane, not legacy synthetic
        // concurrency budget slots. Only the four original model reserves may
        // remain before settlement; idle.require_on binds their exact amounts.
        let model = matches!(
            *dimension,
            "model_input_tokens" | "model_cached_tokens" | "model_output_tokens" | "model_requests"
        );
        if balance.consumed != expected
            || balance.indeterminate != 0
            || balance.reserved < 0
            || (!model && balance.reserved != 0)
        {
            return Err(format!(
                "mapper_loop_expired_original_fee_unresolved:{dimension}"
            ));
        }
    }
    let raw = |sql: &str, key: &str| -> Result<Vec<Vec<SqlValue>>, String> {
        let mut statement = db.prepare(sql).map_err(|e| e.to_string())?;
        let width = statement.column_count();
        let rows = statement
            .query_map([key], |row| {
                (0..width)
                    .map(|i| row.get::<_, SqlValue>(i))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .map_err(|e| e.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| e.to_string());
        rows
    };
    let expiry = raw(
        "SELECT rowid,* FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",
        &worker.id,
    )?;
    let seals=raw("SELECT rowid,* FROM agent_worker_expiry_issuance_seals WHERE receipt_id IN (SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1)",&worker.id)?;
    let journal=raw("SELECT rowid,* FROM agent_mapper_loop_journal WHERE child_run_id=?1 ORDER BY round_number,phase",&child.run_id)?;
    let fees=raw("SELECT rowid,* FROM agent_budget_entries WHERE assignment_id=?1 AND kind IN ('reserve','consume','forfeit') ORDER BY rowid",&child.assignment_id)?;
    let receipt: String = db
        .query_row(
            "SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",
            [&worker.id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(
        json!({"schemaVersion":1,"purpose":"original_expired_mapper_paid_output","rootRunId":actor.root_run_id,"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"coordinatorEpoch":actor.lease_epoch,"coordinatorFence":actor.fencing_token,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":worker.id,"expiryReceiptId":receipt,"output":output.fact(),"wallTimeMs":clock,"immutableSourceHash":store::stable_hash(&format!("{expiry:?}:{seals:?}:{journal:?}:{fees:?}")),"executionAllowed":false}),
    )
}

/// Verified historical financial source, not a fresh signer input or live grant.
pub(crate) struct VerifiedExpiredMapperClosed {
    fact: Value,
    output: VerifiedMapperLoopOutput,
}
impl VerifiedExpiredMapperClosed {
    pub(crate) fn fact(&self) -> &Value {
        &self.fact
    }
    pub(crate) fn output(&self) -> &VerifiedMapperLoopOutput {
        &self.output
    }
}
pub(crate) fn read_closed_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
) -> Result<VerifiedExpiredMapperClosed, String> {
    let worker = require_original_expiry_on(db, actor, child)?;
    let output = verified_original_output_on(db, actor, child)?;
    require_expired_settled_projection_on(db, actor, child, &worker, &output)?;
    let fact = original_fact_on(db, actor, child, &worker, &output)?;
    Ok(VerifiedExpiredMapperClosed { fact, output })
}
fn require_expired_settled_projection_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &attempts::AssignmentAttempt,
    output: &VerifiedMapperLoopOutput,
) -> Result<(), String> {
    attempts::verify_expired_closed_audit(db, actor, child)?;
    // Normal final requires worker=completed. Expired local delivery must
    // instead preserve the immutable signed worker=expired source.
    let projection:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.role='spa_api_mapper' AND a.lane='read_only_analysis' AND a.state='completed' AND a.failure_class='' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.finished_at<>'' AND r.assignment_id=a.id AND r.root_run_id=?3 AND r.parent_run_id=?3 AND r.role=a.role AND r.lane=a.lane AND r.status='terminal' AND r.terminal_state='completed' AND r.finished_at<>'' AND r.cancel_requested_at='' AND r.used_tokens=?4 AND r.used_cached_tokens=?5 AND r.used_requests=?6)",params![child.assignment_id,child.run_id,actor.root_run_id,output.usage().total_tokens,output.usage().cached_input_tokens,output.usage().model_requests],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !projection {
        return Err("mapper_loop_expired_settled_projection_changed".into());
    }
    for dimension in &budget::DIMENSIONS {
        let balance = budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            &worker.id,
            dimension,
        )?;
        if balance.reserved != 0 || balance.indeterminate != 0 {
            return Err("mapper_loop_expired_settlement_unresolved".into());
        }
    }
    Ok(())
}
