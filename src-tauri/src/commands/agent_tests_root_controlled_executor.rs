// Actual shared Root entry and original provider invoices, not a core fixture.
struct RootControlledInvoiceScope {
    path: PathBuf,
    root: String,
    scan: String,
    attempt: i64,
    target: String,
    epoch: i64,
    fence: String,
    round: i64,
    fired: bool,
    fault: Option<&'static str>,
}
thread_local! {
    static ROOT_CONTROLLED_INVOICE_SCOPE: std::cell::RefCell<Option<RootControlledInvoiceScope>> = const { std::cell::RefCell::new(None) };
}
struct RootControlledInvoiceGuard;
impl RootControlledInvoiceGuard {
    fn pause_once(f: &RootTickFixture, round: i64) -> Self {
        ROOT_CONTROLLED_INVOICE_SCOPE.with(|slot| {
            let mut slot = slot.borrow_mut();
            assert!(slot.is_none(), "original invoice scope cannot be nested");
            *slot = Some(RootControlledInvoiceScope {
                path: f.context.db_path.clone(),
                root: f.actor.root_run_id.clone(),
                scan: f.actor.scan_id.clone(),
                attempt: f.actor.attempt_number,
                target: f.actor.target_key.clone(),
                epoch: f.actor.lease_epoch,
                fence: f.actor.fencing_token.clone(),
                round,
                fired: false,
                fault: None,
            });
        });
        Self
    }
    fn fault_once(f: &RootTickFixture, fault: &'static str) -> Self {
        let guard = Self::pause_once(f, 1);
        ROOT_CONTROLLED_INVOICE_SCOPE
            .with(|slot| slot.borrow_mut().as_mut().unwrap().fault = Some(fault));
        guard
    }
    fn assert_fired(&self) {
        ROOT_CONTROLLED_INVOICE_SCOPE.with(|slot| assert!(slot.borrow().as_ref().unwrap().fired));
    }
}
impl Drop for RootControlledInvoiceGuard {
    fn drop(&mut self) {
        ROOT_CONTROLLED_INVOICE_SCOPE.with(|slot| *slot.borrow_mut() = None);
    }
}
// Called only by the cfg(test) bridge after the original invoice transaction
// has committed. No configuration means no action in every other fixture.
pub(super) fn native_root_controlled_post_original_invoice_checkpoint(
    context: &AgentRunContext,
    actor: &crate::agent_runtime::multi_agent::lease::CoordinatorLease,
    round: i64,
) {
    ROOT_CONTROLLED_INVOICE_SCOPE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(scope) = slot.as_mut() else { return; };
        if scope.fired || scope.path != context.db_path
            || scope.root != actor.root_run_id || scope.scan != actor.scan_id
            || scope.attempt != actor.attempt_number || scope.target != actor.target_key
            || scope.epoch != actor.lease_epoch || scope.fence != actor.fencing_token
            || scope.round != round || context.scan_id != scope.scan
            || context.attempt_number != scope.attempt
            || context.target_url != scope.target
        { return; }
        scope.fired = true;
        let db = db::open(&scope.path).unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND round=?2 AND phase='received'", params![scope.root,scope.round], |r|r.get::<_,i64>(0)).unwrap(), 1,
            "pause must follow the committed original received invoice");
        match scope.fault {
            Some("financial") => db.execute_batch("CREATE TRIGGER root_controlled_financial_fault BEFORE INSERT ON agent_budget_entries WHEN NEW.kind='consume' AND NEW.dimension='model_requests' BEGIN SELECT RAISE(ABORT,'root_controlled_financial_fault'); END;").unwrap(),
            Some("publication") => db.execute_batch("CREATE TRIGGER root_controlled_publication_fault BEFORE INSERT ON agent_root_tick_receipts WHEN NEW.phase='publication' BEGIN SELECT RAISE(ABORT,'root_controlled_publication_fault'); END;").unwrap(),
            Some(_) => panic!("unknown test fault"),
            None => { assert_eq!(db.execute("UPDATE sentinel_scans SET status='paused' WHERE id=?1", [&scope.scan]).unwrap(), 1); }
        }
    });
}

