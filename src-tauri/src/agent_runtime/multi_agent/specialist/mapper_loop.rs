//! Original v15 Mapper rounds, immutable paid lookup and fee-chain proof.
//! Dispatch authority comes from the original grant and private typed sources.
//! This never upgrades legacy one-request workers or completes terminal ACKs.
use crate::agent_runtime::{
    contract::AgentRole,
    execution_owner::{self, NativeInvocationOwner},
    model::{gateway::ModelResponse, ModelRequest},
    multi_agent::{
        attempts, budget,
        lease::{self, CoordinatorLease},
        scheduler::{self, ScheduledChild},
    },
    secrets,
    store::{self, UsageDelta},
};
use crate::commands::mapper_loop_request::VerifiedMapperLoopRequestBounds;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};
pub(crate) mod final_seal;
pub(crate) mod expired_saved;
pub(crate) mod expired_seal;
mod idle;
pub(crate) use idle::{verify_for_idle_on, VerifiedMapperIdle};
mod diagnostic;
pub(crate) use diagnostic::cost_phase_original_on;
mod lookup;
mod output;
pub(crate) use output::{
    require_settled_output_on, verified_original_output_on, verified_output_on,
    VerifiedMapperLoopOutput,
};
mod writer;
pub(crate) use lookup::{audit_lookup_on, begin_second_for_transport, VerifiedMapperLookupAudit};
pub(crate) const SCHEMA: &str = include_str!("mapper_loop/schema.sql");
pub(crate) const INVOCATION_KIND: &str = "mapper-loop-sdk";
pub(crate) enum FinancialStage {
    Unsettled,
    FinancialAttemptFailed,
    FinancialCommitted,
}
pub(crate) struct PendingRound {
    actor: CoordinatorLease,
    child: ScheduledChild,
    worker: String,
    id: String,
    request_hash: String,
    dispatch: Value,
    estimate: i64,
    round: i64,
    request: ModelRequest,
    stage: FinancialStage,
    financial_failure: Option<String>,
    _owner: NativeInvocationOwner,
}
pub(crate) struct VerifiedMapperRound {
    actor: CoordinatorLease,
    child: ScheduledChild,
    worker: String,
    id: String,
    request_hash: String,
    fact: Value,
    response: ModelResponse,
    round: i64,
}
impl VerifiedMapperRound {
    pub(crate) fn require_on(
        &self,
        db: &Connection,
        actor: &CoordinatorLease,
        child: &ScheduledChild,
    ) -> Result<(), String> {
        if actor != &self.actor || child != &self.child {
            return Err("mapper_loop_original_received_scope_changed".into());
        }
        let reread = read_received_on(db, actor, child, &self.id)?;
        if reread.worker != self.worker
            || reread.request_hash != self.request_hash
            || reread.fact != self.fact
        {
            return Err("mapper_loop_original_received_changed".into());
        }
        Ok(())
    }
    pub(crate) fn response(&self) -> &ModelResponse {
        &self.response
    }
    pub(crate) fn rejection(&self) -> &str {
        self.fact["rejection"]
            .as_str()
            .unwrap_or("mapper_loop_received_invalid")
    }
}
pub(crate) enum Start {
    Dispatch(PendingRound),
    Received(VerifiedMapperRound),
}

