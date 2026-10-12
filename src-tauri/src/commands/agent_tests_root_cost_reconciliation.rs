#[test]
fn root_cost_reconcile_settles_writer_fault_invoice_and_restores_admission_without_sdk_retry() {
    let _real = RealSpecialistTransport::enter();
    let path = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let writer = path.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let db = db::open(writer.lock().unwrap().as_ref().unwrap()).unwrap();
        db.execute_batch(
            "CREATE TRIGGER finite_cost_fault BEFORE INSERT ON agent_budget_entries \
             WHEN NEW.idempotency_key LIKE '%:actual' \
             BEGIN SELECT RAISE(ABORT,'finite_cost_abort'); END;",
        )
        .unwrap();
        (200, "application/json", root_finite_cost_wire().to_string())
    }));
    let f = root_tick_fixture_protocol_limits(
        "reconcile-writer-fault",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (200_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    *path.lock().unwrap() = Some(f.context.db_path.clone());
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    let call: String = db
        .query_row(
            "SELECT call_id FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::admission::require_determinate(
            &db,
            &f.actor.root_run_id
        )
        .err(),
        Some("budget_indeterminate_requires_reconciliation".into())
    );
    let before_fault = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert!(
        crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::settle_unsettled_received(
            &db, &f.actor.root_run_id, &call,
        ).is_err(),
        "explicit settlement must also retain the invoice when its writer still fails"
    );
    web_mode_assert_rows(&db, &before_fault.0);
    assert_eq!(single_finally_physical(&db), before_fault.1);
    assert_eq!(seen.lock().unwrap().len(), 1);
    db.execute_batch("DROP TRIGGER finite_cost_fault").unwrap();
    crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::settle_unsettled_received(
        &db,
        &f.actor.root_run_id,
        &call,
    )
    .expect("frozen invoice must settle without SDK");
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::admission::require_determinate(
            &db,
            &f.actor.root_run_id
        ),
        Ok(())
    );
    for (dimension, actual) in [
        ("model_input_tokens", 50_000i64),
        ("model_cached_tokens", 10_000),
        ("model_output_tokens", 10),
        ("model_requests", 1),
    ] {
        let b = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            dimension,
        )
        .unwrap();
        assert_eq!(
            (b.consumed, b.reserved, b.indeterminate),
            (actual, 0, 0),
            "{dimension}"
        );
    }
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_root_tick_terminal_proofs WHERE call_id=?1",
            [&call],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::settle_unsettled_received(
            &db,
            &f.actor.root_run_id,
            &call,
        )
        .err(),
        Some("root_tick_reconcile_already_settled".into())
    );
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "reconcile must never repay the SDK"
    );
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
}

#[test]
fn root_cost_reconcile_over_cap_forfeit_keeps_debt_and_blocks_admission_without_sdk_retry() {
    let _real = RealSpecialistTransport::enter();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(|_| {
        (200, "application/json", root_finite_cost_wire().to_string())
    }));
    let f = root_tick_fixture_protocol_limits(
        "reconcile-unknown-cap",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (40_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        native_coordinator_tick(&f.context, &f.actor).err().unwrap(),
        "budget_indeterminate_requires_reconciliation"
    );
    assert_eq!(seen.lock().unwrap().len(), 1);
    let call: String = db
        .query_row(
            "SELECT call_id FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",
            [&f.actor.root_run_id],
            |r| r.get(0),
        )
        .unwrap();
    let held: i64 = db
        .query_row(
            "SELECT json_extract(receipt_json,'$.reservedTokens') FROM agent_root_model_journal
             WHERE call_id=?1 AND phase='dispatch'",
            [&call],
            |r| r.get(0),
        )
        .unwrap();
    assert!(held > 0 && held < 50_010);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(
            &db,
            &f.actor.root_run_id,
            None,
            "model_input_tokens"
        )
        .unwrap()
        .indeterminate,
        held
    );
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::settle_unsettled_received(
            &db,
            &f.actor.root_run_id,
            &call,
        )
        .err(),
        Some("root_tick_reconcile_already_settled".into()),
        "over-cap unknown already wrote terminal proof during original settle"
    );
    // U182: the original U2 assertion treated an estimate smaller than the
    // actual over-cap invoice as settled. Preserve the same paid fixture and
    // no-retry proof, but require the original debt to remain unresolved.
    let before = (web_mode_test_rows(&db), single_finally_physical(&db));
    for _ in 0..2 {
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::reconcile_forfeited_unknown(
                &db,
                &f.actor.root_run_id,
                &call,
            )
            .err(),
            Some("root_tick_reconcile_actual_cost_unverified".into())
        );
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
    }
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::admission::require_determinate(
            &db,
            &f.actor.root_run_id
        ),
        Err("budget_indeterminate_requires_reconciliation".into())
    );
    let b = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        &f.actor.root_run_id,
        None,
        "model_input_tokens",
    )
    .unwrap();
    assert_eq!((b.indeterminate, b.reserved, b.consumed), (held, 0, 0));
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
}

