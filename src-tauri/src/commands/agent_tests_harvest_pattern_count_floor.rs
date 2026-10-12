
// U124: string-harvest pattern table must not silently shrink.
#[test]
fn surface_discovery_harvest_pattern_count_floor() {
    let src = include_str!("agent_tools_http/surface_discovery.rs");
    let start = src
        .find("let patterns = [")
        .expect("patterns table");
    let end = src[start..]
        .find("];")
        .expect("patterns table end")
        + start;
    let block = &src[start..end];
    let needle = "(r#\"";
    let count = block.matches(needle).count();
    assert!(
        count >= 55,
        "expected >= 55 harvest regex patterns, got {}",
        count
    );
}
