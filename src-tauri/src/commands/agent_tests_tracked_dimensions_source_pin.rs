
// U135: TRACKED_DIMENSIONS must keep the seven live-writer dims (U11–U13).
#[test]
fn budget_gaps_source_pins_tracked_dimensions() {
    let src = include_str!("../agent_runtime/multi_agent/budget_gaps.rs");
    assert!(src.contains("TRACKED_DIMENSIONS"));
    for dim in [
        "model_input_tokens",
        "model_cached_tokens",
        "model_output_tokens",
        "model_requests",
        "target_requests",
        "concurrency_batches",
        "wall_time_ms",
    ] {
        assert!(src.contains(dim), "missing tracked dim {dim}");
    }
}
