
// U159: keep the U154-U158 harvest patterns from regressing out of surface_discovery.
#[test]
fn surface_discovery_source_pins_config_harvest_patterns() {
    let src = include_str!("agent_tools_http/surface_discovery.rs");
    for needle in [
        r#"new\s+Request\("#,
        r#""(?:api|apiUrl|apiBase|endpoint|baseURL|basePath)""#,
        r#"\.src\s*=\s*["']"#,
        r#"@import\s+"#,
        r#"importScripts\("#,
    ] {
        assert!(
            src.contains(needle),
            "surface_discovery must keep harvest pattern containing {needle}"
        );
    }
}