#[test]
fn root_controlled_executor_actual_paid_checkpoint_pause_keeps_fee_without_local_publication() {
    let body = root_local_wire(
        json!([root_local_call(
            "must-not-run-after-pause",
            "snapshot.read",
            json!({})
        )]),
        "PRIVATE_POST_INVOICE_PAUSE_DRAFT",
        true,
    );
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        (200, "application/json", body.clone())
    }));
    let f = root_tick_fixture(
        "controlled-root-post-invoice-pause",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let hook = RootControlledInvoiceGuard::pause_once(&f, 1);
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    hook.assert_fired();
    assert_eq!(seen.lock().unwrap().len(), 1);
    let balance = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (balance.reserved, balance.consumed, balance.indeterminate),
        (0, 1, 0)
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 0);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'", [&f.actor.root_run_id], |r|r.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_terminal_proofs WHERE root_run_id=?1 AND round=1 AND phase='received'", [&f.actor.root_run_id], |r|r.get::<_,i64>(0)).unwrap(), 1);
    for table in [
        "agent_http_request_claims",
        "tool_invocations",
        "agent_assignments",
        "agent_capability_leases",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    let before = super::tests::application_table_snapshot(&db);
    assert!(!format!("{before:?}").contains("PRIVATE_POST_INVOICE_PAUSE_DRAFT"));
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
}

