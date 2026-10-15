// Static regression guard, not a desktop IPC integration test.
#[test]
fn implicit_history_import_commands_are_not_registered() {
    let registrations = include_str!("../lib.rs");
    let owner_index = include_str!("../commands.rs");
    let scanning_scope = include_str!("../scanning.rs");
    let modules = concat!(include_str!("../commands.rs"), "\n", include_str!("../scanning.rs"));
    assert!(owner_index.contains("pub(crate) mod assets;"), "owner index source must be live");
    assert!(scanning_scope.contains("include!(\"commands/scan_control.rs\");"), "Scanning source must be live");
    let (prefix, table) = registrations.split_once("tauri::generate_handler![")
        .expect("actual Tauri registration macro must exist");
    assert!(prefix.contains(".invoke_handler("));
    let (table, _) = table.split_once("])").expect("actual registration table must close");
    let registered_paths: Vec<&str> = table.lines()
        .map(|line| line.split("//").next().unwrap_or("").trim().trim_end_matches(',').trim())
        .filter(|line| !line.is_empty())
        .map(|path| {
            assert!(path.contains("::"), "registration input must contain qualified paths");
            path
        })
        .collect();
    let registered: Vec<&str> = registered_paths.iter()
        .map(|path| path.rsplit("::").next().unwrap())
        .collect();
    assert_eq!(registered.len(), 294, "registration source cannot be an empty shell");
    // The local increment adds exactly these five Assets-owned IPCs. Keep the
    // original 289 registrations counted separately, without weakening retired
    // importer, uniqueness, live-command or implicit-root checks below.
    let page_handlers = [
        "commands::page_isolation::preview_page_rule",
        "commands::page_isolation::apply_page_rule",
        "commands::page_isolation::list_page_rules",
        "commands::asset_rule_catalog::get_asset_rule_catalog",
        "commands::page_isolation::disable_page_rule",
    ];
    for handler in page_handlers {
        assert_eq!(registered_paths.iter().copied().filter(|path| *path == handler).count(), 1,
            "new page IPC must occur once under its actual Assets owner: {handler}");
    }
    let retained = registered_paths.iter().copied()
        .filter(|path| !page_handlers.contains(path))
        .count();
    assert_eq!(retained, 289, "the original registration count must remain after excluding the five page IPCs");
    let unique: std::collections::HashSet<&str> = registered.iter().copied().collect();
    assert_eq!(unique.len(), registered.len(), "IPC terminal names must stay unique");
    for live in ["list_projects", "get_app_settings", "list_sentinel_scans"] {
        assert!(registered.contains(&live), "live command must be present: {live}");
    }
    for command in ["sync_sentinel_results", "run_artifact_import"] {
        assert!(
            !registered.contains(&command),
            "retired implicit importer is registered: {command}"
        );
    }
    assert!(!modules.contains("commands/result_ingestion_sync.rs"));
    assert!(!modules.contains("commands/legacy_artifact_compat.rs"));
    let ingestion = include_str!("result_ingestion_runs.rs");
    for retired in [
        "historical_artifact_roots",
        "legacyArtifactDirectories",
        "expand_home_path",
        "strix_run_dirs",
        "LEGACY_RUN_ARTIFACT",
        "legacy_completed_artifact",
    ] {
        assert!(
            !ingestion.contains(retired),
            "implicit root discovery survived: {retired}"
        );
    }
}
