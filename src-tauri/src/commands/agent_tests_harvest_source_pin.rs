
// U109: keep recent string-harvest patterns from regressing out of surface_discovery.
#[test]
fn surface_discovery_source_pins_recent_harvest_patterns() {
    let src = include_str!("agent_tools_http/surface_discovery.rs");
    for needle in [
        r#"\baction\s*=\s*["'](/[^"']+)["']"#,
        r#"\bhref\s*=\s*["'](/[^"']+)["']"#,
        "setAttribute",
        "routerLink|ng-href",
        "xlink:href",
        r#"\b(?:poster|cite)\s*="#,
        r#"\bsrcset\s*="#,
        "formaction",
        "imagesrcset",
        "og:url|og:image|twitter:url|twitter:image",
        r#"\bbackground\s*="#,
        r#"url\("#,
        r#"\b(?:longdesc|usemap)\s*="#,
        r#"<base\b"#,
    ] {
        assert!(
            src.contains(needle),
            "surface_discovery must keep harvest pattern containing {needle}"
        );
    }
}
