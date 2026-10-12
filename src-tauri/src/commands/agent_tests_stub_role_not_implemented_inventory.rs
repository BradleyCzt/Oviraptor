
// U147: Upload/Concurrency are the only unimplemented stub roles, each with its
// own distinct code; every other role has none.
#[test]
fn native_stub_role_not_implemented_inventory_is_exhaustive() {
    use crate::agent_runtime::contract::AgentRole;
    assert_eq!(
        native_stub_role_not_implemented_code(AgentRole::Upload),
        Some("upload_role_not_implemented")
    );
    assert_eq!(
        native_stub_role_not_implemented_code(AgentRole::Concurrency),
        Some("concurrency_role_not_implemented")
    );
    assert_ne!(
        native_stub_role_not_implemented_code(AgentRole::Upload),
        native_stub_role_not_implemented_code(AgentRole::Concurrency),
        "stub codes must stay distinct"
    );
    let stub_count = AgentRole::ALL
        .iter()
        .filter(|r| native_stub_role_not_implemented_code(**r).is_some())
        .count();
    assert_eq!(stub_count, 2, "exactly two stub roles, got {stub_count}");
    for role in AgentRole::ALL {
        if !matches!(role, AgentRole::Upload | AgentRole::Concurrency) {
            assert!(
                native_stub_role_not_implemented_code(*role).is_none(),
                "{role:?} must not carry a stub not-implemented code"
            );
        }
    }
}