#[test]
fn root_cost_reconcile_unreported_or_inconsistent_invoice_keeps_original_debt_zero_write() {
    let _real = RealSpecialistTransport::enter();
    for missing in [false, true] {
        let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
            let mut response = root_finite_cost_wire();
            if missing {
                response.as_object_mut().unwrap().remove("usage");
            } else {
                response["usage"]["total_tokens"] = 1.into();
            }
            (200, "application/json", response.to_string())
        }));
        let f = root_tick_fixture_protocol_limits(
            "reconcile-invalid-invoice",
            &format!("http://127.0.0.1:{port}/v1"),
            true,
            (200_000, 20),
        );
        let db = db::open(&f.context.db_path).unwrap();
        assert_eq!(
            native_coordinator_tick(&f.context, &f.actor).err(),
            Some("budget_indeterminate_requires_reconciliation".into())
        );
        let call: String = db.query_row(
            "SELECT call_id FROM agent_root_model_journal WHERE root_run_id=?1 AND phase='received'",
            [&f.actor.root_run_id], |r| r.get(0),
        ).unwrap();
        let before = (web_mode_test_rows(&db), single_finally_physical(&db));
        for _ in 0..2 {
            assert_eq!(
                crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::reconcile_forfeited_unknown(
                    &db, &f.actor.root_run_id, &call,
                ).err(),
                Some("root_tick_reconcile_actual_cost_unverified".into())
            );
            assert_eq!(
                crate::agent_runtime::multi_agent::budget::admission::require_determinate(
                    &db,
                    &f.actor.root_run_id
                )
                .err(),
                Some("budget_indeterminate_requires_reconciliation".into())
            );
            web_mode_assert_rows(&db, &before.0);
            assert_eq!(single_finally_physical(&db), before.1);
        }
        assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert_eq!(seen.lock().unwrap().len(), 1, "no SDK repayment");
    }
}

include!("agent_tests_root_reconcile_projection.rs");
include!("agent_tests_root_cost_confirmation.rs");
include!("agent_tests_root_financial_continuation.rs");
include!("agent_tests_financial_continuation_issuance.rs");
include!("agent_tests_financial_history_proofs.rs");

include!("agent_tests_financial_continuation_concurrency.rs");
include!("agent_tests_financial_continuation_crash.rs");
include!("agent_tests_financial_continuation_approval_guards.rs");
include!("agent_tests_financial_continuation_remaining_limits.rs");
include!("agent_tests_financial_continuation_status.rs");
include!("agent_tests_financial_continuation_discovery.rs");
include!("agent_tests_financial_continuation_received_crash.rs");
include!("agent_tests_financial_continuation_desktop.rs");
include!("agent_tests_financial_continuation_new_cost_fault.rs");
