
    #[test]
    fn agent_harvest_surface_paths_finds_css_import_string() {
        // U157: bare CSS `@import "/..."` (string form). The url() form is already
        // harvested, but the bare-string @import was not. Single-segment path proves it.
        let html = r##"<style>@import "/JschxxCssImport";</style>"##;
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxCssImport"), "{joined}");
    }
