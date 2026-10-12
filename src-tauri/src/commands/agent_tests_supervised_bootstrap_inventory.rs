// U41: every AgentRole has an explicit yes/no for supervised readonly bootstrap.
#[test]
fn supervised_readonly_bootstrap_inventory_is_exhaustive() {
    use crate::agent_runtime::contract::AgentRole;

    assert_eq!(AgentRole::ALL.len(), 15);

    let allowed = [
        AgentRole::SpaApiMapper,
        AgentRole::IdentitySession,
        AgentRole::ClientSide,
        AgentRole::InputParser,
        AgentRole::BusinessLogic,
    ];
    let denied = [
        AgentRole::Coordinator,
        AgentRole::RepoMapper,
        AgentRole::SourceAnalyst,
        AgentRole::WebExecutor,
        AgentRole::ExternalSurface,
        AgentRole::Authorization,
        AgentRole::Upload,
        AgentRole::Concurrency,
        AgentRole::DeepInvestigator,
        AgentRole::EvidenceReviewer,
    ];
    assert_eq!(allowed.len() + denied.len(), 15);

    for role in allowed {
        assert!(
            role.allows_supervised_readonly_bootstrap(),
            "{role:?} must allow supervised readonly bootstrap"
        );
    }
    for role in denied {
        assert!(
            !role.allows_supervised_readonly_bootstrap(),
            "{role:?} must keep a dedicated recovery path"
        );
    }
    for role in AgentRole::ALL {
        let expect = allowed.contains(role);
        assert_eq!(
            role.allows_supervised_readonly_bootstrap(),
            expect,
            "{role:?} drift vs U41 inventory"
        );
    }
}
