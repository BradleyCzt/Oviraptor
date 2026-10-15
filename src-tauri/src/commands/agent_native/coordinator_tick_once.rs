enum NativeRootOriginalResponse {
    Fresh {
        tick: crate::agent_runtime::multi_agent::budget::root::model::tick::Tick,
        response: crate::agent_runtime::model::gateway::ModelResponse,
    },
    Saved {
        tick: crate::agent_runtime::multi_agent::budget::root::model::tick::Tick,
        decision: crate::agent_runtime::multi_agent::budget::root::model::tick::SavedDecision,
    },
}
enum NativeRootFinancialStage {
    Unsettled,
    FinancialAttemptFailed(String),
    FinancialCommitted,
}
struct NativeRootRoundState {
    prepared: NativeCoordinatorRequest,
    response: NativeRootOriginalResponse,
    financial_stage: NativeRootFinancialStage,
    model_log: Option<crate::agent_runtime::model::diagnostics::ModelLog>,
    _invocation: crate::agent_runtime::execution_owner::NativeInvocationOwner,
}
fn native_coordinator_tick_once(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
    prepared: NativeCoordinatorRequest,
    original_basis: &JsonValue,
    history: &[NativeCoordinatorTickReceipt],
) -> Result<NativeCoordinatorTickReceipt, String> {
    let state =
        native_root_transport_original(context, actor, frame, prepared, original_basis, history)?;
    native_root_complete_original(context, actor, frame, state, original_basis, history, None)
}
fn native_root_transport_original(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
    mut prepared: NativeCoordinatorRequest,
    original_basis: &JsonValue,
    history: &[NativeCoordinatorTickReceipt],
) -> Result<NativeRootRoundState, String> {
    use crate::agent_runtime::{
        model::OneShotFailure,
        multi_agent::budget::root::model::tick::{Begin, Tick},
    };
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    native_coordinator_verify_frame_basis(&tx, context, actor, frame, original_basis)?;
    native_coordinator_verify_local_history(&tx, history)?;
    if (frame.is_some() || !history.is_empty())
        && !native_homogeneous_birth_frame_on(&tx, actor, frame)?
    {
        native_coordinator_require_original_model_on(&tx, &actor.root_run_id, &prepared.fact)?;
    }
    // Read original C/financial identity before any first invocation ownership.
    crate::agent_runtime::multi_agent::budget::root::RootOwner::load_original(
        &tx,
        &actor.root_run_id,
    )?
    .require_original_coordinator(&tx, actor)?;
    // Historical calls only probe their existing original inode. Keep that
    // guard through admission, SDK, cost/log and local publication/replay.
    let original_invocation =
        crate::agent_runtime::multi_agent::budget::root::model::tick::require_idle_original(
            &tx, actor,
        )?;
    prepared.basis = Tick::with_budget_snapshot(&tx, &actor.root_run_id, &prepared.basis)?;
    if prepared.basis.get("budgetSnapshot").is_some() {
        let mut input: JsonValue = serde_json::from_str(
            prepared.request.messages[1]["content"]
                .as_str()
                .ok_or("root_tick_messages_invalid")?,
        )
        .map_err(|_| "root_tick_messages_invalid")?;
        input["basis"]["budgetSnapshot"] = prepared.basis["budgetSnapshot"].clone();
        prepared.request.messages[1]["content"] = input.to_string().into();
        prepared.fact["basisHash"] = Tick::basis_hash(&prepared.basis).into();
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
            .and_then(|n| n.checked_add(i64::try_from(output).ok()?))
            .filter(|n| *n > 0)
            .ok_or("root_tick_estimate_invalid")?;
    }
    let round = Tick::next_round(&tx, &actor.root_run_id, &prepared.basis)?;
    let begin = Tick::begin(
        &tx,
        &actor.root_run_id,
        round,
        &prepared.basis,
        &prepared.fact,
        prepared.estimate,
    )?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    native_coordinator_verify_frame_basis(&tx, context, actor, frame, original_basis)?;
    native_coordinator_verify_local_history(&tx, history)?;
    if frame.is_some() || !history.is_empty() {
        native_coordinator_require_original_model_on(&tx, &actor.root_run_id, &prepared.fact)?;
    }
    // Only validated first admission may create invocation ownership. No
    // rejected input or historical replay can manufacture original exit proof.
    let _root_invocation = match original_invocation {
        Some(guard) => guard,
        None => crate::agent_runtime::execution_owner::claim_native_invocation(
            &context.db_path,
            &actor.scan_id,
            actor.attempt_number,
            crate::agent_runtime::multi_agent::budget::root::model::tick::INVOCATION_KIND,
            &actor.root_run_id,
        )?,
    };
    tx.commit().map_err(|e| e.to_string())?;
    let mut model_log = None;
    let response = match begin {
        Begin::Saved(tick, decision) => NativeRootOriginalResponse::Saved {
            tick,
            decision: *decision,
        },
        Begin::Dispatch(tick) => {
            prepared.request.request_timeout = Some(tick.remaining());
            let deadline = Instant::now() + tick.remaining();
            let owned = context.clone();
            let original = actor.clone();
            let inflight = tick.clone();
            let human_frame = frame.filter(|f| f.kind == "human-directive").cloned();
            let scan =
                agent_scan_cancel_token(&context.db_path, &context.scan_id, context.attempt_number);
            let cancel = CancelToken::from_checker(move || {
                scan.is_cancelled()
                    || Instant::now() >= deadline
                    || !db::open(&owned.db_path).is_ok_and(|db| {
                        inflight.require_executable(&db).is_ok()
                            && native_coordinator_tick_authority(&db, &owned, &original).is_ok()
                            && human_frame
                                .as_ref()
                                .is_none_or(|f| f.verify(&db, &owned, &original).is_ok())
                    })
            });
            let log = crate::agent_runtime::model::diagnostics::ModelLog::coordinator(
                &context.db_path,
                &actor.root_run_id,
                round,
            );
            let observer = log.observer();
            model_log = Some(log);
            match prepared.client.complete_once_observed_with_lifecycle(
                &prepared.request,
                &cancel,
                observer,
            ) {
                Ok(response) => {
                    let db = db::open(&context.db_path)?;
                    tick.persist_received_invoice(&db, &response)?;
                    #[cfg(test)]
                    native_financial_continuation_received_test_checkpoint(context, actor);
                    #[cfg(test)]
                    crate::scanning::agent_tests::native_root_controlled_post_original_invoice_checkpoint(context, actor, round);
                    #[cfg(test)]
                    crate::scanning::agent_tests::native_mapper_source_drift_post_original_invoice_checkpoint(context, actor, round);
                    NativeRootOriginalResponse::Fresh { tick, response }
                }
                Err(error) => {
                    let (phase, code) = match &error {
                        OneShotFailure::BeforeTransport(error) => ("unsent", error.code()),
                        OneShotFailure::TransportOutcomeUnknown(error) => {
                            ("uncertain", error.code())
                        }
                    };
                    let db = db::open(&context.db_path)?;
                    let tx = rusqlite::Transaction::new_unchecked(
                        &db,
                        rusqlite::TransactionBehavior::Immediate,
                    )
                    .map_err(|e| e.to_string())?;
                    let terminal = tick.stopped(&tx, phase, code)?;
                    tx.commit().map_err(|e| e.to_string())?;
                    if let Some(log) = &model_log {
                        log.cost_saved();
                    }
                    tick.persist_terminal_capture(&db, terminal)?;
                    return Err(format!("root_tick_{phase}:{code}"));
                }
            }
        }
    };
    Ok(NativeRootRoundState {
        prepared,
        response,
        financial_stage: NativeRootFinancialStage::Unsettled,
        model_log,
        _invocation: _root_invocation,
    })
}

