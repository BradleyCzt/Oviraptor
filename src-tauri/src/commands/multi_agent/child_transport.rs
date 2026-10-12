#[cfg(not(test))]
fn multi_agent_child_round(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    system: &str,
    input: JsonValue,
) -> Result<(String, AgentTokenUsage), String> {
    multi_agent_child_round_transport(context, lease, child, system, input)
}

// Shared production transport remains compiled in tests so loopback assertions
// exercise actual specialist HTTP instead of only the deterministic role double.
fn multi_agent_child_round_transport(
    context: &AgentRunContext,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    system: &str,
    input: JsonValue,
) -> Result<(String, AgentTokenUsage), String> {
    let run = context
        .run
        .as_ref()
        .ok_or("specialist_context_run_missing")?;
    let authorization_candidate=input.clone();
    specialist_round_transport_bound(
        &SpecialistTransportContext {
            supervision: context.supervision.clone(),
            db_path: &context.db_path,
            scan_id: &context.scan_id,
            attempt_number: context.attempt_number,
            target_key: &context.target_url,
            run_id: &run.run_id,
            environment: &context.environment,
            proxy: context.proxy.as_deref(),
            usage_dir: &context.target_dir,
            deadline: None,
        },
        lease,
        child,
        system,
        input,
        None,
        |tx| {
            if child.role == crate::agent_runtime::contract::AgentRole::BusinessLogic {
                let source = native_business_logic_input_collection_on(tx, lease, &context.target_dir)?;
                let mode = crate::agent_runtime::web_mode::root::read(tx, &lease.root_run_id)?
                    .ok_or("business_logic_prepared_original_birth_missing")?;
                if mode.bootstrap_dispatch().is_some_and(|p| matches!(p["schemaVersion"].as_i64(), Some(14 | 15))) {
                    business_logic_dispatch_receipt::verify_original_on(tx, lease, child)?;
                }
                let valid: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 AND child_run_id=?2 AND coordinator_run_id=?3 AND role='business_logic' AND lane='read_only_analysis' AND trigger_code='business_logic_frozen_evidence_ready' AND evidence_revision=1 AND contract_keys_json='[]' AND identity_handles_json='[]' AND task_slice_json=?4) AND (SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 OR child_run_id=?2)=2 AND (SELECT count(*) FROM agent_capability_leases WHERE assignment_id=?1 AND child_run_id=?2 AND root_run_id=?3 AND lease_epoch=?5 AND fencing_token=?6 AND capability IN ('evidence.read','mailbox.write') AND revoked_at='' AND lease_expires_at>datetime('now','localtime'))=2",
                    params![child.assignment_id, child.run_id, lease.root_run_id, source.task().to_string(), lease.lease_epoch, lease.fencing_token],
                    |r| r.get(0),
                ).map_err(|e| e.to_string())?;
                if !valid || *source.input() != authorization_candidate || system != BUSINESS_LOGIC_READONLY_SYSTEM {
                    return Err("business_logic_prepared_original_input_changed".into());
                }
                let key = crate::agent_runtime::multi_agent::contract_owner::schedule_contract_key(
                    lease.attempt_number, &lease.target_key, "business_logic", "business_logic_frozen_evidence_ready", 1,
                );
                crate::agent_runtime::multi_agent::contract_owner::verify_held_owner(
                    tx, &lease.root_run_id, &key, &child.assignment_id, lease.lease_epoch, &lease.fencing_token,
                )?;
            }
            if child.role == crate::agent_runtime::contract::AgentRole::EvidenceReviewer {
                native_authorization_original_candidate_on(tx,lease,&context.target_dir,&authorization_candidate)?;
            }
            Ok(())
        },
    )
}

include!("child_transport_context.rs");

