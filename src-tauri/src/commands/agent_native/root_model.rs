// Standalone Native model calls use a typed Root budget owner, not a child grant.
fn native_single_model_output_cap(profile: &AgentModelProfile) -> u64 {
    profile.max_output_tokens.unwrap_or(8192)
}

fn native_root_model_transport(
    context: &AgentRunContext,
    client: &AgentModelClient,
    request: &crate::agent_runtime::model::gateway::ModelRequest,
    round: i64,
    request_hash: &str,
) -> Result<
    Option<Result<crate::agent_runtime::model::gateway::ModelResponse, NativeModelRoundFailure>>,
    String,
> {
    use crate::agent_runtime::{
        model::OneShotFailure, multi_agent::budget::root::model::RootModelCall,
    };
    let Some(run) = &context.run else {
        return Ok(None);
    };
    if run.db_path != context.db_path {
        return Err("tool_authorization_unavailable".into());
    }
    let db = db::open(&context.db_path)?;
    let policy: String = db
        .query_row(
            "SELECT orchestration_policy FROM agent_runs WHERE id=?1 AND backend='native'",
            [&run.run_id],
            |r| r.get(0),
        )
        .map_err(|_| "budget_run_binding_missing")?;
    match policy.as_str() {
        "single" => {}
        "multi" => return Ok(None),
        _ => return Err("budget_execution_policy_invalid".into()),
    }
    agent_require_frozen_web_plan(&db, context).map_err(str::to_string)?;
    let bytes=serde_json::to_vec(&json!({"messages":request.messages,"tools":request.tools.iter().map(|tool|tool.as_function_spec()).collect::<Vec<_>>()})).map_err(|e|e.to_string())?.len();
    let output = native_single_model_output_cap(client.profile());
    if output == 0 {
        return Err("budget_root_model_estimate_invalid".into());
    }
    let estimate = i64::try_from(bytes)
        .ok()
        .and_then(|v| {
            i64::try_from(output)
                .ok()
                .and_then(|output| v.checked_add(output))
        })
        .filter(|v| *v > 0)
        .ok_or("budget_root_model_estimate_invalid")?;
    let request_hash = crate::agent_runtime::store::stable_hash(
        &json!({"request": request_hash, "maxOutputTokens": output}).to_string(),
    );
    let mut profile = client.profile().clone();
    profile.max_output_tokens = Some(output);
    let bounded_client = AgentModelClient::new(profile, &request.tools);
    let client = &bounded_client;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // A direct SDK caller must meet the same no-backfill boundary as executor
    // preparation. An existing original owner is verified by the typed claim.
    native_single_policy(&tx, context)?;
    if let Err(reason)=crate::agent_runtime::multi_agent::budget::root::RootOwner::load_single(&tx,&run.run_id)
        .and_then(|owner|owner.require_live(&tx)) {
        return Ok(Some(Err(native_root_model_stop(context,reason))));
    }
    let (call, _model_lifetime) = match RootModelCall::claim_single_transport(&tx, &run.run_id, round, &request_hash, estimate) {
        Ok(call) => call,
        Err(reason) => return Ok(Some(Err(native_root_model_stop(context, reason)))),
    };
    agent_authorize_tool_on(&tx, context, "inspect_evidence").map_err(str::to_string)?;
    tx.commit().map_err(|e| e.to_string())?;
    let model_log = crate::agent_runtime::model::diagnostics::ModelLog::root(&context.db_path, &run.run_id, round);
    let mut request = request.clone();
    request.request_timeout = Some(call.remaining);
    let deadline = Instant::now() + call.remaining;
    let owned = context.clone();
    let running = call.clone();
    let scan = agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number);
    let cancel = CancelToken::from_checker(move || {
        scan.is_cancelled()
            || Instant::now() >= deadline
            || !db::open(&owned.db_path).is_ok_and(|db| {
                running.require_executable(&db).is_ok()
                    && agent_authorize_tool_on(&db, &owned, "inspect_evidence").is_ok()
            })
    });
    let save = |phase: &str,
                response: Option<&crate::agent_runtime::model::gateway::ModelResponse>,
                code: &str|
     -> Result<bool, String> {
        let db = db::open(&context.db_path)?;
        let tx =
            rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
        let unresolved = call.terminal(&tx, phase, response, code)?;
        tx.commit().map_err(|e| e.to_string())?;
        model_log.cost_saved();
        Ok(unresolved)
    };
    let result = match client.complete_once_observed_with_lifecycle(&request, &cancel, model_log.observer()) {
        Ok(response) => {
            if save("received", Some(&response), "")? {
                Err(native_root_model_stop(
                    context,
                    "budget_indeterminate_requires_reconciliation".into(),
                ))
            } else {
                let db = db::open(&context.db_path)?;
                match call.require_next_work(&db).and_then(|_| {
                    agent_authorize_tool_on(&db, context, "inspect_evidence")
                        .map_err(str::to_string)
                }) {
                    Ok(()) => Ok(response),
                    Err(error) => Err(native_root_model_stop(context, error)),
                }
            }
        }
        Err(OneShotFailure::BeforeTransport(error)) => {
            save("unsent", None, error.code())?;
            Err(NativeModelRoundFailure::Model(error))
        }
        Err(OneShotFailure::TransportOutcomeUnknown(error)) => {
            save("uncertain", None, error.code())?;
            note_transport_cancel(context, client);
            if native_original_single_pause_requested(context) {
                return Ok(Some(Err(NativeModelRoundFailure::outcome(AgentTargetOutcome::Cancelled))));
            }
            Err(NativeModelRoundFailure::outcome(AgentTargetOutcome::Incomplete(AgentStop::new(
                terminal_code::REQUEST_RECONCILIATION_REQUIRED,
                format!("原 Root 模型调用结果未知，原费用占用已保存并停止后续派发；须先核对费用：{}",crate::agent_runtime::secrets::redact_text_with(&error.detail(),None)),
            ))))
        }
    };
    model_log.finish(result.is_ok());
    Ok(Some(result))
}

fn native_root_model_stop(context: &AgentRunContext, reason: String) -> NativeModelRoundFailure {
    if reason == "agent_attempt_not_active" && native_original_single_pause_requested(context) {
        return NativeModelRoundFailure::outcome(AgentTargetOutcome::Cancelled);
    }
    let mapped = agent_http_claim_error(reason.clone());
    NativeModelRoundFailure::outcome(
        agent_tool_terminal_outcome(&mapped).unwrap_or_else(|| persistence_stop(context, reason)),
    )
}
