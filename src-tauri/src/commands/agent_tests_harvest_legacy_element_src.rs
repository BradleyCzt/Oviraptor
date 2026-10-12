
    #[test]
    fn agent_harvest_surface_paths_finds_legacy_element_src() {
        // U164: <img|input|frame src="/..."> element sources were never harvested
        // (script/iframe/embed/object/track/video/audio/source had patterns, these
        // three did not). Single-segment, extension-less paths prove the new regex:
        // the two-segment /Controller/Action fallback cannot catch one segment, and
        // `.src=` needs a dot (tag `src=` has a space), so nothing caught these before.
        let html = concat!(
            "<img src=\"/JschxxDynImg\">",
            "<input type=\"image\" src=\"/JschxxCaptcha\">",
            "<frame src=\"/JschxxFrameDoc\">",
        );
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxDynImg"), "img src missing: {joined}");
        assert!(joined.contains("/JschxxCaptcha"), "input src missing: {joined}");
        assert!(joined.contains("/JschxxFrameDoc"), "frame src missing: {joined}");
    }
