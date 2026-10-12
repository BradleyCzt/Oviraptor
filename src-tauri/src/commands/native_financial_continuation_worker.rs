// Dedicated fresh frozen-evidence assessment. No generic pipeline or old queues.
fn native_financial_continuation_context_in(
    db: &rusqlite::Connection,
    db_path: &Path,
    scan: &str,
    root: &str,
) -> Result<
    (
        AgentRunContext,
        crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    ),
    String,
> {
    use crate::agent_runtime::{
        multi_agent::{budget, lease},
        store,
    };
    if db.is_autocommit() {
        return Err("financial_continuation_requires_snapshot".into());
    }
    let birth = verify_financial_continuation_birth_on(db, root)?;
    native_financial_continuation_require_authority_in(db, &birth)?;
    if birth["scanId"] != scan {
        return Err("financial_continuation_worker_scope_invalid".into());
    }
    let (raw, target, attempt): (String, String, i64) = db
        .query_row(
            "SELECT plan_json,target_url,attempt_number FROM agent_runs WHERE id=?1 AND scan_id=?2
         AND status IN ('prepared','running') AND cancel_requested_at=''",
            params![root, scan],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| "financial_continuation_worker_scope_invalid")?;
    let value: JsonValue =
        serde_json::from_str(&raw).map_err(|_| "financial_continuation_plan_invalid")?;
    let plan =
        AgentExecutionPlan::from_json(&value).ok_or("financial_continuation_plan_invalid")?;
    if plan.as_json() != value
        || plan.attempt_number != attempt
        || plan.target_url != target
        || plan.backend != AgentBackendKind::Native
        || plan.hash() != birth["newPlanHash"]
    {
        return Err("financial_continuation_plan_changed".into());
    }
    let mode = crate::agent_runtime::web_mode::root::read(db, root)?
        .ok_or("financial_continuation_mode_missing")?;
    let private_mode = private_web_mode_on(db, scan, attempt)?;
    if mode.fact() != private_mode.fact()
        || mode.mode() != crate::agent_runtime::web_mode::WebMode::Multi
        || mode.local_deliberation().is_some()
        || mode.bootstrap_dispatch().is_some()
        || mode.live_budget_observation().is_some()
    {
        return Err("financial_continuation_requires_plain_root".into());
    }
    let target_dir = resolve_agent_evidence_location(db, root)?;
    if Some(target_dir.as_path()) != birth["newTargetDir"].as_str().map(Path::new) {
        return Err("financial_continuation_evidence_location_changed".into());
    }
    checked_evidence_directory(Path::new(&private_mode.fact().work_dir), &target_dir, scan)?;
    let bytes = financial_read_evidence(&target_dir.join("frontend-evidence.json"))?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| "financial_continuation_evidence_invalid")?;
    if store::stable_hash(text) != birth["frontendEvidenceHash"] {
        return Err("financial_continuation_evidence_changed".into());
    }
    let evidence: JsonValue =
        serde_json::from_slice(&bytes).map_err(|_| "financial_continuation_evidence_invalid")?;
    if !evidence.is_object() || evidence.get("error").is_some() || evidence["url"] != target {
        return Err("financial_continuation_evidence_invalid".into());
    }
    let actor = db
        .query_row(
            "SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,
        fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
            [root],
            |r| {
                Ok(lease::CoordinatorLease {
                    scan_id: r.get(0)?,
                    attempt_number: r.get(1)?,
                    target_key: r.get(2)?,
                    root_run_id: r.get(3)?,
                    lease_epoch: r.get(4)?,
                    fencing_token: r.get(5)?,
                    lease_expires_at: r.get(6)?,
                })
            },
        )
        .map_err(|_| "financial_continuation_coordinator_missing")?;
    if actor.scan_id != scan || actor.attempt_number != attempt || actor.target_key != target {
        return Err("financial_continuation_coordinator_changed".into());
    }
    lease::validate_coordinator_lease(db, &actor)?;
    lease::require_executable_coordinator(db, &actor)?;
    budget::root::RootOwner::load_original(db, root)?.require_original_coordinator(db, &actor)?;
    // Use today's configured transport only after matching every original
    // model/provider/key/proxy/output/schema field; no hook is started here.
    let settings = sentinel_settings(db);
    let environment = model_runtime_env(&settings)?;
    let mut candidates = vec![None];
    candidates.extend(
        approved_agent_proxies(&settings)
            .into_iter()
            .map(|(_, url)| Some(url)),
    );
    let source = birth["sourceRootRunId"]
        .as_str()
        .ok_or("financial_continuation_birth_invalid")?;
    let mut matched_proxy = None;
    for proxy in candidates {
        let profile = agent_model_profile(&environment, proxy.as_deref())?;
        let request = json!({"model":profile.model,"endpointBinding":store::stable_hash(&profile.endpoint),
            "deployment":if profile.local {"local"} else {"cloud"},
            "credentialBinding":store::stable_hash(profile.bearer().unwrap_or("")),
            "proxyBinding":store::stable_hash(profile.proxy.as_deref().unwrap_or("")),
            "maxContextTokens":profile.max_context_tokens,"maxOutputTokens":profile.max_output_tokens.unwrap_or(1024).min(2048),
            "temperature":crate::agent_runtime::model::gateway::ModelRequest::default().temperature,
            "timeoutPolicy":"original_root_clock","schemaHash":store::stable_hash(
                &budget::root::model::tick::decision::DecisionSummary::schema().to_string())});
        if native_coordinator_require_original_model_on(db, source, &request).is_ok() {
            matched_proxy = Some(proxy);
            break;
        }
    }
    let proxy = matched_proxy.ok_or("financial_continuation_original_model_changed")?;
    let (_, sessions) = crate::auth_session::validated_scan_identities(db, scan, &target)?;
    let identities = if sessions.is_empty() {
        vec![AgentIdentity::anonymous()]
    } else {
        sessions.into_iter().map(AgentIdentity::scoped).collect()
    };
    let context = AgentRunContext {
        tool_configuration: None,
        supervision: None,
        external_surface: false,
        scan_id: scan.into(),
        attempt_number: attempt,
        target_url: target.clone(),
        target_dir: target_dir.clone(),
        db_path: db_path.into(),
        #[cfg(test)]
        route: FrontendRoute {
            url: target,
            score: 0,
            mode: plan.mode.clone(),
            surface: plan.surface.clone(),
            reasons: vec!["explicit financial continuation frozen-evidence assessment".into()],
        },
        execution_plan: plan,
        evidence,
        capabilities: json!({"schemaVersion":1,"targetActionsGranted":0}),
        identities,
        proxy,
        log_path: target_dir.join("financial-continuation.log"),
        environment,
        browser: None,
        resume: false,
        run: Some(AgentRunLedger {
            db_path: db_path.into(),
            run_id: root.into(),
        }),
        run_budget: None,
        plan_rejection: None,
    };
    Ok((context, actor))
}

