// Anti-Strix guards. Detection patterns live in the historical fixture registry;
// every live-code baseline must remain empty. PATH traps below may name the retired
// executable, but must never launch it, even as a test positive control.

    fn residual_registry() -> JsonValue {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/legacy_strix/residual_baseline.json");
        let raw = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("缺少残留基线 {}: {error}", path.display()));
        serde_json::from_str(&raw).expect("残留基线 JSON 无效")
    }

    fn residual_walk(path: &Path, files: &mut Vec<PathBuf>) {
        let metadata = fs::symlink_metadata(path)
            .unwrap_or_else(|error| panic!("cannot inspect {}: {error}", path.display()));
        assert!(!metadata.file_type().is_symlink(), "residual scope cannot contain symlinks: {}", path.display());
        if metadata.is_file() {
            let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
            let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");
            // Match runtime scripts and build inputs as well as Rust/frontend
            // code. The separate literal/hash audit also inspects binary and
            // unknown-extension inputs; this guard enforces zero active symbols.
            if matches!(extension, "rs" | "ts" | "vue" | "cjs" | "css" | "json" | "toml"
                | "mjs" | "js" | "mts" | "cts" | "tsx" | "jsx" | "py" | "sh" | "bash"
                | "zsh" | "ps1" | "cmd" | "bat" | "yml" | "yaml" | "html" | "json5"
                | "md" | "txt")
                || matches!(name, "Dockerfile" | "Makefile" | "justfile")
                || name.starts_with("Dockerfile.") {
                files.push(path.to_path_buf());
            }
            return;
        }
        assert!(metadata.is_dir(), "unsupported residual scope entry: {}", path.display());
        let entries = fs::read_dir(path)
            .unwrap_or_else(|error| panic!("cannot scan {}: {error}", path.display()));
        for entry in entries {
            let entry = entry.expect("cannot read residual scan directory entry");
            let path = entry.path();
            if path.is_dir() {
                if matches!(
                    path.file_name().and_then(|value| value.to_str()),
                    Some("target" | "node_modules" | "dist" | ".git")
                ) {
                    continue;
                }
                residual_walk(&path, files);
                continue;
            }
            residual_walk(&path, files);
        }
    }

    /// `(relative path -> matching line count)` for one rule.
    fn residual_regexes(rule: &JsonValue) -> Vec<regex::Regex> {
        rule.get("regexPatterns")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .map(|pattern| regex::Regex::new(pattern.as_str().expect("regex must be a string"))
                .expect("invalid residual regex"))
            .collect()
    }

    fn residual_matches(root: &Path, scope: &str, patterns: &[String], regexes: &[regex::Regex]) -> std::collections::BTreeMap<String, usize> {
        let mut files = Vec::new();
        residual_walk(&root.join(scope), &mut files);
        files.sort();
        let mut found = std::collections::BTreeMap::new();
        for path in files {
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot inspect {}: {error}", path.display()));
            let mut matching_lines = text
                .lines()
                .enumerate()
                .filter_map(|(index, line)| patterns.iter().any(|pattern| line.contains(pattern.as_str())).then_some(index))
                .collect::<std::collections::BTreeSet<_>>();
            // Match the whole file so whitespace/newlines cannot hide a launch.
            // Count each starting line once even if several rules match it.
            for expression in regexes {
                for matched in expression.find_iter(&text) {
                    matching_lines.insert(text[..matched.start()].bytes().filter(|byte| *byte == b'\n').count());
                }
            }
            let count = matching_lines.len();
            if count > 0 {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(path.as_path())
                    .to_string_lossy()
                    .replace('\\', "/");
                found.insert(relative, count);
            }
        }
        found
    }

    fn residual_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    #[test]
    fn strix_residual_baseline_matches_the_source_exactly() {
        let registry = residual_registry();
        let scope = registry
            .get("scope")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(!scope.is_empty(), "残留基线必须声明扫描范围");
        let rules = registry
            .get("rules")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            rules.len() >= 4,
            "至少要有进程启动、CLI 探测/安装、沙箱镜像、后端选择四类规则"
        );
        let root = residual_root();
        for rule in &rules {
            let id = value_first(rule, &["id"]);
            let patterns = rule
                .get("patterns")
                .and_then(JsonValue::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_str)
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            assert!(!patterns.is_empty(), "{id} 必须给出匹配模式");
            let mut found = std::collections::BTreeMap::new();
            for entry in &scope {
                let Some(scope) = entry.as_str() else {
                    continue;
                };
                for (path, count) in residual_matches(&root, scope, &patterns, &residual_regexes(rule)) {
                    *found.entry(path).or_insert(0) += count;
                }
            }
            let baseline = rule
                .get("baseline")
                .and_then(JsonValue::as_object)
                .cloned()
                .expect("residual rule must have an object baseline");
            assert!(baseline.is_empty(), "{id}：活动残留基线已归零，不能重新登记豁免");
            let expected: std::collections::BTreeMap<String, usize> = baseline
                .iter()
                .filter_map(|(path, value)| Some((path.clone(), value.as_u64()? as usize)))
                .collect();
            let added = found
                .iter()
                .filter(|(path, count)| expected.get(*path).copied().unwrap_or(0) < **count)
                .map(|(path, count)| format!("{path}={count}"))
                .collect::<Vec<_>>();
            let stale = expected
                .iter()
                .filter(|(path, count)| found.get(*path).copied().unwrap_or(0) < **count)
                .map(|(path, count)| format!("{path}={count}"))
                .collect::<Vec<_>>();
            assert!(
                added.is_empty(),
                "{id}：出现新的残留位置 {added:?}；Strix 只允许减少，不允许新增"
            );
            assert!(
                stale.is_empty(),
                "{id}：基线仍登记着已经不存在的残留 {stale:?}；删除活路径时必须同步删除条目"
            );
        }
    }

    // The final baseline must be allowed to reach zero. Validate the detector
    // with positive controls instead of keeping a real residual alive forever.
    #[test]
    fn residual_scanner_detects_every_registered_pattern_in_a_nested_fixture() {
        let registry = residual_registry();
        let root = std::env::temp_dir().join(format!("oviraptor-residual-control-{}", Uuid::new_v4()));
        let nested = root.join("src/nested");
        fs::create_dir_all(&nested).unwrap();
        let fixture = nested.join("positive.rs");
        for rule in registry["rules"].as_array().unwrap() {
            for pattern in rule["patterns"].as_array().unwrap() {
                let pattern = pattern.as_str().unwrap().to_string();
                fs::write(&fixture, format!("// {pattern}\n// unrelated\n")).unwrap();
                let found = residual_matches(&root, "src", std::slice::from_ref(&pattern), &[]);
                assert_eq!(found.len(), 1, "positive control missed {pattern}");
                assert_eq!(found.get("src/nested/positive.rs"), Some(&1));
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn residual_scanner_covers_script_and_build_file_formats() {
        let registry = residual_registry();
        let pattern = registry["rules"][0]["patterns"][0].as_str().unwrap().to_string();
        let root = std::env::temp_dir().join(format!("oviraptor-residual-formats-{}", Uuid::new_v4()));
        let nested = root.join("src/nested");
        fs::create_dir_all(&nested).unwrap();
        let names = ["entry.mjs", "entry.js", "entry.cjs", "entry.ts", "entry.mts",
            "entry.cts", "entry.tsx", "entry.jsx", "entry.py", "entry.sh", "entry.bash",
            "entry.zsh", "entry.ps1", "entry.cmd", "entry.bat", "entry.yml", "entry.yaml",
            "entry.json", "entry.toml", "entry.rs", "entry.vue", "entry.html", "entry.css",
            "Dockerfile", "Dockerfile.worker", "Makefile", "justfile"];
        for name in names {
            fs::write(nested.join(name), &pattern).unwrap();
        }
        let found = residual_matches(&root, "src", std::slice::from_ref(&pattern), &[]);
        // Remove temporary data even when an assertion fails.
        fs::remove_dir_all(&root).unwrap();
        for name in names {
            assert_eq!(found.get(&format!("src/nested/{name}")), Some(&1), "unscanned runtime format: {name}");
        }
    }

    #[test]
    fn residual_scanner_covers_explicit_build_entry_files() {
        let registry = residual_registry();
        let pattern = registry["rules"][0]["patterns"][0].as_str().unwrap().to_string();
        let root = std::env::temp_dir().join(format!("oviraptor-residual-entry-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("package.json"), &pattern).unwrap();
        let found = residual_matches(&root, "package.json", std::slice::from_ref(&pattern), &[]);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(found.get("package.json"), Some(&1));
    }

    #[test]
    fn residual_registry_covers_production_resources_and_build_entries() {
        let registry = residual_registry();
        let scope = registry["scope"].as_array().unwrap();
        for required in ["src-tauri/src", "src-tauri/resources", "src", "tools", "scripts",
            "package.json", "vite.config.ts", "src-tauri/build.rs", "src-tauri/Cargo.toml",
            "src-tauri/tauri.conf.json"] {
            assert!(scope.iter().any(|entry| entry.as_str() == Some(required)), "missing runtime scope: {required}");
        }
    }

    #[test]
    fn residual_scanner_detects_launch_syntax_without_executing_it() {
        let registry = residual_registry();
        let root = std::env::temp_dir().join(format!("oviraptor-residual-syntax-{}", Uuid::new_v4()));
        let nested = root.join("src/nested");
        fs::create_dir_all(&nested).unwrap();
        let fixture = nested.join("syntax.rs");
        let cases: JsonValue = serde_json::from_str(&fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy_strix/residual_syntax.json")
        ).unwrap()).unwrap();
        for rule in registry["rules"].as_array().unwrap() {
            let regexes = residual_regexes(rule);
            if regexes.is_empty() { continue; }
            let patterns = rule["patterns"].as_array().unwrap().iter()
                .map(|pattern| pattern.as_str().unwrap().to_string()).collect::<Vec<_>>();
            let cases = cases[rule["id"].as_str().unwrap()].as_array().expect("each regex rule needs independent syntax fixtures");
            assert!(!cases.is_empty());
            for case in cases {
                fs::write(&fixture, case["source"].as_str().unwrap()).unwrap();
                let found = residual_matches(&root, "src", &patterns, &regexes);
                assert_eq!(found.get("src/nested/syntax.rs").copied().unwrap_or(0),
                    case["matchingLines"].as_u64().unwrap() as usize, "case: {case}");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// Records any launch, including version/capability probes. Its output need
    /// not satisfy a parser: the sentinel is written before output is returned.
    #[cfg(unix)]
    fn fake_cli_recording_launches(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
        fs::create_dir_all(dir).unwrap();
        let binary = dir.join(name);
        let sentinel = dir.join(format!("{name}.launched"));
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nprintf 'fake 1.6.2\\n'\nexit 0\n",
            sentinel.display()
        );
        fs::write(&binary, script).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        }
        (binary, sentinel)
    }

    /// Runs the actual per-target Native entry against local model/site fixtures.
    /// The task comes from the real creator/startup/HMAC and a frozen multi Web
    /// Root (`multi_terminal_owned_dispatch`), as the production entry requires;
    /// that real path is the one which selects the Native backend.
    fn native_web_target_retirement_fixture(_tag: &str) -> (String, PathBuf) {
        let (harness, owned, stop) = multi_terminal_owned_dispatch();
        let outcome = &owned.outcome;
        assert!(
            !matches!(outcome, AgentTargetOutcome::Failed(_) if outcome.detail().contains("Strix")),
            "native 路径不得出现 Strix 失败：{}",
            outcome.detail()
        );
        // The real dispatch pins agentBackendPolicy=native; the original fixture ran
        // with default settings, so the default must also select Native here.
        assert_eq!(
            agent_select_backend(
                &harness.db_path,
                &harness.context.scan_id,
                1,
                &harness.context.target_url,
                &serde_json::json!({}),
                true
            ),
            AgentBackendKind::Native
        );
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        let log = fs::read_to_string(&harness.context.log_path).unwrap_or_default();
        (log, harness.db_path.clone())
    }

    #[test]
    fn the_native_web_target_registers_no_strix_run() {
        let (log, db_path) = native_web_target_retirement_fixture("residual-native");
        assert!(log.contains("native backend selected"), "{log}");
        let strix_runs: i64 = db::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM agent_runs WHERE scan_id='agent-scan' AND backend='strix'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(strix_runs, 0, "REM-002：不允许登记 Strix run");
        let native_runs: i64 = db::open(&db_path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM agent_runs WHERE scan_id='agent-scan' AND backend='native'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(native_runs >= 1, "the zero count must observe this task's actual native runs");
    }
