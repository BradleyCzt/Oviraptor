//! Original Mapper transport seam for the shared bounded executor.
use super::mapper_loop_request::VerifiedMapperLoopRequestBounds;
use super::{AgentModelClient, AgentRunContext, CancelToken};
use crate::agent_runtime::{
    controlled_executor::Stop,
    model::{diagnostics::ModelLog, gateway::ModelResponse, OneShotFailure},
    multi_agent::{lease::CoordinatorLease, scheduler::ScheduledChild, specialist::mapper_loop},
};

struct MapperPaidResponse {
    pending: Option<mapper_loop::PendingRound>,
    response: ModelResponse,
    received: Option<mapper_loop::VerifiedMapperRound>,
    log: ModelLog,
}
// A permit is already a durable, original financial dispatch. The caller may
// not replace its request with a new history or an unbound caller JSON body.
fn mapper_transport_original(
    context: &AgentRunContext,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    bounds: &VerifiedMapperLoopRequestBounds,
    mut pending: mapper_loop::PendingRound,
    round: i64,
) -> Result<MapperPaidResponse, Stop> {
    let db = super::db::open(&context.db_path).map_err(Stop::Persistence)?;
    pending
        .require_executable(&db, bounds)
        .map_err(Stop::Refused)?;
    {
        let snapshot = db
            .unchecked_transaction()
            .map_err(|e| Stop::Persistence(e.to_string()))?;
        bounds
            .require_on(
                &snapshot,
                actor,
                &context.environment,
                context.proxy.as_deref(),
            )
            .map_err(Stop::Refused)?;
    }
    if let Some(ticket) = &context.supervision {
        ticket
            .check_actor(&context.db_path, actor)
            .map_err(Stop::Refused)?;
    }
    let remaining =
        crate::agent_runtime::multi_agent::budget::clock::remaining(&db, &actor.root_run_id)
            .map_err(Stop::Refused)?;
    let deadline = std::time::Instant::now() + remaining;
    let scoped = super::specialist_model_cancel_token(
        &context.db_path,
        actor,
        child,
        context.supervision.clone(),
    );
    let cancel = CancelToken::from_checker(move || {
        scoped.is_cancelled() || std::time::Instant::now() >= deadline
    });
    let mut request = pending.request().clone();
    request.request_timeout = Some(remaining);
    let client = AgentModelClient::new(bounds.profile().clone(), &[]);
    let log = ModelLog::mapper_loop(&context.db_path, actor, child, round);
    let response =
        match client.complete_once_observed_with_lifecycle(&request, &cancel, log.observer()) {
            Ok(response) => response,
            Err(failure) => {
                let (original, persist) = match &failure {
                    OneShotFailure::BeforeTransport(error) => (
                        Stop::Refused(format!("mapper_loop_unsent:{}", error.code())),
                        pending.record_not_sent(&db, &failure),
                    ),
                    OneShotFailure::TransportOutcomeUnknown(error) => (
                        Stop::Unknown(format!("mapper_loop_uncertain:{}", error.code())),
                        pending.record_uncertain(&db),
                    ),
                };
                return match persist {
                    Ok(()) => {
                        log.cost_saved();
                        Err(original)
                    }
                    Err(error) => Err(Stop::PaidAbort {
                        original: Box::new(original),
                        settlement: Box::new(Stop::Persistence(error)),
                    }),
                };
            }
        };
    // This independent original invoice precedes semantic parsing and lookup.
    // Failure retains the pending owner until return; never retry its writer.
    pending
        .persist_received_invoice(&db, &response)
        .map_err(Stop::Persistence)?;
    client.record_usage_line(&context.target_dir, &response);
    Ok(MapperPaidResponse {
        pending: Some(pending),
        response,
        received: None,
        log,
    })
}

