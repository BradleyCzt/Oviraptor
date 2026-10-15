// Actual SDK producer; no hand-made response, fee, worker or exit receipt.
struct SourceExhaustedOriginal {
    root: PathBuf,
    db: rusqlite::Connection,
    actor: crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    child: crate::agent_runtime::multi_agent::scheduler::ScheduledChild,
    model: ModelRuntimeEnv,
    parent: crate::agent_runtime::multi_agent::supervisor::WorkerSupervisor,
    calls: Arc<AtomicUsize>,
    stop: Arc<std::sync::atomic::AtomicBool>,
}
impl SourceExhaustedOriginal {
    fn context<'a>(&'a self, path: &'a Path, work: &'a Path) -> SpecialistTransportContext<'a> {
        SpecialistTransportContext {
            supervision: Some(self.parent.ticket()),
            db_path: path,
            scan_id: &self.actor.scan_id,
            attempt_number: 1,
            target_key: &self.actor.target_key,
            run_id: &self.actor.root_run_id,
            environment: &self.model,
            proxy: None,
            usage_dir: work,
            deadline: None,
        }
    }
    fn close(&self) -> Result<(), String> {
        let path = self.root.join("oviraptor.sqlite3");
        let work = self.root.join("attempt-0001");
        close_exhausted_source_tool_assignment(
            &self.db,
            &self.context(&path, &work),
            &self.actor,
            &self.child,
        )
    }
    fn cleanup(self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(self.parent);
        drop(self.db);
        fs::remove_dir_all(self.root).unwrap();
    }
}
fn source_exhausted_original() -> SourceExhaustedOriginal {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{scheduler, source, supervisor::WorkerSupervisor},
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let (port, _, stop) = crate::scanning::agent_tests::spawn_endpoint(Arc::new(move |_| {
        let number = observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (200,"application/json",json!({"choices":[{"message":{"role":"assistant","tool_calls":[{"id":format!("original-exhausted-{number}"),
            "type":"function","function":{"name":"repo.inventory","arguments":"{}"}}]},"finish_reason":"tool_calls"}],
            "usage":{"prompt_tokens":10,"completion_tokens":10,"total_tokens":20}}).to_string())
    }));
    let model = source_specialist_test_environment(port);
    let (root, db, _, actor) = source_tool_true_born_fixture_model(Some(&model));
    db.execute(
        "UPDATE agent_runs SET status='running',started_at=datetime('now','localtime') WHERE id=?1",
        [&actor.root_run_id],
    )
    .unwrap();
    let role = AgentRole::RepoMapper;
    crate::agent_runtime::multi_agent::directive::source_guidance::freeze(&db, &actor, role, true)
        .unwrap();
    let slice = source::tool_task_slice(&db, &actor, role, 1).unwrap();
    let child = scheduler::schedule_child(
        &db,
        &actor,
        role,
        AgentLane::ReadOnlyAnalysis,
        "source_tools_ready",
        &slice,
        1,
        &source::tool_capabilities(role).unwrap(),
        24_000,
        3,
    )
    .unwrap();
    scheduler::mark_child_running(&db, &actor, &child).unwrap();
    let path = root.join("oviraptor.sqlite3");
    let work = root.join("attempt-0001");
    let parent = WorkerSupervisor::start(&path, &actor).unwrap();
    let f = SourceExhaustedOriginal {
        root,
        db,
        actor,
        child,
        model,
        parent,
        calls,
        stop,
    };
    // Let the real three rounds and tools commit. Only this disposable failure
    // of the final settlement leaves their original live obligation available.
    f.db.execute_batch("CREATE TRIGGER exhausted_initial_fault BEFORE UPDATE OF budget_settled_at ON agent_assignments
        WHEN NEW.budget_settled_at<>'' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    let error =
        execute_source_tool_assignment(&f.db, &f.context(&path, &work), &f.actor, &f.child, &slice)
            .unwrap_err();
    assert!(
        error.contains("child_budget_settlement_replayed_or_stale"),
        "{error}"
    );
    f.db.execute_batch("DROP TRIGGER exhausted_initial_fault")
        .unwrap();
    assert_eq!(f.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    f
}

#[test]
fn source_exhausted_original_known_closure_is_atomic_and_keeps_failed_business_state() {
    use crate::agent_runtime::{
        execution_owner,
        multi_agent::{budget, source_rounds},
    };
    let f = source_exhausted_original();
    for fault in [
        "BEFORE UPDATE OF budget_settled_at ON agent_assignments WHEN NEW.budget_settled_at<>'' BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE UPDATE ON agent_budget_ledger BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE UPDATE OF used_tokens ON agent_runs BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE UPDATE OF status ON agent_runs WHEN NEW.status='terminal' BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE UPDATE OF revoked_at ON agent_capability_leases BEGIN SELECT RAISE(IGNORE); END;",
        "BEFORE DELETE ON agent_lane_leases BEGIN SELECT RAISE(IGNORE); END;",
        "AFTER UPDATE OF state ON agent_assignments WHEN NEW.state='failed' BEGIN UPDATE agent_budget_ledger SET spent_tokens=spent_tokens+1; END;",
        "AFTER UPDATE OF budget_settled_at ON agent_assignments WHEN NEW.budget_settled_at<>'' BEGIN UPDATE agent_runs SET cancel_requested_at='injected-cancel' WHERE role='coordinator'; END;",
    ] {
        f.db.execute_batch(&format!("CREATE TRIGGER exhausted_fault {fault}")).unwrap();
        let before=source_exit_snapshot(&f.db);
        assert!(f.close().is_err(),"{fault}");
        assert_eq!(source_exit_snapshot(&f.db),before,"failed closure must roll back every typed row: {fault}");
        f.db.execute_batch("DROP TRIGGER exhausted_fault").unwrap();
    }
    let path = f.root.join("oviraptor.sqlite3");
    let busy = execution_owner::claim_native_invocation(
        &path,
        &f.actor.scan_id,
        1,
        "source-round-sdk",
        &f.child.run_id,
    )
    .unwrap();
    let before = source_exit_snapshot(&f.db);
    let error = f.close().unwrap_err();
    assert!(error.contains("source_round_transport_not_idle"), "{error}");
    assert_eq!(source_exit_snapshot(&f.db), before);
    drop(busy);
    let tx = f.db.unchecked_transaction().unwrap();
    let usage = source_rounds::audit_exhausted(&tx, &f.actor, &f.child).unwrap();
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.model_requests
        ),
        (30, 30, 60, 3)
    );
    tx.rollback().unwrap();
    f.close().unwrap();
    let after = source_exit_snapshot(&f.db);
    for name in [
        "agent_source_model_rounds",
        "agent_source_tool_receipts",
        "agent_model_cost_facts",
        "agent_events",
        "agent_messages",
    ] {
        assert_eq!(
            before.iter().find(|(n, _)| n == name),
            after.iter().find(|(n, _)| n == name),
            "original {name}"
        );
    }
    let ledger:(i64,i64,i64,i64)=f.db.query_row("SELECT spent_tokens,spent_requests,reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",
        [&f.actor.root_run_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(ledger, (60, 3, 0, 0));
    for (dimension, consumed) in [
        ("model_requests", 3),
        ("model_input_tokens", 30),
        ("model_output_tokens", 30),
    ] {
        let balance = budget::balance(
            &f.db,
            &f.actor.root_run_id,
            Some(&f.child.assignment_id),
            dimension,
        )
        .unwrap();
        assert_eq!(
            (balance.consumed, balance.reserved, balance.indeterminate),
            (consumed, 0, 0)
        );
    }
    assert!(
        f.close().is_err(),
        "closed failure cannot mint another closure or new work"
    );
    assert_eq!(source_exit_snapshot(&f.db), after);
    assert_eq!(f.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    f.cleanup();
}

#[test]
fn source_exhausted_original_receipt_damage_or_unknown_cannot_supply_known_usage() {
    use crate::agent_runtime::multi_agent::source_rounds;
    let f = source_exhausted_original();
    let original = source_exit_snapshot(&f.db);
    for fault in [
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET response_hash='damaged' WHERE round_number=2;",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET state='uncertain' WHERE round_number=3;",
        "DROP TRIGGER agent_source_round_immutable; UPDATE agent_source_model_rounds SET request_hash='damaged' WHERE round_number=1;",
        "DELETE FROM agent_events WHERE run_id IN (SELECT child_run_id FROM agent_source_model_rounds) AND event_type='tool_invocation_completed';",
        "UPDATE agent_snapshots SET snapshot_json='{}';",
    ] {
        f.db.execute_batch("SAVEPOINT exhausted_material_fault").unwrap();
        f.db.execute_batch(fault).unwrap();let before=source_exit_snapshot(&f.db);
        assert!(source_rounds::audit_exhausted(&f.db,&f.actor,&f.child).is_err(),"{fault}");
        assert_eq!(source_exit_snapshot(&f.db),before,"read-only original proof: {fault}");
        f.db.execute_batch("ROLLBACK TO exhausted_material_fault; RELEASE exhausted_material_fault").unwrap();
        assert_eq!(source_exit_snapshot(&f.db),original);
    }
    // Revoked executable authority cannot authorize this paid local closure.
    f.db.execute(
        "UPDATE agent_runs SET cancel_requested_at='requested' WHERE id=?1",
        [&f.actor.root_run_id],
    )
    .unwrap();
    let before = source_exit_snapshot(&f.db);
    assert!(f.close().is_err());
    assert_eq!(source_exit_snapshot(&f.db), before);
    assert_eq!(f.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
    f.cleanup();
}

fn assert_source_exhausted_launcher_accounting(
    db: &rusqlite::Connection,
    actor: &str,
    path: &Path,
    scan: &str,
) {
    // All three original Analyst SDKs returned known usage and actual tools.
    // Failing to finish is a business failure, not unknown provider billing.
    let ledger:(i64,i64,i64,i64)=db.query_row("SELECT spent_tokens,spent_requests,reserved_tokens,reserved_requests FROM agent_budget_ledger WHERE root_run_id=?1",[actor],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(
        ledger,
        (140, 7, 0, 0),
        "known exhausted rounds must reconcile without losing paid usage"
    );
    let exhausted:(String,String,String,i64,i64,i64)=db.query_row("SELECT a.state,r.status,r.terminal_state,a.reserved_tokens,
        (SELECT count(*) FROM agent_lane_leases WHERE assignment_id=a.id),
        (SELECT count(*) FROM agent_capability_leases WHERE assignment_id=a.id AND revoked_at='')
        FROM agent_assignments a JOIN agent_runs r ON r.id=a.child_run_id
        WHERE a.coordinator_run_id=?1 AND a.role='source_analyst' AND json_extract(a.task_slice_json,'$.phase')='source_tools'",[actor],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).unwrap();
    assert_eq!(
        exhausted,
        ("failed".into(), "terminal".into(), "failed".into(), 0, 0, 0)
    );
    let original = source_exit_snapshot(db);
    let tx = db.unchecked_transaction().unwrap();
    let c = native_source_fresh_finance::original_for_financial_exit(&tx, actor).unwrap();
    let bases = crate::agent_runtime::multi_agent::source::completion_task_slices(&tx, &c).unwrap();
    assert!(
        source_assessment_completion(&tx, &c, &bases).is_err(),
        "a failed Source is never a completed proof"
    );
    let prepared = crate::agent_runtime::deleted_scan_audit::prepare(&tx, scan).unwrap();
    prepared.verify_scope(&tx, scan).unwrap();
    drop(prepared);
    tx.rollback().unwrap();
    assert_eq!(
        source_exit_snapshot(db),
        original,
        "failure audit is read-only"
    );
    assert_eq!(db.path(), path.to_str());
}