fn specialist_model_cancel_token(
    db_path: &Path,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    supervision: Option<crate::agent_runtime::multi_agent::supervision_ticket::SupervisionTicket>,
) -> CancelToken {
    let database = db_path.to_path_buf();
    let lease = lease.clone();
    let child = child.clone();
    CancelToken::from_checker(move || {
        if supervision
            .as_ref()
            .is_some_and(|ticket| ticket.check_actor(&database, &lease).is_err())
        {
            return true;
        }
        let Ok(connection) = db::open(&database) else {
            return true;
        };
        if !native_source_attempt_active(&connection, &lease.scan_id, lease.attempt_number)
            || crate::agent_runtime::multi_agent::attempts::require_live_for_run(
                &connection,
                &child.run_id,
            )
            .is_err()
            || crate::agent_runtime::multi_agent::lease::validate_coordinator_lease(
                &connection,
                &lease,
            )
            .is_err()
        {
            return true;
        }
        // A live scan is insufficient: an operator may cancel one assignment,
        // revoke this root, or replace its fence while sibling work continues.
        !connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_assignments a \
             JOIN agent_runs r ON r.id=a.child_run_id JOIN agent_runs root ON root.id=a.coordinator_run_id \
             WHERE a.id=?1 AND a.child_run_id=?2 AND a.coordinator_run_id=?3 AND a.target_key=?4 \
               AND a.lease_epoch=?5 AND a.fencing_token=?6 AND a.state='running' AND a.role=?7 \
               AND r.assignment_id=a.id AND r.root_run_id=root.id AND r.role=a.role AND r.lane=a.lane \
               AND r.scan_id=?8 AND r.attempt_number=?9 AND r.target_url=a.target_key \
               AND r.status='running' AND r.cancel_requested_at='' \
               AND root.root_run_id=root.id AND root.role='coordinator' \
               AND root.scan_id=r.scan_id AND root.attempt_number=r.attempt_number AND root.target_url=r.target_url \
               AND root.status IN ('prepared','running') AND root.cancel_requested_at='' \
               AND EXISTS(SELECT 1 FROM agent_capability_leases c WHERE c.assignment_id=a.id \
                 AND c.child_run_id=r.id AND c.root_run_id=root.id AND c.capability='evidence.read' \
                 AND c.lease_epoch=a.lease_epoch AND c.fencing_token=a.fencing_token \
                 AND c.revoked_at='' AND c.lease_expires_at>datetime('now','localtime')) \
               AND (EXISTS(SELECT 1 FROM agent_lane_leases l WHERE l.assignment_id=a.id AND l.lane=a.lane \
                 AND l.scan_id=r.scan_id AND l.attempt_number=r.attempt_number AND l.target_key=a.target_key) \
                 OR (a.scheduling_model='homogeneous_workers' AND a.lane='read_only_analysis' \
                   AND EXISTS(SELECT 1 FROM agent_lane_slots s WHERE s.assignment_id=a.id AND s.root_run_id=root.id AND s.lane=a.lane AND s.released_at=''))))",
            params![child.assignment_id,child.run_id,lease.root_run_id,lease.target_key,
                lease.lease_epoch,lease.fencing_token,child.role.as_str(),lease.scan_id,lease.attempt_number],
            |row| row.get::<_,bool>(0),
        ).unwrap_or(false)
    })
}

fn specialist_round_transport(
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    system: &str,
    input: JsonValue,
) -> Result<(String, AgentTokenUsage), String> {
    specialist_round_transport_bound(context, lease, child, system, input, None, |_| Ok(()))
}

