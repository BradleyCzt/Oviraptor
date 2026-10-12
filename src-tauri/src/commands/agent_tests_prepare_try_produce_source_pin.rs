// U133: Root prepare gates must call named try_produce before returning not_ready.
#[test]
fn gate_files_source_pin_prepare_calls_try_produce() {
    let checks = [
        (
            include_str!("agent_native/coordinator_input_parser_gate.rs"),
            "native_try_produce_input_parser_output_frame",
        ),
        (
            include_str!("agent_native/coordinator_business_logic_gate.rs"),
            "native_try_produce_business_logic_output_frame",
        ),
        (
            include_str!("agent_native/coordinator_authorization_gate.rs"),
            "native_try_produce_authorization_output_frame",
        ),
        (
            include_str!("agent_native/coordinator_external_surface_gate.rs"),
            "native_try_produce_external_surface_output_frame",
        ),
    ];
    for (src, call) in checks {
        assert!(src.contains(call), "prepare gate missing {call}");
        assert!(src.contains("native_root_role_prepare_implemented"), "U138: must consult prepare_implemented");
        assert!(
            src.contains("\"prepare\""),
            "must pass prepare semantic"
        );
    }
}
