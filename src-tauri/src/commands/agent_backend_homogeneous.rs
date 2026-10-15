/// prepare has activated the frozen homogeneous task and released its legacy owner.
/// Keep one parent supervisor alive across the original paid Root decisions.
fn run_homogeneous_agent_target(context: &mut AgentRunContext) -> AgentTargetOutcome {
    use crate::agent_runtime::multi_agent::{
        lease, parent_invocation_owner::ParentInvocationOwner, supervisor::WorkerSupervisor,
    };
    let setup = (|| {
        let root = &context
            .run
            .as_ref()
            .ok_or("homogeneous_root_run_missing")?
            .run_id;
        let db = db::open(&context.db_path)?;
        let actor = lease::acquire_coordinator_lease(
            &db,
            &context.scan_id,
            context.attempt_number,
            &context.target_url,
            root,
            600,
        )?;
        let owner = ParentInvocationOwner::claim(
            &context.db_path,
            &context.scan_id,
            context.attempt_number,
            root,
        )?;
        let supervisor = match original_withdrawal_issuer_on(&db, &actor)? {
            Some(issuer) => {
                WorkerSupervisor::start_owned_signed(&context.db_path, &actor, owner, issuer)?
            }
            None => WorkerSupervisor::start_owned(&context.db_path, &actor, owner)?,
        };
        context.supervision = Some(supervisor.ticket());
        native_coordinator_initialize_control(context, &actor)?;
        native_homogeneous_seed_observations(context, &actor)?;
        Ok::<_, String>((actor, supervisor))
    })();
    let (actor, supervisor) = match setup {
        Ok(session) => session,
        Err(error) => {
            return AgentTargetOutcome::persistence_failure(format!(
                "homogeneous_root_activation_failed:{error}"
            ))
        }
    };
    append_runner_log(
        &context.log_path,
        "homogeneous Worker supervisor active; legacy bootstrap stepped aside",
    );
    let outcome = run_homogeneous_root_loop(context, &actor, &supervisor);
    let result = finalize_agent_target(context, &actor, outcome);
    // The original financial closure and held SDK exits must finish first.
    // This separate transaction only releases homogeneous business metadata.
    let cleanup = (|| -> Result<(), String> {
        let db = db::open(&context.db_path)?;
        if !crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(
            &db,
            &actor.scan_id,
            actor.attempt_number,
        )? {
            return Ok(());
        }
        let tx = db.unchecked_transaction().map_err(|e| e.to_string())?;
        crate::agent_runtime::multi_agent::budget::clock::FinalClock::verify_original_exit(
            &tx,
            &actor.root_run_id,
        )?;
        let stopped:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND status='terminal' AND terminal_state NOT IN ('completed','completed_with_gaps'))",[&actor.root_run_id],|r|r.get(0)).map_err(|e|e.to_string())?;
        if stopped {
            tx.execute("UPDATE agent_capability_leases SET revoked_at=datetime('now','localtime') WHERE root_run_id=?1 AND revoked_at=''",[&actor.root_run_id]).map_err(|e|e.to_string())?;
            tx.execute("UPDATE agent_assignment_attempts SET state='cancelled',failure_class='specialist_task_ended_reconciliation_required',finished_at=datetime('now','localtime') WHERE root_run_id=?1 AND state IN ('leased','running','paused')",[&actor.root_run_id]).map_err(|e|e.to_string())?;
            tx.execute("UPDATE agent_runs SET status='terminal',terminal_state='cancelled',terminal_reason='specialist_task_ended_reconciliation_required',finished_at=datetime('now','localtime') WHERE root_run_id=?1 AND id<>?1 AND scheduling_model='homogeneous_workers' AND status<>'terminal'",[&actor.root_run_id]).map_err(|e|e.to_string())?;
            tx.execute("UPDATE agent_assignments SET state='cancelled',updated_at=datetime('now','localtime') WHERE coordinator_run_id=?1 AND scheduling_model='homogeneous_workers' AND state IN ('created','assigned','running','reported')",[&actor.root_run_id]).map_err(|e|e.to_string())?;
        }
        tx.execute("UPDATE agent_task_slices SET state='cancelled',updated_at=datetime('now','localtime') WHERE parent_task_id IN (SELECT task_id FROM agent_homogeneous_tasks WHERE root_run_id=?1) AND state IN ('pending','assigned','in_progress','reported','rejected') AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND terminal_state IN ('completed','completed_with_gaps'))",[&actor.root_run_id]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE agent_lane_slots SET released_at=datetime('now','localtime') WHERE root_run_id=?1 AND released_at='' AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND terminal_state IN ('completed','completed_with_gaps'))",[&actor.root_run_id]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE agent_worker_instances SET state='dead',assignment_id='',updated_at=datetime('now','localtime') WHERE root_run_id=?1 AND state='busy' AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND terminal_state IN ('completed','completed_with_gaps'))",[&actor.root_run_id]).map_err(|e|e.to_string())?;
        tx.execute("UPDATE agent_homogeneous_tasks SET state=CASE WHEN (SELECT terminal_state FROM agent_runs WHERE id=?1)='cancelled' THEN 'cancelled' ELSE 'stopped' END,updated_at=datetime('now','localtime') WHERE root_run_id=?1 AND state IN ('created','decomposed','running','partial_success') AND NOT EXISTS(SELECT 1 FROM agent_runs WHERE id=?1 AND terminal_state IN ('completed','completed_with_gaps'))",[&actor.root_run_id]).map_err(|e|e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    })();
    if let Err(error) = cleanup {
        append_runner_log(
            &context.log_path,
            &format!("homogeneous_business_cleanup_failed:{error}"),
        );
        return AgentTargetOutcome::persistence_failure(format!(
            "homogeneous_business_cleanup_failed:{error};original_reason={}",
            result.detail()
        ));
    }
    result
}

