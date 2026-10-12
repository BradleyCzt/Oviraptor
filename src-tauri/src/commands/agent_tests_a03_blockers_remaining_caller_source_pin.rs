
// U170: the A03 producer-blockers accessor must stay called at the honest not_ready
// try_produce site, so the outstanding producer work cannot drift out of view
// (mirrors U146 wiring the A04 (a) blockers into each Root gate).
#[test]
fn coordinator_source_pins_a03_blockers_remaining_caller() {
    let src = include_str!("agent_native/coordinator_rust_policy.rs");
    assert!(
        src.contains("let _ = native_a03_producer_blockers_remaining(kind);"),
        "A03 blockers accessor call missing from not_ready try_produce site"
    );
    // The producer-ready honesty flag must not be removed either.
    assert!(
        src.contains("native_not_ready_supervision_producer_ready"),
        "producer_ready honesty flag missing"
    );
}
