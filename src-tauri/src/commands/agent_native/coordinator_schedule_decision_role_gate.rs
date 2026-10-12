/// Exhaustive Root `schedule_decision` role gate: Ok(step) means the role may
/// continue into frame/suggestion checks; Err is a stable terminal code.
fn native_root_schedule_decision_role_gate(
    role: crate::agent_runtime::contract::AgentRole,
) -> Result<&'static str, String> {
    use crate::agent_runtime::contract::AgentRole;
    match role {
        AgentRole::WebExecutor => Ok("dispatch:web_executor"),
        AgentRole::DeepInvestigator => Ok("dispatch:deep_investigator"),
        AgentRole::InputParser => Err(NATIVE_INPUT_PARSER_DISPATCH_NOT_READY.into()),
        AgentRole::BusinessLogic => Err(NATIVE_BUSINESS_LOGIC_DISPATCH_NOT_READY.into()),
        AgentRole::Upload => Err(NATIVE_UPLOAD_ROLE_NOT_IMPLEMENTED.into()),
        AgentRole::Concurrency => Err(NATIVE_CONCURRENCY_ROLE_NOT_IMPLEMENTED.into()),
        AgentRole::Authorization => Err(NATIVE_AUTHORIZATION_DISPATCH_NOT_READY.into()),
        AgentRole::ExternalSurface => Err(NATIVE_EXTERNAL_SURFACE_DISPATCH_NOT_READY.into()),
        AgentRole::Coordinator => Err("child_role_must_not_be_coordinator".into()),
        AgentRole::SpaApiMapper => Err("spa_api_mapper_requires_bootstrap_prepare".into()),
        AgentRole::ClientSide => Err("client_side_requires_frame_hook".into()),
        AgentRole::IdentitySession => Err("identity_session_requires_frame_hook".into()),
        AgentRole::EvidenceReviewer => Err("evidence_reviewer_requires_review_frame".into()),
        AgentRole::RepoMapper | AgentRole::SourceAnalyst => {
            Err("source_role_requires_source_prepare".into())
        }
    }
}
