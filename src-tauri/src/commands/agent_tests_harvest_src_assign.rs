
    #[test]
    fn agent_harvest_surface_paths_finds_src_property_assignment() {
        // U156: JS `.src = "/..."` dynamic assignment (new Image().src, script.src, ...).
        // Attribute `src=` in tags has a space before src, not a dot, so this is distinct.
        // Single-segment, extension-less path proves the new pattern.
        let html = r##"<script>var i=new Image(); i.src = "/JschxxDynSrc";</script>"##;
        let paths = agent_harvest_surface_paths(html, "https://www.jschxx.com/");
        let joined = paths
            .iter()
            .map(|(m, p)| format!("{m}|{p}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(joined.contains("/JschxxDynSrc"), "{joined}");
    }