fn run_homogeneous_root_loop(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    supervisor: &crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor,
) -> AgentTargetOutcome {
    use crate::agent_runtime::multi_agent::homogeneous::{apply, gate, intent, pool, store, types};
    let result = std::thread::scope(|scope| {
        let mut started = std::collections::HashSet::new();
        let failures = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let control = std::sync::Arc::new(std::sync::Mutex::new(()));
        (|| -> Result<AgentTargetOutcome, String> {
            loop {
                let control_guard = control.lock().map_err(|_| "homogeneous_control_poisoned")?;
                if !native_web_attempt_active(
                    &context.db_path,
                    &context.scan_id,
                    context.attempt_number,
                ) {
                    return Ok(AgentTargetOutcome::Cancelled);
                }
                supervisor.check()?;
                if let Some(error) = failures
                    .lock()
                    .map_err(|_| "homogeneous_worker_join_poisoned")?
                    .first()
                    .cloned()
                {
                    return Err(error);
                }
                let db = db::open(&context.db_path)?;
                let task = store::load_task_by_root_run(&db, &actor.root_run_id)
                    .map_err(|error| error.reason_code.to_string())?
                    .ok_or("homogeneous_root_task_missing")?;
                if types::is_member(
                    &task.state,
                    &["succeeded", "failed", "cancelled", "stopped"],
                ) {
                    let reason = intent::latest_stop_reason(&db, &task.task_id)
                        .map_err(|error| error.reason_code.to_string())?;
                    return Ok(homogeneous_target_terminal(&db, &task, reason.as_deref())?);
                }
                pool::reclaim_expired_for_task(&db, &task)
                    .map_err(|error| error.reason_code.to_string())?;
                // Frozen capacity, never slice-count-based expansion. Dead audit identities
                // may be replaced inside the same capacity before the next decision card.
                pool::provision_workers(&db, &task, actor.lease_epoch, task.allowed_parallel)
                    .map_err(|error| error.reason_code.to_string())?;
                if crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(
                    &db,
                    &actor.scan_id,
                    actor.attempt_number,
                )? {
                    for id in store::list_assignment_ids_in_state(
                        &db,
                        &actor.root_run_id,
                        &["assigned", "running"],
                    )
                    .map_err(|e| e.detail)?
                    {
                        let key: String = db
                            .query_row(
                                "SELECT child_run_id FROM agent_assignments WHERE id=?1",
                                [&id],
                                |r| r.get(0),
                            )
                            .map_err(|e| e.to_string())?;
                        if !started.insert(key) {
                            continue;
                        }
                        let transport = SpecialistTransportContext {
                            supervision: context.supervision.clone(),
                            db_path: &context.db_path,
                            scan_id: &context.scan_id,
                            attempt_number: context.attempt_number,
                            target_key: &context.target_url,
                            run_id: &actor.root_run_id,
                            environment: &context.environment,
                            proxy: context.proxy.as_deref(),
                            usage_dir: &context.target_dir,
                            deadline: None,
                        };
                        let errors = failures.clone();
                        let worker_control = control.clone();
                        let configuration=context.tool_configuration.clone();
                        let scoped_context=context.clone();
                        scope.spawn(move || {
                            let lane=db::open(&scoped_context.db_path).and_then(|db|db.query_row("SELECT lane FROM agent_assignments WHERE id=?1",[&id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string()));
                            let executed=match lane {
                                Ok(lane) if lane=="target_touching" => crate::scanning::homogeneous_execution::run_target(&scoped_context,actor,&id),
                                Ok(_) => native_homogeneous_model_worker(&transport,actor,&id,&worker_control,configuration.as_ref()),
                                Err(e)=>Err(e),
                            };
                            if let Err(error) = executed {
                                append_runner_log(
                                    &context.log_path,
                                    &format!("homogeneous_worker_failed:{id}:{error}"),
                                );
                                if let Ok(mut errors) = errors.lock() {
                                    errors.push(error);
                                }
                            }
                        });
                    }
                    let waiting:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1 AND scheduling_model='homogeneous_workers' AND state IN ('assigned','running')) AND NOT EXISTS(SELECT 1 FROM agent_assignments WHERE coordinator_run_id=?1 AND scheduling_model='homogeneous_workers' AND state='reported') AND NOT EXISTS(SELECT 1 FROM agent_task_slices WHERE parent_task_id=?2 AND (state='pending' OR (state='rejected' AND retry_count<max_retries))) AND NOT EXISTS(SELECT 1 FROM agent_human_directive_gated WHERE parent_task_id=?2 AND state IN ('rule_screened','awaiting_coordinator'))",params![actor.root_run_id,task.task_id],|r|r.get(0)).map_err(|e|e.to_string())?;
                    if waiting {
                        drop(db);
                        drop(control_guard);
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        continue;
                    }
                }
                if task.schema_fail_streak >= gate::STREAK_LIMIT
                    || task.hard_reject_streak >= gate::STREAK_LIMIT
                {
                    // The existing streak gate forbids another paid model request.
                    let fallback = apply::run_coordinator_turn(&db, actor, "")
                        .map_err(|error| error.reason_code.to_string())?;
                    append_runner_log(
                        &context.log_path,
                        &format!("root_homogeneous_fallback:{fallback:?}"),
                    );
                    if !crate::agent_runtime::multi_agent::homogeneous::model_worker::enabled(
                        &db,
                        &actor.scan_id,
                        actor.attempt_number,
                    )? {
                        append_runner_log(
                            &context.log_path,
                            &native_homogeneous_drain_workers(&db, actor),
                        );
                    }
                    let next = store::load_task_by_root_run(&db, &actor.root_run_id)
                        .map_err(|error| error.reason_code.to_string())?
                        .ok_or("homogeneous_root_task_missing")?;
                    if next.schema_fail_streak >= gate::STREAK_LIMIT
                        || next.hard_reject_streak >= gate::STREAK_LIMIT
                    {
                        return Ok(AgentTargetOutcome::incomplete(
                            "同质主管确定性降级等待可推进切片；已保留付费回执与拒绝原因",
                        ));
                    }
                    continue;
                }
                drop(db);
                native_coordinator_tick(context, actor)?;
                drop(control_guard);
                if !started.is_empty() {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        })()
    });
    match result {
        Ok(outcome) => outcome,
        Err(error) if error.contains("reconciliation") || error.contains("indeterminate") => {
            AgentTargetOutcome::Incomplete(AgentStop::new(
                terminal_code::REQUEST_RECONCILIATION_REQUIRED,
                error,
            ))
        }
        Err(error)
            if error.starts_with("budget_")
                || error == "child_budget_reservation_exceeded_or_stale" =>
        {
            AgentTargetOutcome::limited(error)
        }
        Err(error) => AgentTargetOutcome::failed(format!("homogeneous_root_failed:{error}")),
    }
}

fn homogeneous_target_terminal(
    db: &rusqlite::Connection,
    task: &crate::agent_runtime::multi_agent::homogeneous::types::HomogeneousTask,
    stop_reason: Option<&str>,
) -> Result<AgentTargetOutcome, String> {
    use crate::agent_runtime::multi_agent::homogeneous::store;
    if task.state == "cancelled" || stop_reason == Some("user_cancelled") {
        return Ok(AgentTargetOutcome::Cancelled);
    }
    if stop_reason == Some("budget_exhausted") {
        return Ok(AgentTargetOutcome::limited(
            "同质主管按预算耗尽收尾；已验收的只读证据保持可审计",
        ));
    }
    let slices =
        store::load_slices(db, &task.task_id).map_err(|error| error.reason_code.to_string())?;
    let accepted = slices
        .iter()
        .filter(|slice| slice.state == "accepted")
        .count();
    if stop_reason == Some("goal_reached") && accepted > 0 {
        let requests = crate::agent_runtime::multi_agent::budget::balance(
            db,
            &task.root_run_id,
            None,
            "model_requests",
        )?
        .consumed;
        let input = crate::agent_runtime::multi_agent::budget::balance(
            db,
            &task.root_run_id,
            None,
            "model_input_tokens",
        )?
        .consumed;
        let output = crate::agent_runtime::multi_agent::budget::balance(
            db,
            &task.root_run_id,
            None,
            "model_output_tokens",
        )?
        .consumed;
        let tokens = input.checked_add(output).ok_or("budget_amount_overflow")?;
        return Ok(AgentTargetOutcome::BoundedCompleted(AgentCompletion {
            summary: format!("同质主管已验收 {accepted} 条只读切片并按 goal_reached 收尾"),
            terminal_code: AGENT_STOP_DERIVED,
            ledger_reported: false,
            model_requests: requests,
            total_tokens: tokens,
            verified_tool_results: 0,
            covered_families: Vec::new(),
            uncovered_families: Vec::new(),
            confirmed_findings: 0,
        }));
    }
    Ok(AgentTargetOutcome::incomplete(format!(
        "同质主管停止：{}；已验收 {accepted} 条只读切片",
        stop_reason.unwrap_or(&task.state)
    )))
}
