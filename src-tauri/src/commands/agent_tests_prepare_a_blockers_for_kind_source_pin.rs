
// U173: pin the U172 per-kind A04 blockers accessor so it keeps delegating through the
// reserved-kind→role map and the per-role blockers, and never grows a `=> true` arm
// (which would amount to silently claiming prepare is implemented).
#[test]
fn coordinator_source_pins_prepare_a_blockers_remaining_for_kind() {
    let src = include_str!("agent_native/coordinator_rust_policy.rs");
    let start = src
        .find("fn native_root_prepare_a_blockers_remaining_for_kind")
        .expect("per-kind A04 blockers accessor");
    let end = src[start..].find("\n}").expect("accessor end") + start;
    let block = &src[start..end];
    assert!(
        block.contains("native_agent_role_for_reserved_role_output_kind(kind)?"),
        "accessor must resolve the kind to a role via the reserved-kind map"
    );
    assert!(
        block.contains("native_root_prepare_a_blockers_remaining(role)"),
        "accessor must reuse the per-role blockers inventory"
    );
    assert!(
        !block.contains("=> true"),
        "accessor must not grow a => true arm"
    );
}
