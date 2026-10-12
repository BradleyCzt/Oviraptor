
    #[test]
    fn agent_harvest_surface_paths_finds_new_request() {
        // U154: fetch(new Request("/...")) wrapper. Plain `fetch(` needs a quote right
        // after the paren, so the Request() wrap is missed without this pattern.
        // Single-segment path proves the new regex (two-segment fallback can't catch it).
        let html = r##"<script>fetch(new Request("/JschxxNewReq", {method:"POST"}));</script>"##;
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxNewReq"), "{joined}");
    }
