// Web policy adapter for the shared original model/tool/observation controller.
use crate::agent_runtime::controlled_executor::{
    Adapter as WebControlledAdapter, RawToolCall as WebControlledRawToolCall,
    Received as WebControlledReceived, Stop as WebControlledStop,
    TextPolicy as WebControlledTextPolicy, ToolEffect as WebControlledToolEffect,
    ToolPolicy as WebControlledToolPolicy,
};
struct WebExecutorRoundPermit {
    messages: Vec<JsonValue>,
    directives: HumanDirectiveContext,
}
struct WebExecutorPaidResponse {
    turn: crate::agent_runtime::model::gateway::ModelResponse,
    directives: HumanDirectiveContext,
}
/// DESIGN §5.5 / §8.6：主管把驳回四字段投影给用户之后，`root_human_directive_deferred`
/// 才允许本轮继续而不是打成 incomplete。缺少 run_id、没有同质任务或读取失败一律 false，
/// 因此 legacy 路径与任何异常都退回原有的 incomplete 行为，绝不静默放过。
pub(crate) fn human_deferral_answered_by_reject(
    db_path: &std::path::Path,
    root_run_id: &str,
) -> bool {
    if root_run_id.is_empty() {
        return false;
    }
    let projected = (|| -> Result<bool, String> {
        let connection = crate::db::open(db_path)?;
        crate::agent_runtime::multi_agent::homogeneous::store::reject_feedback_projected(
            &connection,
            root_run_id,
        )
    })();
    matches!(projected, Ok(true))
}

