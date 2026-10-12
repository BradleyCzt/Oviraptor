//! Original expired Web fees only. No worker/C renewal or business completion.
use crate::agent_runtime::{
    contract::AgentRole,
    execution_owner::NativeInvocationOwner,
    multi_agent::{attempts, budget, lease::CoordinatorLease, scheduler::ScheduledChild},
    store::{self, UsageDelta},
    web_mode::{self, withdrawal_issuer::WithdrawalIssuer},
};
use rusqlite::{params, types::Value, Connection, Transaction};
use serde_json::{json, Value as JsonValue};

pub(crate) const RULE: &str = "original_expired_web_fee_delivery_advisory_v1";

pub(crate) struct ExpiredWebFinancialProof {
    actor: CoordinatorLease,
    child: ScheduledChild,
    worker: String,
    expiry_receipt: String,
    usage: UsageDelta,
    http_receipts: Vec<String>,
    rows: Vec<Vec<Vec<Value>>>,
    // One original SDK/execution exit ownership lifetime. Readers below do not
    // reprobe inodes already held by this outer proof.
    _idle: Vec<NativeInvocationOwner>,
}
impl ExpiredWebFinancialProof {
    #[cfg(test)]
    pub(crate) fn usage(&self) -> &UsageDelta {
        &self.usage
    }
    pub(crate) fn worker_attempt_id(&self) -> &str {
        &self.worker
    }
    pub(crate) fn expiry_receipt_id(&self) -> &str {
        &self.expiry_receipt
    }
    #[cfg(test)]
    pub(crate) fn http_receipt_ids(&self) -> &[String] {
        &self.http_receipts
    }
    pub(crate) fn fact(&self) -> JsonValue {
        json!({"schemaVersion":1,"purpose":"original_expired_web_financial_closure",
            "rootRunId":self.actor.root_run_id,"scanId":self.actor.scan_id,"attemptNumber":self.actor.attempt_number,
            "targetKey":self.actor.target_key,"coordinatorEpoch":self.actor.lease_epoch,"coordinatorFence":self.actor.fencing_token,
            "assignmentId":self.child.assignment_id,"childRunId":self.child.run_id,"workerAttemptId":self.worker,
            "expiryReceiptId":self.expiry_receipt,"usage":self.usage.as_json(),"httpReceiptIds":self.http_receipts,
            "physicalFactsHash":store::stable_hash(&format!("{:?}",self.rows)),
            "executionAllowed":false,"recoveryImplemented":false})
    }
    pub(crate) fn require_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        if db.is_autocommit() {
            return Err("expired_web_financial_snapshot_required".into());
        }
        if actor.root_run_id != self.actor.root_run_id
            || actor.scan_id != self.actor.scan_id
            || actor.attempt_number != self.actor.attempt_number
            || actor.target_key != self.actor.target_key
            || actor.lease_epoch != self.actor.lease_epoch
            || actor.fencing_token != self.actor.fencing_token
            || child.assignment_id != self.child.assignment_id
            || child.run_id != self.child.run_id
            || child.role != AgentRole::WebExecutor
        {
            return Err("expired_web_financial_scope_changed".into());
        }
        require_protocol_on(db, actor)?;
        attempts::withdrawal_receipt::verify_original(db, &self.worker)?;
        budget::root::RootOwner::load_original(db, &actor.root_run_id)?
            .require_original_coordinator(db, actor)?;
        let worker = attempts::current(db, actor, &child.assignment_id)?;
        if worker.id != self.worker
            || worker.state != "expired"
            || worker.child_run_id != child.run_id
        {
            return Err("expired_web_financial_worker_changed".into());
        }
        let usage = budget::web_lifetime::child_usage_original(
            db,
            actor,
            &child.assignment_id,
            &child.run_id,
        )?;
        if usage != self.usage
            || http_receipts_on(db, actor, child, &self.worker)? != self.http_receipts
            || original_rows_on(db, actor, child, &self.worker)? != self.rows
        {
            return Err("expired_web_financial_original_fact_changed".into());
        }
        require_settled_projection_on(db, actor, child, &self.usage)?;
        require_settled_balances_on(
            db,
            actor,
            child,
            &self.worker,
            &self.usage,
            self.http_receipts.len(),
        )
    }
}

