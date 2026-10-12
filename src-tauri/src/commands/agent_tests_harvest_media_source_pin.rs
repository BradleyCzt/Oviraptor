
// U153: keep the U148-U150 media/lazy harvest patterns from regressing out.
#[test]
fn surface_discovery_source_pins_media_harvest_patterns() {
    let src = include_str!("agent_tools_http/surface_discovery.rs");
    for needle in [
        r#"<track\b"#,
        r#"<(?:video|audio|source)\b"#,
        r#"\bdata-src\s*="#,
    ] {
        assert!(
            src.contains(needle),
            "surface_discovery must keep harvest pattern containing {needle}"
        );
    }
}
