
// U146: each A04 Root gate must keep the (a) blockers source-visible in its
// not_ready branch — so the honest reason it stays not_ready cannot drift away.
#[test]
fn gate_source_pins_prepare_a_blockers_remaining_call() {
    for (file, src) in [
        (
            "input_parser",
            include_str!("agent_native/coordinator_input_parser_gate.rs"),
        ),
        (
            "business_logic",
            include_str!("agent_native/coordinator_business_logic_gate.rs"),
        ),
        (
            "authorization",
            include_str!("agent_native/coordinator_authorization_gate.rs"),
        ),
        (
            "external_surface",
            include_str!("agent_native/coordinator_external_surface_gate.rs"),
        ),
    ] {
        assert!(
            src.contains("native_root_prepare_a_blockers_remaining"),
            "{file} gate must reference the (a) blockers inventory"
        );
        // Honesty guards must not be removed either.
        assert!(
            src.contains("native_root_role_prepare_implemented"),
            "{file} gate must still consult prepare_implemented"
        );
        assert!(
            src.contains("dispatch_not_ready") || src.contains("DISPATCH_NOT_READY"),
            "{file} gate must still return a not_ready code"
        );
    }
}
