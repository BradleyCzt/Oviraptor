
// U119: Root IP/BL/AZ/ES gates stay honest not_ready until true prepare ships.
#[test]
fn root_gate_files_source_pin_dispatch_not_ready() {
    let files = [
        (
            "agent_native/coordinator_input_parser_gate.rs",
            include_str!("agent_native/coordinator_input_parser_gate.rs"),
            "input_parser_dispatch_not_ready",
        ),
        (
            "agent_native/coordinator_business_logic_gate.rs",
            include_str!("agent_native/coordinator_business_logic_gate.rs"),
            "business_logic_dispatch_not_ready",
        ),
        (
            "agent_native/coordinator_authorization_gate.rs",
            include_str!("agent_native/coordinator_authorization_gate.rs"),
            "authorization_dispatch_not_ready",
        ),
        (
            "agent_native/coordinator_external_surface_gate.rs",
            include_str!("agent_native/coordinator_external_surface_gate.rs"),
            "external_surface_dispatch_not_ready",
        ),
    ];
    for (name, src, code) in files {
        assert!(
            src.contains(code),
            "{name} must keep {code}"
        );
    }
}