fn native_financial_continuation_work_once(
    db_path: &Path,
    scan: &str,
    new_root: &str,
    owner: crate::agent_runtime::multi_agent::parent_invocation_owner::ParentInvocationOwner,
) -> Result<JsonValue, String> {
    use crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor;
    let db = db::open(db_path)?;
    let (mut context, actor) = {
        let tx =
            rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
        let original = native_financial_continuation_context_in(&tx, db_path, scan, new_root)?;
        let changed = tx
            .execute(
                "UPDATE agent_runs SET status='running',
            started_at=CASE WHEN started_at='' THEN datetime('now','localtime') ELSE started_at END,
            updated_at=datetime('now','localtime') WHERE id=?1 AND status IN ('prepared','running')
            AND cancel_requested_at=''",
                [new_root],
            )
            .map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err("financial_continuation_activation_changed".into());
        }
        native_financial_continuation_context_in(&tx, db_path, scan, new_root)?;
        tx.commit().map_err(|e| e.to_string())?;
        original
    };
    let supervisor = WorkerSupervisor::start_owned(db_path, &actor, owner)?;
    context.supervision = Some(supervisor.ticket());
    // This single plain Tick owns admission, once-only SDK, real costs, strict
    // DecisionSummary and publication. Never prepare a Mapper or replay an old queue.
    let receipt = native_coordinator_bootstrap(&context, &actor)?;
    supervisor.check()?;
    let result = {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        native_coordinator_tick_authority(&snapshot, &context, &actor)?;
        if receipt.saved.local_step.is_some()
            || receipt.tick.published(&snapshot, &receipt.saved)? != Some(receipt.event_sequence)
        {
            return Err("financial_continuation_publication_unverified".into());
        }
        native_financial_continuation_published_in(&snapshot, scan, new_root)?
            .ok_or("financial_continuation_publication_unverified")?
    };
    // Fault injection runs only in a separately launched test process. Exiting
    // here leaves the genuine paid publication durable, with no exit receipt.
    #[cfg(test)]
    if std::env::var("OVIRAPTOR_TEST_FINANCIAL_EXIT_AFTER_PUBLICATION")
        .ok()
        .as_deref()
        == Some(new_root)
    {
        std::process::exit(86);
    }
    drop(supervisor);
    native_financial_continuation_finish(&context, &actor, &receipt)?;
    Ok(result)
}

