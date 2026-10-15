// REM-012 complements the zero-tolerance active-symbol gate: every remaining
// literal belongs to a reviewed compatibility boundary or test fixture. Full
// file hashes bind the review to context, not just a matching line or count.

fn retirement_file_evidence(relative: &str, raw: &[u8]) -> Option<JsonValue> {
    let path_match = relative.to_ascii_lowercase().contains("strix");
    let occurrences = raw.windows(5).filter(|word| word.eq_ignore_ascii_case(b"strix")).count();
    if occurrences == 0 && !path_match {
        return None;
    }
    // Git may check out text with CRLF on Windows. Normalize only line endings;
    // no whitespace, comments, or surrounding executable code is ignored.
    let normalized = std::str::from_utf8(raw).ok().map(|text| text.replace("\r\n", "\n"));
    let bytes = normalized.as_ref().map(|text| text.as_bytes()).unwrap_or(raw);
    Some(serde_json::json!({
        "sha256":format!("{:x}", Sha256::digest(bytes)),
        "literalOccurrences":occurrences,
        "pathMatch":path_match,
    }))
}

fn retirement_scan_path(
    root: &Path,
    path: &Path,
    found: &mut std::collections::BTreeMap<String, JsonValue>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("retirement scope must not contain symlinks: {}", path.display()));
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
            let child = entry.map_err(|error| error.to_string())?.path();
            retirement_scan_path(root, &child, found)?;
        }
    } else if metadata.is_file() {
        let relative = path.strip_prefix(root).map_err(|error| error.to_string())?
            .to_string_lossy().replace('\\', "/");
        let bytes = fs::read(path).map_err(|error| format!("cannot read {relative}: {error}"))?;
        if let Some(evidence) = retirement_file_evidence(&relative, &bytes) {
            found.insert(relative, evidence);
        }
    } else {
        return Err(format!("unsupported retirement scope entry: {}", path.display()));
    }
    Ok(())
}

const RETIREMENT_SOURCE_SCOPES: &[&str] = &[
    "src", "src-tauri/src", "src-tauri/resources", "src-tauri/capabilities", "src-tauri/icons",
    "tools", "scripts", "public",
];

// Git does not retain empty directories. These two future source roots may be
// absent, but when present they receive the same recursive and symlink checks.
const RETIREMENT_OPTIONAL_SOURCE_SCOPES: &[&str] = &["scripts", "public"];

fn retirement_reviewed_resource_path(value: &str) -> bool {
    // Deliberately a bounded project policy, not a second glob implementation.
    // Only these literal roots are scanned recursively on every platform.
    if value.is_empty() || value.starts_with('/') || value.contains(['\\', ':', '\0']) {
        return false;
    }
    let mut parts = value.split('/').filter(|part| !part.is_empty() && *part != ".");
    matches!(parts.next(), Some("resources" | "icons")) && parts.all(|part| part != "..")
}