#[test]
fn root_controlled_executor_actual_local_observation_and_free_original_reentry() {
    let turns = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = turns.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |raw| {
        let turn = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let response = match turn {
            0 => root_local_wire(
                json!([root_local_call(
                    "original-snapshot",
                    "snapshot.read",
                    json!({})
                )]),
                "PRIVATE_ROOT_CONTROLLED_DRAFT",
                true,
            ),
            1 => {
                let wire: JsonValue =
                    serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
                let observation = wire["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|m| m["role"] == "tool" && m["tool_call_id"] == "original-snapshot")
                    .expect("next original SDK must receive the audited local observation");
                let value: JsonValue =
                    serde_json::from_str(observation["content"].as_str().unwrap()).unwrap();
                assert!(
                    value.is_object(),
                    "original snapshot result must be structured"
                );
                proposal_model_response(&root_tick_valid_text(
                    "original controlled snapshot assessed",
                ))
            }
            _ => panic!("original paid reentry must not issue another SDK"),
        };
        (200, "application/json", response)
    }));
    let f = root_tick_fixture(
        "controlled-root-original-reentry",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let result = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(turns.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(seen.lock().unwrap().len(), 2);
    let balance = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (balance.reserved, balance.consumed, balance.indeterminate),
        (0, 2, 0)
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 2);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 2);
    let before = super::tests::application_table_snapshot(&db);
    let replay = native_coordinator_tick(&f.context, &f.actor).unwrap();
    assert_eq!(replay.summary, result.summary);
    assert_eq!(replay.event_sequence, result.event_sequence);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 2);
    for table in [
        "agent_http_request_claims",
        "tool_invocations",
        "agent_assignments",
        "agent_capability_leases",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn root_controlled_executor_actual_schema_refusal_retains_invoice_without_retry() {
    let marker = "PRIVATE_CONTROLLED_ARGUMENT";
    let body = root_local_wire(
        json!([root_local_call(
            "invalid-local-schema",
            "snapshot.read",
            json!({"path":marker})
        )]),
        "PRIVATE_CONTROLLED_RESPONSE",
        true,
    );
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        (200, "application/json", body.clone())
    }));
    let f = root_tick_fixture(
        "controlled-root-schema-rejection",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let error = native_coordinator_tick(&f.context, &f.actor).err().unwrap();
    assert!(error.starts_with("root_local_"), "{error}");
    assert_eq!(seen.lock().unwrap().len(), 1);
    let balance = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (balance.reserved, balance.consumed, balance.indeterminate),
        (0, 1, 0)
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'", [&f.actor.root_run_id], |r| r.get::<_,i64>(0)).unwrap(), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 0);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    let before = super::tests::application_table_snapshot(&db);
    let original = format!("{before:?}");
    assert!(!original.contains(marker));
    assert!(!original.contains("PRIVATE_CONTROLLED_RESPONSE"));
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    for table in [
        "agent_http_request_claims",
        "tool_invocations",
        "agent_assignments",
        "agent_capability_leases",
    ] {
        assert_eq!(
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn root_controlled_executor_actual_financial_rollback_preserves_invoice_and_both_causes() {
    let body = proposal_model_response(&root_tick_valid_text("original financial failure probe"));
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        (200, "application/json", body.clone())
    }));
    let f = root_tick_fixture(
        "controlled-root-financial-rollback",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let guard = RootControlledInvoiceGuard::fault_once(&f, "financial");
    let error = native_coordinator_tick(&f.context, &f.actor).err().unwrap();
    guard.assert_fired();
    assert!(error.starts_with("root_local_paid_abort:"), "{error}");
    let causes = error.strip_prefix("root_local_paid_abort:").unwrap();
    // The abort reports the original failed attempt; it cannot retry a writer
    // or reinterpret an empty paid state as a successful financial settlement.
    assert!(causes.len() > 1 && causes.len() % 2 == 1, "{error}");
    let midpoint = causes.len() / 2;
    assert_eq!(&causes[midpoint..midpoint + 1], ":", "{error}");
    assert_eq!(
        &causes[..midpoint],
        &causes[midpoint + 1..],
        "original transaction failure and abort failure must both survive unchanged"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let balance = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (balance.reserved, balance.consumed, balance.indeterminate),
        (1, 0, 0)
    );
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",[&f.actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 0);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_tick_terminal_proofs WHERE root_run_id=?1",
            [&f.actor.root_run_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    // Remove the fault: re-entry must still refuse the original unclosed call,
    // rather than secretly settle or dispatch it as a fresh provider request.
    db.execute_batch("DROP TRIGGER root_controlled_financial_fault")
        .unwrap();
    let before = super::tests::application_table_snapshot(&db);
    assert_eq!(
        native_coordinator_tick(&f.context, &f.actor).err().unwrap(),
        "root_tick_dispatch_unknown_no_retry"
    );
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_controlled_executor_actual_publication_failure_keeps_committed_fee_once() {
    let body = proposal_model_response(&root_tick_valid_text("original publication failure probe"));
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        (200, "application/json", body.clone())
    }));
    let f = root_tick_fixture(
        "controlled-root-publication-failure",
        &format!("http://127.0.0.1:{port}/v1"),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let guard = RootControlledInvoiceGuard::fault_once(&f, "publication");
    let error = native_coordinator_tick(&f.context, &f.actor).err().unwrap();
    guard.assert_fired();
    assert!(
        !error.starts_with("root_local_paid_abort:"),
        "committed fees must not be attempted twice: {error}"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let balance = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_requests",
    )
    .unwrap();
    assert_eq!(
        (balance.reserved, balance.consumed, balance.indeterminate),
        (0, 1, 0)
    );
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "decision"), 1);
    assert_eq!(root_tick_count(&db, &f.actor.root_run_id, "publication"), 0);
    assert_eq!(db.query_row("SELECT count(*) FROM agent_root_tick_terminal_proofs WHERE root_run_id=?1 AND phase='received'",[&f.actor.root_run_id],|r|r.get::<_,i64>(0)).unwrap(),1);
    let before = super::tests::application_table_snapshot(&db);
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(super::tests::application_table_snapshot(&db), before);
    assert_eq!(seen.lock().unwrap().len(), 1);
}