// Only original issued withdrawal plus actual original returned/unsent invoices
// can enter. No constructor accepts UsageDelta, checkpoint, JSON or row hashes.
pub(crate) fn settle_expired_web_receipt_on(
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    issuer: &WithdrawalIssuer,
) -> Result<ExpiredWebFinancialProof, String> {
    let savepoint = format!("expired_web_financial_{}", uuid::Uuid::new_v4().simple());
    tx.execute_batch(&format!("SAVEPOINT {savepoint}"))
        .map_err(|e| e.to_string())?;
    match settle_expired_web_inner_on(tx, actor, child, issuer) {
        Ok(proof) => {
            if let Err(error) = tx.execute_batch(&format!("RELEASE {savepoint}")) {
                tx.execute_batch(&format!("ROLLBACK TO {savepoint}; RELEASE {savepoint}"))
                    .map_err(|cleanup| {
                        format!("{error};expired_web_financial_rollback:{cleanup}")
                    })?;
                return Err(error.to_string());
            }
            Ok(proof)
        }
        Err(error) => {
            tx.execute_batch(&format!("ROLLBACK TO {savepoint}; RELEASE {savepoint}"))
                .map_err(|cleanup| format!("{error};expired_web_financial_rollback:{cleanup}"))?;
            Err(error)
        }
    }
}

fn settle_expired_web_inner_on(
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    issuer: &WithdrawalIssuer,
) -> Result<ExpiredWebFinancialProof, String> {
    require_protocol_on(tx, actor)?;
    if child.role != AgentRole::WebExecutor {
        return Err("expired_web_financial_role_invalid".into());
    }
    budget::root::RootOwner::load_original(tx, &actor.root_run_id)?
        .require_original_coordinator(tx, actor)?;
    let worker = attempts::current(tx, actor, &child.assignment_id)?;
    if worker.state != "expired"
        || worker.failure_class != "worker_lease_expired"
        || worker.child_run_id != child.run_id
    {
        return Err("expired_web_financial_original_expiry_required".into());
    }
    issuer.require_actor(tx, actor)?;
    issuer.verify_issued(tx, &worker.id)?;
    let paused:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id AND r.assignment_id=a.id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.role='web_executor' AND r.role=a.role AND r.status='paused' AND r.terminal_state='' AND r.finished_at='' AND a.state='paused' AND a.failure_class='worker_lease_expired' AND a.finished_at='' AND r.cancel_requested_at='' AND NOT EXISTS(SELECT 1 FROM agent_capability_leases WHERE (assignment_id=a.id OR child_run_id=r.id) AND revoked_at=''))",params![child.assignment_id,child.run_id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if !paused {
        return Err("expired_web_financial_paused_source_required".into());
    }
    super::mapper_allocation::require_original_invoice_guards(tx)?;
    // Verify HTTP sources before any tentative settlement; missing/unknown
    // receipts stay missing/unknown, with their original reservations intact.
    let http_receipts = http_receipts_on(tx, actor, child, &worker.id)?;
    let guard = FinancialWriteGuard::install(tx)?;
    // Reuse the complete normal original physical verifier and BoundReceipt
    // settlement without weakening its running caller's original contract.
    let (usage, idle) =
        super::settle_original_web_receipt(tx, actor, &child.assignment_id, &child.run_id)?;
    settle_paused_projection_on(tx, actor, child, &usage)?;
    drop(guard);
    issuer.verify_issued(tx, &worker.id)?;
    require_settled_balances_on(tx, actor, child, &worker.id, &usage, http_receipts.len())?;
    let expiry_receipt=tx.query_row("SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1 AND root_run_id=?2",params![worker.id,actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    let rows = original_rows_on(tx, actor, child, &worker.id)?;
    let proof = ExpiredWebFinancialProof {
        actor: actor.clone(),
        child: ScheduledChild {
            assignment_id: child.assignment_id.clone(),
            run_id: child.run_id.clone(),
            role: child.role,
        },
        worker: worker.id,
        expiry_receipt,
        usage,
        http_receipts,
        rows,
        _idle: idle,
    };
    proof.require_on(tx, actor, child)?;
    Ok(proof)
}

fn require_protocol_on(db: &Connection, actor: &CoordinatorLease) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("expired_web_financial_snapshot_required".into());
    }
    let mode = web_mode::root::read(db, &actor.root_run_id)?
        .ok_or("expired_web_original_protocol_required")?;
    let bootstrap = mode.bootstrap_dispatch().unwrap_or(JsonValue::Null);
    if mode.mode() != web_mode::WebMode::Multi
        || !matches!(bootstrap["schemaVersion"].as_i64(), Some(9..=15))
        || bootstrap["webLeaseExpiryRule"] != RULE
        || mode.lease_expiry_supervision().is_none()
    {
        return Err("expired_web_original_protocol_required".into());
    }
    Ok(())
}