fn retirement_review_package_config(path: &Path) -> Result<(), String> {
    let reject = |detail: &str| format!("unreviewed package input in {}: {detail}", path.display());
    let raw = fs::read(path).map_err(|error| reject(&error.to_string()))?;
    let config: JsonValue = serde_json::from_slice(&raw).map_err(|error| reject(&error.to_string()))?;
    if !config.is_object() || config.get("bundle").is_some_and(|bundle| !bundle.is_object()) {
        return Err(reject("configuration and bundle must be objects"));
    }
    for field in ["resources", "icon"] {
        let value = &config["bundle"][field];
        let valid = match value {
            JsonValue::Null => true,
            JsonValue::Array(paths) => paths.iter().all(|value|
                value.as_str().is_some_and(retirement_reviewed_resource_path)),
            JsonValue::Object(paths) if field == "resources" => paths.iter().all(|(source, destination)|
                retirement_reviewed_resource_path(source) && destination.is_string()),
            _ => false,
        };
        if !valid { return Err(reject(&format!("bundle.{field} must use audited resources/ or icons/ sources"))); }
    }
    // New sidecars, installer hooks and platform file-copy rules need explicit
    // scope/review support before use; scanning their config text is insufficient.
    for field in [
        "/bundle/externalBin", "/bundle/licenseFile", "/bundle/macOS/frameworks",
        "/bundle/macOS/files", "/bundle/macOS/entitlements", "/bundle/macOS/infoPlist",
        "/bundle/macOS/dmg/background", "/bundle/windows/wix", "/bundle/windows/nsis",
        "/bundle/linux/appimage/files", "/bundle/linux/deb/files", "/bundle/linux/rpm/files",
        "/bundle/linux/deb/changelog", "/bundle/linux/deb/desktopTemplate",
        "/bundle/linux/deb/preInstallScript", "/bundle/linux/deb/postInstallScript",
        "/bundle/linux/deb/preRemoveScript", "/bundle/linux/deb/postRemoveScript",
        "/bundle/linux/rpm/desktopTemplate", "/bundle/linux/rpm/preInstallScript",
        "/bundle/linux/rpm/postInstallScript", "/bundle/linux/rpm/preRemoveScript",
        "/bundle/linux/rpm/postRemoveScript", "/bundle/iOS/template",
        "/bundle/iOS/frameworks", "/bundle/iOS/infoPlist",
    ] {
        let empty = match config.pointer(field) {
            None | Some(JsonValue::Null) => true,
            Some(JsonValue::Array(values)) => values.is_empty(),
            Some(JsonValue::Object(values)) => values.is_empty(),
            _ => false,
        };
        if !empty { return Err(reject(&format!("{field} requires explicit package-input review"))); }
    }
    if let Some(value) = config.pointer("/app/trayIcon/iconPath") {
        if !value.as_str().is_some_and(retirement_reviewed_resource_path) {
            return Err(reject("tray icon must use an audited source"));
        }
    }
    if let Some(value) = config.pointer("/build/frontendDist") {
        if value.as_str() != Some("../dist") {
            return Err(reject("frontendDist must use the reviewed local build output"));
        }
    }
    Ok(())
}

fn retirement_scan_inputs(root: &Path) -> Result<std::collections::BTreeMap<String, JsonValue>, String> {
    let mut found = std::collections::BTreeMap::new();
    // Fixed in code rather than in the allowlist: removing a manifest entry
    // cannot shrink the scan. No extension filter, ignored files, or glob skips.
    for scope in RETIREMENT_SOURCE_SCOPES {
        let path = root.join(scope);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
                && RETIREMENT_OPTIONAL_SOURCE_SCOPES.contains(scope) => {}
            _ => retirement_scan_path(root, &path, &mut found)?,
        }
    }
    // Include immediate build/config inputs, including future config variants.
    // Generated outputs still need separate artifact acceptance; this inventory
    // covers source inputs, not built bundles, design docs or local logs.
    for directory in [root, &root.join("src-tauri")] {
        // Cargo also reads extensionless config files in these optional trees.
        // Inspect metadata without following links; only absence is skippable.
        let cargo_config = directory.join(".cargo");
        match fs::symlink_metadata(&cargo_config) {
            Ok(_) => retirement_scan_path(root, &cargo_config, &mut found)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot inspect {}: {error}", cargo_config.display())),
        }
        for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
            let path = entry.map_err(|error| error.to_string())?.path();
            let name = path.file_name().unwrap().to_string_lossy();
            let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");
            let build_input = matches!(extension,
                "json" | "json5" | "toml" | "lock" | "ts" | "js" | "cjs" | "mjs" |
                "rs" | "sh" | "py" | "ps1" | "bat" | "cmd" | "html" | "yaml" | "yml" | "plist")
                || matches!(name.as_ref(), "Dockerfile" | "Makefile");
            if path.is_dir() || !build_input {
                continue;
            }
            retirement_scan_path(root, &path, &mut found)?;
        }
    }
    let directory = root.join("src-tauri");
    retirement_review_package_config(&directory.join("tauri.conf.json"))?;
    for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let name = path.file_name().unwrap().to_string_lossy();
        if (name.starts_with("tauri.") && name.ends_with(".conf.json5")) || name == "tauri.conf.json5"
            || (name.starts_with("Tauri.") && name.ends_with(".toml")) {
            return Err(format!("unreviewed package input: {} needs a reviewed parser", path.display()));
        }
        if name != "tauri.conf.json" && name.starts_with("tauri.") && name.ends_with(".conf.json") {
            retirement_review_package_config(&path)?;
        }
    }
    Ok(found)
}

