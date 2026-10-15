pub(super) fn replace_target_cost_coordinator(
    db: &rusqlite::Connection,
    original: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    different_root: bool,
) -> crate::agent_runtime::multi_agent::lease::CoordinatorLease {
    use crate::agent_runtime::{multi_agent::lease, store};
    let replacement = if different_root {
        let mut row = store::load_run(db, &original.root_run_id).unwrap().unwrap();
        row.id = format!("replacement-root-{}", uuid::Uuid::new_v4());
        row.root_run_id = row.id.clone();
        store::create_run(db, &row).unwrap();
        row.id
    } else {
        original.root_run_id.clone()
    };
    db.execute(
        "UPDATE agent_coordinator_leases SET lease_expires_at='2000-01-01' WHERE root_run_id=?1",
        [&original.root_run_id],
    )
    .unwrap();
    let new = lease::acquire_coordinator_lease(
        db,
        &original.scan_id,
        original.attempt_number,
        &original.target_key,
        &replacement,
        600,
    )
    .unwrap();
    assert_eq!(new.lease_epoch, original.lease_epoch + 1);
    assert_ne!(new.fencing_token, original.fencing_token);
    new
}

// Actual transport needs an authenticated original Web birth. Keep the old
// journal-only fixture for direct receipt and damaged-history tests; create this
// transport fixture through the existing creator/start/HMAC path instead.
fn real_http_transport_fixture(
    target: &str,
    baseline: i64,
) -> (PathBuf, AgentRunContext, AgentToolRuntime, AgentHttpRequest) {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };
    // The ordinary draft creator removes a trailing slash from a single URL.
    // Bind its original Root and context to that same target; the wire request
    // below retains the caller's full URL, including its path and query.
    let canonical_target = target.trim().trim_matches(|character| {
        matches!(character, '"' | '\'' | '<' | '>' | '，' | '。')
    }).trim_end_matches('/');
    let (root, path, run_id, lease) = multi_agent_task_root_for_target_protocol(
        "http-real-transport", 60_000, 20, canonical_target, false,
    );
    let mut context = test_context(&path, canonical_target, vec![AgentIdentity::anonymous()]);
    context.scan_id = lease.scan_id.clone();
    context.external_surface = true;
    context.run = Some(AgentRunLedger { db_path: path.clone(), run_id });
    let db = db::open(&path).unwrap();
    let child = scheduler::schedule_child(
        &db, &lease, AgentRole::WebExecutor, AgentLane::TargetTouching,
        "http-journal", &serde_json::json!({}), 1, &["replay_http".into()], 100, 1,
    ).unwrap();
    scheduler::mark_child_running(&db, &lease, &child).unwrap();
    context.run.as_mut().unwrap().run_id = child.run_id;
    accounting_native(&context, baseline);
    let mut runtime = AgentToolRuntime {
        target_requests: baseline as usize,
        ..Default::default()
    };
    let invocation = runtime.begin_invocation(1);
    context.run.as_ref().unwrap()
        .begin_tool("replay_http", &serde_json::json!({}), &invocation).unwrap();
    let request = AgentHttpRequest {
        identity: AgentIdentity::anonymous(), method: "GET".into(), url: target.into(),
        extra_headers: vec![], body: None, content_type: None, contract_key: String::new(),
        family: "authorization".into(), source: ScopeSource::IdentityComparison,
        tool: "replay_http".into(), timeout_seconds: 5,
    };
    (root, context, runtime, request)
}

