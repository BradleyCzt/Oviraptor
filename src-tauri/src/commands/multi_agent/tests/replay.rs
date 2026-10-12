#[test]
fn unimplemented_specialists_cannot_mint_assignments_or_capability_leases() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("specialist-gate", 100, 3);
    let connection = db::open(&db_path).unwrap();
    let before = receipt_database_snapshot(&connection);
    for role in [
        AgentRole::InputParser,
        AgentRole::Upload,
        AgentRole::BusinessLogic,
        AgentRole::Concurrency,
        AgentRole::ClientSide,
    ] {
        assert_eq!(
            scheduler::schedule_child(
                &connection,
                &lease,
                role,
                AgentLane::TargetTouching,
                "unimplemented",
                &serde_json::json!({"task":"test"}),
                1,
                &["replay_http".into()],
                10,
                1,
            )
            .unwrap_err(),
            if matches!(
                role,
                AgentRole::ClientSide | AgentRole::InputParser | AgentRole::BusinessLogic
            ) {
                "analysis_specialist_requires_read_only_lane"
            } else if role == AgentRole::Upload {
                "upload_role_not_implemented"
            } else if role == AgentRole::Concurrency {
                "concurrency_role_not_implemented"
            } else {
                "specialist_role_not_implemented"
            }
        );
    }
    assert_eq!(receipt_database_snapshot(&connection), before);
    let assignments: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1",
            [&root_run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(assignments, 0);
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn input_parser_and_business_logic_schedule_on_read_only_lane() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    for (label, role) in [
        ("readonly-schedule-input", AgentRole::InputParser),
        ("readonly-schedule-biz", AgentRole::BusinessLogic),
    ] {
        let (root, db_path, _root_run_id, lease) = multi_agent_test_root(label, 100_000, 20);
        let connection = db::open(&db_path).unwrap();
        let child = scheduler::schedule_child(
            &connection,
            &lease,
            role,
            AgentLane::ReadOnlyAnalysis,
            "validated_readonly_ready",
            &serde_json::json!({"objective": "readonly schedule path"}),
            1,
            &["evidence.read".into(), "mailbox.write".into()],
            4_000,
            1,
        )
        .unwrap_or_else(|e| panic!("{role:?} schedule failed: {e}"));
        assert_eq!(child.role, role);
        assert_eq!(
            scheduler::schedule_child(
                &connection,
                &lease,
                AgentRole::Upload,
                AgentLane::ReadOnlyAnalysis,
                "validated_readonly_ready",
                &serde_json::json!({"objective": "upload still stub"}),
                1,
                &["evidence.read".into()],
                4_000,
                1,
            )
            .unwrap_err(),
            "upload_role_not_implemented"
        );
        drop(connection);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn resumed_attempt_refuses_legacy_target_child_and_new_executor_replay() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::{assignment, scheduler},
    };

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("role-migration", 100, 3);
    let connection = db::open(&db_path).unwrap();
    scheduler::ensure_fresh_web_executor_attempt(&connection, &lease).unwrap();
    // An interrupted older build can leave the assignment without its child
    // run. The old target-touching role is still evidence that requests may
    // already have left the process, even if its state says prepared.
    let old = assignment::AgentAssignment::new(
        "legacy-target-child",
        &root_run_id,
        AgentRole::DeepInvestigator,
        AgentLane::TargetTouching,
        "https://authorized.example.test",
        "legacy-executor",
    );
    assignment::insert_assignment(&connection, &old).unwrap();
    assert_eq!(
        scheduler::ensure_fresh_web_executor_attempt(&connection, &lease).unwrap_err(),
        "target_execution_recovery_requires_fresh_attempt"
    );
    assert_eq!(
        scheduler::schedule_child(
            &connection,
            &lease,
            AgentRole::WebExecutor,
            AgentLane::TargetTouching,
            "mapper_handoff_ready",
            &serde_json::json!({"task":"replay"}),
            1,
            &["replay_http".into()],
            10,
            1,
        )
        .unwrap_err(),
        "target_execution_recovery_requires_fresh_attempt"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM agent_assignments WHERE coordinator_run_id=?1",
                [&root_run_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();

    let (root, db_path, _, lease) = multi_agent_test_root("executor-replay", 100, 3);
    let connection = db::open(&db_path).unwrap();
    scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::WebExecutor,
        AgentLane::TargetTouching,
        "mapper_handoff_ready",
        &serde_json::json!({"task":"first"}),
        1,
        &["replay_http".into()],
        10,
        1,
    )
    .unwrap();
    assert_eq!(
        scheduler::ensure_fresh_web_executor_attempt(&connection, &lease).unwrap_err(),
        "target_execution_recovery_requires_fresh_attempt"
    );
    assert!(scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::WebExecutor,
        AgentLane::TargetTouching,
        "mapper_handoff_ready",
        &serde_json::json!({"task":"second"}),
        2,
        &["replay_http".into()],
        10,
        1,
    )
    .is_err());
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn multi_agent_budget_overrun_rolls_back_and_failure_releases_reservation() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("budget-overrun", 50, 2);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "overrun-test",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        20,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    let error = settle_child_usage(
        &connection,
        &lease,
        &child,
        &AgentTokenUsage {
            total_tokens: 21,
            model_requests: 1,
            ..AgentTokenUsage::default()
        },
    )
    .unwrap_err();
    assert_eq!(error, "child_budget_settlement_exceeded_or_stale");
    scheduler::finish_child(&connection, &lease, &child, false, &error).unwrap();
    let ledger: (i64, i64, i64, i64) = connection
        .query_row(
            "SELECT reserved_tokens,spent_tokens,reserved_requests,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [&root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(ledger, (0, 0, 0, 0));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn multi_agent_zero_total_budget_is_explicitly_unlimited() {
    use crate::agent_runtime::{
        contract::{AgentLane, AgentRole},
        multi_agent::scheduler,
    };

    let (root, db_path, root_run_id, lease) = multi_agent_test_root("budget-unlimited", 0, 0);
    let connection = db::open(&db_path).unwrap();
    let child = scheduler::schedule_child(
        &connection,
        &lease,
        AgentRole::SpaApiMapper,
        AgentLane::ReadOnlyAnalysis,
        "unlimited-test",
        &serde_json::json!({}),
        1,
        &["evidence.read".into()],
        0,
        0,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &lease, &child).unwrap();
    settle_child_usage(
        &connection,
        &lease,
        &child,
        &AgentTokenUsage {
            total_tokens: 123,
            model_requests: 2,
            ..AgentTokenUsage::default()
        },
    )
    .unwrap();
    let ledger: (i64, i64, i64, i64) = connection
        .query_row(
            "SELECT total_tokens,spent_tokens,total_requests,spent_requests FROM agent_budget_ledger WHERE root_run_id=?1",
            [&root_run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(ledger, (0, 123, 0, 2));
    drop(connection);
    std::fs::remove_dir_all(root).unwrap();
}