#[test]
fn retirement_literal_allowlist_matches_reviewed_sources_and_packaged_inputs() {
    let root = residual_root();
    let found = retirement_scan_inputs(&root).unwrap();
    let raw = fs::read_to_string(root.join("src-tauri/tests/backend_retirement_allowlist.json")).unwrap();
    let registry: JsonValue = serde_json::from_str(&raw).unwrap();
    assert_eq!(registry["schemaVersion"], 1);
    let entries = registry["entries"].as_object().expect("literal allowlist entries must be an object");
    let mut expected = std::collections::BTreeMap::new();
    for (path, entry) in entries {
        let category = entry["category"].as_str().unwrap_or("");
        assert!(matches!(category, "importer" | "migration" | "fixture" | "historical_label"), "{path}: invalid category");
        assert!(entry["reason"].as_str().is_some_and(|reason| reason.trim().len() >= 20), "{path}: review rationale is required");
        expected.insert(path.clone(), entry["evidence"].clone());
    }
    assert!(!expected.is_empty(), "compatibility boundaries must be explicitly reviewed");
    let added: Vec<_> = found.keys().filter(|path| !expected.contains_key(*path)).collect();
    let stale: Vec<_> = expected.keys().filter(|path| !found.contains_key(*path)).collect();
    let changed: Vec<_> = found.iter().filter(|(path, value)| expected.get(*path).is_some_and(|old| old != *value)).map(|(path, _)| path).collect();
    assert!(added.is_empty() && stale.is_empty() && changed.is_empty(),
        "REM-012 requires source review, not automatic blessing. New={added:?}; stale={stale:?}; changed={changed:?}");
}

#[test]
fn retirement_literal_scan_detects_case_paths_binary_and_context_changes() {
    assert!(retirement_file_evidence("src/neutral.rs", b"native backend").is_none());
    let old = retirement_file_evidence("src/import.rs", b"// STRIX\nread_only();").unwrap();
    assert_eq!(old["literalOccurrences"], 1);
    assert_eq!(old, retirement_file_evidence("src/import.rs", b"// STRIX\r\nread_only();").unwrap());
    let changed = retirement_file_evidence("src/import.rs", b"// STRIX\nexecute();").unwrap();
    assert_eq!(old["literalOccurrences"], changed["literalOccurrences"]);
    assert_ne!(old["sha256"], changed["sha256"], "same matching line cannot exempt changed execution context");
    assert_eq!(retirement_file_evidence("resources/STRIX", b"no text").unwrap()["pathMatch"], true);
    assert_eq!(retirement_file_evidence("resources/backend.bin", b"\xffStRiX\x00").unwrap()["literalOccurrences"], 1);
}

#[test]
fn retirement_literal_scan_checks_nested_unusual_extensions_and_missing_inputs() {
    let root = std::env::temp_dir().join(format!("oviraptor-retirement-literal-{}", Uuid::new_v4()));
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::write(root.join("nested/tool.custom"), b"StRiX").unwrap();
    let mut found = std::collections::BTreeMap::new();
    retirement_scan_path(&root, &root, &mut found).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found.contains_key("nested/tool.custom"));
    assert!(retirement_scan_path(&root, &root.join("missing"), &mut found).is_err());
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(unix)]
#[test]
fn retirement_literal_scan_rejects_symlinks_instead_of_following_or_skipping_them() {
    let root = std::env::temp_dir().join(format!("oviraptor-retirement-symlink-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink("missing", root.join("link")).unwrap();
    assert!(retirement_scan_path(&root, &root, &mut std::collections::BTreeMap::new()).is_err());
    fs::remove_dir_all(&root).unwrap();
}

fn retirement_package_fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("oviraptor-package-scope-{}", Uuid::new_v4()));
    for scope in RETIREMENT_SOURCE_SCOPES { fs::create_dir_all(root.join(scope)).unwrap(); }
    fs::write(root.join("src-tauri/tauri.conf.json"), b"{}").unwrap();
    root
}

