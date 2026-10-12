
// U116: Surface explore stays string_only until CDP BFS is deliberately shipped.
#[test]
fn surface_discovery_source_pins_string_only_explore_mode() {
    let src = include_str!("agent_tools_http/surface_discovery.rs");
    assert!(
        src.contains("NATIVE_SURFACE_EXPLORE_MODE"),
        "must keep NATIVE_SURFACE_EXPLORE_MODE"
    );
    assert!(
        src.contains("string_only"),
        "explore mode must remain string_only until click_explore is ready"
    );
    assert!(
        src.contains(r#"= "string_only""#),
        "must assign string_only literally"
    );
    assert!(
        src.contains("native_surface_click_explore_candidates"),
        "must keep click_explore entry (not_ready path)"
    );
}