// Dispatch metadata stays in the original frozen request, never provider messages.
fn specialist_round_transport_bound(
    context: &SpecialistTransportContext<'_>,
    lease: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    system: &str,
    input: JsonValue,
    human_dispatch: Option<JsonValue>,
    authorize_dispatch: impl Fn(&rusqlite::Connection) -> Result<(), String>,
) -> Result<(String, AgentTokenUsage), String> {
    use crate::agent_runtime::{multi_agent::specialist, store::stable_hash};
    if context.scan_id != lease.scan_id
        || context.attempt_number != lease.attempt_number
        || context.target_key != lease.target_key
    {
        return Err("specialist_context_binding_invalid".into());
    }
    let connection = db::open(context.db_path)?;
    let bound: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND (id=?2 OR root_run_id=?2) \
         AND scan_id=?3 AND attempt_number=?4 AND target_url=?5)",
            params![
                context.run_id,
                lease.root_run_id,
                lease.scan_id,
                lease.attempt_number,
                lease.target_key
            ],
            |r| r.get(0),
        )
        .map_err(|e| format!("specialist_context_lookup:{e}"))?;
    if !bound {
        return Err("specialist_context_binding_invalid".into());
    }
    let human_proposal = human_dispatch.is_some();
    let mut profile = agent_model_profile(context.environment, context.proxy)?;
    if human_proposal {
        profile.max_output_tokens = Some(profile.max_output_tokens.unwrap_or(512).min(512));
    }
    let plaintext = crate::agent_runtime::multi_agent::homogeneous::material_policy::for_root(&connection, &lease.root_run_id)?;
    let payload = if plaintext { input.clone() } else { crate::agent_runtime::secrets::redact_json(&input) };
    let messages = vec![
        serde_json::json!({"role":"system","content":system}),
        serde_json::json!({"role":"user","content":payload.to_string()}),
    ];
    let source_role = crate::agent_runtime::multi_agent::source::uses_source_runtime(
        &connection,
        lease,
        child.role,
    )?;
    let client_side = child.role == crate::agent_runtime::contract::AgentRole::ClientSide;
    let input_parser = child.role == crate::agent_runtime::contract::AgentRole::InputParser;
    let homogeneous = connection.query_row("SELECT scheduling_model='homogeneous_workers' FROM agent_assignments WHERE id=?1 AND child_run_id=?2",params![child.assignment_id,child.run_id],|r|r.get::<_,bool>(0)).unwrap_or(false);
    let required_tokens = if homogeneous {
        let review=child.role==crate::agent_runtime::contract::AgentRole::EvidenceReviewer;
        let worker = if review{crate::commands::homogeneous_execution::review_minimum(&input)}else{crate::agent_runtime::multi_agent::homogeneous::model_worker::minimum_tokens(&input)};
        let execution=crate::agent_runtime::multi_agent::homogeneous::execution_policy::read_original_on(&connection,&lease.root_run_id)?.is_some();
        let (expected,output)=if review && execution{(crate::commands::homogeneous_execution::REVIEW_SYSTEM,crate::commands::homogeneous_execution::REVIEW_OUTPUT)}else if execution{(crate::agent_runtime::multi_agent::homogeneous::execution_worker::SYSTEM,crate::agent_runtime::multi_agent::homogeneous::execution_worker::MAX_OUTPUT)}else{(crate::agent_runtime::multi_agent::homogeneous::model_worker::SYSTEM,crate::agent_runtime::multi_agent::homogeneous::model_worker::MAX_OUTPUT)};
        if system != expected {
            return Err("homogeneous_worker_system_invalid".into());
        }
        profile.max_output_tokens = Some(output as u64);
        Some(worker)
    } else if client_side {
        client_side_verify_input(context, &connection, lease, child, &input)?;
        profile.max_output_tokens = Some(256);
        Some(client_side_model_budget(&messages, &profile)?)
    } else if input_parser {
        input_parser_verify_input(context, &connection, lease, child, &input)?;
        profile.max_output_tokens = Some(512);
        Some(input_parser_model_budget(&messages, &profile)?)
    } else if source_role {
        let (tokens, output) = source_assessment_budget(&messages, &profile)?;
        profile.max_output_tokens = Some(output);
        Some(tokens)
    } else {
        None
    };
    // Model settings are frozen without persisting endpoint/proxy credentials.
    let mut frozen = serde_json::json!({"schemaVersion":1,"messages":messages,"tools":[],"temperature":0.0,
        "model":profile.model,"endpointHash":stable_hash(&profile.endpoint),
        "proxyHash":stable_hash(profile.proxy.as_deref().unwrap_or_default()),
        "local":profile.local,"maxOutputTokens":profile.max_output_tokens,"maxContextTokens":profile.max_context_tokens,
        "providerOptions":profile.bounded_wire_options()});
    if let Some(proof) = human_dispatch {
        frozen["humanDirectiveDispatch"] = proof;
    }
    // Original binding and journal decide CREATE versus probe under the same SQL lock.
    // Keep the returned owner through gateway, original receipt and diagnostics.
    let dispatch_guard=if human_proposal {
        Some(crate::agent_runtime::multi_agent::directive::proposals::owned::DispatchGuard::install(&connection)?)
    } else {None};
    let validate_dispatch = |tx: &rusqlite::Connection| {
        context.require_supervision(lease)?;
        authorize_dispatch(tx)?;
        if context
            .deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return Err("source_model_deadline_exceeded".into());
        }
        if let Some(required) = required_tokens {
            if source_role {
                authorize_source_specialist(tx, context, lease)?;
            }
            if client_side {
                client_side_verify_input(context, tx, lease, child, &input)?;
            }
            if input_parser {
                input_parser_verify_input(context, tx, lease, child, &input)?;
            }
            // Received replays consume no new reservation. For a new dispatch,
            // verify the actual assignment under the dispatch transaction.
            let enough: bool=tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE id=?1 AND reserved_tokens>=?2 AND reserved_requests>=1) OR EXISTS(SELECT 1 FROM agent_specialist_calls WHERE assignment_id=?1 AND child_run_id=?3 AND state='received')",
                params![child.assignment_id,required,child.run_id],|r|r.get(0)).map_err(|_|"source_model_reservation_lookup")?;
            if !enough {
                return Err(if client_side {
                    "client_side_model_reservation_insufficient"
                } else if input_parser {
                    "input_parser_model_reservation_insufficient"
                } else {
                    "source_model_reservation_insufficient"
                }
                .into());
            }
        }
        Ok(())
    };
    let (started, _specialist_invocation) = if client_side {
        client_sdk_dispatch_writer::start(context.db_path, lease, child, &frozen, validate_dispatch)?
    } else {
        specialist::start_for_transport(&connection, lease, child, &frozen, validate_dispatch)?
    };
    if let Some(guard)=dispatch_guard {guard.finish()?;}
    let pending = match started {
        specialist::Start::Dispatch(call) => call,
        specialist::Start::Received(response) => {
            context.require_supervision(lease)?;
            return specialist_response_result(child, response);
        }
    };
    let model_log = crate::agent_runtime::model::diagnostics::ModelLog::child(context.db_path, lease, child, 1, false);
    let client = AgentModelClient::new(profile, &[]);
    let mut request = agent_model_request(messages, &[]);
    let remaining = match crate::agent_runtime::multi_agent::budget::clock::remaining(
        &connection,
        &lease.root_run_id,
    ) {
        Ok(remaining) => remaining,
        // Let the gateway prove a deadline cancellation before transport.
        // This is not a fabricated provider receipt or a new timeout.
        Err(_) if human_proposal => std::time::Duration::ZERO,
        Err(error) => {
            specialist::record_not_sent(&connection, &pending, &error)?;
            return Err(error);
        }
    };
    let shared_deadline = std::time::Instant::now() + remaining;
    let deadline = Some(
        context
            .deadline
            .map_or(shared_deadline, |d| d.min(shared_deadline)),
    );
    request.request_timeout =
        deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()));
    let scoped_cancel =
        specialist_model_cancel_token(context.db_path, lease, child, context.supervision.clone());
    let supervision = context.supervision.clone();
    let (database, model, proxy, source_lease, usage_dir) = (
        context.db_path.to_path_buf(),
        context.environment.clone(),
        context.proxy.map(str::to_string),
        lease.clone(),
        context.usage_dir.to_path_buf(),
    );
    let source_child = child.clone();
    let cancel = CancelToken::from_checker(move || {
        if scoped_cancel.is_cancelled()
            || deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            return true;
        }
        if !source_role {
            return false;
        }
        let Ok(connection) = db::open(&database) else {
            return true;
        };
        let Ok(tx) = rusqlite::Transaction::new_unchecked(
            &connection,
            rusqlite::TransactionBehavior::Deferred,
        ) else {
            return true;
        };
        let current = SpecialistTransportContext {
            supervision: supervision.clone(),
            db_path: &database,
            scan_id: &source_lease.scan_id,
            attempt_number: source_lease.attempt_number,
            target_key: &source_lease.target_key,
            run_id: &source_lease.root_run_id,
            environment: &model,
            proxy: proxy.as_deref(),
            usage_dir: &usage_dir,
            deadline,
        };
        if authorize_source_specialist(&tx, &current, &source_lease).is_err()
            || heartbeat_source_specialist(&tx, &source_lease, &source_child).is_err()
            || authorize_source_specialist(&tx, &current, &source_lease).is_err()
        {
            return true;
        }
        tx.commit().is_err()
    });
    let response = match client.complete_once_observed_with_lifecycle(&request, &cancel, model_log.observer()) {
        Ok(response) => response,
        Err(crate::agent_runtime::model::OneShotFailure::BeforeTransport(error)) => {
            let original = format!(
                "{} 子智能体模型调用未发出：{}",
                child.role.as_str(),
                error.detail()
            );
            return Err(
                match specialist::record_not_sent(&connection, &pending, error.code()) {
                    Ok(()) => original,
                    Err(persist) => format!("{original};specialist_no_send_receipt:{persist}"),
                },
            );
        }
        Err(crate::agent_runtime::model::OneShotFailure::TransportOutcomeUnknown(error)) => {
            let original = format!(
                "{} 子智能体模型调用失败：{}",
                child.role.as_str(),
                error.detail()
            );
            return Err(
                match specialist::record_uncertain(&connection, &pending, error.code()) {
                    Ok(()) => original,
                    Err(persist) => format!("{original};specialist_dispatch_receipt:{persist}"),
                },
            );
        }
    };
    client.record_usage_line(context.usage_dir, &response);
    let saved = specialist::record_received_with_provenance(
        &connection,
        &pending,
        &response.text,
        !response.tool_calls.is_empty(),
        &response.usage,
        response.usage_reported,
    );
    model_log.cost_saved();
    let saved = saved?;
    context.require_supervision(lease)?;
    // The original received bill survives a changed business source. A
    // Reviewer may deliver only after its Authorization facts still verify.
    if human_proposal || child.role == crate::agent_runtime::contract::AgentRole::EvidenceReviewer {
        let tx=rusqlite::Transaction::new_unchecked(&connection,rusqlite::TransactionBehavior::Deferred)
            .map_err(|e|e.to_string())?;
        authorize_dispatch(&tx)?;
        tx.commit().map_err(|e|e.to_string())?;
    }
    let result = specialist_response_result(child, saved);
    model_log.finish(result.is_ok());
    result
}

fn specialist_response_result(
    child: &crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    response: crate::agent_runtime::multi_agent::specialist::StoredResponse,
) -> Result<(String, AgentTokenUsage), String> {
    match response.rejection.as_str() {
        "" => Ok((response.text, response.usage)),
        "empty_response" => Err(format!("{} 子智能体返回空结果", child.role.as_str())),
        reason => Err(format!("{}_{reason}", child.role.as_str())),
    }
}

include!("child_transport_tests.rs");
