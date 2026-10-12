
    #[test]
    fn agent_harvest_surface_paths_finds_json_config_endpoint() {
        // U155: JSON/config endpoint keys like "api":"/...". The quoted key breaks the
        // `url\s*:` regex (a quote sits before the colon), so this is a genuinely new hook.
        // Single-segment path proves it (two-segment fallback can't catch it).
        let html = r##"<script>var cfg={"api":"/JschxxApiCfg","endpoint":"/JschxxEndpointCfg"};</script>"##;
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxApiCfg"), "{joined}");
        assert!(joined.contains("/JschxxEndpointCfg"), "{joined}");
    }
