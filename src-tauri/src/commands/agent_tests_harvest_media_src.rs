
    #[test]
    fn agent_harvest_surface_paths_finds_media_src() {
        // U149: <video|audio|source src="/..."> media/stream routes. Single-segment
        // paths prove the new pattern (two-segment fallback cannot catch them).
        let html = r##"
            <video src="/JschxxStream"></video>
            <audio src="/JschxxPodcast"></audio>
            <video><source src="/JschxxHls"></video>
        "##;
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxStream"), "{joined}");
        assert!(joined.contains("/JschxxPodcast"), "{joined}");
        assert!(joined.contains("/JschxxHls"), "{joined}");
    }
