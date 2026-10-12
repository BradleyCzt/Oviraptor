// U54: Root paid-dispatch not_ready / stub roles stay explicit vs AgentRole::ALL.
#[test]
fn root_paid_dispatch_not_ready_inventory_is_stable() {
    use crate::agent_runtime::contract::AgentRole;

    let not_ready = [
        (AgentRole::InputParser, "input_parser_dispatch_not_ready"),
        (AgentRole::BusinessLogic, "business_logic_dispatch_not_ready"),
        (AgentRole::Authorization, "authorization_dispatch_not_ready"),
        (AgentRole::ExternalSurface, "external_surface_dispatch_not_ready"),
    ];
    let stubs = [
        (AgentRole::Upload, "upload_role_not_implemented"),
        (AgentRole::Concurrency, "concurrency_role_not_implemented"),
    ];
    for (role, code) in not_ready.iter().chain(stubs.iter()) {
        assert_eq!(
            native_root_schedule_decision_role_gate(*role).unwrap_err(),
            *code,
            "{role:?}"
        );
    }
    // Roles with live Root dispatch steps stay Ok.
    assert_eq!(
        native_root_schedule_decision_role_gate(AgentRole::WebExecutor).unwrap(),
        "dispatch:web_executor"
    );
    assert_eq!(
        native_root_schedule_decision_role_gate(AgentRole::DeepInvestigator).unwrap(),
        "dispatch:deep_investigator"
    );
}
