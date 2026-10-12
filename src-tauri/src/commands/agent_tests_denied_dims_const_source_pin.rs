
// U136: DENIED_BY_CONTRACT_DIMENSIONS must stay the exact triple (U18/U32).
#[test]
fn budget_source_pins_denied_by_contract_dimensions_const() {
    let src = include_str!("../agent_runtime/multi_agent/budget.rs");
    assert!(src.contains("DENIED_BY_CONTRACT_DIMENSIONS"));
    let start = src
        .find("pub const DENIED_BY_CONTRACT_DIMENSIONS")
        .expect("DENIED const");
    let end = src[start..]
        .find("];")
        .expect("DENIED const end")
        + start;
    let block = &src[start..end];
    for dim in ["browser_actions", "controlled_writes", "upload_bytes"] {
        assert!(block.contains(dim), "DENIED const missing {dim}");
    }
    // Exactly three string entries in the const table.
    let entries = block.matches('"').count() / 2;
    assert_eq!(entries, 3, "DENIED const must list exactly 3 dims, got {entries}");
}
