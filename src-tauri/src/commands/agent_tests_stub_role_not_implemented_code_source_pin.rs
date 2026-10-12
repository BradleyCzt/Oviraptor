
// U171: pin the U147 stub not-implemented-code accessor so a new stub role cannot
// silently reuse a neighbour's code and the two distinct codes cannot drift.
// (U134's pin covers the prepare_* entries/codes but not this accessor map.)
#[test]
fn coordinator_source_pins_stub_role_not_implemented_code() {
    let src = include_str!("agent_native/coordinator_stub_role_gate.rs");
    let start = src
        .find("fn native_stub_role_not_implemented_code")
        .expect("stub not-implemented-code accessor");
    let end = src[start..].find("\n}").expect("accessor end") + start;
    let block = &src[start..end];
    assert!(
        block.contains("AgentRole::Upload => Some(NATIVE_UPLOAD_ROLE_NOT_IMPLEMENTED)"),
        "Upload must map to its own not_implemented code"
    );
    assert!(
        block.contains("AgentRole::Concurrency => Some(NATIVE_CONCURRENCY_ROLE_NOT_IMPLEMENTED)"),
        "Concurrency must map to its own not_implemented code"
    );
    assert!(
        block.contains("_ => None"),
        "every other role must return None, never a borrowed code"
    );
}
