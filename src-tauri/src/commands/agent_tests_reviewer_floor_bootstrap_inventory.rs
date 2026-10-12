// U43: every AgentRole has an explicit yes/no for web Reviewer floor on bootstrap.
#[test]
fn web_reviewer_floor_bootstrap_inventory_is_exhaustive() {
    use crate::agent_runtime::contract::AgentRole;

    assert_eq!(AgentRole::ALL.len(), 15);

    // SpaApiMapper is readonly bootstrap but does not take the web Reviewer floor.
    let allowed = [
        AgentRole::ClientSide,
        AgentRole::IdentitySession,
        AgentRole::InputParser,
        AgentRole::BusinessLogic,
    ];
    for role in allowed {
        assert!(
            role.requires_web_reviewer_floor_on_bootstrap(),
            "{role:?} must require Reviewer floor on bootstrap"
        );
    }
    for role in AgentRole::ALL {
        let expect = allowed.contains(role);
        assert_eq!(
            role.requires_web_reviewer_floor_on_bootstrap(),
            expect,
            "{role:?} drift vs U43 inventory"
        );
    }
    assert!(!AgentRole::SpaApiMapper.requires_web_reviewer_floor_on_bootstrap());
    assert!(AgentRole::SpaApiMapper.is_readonly_bootstrap_role());
}
