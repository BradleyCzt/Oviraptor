
// U144: pin the A03 producer blockers inventory against silent drift / emptying.
#[test]
fn coordinator_source_pins_a03_producer_blockers_const() {
    let src = include_str!("agent_native/coordinator_rust_policy.rs");
    assert!(src.contains("NATIVE_A03_PRODUCER_BLOCKERS"));
    let start = src
        .find("const NATIVE_A03_PRODUCER_BLOCKERS")
        .expect("A03 blockers const");
    let end = src[start..]
        .find("];")
        .expect("A03 blockers const end")
        + start;
    let block = &src[start..end];
    for item in [
        "lease_scan_source",
        "evidence_revision_feed",
        "protection_watch_source",
        "human_recovery_directive_feed",
        "zero_write_proof_on_failure",
    ] {
        assert!(block.contains(item), "A03 blockers const missing {item}");
    }
    let entries = block.matches('"').count() / 2;
    assert_eq!(entries, 5, "A03 blockers const must list exactly 5 items, got {entries}");
}
