
// U137: Root prepare (a) not implemented for gated roles — stay hard-false.
// U181 (pin updated, reason: U181 按计划只翻 InputParser): InputParser's (a) blocker
// inventory is empty and U180 proved the production end-to-end dispatch, so it alone is
// now true; BusinessLogic/Authorization/ExternalSurface and the Upload/Concurrency stubs
// stay hard-false. Exact per-role values, nothing relaxed.
#[test]
fn native_root_role_prepare_implemented_is_false_for_gated_roles() {
    use crate::agent_runtime::contract::AgentRole::*;
    assert!(
        native_root_role_prepare_implemented(InputParser),
        "U181: InputParser prepare (a) is implemented"
    );
    for role in [
        BusinessLogic,
        Authorization,
        ExternalSurface,
        Upload,
        Concurrency,
    ] {
        assert!(
            !native_root_role_prepare_implemented(role),
            "{role:?} prepare must stay unimplemented until deliberate (a) slice"
        );
    }
    // Every role other than InputParser is false.
    for role in crate::agent_runtime::contract::AgentRole::ALL {
        assert_eq!(native_root_role_prepare_implemented(*role), *role == InputParser, "{role:?}");
    }
}
