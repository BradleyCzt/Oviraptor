// Actual fresh creator/startup/Root/SDK/owned terminal pipeline. Historical
// rows are disposable negative inputs only, never execution or cost grants.
fn bootstrap_original_actor(
    h: &AgentHarness,
) -> crate::agent_runtime::multi_agent::lease::CoordinatorLease {
    let db = db::open(&h.db_path).unwrap();
    db.query_row("SELECT scan_id,attempt_number,target_key,root_run_id,lease_epoch,fencing_token,lease_expires_at FROM agent_coordinator_leases WHERE root_run_id=?1",
        [&h.context.run.as_ref().unwrap().run_id],|r|Ok(crate::agent_runtime::multi_agent::lease::CoordinatorLease {
            scan_id:r.get(0)?,attempt_number:r.get(1)?,target_key:r.get(2)?,root_run_id:r.get(3)?,lease_epoch:r.get(4)?,fencing_token:r.get(5)?,lease_expires_at:r.get(6)?,
        })).unwrap()
}
fn bootstrap_original_sources(
    db: &rusqlite::Connection,
) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    single_finally_physical(db)
        .into_iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "agent_root_budget_attempts"
                    | "agent_budget_limits"
                    | "agent_budget_clock_origins"
                    | "agent_root_model_journal"
                    | "agent_root_tick_receipts"
                    | "native_web_mode_receipts"
                    | "agent_root_mode_definitions"
            )
        })
        .collect()
}
fn bootstrap_original_history(db: &rusqlite::Connection, kind: &str, c: &AgentRunContext) {
    let root = &c.run.as_ref().unwrap().run_id;
    let (role, lane) = match kind {
        "readonly" => ("spa_api_mapper", "read_only_analysis"),
        "target" => ("web_executor", "target_touching"),
        "public" => ("external_surface", "target_touching"),
        _ => unreachable!(),
    };
    // Incomplete old native history: no child budget, SDK, capability or exit is
    // issued. A current owner must refuse its reuse instead of inventing proof.
    db.execute("INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,role,parent_run_id,root_run_id,lane,status)
        VALUES('historical-bootstrap-child',?1,1,?2,?3,?4,?4,?5,'prepared')",params![c.scan_id,c.target_url,role,root,lane]).unwrap();
    if kind == "readonly" {
        db.execute("INSERT INTO agent_assignments(id,coordinator_run_id,child_run_id,role,lane,target_key,dedup_key,lease_epoch,fencing_token)
            VALUES('historical-bootstrap-assignment',?1,'historical-bootstrap-child',?2,?3,?4,'historical-bootstrap',0,'original-historical-fence')",params![root,role,lane,c.target_url]).unwrap();
    }
}
fn bootstrap_original_case(kind: &str, expected: &str) {
    use crate::agent_runtime::multi_agent::{budget, supervisor};
    let mut h = fresh_multi_production_harness(&format!("bootstrap-original-{kind}"));
    freeze_fresh_multi_production_harness(&mut h);
    retarget_model(
        &mut h,
        vec![proposal_model_response(&root_tick_valid_text(
            "paid original before recovery refusal",
        ))],
    );
    let actor = bootstrap_original_actor(&h);
    // This freshly born Root requires its actual private birth issuer. Keep
    // the same original C; do not retrofit a protocol or unsigned supervisor.
    let issuer = {
        let db = db::open(&h.db_path).unwrap();
        original_withdrawal_issuer_on(&db, &actor)
            .unwrap()
            .expect("fresh bootstrap Root must retain its original signed issuer")
    };
    let owner = crate::agent_runtime::multi_agent::parent_invocation_owner::ParentInvocationOwner::claim(
        &h.db_path, &actor.scan_id, actor.attempt_number, &actor.root_run_id,
    ).unwrap();
    let parent = supervisor::WorkerSupervisor::start_owned_signed(
        &h.db_path, &actor, owner, issuer,
    ).unwrap();
    h.context.supervision = Some(parent.ticket());
    let decision = native_coordinator_tick(&h.context, &actor).unwrap();
    assert!(!decision.replayed);
    drop(parent);
    h.context.supervision = None;
    assert_eq!(h.model_seen.lock().unwrap().len(), 1);
    assert!(h.site_seen.lock().unwrap().is_empty());
    let db = db::open(&h.db_path).unwrap();
    let root = &actor.root_run_id;
    let original_cost = |dimension| budget::balance(&db, root, None, dimension).unwrap();
    assert_eq!(original_cost("model_requests").consumed, 1);
    assert!(original_cost("model_input_tokens").consumed > 0);
    assert!(original_cost("model_output_tokens").consumed > 0);
    let fees: Vec<_> = budget::DIMENSIONS
        .iter()
        .filter(|d| **d != "wall_time_ms")
        .map(|d| {
            let b = original_cost(d);
            (*d, b.consumed, b.reserved, b.indeterminate)
        })
        .collect();
    let sources = bootstrap_original_sources(&db);
    assert_eq!(sources.len(), 7);
    assert!(sources.iter().all(|(_, rows)| !rows.is_empty()));
    bootstrap_original_history(&db, kind, &h.context);
    let history=single_finally_physical(&db).into_iter().filter(|(name,_)|matches!(name.as_str(),"agent_runs"|"agent_assignments"))
        .map(|(name,rows)| (name,rows.into_iter().filter(|r| matches!(r.get(1),Some(rusqlite::types::Value::Text(id)) if id.starts_with("historical-bootstrap"))).collect::<Vec<_>>())).collect::<Vec<_>>();
    let prepared = PreparedFrontendTarget {
        position: 1,
        route: h.context.route.clone(),
        target_dir: h.context.target_dir.clone(),
        proxy: None,
        browser: None,
    };
    let settings = json!({});
    let owned = run_agent_target(
        &prepared,
        AgentTargetExecution {
            db_path: &h.db_path,
            scan_id: &h.context.scan_id,
            attempt_number: 1,
            settings: &settings,
            environment: &h.context.environment,
            adaptive: &AgentBudgetSettings::from_json(&settings),
            log_path: &h.context.log_path,
        },
    )
    .unwrap();
    assert!(
        matches!(owned.outcome, AgentTargetOutcome::ResumeIncompatible(_)),
        "{kind}: {:?}",
        owned.outcome
    );
    assert_eq!(
        owned.outcome.terminal_code(),
        AGENT_STOP_RESUME_INCOMPATIBLE
    );
    assert_eq!(owned.outcome.detail(), expected);
    assert_eq!(owned.original_terminal.root_run_id.as_ref(), Some(root));
    assert_eq!(
        h.model_seen.lock().unwrap().len(),
        1,
        "no SDK retry or next role after refusal"
    );
    assert!(h.site_seen.lock().unwrap().is_empty());
    assert!(
        NativeAgentState::load(&h.db_path, &h.context.scan_id, &h.context.target_url)
            .unwrap()
            .is_none()
    );
    let tx = db.unchecked_transaction().unwrap();
    budget::clock::FinalClock::verify_original_exit(&tx, root).unwrap();
    tx.rollback().unwrap();
    assert_eq!(bootstrap_original_sources(&db), sources);
    for (d, consumed, reserved, indeterminate) in fees {
        let b = original_cost(d);
        assert_eq!(
            (b.consumed, b.reserved, b.indeterminate),
            (consumed, reserved, indeterminate),
            "{d}"
        );
    }
    let after_history=single_finally_physical(&db).into_iter().filter(|(name,_)|matches!(name.as_str(),"agent_runs"|"agent_assignments"))
        .map(|(name,rows)| (name,rows.into_iter().filter(|r| matches!(r.get(1),Some(rusqlite::types::Value::Text(id)) if id.starts_with("historical-bootstrap"))).collect::<Vec<_>>())).collect::<Vec<_>>();
    assert_eq!(
        after_history, history,
        "no relabel, cleanup, grant or fee adoption for historical rows"
    );
    let mut tally = AgentPipelineTally::default();
    assert!(
        record_owned_agent_target_outcome(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            &owned,
            &mut tally
        ),
        "{kind}: owned publication rejected"
    );
    assert_eq!(
        (tally.manual_review, tally.failed, tally.counted()),
        (1, 0, 1)
    );
    let (status,reasons):(String,String)=db.query_row("SELECT status,COALESCE(routing_reason,'') FROM sentinel_targets WHERE scan_id=?1 AND url=?2",params![h.context.scan_id,h.context.target_url],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
    assert_eq!(status, "resume_incompatible");
    assert!(reasons.contains("需要重新执行"));
    let terminal = read_agent_checkpoint(
        &h.db_path,
        &h.context.scan_id,
        &h.context.target_url,
        "agent_terminal",
    );
    assert_eq!(terminal["status"], status);
    assert_eq!(terminal["code"], AGENT_STOP_RESUME_INCOMPATIBLE);
    assert_eq!(terminal["stop"]["code"], AGENT_STOP_RESUME_INCOMPATIBLE);
    assert!(findings_for(&h.db_path, AGENT_VULNERABILITY_STAGE).is_empty());
    assert!(findings_for(&h.db_path, AGENT_COVERAGE_STAGE).is_empty());
    let physical = single_finally_physical(&db);
    assert!(record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(
        (tally.manual_review, tally.failed, tally.counted()),
        (1, 0, 1)
    );
    assert!(
        single_finally_physical(&db) == physical,
        "owned replay must preserve every typed row/rowid/fee"
    );
    for outcome in [
        AgentTargetOutcome::Completed(AgentCompletion::without_ledger(
            "forged completion after original refusal",
            AGENT_STOP_FINISH,
        )),
        AgentTargetOutcome::resume_incompatible("changed original refusal reason"),
        AgentTargetOutcome::failed("forged replacement failure"),
    ] {
        assert!(!record_agent_target_outcome(
            &h.db_path,
            &h.context.scan_id,
            &h.context.route,
            outcome,
            &mut tally
        ));
        assert_eq!(
            (tally.manual_review, tally.failed, tally.counted()),
            (1, 0, 1)
        );
        assert!(
            single_finally_physical(&db) == physical,
            "mismatched original outcome must not rewrite any row/rowid/fee"
        );
    }
    // Isolated current-attempt rotation only, not a complete restart claim.
    db.execute(
        "UPDATE sentinel_scans SET attempt_count=2 WHERE id=?1",
        [&h.context.scan_id],
    )
    .unwrap();
    let replaced = single_finally_physical(&db);
    assert!(!record_owned_agent_target_outcome(
        &h.db_path,
        &h.context.scan_id,
        &h.context.route,
        &owned,
        &mut tally
    ));
    assert_eq!(
        (tally.manual_review, tally.failed, tally.counted()),
        (1, 0, 1)
    );
    assert!(
        single_finally_physical(&db) == replaced,
        "late owned callback cannot write current or original rows"
    );
    assert_eq!(h.model_seen.lock().unwrap().len(), 1);
    assert!(h.site_seen.lock().unwrap().is_empty());
    drop(owned);
    drop(db);
    fs::remove_dir_all(h.root).unwrap();
}
#[test]
fn bootstrap_recovery_refusals_require_a_fresh_attempt_instead_of_reporting_tool_failure() {
    for (kind, code) in [
        (
            "readonly",
            "readonly_fencing_changed_requires_fresh_attempt",
        ),
        ("target", "target_execution_recovery_requires_fresh_attempt"),
        ("public", "public_surface_recovery_requires_fresh_attempt"),
    ] {
        bootstrap_original_case(kind, code);
    }
    assert!(matches!(
        multi_agent_bootstrap_outcome("bootstrap_storage_error"),
        AgentTargetOutcome::Failed(_)
    ));
}
