
// U166: pin the U164/U165 harvest patterns (img|input|frame src, fetch backtick)
// so a refactor cannot silently drop these element/template sources.
#[test]
fn surface_discovery_source_pins_legacy_and_template_patterns() {
    let src = include_str!("agent_tools_http/surface_discovery.rs");
    assert!(
        src.contains("<(?:img|input|frame)"),
        "img|input|frame src pattern missing"
    );
    assert!(
        src.contains("`(/[^`]+)`"),
        "fetch backtick template pattern missing"
    );
    // The backtick pattern must stay bound to fetch(.
    assert!(src.contains("\\bfetch\\(\\s*`"), "fetch backtick binding missing");
}
