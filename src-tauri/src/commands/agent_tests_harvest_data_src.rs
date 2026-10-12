
    #[test]
    fn agent_harvest_surface_paths_finds_data_src() {
        // U150: data-src lazy-load hook. It is NOT in the data-* nav alternation
        // (href|url|path|link|action|route|api|endpoint|target); single-segment path
        // also escapes the two-segment fallback, so this proves the new pattern.
        let html = r##"<img data-src="/JschxxLazyThumb" class="lazy">"##;
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxLazyThumb"), "{joined}");
    }