fn mapper_original_final_text(received: &mapper_loop::VerifiedMapperRound) -> Result<String, Stop> {
    if !received.rejection().is_empty() {
        return Err(
            if received.rejection() == "model_usage_requires_reconciliation" {
                Stop::Unknown("budget_indeterminate_requires_reconciliation".into())
            } else {
                Stop::Refused(received.rejection().into())
            },
        );
    }
    let response = received.response();
    if !response.tool_calls.is_empty() {
        return Err(Stop::Refused(
            "mapper_loop_final_original_tools_present".into(),
        ));
    }
    let value: serde_json::Value = serde_json::from_str(&response.text)
        .map_err(|_| Stop::Refused("mapper_loop_final_json_invalid".into()))?;
    let valid = value.as_object().is_some_and(|object| {
        object.len() == 3
            && object
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
            && ["priorityContracts", "risks"].iter().all(|key| {
                object
                    .get(*key)
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|items| items.iter().all(serde_json::Value::is_string))
            })
    });
    if !valid {
        return Err(Stop::Refused("mapper_loop_final_schema_invalid".into()));
    }
    Ok(value.to_string())
}

use crate::agent_runtime::controlled_executor::{
    Adapter, Limits, RawToolCall, Received, TextPolicy, ToolEffect, ToolPolicy,
};
use std::{cell::RefCell, rc::Rc};
type MapperPaid = Rc<RefCell<MapperPaidResponse>>;
struct MapperRoundPermit {
    start: mapper_loop::Start,
    round: i64,
}
struct MapperControlledAdapter<'a> {
    context: &'a AgentRunContext,
    actor: &'a CoordinatorLease,
    child: &'a ScheduledChild,
    bounds: &'a VerifiedMapperLoopRequestBounds,
    registered: Vec<String>,
    first: Option<mapper_loop::VerifiedMapperRound>,
    audit: Option<mapper_loop::VerifiedMapperLookupAudit>,
}
impl MapperControlledAdapter<'_> {
    fn live(&self) -> Result<(), Stop> {
        let db = super::db::open(&self.context.db_path).map_err(Stop::Persistence)?;
        if let Some(ticket) = &self.context.supervision {
            ticket
                .check_actor(&self.context.db_path, self.actor)
                .map_err(Stop::Refused)?;
        }
        if super::specialist_model_cancel_token(
            &self.context.db_path,
            self.actor,
            self.child,
            self.context.supervision.clone(),
        )
        .is_cancelled()
        {
            return Err(Stop::Cancelled);
        }
        let tx = db
            .unchecked_transaction()
            .map_err(|e| Stop::Persistence(e.to_string()))?;
        self.bounds
            .require_on(
                &tx,
                self.actor,
                &self.context.environment,
                self.context.proxy.as_deref(),
            )
            .map_err(Stop::Refused)?;
        crate::agent_runtime::multi_agent::budget::admission::require_determinate(
            &tx,
            &self.actor.root_run_id,
        )
        .map_err(Stop::Unknown)
    }
    fn original_lookup(&mut self, paid: &MapperPaid) -> Result<serde_json::Value, Stop> {
        if self.audit.is_some() {
            return Err(Stop::Refused("mapper_loop_lookup_limit".into()));
        }
        self.live()?;
        let borrowed = paid.borrow();
        let received = borrowed
            .received
            .as_ref()
            .ok_or_else(|| Stop::Persistence("mapper_loop_original_paid_round_missing".into()))?;
        let db = super::db::open(&self.context.db_path).map_err(Stop::Persistence)?;
        let audit =
            mapper_loop::audit_lookup_on(&db, self.actor, self.child, self.bounds, received)
                .map_err(Stop::Refused)?;
        let observation = audit.observation().clone();
        self.audit = Some(audit);
        Ok(observation)
    }
    fn require_original_call(
        paid: &MapperPaid,
        id: &str,
        name: &str,
        arguments: &str,
    ) -> Result<(), Stop> {
        let borrowed = paid.borrow();
        let round = borrowed
            .received
            .as_ref()
            .ok_or_else(|| Stop::Persistence("mapper_loop_original_paid_round_missing".into()))?;
        let [call] = round.response().tool_calls.as_slice() else {
            return Err(Stop::Refused(
                "mapper_loop_original_call_group_invalid".into(),
            ));
        };
        if call.id != id || call.name != name || call.arguments.to_string().as_bytes() != arguments.as_bytes() {
            return Err(Stop::Refused("mapper_loop_original_call_changed".into()));
        }
        Ok(())
    }
}
impl Adapter for MapperControlledAdapter<'_> {
    type RoundPermit = MapperRoundPermit;
    type PaidResponse = MapperPaid;
    type ToolPermit = MapperPaid;
    type ToolOriginal = ();
    type Audit = ();
    type Output = mapper_loop::VerifiedMapperLoopOutput;
    fn registered_tools(&self) -> &[String] {
        &self.registered
    }
    fn check_continuation(&mut self) -> Result<(), Stop> {
        self.live()
    }
    fn before_round(&mut self, round: usize) -> Result<MapperRoundPermit, Stop> {
        self.live()?;
        let db = super::db::open(&self.context.db_path).map_err(Stop::Persistence)?;
        let start = match round {
            1 => mapper_loop::begin_first_for_transport(&db, self.actor, self.child, self.bounds),
            2 => mapper_loop::begin_second_for_transport(
                &db,
                self.actor,
                self.child,
                self.bounds,
                self.first.as_ref().ok_or_else(|| {
                    Stop::Persistence("mapper_loop_original_first_missing".into())
                })?,
                self.audit.as_ref().ok_or_else(|| {
                    Stop::Persistence("mapper_loop_original_lookup_missing".into())
                })?,
            ),
            _ => return Err(Stop::Limit("mapper_loop_round_limit")),
        }
        .map_err(Stop::Refused)?;
        Ok(MapperRoundPermit {
            start,
            round: i64::try_from(round).map_err(|_| Stop::Limit("mapper_loop_round_limit"))?,
        })
    }
    fn model_once(
        &mut self,
        permit: MapperRoundPermit,
        _history: &[serde_json::Value],
    ) -> Result<Received<MapperPaid>, Stop> {
        let state = match permit.start {
            mapper_loop::Start::Dispatch(pending) => mapper_transport_original(
                self.context,
                self.actor,
                self.child,
                self.bounds,
                pending,
                permit.round,
            )?,
            mapper_loop::Start::Received(received) => MapperPaidResponse {
                response: received.response().clone(),
                pending: None,
                received: Some(received),
                log: ModelLog::mapper_loop(
                    &self.context.db_path,
                    self.actor,
                    self.child,
                    permit.round,
                ),
            },
        };
        let calls = state
            .response
            .tool_calls
            .iter()
            .map(|c| RawToolCall {
                id: c.id.clone(),
                name: c.name.clone(),
                arguments: c.arguments.to_string(),
            })
            .collect();
        let text = state.response.text.clone();
        Ok(Received {
            paid: Rc::new(RefCell::new(state)),
            text,
            calls,
        })
    }
    fn record_paid_response(&mut self, paid: &MapperPaid, _round: usize) -> Result<(), Stop> {
        let mut state = paid.borrow_mut();
        if state.received.is_none() {
            let response = state.response.clone();
            let db = super::db::open(&self.context.db_path).map_err(Stop::Persistence)?;
            let pending = state
                .pending
                .as_mut()
                .ok_or_else(|| Stop::Persistence("mapper_loop_original_pending_missing".into()))?;
            let received = pending
                .settle_received(&db, &response)
                .map_err(Stop::Persistence)?;
            state.received = Some(received);
            state.log.cost_saved();
        }
        let received = state
            .received
            .as_ref()
            .ok_or_else(|| Stop::Persistence("mapper_loop_original_paid_round_missing".into()))?;
        if !received.rejection().is_empty() {
            return Err(
                if received.rejection() == "model_usage_requires_reconciliation" {
                    Stop::Unknown("budget_indeterminate_requires_reconciliation".into())
                } else {
                    Stop::Refused(received.rejection().into())
                },
            );
        }
        Ok(())
    }
    fn abort_paid_response(&mut self, paid: &MapperPaid, _stop: &Stop) -> Result<(), Stop> {
        let mut state = paid.borrow_mut();
        let response = state.response.clone();
        let Some(pending) = state.pending.as_mut() else {
            return Ok(());
        };
        if let Some(error) = pending.financial_failure() {
            return Err(Stop::Persistence(error.into()));
        }
        if pending.financial_committed() {
            return Ok(());
        }
        let db = super::db::open(&self.context.db_path).map_err(Stop::Persistence)?;
        let received = pending
            .settle_received(&db, &response)
            .map_err(Stop::Persistence)?;
        state.received = Some(received);
        state.log.cost_saved();
        Ok(())
    }
    fn text_policy(&mut self, paid: &MapperPaid, _text: &str) -> Result<TextPolicy, Stop> {
        let state = paid.borrow();
        mapper_original_final_text(
            state.received.as_ref().ok_or_else(|| {
                Stop::Persistence("mapper_loop_original_paid_round_missing".into())
            })?,
        )?;
        self.live()?;
        Ok(TextPolicy::Complete)
    }
    fn validate_arguments(&self, name: &str, args: &serde_json::Value) -> Result<(), String> {
        if name != "evidence.lookup"
            || !args.as_object().is_some_and(|o| {
                o.len() == 1
                    && o.get("evidenceId")
                        .is_some_and(|id| id == "frontend-evidence")
            })
        {
            Err("mapper_lookup_arguments_invalid".into())
        } else {
            Ok(())
        }
    }
    fn before_tool(
        &mut self,
        paid: &MapperPaid,
        id: &str,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<MapperPaid, Stop> {
        self.live()?;
        Self::require_original_call(paid, id, name, &args.to_string())?;
        Ok(paid.clone())
    }
    fn execute_tool(
        &mut self,
        paid: MapperPaid,
        _name: &str,
        _args: &serde_json::Value,
    ) -> Result<ToolEffect<()>, Stop> {
        Ok(ToolEffect {
            original: (),
            observation: self.original_lookup(&paid)?,
            policy: ToolPolicy::Continue,
        })
    }
    fn audit_tool(
        &mut self,
        _paid: &MapperPaid,
        _id: &str,
        _name: &str,
        _args: &serde_json::Value,
        effect: &ToolEffect<()>,
    ) -> Result<(), Stop> {
        let db = super::db::open(&self.context.db_path).map_err(Stop::Persistence)?;
        let tx = db
            .unchecked_transaction()
            .map_err(|e| Stop::Persistence(e.to_string()))?;
        let audit = self
            .audit
            .as_ref()
            .ok_or_else(|| Stop::Persistence("mapper_loop_original_lookup_missing".into()))?;
        audit
            .require_on(&tx, self.actor, self.child)
            .map_err(Stop::Persistence)?;
        if audit.observation() != &effect.observation {
            return Err(Stop::Persistence(
                "mapper_loop_original_observation_changed".into(),
            ));
        }
        Ok(())
    }
    fn audit_rejection(
        &mut self,
        paid: &MapperPaid,
        id: &str,
        name: &str,
        raw: &str,
        _rejection: &str,
    ) -> Result<((), serde_json::Value), Stop> {
        Self::require_original_call(paid, id, name, raw)?;
        Ok(((), self.original_lookup(paid)?))
    }
    fn checkpoint(&mut self, _history: &[serde_json::Value], _audits: &[()]) -> Result<(), Stop> {
        // The financial journal and lookup audit are the immutable checkpoint,
        // re-read before any continuation; no caller history becomes authority.
        if let Some(audit) = &self.audit {
            let db = super::db::open(&self.context.db_path).map_err(Stop::Persistence)?;
            let tx = db
                .unchecked_transaction()
                .map_err(|e| Stop::Persistence(e.to_string()))?;
            audit
                .require_on(&tx, self.actor, self.child)
                .map_err(Stop::Persistence)?;
        }
        Ok(())
    }
    fn after_tools(
        &mut self,
        paid: &MapperPaid,
        _history: &[serde_json::Value],
    ) -> Result<TextPolicy, Stop> {
        self.live()?;
        let mut state = paid.borrow_mut();
        if state
            .pending
            .as_ref()
            .is_some_and(|p| !p.financial_committed())
        {
            return Err(Stop::Persistence(
                "mapper_loop_original_fee_not_committed".into(),
            ));
        }
        // Only the committed first call releases its held inode. Round 2 gets
        // a new original owner while preserving the typed paid/audit chain.
        self.first =
            Some(state.received.take().ok_or_else(|| {
                Stop::Persistence("mapper_loop_original_paid_round_missing".into())
            })?);
        state.pending.take();
        state.log.finish(true);
        Ok(TextPolicy::Continue(Vec::new()))
    }
    fn finish(
        &mut self,
        paid: &MapperPaid,
        _history: &[serde_json::Value],
    ) -> Result<Self::Output, Stop> {
        self.live()?;
        let db = super::db::open(&self.context.db_path).map_err(Stop::Persistence)?;
        let tx = db
            .unchecked_transaction()
            .map_err(|e| Stop::Persistence(e.to_string()))?;
        let proof = mapper_loop::verified_output_on(&tx, self.actor, self.child, self.bounds)
            .map_err(Stop::Refused)?;
        paid.borrow().log.finish(true);
        Ok(proof)
    }
}
pub(super) fn native_run_mapper_controlled_executor(
    context: &AgentRunContext,
    actor: &CoordinatorLease,
    child: &ScheduledChild,
    bounds: &VerifiedMapperLoopRequestBounds,
) -> Result<mapper_loop::VerifiedMapperLoopOutput, String> {
    let db = super::db::open(&context.db_path)?;
    let snapshot = db.unchecked_transaction().map_err(|error| error.to_string())?;
    let completed: bool = snapshot.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND state='completed')",
        rusqlite::params![child.assignment_id, child.run_id], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if completed {
        if crate::agent_runtime::multi_agent::attempts::current(&snapshot, actor, &child.assignment_id)?.state == "expired" {
            mapper_loop::expired_seal::read_original_on(&snapshot, actor, child)?
                .ok_or("mapper_loop_expired_original_receipt_missing")?;
        } else {
            mapper_loop::final_seal::read_final_original_on(&snapshot, actor, child)?
                .ok_or("mapper_loop_final_original_receipt_missing")?;
        }
        return mapper_loop::verified_original_output_on(&snapshot, actor, child);
    }
    if crate::agent_runtime::multi_agent::attempts::current(&snapshot, actor, &child.assignment_id)?.state == "expired" {
        let original = mapper_loop::expired_saved::capture_expired_output_on(&snapshot, actor, child)?;
        original.require_before_on(&snapshot, actor, child)?;
        return mapper_loop::verified_original_output_on(&snapshot, actor, child);
    }
    drop(snapshot);
    drop(db);
    let mut adapter = MapperControlledAdapter {
        context,
        actor,
        child,
        bounds,
        registered: vec!["evidence.lookup".into()],
        first: None,
        audit: None,
    };
    let limits = Limits {
        rounds: 2,
        tools: Some(1),
        calls_per_round: Some(1),
        argument_bytes: Some(64),
    };
    let result = crate::agent_runtime::controlled_executor::run(&mut adapter, &limits, Vec::new())
        .map_err(mapper_core_stop_text);
    if result.is_err() {
        // A final paid reply may outlive its original worker. Drop transport
        // ownership before the dedicated signed-expiry reader probes its inode.
        drop(adapter);
        let db = super::db::open(&context.db_path)?;
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        if crate::agent_runtime::multi_agent::attempts::current(&tx, actor, &child.assignment_id)?.state == "expired" {
            let original = mapper_loop::expired_saved::capture_expired_output_on(&tx, actor, child)?;
            original.require_before_on(&tx, actor, child)?;
            return mapper_loop::verified_original_output_on(&tx, actor, child);
        }
    }
    result
}
fn mapper_core_stop_text(stop: Stop) -> String {
    match stop {
        Stop::Cancelled => "mapper_loop_cancelled".into(),
        Stop::Limit(code) => code.into(),
        Stop::Unknown(error) | Stop::Refused(error) | Stop::Persistence(error) => error,
        Stop::PaidAbort {
            original,
            settlement,
        } => format!(
            "mapper_loop_paid_abort:{}:{}",
            mapper_core_stop_text(*original),
            mapper_core_stop_text(*settlement)
        ),
    }
}
