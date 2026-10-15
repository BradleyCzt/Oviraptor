#[test]
fn source_round_worker_deadline_expiry_records_only_original_fee_without_tools() {
    use crate::agent_runtime::multi_agent::{budget, lease as authority, source_rounds as rounds};
    for uncertain in [false, true] {
        let (root, db, context, lease, child, request) = source_round_fixture();
        let check = |db: &rusqlite::Connection| {
            agent_native_source_tool_authority(db, &context, "assignment.finish")
                .map(|_| ())
                .map_err(str::to_string)
        };
        let rounds::Start::Dispatch(call) =
            rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
        else {
            panic!()
        };
        crate::scanning::agent_tests::expire_worker_deadline(&db, &child.run_id);
        authority::validate_coordinator_lease(&db, &lease).unwrap();
        let before = application_table_snapshot(&db);
        if uncertain {
            rounds::record_uncertain(&db, &call, "unknown original round").unwrap();
        } else {
            assert!(rounds::record_received(
                &db,
                &call,
                &source_round_result("late-tool", "repo.inventory", json!({}))
            )
            .is_err());
        }
        let cost = budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_requests",
        )
        .unwrap();
        assert_eq!(
            (cost.consumed, cost.indeterminate),
            if uncertain { (0, 1) } else { (1, 0) }
        );
        assert_eq!(
            budget::admission::require_determinate(&db, &lease.root_run_id).is_ok(),
            !uncertain
        );
        let after = application_table_snapshot(&db);
        for (table, rows) in before {
            if !["agent_budget_entries", "agent_model_cost_facts"].contains(&table.as_str()) {
                assert_eq!(
                    after.iter().find(|(name, _)| name == &table).unwrap().1,
                    rows,
                    "{uncertain}: {table}"
                );
            }
        }
        assert!(rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).is_err());
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_round_worker_deadline_or_missing_worker_blocks_pending_budget_without_writes() {
    use crate::agent_runtime::multi_agent::{budget, source_rounds as rounds};
    for missing in [false, true] {
        let (root, db, context, lease, child, request) = source_round_fixture();
        let check = |db: &rusqlite::Connection| {
            agent_native_source_tool_authority(db, &context, "assignment.finish")
                .map(|_| ())
                .map_err(str::to_string)
        };
        let rounds::Start::Dispatch(_) =
            rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).unwrap()
        else {
            panic!()
        };
        budget::admission::require_determinate(&db, &lease.root_run_id).unwrap();
        if missing {
            let intact = application_table_snapshot(&db);
            assert!(db
                .execute(
                    "DELETE FROM agent_assignment_attempts WHERE child_run_id=?1",
                    [&child.run_id]
                )
                .unwrap_err()
                .to_string()
                .contains("assignment_attempt_audit_immutable"));
            assert_eq!(application_table_snapshot(&db), intact);
            // Corrupt restored fixture, limited to this temporary database.
            db.execute_batch("DROP TRIGGER assignment_attempt_no_delete")
                .unwrap();
            db.execute(
                "DELETE FROM agent_assignment_attempts WHERE child_run_id=?1",
                [&child.run_id],
            )
            .unwrap();
        } else {
            crate::scanning::agent_tests::expire_worker_deadline(&db, &child.run_id);
        }
        let before = application_table_snapshot(&db);
        assert!(
            budget::admission::require_determinate(&db, &lease.root_run_id).is_err(),
            "missing={missing}"
        );
        assert_eq!(application_table_snapshot(&db), before);
        assert_eq!(
            budget::balance(
                &db,
                &lease.root_run_id,
                Some(&child.assignment_id),
                "model_requests"
            )
            .unwrap()
            .reserved,
            3
        );
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
