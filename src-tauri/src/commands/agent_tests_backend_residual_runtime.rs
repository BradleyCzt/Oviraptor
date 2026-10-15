// Runtime retirement guards live separately from the static residual inventory.
// Tests that change the process environment run in an isolated child.

    /// PATH trap with a positive control; never modify the parent test process.
    #[cfg(unix)]
    #[test]
    fn retired_cli_path_trap_covers_native_web_and_source_branches() {
        const CHILD_ROOT: &str = "OVIRAPTOR_RETIREMENT_CLI_TEST_ROOT";
        let Some(root) = std::env::var_os(CHILD_ROOT).map(PathBuf::from) else {
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!("oviraptor-cli-path-trap-{}", Uuid::new_v4()));
            let (binary, _) = fake_cli_recording_launches(&root.join("bin"), "strix");
            fake_cli_recording_launches(&root.join("bin"), "oviraptor-path-control");
            let mut paths = vec![binary.parent().unwrap().to_path_buf()];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()));
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "scanning::agent_tests::retired_cli_path_trap_covers_native_web_and_source_branches", "--nocapture", "--test-threads=1"])
                .env(CHILD_ROOT, &root)
                .env("PATH", std::env::join_paths(paths).unwrap())
                .output().unwrap();
            let receipt = fs::read_to_string(root.join("completed.json"));
            fs::remove_dir_all(&root).unwrap();
            assert!(output.status.success(), "PATH trap child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            let receipt: JsonValue = serde_json::from_str(&receipt.expect("child must execute all branches, not merely exit with zero selected tests")).unwrap();
            assert_eq!(receipt, serde_json::json!({"positiveControl":true,"branches":["web","code","greybox","cicd"]}));
            return;
        };
        let sentinel = root.join("bin/strix.launched");
        assert_eq!(executable_on_path("strix"), Some(root.join("bin/strix")));
        assert_eq!(executable_on_path("oviraptor-path-control"), Some(root.join("bin/oviraptor-path-control")));
        let control = Command::new("oviraptor-path-control").arg("--positive-control").output().unwrap();
        assert!(control.status.success());
        assert!(fs::read_to_string(root.join("bin/oviraptor-path-control.launched")).unwrap().contains("--positive-control"));
        assert!(!sentinel.exists(), "the retired CLI must not run even for a positive control");

        let (log, _) = native_web_target_retirement_fixture("path-trap-web");
        assert!(log.contains("native backend selected"), "{log}");
        assert!(!sentinel.exists(), "Web entry executed the retired CLI");

        // Missing optional analyzers leave coverage gaps, not a CLI fallback.
        let repository = root.join("repo");
        fs::create_dir_all(repository.join("src")).unwrap();
        fs::write(repository.join("src/app.py"), "print('fixture')\n").unwrap();
        for scan_type in ["code", "greybox", "cicd"] {
            let app_dir = root.join(scan_type).join("app");
            let db_path = db::initialize(&app_dir).unwrap();
            let connection = db::open(&db_path).unwrap();
            let work = root.join(scan_type).join("work");
            fs::create_dir_all(&work).unwrap();
            source_runtime_publication_fixture(&connection,scan_type,&work,&repository,scan_type,"full","");
            let report = run_native_source_scan(&db_path, &app_dir, scan_type, 1, &work,
                repository.to_str().unwrap(), scan_type, "").unwrap();
            assert_eq!(report["backend"], "native", "{scan_type}: {report}");
            assert!(report["fileCount"].as_u64().unwrap() > 0);
            assert!(!report["gaps"].as_array().unwrap().is_empty());
            let plan = crate::native_pipeline::NativeSourcePlan::load(&connection, scan_type, 1)
                .unwrap().expect("actual source execution must persist its frozen plan");
            assert_eq!(plan.backend, "native");
            assert_eq!(plan.scan_type, scan_type);
            assert!(!sentinel.exists(), "{scan_type} source branch executed the retired CLI");
        }
        fs::write(root.join("completed.json"), serde_json::json!({
            "positiveControl":true,"branches":["web","code","greybox","cicd"]
        }).to_string()).unwrap();
    }

    const EGRESS_KEYS: [&str; 8] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy",
        "https_proxy", "all_proxy", "NO_PROXY", "no_proxy"];

    /// The child owns its environment, so sibling tests cannot observe a
    /// temporary proxy change. Restore on panic inside that child as well.
    struct ProxyCanary(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl ProxyCanary {
        fn route_to(port: u16) -> Self {
            let saved = EGRESS_KEYS.iter().map(|key| (*key, std::env::var_os(key))).collect();
            for key in EGRESS_KEYS[..6].iter() {
                std::env::set_var(key, format!("http://127.0.0.1:{port}"));
            }
            for key in EGRESS_KEYS[6..].iter() {
                std::env::set_var(key, "127.0.0.1,localhost,::1");
            }
            Self(saved)
        }
    }
    impl Drop for ProxyCanary {
        fn drop(&mut self) {
            for (key, value) in self.0.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn spawn_egress_canary() -> (u16, std::sync::Arc<Mutex<Vec<String>>>) {
        let (port, seen, _stop) = spawn_endpoint(std::sync::Arc::new(|_| {
            (502, "text/plain", "blocked by local test canary".into())
        }));
        (port, seen)
    }

    fn blocked_hosts() -> Vec<String> {
        residual_registry()["blockedHosts"].as_array().cloned().unwrap_or_default()
            .into_iter().filter_map(|item| item.as_str().map(str::to_string)).collect()
    }

    fn saw_blocked_host(seen: &[String], hosts: &[String]) -> Vec<String> {
        seen.iter().filter(|line| hosts.iter().any(|host| line.contains(host.as_str())))
            .cloned().collect()
    }

    #[test]
    fn neither_the_native_path_nor_the_legacy_importer_reaches_a_blocked_host() {
        const CHILD_ROOT: &str = "OVIRAPTOR_EGRESS_CANARY_CHILD_ROOT";
        let Some(root) = std::env::var_os(CHILD_ROOT).map(PathBuf::from) else {
            let root = std::env::temp_dir().canonicalize().unwrap()
                .join(format!("oviraptor-egress-canary-{}", Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "scanning::agent_tests::neither_the_native_path_nor_the_legacy_importer_reaches_a_blocked_host", "--nocapture", "--test-threads=1"])
                .env(CHILD_ROOT, &root).output().unwrap();
            let receipt = fs::read_to_string(root.join("completed.json"));
            fs::remove_dir_all(&root).unwrap();
            assert!(output.status.success(), "egress canary child failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert_eq!(receipt.expect("child must execute the egress paths"), "native-and-importer-checked");
            return;
        };
        let hosts = blocked_hosts();
        assert!(hosts.len() >= 3, "基线必须列出被阻断域名，否则本测试没有对象");
        let (port, seen) = spawn_egress_canary();
        let canary = ProxyCanary::route_to(port);
        // Positive control: reqwest's ordinary environment proxy discovery
        // must reach the canary before a zero-egress result is meaningful.
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5)).build().unwrap();
        let attempted = client.get(format!("https://{}/", hosts[0])).send();
        assert!(attempted.is_err(), "被阻断域名不该真的连通：{hosts:?}");
        assert!(!saw_blocked_host(&seen.lock().unwrap(), &hosts).is_empty(),
            "机制自检失败：代理没有记录到 {hosts:?}");
        seen.lock().unwrap().clear();

        let (log, _db) = native_web_target_retirement_fixture("residual-egress");
        assert!(log.contains("native backend selected"), "{log}");
        let source = root.join("source-bundles");
        super::tests::copy_tree_frozen_times(&super::tests::legacy_fixture_root().join("bundles"), &source);
        super::tests::claim_legacy_bundles(&source);
        let synced = super::tests::import_legacy_bundles(&source).expect("历史导入必须成功");
        assert!(synced >= 1, "导入没有认领任何 bundle，本断言就成了空断言");
        let leaked = saw_blocked_host(&seen.lock().unwrap(), &hosts);
        drop(canary);
        assert!(leaked.is_empty(), "产品路径试图访问被阻断域名：{leaked:?}");
        fs::write(root.join("completed.json"), "native-and-importer-checked").unwrap();
    }