fn native_root_complete_original(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
    mut state: NativeRootRoundState,
    original_basis: &JsonValue,
    history: &[NativeCoordinatorTickReceipt],
    semantic: Option<
        crate::agent_runtime::multi_agent::budget::root::model::tick::OriginalSemanticToken,
    >,
) -> Result<NativeCoordinatorTickReceipt, String> {
    native_root_complete_original_on(
        context,
        actor,
        frame,
        &mut state,
        original_basis,
        history,
        semantic,
    )
}
fn native_root_complete_original_on(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    frame: Option<&NativeCoordinatorFrame>,
    state: &mut NativeRootRoundState,
    original_basis: &JsonValue,
    history: &[NativeCoordinatorTickReceipt],
    semantic: Option<
        crate::agent_runtime::multi_agent::budget::root::model::tick::OriginalSemanticToken,
    >,
) -> Result<NativeCoordinatorTickReceipt, String> {
    let NativeRootRoundState {
        prepared,
        response,
        financial_stage,
        model_log,
        ..
    } = state;
    let mut homogeneous_text: Option<String> = None;
    let (tick, decision, replayed) = match response {
        NativeRootOriginalResponse::Saved { tick, decision } => {
            // 同质轮次的副作用只属于原始那一轮：账本里存的是投影摘要，
            // 无法（也不应）从历史重放出第二次副作用。
            if frame.is_some_and(|f| f.kind == NATIVE_HOMOGENEOUS_FRAME_KIND) {
                return Err("root_homogeneous_replay_unsupported".into());
            }
            (tick.clone(), decision.clone(), true)
        }
        NativeRootOriginalResponse::Fresh { tick, response } => {
            if frame.is_some_and(|f| f.kind == NATIVE_HOMOGENEOUS_FRAME_KIND) {
                // 只在内存里留给本函数末尾的副作用；原文永不落库。
                homogeneous_text = Some(if response.tool_calls.is_empty()
                    && response.finish_reason == "stop" && response.text.len() <= 48 * 1024
                { response.text.clone() } else { "{}".to_string() });
            }
            match financial_stage {
                NativeRootFinancialStage::FinancialAttemptFailed(error) => {
                    return Err(error.clone())
                }
                NativeRootFinancialStage::FinancialCommitted => {
                    return Err("root_original_financial_already_committed".into())
                }
                NativeRootFinancialStage::Unsettled => {}
            }
            // This is one financial attempt, not a retry policy. Keep original
            // ownership and invoice alive if any transaction step rolls back.
            *financial_stage = NativeRootFinancialStage::FinancialAttemptFailed(
                "root_original_financial_attempt_incomplete".into(),
            );
            let financial = (|| -> Result<_, String> {
                let db = db::open(&context.db_path)?;
                let tx = rusqlite::Transaction::new_unchecked(
                    &db,
                    rusqlite::TransactionBehavior::Immediate,
                )
                .map_err(|e| e.to_string())?;
                let received = match semantic {
                    Some(token) => tick.received_with_original_semantic(&tx, response, token)?,
                    None => tick.received(&tx, response)?,
                };
                tx.commit().map_err(|e| e.to_string())?;
                Ok((db, received))
            })();
            let (db, received) = match financial {
                Ok(value) => value,
                Err(error) => {
                    *financial_stage =
                        NativeRootFinancialStage::FinancialAttemptFailed(error.clone());
                    return Err(error);
                }
            };
            *financial_stage = NativeRootFinancialStage::FinancialCommitted;
            if let Some(log) = model_log.as_ref() {
                log.cost_saved();
            }
            tick.persist_terminal_capture(&db, received.terminal)?;
            #[cfg(test)]
            native_coordinator_tick_paid_checkpoint();
            if received.unknown {
                return Err("budget_indeterminate_requires_reconciliation".into());
            }
            // Malformed homogeneous replies have an auditable refusal summary.
            // Any failure to persist that original decision still blocks effects.
            let decision = received.decision?;
            (tick.clone(), decision, false)
        }
    };
    let db = db::open(&context.db_path)?;
    let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    native_coordinator_verify_frame_basis(&tx, context, actor, frame, original_basis)?;
    native_coordinator_verify_local_history(&tx, history)?;
    if frame.is_some() || !history.is_empty() {
        native_coordinator_require_original_model_on(&tx, &actor.root_run_id, &prepared.fact)?;
    }
    if frame.is_some_and(|f| f.kind == "lease-expired")
        && !decision.summary.as_json()["suggestions"]
            .as_array()
            .is_some_and(|items| {
                items
                    .iter()
                    .all(|item| item.as_str() == Some("assess:lease_expired"))
            })
    {
        return Err("root_lease_expired_step_not_bounded".into());
    }
    if frame.is_some_and(|f| f.kind == "protection-changed")
        && !decision.summary.as_json()["suggestions"]
            .as_array()
            .is_some_and(|items| {
                items
                    .iter()
                    .all(|item| item.as_str() == Some("assess:protection_changed"))
            })
    {
        return Err("root_protection_step_not_bounded".into());
    }
    if frame.is_some_and(|f| f.kind == "capability-changed")
        && !decision.summary.as_json()["suggestions"]
            .as_array()
            .is_some_and(|items| {
                items
                    .iter()
                    .all(|item| item.as_str() == Some("assess:capability_changed"))
            })
    {
        return Err("root_capability_changed_step_not_bounded".into());
    }
    if frame.is_some_and(|f| f.kind == "authorization-output") {
        native_authorization_output_require_bounded(&decision.summary.as_json())?;
    }
    if frame.is_some_and(|f| f.kind == "external-surface-output") {
        native_external_surface_output_require_bounded(&decision.summary.as_json())?;
    }
    if frame.is_some_and(|f| f.kind == "business-logic-output") {
        native_business_logic_output_require_bounded(&decision.summary.as_json())?;
    }
    if frame.is_some_and(|f| f.kind == "business-logic-input")
        && !decision.summary.as_json()["suggestions"]
            .as_array()
            .is_some_and(|items| {
                items
                    .iter()
                    .all(|item| item.as_str() == Some("dispatch:business_logic"))
            })
    {
        return Err("root_business_logic_dispatch_step_not_bounded".into());
    }
    let sequence = tick.publish(&tx, &decision)?;
    native_coordinator_tick_authority(&tx, context, actor)?;
    native_coordinator_verify_frame_basis(&tx, context, actor, frame, original_basis)?;
    native_coordinator_verify_local_history(&tx, history)?;
    if frame.is_some() || !history.is_empty() {
        native_coordinator_require_original_model_on(&tx, &actor.root_run_id, &prepared.fact)?;
    }
    tick.require_executable(&tx)?;
    if tick.published(&tx, &decision)? != Some(sequence) {
        return Err("root_tick_publication_unconfirmed".into());
    }
    tx.commit().map_err(|e| e.to_string())?;
    if let Some(log) = model_log.as_ref() {
        log.finish(true);
    }
    if let Some(text) = homogeneous_text {
        // P0-1：发票已结算、decision 事件已发布，现在才允许副作用进入控制面。
        native_homogeneous_require_bounded(&decision.summary.as_json())?;
        let applied = {
            let db = db::open(&context.db_path)?;
            native_homogeneous_apply_after_paid_tick(&db, actor, &text)?
        };
        append_runner_log(&context.log_path, &applied);
        // Phase 1.1：主管决策可能刚派发出新切片，立刻让 Worker 消费，
        // 下一轮的候选快照就能看到 reported。drain 失败只留痕，不推翻已结算轮次。
        let drained = {
            let db = db::open(&context.db_path)?;
            if crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(&db,&actor.scan_id,actor.attempt_number)? {
                "root_homogeneous_workers:supervised_model_dispatch".to_string()
            } else { native_homogeneous_drain_workers(&db, actor) }
        };
        append_runner_log(&context.log_path, &drained);
    }
    Ok(NativeCoordinatorTickReceipt {
        summary: decision.summary.clone(),
        event_sequence: sequence,
        replayed,
        tick,
        saved: decision,
    })
}
