
// U145: per-role Root prepare (a) blockers view stays honest.
#[test]
fn native_root_prepare_a_blockers_remaining_matches_gated_roles() {
    use crate::agent_runtime::contract::AgentRole;
    // U175: InputParser cleared `freeze_bound_basis` (real frozen, re-verified trigger
    // frame). U177 (pin updated, reason: real fixed-grant child mint behind the
    // Reviewer + Root supervision floors) cleared `scheduled_child_mint` and
    // `reviewer_floor_grant_wire`; the other A04 items stay open. The other three A04
    // Root-gated roles still share the full outstanding blocker list.
    // U178 cleared `readonly_sdk_input_surface`; U179 (pin updated, reason: cleared
    // `zero_write_proof_on_failure`) clears the final item, so InputParser has cleared
    // every shared (a) blocker and its inventory is empty.
    const CLEARED: [&str; 5] = ["freeze_bound_basis", "scheduled_child_mint", "reviewer_floor_grant_wire", "readonly_sdk_input_surface", "zero_write_proof_on_failure"];
    let input_parser_expected: Vec<&str> = NATIVE_ROOT_PREPARE_A_BLOCKERS
        .iter()
        .copied()
        .filter(|item| !CLEARED.contains(item))
        .collect();
    assert_eq!(input_parser_expected.len(), NATIVE_ROOT_PREPARE_A_BLOCKERS.len() - 5);
    assert!(input_parser_expected.is_empty());
    assert_eq!(
        native_root_prepare_a_blockers_remaining(AgentRole::InputParser),
        Some(input_parser_expected.as_slice())
    );
    for role in [
        AgentRole::InputParser,
        AgentRole::BusinessLogic,
        AgentRole::Authorization,
        AgentRole::ExternalSurface,
    ] {
        let remaining = native_root_prepare_a_blockers_remaining(role)
            .unwrap_or_else(|| panic!("{role:?} must expose (a) blockers"));
        if role == AgentRole::InputParser {
            assert!(CLEARED.iter().all(|item| !remaining.contains(item)));
        } else {
            assert_eq!(
                remaining, NATIVE_ROOT_PREPARE_A_BLOCKERS,
                "{role:?} must share the full blocker inventory"
            );
        }
        if role != AgentRole::InputParser {
            assert!(!remaining.is_empty());
        }
        // Honesty: U181 (pin updated, reason: U181 按计划只翻 InputParser) flipped
        // prepare only for InputParser, whose inventory is empty; the others still
        // have the full blocker inventory and stay unimplemented.
        assert_eq!(native_root_role_prepare_implemented(role), role == AgentRole::InputParser, "{role:?}");
        assert_eq!(native_root_role_prepare_implemented(role), remaining.is_empty(), "{role:?}: flag true iff no blockers");
    }
    // Every other role has no pending Root prepare (a) work.
    for role in [
        AgentRole::Coordinator,
        AgentRole::WebExecutor,
        AgentRole::SpaApiMapper,
        AgentRole::ClientSide,
        AgentRole::IdentitySession,
        AgentRole::EvidenceReviewer,
        AgentRole::DeepInvestigator,
        AgentRole::RepoMapper,
        AgentRole::SourceAnalyst,
        AgentRole::Upload,
        AgentRole::Concurrency,
    ] {
        assert!(
            native_root_prepare_a_blockers_remaining(role).is_none(),
            "{role:?} must not expose A04 (a) blockers"
        );
    }
}
