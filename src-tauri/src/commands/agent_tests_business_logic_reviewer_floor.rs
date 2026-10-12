// BusinessLogic readonly bootstrap must preserve EvidenceReviewer 15_000 floor.
#[test]
fn business_logic_prepare_preserves_reviewer_floor_15000() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    let (port, _seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let f = root_tick_fixture_protocol_limits(
        "business-logic-floor-deny",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (18_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let err = scheduler::prepare_readonly_child(
        &db,
        &f.actor,
        AgentRole::BusinessLogic,
        "validated_business_flow_ready",
        &json!({"objective": "readonly business flow inventory, no target access"}),
        4_000,
    )
    .unwrap_err();
    assert_eq!(
        err, "client_side_budget_headroom_missing_reviewer_floor_preserved",
        "BusinessLogic must not eat Reviewer 15k floor"
    );
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE role='business_logic'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn business_logic_prepare_admits_when_surplus_covers_reviewer_floor() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    let (port, _seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let f = root_tick_fixture_protocol_limits(
        "business-logic-floor-admit",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (40_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    let child = scheduler::prepare_readonly_child(
        &db,
        &f.actor,
        AgentRole::BusinessLogic,
        "validated_business_flow_ready",
        &json!({"objective": "readonly business flow inventory, no target access"}),
        4_000,
    )
    .unwrap();
    assert_eq!(child.role, AgentRole::BusinessLogic);
    let reserved: i64 = db
        .query_row(
            "SELECT reserved_tokens FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 4_000);
}
