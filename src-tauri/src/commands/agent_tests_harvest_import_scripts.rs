
    #[test]
    fn agent_harvest_surface_paths_finds_import_scripts() {
        // U158: Worker `importScripts("/...")`. The existing `\bimport(` pattern needs
        // "import(" immediately, but here it's "importScripts(", so this is distinct.
        // Single-segment path proves the new pattern.
        let html = r##"<script>importScripts("/JschxxWorkerLib");</script>"##;
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxWorkerLib"), "{joined}");
    }