fn http_receipts_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
) -> Result<Vec<String>, String> {
    let unknown:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_http_request_claims WHERE run_id=?1 AND (response_status=0 OR received_at='')) OR EXISTS(SELECT 1 FROM tool_invocations WHERE run_id=?1 AND status='running')",[&child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
    if unknown {
        return Err("budget_indeterminate_requires_reconciliation".into());
    }
    web_mode::http_budget_storage::verify_schema_on(db)?;
    let prefix = format!("http:{}:", child.run_id);
    let sources=db.prepare("SELECT 'http:'||run_id||':'||invocation_id||':'||request_index||':'||request_hash FROM agent_http_request_claims WHERE run_id=?2 UNION SELECT source_id FROM main.agent_http_budget_receipts WHERE root_run_id=?1 AND run_id=?2 UNION SELECT source_id FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?3 AND lease_attempt_id=?4 AND dimension='target_requests' AND substr(source_id,1,length(?5))=?5 ORDER BY 1").map_err(|e|e.to_string())?
        .query_map(params![actor.root_run_id,child.run_id,child.assignment_id,worker,prefix],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
    sources
        .iter()
        .map(|source| {
            let receipt = web_mode::http_budget_storage::read_original_on(db, actor, source)?
                .ok_or("expired_web_original_http_receipt_missing")?;
            if receipt.fact()["claim"]["runId"] != child.run_id {
                return Err("expired_web_financial_scope_changed".into());
            }
            Ok(receipt.receipt_id().to_owned())
        })
        .collect()
}

fn settle_paused_projection_on(
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    usage: &UsageDelta,
) -> Result<(), String> {
    let (tokens,requests,settled):(i64,i64,String)=tx.query_row("SELECT reserved_tokens,reserved_requests,budget_settled_at FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 AND state='paused' AND failure_class='worker_lease_expired'",params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(|_|"expired_web_financial_projection_conflict")?;
    if !settled.is_empty() {
        return require_settled_projection_on(tx, actor, child, usage);
    }
    if tokens < 0 || requests < 0 {
        return Err("expired_web_financial_projection_conflict".into());
    }
    let count=tx.execute("UPDATE agent_budget_ledger SET reserved_tokens=reserved_tokens-?1,reserved_requests=reserved_requests-?2,spent_tokens=spent_tokens+?3,spent_requests=spent_requests+?4,updated_at=datetime('now','localtime') WHERE root_run_id=?5 AND lease_epoch=?6 AND fencing_token=?7 AND reserved_tokens>=?1 AND reserved_requests>=?2 AND (total_tokens=0 OR ?3<=?1) AND (total_requests=0 OR ?4<=?2) AND (total_tokens=0 OR spent_tokens+reserved_tokens-?1+?3<=total_tokens) AND (total_requests=0 OR spent_requests+reserved_requests-?2+?4<=total_requests)",params![tokens,requests,usage.total_tokens,usage.model_requests,actor.root_run_id,actor.lease_epoch,actor.fencing_token]).map_err(|e|e.to_string())?;
    if count != 1 {
        return Err("expired_web_financial_projection_conflict".into());
    }
    let count=tx.execute("UPDATE agent_runs SET used_tokens=?1,used_cached_tokens=?2,used_requests=?3,updated_at=datetime('now','localtime') WHERE id=?4 AND root_run_id=?5 AND status='paused' AND terminal_state='' AND finished_at='' AND cancel_requested_at=''",params![usage.total_tokens,usage.cached_input_tokens,usage.model_requests,child.run_id,actor.root_run_id]).map_err(|e|e.to_string())?;
    if count != 1 {
        return Err("expired_web_financial_projection_conflict".into());
    }
    let count=tx.execute("UPDATE agent_assignments SET reserved_tokens=0,reserved_requests=0,budget_settled_at=datetime('now','localtime'),updated_at=datetime('now','localtime') WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND lease_epoch=?4 AND fencing_token=?5 AND state='paused' AND failure_class='worker_lease_expired' AND budget_settled_at=''",params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token]).map_err(|e|e.to_string())?;
    if count != 1 {
        return Err("expired_web_financial_projection_conflict".into());
    }
    require_settled_projection_on(tx, actor, child, usage)
}

fn require_settled_projection_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    usage: &UsageDelta,
) -> Result<(), String> {
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id AND r.assignment_id=a.id WHERE a.id=?1 AND r.id=?2 AND a.coordinator_run_id=?3 AND a.lease_epoch=?4 AND a.fencing_token=?5 AND a.state='paused' AND a.failure_class='worker_lease_expired' AND a.budget_settled_at<>'' AND a.reserved_tokens=0 AND a.reserved_requests=0 AND a.finished_at='' AND r.status='paused' AND r.terminal_state='' AND r.finished_at='' AND r.used_tokens=?6 AND r.used_cached_tokens=?7 AND r.used_requests=?8)",params![child.assignment_id,child.run_id,actor.root_run_id,actor.lease_epoch,actor.fencing_token,usage.total_tokens,usage.cached_input_tokens,usage.model_requests],|r|r.get(0)).map_err(|e|e.to_string())?;
    if valid {
        Ok(())
    } else {
        Err("expired_web_financial_projection_conflict".into())
    }
}

