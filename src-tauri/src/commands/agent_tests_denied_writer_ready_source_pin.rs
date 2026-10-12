
// U115: DENIED budget dims must keep writer_ready honesty helper (no silent live writers).
#[test]
fn budget_source_pins_denied_writer_ready() {
    let src = include_str!("../agent_runtime/multi_agent/budget.rs");
    assert!(
        src.contains("denied_budget_dimension_writer_ready"),
        "budget.rs must define denied_budget_dimension_writer_ready"
    );
    for dim in ["browser_actions", "controlled_writes", "upload_bytes"] {
        assert!(
            src.contains(dim),
            "budget.rs must name DENIED dim {dim}"
        );
    }
}
