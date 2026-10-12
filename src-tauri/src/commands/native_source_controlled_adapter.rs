use crate::agent_runtime::controlled_executor::{
    Adapter as SourceControlledAdapter, RawToolCall as SourceControlledRawToolCall,
    Received as SourceControlledReceived, Stop as SourceControlledStop,
    TextPolicy as SourceControlledTextPolicy, ToolEffect as SourceControlledToolEffect,
    ToolPolicy as SourceControlledToolPolicy,
};
use crate::agent_runtime::multi_agent::source_rounds;

struct NativeSourceRoundPermit {
    start: source_rounds::Start,
    lifetime: Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
    number: i64,
}
struct NativeSourcePaidResponse {
    pending: source_rounds::PendingRound,
    receipt: source_rounds::Receipt,
    _lifetime: Option<crate::agent_runtime::execution_owner::NativeInvocationOwner>,
}
struct NativeSourceToolPermit {
    pending: source_rounds::PendingRound,
    index: i64,
}
struct NativeSourceControlledAdapter<'a, 'environment> {
    connection: &'a rusqlite::Connection,
    context: &'a SpecialistTransportContext<'environment>,
    lease: &'a crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &'a crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    profile: AgentModelProfile,
    tools: Vec<AgentToolSpec>,
    client: AgentModelClient,
    frozen: JsonValue,
    registered: Vec<String>,
    next_tool: usize,
    original_error: Option<String>,
}
impl NativeSourceControlledAdapter<'_, '_> {
    fn stop(&mut self, error: String) -> SourceControlledStop {
        self.original_error = Some(error.clone());
        SourceControlledStop::Refused(error)
    }
}
impl SourceControlledAdapter for NativeSourceControlledAdapter<'_, '_> {
    type RoundPermit = NativeSourceRoundPermit;
    type PaidResponse = NativeSourcePaidResponse;
    type ToolPermit = NativeSourceToolPermit;
    type ToolOriginal = JsonValue;
    type Audit = ();
    type Output = JsonValue;
    fn registered_tools(&self) -> &[String] {
        &self.registered
    }
    fn check_continuation(&mut self) -> Result<(), SourceControlledStop> {
        let result = (|| {
            let tx = rusqlite::Transaction::new_unchecked(
                self.connection,
                rusqlite::TransactionBehavior::Deferred,
            )
            .map_err(|e| e.to_string())?;
            authorize_source_tool_phase(&tx, self.context, self.lease, self.child)
        })();
        result.map_err(|e| self.stop(e))
    }
    fn before_round(&mut self, round: usize) -> Result<Self::RoundPermit, SourceControlledStop> {
        let result = (|| -> Result<NativeSourceRoundPermit, String> {
            let number = i64::try_from(round).map_err(|_| "source_tool_phase_round_invalid")?;
            let check = |tx: &rusqlite::Connection| {
                authorize_source_tool_phase(tx, self.context, self.lease, self.child)
            };
            if number > 1 {
                self.frozen = source_rounds::prepare_next_authorized(
                    self.connection,
                    self.lease,
                    self.child,
                    number,
                    &self.frozen,
                    check,
                )?;
            }
            let (required, _) = source_assessment_budget(
                &[json!({"messages":self.frozen["messages"],"tools":self.frozen["tools"]})],
                &self.profile,
            )?;
            let (start, lifetime) = source_rounds::start_for_transport(
                self.connection,
                self.lease,
                self.child,
                number,
                &self.frozen,
                required,
                check,
            )?;
            self.next_tool = 0;
            Ok(NativeSourceRoundPermit {
                start,
                lifetime,
                number,
            })
        })();
        result.map_err(|e| self.stop(e))
    }
    fn model_once(
        &mut self,
        permit: Self::RoundPermit,
        _history: &[JsonValue],
    ) -> Result<SourceControlledReceived<Self::PaidResponse>, SourceControlledStop> {
        let result = (|| -> Result<SourceControlledReceived<NativeSourcePaidResponse>, String> {
            let connection = self.connection;
            let context = self.context;
            let lease = self.lease;
            let child = self.child;
            let client = &self.client;
            let tools = &self.tools;
            let number = permit.number;
            let start = permit.start;
            let (pending, receipt, model_log) =
                match start {
                    source_rounds::Start::Received(pending, receipt) => (pending, receipt, None),
                    source_rounds::Start::Dispatch(pending) => {
                        let model_log = crate::agent_runtime::model::diagnostics::ModelLog::child(
                            context.db_path,
                            lease,
                            child,
                            number,
                            true,
                        );
                        let messages = pending.request()["messages"]
                            .as_array()
                            .ok_or("source_tool_phase_messages_invalid")?
                            .clone();
                        let mut request = agent_model_request(messages, tools);
                        let remaining =
                            crate::agent_runtime::multi_agent::budget::clock::remaining(
                                connection,
                                &lease.root_run_id,
                            )?;
                        let shared_deadline = std::time::Instant::now() + remaining;
                        let deadline = context
                            .deadline
                            .map_or(shared_deadline, |d| d.min(shared_deadline));
                        request.request_timeout =
                            Some(deadline.saturating_duration_since(std::time::Instant::now()));
                        let scoped_cancel = source_tool_model_cancel_token(context, lease, child);
                        let cancel = CancelToken::from_checker(move || {
                            scoped_cancel.is_cancelled() || std::time::Instant::now() >= deadline
                        });
                        let result = match client.complete_once_observed_with_lifecycle(
                    &request,
                    &cancel,
                    model_log.observer(),
                ) {
                    Ok(result) => result,
                    Err(crate::agent_runtime::model::OneShotFailure::BeforeTransport(error)) => {
                        source_rounds::record_not_sent(connection, &pending, error.code())?;
                        return Err(format!("source_tool_model_failed:{}", error.code()));
                    }
                    Err(crate::agent_runtime::model::OneShotFailure::TransportOutcomeUnknown(
                        error,
                    )) => {
                        source_rounds::record_uncertain(connection, &pending, error.code())?;
                        return Err(format!("source_tool_model_failed:{}", error.code()));
                    }
                };
                        client.record_usage_line(context.usage_dir, &result);
                        let receipt = source_rounds::record_received(connection, &pending, &result);
                        model_log.cost_saved();
                        (pending, receipt?, Some(model_log))
                    }
                };
            context.require_supervision(lease)?;
            if receipt.response["rejection"] != "" {
                return Err(format!(
                    "source_tool_response_rejected:{}",
                    receipt.response["rejection"]
                ));
            }
            let calls = receipt.response["toolCalls"]
                .as_array()
                .ok_or("source_tool_phase_response_invalid")?
                .iter()
                .map(|call| {
                    Ok(SourceControlledRawToolCall {
                        id: call["id"]
                            .as_str()
                            .ok_or("source_round_tool_invalid")?
                            .into(),
                        name: call["name"]
                            .as_str()
                            .ok_or("source_round_tool_invalid")?
                            .into(),
                        arguments: call["arguments"].to_string(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            if let Some(log) = model_log {
                log.finish(true);
            }
            let text = receipt.response["text"]
                .as_str()
                .ok_or("source_tool_phase_response_invalid")?
                .to_string();
            Ok(SourceControlledReceived {
                paid: NativeSourcePaidResponse {
                    pending,
                    receipt,
                    _lifetime: permit.lifetime,
                },
                text,
                calls,
            })
        })();
        result.map_err(|e| self.stop(e))
    }
    fn record_paid_response(
        &mut self,
        _paid: &Self::PaidResponse,
        _round: usize,
    ) -> Result<(), SourceControlledStop> {
        Ok(())
    }
    fn text_policy(
        &mut self,
        paid: &Self::PaidResponse,
        _text: &str,
    ) -> Result<SourceControlledTextPolicy, SourceControlledStop> {
        // The original receipt reader refuses prose-only continuation; no new prompt or retry.
        let result = source_rounds::continuation(self.connection, &paid.pending);
        match result {
            Ok(_) => Err(self.stop("source_tool_phase_response_invalid".into())),
            Err(error) => Err(self.stop(error)),
        }
    }
    fn validate_arguments(&self, _name: &str, _arguments: &JsonValue) -> Result<(), String> {
        Ok(())
    }
    fn before_tool(
        &mut self,
        paid: &Self::PaidResponse,
        id: &str,
        name: &str,
        arguments: &JsonValue,
    ) -> Result<Self::ToolPermit, SourceControlledStop> {
        let calls = paid.receipt.response["toolCalls"].as_array();
        let original = calls.and_then(|calls| calls.get(self.next_tool));
        if original.is_none_or(|call| {
            call["id"] != id || call["name"] != name || call["arguments"] != *arguments
        }) {
            return Err(self.stop("source_round_tool_binding_changed".into()));
        }
        let index = i64::try_from(self.next_tool)
            .map_err(|_| self.stop("source_round_tool_index_invalid".into()))?;
        self.next_tool += 1;
        Ok(NativeSourceToolPermit {
            pending: paid.pending.clone(),
            index,
        })
    }
    fn execute_tool(
        &mut self,
        permit: Self::ToolPermit,
        name: &str,
        _arguments: &JsonValue,
    ) -> Result<SourceControlledToolEffect<Self::ToolOriginal>, SourceControlledStop> {
        let output = source_rounds::execute_tool(
            self.connection,
            &permit.pending,
            permit.index,
            |tx| authorize_source_tool_phase(tx, self.context, self.lease, self.child),
            |tx, name, args| source_tool_broker_call(tx, self.lease, self.child, name, args),
        )
        .map_err(|e| self.stop(e))?;
        Ok(SourceControlledToolEffect {
            original: output.clone(),
            observation: output,
            policy: if name == "assignment.finish" {
                SourceControlledToolPolicy::Complete
            } else {
                SourceControlledToolPolicy::Continue
            },
        })
    }
    fn audit_tool(
        &mut self,
        _paid: &Self::PaidResponse,
        _id: &str,
        _name: &str,
        _arguments: &JsonValue,
        _effect: &SourceControlledToolEffect<Self::ToolOriginal>,
    ) -> Result<(), SourceControlledStop> {
        // execute_tool already atomically committed the original receipt/event and local effect.
        Ok(())
    }
    fn audit_rejection(
        &mut self,
        _paid: &Self::PaidResponse,
        _id: &str,
        _name: &str,
        _raw_arguments: &str,
        _rejection: &str,
    ) -> Result<((), JsonValue), SourceControlledStop> {
        // record_received must have rejected this group first. Never manufacture a Source receipt.
        Err(self.stop("source_round_tool_binding_changed".into()))
    }
    fn checkpoint(
        &mut self,
        _history: &[JsonValue],
        _audits: &[()],
    ) -> Result<(), SourceControlledStop> {
        Ok(())
    }
    fn after_tools(
        &mut self,
        paid: &Self::PaidResponse,
        _history: &[JsonValue],
    ) -> Result<SourceControlledTextPolicy, SourceControlledStop> {
        self.frozen = source_rounds::continuation(self.connection, &paid.pending)
            .map_err(|e| self.stop(e))?;
        Ok(SourceControlledTextPolicy::Continue(Vec::new()))
    }
    fn finish(
        &mut self,
        paid: &Self::PaidResponse,
        _history: &[JsonValue],
    ) -> Result<JsonValue, SourceControlledStop> {
        complete_source_tool_assignment(
            self.connection,
            self.context,
            self.lease,
            self.child,
            &paid.pending,
        )
        .map_err(|e| self.stop(e))
    }
}
fn native_run_source_controlled_adapter(
    mut adapter: NativeSourceControlledAdapter<'_, '_>,
) -> Result<JsonValue, String> {
    let limits = crate::agent_runtime::controlled_executor::Limits {
        rounds: 3,
        tools: None,
        calls_per_round: Some(64),
        argument_bytes: Some(1_048_576),
    };
    match crate::agent_runtime::controlled_executor::run(&mut adapter, &limits, Vec::new()) {
        Ok(value) => Ok(value),
        Err(stop) => {
            if let SourceControlledStop::PaidAbort {
                original,
                settlement,
            } = &stop
            {
                return Err(format!(
                    "source_paid_abort_failed: original={original:?}; settlement={settlement:?}"
                ));
            }
            if let Some(error) = adapter.original_error.take() {
                return Err(error);
            }
            match stop {
                SourceControlledStop::Limit("controlled_executor_round_limit") => {
                    close_exhausted_source_tool_assignment(
                        adapter.connection,
                        adapter.context,
                        adapter.lease,
                        adapter.child,
                    )?;
                    Err("source_tool_phase_round_budget_exhausted_without_finish".into())
                }
                SourceControlledStop::Cancelled => Err("source_tool_phase_cancelled".into()),
                SourceControlledStop::Limit(code) => Err(code.into()),
                SourceControlledStop::Unknown(error)
                | SourceControlledStop::Refused(error)
                | SourceControlledStop::Persistence(error) => Err(error),
                SourceControlledStop::PaidAbort {
                    original,
                    settlement,
                } => Err(format!(
                    "source_paid_abort_failed: original={original:?}; settlement={settlement:?}"
                )),
            }
        }
    }
}