pub(crate) fn require_schema(db: &Connection) -> Result<(), String> {
    let expected = Connection::open_in_memory().map_err(|e| e.to_string())?;
    expected.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    for name in [
        "agent_mapper_loop_journal",
        "mapper_loop_journal_no_update",
        "mapper_loop_journal_no_delete",
        "mapper_loop_journal_no_replace",
    ] {
        let read = |c: &Connection| -> Result<String, String> {
            c.query_row(
                "SELECT sql FROM main.sqlite_master WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .map_err(|_| "mapper_loop_schema_missing".into())
        };
        if read(db)? != read(&expected)? {
            return Err("mapper_loop_schema_changed".into());
        }
    }
    Ok(())
}
fn read(db: &Connection, id: &str, phase: &str) -> Result<Option<Value>, String> {
    let raw:Option<(String,String)>=db.query_row("SELECT fact_json,fact_hash FROM main.agent_mapper_loop_journal WHERE call_id=?1 AND phase=?2",params![id,phase],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|e|e.to_string())?;
    raw.map(|(text, hash)| {
        let value: Value =
            serde_json::from_str(&text).map_err(|_| "mapper_loop_original_json_invalid")?;
        let canonical = value.to_string();
        if canonical != text || store::stable_hash(&text) != hash {
            return Err("mapper_loop_original_hash_changed".into());
        }
        Ok(value)
    })
    .transpose()
}
fn insert(
    tx: &Transaction<'_>,
    pending: &PendingRound,
    phase: &str,
    fact: &Value,
) -> Result<(), String> {
    if read(tx, &pending.id, phase)?.is_some() {
        return Err("mapper_loop_phase_already_written".into());
    }
    let text = fact.to_string();
    let n=tx.execute("INSERT INTO main.agent_mapper_loop_journal(call_id,phase,root_run_id,assignment_id,child_run_id,worker_attempt_id,round_number,request_hash,fact_json,fact_hash) VALUES(?1,?2,?3,?4,?5,?6,?10,?7,?8,?9)",params![pending.id,phase,pending.actor.root_run_id,pending.child.assignment_id,pending.child.run_id,pending.worker,pending.request_hash,text,store::stable_hash(&text),pending.round]).map_err(|e|e.to_string())?;
    if n != 1 || read(tx, &pending.id, phase)?.as_ref() != Some(fact) {
        return Err("mapper_loop_phase_write_unconfirmed".into());
    }
    Ok(())
}
fn original(
    db: &Connection,
    pending: &PendingRound,
) -> Result<budget::receipts::OriginalReceiptOwner, String> {
    require_schema(db)?;
    budget::root::RootOwner::load_original(db, &pending.actor.root_run_id)?
        .require_original_coordinator(db, &pending.actor)?;
    let owner = budget::receipts::original_owner(
        db,
        &pending.child.run_id,
        &pending.actor,
        &pending.child.assignment_id,
    )?;
    if owner.attempt_id() != pending.worker
        || read(db, &pending.id, "dispatch")?.as_ref() != Some(&pending.dispatch)
    {
        return Err("mapper_loop_original_dispatch_changed".into());
    }
    require_rows(
        db,
        &pending.id,
        &pending.actor,
        &pending.child,
        &pending.worker,
        (&pending.request_hash, pending.round),
    )?;
    if pending.round == 2 {
        lookup::verify_second_on(db, &pending.actor, &pending.child, &pending.dispatch)?;
    }
    owner.verify(db)?;
    Ok(owner)
}
fn require_rows(
    db: &Connection,
    id: &str,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    worker: &str,
    identity: (&str, i64),
) -> Result<(), String> {
    let (hash, round) = identity;
    let bad:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM main.agent_mapper_loop_journal WHERE call_id=?1 AND (root_run_id<>?2 OR assignment_id<>?3 OR child_run_id<>?4 OR worker_attempt_id<>?5 OR round_number<>?7 OR request_hash<>?6))",params![id,actor.root_run_id,child.assignment_id,child.run_id,worker,hash,round],|r|r.get(0)).map_err(|e|e.to_string())?;
    let expected=store::stable_hash(&json!({"root":actor.root_run_id,"assignment":child.assignment_id,"worker":worker,"child":child.run_id,"round":round,"requestHash":hash}).to_string());
    if bad || expected != id {
        return Err("mapper_loop_original_scope_changed".into());
    }
    Ok(())
}
fn live(
    tx: &Transaction<'_>,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    bounds: &VerifiedMapperLoopRequestBounds,
    estimate: i64,
) -> Result<(), String> {
    if child.role != AgentRole::SpaApiMapper {
        return Err("mapper_loop_role_invalid".into());
    }
    lease::require_executable_coordinator(tx, actor)?;
    attempts::require_live_for_run(tx, &child.run_id)?;
    budget::admission::require_determinate(tx, &actor.root_run_id)?;
    budget::clock::remaining(tx, &actor.root_run_id)?;
    let grant = budget::model::original_mapper_loop_grant_on(tx, actor, &child.assignment_id)?;
    if grant.input().fact() != bounds.fact() || grant.requests() != 2 {
        return Err("mapper_loop_original_input_changed".into());
    }
    bounds.require_original_on(tx, actor)?;
    let raw: String = tx
        .query_row(
            "SELECT task_slice_json FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let task = serde_json::from_str(&raw).map_err(|_| "mapper_loop_task_invalid")?;
    scheduler::verify_running_mapper_loop_in_transaction(tx, actor, child, &task, grant.input())?;
    for (dimension, needed) in [
        ("model_input_tokens", estimate),
        ("model_cached_tokens", estimate),
        ("model_output_tokens", estimate),
        ("model_requests", 1),
    ] {
        let balance = budget::balance(
            tx,
            &actor.root_run_id,
            Some(&child.assignment_id),
            dimension,
        )?;
        if balance.indeterminate != 0 || balance.reserved < needed {
            return Err("mapper_loop_reserved_budget_exhausted".into());
        }
    }
    Ok(())
}
pub(crate) fn begin_first_for_transport(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    bounds: &VerifiedMapperLoopRequestBounds,
) -> Result<Start, String> {
    writer::run(db, |writer| {
        let tx = Transaction::new_unchecked(writer, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        require_schema(&tx)?;
        let grant = budget::model::original_mapper_loop_grant_on(&tx, actor, &child.assignment_id)?;
        if grant.input().fact() != bounds.fact() {
            return Err("mapper_loop_original_input_changed".into());
        }
        let hash = store::stable_hash(&bounds.fact()["request"].to_string());
        let id=store::stable_hash(&json!({"root":actor.root_run_id,"assignment":child.assignment_id,"worker":grant.worker_attempt_id(),"child":child.run_id,"round":1,"requestHash":hash}).to_string());
        if read(&tx, &id, "dispatch")?.is_some() {
            return if read(&tx, &id, "received")?.is_some() {
                Ok(Start::Received(read_received_on(&tx, actor, child, &id)?))
            } else {
                Err("mapper_loop_original_outcome_requires_reconciliation".into())
            };
        }
        let conflict:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_specialist_calls WHERE child_run_id=?1) OR EXISTS(SELECT 1 FROM agent_mapper_loop_journal WHERE child_run_id=?1)",[&child.run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if conflict {
            return Err("mapper_loop_original_history_conflict".into());
        }
        live(&tx, actor, child, bounds, bounds.first_estimate())?;
        let path = writer
            .path()
            .filter(|p| !p.is_empty())
            .ok_or("mapper_loop_database_missing")?;
        let owner = execution_owner::claim_native_invocation(
            Path::new(path),
            &actor.scan_id,
            actor.attempt_number,
            INVOCATION_KIND,
            &child.run_id,
        )?;
        budget::clock::sample(&tx, actor, &child.assignment_id)?;
        let pending = PendingRound {
            actor: actor.clone(),
            child: child.clone(),
            worker: grant.worker_attempt_id().into(),
            id,
            request_hash: hash,
            dispatch: json!({"schemaVersion":1,"scope":{"rootRunId":actor.root_run_id,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":grant.worker_attempt_id(),"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"epoch":actor.lease_epoch,"fence":actor.fencing_token},"round":1,"inputHash":grant.input().hash(),"request":bounds.fact()["request"],"estimate":bounds.first_estimate(),"configuration":bounds.original_configuration_binding()}),
            estimate: bounds.first_estimate(),
            round: 1,
            request: bounds.first_request().clone(),
            stage: FinancialStage::Unsettled,
            financial_failure: None,
            _owner: owner,
        };
        insert(&tx, &pending, "dispatch", &pending.dispatch)?;
        original(&tx, &pending)?;
        live(&tx, actor, child, bounds, bounds.first_estimate())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(Start::Dispatch(pending))
    })
}
use std::path::Path;
fn invoice(response: &ModelResponse) -> Value {
    json!({"rawResponseHash":store::stable_hash(&json!({"text":response.text,"toolCalls":response.tool_calls.iter().map(|c|json!({"id":c.id,"name":c.name,"arguments":c.arguments})).collect::<Vec<_>>(),"finishReason":response.finish_reason,"usage":response.usage.as_json(),"usageReported":response.usage_reported,"cachedUsageReported":response.cached_usage_reported}).to_string()),"usage":response.usage.as_json(),"usageReported":response.usage_reported,"cachedUsageReported":response.cached_usage_reported})
}
impl PendingRound {
    pub(crate) fn request(&self) -> &ModelRequest {
        &self.request
    }
    pub(crate) fn financial_failure(&self) -> Option<&str> {
        self.financial_failure.as_deref()
    }
    pub(crate) fn financial_committed(&self) -> bool {
        matches!(self.stage, FinancialStage::FinancialCommitted)
    }
    pub(crate) fn require_executable(
        &self,
        db: &Connection,
        bounds: &VerifiedMapperLoopRequestBounds,
    ) -> Result<(), String> {
        let tx = Transaction::new_unchecked(db, TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        original(&tx, self)?;
        live(&tx, &self.actor, &self.child, bounds, self.estimate)
    }
    pub(crate) fn persist_received_invoice(
        &mut self,
        db: &Connection,
        response: &ModelResponse,
    ) -> Result<(), String> {
        if !matches!(self.stage, FinancialStage::Unsettled) {
            return Err("mapper_loop_financial_attempt_no_retry".into());
        }
        let bill = invoice(response);
        let result = writer::run(db, |writer| {
            let tx = Transaction::new_unchecked(writer, TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            original(&tx, self)?;
            if let Some(saved) = read(&tx, &self.id, "invoice")? {
                if saved != bill {
                    return Err("mapper_loop_original_invoice_changed".into());
                }
            } else {
                insert(&tx, self, "invoice", &bill)?;
            }
            tx.commit().map_err(|e| e.to_string())
        });
        if result.is_err() {
            self.stage = FinancialStage::FinancialAttemptFailed;
            self.financial_failure = result.as_ref().err().cloned();
        }
        result
    }
    pub(crate) fn settle_received(
        &mut self,
        db: &Connection,
        response: &ModelResponse,
    ) -> Result<VerifiedMapperRound, String> {
        if !matches!(self.stage, FinancialStage::Unsettled) {
            return Err("mapper_loop_financial_attempt_no_retry".into());
        }
        self.stage = FinancialStage::FinancialAttemptFailed;
        let result = writer::run(db, |writer| {
            let tx = Transaction::new_unchecked(writer, TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let owner = original(&tx, self)?;
            let bill = invoice(response);
            if read(&tx, &self.id, "invoice")?.as_ref() != Some(&bill) {
                return Err("mapper_loop_original_invoice_missing_or_changed".into());
            }
            let fact = semantic(response, &bill, self.estimate)?;
            let hash = store::stable_hash(&fact.to_string());
            budget::receipts::record_original_estimated(
                &tx,
                &owner,
                &source(&self.id),
                &hash,
                &response.usage,
                Some(self.estimate),
                response.usage_reported,
            )?;
            insert(&tx, self, "received", &fact)?;
            let received = read_received_on(&tx, &self.actor, &self.child, &self.id)?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(received)
        });
        if result.is_ok() {
            self.stage = FinancialStage::FinancialCommitted;
        } else if let Err(error) = &result {
            self.financial_failure = Some(error.clone());
        }
        result
    }
    pub(crate) fn record_uncertain(&mut self, db: &Connection) -> Result<(), String> {
        if !matches!(self.stage, FinancialStage::Unsettled) {
            return Err("mapper_loop_financial_attempt_no_retry".into());
        }
        self.stage = FinancialStage::FinancialAttemptFailed;
        let result = writer::run(db, |writer| {
            let tx = Transaction::new_unchecked(writer, TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            let owner = original(&tx, self)?;
            if read(&tx, &self.id, "invoice")?.is_some() {
                return Err("mapper_loop_uncertain_after_original_invoice_denied".into());
            }
            budget::receipts::forfeit_original_call_estimated(
                &tx,
                &owner,
                &source(&self.id),
                Some(self.estimate),
            )?;
            insert(
                &tx,
                self,
                "uncertain",
                &json!({"code":"model_outcome_unknown"}),
            )?;
            tx.commit().map_err(|e| e.to_string())
        });
        if result.is_ok() {
            self.stage = FinancialStage::FinancialCommitted;
        } else if let Err(error) = &result {
            self.financial_failure = Some(error.clone());
        }
        result
    }
    pub(crate) fn record_not_sent(
        &mut self,
        db: &Connection,
        failure: &crate::agent_runtime::model::OneShotFailure,
    ) -> Result<(), String> {
        let crate::agent_runtime::model::OneShotFailure::BeforeTransport(error) = failure else {
            return Err("mapper_loop_no_send_proof_invalid".into());
        };
        if !matches!(self.stage, FinancialStage::Unsettled) {
            return Err("mapper_loop_financial_attempt_no_retry".into());
        }
        self.stage = FinancialStage::FinancialAttemptFailed;
        let result = writer::run(db, |writer| {
            let tx = Transaction::new_unchecked(writer, TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            original(&tx, self)?;
            if read(&tx, &self.id, "invoice")?.is_some() {
                return Err("mapper_loop_no_send_after_original_invoice_denied".into());
            }
            insert(&tx, self, "unsent", &json!({"code":error.code()}))?;
            tx.commit().map_err(|e| e.to_string())
        });
        if result.is_ok() {
            self.stage = FinancialStage::FinancialCommitted;
        } else if let Err(error) = &result {
            self.financial_failure = Some(error.clone());
        }
        result
    }
}
fn source(id: &str) -> String {
    format!("mapper-loop:{id}")
}
fn semantic(response: &ModelResponse, bill: &Value, estimate: i64) -> Result<Value, String> {
    let used = &response.usage;
    if used.model_requests != 1
        || used.input_tokens < 0
        || used.cached_input_tokens < 0
        || used.output_tokens < 0
        || used.cached_input_tokens > used.input_tokens
        || used.input_tokens.checked_add(used.output_tokens) != Some(used.total_tokens)
    {
        return Err("mapper_loop_original_usage_invalid".into());
    }
    let mut rejection = if !response.usage_reported {
        "model_usage_requires_reconciliation"
    } else if used.output_tokens > 512 {
        "mapper_loop_provider_output_cap_exceeded"
    } else if used.total_tokens > estimate {
        "mapper_loop_provider_over_budget"
    } else {
        ""
    };
    if response.tool_calls.len() > 1
        || !response.tool_calls.is_empty()
            && (!response.text.is_empty() || response.finish_reason != "tool_calls")
    {
        rejection = "mapper_loop_response_shape_invalid";
    }
    let mut calls = Vec::new();
    for call in &response.tool_calls {
        if call.name != "evidence.lookup"
            || call.id.is_empty()
            || call.id.len() > 64
            || !call.arguments.is_object()
            || call.arguments.to_string().len() > 64
            || secrets::redact_json(&call.arguments) != call.arguments
            || secrets::redact_text_with(&call.id, None) != call.id
        {
            rejection = "mapper_loop_original_tool_invalid";
        }
        calls.push(json!({"id":call.id,"name":call.name,"arguments":call.arguments}));
    }
    let text = if rejection.is_empty() && calls.is_empty() {
        secrets::redact_text_with(&response.text, None)
    } else {
        String::new()
    };
    if !rejection.is_empty() {
        calls.clear();
    }
    Ok(
        json!({"schemaVersion":1,"invoice":bill,"text":text,"toolCalls":calls,"finishReason":response.finish_reason,"rejection":rejection}),
    )
}
pub(crate) fn verify_dispatch_original_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    id: &str,
) -> Result<(), String> {
    if db.is_autocommit() {
        return Err("mapper_loop_snapshot_required".into());
    }
    require_schema(db)?;
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    let dispatch = read(db, id, "dispatch")?.ok_or("mapper_loop_original_dispatch_missing")?;
    let scope = &dispatch["scope"];
    require_rows(
        db,
        id,
        actor,
        child,
        grant.worker_attempt_id(),
        (
            &store::stable_hash(&dispatch["request"].to_string()),
            dispatch["round"]
                .as_i64()
                .filter(|r| (1..=2).contains(r))
                .ok_or("mapper_loop_round_invalid")?,
        ),
    )?;
    if child.role != AgentRole::SpaApiMapper
        || scope
            != &json!({"rootRunId":actor.root_run_id,"assignmentId":child.assignment_id,"childRunId":child.run_id,"workerAttemptId":grant.worker_attempt_id(),"scanId":actor.scan_id,"attemptNumber":actor.attempt_number,"targetKey":actor.target_key,"epoch":actor.lease_epoch,"fence":actor.fencing_token})
        || dispatch["inputHash"] != grant.input().hash()
        || dispatch["configuration"] != grant.input().fact()["configurationBinding"]
        || (dispatch["round"] == 1
            && dispatch["estimate"] != grant.input().fact()["firstRoundEstimatedTokens"])
        || (dispatch["round"] == 1 && dispatch["request"] != grant.input().fact()["request"])
    {
        return Err("mapper_loop_original_scope_changed".into());
    }
    if dispatch["round"] == 2 {
        lookup::verify_second_on(db, actor, child, &dispatch)?;
    }
    let owner = budget::receipts::original_owner(db, &child.run_id, actor, &child.assignment_id)?;
    if owner.attempt_id() != grant.worker_attempt_id() {
        return Err("mapper_loop_original_worker_changed".into());
    }
    owner.verify(db)
}
pub(crate) fn read_received_on(
    db: &Connection,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    id: &str,
) -> Result<VerifiedMapperRound, String> {
    if db.is_autocommit() {
        return Err("mapper_loop_snapshot_required".into());
    }
    verify_dispatch_original_on(db, actor, child, id)?;
    let grant = budget::model::original_mapper_loop_grant_on(db, actor, &child.assignment_id)?;
    let dispatch = read(db, id, "dispatch")?.ok_or("mapper_loop_original_dispatch_missing")?;
    if read(db, id, "uncertain")?.is_some() || read(db, id, "unsent")?.is_some() {
        return Err("mapper_loop_received_terminal_conflict".into());
    }
    let fact = read(db, id, "received")?.ok_or("mapper_loop_original_received_missing")?;
    let bill = read(db, id, "invoice")?.ok_or("mapper_loop_original_invoice_missing_or_changed")?;
    if fact["invoice"] != bill {
        return Err("mapper_loop_original_invoice_changed".into());
    }
    let usage = &bill["usage"];
    let number = |key: &str| {
        usage[key]
            .as_i64()
            .filter(|v| *v >= 0)
            .ok_or("mapper_loop_original_usage_invalid")
    };
    let used = UsageDelta {
        input_tokens: number("inputTokens")?,
        cached_input_tokens: number("cachedInputTokens")?,
        output_tokens: number("outputTokens")?,
        total_tokens: number("totalTokens")?,
        model_requests: number("modelRequests")?,
    };
    budget::receipts::verify(
        db,
        actor,
        &child.assignment_id,
        &source(id),
        &store::stable_hash(&fact.to_string()),
        &used,
    )?;
    let calls = fact["toolCalls"]
        .as_array()
        .ok_or("mapper_loop_received_shape_invalid")?
        .iter()
        .map(|call| {
            Ok(crate::agent_runtime::model::gateway::ToolCall {
                id: call["id"]
                    .as_str()
                    .ok_or("mapper_loop_received_shape_invalid")?
                    .into(),
                name: call["name"]
                    .as_str()
                    .ok_or("mapper_loop_received_shape_invalid")?
                    .into(),
                arguments: call["arguments"].clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let response = ModelResponse {
        text: fact["text"]
            .as_str()
            .ok_or("mapper_loop_received_shape_invalid")?
            .into(),
        tool_calls: calls,
        usage: used,
        usage_reported: bill["usageReported"]
            .as_bool()
            .ok_or("mapper_loop_received_shape_invalid")?,
        cached_usage_reported: bill["cachedUsageReported"]
            .as_bool()
            .ok_or("mapper_loop_received_shape_invalid")?,
        finish_reason: fact["finishReason"]
            .as_str()
            .ok_or("mapper_loop_received_shape_invalid")?
            .into(),
    };
    Ok(VerifiedMapperRound {
        actor: actor.clone(),
        child: child.clone(),
        worker: grant.worker_attempt_id().into(),
        id: id.into(),
        request_hash: store::stable_hash(&dispatch["request"].to_string()),
        fact,
        response,
        round: dispatch["round"]
            .as_i64()
            .ok_or("mapper_loop_round_invalid")?,
    })
}