fn require_settled_balances_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
    usage: &UsageDelta,
    http_count: usize,
) -> Result<(), String> {
    let targets = i64::try_from(http_count).map_err(|_| "budget_amount_overflow")?;
    for (index, dimension) in budget::DIMENSIONS.iter().enumerate() {
        let balance = budget::balance_for_attempt(
            db,
            &actor.root_run_id,
            &child.assignment_id,
            worker,
            dimension,
        )?;
        let expected = match index {
            0 => usage.input_tokens,
            1 => usage.cached_input_tokens,
            2 => usage.output_tokens,
            3 => usage.model_requests,
            4 => targets,
            9 => budget::clock::original_worker_samples::original_worker_consumed_on(
                db, actor, child, worker,
            )?,
            _ => 0,
        };
        if balance.reserved != 0 || balance.indeterminate != 0 || balance.consumed != expected {
            return Err(format!(
                "expired_web_financial_budget_unresolved:dimension={dimension}:reserved={}:indeterminate={}:consumed={}:expected={expected}",
                balance.reserved, balance.indeterminate, balance.consumed,
            ));
        }
    }
    Ok(())
}

fn original_rows_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
) -> Result<Vec<Vec<Vec<Value>>>, String> {
    let mut result = Vec::new();
    for (sql,keys) in [
        ("SELECT rowid,* FROM agent_root_mode_birth_seals WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_root_mode_definitions WHERE root_run_id=?1",vec![actor.root_run_id.as_str()]),
        ("SELECT rowid,* FROM agent_assignment_attempts WHERE id=?1",vec![worker]),
        ("SELECT rowid,* FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?1",vec![worker]),
        ("SELECT rowid,* FROM agent_worker_expiry_issuance_seals WHERE root_run_id=?1 AND receipt_id IN (SELECT receipt_id FROM agent_worker_expiry_receipts WHERE worker_attempt_id=?2)",vec![actor.root_run_id.as_str(),worker]),
        ("SELECT rowid,* FROM agent_web_model_journal WHERE child_run_id=?1 ORDER BY rowid",vec![child.run_id.as_str()]),
        ("SELECT rowid,* FROM agent_budget_entries WHERE root_run_id=?1 AND assignment_id=?2 AND lease_attempt_id=?3 ORDER BY rowid",vec![actor.root_run_id.as_str(),child.assignment_id.as_str(),worker]),
        ("SELECT * FROM main.agent_http_budget_receipts WHERE root_run_id=?1 AND run_id=?2 ORDER BY receipt_id",vec![actor.root_run_id.as_str(),child.run_id.as_str()]),
    ] {
        let mut query=db.prepare(sql).map_err(|e|e.to_string())?;
        let count=query.column_count();
        let rows=query.query_map(rusqlite::params_from_iter(keys),|r|(0..count).map(|i|r.get::<_,Value>(i)).collect::<rusqlite::Result<Vec<_>>>()).map_err(|e|e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e|e.to_string())?;
        result.push(rows);
    }
    Ok(result)
}

struct FinancialWriteGuard<'a>(&'a Connection);
impl FinancialWriteGuard<'_> {
    fn install(db: &Connection) -> Result<FinancialWriteGuard<'_>, String> {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        db.authorizer(Some(|a: AuthContext<'_>| {
            let direct = a.database_name == Some("main") && a.accessor.is_none();
            let allow = match a.action {
                AuthAction::Insert { table_name } => direct && table_name == "agent_budget_entries",
                AuthAction::Update {
                    table_name,
                    column_name,
                } => {
                    direct
                        && match table_name {
                            "agent_budget_ledger" => matches!(
                                column_name,
                                "reserved_tokens"
                                    | "reserved_requests"
                                    | "spent_tokens"
                                    | "spent_requests"
                                    | "updated_at"
                            ),
                            "agent_runs" => matches!(
                                column_name,
                                "used_tokens"
                                    | "used_cached_tokens"
                                    | "used_requests"
                                    | "updated_at"
                            ),
                            "agent_assignments" => matches!(
                                column_name,
                                "reserved_tokens"
                                    | "reserved_requests"
                                    | "budget_settled_at"
                                    | "updated_at"
                            ),
                            _ => false,
                        }
                }
                AuthAction::Read { .. }
                | AuthAction::Select
                | AuthAction::Function { .. }
                | AuthAction::Transaction { .. }
                | AuthAction::Savepoint { .. }
                | AuthAction::Recursive => true,
                _ => false,
            };
            if allow {
                Authorization::Allow
            } else {
                Authorization::Deny
            }
        }))
        .map_err(|_| "expired_web_financial_writer_guard_unavailable")?;
        Ok(FinancialWriteGuard(db))
    }
}
impl Drop for FinancialWriteGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.authorizer(
            None::<fn(rusqlite::hooks::AuthContext<'_>) -> rusqlite::hooks::Authorization>,
        );
    }
}
