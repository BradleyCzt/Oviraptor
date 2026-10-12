// U46: every AgentRole has an explicit yes/no for read_only_analysis lane.
#[test]
fn readonly_analysis_lane_inventory_is_exhaustive() {
    use crate::agent_runtime::contract::AgentRole;

    assert_eq!(AgentRole::ALL.len(), 15);
    let allowed = [
        AgentRole::SpaApiMapper,
        AgentRole::IdentitySession,
        AgentRole::RepoMapper,
        AgentRole::SourceAnalyst,
        AgentRole::ClientSide,
        AgentRole::InputParser,
        AgentRole::BusinessLogic,
    ];
    for role in allowed {
        assert!(
            role.requires_read_only_analysis_lane(),
            "{role:?} must require read_only_analysis lane"
        );
        assert!(role.is_readonly_bootstrap_role());
    }
    for role in AgentRole::ALL {
        let expect = allowed.contains(role);
        assert_eq!(
            role.requires_read_only_analysis_lane(),
            expect,
            "{role:?} drift vs U46 inventory"
        );
    }
}
