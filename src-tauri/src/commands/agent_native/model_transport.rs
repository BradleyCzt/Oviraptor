enum NativeModelRoundFailure {
    Model(AgentModelError),
    Outcome(Box<AgentTargetOutcome>),
}

impl NativeModelRoundFailure {
    fn outcome(value: AgentTargetOutcome) -> Self {
        Self::Outcome(Box::new(value))
    }
}

fn native_model_transport(
    context: &AgentRunContext,
    client: &AgentModelClient,
    messages: Vec<JsonValue>,
    specs: &[AgentToolSpec],
    round: i64,
) -> Result<crate::agent_runtime::model::gateway::ModelResponse, NativeModelRoundFailure> {
    let mut request = agent_model_request(messages, specs);
    let unbounded = native_model_unbounded_window(context)
        .map_err(|e|NativeModelRoundFailure::outcome(persistence_stop(context,e)))?;
    // Only newly declared Native5 unbounded windows gain this per-call bound.
    let bounded_client;
    let client = if unbounded {
        let mut profile = client.profile().clone();
        let output = profile.max_output_tokens.unwrap_or(8192);
        if output == 0 { return Err(NativeModelRoundFailure::outcome(persistence_stop(context,"budget_unbounded_model_estimate_invalid".into()))); }
        profile.max_output_tokens=Some(output);
        bounded_client=AgentModelClient::new(profile,specs);
        &bounded_client
    } else {client};
    let unbounded_estimate=if unbounded {
        let bytes=serde_json::to_vec(&json!({"messages":request.messages,"tools":request.tools.iter().map(|tool|tool.as_function_spec()).collect::<Vec<_>>()}))
            .map_err(|e|NativeModelRoundFailure::outcome(persistence_stop(context,e.to_string())))?.len();
        Some(i64::try_from(bytes).ok().and_then(|n|i64::try_from(client.profile().max_output_tokens.unwrap()).ok().and_then(|o|n.checked_add(o)))
            .filter(|n|*n>0).ok_or_else(||NativeModelRoundFailure::outcome(persistence_stop(context,"budget_unbounded_model_estimate_invalid".into())))?)
    } else {None};
    let request_hash=crate::agent_runtime::store::stable_hash(&serde_json::json!({"messages":request.messages,
        "tools":request.tools.iter().map(|tool|tool.as_function_spec()).collect::<Vec<_>>(),
        "temperature":request.temperature,"model":client.profile().model,"endpoint":client.profile().endpoint,
        "maxOutputTokens":client.profile().max_output_tokens,"providerOptions":client.profile().bounded_wire_options(),"schemaHash":client.schema_hash()}).to_string());
    match native_root_model_transport(context, client, &request, round, &request_hash) {
        Ok(Some(result)) => return result,
        Ok(None) => {}
        Err(error) => {
            return Err(NativeModelRoundFailure::outcome(persistence_stop(
                context, error,
            )))
        }
    }
    let admitted = (|| {
        native_model_budget_preflight(context)?;
        native_coordinator_budget_consumption_before_action(context)?;
        native_coordinator_http_budget_consumption_before_action(context)?;
        if unbounded {
            native_model_budget_admission_with_estimate(context, round, request_hash, unbounded_estimate)
        } else {
            native_model_budget_admission(context, round, request_hash)
        }
    })();
    let admission = match admitted {
        Ok(admission) => admission,
        Err(reason) if reason == "budget_model_requests_exhausted" => {
            return Err(NativeModelRoundFailure::outcome(
                AgentTargetOutcome::Limited(AgentStop::new(
                    AGENT_STOP_HARD_REQUESTS,
                    "原子账本中的本执行分支模型请求预留已用尽，停止新请求",
                )),
            ))
        }
        Err(reason) => {
            let mapped = agent_http_claim_error(reason.clone());
            return Err(NativeModelRoundFailure::outcome(
                agent_tool_terminal_outcome(&mapped)
                    .unwrap_or_else(|| persistence_stop(context, reason)),
            ));
        }
    };
    let mut cancel =
        agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number);
    if let Some(admission) = &admission {
        request.request_timeout = Some(admission.remaining);
        let deadline = Instant::now() + admission.remaining;
        let scoped_cancel = cancel;
        let owned = context.clone();
        let capability = admission.capability.clone();
        cancel = CancelToken::from_checker(move || {
            scoped_cancel.is_cancelled()
                || Instant::now() >= deadline
                || agent_authorize_tool(&owned, &capability).is_err()
        });
    }
    let result = if let Some(admission) = &admission {
        match client.complete_once_observed(&request, &cancel) {
            Ok(turn) => {
                let unresolved =
                    native_model_terminal_receipt(context, admission, "received", Some(&turn), "")
                        .map_err(|e| {
                            NativeModelRoundFailure::outcome(persistence_stop(context, e))
                        })?;
                if !turn.usage_reported || unresolved {
                    return Err(NativeModelRoundFailure::outcome(AgentTargetOutcome::Incomplete(AgentStop::new(
                        terminal_code::REQUEST_RECONCILIATION_REQUIRED,"模型计费用量缺失或超过原预留，费用回执已保留、原估算保持未决；停止后续工具和模型派发，须先核对费用"))));
                }
                // A known bill is a durable fact, never permission to consume
                // model output after the original worker loses its grant.
                if agent_authorize_tool(context, &admission.capability).is_err() {
                    return Err(NativeModelRoundFailure::outcome(
                        AgentTargetOutcome::resume_incompatible(
                            "原模型账单已保存；此执行分支的权限已失效，停止使用该响应及派发后续动作",
                        ),
                    ));
                }
                Ok(turn)
            }
            Err(crate::agent_runtime::model::OneShotFailure::BeforeTransport(error)) => {
                native_model_terminal_receipt(context, admission, "unsent", None, error.code())
                    .map_err(|e| NativeModelRoundFailure::outcome(persistence_stop(context, e)))?;
                Err(error)
            }
            Err(crate::agent_runtime::model::OneShotFailure::TransportOutcomeUnknown(error)) => {
                note_transport_cancel(context, client);
                if let Err(persist) = native_model_terminal_receipt(
                    context,
                    admission,
                    "uncertain",
                    None,
                    error.code(),
                ) {
                    return Err(NativeModelRoundFailure::outcome(persistence_stop(
                        context,
                        format!(
                            "web_model_unknown_cost_record_failed:{persist}; cause={}",
                            error.code()
                        ),
                    )));
                }
                return Err(NativeModelRoundFailure::outcome(AgentTargetOutcome::Incomplete(AgentStop::new(terminal_code::REQUEST_RECONCILIATION_REQUIRED,
                        format!("模型调用结果未知，已保留原请求成本并停止此执行分支；不自动重试、退款或继续派发，请先核对。原始失败：{}（{}）",
                            crate::agent_runtime::secrets::redact_text_with(&error.detail(),None),error.code())))));
            }
        }
    } else {
        client.complete(&request, &cancel)
    };
    result.map_err(NativeModelRoundFailure::Model)
}
