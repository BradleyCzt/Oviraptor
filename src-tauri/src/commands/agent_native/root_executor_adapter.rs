use crate::agent_runtime::controlled_executor::{
    Adapter as RootControlledAdapter, RawToolCall as RootControlledRawToolCall,
    Received as RootControlledReceived, Stop as RootControlledStop,
    TextPolicy as RootControlledTextPolicy, ToolEffect as RootControlledToolEffect,
    ToolPolicy as RootControlledToolPolicy,
};
use crate::agent_runtime::multi_agent::budget::root::model::tick::local::{
    OriginalLocalCalls, OriginalLocalResult,
};
type NativeRootPaidResponse = std::rc::Rc<std::cell::RefCell<Option<NativeRootRoundState>>>;
struct NativeRootToolPermit {
    index: usize,
    paid: NativeRootPaidResponse,
}
struct NativeRootControlledAdapter<'a> {
    context: &'a AgentRunContext,
    actor: &'a crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&'a NativeCoordinatorFrame>,
    original_basis: JsonValue,
    first: Option<NativeCoordinatorRequest>,
    history: Vec<NativeCoordinatorTickReceipt>,
    registered: Vec<String>,
    calls_used: usize,
    next_tool: usize,
    local: Option<OriginalLocalCalls>,
    results: Vec<OriginalLocalResult>,
    original_error: Option<String>,
}
impl NativeRootControlledAdapter<'_> {
    fn stop(&mut self, error: String) -> RootControlledStop {
        self.original_error = Some(error.clone());
        RootControlledStop::Refused(error)
    }
    fn live(&self) -> Result<(), String> {
        let db = db::open(&self.context.db_path)?;
        let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Deferred)
            .map_err(|e| e.to_string())?;
        native_coordinator_tick_authority(&tx, self.context, self.actor)?;
        native_coordinator_verify_frame_basis(
            &tx,
            self.context,
            self.actor,
            self.frame,
            &self.original_basis,
        )?;
        native_coordinator_verify_local_history(&tx, &self.history)
    }
    fn original_calls(
        state: &NativeRootRoundState,
    ) -> Result<Vec<RootControlledRawToolCall>, String> {
        match &state.response {
            NativeRootOriginalResponse::Fresh { response, .. } => Ok(response
                .tool_calls
                .iter()
                .map(|call| RootControlledRawToolCall {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.to_string(),
                })
                .collect()),
            NativeRootOriginalResponse::Saved { decision, .. } => {
                let Some(local) = &decision.local_step else {
                    return Ok(Vec::new());
                };
                local.as_json()["calls"]
                    .as_array()
                    .ok_or("root_local_result_invalid")?
                    .iter()
                    .map(|call| {
                        Ok(RootControlledRawToolCall {
                            id: call["id"]
                                .as_str()
                                .ok_or("root_local_result_invalid")?
                                .into(),
                            name: call["name"]
                                .as_str()
                                .ok_or("root_local_result_invalid")?
                                .into(),
                            arguments: call["arguments"].to_string(),
                        })
                    })
                    .collect()
            }
        }
    }
    fn complete(
        &mut self,
        paid: &NativeRootPaidResponse,
    ) -> Result<NativeCoordinatorTickReceipt, String> {
        let token = {
            let borrowed = paid.borrow();
            let state = borrowed
                .as_ref()
                .ok_or("root_local_original_round_missing")?;
            match &state.response {
                NativeRootOriginalResponse::Fresh { tick, response } => {
                    if self.frame.is_some_and(|frame| frame.kind == NATIVE_HOMOGENEOUS_FRAME_KIND) {
                        // This paid reply uses CoordinatorDecision. Tick::received owns its
                        // parser and keeps the existing camelCase ledger projection intact.
                        None
                    } else if response.tool_calls.is_empty() {
                        Some(tick.original_semantic_without_local(response)?)
                    } else {
                        let original = self
                            .local
                            .take()
                            .ok_or("root_local_original_calls_missing")?;
                        let proof =
                            original.finalize_original(std::mem::take(&mut self.results))?;
                        Some(tick.original_semantic_with_local(response, proof)?)
                    }
                }
                NativeRootOriginalResponse::Saved { .. } => None,
            }
        };
        let mut borrowed = paid.borrow_mut();
        let state = borrowed
            .as_mut()
            .ok_or("root_local_original_round_missing")?;
        native_root_complete_original_on(
            self.context,
            self.actor,
            self.frame,
            state,
            &self.original_basis,
            &self.history,
            token,
        )
    }
}
impl RootControlledAdapter for NativeRootControlledAdapter<'_> {
    type RoundPermit = NativeCoordinatorRequest;
    type PaidResponse = NativeRootPaidResponse;
    type ToolPermit = NativeRootToolPermit;
    type ToolOriginal = ();
    type Audit = ();
    type Output = NativeCoordinatorTickReceipt;
    fn registered_tools(&self) -> &[String] {
        &self.registered
    }
    fn check_continuation(&mut self) -> Result<(), RootControlledStop> {
        self.live().map_err(|e| self.stop(e))
    }
    fn before_round(&mut self, round: usize) -> Result<Self::RoundPermit, RootControlledStop> {
        let result = (|| -> Result<NativeCoordinatorRequest, String> {
            let mut prepared = match self.first.take() {
                Some(first) => first,
                None => {
                    let mut next =
                        native_coordinator_request_for_frame(self.context, self.actor, self.frame)?;
                    if next.basis != self.original_basis {
                        return Err("root_tick_original_basis_changed".into());
                    }
                    for receipt in &self.history {
                        next.request.messages.extend(
                            receipt
                                .saved
                                .local_step
                                .as_ref()
                                .ok_or("root_local_original_history_changed")?
                                .messages(),
                        );
                    }
                    next
                }
            };
            let db = db::open(&self.context.db_path)?;
            let parents = self
                .history
                .iter()
                .map(|r| r.tick.local_parent_proof(&db, &r.saved, r.event_sequence))
                .collect::<Result<Vec<_>, _>>()?;
            prepared.basis["localParents"] = json!(parents);
            prepared.basis["localRound"] = json!(round - 1);
            prepared.basis["localCallsUsed"] = json!(self.calls_used);
            prepared.basis["localHistory"] = json!(self
                .history
                .iter()
                .map(|r| r
                    .saved
                    .local_step
                    .as_ref()
                    .expect("only original local receipts enter history")
                    .as_json())
                .collect::<Vec<_>>());
            prepared.fact["basisHash"] =
                crate::agent_runtime::store::stable_hash(&prepared.basis.to_string()).into();
            prepared.fact["messages"] = json!(prepared.request.messages);
            let bytes = serde_json::to_vec(
                &json!({"messages":prepared.request.messages,"tools":prepared.fact["tools"]}),
            )
            .map_err(|e| e.to_string())?
            .len();
            if bytes > 96 * 1024 {
                return Err("root_local_context_limit".into());
            }
            let output = prepared
                .client
                .profile()
                .max_output_tokens
                .ok_or("root_tick_output_invalid")?;
            prepared.estimate = i64::try_from(bytes)
                .ok()
                .and_then(|b| b.checked_add(i64::try_from(output).ok()?))
                .ok_or("root_tick_estimate_invalid")?;
            self.next_tool = 0;
            self.local = None;
            self.results.clear();
            Ok(prepared)
        })();
        result.map_err(|e| self.stop(e))
    }
    fn model_once(
        &mut self,
        permit: Self::RoundPermit,
        _history: &[JsonValue],
    ) -> Result<RootControlledReceived<Self::PaidResponse>, RootControlledStop> {
        let state = native_root_transport_original(
            self.context,
            self.actor,
            self.frame,
            permit,
            &self.original_basis,
            &self.history,
        )
        .map_err(|e| self.stop(e))?;
        let calls = if self.frame.is_some_and(|frame| frame.kind == NATIVE_HOMOGENEOUS_FRAME_KIND) {
            // Homogeneous decision rounds have no local-tool authority. Keep the
            // original provider response for billed refusal, expose no executable calls.
            Vec::new()
        } else { Self::original_calls(&state).map_err(|e| self.stop(e))? };
        let text = match &state.response {
            NativeRootOriginalResponse::Fresh { response, .. } => response.text.clone(),
            NativeRootOriginalResponse::Saved { .. } => String::new(),
        };
        Ok(RootControlledReceived {
            paid: std::rc::Rc::new(std::cell::RefCell::new(Some(state))),
            text,
            calls,
        })
    }
    fn record_paid_response(
        &mut self,
        paid: &Self::PaidResponse,
        _round: usize,
    ) -> Result<(), RootControlledStop> {
        if self.frame.is_some_and(|frame| frame.kind == NATIVE_HOMOGENEOUS_FRAME_KIND) {
            return Ok(());
        }
        let result = (|| -> Result<(), String> {
            let borrowed = paid.borrow();
            let state = borrowed
                .as_ref()
                .ok_or("root_local_original_round_missing")?;
            if let NativeRootOriginalResponse::Fresh { response, .. } = &state.response {
                if !response.tool_calls.is_empty() {
                    let phase = state.prepared.basis["phase"].as_str();
                    let denied = match phase {
                        Some("authorization-output") => {
                            Some("root_authorization_output_local_tool_denied")
                        }
                        Some("external-surface-output") => {
                            Some("root_external_surface_output_local_tool_denied")
                        }
                        Some("business-logic-output") => {
                            Some("root_business_logic_output_local_tool_denied")
                        }
                        Some("business-logic-input") => {
                            Some("root_business_logic_dispatch_local_tool_denied")
                        }
                        _ => None,
                    };
                    if let Some(code) = denied {
                        return Err(code.into());
                    }
                    self.local = OriginalLocalCalls::capture(response, &state.prepared.basis)?;
                }
            }
            Ok(())
        })();
        result.map_err(|e| self.stop(e))
    }
    fn validate_call_group(
        &mut self,
        _paid: &Self::PaidResponse,
        _calls: &[RootControlledRawToolCall],
    ) -> Result<(), RootControlledStop> {
        // The original opaque per-call executor validates ID before arguments
        // and size, preserving the original earliest error in mixed bad groups.
        Ok(())
    }
    fn abort_paid_response(
        &mut self,
        paid: &Self::PaidResponse,
        _stop: &RootControlledStop,
    ) -> Result<(), RootControlledStop> {
        let mut borrowed = paid.borrow_mut();
        let Some(state) = borrowed.as_mut() else {
            return Err(RootControlledStop::Persistence(
                "root_local_original_round_missing".into(),
            ));
        };
        match &state.financial_stage {
            NativeRootFinancialStage::FinancialCommitted => return Ok(()),
            NativeRootFinancialStage::FinancialAttemptFailed(error) => {
                return Err(RootControlledStop::Persistence(error.clone()));
            }
            NativeRootFinancialStage::Unsettled => {}
        }
        let token=match &state.response {
            NativeRootOriginalResponse::Saved{..}=>return Ok(()),
            NativeRootOriginalResponse::Fresh{tick,response}=>tick.original_semantic_failure(response,match &self.original_error {
                Some(error)=>crate::agent_runtime::multi_agent::budget::root::model::tick::OriginalSemanticFailure::LocalValidation(error.clone()),
                None=>crate::agent_runtime::multi_agent::budget::root::model::tick::OriginalSemanticFailure::Stopped,
            }),
        };
        let expected = self
            .original_error
            .as_deref()
            .unwrap_or("root_original_semantic_stopped")
            .to_string();
        match native_root_complete_original_on(
            self.context,
            self.actor,
            self.frame,
            state,
            &self.original_basis,
            &self.history,
            Some(token),
        ) {
            Err(error)
                if error == expected
                    && matches!(
                        state.financial_stage,
                        NativeRootFinancialStage::FinancialCommitted
                    ) =>
            {
                Ok(())
            }
            Err(error) => Err(RootControlledStop::Persistence(format!(
                "root_local_abort_original_fee_unconfirmed:{error}"
            ))),
            Ok(_) => Err(RootControlledStop::Persistence(
                "root_local_abort_published_unexpectedly".into(),
            )),
        }
    }
    fn text_policy(
        &mut self,
        _paid: &Self::PaidResponse,
        _text: &str,
    ) -> Result<RootControlledTextPolicy, RootControlledStop> {
        Ok(RootControlledTextPolicy::Complete)
    }
    fn validate_arguments(&self, _name: &str, _arguments: &JsonValue) -> Result<(), String> {
        Ok(())
    }
    fn before_tool(
        &mut self,
        paid: &Self::PaidResponse,
        _id: &str,
        _name: &str,
        _arguments: &JsonValue,
    ) -> Result<Self::ToolPermit, RootControlledStop> {
        let permit = NativeRootToolPermit {
            index: self.next_tool,
            paid: paid.clone(),
        };
        self.next_tool += 1;
        Ok(permit)
    }
    fn execute_tool(
        &mut self,
        permit: Self::ToolPermit,
        _name: &str,
        _arguments: &JsonValue,
    ) -> Result<RootControlledToolEffect<Self::ToolOriginal>, RootControlledStop> {
        let result = (|| -> Result<JsonValue, String> {
            if let Some(local) = &self.local {
                let result = local.execute_call(permit.index)?;
                let observation = result.observation().clone();
                self.results.push(result);
                Ok(observation)
            } else {
                let borrowed = permit.paid.borrow();
                let state = borrowed
                    .as_ref()
                    .ok_or("root_local_original_round_missing")?;
                let NativeRootOriginalResponse::Saved { decision, .. } = &state.response else {
                    return Err("root_local_original_calls_missing".into());
                };
                let step = decision
                    .local_step
                    .as_ref()
                    .ok_or("root_local_original_calls_missing")?
                    .as_json();
                let call = step["calls"]
                    .as_array()
                    .and_then(|calls| calls.get(permit.index))
                    .ok_or("root_local_tool_index_invalid")?;
                Ok(call["result"].clone())
            }
        })();
        let observation = result.map_err(|e| self.stop(e))?;
        Ok(RootControlledToolEffect {
            original: (),
            observation,
            policy: RootControlledToolPolicy::Continue,
        })
    }
    fn audit_tool(
        &mut self,
        _paid: &Self::PaidResponse,
        _id: &str,
        _name: &str,
        _arguments: &JsonValue,
        _effect: &RootControlledToolEffect<Self::ToolOriginal>,
    ) -> Result<(), RootControlledStop> {
        Ok(())
    }
    fn audit_rejection(
        &mut self,
        _paid: &Self::PaidResponse,
        _id: &str,
        _name: &str,
        _raw_arguments: &str,
        _rejection: &str,
    ) -> Result<((), JsonValue), RootControlledStop> {
        let result = self
            .local
            .as_ref()
            .ok_or("root_local_original_calls_missing")
            .map_err(str::to_string)
            .and_then(|local| local.execute_call(self.next_tool));
        match result {
            Err(error) => Err(self.stop(error)),
            Ok(_) => Err(self.stop("root_local_tool_denied".into())),
        }
    }
    fn checkpoint(
        &mut self,
        _history: &[JsonValue],
        _audits: &[()],
    ) -> Result<(), RootControlledStop> {
        Ok(())
    }
    fn after_tools(
        &mut self,
        paid: &Self::PaidResponse,
        _history: &[JsonValue],
    ) -> Result<RootControlledTextPolicy, RootControlledStop> {
        let receipt = self.complete(paid).map_err(|e| self.stop(e))?;
        let count = receipt
            .saved
            .local_step
            .as_ref()
            .ok_or_else(|| self.stop("root_local_original_calls_missing".into()))?
            .count();
        self.calls_used += count;
        if self.calls_used > 4 {
            return Err(self.stop("root_local_call_limit".into()));
        }
        self.history.push(receipt);
        if self.history.len() == 3 {
            return Err(self.stop("root_local_react_round_limit".into()));
        }
        Ok(RootControlledTextPolicy::Continue(Vec::new()))
    }
    fn finish(
        &mut self,
        paid: &Self::PaidResponse,
        _history: &[JsonValue],
    ) -> Result<Self::Output, RootControlledStop> {
        self.complete(paid).map_err(|e| self.stop(e))
    }
}
fn native_run_root_controlled_adapter(
    mut adapter: NativeRootControlledAdapter<'_>,
) -> Result<NativeCoordinatorTickReceipt, String> {
    let limits = crate::agent_runtime::controlled_executor::Limits {
        rounds: 3,
        tools: None,
        calls_per_round: None,
        argument_bytes: None,
    };
    match crate::agent_runtime::controlled_executor::run(&mut adapter, &limits, Vec::new()) {
        Ok(receipt) => Ok(receipt),
        Err(stop) => {
            if let RootControlledStop::PaidAbort {
                original,
                settlement,
            } = stop
            {
                return Err(format!(
                    "root_local_paid_abort:{}:{}",
                    native_root_core_stop_text(*original),
                    native_root_core_stop_text(*settlement)
                ));
            }
            adapter
                .original_error
                .take()
                .map_or_else(|| Err(native_root_core_stop_text(stop)), Err)
        }
    }
}
fn native_root_core_stop_text(stop: RootControlledStop) -> String {
    match stop {
        RootControlledStop::Cancelled => "root_local_cancelled".into(),
        RootControlledStop::Limit(code) => code.into(),
        RootControlledStop::Unknown(error)
        | RootControlledStop::Refused(error)
        | RootControlledStop::Persistence(error) => error,
        RootControlledStop::PaidAbort {
            original,
            settlement,
        } => format!(
            "root_local_paid_abort:{}:{}",
            native_root_core_stop_text(*original),
            native_root_core_stop_text(*settlement)
        ),
    }
}
