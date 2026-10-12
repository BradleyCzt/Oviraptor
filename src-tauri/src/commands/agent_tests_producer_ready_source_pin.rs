
// U121/U122: producer_ready helpers stay hard-false (skeletons ≠ live producers).
#[test]
fn policy_source_pins_producer_ready_hard_false() {
    let src = include_str!("agent_native/coordinator_rust_policy.rs");
    assert!(src.contains("fn native_not_ready_supervision_producer_ready"));
    assert!(src.contains("fn native_reserved_role_output_producer_ready"));
    // Each ready helper must still return a literal false after the inventory gate.
    let a03 = src
        .split("fn native_not_ready_supervision_producer_ready")
        .nth(1)
        .and_then(|s| s.split("fn native_try_produce_not_ready_supervision_frame").next())
        .expect("a03 producer_ready body");
    assert!(
        a03.contains("false") && !a03.contains("true"),
        "A03 producer_ready must stay hard-false: {a03}"
    );
    let a04 = src
        .split("fn native_reserved_role_output_producer_ready")
        .nth(1)
        .and_then(|s| s.split("fn native_try_produce_reserved_role_output_frame").next())
        .expect("a04 producer_ready body");
    assert!(
        a04.contains("false") && !a04.contains("true"),
        "A04 producer_ready must stay hard-false: {a04}"
    );
}