// Original paid publication is historical evidence, even after this dedicated
// attempt stops. Reentry reads it before any activation, owner, C or SDK.
fn native_financial_continuation_published_in(
    db: &rusqlite::Connection,
    scan: &str,
    root: &str,
) -> Result<Option<JsonValue>, String> {
    if db.is_autocommit() {
        return Err("financial_continuation_requires_snapshot".into());
    }
    let birth = verify_financial_continuation_birth_on(db, root)?;
    if birth["scanId"] != scan {
        return Err("financial_continuation_worker_scope_invalid".into());
    }
    let sequences: Vec<i64> = db
        .prepare(
            "SELECT json_extract(fact_json,'$.eventSequence')
        FROM agent_root_tick_receipts WHERE root_run_id=?1 AND phase='publication' ORDER BY round",
        )
        .map_err(|e| e.to_string())?
        .query_map([root], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| e.to_string())?;
    if sequences.is_empty() {
        return Ok(None);
    }
    if sequences.len() != 1 {
        return Err("financial_continuation_publication_count_invalid".into());
    }
    let event = sequences[0];
    let p =
        crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::project_publication(
            db, root, event,
        )?;
    if p.scan != scan || p.attempt != birth["newAttemptNumber"] || p.record["round"] != 1 {
        return Err("financial_continuation_publication_scope_changed".into());
    }
    Ok(Some(
        json!({"schema":1,"scanId":scan,"attemptNumber":p.attempt,"rootRunId":root,
        "status":"fresh_root_assessment_published","assessmentCompleted":true,
        "taskPurpose":"coordinator_financial_continuation_v1","eventSequence":event,
        "summary":p.record["summary"],"callId":p.record["callId"],"round":p.record["round"],
        "targetActionsExecuted":0,"oldSemanticReplayAllowed":false}),
    ))
}

fn native_financial_continuation_work(
    db_path: &Path,
    scan: &str,
    new_root: &str,
) -> Result<JsonValue, String> {
    let db = db::open(db_path)?;
    let existing = {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        native_financial_continuation_published_in(&snapshot, scan, new_root)?
    };
    if let Some(result) = existing {
        let pending = {
            let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
            let (status, current): (String, i64) = snapshot
                .query_row(
                    "SELECT status,attempt_count FROM sentinel_scans WHERE id=?1",
                    [scan],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(|_| "financial_continuation_current_scope_invalid")?;
            if status == "scanning" && Some(current) == result["attemptNumber"].as_i64() {
                let (context, actor) =
                    native_financial_continuation_context_in(&snapshot, db_path, scan, new_root)?;
                let event = result["eventSequence"]
                    .as_i64()
                    .ok_or("financial_continuation_publication_unverified")?;
                let (tick,saved) = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::original_publication_for_continuation(
                    &snapshot,new_root,event)?;
                let receipt = NativeCoordinatorTickReceipt {
                    summary: saved.summary.clone(),
                    event_sequence: event,
                    replayed: true,
                    tick,
                    saved,
                };
                Some((context, actor, receipt))
            } else {
                None
            }
        };
        if let Some((context, actor, receipt)) = pending {
            native_financial_continuation_finish(&context, &actor, &receipt)?;
        }
        return Ok(result);
    }
    let attempt = {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        let birth = verify_financial_continuation_birth_on(&snapshot, new_root)?;
        birth["newAttemptNumber"]
            .as_i64()
            .ok_or("financial_continuation_birth_invalid")?
    };
    // Claim before activation. A duplicate observer cannot stop the original
    // living worker or publish a collateral partial status on its behalf.
    let owner =
        crate::agent_runtime::multi_agent::parent_invocation_owner::ParentInvocationOwner::claim(
            db_path, scan, attempt, new_root,
        )?;
    let target = {
        let snapshot = db.unchecked_transaction().map_err(|e| e.to_string())?;
        let birth = verify_financial_continuation_birth_on(&snapshot, new_root)?;
        birth["financialSource"]["targetKey"]
            .as_str()
            .ok_or("financial_continuation_worker_scope_invalid")?
            .to_string()
    };
    let _target_owner = claim_native_invocation(db_path, scan, attempt, "target", &target)?;
    match native_financial_continuation_work_once(db_path, scan, new_root, owner) {
        Ok(result) => Ok(result),
        Err(error) => {
            // Preserve all original invoices/reservations. The fixed status is
            // an honest request for review, never a synthetic completion proof.
            native_financial_continuation_stop_attempt(&db, scan, new_root)?;
            Err(error)
        }
    }
}

// Exact plain SDK payload headroom, checked before issuance commits. This
// reader never starts a model hook, claims a Tick or appends reservations.
