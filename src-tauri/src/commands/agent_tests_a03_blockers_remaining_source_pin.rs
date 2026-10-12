
// U163: pin the U162 per-kind A03 blockers accessor so it stays honest and wired to
// the not_ready kinds + the shared blocker const (no silent flip to a live producer).
#[test]
fn coordinator_source_pins_a03_blockers_remaining_accessor() {
    let src = include_str!("agent_native/coordinator_rust_policy.rs");
    let start = src
        .find("fn native_a03_producer_blockers_remaining")
        .expect("per-kind A03 blockers accessor must exist");
    let end = src[start..].find("\n}\n").expect("accessor body end") + start;
    let body = &src[start..end];
    assert!(
        body.contains("NATIVE_NOT_READY_SUPERVISION_FRAME_KINDS"),
        "accessor must gate on the not_ready supervision kinds"
    );
    assert!(
        body.contains("NATIVE_A03_PRODUCER_BLOCKERS"),
        "accessor must return the shared A03 blocker inventory"
    );
    assert!(
        body.contains("None"),
        "accessor must return None for kinds with no pending A03 producer work"
    );
}
