// U30/U31: every AgentRole maps to a stable schedule_decision gate code or step.
#[test]
fn schedule_decision_role_gate_is_exhaustive_and_stable() {
    use crate::agent_runtime::contract::AgentRole;
    let cases: &[(AgentRole, Result<&str, &str>)] = &[
        (AgentRole::WebExecutor, Ok("dispatch:web_executor")),
        (AgentRole::DeepInvestigator, Ok("dispatch:deep_investigator")),
        (AgentRole::InputParser, Err("input_parser_dispatch_not_ready")),
        (AgentRole::BusinessLogic, Err("business_logic_dispatch_not_ready")),
        (AgentRole::Upload, Err("upload_role_not_implemented")),
        (AgentRole::Concurrency, Err("concurrency_role_not_implemented")),
        (AgentRole::Authorization, Err("authorization_dispatch_not_ready")),
        (AgentRole::ExternalSurface, Err("external_surface_dispatch_not_ready")),
        (AgentRole::Coordinator, Err("child_role_must_not_be_coordinator")),
        (AgentRole::SpaApiMapper, Err("spa_api_mapper_requires_bootstrap_prepare")),
        (AgentRole::RepoMapper, Err("source_role_requires_source_prepare")),
        (AgentRole::SourceAnalyst, Err("source_role_requires_source_prepare")),
        (AgentRole::IdentitySession, Err("identity_session_requires_frame_hook")),
        (AgentRole::ClientSide, Err("client_side_requires_frame_hook")),
        (AgentRole::EvidenceReviewer, Err("evidence_reviewer_requires_review_frame")),
    ];
    assert_eq!(cases.len(), AgentRole::ALL.len(), "all AgentRole variants must be listed");
    let listed: Vec<AgentRole> = cases.iter().map(|(r, _)| *r).collect();
    for role in AgentRole::ALL {
        assert!(
            listed.contains(role),
            "{role:?} missing from schedule_decision gate cases"
        );
    }
    for (role, expected) in cases {
        let got = native_root_schedule_decision_role_gate(*role);
        match expected {
            Ok(step) => assert_eq!(got.as_deref(), Ok(*step), "{role:?}"),
            Err(code) => assert_eq!(got.as_deref().unwrap_err(), *code, "{role:?}"),
        }
    }
}

#[test]
fn schedule_decision_role_gate_never_returns_generic_not_bounded() {
    use crate::agent_runtime::contract::AgentRole;
    for role in AgentRole::ALL {
        match native_root_schedule_decision_role_gate(*role) {
            Ok(_) => {}
            Err(code) => assert_ne!(
                code, "root_decision_role_not_bounded",
                "{role:?} must use a distinct entry/not-ready code"
            ),
        }
    }
}