#[test]
fn assignment_attempt_late_headers_after_takeover_charge_only_original_worker() {
    use crate::agent_runtime::multi_agent::{attempts, budget};
    for different_root in [false, true] {
        let (root, context, runtime, request) = http_journal_fixture("http://127.0.0.1:9/", 0);
        let db = db::open(&context.db_path).unwrap();
        let tx = db.unchecked_transaction().unwrap();
        let (old, assignment) =
            budget::target::child_owner(&tx, &context.run.as_ref().unwrap().run_id)
                .unwrap()
                .unwrap();
        tx.rollback().unwrap();
        let worker = attempts::current(&db, &old, &assignment).unwrap();
        let claim = claim_agent_http_request(&context, &runtime, &request, &request.url)
            .unwrap()
            .unwrap();
        let new = replace_target_cost_coordinator(&db, &old, different_root);
        let before = super::tests::application_table_snapshot(&db);
        receive_agent_http_headers(&context, &claim, 200)
            .expect("known late cost must remain attributable after Coordinator takeover");
        let after = super::tests::application_table_snapshot(&db);
        for (table, rows) in &before {
            if !matches!(
                table.as_str(),
                "agent_http_request_claims" | "agent_budget_entries"
            ) {
                assert_eq!(
                    after
                        .iter()
                        .find(|(name, _)| name == table)
                        .map(|(_, values)| values),
                    Some(rows),
                    "{table}: cost receipt changed execution or new Coordinator"
                );
            }
        }
        let b =
            budget::balance(&db, &old.root_run_id, Some(&assignment), "target_requests").unwrap();
        assert_eq!((b.reserved, b.consumed, b.indeterminate), (0, 1, 0));
        assert!(db.query_row("SELECT count(*)=2 AND count(DISTINCT lease_attempt_id)=1 AND min(lease_attempt_id)=?1
            FROM agent_budget_entries WHERE assignment_id=?2 AND dimension='target_requests' AND kind IN ('forfeit','reconcile')",
            params![worker.id, assignment], |r| r.get::<_,bool>(0)).unwrap());
        if different_root {
            assert_eq!(
                budget::balance(&db, &new.root_run_id, None, "target_requests").unwrap(),
                budget::Balance::default()
            );
        }
        assert!(agent_authorize_tool_on(&db, &context, "replay_http").is_err());
        let settled = super::tests::application_table_snapshot(&db);
        assert!(
            native_model_budget_admission(&context, 1, "late-cost-is-not-a-grant".into()).is_err()
        );
        assert!(receive_agent_http_headers(&context, &claim, 201).is_err());
        assert_eq!(super::tests::application_table_snapshot(&db), settled);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_late_target_receipt_rechecks_original_child_after_last_write() {
    for damaged in [
        "assignment_id",
        "root_run_id",
        "target_url",
        "backend",
        "role",
        "lane",
    ] {
        let (root, context, runtime, request) = http_journal_fixture("http://127.0.0.1:9/", 0);
        let claim = claim_agent_http_request(&context, &runtime, &request, &request.url)
            .unwrap()
            .unwrap();
        let db = db::open(&context.db_path).unwrap();
        db.execute_batch(&format!(
            "CREATE TRIGGER damage_cost_owner AFTER INSERT ON agent_budget_entries
            WHEN NEW.dimension='target_requests' AND NEW.kind='reconcile' BEGIN
            UPDATE agent_runs SET {damaged}='foreign-owner' WHERE id='{}'; END;",
            claim.run_id
        ))
        .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            receive_agent_http_headers(&context, &claim, 200).is_err(),
            "{damaged}: late receipt accepted damaged original owner"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{damaged}"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_target_receipt_rejects_foreign_original_claim_and_legacy_worker() {
    for damaged in [
        "assignment_id",
        "root_run_id",
        "target_url",
        "worker_missing",
        "dispatch_missing",
        "single_policy",
    ] {
        let (root, context, runtime, request) = http_journal_fixture("http://127.0.0.1:9/", 0);
        let claim = claim_agent_http_request(&context, &runtime, &request, &request.url)
            .unwrap()
            .unwrap();
        let db = db::open(&context.db_path).unwrap();
        match damaged {
            "worker_missing" => db.execute_batch("DROP TRIGGER assignment_attempt_no_delete; DELETE FROM agent_assignment_attempts").unwrap(),
            "dispatch_missing" => db.execute_batch("DROP TRIGGER budget_entry_no_delete; DELETE FROM agent_budget_entries WHERE kind='forfeit'").unwrap(),
            "single_policy" => {db.execute("UPDATE agent_runs SET orchestration_policy='single' WHERE id=?1", [&claim.run_id]).unwrap();}
            column => {db.execute(&format!("UPDATE agent_runs SET {column}='foreign-owner' WHERE id=?1"), [&claim.run_id]).unwrap();}
        }
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            receive_agent_http_headers(&context, &claim, 200).is_err(),
            "{damaged}"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{damaged}"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_target_cost_and_http_receipt_remain_atomic_after_last_write() {
    for damage in ["claim_delete", "invocation_delete", "invocation_tool"] {
        let (root, context, runtime, request) = http_journal_fixture("http://127.0.0.1:9/", 0);
        let claim = claim_agent_http_request(&context, &runtime, &request, &request.url)
            .unwrap()
            .unwrap();
        let db = db::open(&context.db_path).unwrap();
        let effect = match damage {
            "claim_delete" => "DELETE FROM agent_http_request_claims",
            "invocation_delete" => "DELETE FROM tool_invocations",
            _ => "UPDATE tool_invocations SET tool_name='foreign-tool'",
        };
        db.execute_batch(&format!(
            "CREATE TRIGGER damage_http_fact AFTER INSERT ON agent_budget_entries
            WHEN NEW.dimension='target_requests' AND NEW.kind='reconcile' BEGIN {effect}; END;"
        ))
        .unwrap();
        let before = super::tests::application_table_snapshot(&db);
        assert!(
            receive_agent_http_headers(&context, &claim, 200).is_err(),
            "{damage}: known cost survived missing original HTTP proof"
        );
        assert_eq!(
            super::tests::application_table_snapshot(&db),
            before,
            "{damage}"
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn assignment_attempt_budget_transition_never_borrows_another_workers_reservation() {
    use crate::agent_runtime::{
        multi_agent::{attempts, budget},
        store,
    };
    let (root, context, _, _) = http_journal_fixture("http://127.0.0.1:9/", 0);
    let db = db::open(&context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let (lease, assignment) =
        budget::target::child_owner(&tx, &context.run.as_ref().unwrap().run_id)
            .unwrap()
            .unwrap();
    tx.rollback().unwrap();
    let original = attempts::current(&db, &lease, &assignment).unwrap();
    // Restored multi-attempt fixture only: no production reassign API or
    // capabilities are fabricated. Old ledger reservations remain untouched.
    db.execute("UPDATE agent_assignment_attempts SET state='expired',finished_at=datetime('now','localtime') WHERE id=?1", [&original.id]).unwrap();
    let mut replacement = store::load_run(&db, &original.child_run_id)
        .unwrap()
        .unwrap();
    replacement.id = format!("restored-child-{}", uuid::Uuid::new_v4());
    store::create_run(&db, &replacement).unwrap();
    db.execute("INSERT INTO agent_assignment_attempts
        (id,root_run_id,assignment_id,child_run_id,coordinator_epoch,coordinator_fencing_token,lease_epoch,fencing_token,worker_id,state,expires_at)
        VALUES(?1,?2,?3,?4,?5,?6,2,?7,?8,'running',?9)",
        params![uuid::Uuid::new_v4().to_string(),lease.root_run_id,assignment,replacement.id,lease.lease_epoch,
            lease.fencing_token,uuid::Uuid::new_v4().to_string(),uuid::Uuid::new_v4().to_string(),lease.lease_expires_at]).unwrap();
    db.execute(
        "UPDATE agent_assignments SET child_run_id=?1 WHERE id=?2",
        params![replacement.id, assignment],
    )
    .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    let tx = db.unchecked_transaction().unwrap();
    assert!(
        budget::append(
            &tx,
            &lease,
            &assignment,
            "model_input_tokens",
            budget::Kind::Consume,
            1,
            "restored-worker-unreserved-consume",
            "restored-worker"
        )
        .is_err(),
        "new attempt borrowed original worker's reserved tokens"
    );
    tx.rollback().unwrap();
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn assignment_attempt_real_http_takeover_records_original_bill_without_publishing_evidence() {
    use crate::agent_runtime::multi_agent::budget;
    let owner = std::sync::Arc::new(std::sync::Mutex::new(
        None::<(
            PathBuf,
            crate::agent_runtime::multi_agent::lease::CoordinatorLease,
        )>,
    ));
    let handler_owner = owner.clone();
    let (port, seen, stop) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let (path, old) = handler_owner.lock().unwrap().clone().unwrap();
        let db = db::open(&path).unwrap();
        replace_target_cost_coordinator(&db, &old, false);
        (
            200,
            "text/plain",
            "one real response after Coordinator takeover".into(),
        )
    }));
    let (root, context, mut runtime, request) =
        real_http_transport_fixture(&format!("http://127.0.0.1:{port}/"), 0);
    let db = db::open(&context.db_path).unwrap();
    let tx = db.unchecked_transaction().unwrap();
    let (old, assignment) = budget::target::child_owner(&tx, &context.run.as_ref().unwrap().run_id)
        .unwrap()
        .unwrap();
    tx.rollback().unwrap();
    *owner.lock().unwrap() = Some((context.db_path.clone(), old.clone()));
    let before = super::tests::application_table_snapshot(&db);
    let result = agent_http_exchange(&context, &mut runtime, &request);
    assert!(
        result.is_err(),
        "stale worker published a live HTTP result"
    );
    assert_eq!(seen.lock().unwrap().len(), 1, "transport result: {result:?}");
    assert_eq!(
        db.query_row(
            "SELECT response_status FROM agent_http_request_claims",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        200
    );
    let balance =
        budget::balance(&db, &old.root_run_id, Some(&assignment), "target_requests").unwrap();
    assert_eq!(
        (balance.reserved, balance.consumed, balance.indeterminate),
        (0, 1, 0)
    );
    let after = super::tests::application_table_snapshot(&db);
    for (table, values) in &before {
        if !matches!(
            table.as_str(),
            "agent_http_budget_origins"
                | "agent_http_request_claims"
                | "agent_budget_entries"
                | "agent_coordinator_leases"
        ) {
            assert_eq!(
                after
                    .iter()
                    .find(|(name, _)| name == table)
                    .map(|(_, rows)| rows),
                Some(values),
                "{table}: stale HTTP result changed business state"
            );
        }
    }
    assert!(agent_http_exchange(&context, &mut runtime, &request).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    drop(db);
    fs::remove_dir_all(root).unwrap();
}
