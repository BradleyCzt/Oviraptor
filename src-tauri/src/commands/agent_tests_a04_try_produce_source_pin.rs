
// U131: A04 named reserved role-output try_produce wrappers must stay present.
#[test]
fn policy_source_pins_a04_named_try_produce() {
    let src = include_str!("agent_native/coordinator_rust_policy.rs");
    for name in [
        "native_try_produce_input_parser_output_frame",
        "native_try_produce_business_logic_output_frame",
        "native_try_produce_authorization_output_frame",
        "native_try_produce_external_surface_output_frame",
    ] {
        assert!(src.contains(name), "missing {name}");
    }
}
