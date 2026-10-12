
// U101: keep observations wired to click_explore entry (string_only → not_ready).
#[test]
fn observations_source_pins_click_explore_entry() {
    let src = include_str!("agent_tools_http/observations.rs");
    assert!(
        src.contains("native_surface_click_explore_candidates"),
        "observations must call click_explore entry"
    );
    assert!(
        src.contains("surface_click_explore_not_ready"),
        "observations must treat click_explore not_ready as expected"
    );
    assert!(
        src.contains("surface-click:"),
        "Ok branch must emit surface-click: inventory tags when ready"
    );
}
