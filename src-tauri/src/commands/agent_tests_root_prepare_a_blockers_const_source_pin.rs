
// U169: pin the A04 Root prepare (a) blockers inventory against silent drift /
// emptying — mirrors the A03 producer-blockers const pin (U144). Emptying this
// list must stay a deliberate, reviewed change, never an accidental refactor.
#[test]
fn coordinator_source_pins_root_prepare_a_blockers_const() {
    let src = include_str!("agent_native/coordinator_rust_policy.rs");
    assert!(src.contains("NATIVE_ROOT_PREPARE_A_BLOCKERS"));
    let start = src
        .find("const NATIVE_ROOT_PREPARE_A_BLOCKERS")
        .expect("A04 prepare blockers const");
    let end = src[start..]
        .find("];")
        .expect("A04 prepare blockers const end")
        + start;
    let block = &src[start..end];
    for item in [
        "freeze_bound_basis",
        "readonly_sdk_input_surface",
        "scheduled_child_mint",
        "reviewer_floor_grant_wire",
        "zero_write_proof_on_failure",
    ] {
        assert!(block.contains(item), "A04 prepare blockers const missing {item}");
    }
    let entries = block.matches('"').count() / 2;
    assert_eq!(entries, 5, "A04 prepare blockers const must list exactly 5 items, got {entries}");
}
