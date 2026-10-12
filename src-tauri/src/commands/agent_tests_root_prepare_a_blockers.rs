
// U142: (a) blockers inventory must stay non-empty until true prepare ships.
#[test]
fn native_root_prepare_a_blockers_inventory_nonempty() {
    assert!(
        !NATIVE_ROOT_PREPARE_A_BLOCKERS.is_empty(),
        "blockers list must not silently empty before (a) is ready"
    );
    for item in [
        "freeze_bound_basis",
        "readonly_sdk_input_surface",
        "scheduled_child_mint",
        "reviewer_floor_grant_wire",
        "zero_write_proof_on_failure",
    ] {
        assert!(
            NATIVE_ROOT_PREPARE_A_BLOCKERS.contains(&item),
            "missing blocker {item}"
        );
    }
    // prepare_implemented stays false while blockers remain.
    // U181 (pin updated, reason: U181 按计划只翻 InputParser): the shared list is
    // still the inventory of BusinessLogic/Authorization/ExternalSurface, which stay
    // false; only InputParser (its own inventory empty) flipped to true.
    use crate::agent_runtime::contract::AgentRole;
    for role in [AgentRole::BusinessLogic, AgentRole::Authorization, AgentRole::ExternalSurface] {
        assert_eq!(native_root_prepare_a_blockers_remaining(role), Some(NATIVE_ROOT_PREPARE_A_BLOCKERS));
        assert!(!native_root_role_prepare_implemented(role), "{role:?}");
    }
    assert!(NATIVE_INPUT_PARSER_PREPARE_A_BLOCKERS.is_empty());
    assert!(native_root_role_prepare_implemented(AgentRole::InputParser));
}
