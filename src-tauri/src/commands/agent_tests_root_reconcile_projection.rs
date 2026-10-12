// Original SDK projection-fault cases for exact local reconciliation.
fn root_cost_reconcile_projection_fault_boundary(exact: bool) {
    let _real = RealSpecialistTransport::enter();
    let path = std::sync::Arc::new(std::sync::Mutex::new(None::<PathBuf>));
    let writer = path.clone();
    let (port, seen, _) = spawn_endpoint(std::sync::Arc::new(move |_| {
        let db = db::open(writer.lock().unwrap().as_ref().unwrap()).unwrap();
        let (root, held): (String, i64) = db
            .query_row(
                "SELECT root_run_id,json_extract(receipt_json,'$.reservedTokens')
             FROM agent_root_model_journal WHERE phase='dispatch'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        // Fault injection at the actual SDK boundary: a broken coarse
        // projection makes the original cost validator retain an unknown bill.
        // No provider usage or immutable invoice/proof is rewritten later.
        db.execute(
            "INSERT INTO agent_budget_ledger(root_run_id,total_tokens,total_requests,reserved_tokens)
             VALUES(?1,200000,20,1)", [&root],
        ).unwrap();
        let mut response = root_finite_cost_wire();
        response["usage"] = json!({
            "prompt_tokens":held,"completion_tokens":if exact {held} else {0},
            "total_tokens":if exact {2 * held} else {held},
            "prompt_tokens_details":{"cached_tokens":if exact {held} else {0}}
        });
        (200, "application/json", response.to_string())
    }));
    let f = root_tick_fixture_protocol_limits(
        "reconcile-exact-dimensions",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (200_000, 20),
    );
    *path.lock().unwrap() = Some(f.context.db_path.clone());
    let db = db::open(&f.context.db_path).unwrap();
    assert_eq!(
        native_coordinator_tick(&f.context, &f.actor).err(),
        Some("budget_indeterminate_requires_reconciliation".into())
    );
    let call: String = db
        .query_row(
            "SELECT call_id FROM agent_root_model_journal WHERE phase='received'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // Restore only the injected coarse projection, not the original bill.
    db.execute(
        "UPDATE agent_budget_ledger SET reserved_tokens=0 WHERE root_run_id=?1",
        [&f.actor.root_run_id],
    )
    .unwrap();
    assert!(
        crate::agent_runtime::multi_agent::budget::root::remaining_child_capacity(
            &db,
            &f.actor.root_run_id
        )
        .unwrap()
        .0
        .unwrap()
            > 0
    );
    let mut before = (web_mode_test_rows(&db), single_finally_physical(&db));
    for retry in 0..2 {
        // Input forfeit equals the actual input bill and is visited first;
        // cached/output forfeits do not. The first append must roll back.
        let result = crate::agent_runtime::multi_agent::budget::root::model::tick::Tick::reconcile_forfeited_unknown(
                &db, &f.actor.root_run_id, &call,
            );
        if exact && retry == 0 {
            assert_eq!(result, Ok(()));
            let billed: i64 = db.query_row(
                "SELECT json_extract(receipt_json,'$.usage.inputTokens') FROM agent_root_model_journal
                 WHERE call_id=?1 AND phase='received'", [&call], |r| r.get(0),
            ).unwrap();
            for dimension in &crate::agent_runtime::multi_agent::budget::DIMENSIONS[..3] {
                let balance = crate::agent_runtime::multi_agent::budget::balance(
                    &db,
                    &f.actor.root_run_id,
                    None,
                    dimension,
                )
                .unwrap();
                assert_eq!(
                    (balance.consumed, balance.reserved, balance.indeterminate),
                    (billed, 0, 0)
                );
            }
            let after = (web_mode_test_rows(&db), single_finally_physical(&db));
            // The confirmation writes only append-only budget transitions;
            // original physical invoices/proofs, plans and permissions stay fixed.
            for (old, new) in before.1.iter().zip(&after.1) {
                if old.0 != "agent_budget_entries" {
                    assert_eq!(old, new);
                }
            }
            assert_ne!(before.0, after.0);
            before = after;
        } else {
            assert_eq!(
                result.err(),
                Some(
                    if exact {
                        "root_tick_reconcile_already_done"
                    } else {
                        "root_tick_reconcile_actual_cost_mismatch"
                    }
                    .into()
                )
            );
        }
        web_mode_assert_rows(&db, &before.0);
        assert_eq!(single_finally_physical(&db), before.1);
        assert_eq!(
            crate::agent_runtime::multi_agent::budget::admission::require_determinate(
                &db,
                &f.actor.root_run_id
            )
            .err(),
            if exact {
                None
            } else {
                Some("budget_indeterminate_requires_reconciliation".into())
            }
        );
    }
    assert!(native_coordinator_tick(&f.context, &f.actor).is_err());
    web_mode_assert_rows(&db, &before.0);
    assert_eq!(single_finally_physical(&db), before.1);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn root_cost_reconcile_exact_dimensions_roll_back_partial_transition_when_capacity_fits() {
    root_cost_reconcile_projection_fault_boundary(false);
}

#[test]
fn root_cost_reconcile_exact_invoice_consumes_once_without_changing_original_proofs() {
    root_cost_reconcile_projection_fault_boundary(true);
}