#[test]
fn retirement_optional_source_roots_are_absent_or_fully_audited() {
    let root = retirement_package_fixture();
    for scope in RETIREMENT_OPTIONAL_SOURCE_SCOPES {
        fs::remove_dir_all(root.join(scope)).unwrap();
    }
    assert!(retirement_scan_inputs(&root).unwrap().is_empty());
    for scope in RETIREMENT_OPTIONAL_SOURCE_SCOPES {
        let relative = format!("{scope}/nested/input.custom");
        let path = root.join(&relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"StRiX").unwrap();
        let found = retirement_scan_inputs(&root).unwrap();
        assert_eq!(found[&relative]["literalOccurrences"], 1);
    }
    for required in ["src", "tools"] {
        let path = root.join(required);
        fs::remove_dir_all(&path).unwrap();
        let error = retirement_scan_inputs(&root).unwrap_err();
        assert!(error.contains(&path.display().to_string()), "{error}");
        fs::create_dir(path).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn retirement_optional_source_roots_still_reject_root_and_nested_symlinks() {
    let root = retirement_package_fixture();
    for scope in RETIREMENT_OPTIONAL_SOURCE_SCOPES {
        let path = root.join(scope);
        fs::remove_dir_all(&path).unwrap();
        std::os::unix::fs::symlink("missing", &path).unwrap();
        assert!(retirement_scan_inputs(&root).unwrap_err().contains("symlinks"));
        fs::remove_file(&path).unwrap();
        let nested = path.join("nested/link.custom");
        fs::create_dir_all(nested.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("missing", &nested).unwrap();
        assert!(retirement_scan_inputs(&root).unwrap_err().contains("symlinks"));
        fs::remove_file(nested).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn retirement_package_scope_checks_optional_project_cargo_configuration_trees() {
    let root = retirement_package_fixture();
    let mut expected = Vec::new();
    for directory in [".cargo", "src-tauri/.cargo"] {
        fs::create_dir_all(root.join(directory).join("nested")).unwrap();
        for name in ["config", "config.toml", "nested/input.custom"] {
            let relative = format!("{directory}/{name}");
            fs::write(root.join(&relative), b"StRiX").unwrap();
            expected.push(relative);
        }
    }
    let result = retirement_scan_inputs(&root);
    fs::remove_dir_all(root).unwrap();
    let found = result.expect("local build configuration must be inventoried without execution");
    assert_eq!(found.len(), expected.len());
    for relative in expected {
        assert_eq!(found[&relative]["literalOccurrences"], 1);
    }
}

#[test]
#[cfg(unix)]
fn retirement_package_scope_rejects_optional_cargo_symlinks() {
    let root = retirement_package_fixture();
    let mut missed = Vec::new();
    for relative in [".cargo", "src-tauri/.cargo"] {
        let path = root.join(relative);
        std::os::unix::fs::symlink("missing", &path).unwrap();
        if !retirement_scan_inputs(&root).is_err_and(|error| error.contains("symlinks")) {
            missed.push(relative);
        }
        fs::remove_file(path).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
    assert!(missed.is_empty(), "optional build configuration escaped inspection: {missed:?}");
}

#[test]
fn retirement_package_scope_rejects_unreviewed_sources_and_sidecars() {
    let cases = [
        json!({"bundle":{"resources":["../unreviewed/tool.bin"]}}),
        json!({"bundle":{"resources":{"../unreviewed/tool.bin":"resources/tool.bin"}}}),
        json!({"bundle":{"resources":["resources/../../unreviewed/*"]}}),
        json!({"bundle":{"resources":["/tmp/tool.bin"]}}),
        json!({"bundle":{"resources":["C:\\tool.bin"]}}),
        json!({"bundle":{"resources":["resources\\..\\tool.bin"]}}),
        json!({"bundle":{"resources":["resource*/tool.bin"]}}),
        json!({"bundle":{"resources":["resources-old/tool.bin"]}}),
        json!({"bundle":{"resources":[""]}}),
        json!({"bundle":{"resources":[false]}}),
        json!({"bundle":{"resources":{"resources/tool.bin":false}}}),
        json!({"bundle":{"resources":true}}),
        json!({"bundle":{"icon":["../unreviewed/logo.png"]}}),
        json!({"bundle":{"externalBin":["resources/tool"]}}),
        json!({"bundle":{"macOS":{"files":{"/Applications/tool":"../unreviewed/tool"}}}}),
        json!({"bundle":{"macOS":{"frameworks":["../unreviewed/library"]}}}),
        json!({"bundle":{"windows":{"nsis":{"installerHooks":"../unreviewed/hook.nsh"}}}}),
        json!({"bundle":{"windows":{"wix":{"fragmentPaths":["../unreviewed/fragment.wxs"]}}}}),
        json!({"bundle":{"linux":{"deb":{"files":{"/usr/bin/tool":"../unreviewed/tool"}}}}}),
        json!({"bundle":{"linux":{"rpm":{"preInstallScript":"../unreviewed/install.sh"}}}}),
        json!({"app":{"trayIcon":{"iconPath":"../unreviewed/tray.png"}}}),
        json!({"build":{"frontendDist":"../unreviewed-ui"}}),
        json!({"bundle":false}),
        json!([]),
    ];
    let root = retirement_package_fixture();
    let mut missed = Vec::new();
    for config in cases {
        fs::write(root.join("src-tauri/tauri.conf.json"), config.to_string()).unwrap();
        if !retirement_scan_inputs(&root).is_err_and(|error| error.contains("unreviewed package input")) {
            missed.push(config);
        }
    }
    fs::remove_dir_all(root).unwrap();
    assert!(missed.is_empty(), "package inputs escaped review: {missed:?}");
}

#[test]
fn retirement_package_scope_checks_platform_overrides_and_unsupported_formats() {
    let root = retirement_package_fixture();
    let mut missed = Vec::new();
    for name in ["tauri.linux.conf.json", "tauri.windows.conf.json", "tauri.macos.conf.json",
        "tauri.android.conf.json", "tauri.ios.conf.json", "tauri.conf.json5", "Tauri.toml",
        "tauri.windows.conf.json5", "Tauri.windows.toml"] {
        let path = root.join("src-tauri").join(name);
        fs::write(&path, br#"{"bundle":{"resources":["../unreviewed/tool.bin"]}}"#).unwrap();
        if !retirement_scan_inputs(&root).is_err_and(|error| error.contains("unreviewed package input")) {
            missed.push(name);
        }
        fs::remove_file(path).unwrap();
    }
    fs::remove_dir_all(root).unwrap();
    assert!(missed.is_empty(), "unreviewed configuration variants: {missed:?}");
}

#[test]
fn retirement_package_scope_keeps_reviewed_lists_maps_globs_and_platform_options() {
    let root = retirement_package_fixture();
    fs::write(root.join("src-tauri/tauri.conf.json"), json!({
        "build":{"frontendDist":"../dist"},
        "app":{"trayIcon":{"iconPath":"icons/tray.png"}},
        "bundle":{"resources":["resources", "./resources/workers/*.cjs", "resources/**/*.json"],
            "icon":["icons/icon.png"], "externalBin":[], "macOS":{"signingIdentity":"-"}},
    }).to_string()).unwrap();
    fs::write(root.join("src-tauri/tauri.windows.conf.json"), json!({"bundle":{
        "resources":{"resources/workers/*.cjs":"workers/", "./resources/config":""},
        "windows":{"allowDowngrades":false},
    }}).to_string()).unwrap();
    fs::write(root.join("src-tauri/resources/tool.custom"), b"StRiX").unwrap();
    // Default platform metadata can be consumed without an explicit config path.
    fs::write(root.join("src-tauri/Info.plist"), b"StRiX").unwrap();
    let result = retirement_scan_inputs(&root);
    fs::remove_dir_all(root).unwrap();
    let found = result.expect("reviewed package sources must remain usable");
    assert_eq!(found.len(), 2);
    assert_eq!(found["src-tauri/resources/tool.custom"]["literalOccurrences"], 1);
    assert_eq!(found["src-tauri/Info.plist"]["literalOccurrences"], 1);
}