struct WebExecutorToolPermit {
    invocation_id: String,
    row: Option<i64>,
    target_before: usize,
}
struct WebExecutorOriginalTool {
    result: ToolExecutionResult,
    permit: WebExecutorToolPermit,
}
struct NativeWebExecutorAdapter<'a> {
    context: &'a AgentRunContext,
    client: AgentModelClient,
    specs: Vec<AgentToolSpec>,
    registered: Vec<String>,
    state: NativeAgentState,
    runtime: AgentToolRuntime,
    queue: Vec<String>,
    history: Vec<JsonValue>,
    attempt_round_base: i64,
    finished: bool,
    stop_outcome: Option<AgentTargetOutcome>,
}
impl NativeWebExecutorAdapter<'_> {
    fn retain_stop(&mut self, outcome: AgentTargetOutcome) -> WebControlledStop {
        let stop = match &outcome {
            AgentTargetOutcome::Cancelled => WebControlledStop::Cancelled,
            AgentTargetOutcome::Incomplete(original)
                if original.code == terminal_code::REQUEST_RECONCILIATION_REQUIRED =>
            {
                WebControlledStop::Unknown(original.reason.clone())
            }
            _ => WebControlledStop::Refused("native_web_original_stop".into()),
        };
        self.stop_outcome = Some(outcome);
        stop
    }
    fn prepare_web_round(&mut self) -> Result<WebExecutorRoundPermit, Box<AgentTargetOutcome>> {
        let context = self.context;
        let plan = &context.execution_plan;
        let state = &mut self.state;
        let runtime = &mut self.runtime;
        let queue = &mut self.queue;
        let history = &mut self.history;
        let client = &self.client;
        // An old worker may not checkpoint over the new attempt's shared row.
        if !native_web_attempt_current(&context.db_path, &context.scan_id, context.attempt_number) {
            return Err(Box::new(AgentTargetOutcome::Cancelled));
        }
        if sentinel_scan_pause_requested(&context.db_path, &context.scan_id) {
            // A pause must stay resumable: the ledger is written, but no terminal
            // reason is recorded, so the next attempt keeps its spent budget.
            if let Err(error) = native_commit(context, state, runtime, queue) {
                return Err(Box::new(persistence_stop(context, error)));
            }
            return Err(Box::new(AgentTargetOutcome::Cancelled));
        }
        if !sentinel_scan_is_active(&context.db_path, &context.scan_id) {
            state.terminal_reason = AGENT_STOP_USER_CANCELLED.to_string();
            if let Err(error) = native_commit(context, state, runtime, queue) {
                return Err(Box::new(persistence_stop(context, error)));
            }
            return Err(Box::new(AgentTargetOutcome::Cancelled));
        }
        if let Err(error) = native_web_require_determinate(context) {
            return Err(Box::new(
                agent_tool_terminal_outcome(&agent_http_claim_error(error.clone()))
                    .unwrap_or_else(|| persistence_stop(context, error)),
            ));
        }
        if plan.hard_model_requests > 0
            && agent_run_requests(context, state) >= plan.hard_model_requests
            && state.budget_extensions == 0
            && !queue.is_empty()
        {
            state.budget_extensions = 1;
            state.last_expansion_reason = "队列未完成，延长一次同等模型预算".to_string();
        }
        if state.turns >= plan.max_turns.max(1) {
            return Err(Box::new(native_exit(
                context,
                state,
                runtime,
                queue,
                "达到计划内最大轮数",
                AGENT_STOP_DERIVED,
                true,
            )));
        }
        if let Some((code, reason)) = agent_budget_stop(context, plan, state, runtime) {
            return Err(Box::new(native_exit(
                context, state, runtime, queue, &reason, code, true,
            )));
        }
        let mut directives = match take_human_directives(context) {
            Ok(directives) => directives,
            Err(error) => return Err(Box::new(persistence_stop(context, error))),
        };
        if let Err(error) = apply_assessed_human_queue_actions(context, &mut directives, queue) {
            append_runner_log(&context.log_path, &format!("原人工确认监督停止：{error}"));
            if matches!(
                error.as_str(),
                "child_budget_reservation_exceeded_or_stale"
                    | "budget_model_requests_exhausted"
                    | "budget_hard_limit_exceeded"
            ) {
                // No human action is authorized without a paid assessment.
                // Defer it under the original parent, then use the existing Web grant.
                if let Err(reason) = defer_human_directives_for_capacity(context, &mut directives) {
                    return Err(Box::new(persistence_stop(context, reason)));
                }
            } else if error == "root_human_directive_deferred"
                && human_deferral_answered_by_reject(
                    &context.db_path,
                    context
                        .run
                        .as_ref()
                        .map(|run| run.run_id.as_str())
                        .unwrap_or_default(),
                )
            {
                append_runner_log(
                    &context.log_path,
                    "原人工引导已由主管驳回并回用户，本轮按原计划继续（不中断任务）",
                );
            } else {
                return Err(Box::new(if error == "root_human_directive_deferred" {
                    AgentTargetOutcome::incomplete(error)
                } else {
                    multi_agent_bootstrap_outcome(&error)
                }));
            }
        };
        let proposal_messages = match apply_human_proposal_actions(context, &mut directives) {
            Ok(messages) => messages,
            Err(error)
                if matches!(
                    error.as_str(),
                    "budget_indeterminate_requires_reconciliation"
                        | "budget_history_requires_reconciliation"
                ) =>
            {
                // Already published original invoices remain intact. Unknown
                // Root costs prohibit proposal work and the next model round.
                return Err(Box::new(
                    agent_tool_terminal_outcome(&agent_http_claim_error(error.clone()))
                        .unwrap_or_else(|| persistence_stop(context, error)),
                ));
            }
            Err(error) => return Err(Box::new(persistence_stop(context, error))),
        };
        history[1] = agent_state_block(context, runtime, state, queue);
        // Rehydrate accepted instructions, including on resume; never append
        // duplicate copies to history or let compaction rewrite their meaning.
        let mut round_history = history.clone();
        round_history.extend(directives.messages());
        round_history.extend(proposal_messages);
        let (messages, _compacted) =
            agent_compact_messages(&round_history, client.profile().request_budget_bytes());
        Ok(WebExecutorRoundPermit {
            messages,
            directives,
        })
    }
}
impl WebControlledAdapter for NativeWebExecutorAdapter<'_> {
    type RoundPermit = WebExecutorRoundPermit;
    type PaidResponse = WebExecutorPaidResponse;
    type ToolPermit = WebExecutorToolPermit;
    type ToolOriginal = WebExecutorOriginalTool;
    type Audit = ();
    type Output = AgentTargetOutcome;
    fn registered_tools(&self) -> &[String] {
        &self.registered
    }
    fn check_continuation(&mut self) -> Result<(), WebControlledStop> {
        if !native_web_attempt_current(
            &self.context.db_path,
            &self.context.scan_id,
            self.context.attempt_number,
        ) {
            return Err(self.retain_stop(AgentTargetOutcome::Cancelled));
        }
        let context = self.context;
        if sentinel_scan_pause_requested(&context.db_path, &context.scan_id) {
            if let Err(error) = native_commit(context, &mut self.state, &self.runtime, &self.queue)
            {
                return Err(self.retain_stop(persistence_stop(context, error)));
            }
            return Err(self.retain_stop(AgentTargetOutcome::Cancelled));
        }
        if !sentinel_scan_is_active(&context.db_path, &context.scan_id) {
            self.state.terminal_reason = AGENT_STOP_USER_CANCELLED.to_string();
            if let Err(error) = native_commit(context, &mut self.state, &self.runtime, &self.queue)
            {
                return Err(self.retain_stop(persistence_stop(context, error)));
            }
            return Err(self.retain_stop(AgentTargetOutcome::Cancelled));
        }
        let live = (|| -> Result<(), String> {
            native_web_require_determinate(context)?;
            let Some(run) = &context.run else {
                return Ok(());
            };
            let db = db::open(&context.db_path)?;
            let tx =
                rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Deferred)
                    .map_err(|e| e.to_string())?;
            if let Some((actor, assignment)) =
                crate::agent_runtime::multi_agent::budget::target::child_owner(&tx, &run.run_id)?
            {
                crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(&tx, &actor)?;
                crate::agent_runtime::multi_agent::lease::require_executable_coordinator(
                    &tx, &actor,
                )?;
                crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
                    &tx,
                    &actor.root_run_id,
                )?
                .require_original_coordinator(&tx, &actor)?;
                // The original execution owner is held by run_native_agent for
                // this whole loop. Its entry-only no-history guard must not
                // reject the loop's own already-paid preceding model rounds.
                let capability =
                    native_model_authorized_capability_on(&tx, context, &actor, &assignment)?;
                native_model_require_authority_on(&tx, context, &capability)?;
                crate::agent_runtime::multi_agent::budget::receipts::original_owner(
                    &tx,
                    &run.run_id,
                    &actor,
                    &assignment,
                )?
                .verify(&tx)?;
            }
            Ok(())
        })();
        if let Err(error) = live {
            let outcome = agent_tool_terminal_outcome(&agent_http_claim_error(error.clone()))
                .unwrap_or_else(|| persistence_stop(context, error));
            return Err(self.retain_stop(outcome));
        }
        Ok(())
    }
    fn before_round(&mut self, _round: usize) -> Result<Self::RoundPermit, WebControlledStop> {
        self.prepare_web_round()
            .map_err(|outcome| self.retain_stop(*outcome))
    }
    fn model_once(
        &mut self,
        permit: Self::RoundPermit,
        _history: &[JsonValue],
    ) -> Result<WebControlledReceived<Self::PaidResponse>, WebControlledStop> {
        let turn = match native_model_transport(
            self.context,
            &self.client,
            permit.messages,
            &self.specs,
            self.state.turns - self.attempt_round_base + 1,
        ) {
            Ok(turn) => turn,
            Err(NativeModelRoundFailure::Outcome(outcome)) => {
                return Err(self.retain_stop(*outcome))
            }
            Err(NativeModelRoundFailure::Model(error)) => {
                let outcome = native_model_error(
                    self.context,
                    &mut self.state,
                    &self.runtime,
                    &self.queue,
                    &self.client,
                    error,
                );
                return Err(self.retain_stop(outcome));
            }
        };
        let calls = turn
            .deduplicated_tool_calls()
            .into_iter()
            .map(|call| WebControlledRawToolCall {
                id: call.id,
                name: call.name,
                arguments: call.arguments.to_string(),
            })
            .collect();
        Ok(WebControlledReceived {
            text: turn.text.clone(),
            calls,
            paid: WebExecutorPaidResponse {
                turn,
                directives: permit.directives,
            },
        })
    }
    fn record_paid_response(
        &mut self,
        paid: &Self::PaidResponse,
        _round: usize,
    ) -> Result<(), WebControlledStop> {
        if let Err(error) = native_publish_model_round(
            self.context,
            &self.client,
            &paid.turn,
            &mut self.state,
            &paid.directives,
        ) {
            return Err(self.retain_stop(persistence_stop(self.context, error)));
        }
        if let Err(error) = native_web_require_determinate(self.context) {
            let outcome = agent_tool_terminal_outcome(&agent_http_claim_error(error.clone()))
                .unwrap_or_else(|| persistence_stop(self.context, error));
            return Err(self.retain_stop(outcome));
        }
        Ok(())
    }
    fn validate_arguments(&self, _name: &str, _arguments: &JsonValue) -> Result<(), String> {
        // The original Tool Broker compiles/checks its exact schema and audits
        // deterministic errors under the same original invocation as execution.
        Ok(())
    }
    fn before_tool(
        &mut self,
        _paid: &Self::PaidResponse,
        _id: &str,
        name: &str,
        arguments: &JsonValue,
    ) -> Result<Self::ToolPermit, WebControlledStop> {
        let invocation_id = self.runtime.begin_invocation(self.context.attempt_number);
        let row = match &self.context.run {
            Some(run) => match run.begin_tool(name, arguments, &invocation_id) {
                Ok(row) => Some(row),
                Err(error) => return Err(self.retain_stop(persistence_stop(self.context, error))),
            },
            None => None,
        };
        Ok(WebExecutorToolPermit {
            invocation_id,
            row,
            target_before: self.runtime.target_requests,
        })
    }
    fn execute_tool(
        &mut self,
        permit: Self::ToolPermit,
        name: &str,
        arguments: &JsonValue,
    ) -> Result<WebControlledToolEffect<Self::ToolOriginal>, WebControlledStop> {
        let mut result = agent_execute_tool_at(
            self.context,
            &mut self.runtime,
            name,
            arguments,
            &permit.invocation_id,
        );
        if name == "finish_target" {
            if let Some(rejected) = agent_reject_premature_finish(
                self.context,
                &self.context.execution_plan,
                &self.state,
                &mut self.runtime,
                &self.queue,
                &result.model_view,
            ) {
                result.model_view = rejected;
            }
        }
        // Keep processing the original paid group before final Web queue and
        // coverage policy. A finish proposal is not an early group exit.
        if name == "finish_target"
            && result
                .model_view
                .get("accepted")
                .and_then(JsonValue::as_bool)
                == Some(true)
        {
            self.finished = true;
        }
        let observation = json!({"modelView":result.model_view,"artifactId":result.artifact_id});
        Ok(WebControlledToolEffect {
            original: WebExecutorOriginalTool { result, permit },
            observation,
            policy: WebControlledToolPolicy::Continue,
        })
    }
    fn audit_tool(
        &mut self,
        _paid: &Self::PaidResponse,
        _id: &str,
        name: &str,
        arguments: &JsonValue,
        effect: &WebControlledToolEffect<Self::ToolOriginal>,
    ) -> Result<(), WebControlledStop> {
        let original = &effect.original;
        if let Some(run) = &self.context.run {
            if let Err(error) = run.finish_tool(
                original.permit.row,
                name,
                arguments,
                &original.result.model_view,
                &original.result.invocation_id,
                self.runtime
                    .target_requests
                    .saturating_sub(original.permit.target_before),
            ) {
                return Err(self.retain_stop(persistence_stop(self.context, error)));
            }
        }
        if let Some(outcome) = self
            .runtime
            .target_request_stop
            .as_ref()
            .and_then(agent_tool_terminal_outcome)
            .or_else(|| agent_tool_terminal_outcome(&original.result.model_view))
        {
            append_runner_log(&self.context.log_path, &outcome.detail());
            return Err(self.retain_stop(outcome));
        }
        Ok(())
    }
    fn audit_rejection(
        &mut self,
        _paid: &Self::PaidResponse,
        _id: &str,
        name: &str,
        raw_arguments: &str,
        rejection: &str,
    ) -> Result<(Self::Audit, JsonValue), WebControlledStop> {
        let arguments: JsonValue = serde_json::from_str(raw_arguments)
            .unwrap_or_else(|_| json!({"__parseError":agent_text_truncated(raw_arguments,2_000)}));
        let invocation_id = self.runtime.begin_invocation(self.context.attempt_number);
        let result = match agent_validate_arguments(name, &arguments) {
            Err(original) => {
                self.runtime.note_argument_error(&original.fingerprint);
                json!({"error":original.message,"code":"invalid_arguments","schemaErrors":original.errors,"errorFingerprint":original.fingerprint})
            }
            Ok(()) => json!({"code":rejection,"executed":false}),
        };
        if let Some(run) = &self.context.run {
            let row = run
                .begin_tool(name, &arguments, &invocation_id)
                .map_err(WebControlledStop::Persistence)?;
            run.finish_tool(Some(row), name, &arguments, &result, &invocation_id, 0)
                .map_err(WebControlledStop::Persistence)?;
        }
        Ok(((), result))
    }
    fn tool_message(&self, id: &str, name: &str, observation: &JsonValue) -> JsonValue {
        if let Some(model_view) = observation.get("modelView") {
            json!({"role":"tool","tool_call_id":id,"name":name,"artifactId":observation["artifactId"],"content":agent_text_truncated(&model_view.to_string(),24_000)})
        } else {
            json!({"role":"tool","tool_call_id":id,"name":name,"content":agent_text_truncated(&observation.to_string(),24_000)})
        }
    }
    fn checkpoint(
        &mut self,
        history: &[JsonValue],
        _audits: &[()],
    ) -> Result<(), WebControlledStop> {
        self.history = history.to_vec();
        Ok(())
    }

    fn text_policy(
        &mut self,
        _paid: &Self::PaidResponse,
        _text: &str,
    ) -> Result<WebControlledTextPolicy, WebControlledStop> {
        self.state.no_progress_streak += 1;
        if let Some((code, reason)) = agent_text_only_stop(
            self.context,
            &self.context.execution_plan,
            &mut self.state,
            &self.runtime,
        ) {
            let outcome = native_exit(
                self.context,
                &mut self.state,
                &self.runtime,
                &self.queue,
                &reason,
                code,
                true,
            );
            return Err(self.retain_stop(outcome));
        }
        let mut messages = vec![
            json!({"role":"user","content":"不要只用文字结束。请调用工具继续处理 pendingQueue 和未覆盖族；预算未耗尽时不要结束。"}),
        ];
        if self.state.last_expansion_reason == "连续无进展，先检查队列再继续"
            && self.state.no_progress_streak == 0
        {
            messages.push(json!({"role":"user","content":"检查：上一轮没有新证据。请从 pendingQueue 和未覆盖族里继续下一项，不要重复已完成的请求。"}));
        }
        if let Err(error) = native_commit(self.context, &mut self.state, &self.runtime, &self.queue)
        {
            return Err(self.retain_stop(persistence_stop(self.context, error)));
        }
        Ok(WebControlledTextPolicy::Continue(messages))
    }
    fn after_tools(
        &mut self,
        _paid: &Self::PaidResponse,
        history: &[JsonValue],
    ) -> Result<WebControlledTextPolicy, WebControlledStop> {
        self.history = history.to_vec();
        self.after_web_tools()
            .map_err(|outcome| self.retain_stop(*outcome))
    }
    fn finish(
        &mut self,
        _paid: &Self::PaidResponse,
        _history: &[JsonValue],
    ) -> Result<AgentTargetOutcome, WebControlledStop> {
        self.stop_outcome.take().ok_or_else(|| {
            WebControlledStop::Persistence("native_web_finish_without_original_outcome".into())
        })
    }
}
impl NativeWebExecutorAdapter<'_> {
    fn after_web_tools(&mut self) -> Result<WebControlledTextPolicy, Box<AgentTargetOutcome>> {
        let context = self.context;
        let plan = &context.execution_plan;
        let state = &mut self.state;
        let runtime = &mut self.runtime;
        let queue = &mut self.queue;
        let history = &mut self.history;
        // Queue entries are consumed by what actually ran, never by what the
        // model merely passed as an argument.
        // Public-surface paths harvested from HTML/JS join the queue as api: entries
        // so classic onclick/ajax sites keep expanding beyond browser-observed XHR.
        for endpoint in runtime.drain_pending_apis() {
            let key = format!("api:{endpoint}");
            if !queue.contains(&key) {
                queue.push(key);
            }
        }
        queue.retain(|key| !runtime.queue_key_satisfied(context, key));
        if let Some(stop) = runtime.protected_stop.clone() {
            native_sync(context, state, runtime, queue);
            state.terminal_reason = stop.reason.clone();
            let commit = native_commit(context, state, runtime, queue)
                .and_then(|_| persist_agent_coverage(context, runtime, state, &stop.reason));
            if let Err(error) = commit {
                return Err(Box::new(persistence_stop(context, error)));
            }
            return Err(Box::new(AgentTargetOutcome::Limited(stop)));
        }
        let progress = runtime.take_progress();
        // §7: repeating an already-rejected call is not progress, even if some
        // other counter moved.
        if progress.advanced() && !runtime.repeats_argument_error() {
            state.no_progress_streak = 0;
            state.progress_signature = progress.signature();
            if agent_soft_budget_exceeded(plan, state) {
                state.last_expansion_reason =
                    format!("已超过软预算，但本轮仍有新增证据：{}", progress.signature());
            }
        } else {
            state.no_progress_streak += 1;
        }
        if self.finished {
            let gaps_remain = !queue.is_empty() || !runtime.uncovered_families.is_empty();
            return Err(Box::new(native_exit(
                context,
                state,
                runtime,
                queue,
                "模型已调用 finish_target 输出覆盖账本",
                AGENT_STOP_FINISH,
                gaps_remain,
            )));
        }
        let history_len = history.len();
        match agent_stall_action(state, plan) {
            StallAction::Stop => {
                return Err(Box::new(native_exit(
                    context,
                    state,
                    runtime,
                    queue,
                    "连续两次检查都没有新增端点、响应差异或结论",
                    AGENT_STOP_NO_PROGRESS,
                    true,
                )));
            }
            StallAction::Check => {
                let next = queue
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "没有剩余队列".to_string());
                history.push(serde_json::json!({
                    "role": "user",
                    "content": format!("检查：上一轮没有新证据。下一项是 {next}。请继续这一项，不要重复已完成的请求，也不要因为一条 404 结束目标。"),
                }));
            }
            StallAction::Continue => {}
        }
        if let Err(error) = native_commit(context, state, runtime, queue) {
            return Err(Box::new(persistence_stop(context, error)));
        }
        // The state block is replaced in place: appending one per round would
        // re-feed the model a growing, already-known snapshot.
        history[1] = agent_state_block(context, runtime, state, queue);
        Ok(WebControlledTextPolicy::Continue(
            history[history_len..].to_vec(),
        ))
    }
}
fn native_run_web_controlled_adapter(
    mut adapter: NativeWebExecutorAdapter<'_>,
) -> AgentTargetOutcome {
    let rounds = match usize::try_from(
        adapter
            .context
            .execution_plan
            .max_turns
            .max(1)
            .saturating_sub(adapter.state.turns)
            .max(1),
    ) {
        Ok(rounds) => rounds,
        Err(_) => {
            return persistence_stop(
                adapter.context,
                "native_web_original_round_limit_invalid".into(),
            )
        }
    };
    let history = adapter.history.clone();
    let limits = crate::agent_runtime::controlled_executor::Limits {
        rounds,
        tools: None,
        calls_per_round: None,
        argument_bytes: None,
    };
    match crate::agent_runtime::controlled_executor::run(&mut adapter, &limits, history) {
        Ok(outcome) => outcome,
        Err(stop) => {
            if let WebControlledStop::PaidAbort {
                original,
                settlement,
            } = &stop
            {
                return persistence_stop(adapter.context, format!("native_web_paid_abort_failed: original={original:?}; settlement={settlement:?}"));
            }
            if let Some(outcome) = adapter.stop_outcome.take() {
                return outcome;
            }
            match stop {
                WebControlledStop::Cancelled => AgentTargetOutcome::Cancelled,
                WebControlledStop::Limit("controlled_executor_round_limit") => {
                    match adapter.prepare_web_round() {
                        Err(outcome) => *outcome,
                        Ok(_) => persistence_stop(
                            adapter.context,
                            "native_web_round_limit_without_original_stop".into(),
                        ),
                    }
                }
                WebControlledStop::Limit(code) => persistence_stop(adapter.context, code.into()),
                WebControlledStop::Unknown(error)
                | WebControlledStop::Refused(error)
                | WebControlledStop::Persistence(error) => persistence_stop(adapter.context, error),
                WebControlledStop::PaidAbort { original, settlement } => persistence_stop(adapter.context, format!("native_web_paid_abort_failed: original={original:?}; settlement={settlement:?}")),
            }
        }
    }
}
