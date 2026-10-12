
    #[test]
    fn agent_harvest_surface_paths_finds_fetch_template_literal() {
        // U165: fetch(`/...`) with a backtick template literal. The existing
        // `fetch(` pattern only accepts ' or " right after the paren, and the
        // two-segment /Controller/Action fallback (which does allow backticks)
        // cannot catch a single-segment path. So this was missed before.
        let html = "<script>fetch(`/JschxxFetchTick`);</script>";
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxFetchTick"), "fetch backtick missing: {joined}");
    }
