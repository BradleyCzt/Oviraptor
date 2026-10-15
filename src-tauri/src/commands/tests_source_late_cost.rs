#[test]
fn source_round_late_cost_is_original_only_and_never_plans_tools() {
    use crate::agent_runtime::multi_agent::{budget, source_rounds as rounds};
    for uncertain in [false, true] {
        for different_root in [false, true] {
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
            crate::scanning::agent_tests::replace_target_cost_coordinator(
                &db,
                &lease,
                different_root,
            );
            let before = application_table_snapshot(&db);
            if uncertain {
                rounds::record_uncertain(&db, &call, "provider_outcome_unknown").unwrap();
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
                if uncertain { (0, 1) } else { (1, 0) },
                "original Source cost was lost"
            );
            if uncertain {
                let input = budget::balance(
                    &db,
                    &lease.root_run_id,
                    Some(&child.assignment_id),
                    "model_input_tokens",
                )
                .unwrap();
                assert_eq!((input.reserved, input.indeterminate), (16_000, 8_000));
            }
            let after = application_table_snapshot(&db);
            for (table, rows) in &before {
                if !matches!(
                    table.as_str(),
                    "agent_budget_entries" | "agent_model_cost_facts"
                ) {
                    assert_eq!(
                        after
                            .iter()
                            .find(|(name, _)| name == table)
                            .map(|(_, values)| values),
                        Some(rows),
                        "{table}"
                    );
                }
            }
            assert!(
                rounds::start_authorized(&db, &lease, &child, 1, &request, 8_000, check).is_err()
            );
            drop(db);
            fs::remove_dir_all(root).unwrap();
        }
    }
}

#[test]
fn source_round_late_cost_last_write_and_original_request_damage_rollback() {
    use crate::agent_runtime::multi_agent::source_rounds as rounds;
    for mutation in [
        "DELETE FROM agent_source_model_rounds WHERE child_run_id=NEW.child_run_id;",
        "UPDATE agent_assignment_attempts SET failure_class='damaged' WHERE child_run_id=NEW.child_run_id;",
        "UPDATE agent_runs SET role='evidence_reviewer',lane='review' WHERE id=NEW.child_run_id; UPDATE agent_assignments SET role='evidence_reviewer',lane='review' WHERE id=NEW.assignment_id;",
    ] {
        let (root,db,context,lease,child,request)=source_round_fixture();
        let check=|db:&rusqlite::Connection|agent_native_source_tool_authority(db,&context,"assignment.finish").map(|_|()).map_err(str::to_string);
        let rounds::Start::Dispatch(call)=rounds::start_authorized(&db,&lease,&child,1,&request,8_000,check).unwrap() else {panic!()};
        crate::scanning::agent_tests::replace_target_cost_coordinator(&db,&lease,true);
        db.execute_batch(&format!("CREATE TRIGGER damage_source_cost AFTER INSERT ON agent_model_cost_facts BEGIN {mutation} END;")).unwrap();
        let before=application_table_snapshot(&db);
        assert!(rounds::record_received(&db,&call,&source_round_result("late-tool","repo.inventory",json!({}))).is_err());
        assert_eq!(application_table_snapshot(&db),before,"{mutation}");
        drop(db);fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn source_round_late_cost_estimates_and_overruns_keep_original_round_cap() {
    use crate::agent_runtime::multi_agent::{budget, source_rounds as rounds};
    for reported in [false, true] {
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
        crate::scanning::agent_tests::replace_target_cost_coordinator(&db, &lease, false);
        let mut response = source_round_result("late-tool", "repo.inventory", json!({}));
        response.usage_reported = reported;
        if reported {
            response.usage.output_tokens = 7_989;
            response.usage.total_tokens = 8_001;
        }
        assert!(rounds::record_received(&db, &call, &response).is_err());
        let input = budget::balance(
            &db,
            &lease.root_run_id,
            Some(&child.assignment_id),
            "model_input_tokens",
        )
        .unwrap();
        assert_eq!(
            (input.reserved, input.consumed, input.indeterminate),
            (16_000, 0, 8_000)
        );
        assert!(budget::admission::require_determinate(&db, &lease.root_run_id).is_err());
        let saved = application_table_snapshot(&db);
        assert!(rounds::record_received(&db, &call, &response).is_err());
        assert_eq!(application_table_snapshot(&db), saved);
        drop(db);
        fs::remove_dir_all(root).unwrap();
    }
}
