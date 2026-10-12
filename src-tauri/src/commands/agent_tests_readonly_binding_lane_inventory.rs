// U53: every AgentRole has an explicit binding lane (or none).
#[test]
fn specialist_readonly_binding_lane_inventory_is_exhaustive() {
    use crate::agent_runtime::contract::AgentRole;

    assert_eq!(AgentRole::ALL.len(), 15);
    let expected: &[(AgentRole, Option<&str>)] = &[
        (AgentRole::Coordinator, None),
        (AgentRole::SpaApiMapper, Some("read_only_analysis")),
        (AgentRole::RepoMapper, Some("read_only_analysis")),
        (AgentRole::SourceAnalyst, Some("read_only_analysis")),
        (AgentRole::WebExecutor, None),
        (AgentRole::ExternalSurface, Some("target_touching")),
        (AgentRole::IdentitySession, Some("read_only_analysis")),
        (AgentRole::Authorization, None),
        (AgentRole::InputParser, Some("read_only_analysis")),
        (AgentRole::Upload, None),
        (AgentRole::BusinessLogic, Some("read_only_analysis")),
        (AgentRole::Concurrency, None),
        (AgentRole::ClientSide, Some("read_only_analysis")),
        (AgentRole::DeepInvestigator, Some("read_only_analysis")),
        (AgentRole::EvidenceReviewer, Some("review")),
    ];
    assert_eq!(expected.len(), 15);
    for role in AgentRole::ALL {
        let want = expected
            .iter()
            .find(|(r, _)| *r == *role)
            .map(|(_, lane)| *lane)
            .expect("role listed");
        assert_eq!(
            role.specialist_readonly_binding_lane(),
            want,
            "{role:?} drift vs U53 inventory"
        );
    }
}
