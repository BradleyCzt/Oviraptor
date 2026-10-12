
    #[test]
    fn agent_harvest_surface_paths_finds_track_src() {
        // U148: <track src="/..."> text tracks. Single-segment path is NOT caught by
        // the two-segment /Controller/Action fallback, so this proves the new pattern.
        let html = r##"<video><track kind="subtitles" src="/JschxxCaptions"></video>"##;
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxCaptions"), "{joined}");
    }
