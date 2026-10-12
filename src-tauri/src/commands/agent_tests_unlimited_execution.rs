#[test]
fn unlimited_execution_actual_native_creator_reaches_paid_root_web_broker_client_chain() {
    let _real = RealSpecialistTransport::enter();
    let mut f = unlimited_execution_fixture((0, 0), 0);
    let outcome = run_native_agent(&f.h.context);
    assert_eq!(
        f.h.site_seen.lock().unwrap().len(),
        1,
        "actual original Broker GET is required, not allocation-only admission: {} (SDK count {})",
        outcome.detail(),
        f.h.model_seen.lock().unwrap().len()
    );
    multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
    let db = db::open(&f.h.db_path).unwrap();
    let root = &f.session.as_ref().unwrap().lease.root_run_id;
    assert_eq!(
        f.h.model_seen.lock().unwrap().len(),
        15,
        "paid Root x5 (including original new-evidence assessment), Mapper, Web reaches original max_turns=8, Client"
    );
    assert_eq!(root_tick_count(&db, root, "publication"), 5);
    for (dimension, paid) in [
        ("model_input_tokens", 150),
        ("model_cached_tokens", 0),
        ("model_output_tokens", 390),
        ("model_requests", 15),
    ] {
        let fees =
            crate::agent_runtime::multi_agent::budget::balance(&db, root, None, dimension).unwrap();
        assert_eq!(fees.consumed, paid, "actual paid usage {dimension}");
        assert_eq!(fees.indeterminate, 0, "known receipts {dimension}");
        assert_eq!(fees.reserved, 0, "closed original chain {dimension}");
    }
    let web_requests = crate::agent_runtime::multi_agent::budget::balance(
        &db,
        root,
        Some(&f.session.as_ref().unwrap().executor.assignment_id),
        "model_requests",
    )
    .unwrap();
    assert_eq!(web_requests.consumed, 8);
    assert_eq!(
        crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "model_requests")
            .unwrap()
            .consumed,
        15,
        "unlimited still records every original paid SDK"
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_assignments WHERE role='web_executor'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        db.query_row(
            "SELECT count(*) FROM agent_assignments WHERE role='client_side' AND state='completed'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
}

#[test]
fn unlimited_execution_mixed_original_ceiling_keeps_finite_grant_and_actual_sdk_costs() {
    let _real = RealSpecialistTransport::enter();
    for limits in [(0, 20), (60000, 0)] {
        let mut f = unlimited_execution_fixture(limits, 0);
        let db = db::open(&f.h.db_path).unwrap();
        let session = f.session.as_ref().unwrap();
        let before = crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
            &db,
            "SELECT rowid,* FROM agent_budget_limits ORDER BY rowid",
            [],
        )
        .unwrap();
        let grant = crate::agent_runtime::multi_agent::budget::model::original_web_grant(
            &db,
            &session.lease,
            &session.executor.assignment_id,
        )
        .unwrap();
        assert_eq!(grant.0 == 0, limits.0 == 0);
        assert_eq!(grant.1 == 0, limits.1 == 0);
        let outcome = run_native_agent(&f.h.context);
        assert_eq!(
            f.h.site_seen.lock().unwrap().len(),
            1,
            "{}",
            outcome.detail()
        );
        multi_agent_finish_execution(&f.h.context, f.session.as_mut().unwrap(), &outcome).unwrap();
        let root = &f.session.as_ref().unwrap().lease.root_run_id;
        // v6 keeps finite request headroom, shortening Web's paid execution.
        // Unlimited requests retain the original physical 15-call flow.
        let expected_calls = if limits.1 > 0 { 10 } else { 15 };
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "model_requests")
                .unwrap()
                .consumed,
            expected_calls
        );
        assert_eq!(f.h.model_seen.lock().unwrap().len(), expected_calls as usize);
        assert!(
            crate::agent_runtime::multi_agent::attempts::audit_rows::Rows::read(
                &db,
                "SELECT rowid,* FROM agent_budget_limits ORDER BY rowid",
                []
            )
            .unwrap()
                == before
        );
        let tokens = crate::agent_runtime::multi_agent::budget::balance(
            &db,
            root,
            None,
            "model_input_tokens",
        )
        .unwrap();
        let requests =
            crate::agent_runtime::multi_agent::budget::balance(&db, root, None, "model_requests")
                .unwrap();
        if limits.0 > 0 {
            assert!(tokens.consumed + tokens.reserved + tokens.indeterminate <= limits.0);
        }
        if limits.1 > 0 {
            assert!(requests.consumed + requests.reserved + requests.indeterminate <= limits.1);
        }
    }
}
#[test]
fn unlimited_execution_original_cost_rule_keeps_old_native_and_finite_pressure_exact() {
    let old = NativeCoordinatorModelCostRule::OriginalConservative;
    for version in 1..=4 {
        assert!(
            NativeCoordinatorModelCostRule::from_original(&json!({"schemaVersion":version}))
                .unwrap()
                == old
        );
        assert_eq!(old.weight(0, None).unwrap(), 20);
        assert!(NativeCoordinatorModelCostRule::from_original(
            &json!({"schemaVersion":version,"modelCostRule":NATIVE_UNLIMITED_MODEL_COST_RULE})
        )
        .is_err());
    }
    assert!(NativeCoordinatorModelCostRule::from_original(&json!({"schemaVersion":5})).is_err());
    let current = NativeCoordinatorModelCostRule::from_original(
        &json!({"schemaVersion":5,"modelCostRule":NATIVE_UNLIMITED_MODEL_COST_RULE}),
    )
    .unwrap();
    assert_eq!(current.weight(0, None).unwrap(), 0);
    for amount in [0, 1, 100, i64::MAX] {
        for ceiling in [0, 1, 60000, i64::MAX] {
            assert_eq!(
                current.weight(amount, Some(ceiling)).unwrap(),
                old.weight(amount, Some(ceiling)).unwrap()
            );
        }
    }
    assert!(current.weight(-1, None).is_err());
    assert!(current.weight(1, Some(-1)).is_err());
}
