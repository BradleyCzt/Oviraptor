
// U172: per reserved-output-kind view of the Root prepare (a) blockers, mirroring the
// A03 per-kind accessor. Role-to-output mapping remains independent of wired paid consumers.
// The four role-output prepare contracts expose the shared blocker
// inventory; wired / A03 not_ready / unknown kinds expose none. Exposing them must never
// flip prepare_implemented to true.
#[test]
fn native_root_prepare_a_blockers_remaining_for_kind_matches_reserved_kinds() {
    for kind in [
        "input-parser-output",
        "business-logic-output",
        "authorization-output",
        "external-surface-output",
    ] {
        let got = native_root_prepare_a_blockers_remaining_for_kind(kind)
            .unwrap_or_else(|| panic!("reserved kind {kind} must expose (a) blockers"));
        // U175/U177/U178/U179 (pin changed, reason: cleared `zero_write_proof_on_failure`):
        // the InputParser kind reuses the per-role view, which has now cleared every (a)
        // blocker. The other prepare contracts keep the full shared inventory. Exposing an
        // empty list must not flip prepare_implemented (checked below).
        if kind == "input-parser-output" {
            assert_eq!(got, NATIVE_INPUT_PARSER_PREPARE_A_BLOCKERS, "{kind}");
            assert!(got.is_empty(), "{kind}: every InputParser (a) blocker is cleared");
            for cleared in [
                "freeze_bound_basis",
                "scheduled_child_mint",
                "reviewer_floor_grant_wire",
                "readonly_sdk_input_surface",
                "zero_write_proof_on_failure",
            ] {
                assert!(!got.contains(&cleared), "{kind} still lists {cleared}");
                assert!(NATIVE_ROOT_PREPARE_A_BLOCKERS.contains(&cleared), "{cleared}");
            }
        } else {
            assert_eq!(got, NATIVE_ROOT_PREPARE_A_BLOCKERS, "{kind}");
            assert!(!got.is_empty(), "{kind} blockers must be non-empty");
        }
    }
    for kind in [
        "human-directive",
        "budget-allocation",
        "mapper-output",
        "reviewer-decision",
        "new-evidence",
        "lease-expired",
        "protection-changed",
        "human-recovery",
        "totally-unknown-kind",
    ] {
        assert!(
            native_root_prepare_a_blockers_remaining_for_kind(kind).is_none(),
            "non-reserved kind {kind} must have no (a) blockers"
        );
    }
    use crate::agent_runtime::contract::AgentRole;
    for role in [
        AgentRole::InputParser,
        AgentRole::BusinessLogic,
        AgentRole::Authorization,
        AgentRole::ExternalSurface,
    ] {
        // U181 (pin updated, reason: U181 按计划只翻 InputParser): the per-kind view
        // never flips a flag by itself; only InputParser (empty inventory) is true.
        assert_eq!(
            native_root_role_prepare_implemented(role),
            role == AgentRole::InputParser,
            "exposing per-kind blockers must not flip prepare_implemented for {role:?}"
        );
    }
}
