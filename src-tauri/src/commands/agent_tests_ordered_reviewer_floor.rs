// Ordered dispatch preserves EvidenceReviewer 15_000 when a WebExecutor holds budget
// (same room as human-proposal scheduling; legacy SQL used +8000).
#[test]
fn ordered_execution_with_active_executor_preserves_reviewer_floor_15000() {
    use crate::agent_runtime::multi_agent::scheduler;
    // 27k with 8k held executor → ~19k free: first ordered action needs 8k + 15k floor.
    let f = root_tick_fixture_protocol_limits(
        "ordered-executor-floor",
        "http://127.0.0.1:9/v1",
        true,
        (27_000, 20),
    );
    let connection = db::open(&f.context.db_path).unwrap();
    let executor = scheduler::schedule_child(
        &connection,
        &f.actor,
        crate::agent_runtime::contract::AgentRole::WebExecutor,
        crate::agent_runtime::contract::AgentLane::TargetTouching,
        "ordered-reviewer-floor",
        &f.context.execution_plan.as_json(),
        1,
        &[],
        8_000,
        1,
    )
    .unwrap();
    scheduler::mark_child_running(&connection, &f.actor, &executor).unwrap();
    let id = confirm_queue_directive(
        &connection,
        &f.actor,
        "@investigator 先评估原证据，再由 @mapper 评估建议",
    );
    let mut inbox = take_human_directives(&f.context).unwrap();
    let applied = apply_human_proposal_actions(&f.context, &mut inbox).unwrap();
    assert!(
        applied.is_empty(),
        "must defer ordered plan rather than eat Reviewer 15k"
    );
    let (status, reason): (String, String) = connection
        .query_row(
            "SELECT status,rejection_code FROM agent_user_directives WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "deferred");
    assert_eq!(reason, "ordered_budget_unavailable");
    let tokens: i64 = connection
        .query_row(
            "SELECT total_tokens-spent_tokens-reserved_tokens FROM agent_budget_ledger",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        tokens >= 15_000,
        "Reviewer floor must remain free after defer: {tokens}"
    );
    let ordered_children: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE trigger_code LIKE 'human_ordered:%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ordered_children, 0);
    scheduler::finish_child(&connection, &f.actor, &executor, true, "fixture done").unwrap();
    let _ = fs::remove_dir_all(&f.f.root);
}
