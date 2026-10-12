// U39: every AgentRole has an explicit yes/no for undispatched worker replacement.
#[test]
fn undispatched_worker_replacement_inventory_is_exhaustive() {
    use crate::agent_runtime::contract::AgentRole;

    assert_eq!(AgentRole::ALL.len(), 15, "all fifteen AgentRole variants must be listed");

    let allowed = [
        AgentRole::SpaApiMapper,
        AgentRole::IdentitySession,
        AgentRole::DeepInvestigator,
        AgentRole::ClientSide,
        AgentRole::InputParser,
        AgentRole::BusinessLogic,
        AgentRole::ExternalSurface,
    ];
    let denied = [
        AgentRole::Coordinator,
        AgentRole::RepoMapper,
        AgentRole::SourceAnalyst,
        AgentRole::WebExecutor,
        AgentRole::Authorization,
        AgentRole::Upload,
        AgentRole::Concurrency,
        AgentRole::EvidenceReviewer,
    ];
    assert_eq!(allowed.len() + denied.len(), 15);

    for role in allowed {
        assert!(
            role.allows_undispatched_worker_replacement(),
            "{role:?} must allow undispatched replacement"
        );
    }
    for role in denied {
        assert!(
            !role.allows_undispatched_worker_replacement(),
            "{role:?} must keep a dedicated recovery path"
        );
    }
    for role in AgentRole::ALL {
        let expect = allowed.contains(role);
        assert_eq!(
            role.allows_undispatched_worker_replacement(),
            expect,
            "{role:?} drift vs U39 inventory"
        );
    }
}
