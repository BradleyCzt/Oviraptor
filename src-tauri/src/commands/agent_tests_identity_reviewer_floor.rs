// IdentitySession must preserve the same EvidenceReviewer floor as ClientSide.
#[test]
fn identity_session_prepare_preserves_reviewer_floor_15000() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    let (port, _seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    // 18k < 15k floor + 4k Identity grant.
    let f = root_tick_fixture_protocol_limits(
        "identity-floor-deny",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (18_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    configure_specialist_identity(&db, &f.actor);
    let err = scheduler::prepare_readonly_child(
        &db,
        &f.actor,
        AgentRole::IdentitySession,
        "validated_identity_metadata_ready",
        &json!({
            "identityMode": "IdentitySet",
            "identityCount": 1,
            "mapperAssignmentId": "asg-none",
            "objective": "readonly metadata, no target access"
        }),
        4_000,
    )
    .unwrap_err();
    assert_eq!(
        err, "client_side_budget_headroom_missing_reviewer_floor_preserved",
        "Identity must not eat Reviewer 15k floor"
    );
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM agent_assignments WHERE role='identity_session'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn identity_session_prepare_admits_when_surplus_covers_reviewer_floor() {
    use crate::agent_runtime::{contract::AgentRole, multi_agent::scheduler};
    let (port, _seen, _) = spawn_endpoint(std::sync::Arc::new(|request| {
        (
            200,
            "application/json",
            changed_fact_response(&request, true),
        )
    }));
    let f = root_tick_fixture_protocol_limits(
        "identity-floor-admit",
        &format!("http://127.0.0.1:{port}/v1"),
        true,
        (40_000, 20),
    );
    let db = db::open(&f.context.db_path).unwrap();
    configure_specialist_identity(&db, &f.actor);
    let child = scheduler::prepare_readonly_child(
        &db,
        &f.actor,
        AgentRole::IdentitySession,
        "validated_identity_metadata_ready",
        &json!({
            "identityMode": "IdentitySet",
            "identityCount": 1,
            "mapperAssignmentId": "asg-none",
            "objective": "readonly metadata, no target access"
        }),
        4_000,
    )
    .unwrap();
    assert_eq!(child.role, AgentRole::IdentitySession);
    let reserved: i64 = db
        .query_row(
            "SELECT reserved_tokens FROM agent_assignments WHERE id=?1",
            [&child.assignment_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(reserved, 4_000);
}
