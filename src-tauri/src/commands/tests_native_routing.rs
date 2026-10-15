// §12 Stage 4 出口门禁 — the four task types are owned by the native runtime: a frozen
// plan per attempt, a source pipeline that completes with gaps instead of handing the work
// to another engine, and a tool surface that is only offered when a snapshot exists.

#[test]
fn native_source_requires_a_pinned_container_before_locating_any_runtime() {
    for settings in [json!({}), json!({"nativeAnalyzers":{"semgrep":{"image":"fixture:latest"}}})] {
        let result = configured_native_analyzer_runtime(&settings, "semgrep", |_| {
            panic!("missing/invalid image must not locate a host analyzer or Docker");
        });
        assert!(result.unwrap_err().starts_with("analyzer_missing:semgrep:"));
    }
    let image = format!("fixture/analyzer@sha256:{}", "1".repeat(64));
    let settings = json!({"nativeAnalyzers":{"semgrep":{"image":image}}});
    let (program, selected_image) = configured_native_analyzer_runtime(&settings, "semgrep", |name| {
        assert_eq!(name, "docker", "never resolve the host analyzer");
        Some(PathBuf::from("fixture-container-runner"))
    }).unwrap();
    assert_eq!(program, PathBuf::from("fixture-container-runner"));
    assert_eq!(selected_image, Some(image));
    assert!(configured_native_analyzer_runtime(&settings, "semgrep", |_| None)
        .unwrap_err().contains("container_runtime_unavailable"));
}

#[test]
fn native_entry_dependencies_never_probe_legacy_tools_for_any_scan_type() {
    for (scan_type, source, urls) in [
        ("web", "", vec!["http://127.0.0.1:1".to_string()]),
        ("code", "/fixture/repo", vec![]),
        ("greybox", "/fixture/repo", vec!["http://127.0.0.1:1".to_string()]),
        ("cicd", "/fixture/repo", vec![]),
    ] {
        assert!(agent_native_eligible(scan_type, source, &urls));
        let mut plan = ScanBackendPlan {
            scan_id: scan_type.into(),
            attempt_number: 1,
            targets: urls.iter().map(|url| ScanTargetBackend {
                url: url.clone(), backend: AgentBackendKind::Native,
                selection_reason: "native fixture".into(),
            }).collect(),
            requires_node: false, requires_browser: false,
        };
        plan.refresh_requirements(!urls.is_empty());
        prepare_scan_dependencies(&plan).unwrap();
    }
}

#[test]
fn legacy_dependency_flags_are_ignored_for_a_native_historical_plan() {
    let plan = ScanBackendPlan::from_json(&json!({
        "schemaVersion": 1, "scanId": "historical", "attemptNumber": 1,
        "targets": [{"url": "http://127.0.0.1:1", "backend": "native"}],
        "requiresStrix": true, "requiresDocker": true,
    })).unwrap();
    prepare_scan_dependencies(&plan).unwrap();
    assert!(plan.as_json().get("requiresStrix").is_none());
    assert!(plan.as_json().get("requiresDocker").is_none());
}

#[test]
fn retired_backend_is_rejected_without_any_dependency_probe() {
    let plan = ScanBackendPlan::from_json(&json!({
        "schemaVersion": 1, "scanId": "mixed", "attemptNumber": 1,
        "targets": [
            {"url": "http://127.0.0.1:1", "backend": "native"},
            {"url": "http://127.0.0.1:2", "backend": "strix"}
        ], "requiresStrix": true, "requiresDocker": true,
    }));
    assert!(plan.is_none(), "unsupported backend rejects the whole matrix before dependency selection");
    let plan = ScanBackendPlan::from_json(&json!({
        "schemaVersion": 1, "scanId": "sealed", "attemptNumber": 1,
        "targets": [{"url": "https://app.example.invalid", "backend": "legacy_backend_removed"}]
    })).unwrap();
    assert!(prepare_scan_dependencies(&plan).unwrap_err().starts_with("backend_retired:"));
}

#[test]
fn stage_4_writes_a_native_frozen_plan_for_a_source_scan_and_completes_with_gaps() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("oviraptor-native-route-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let repository = root.join("repo");
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::write(
        repository.join("src/app.py"),
        "def load(cur, pid):\n    cur.execute('SELECT * FROM t WHERE id=' + pid)\n",
    )
    .unwrap();
    fs::write(repository.join("requirements.txt"), "flask==2.0\n").unwrap();
    let db_path = db::initialize(&root.join("app")).unwrap();
    let connection = db::open(&db_path).unwrap();
    let work_dir = root.join("work");
    fs::create_dir_all(&work_dir).unwrap();

    source_runtime_publication_fixture(&connection,"route-code",&work_dir,&repository,"code","full","");
    let finding_snapshot = || {
        let mut statement=connection.prepare("SELECT * FROM sentinel_findings ORDER BY id").unwrap();
        let columns=statement.column_count();
        statement.query_map([],|row|(0..columns).map(|index|row.get::<_,rusqlite::types::Value>(index))
            .collect::<Result<Vec<_>,_>>()).unwrap().collect::<Result<Vec<_>,_>>().unwrap()
    };
    let published_findings=finding_snapshot();
    assert_eq!(published_findings.len(),1,"publication records an informational source inventory");
    assert_eq!(connection.query_row("SELECT count(*) FROM sentinel_findings WHERE stage='local-inventory' AND kind='source_inventory' AND severity='info'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    let report = run_native_source_scan(
        &db_path,
        &root.join("app"),
        "route-code",
        1,
        &work_dir,
        &repository.to_string_lossy(),
        "code",
        "",
    )
    .unwrap();
    assert_eq!(report["backend"], json!("native"));
    assert_eq!(report["fileCount"], json!(2));
    assert_eq!(report["languages"].as_array().unwrap().len(), 1);
    // Nothing is installed and no rule pack is ready here, so the honest answer is a
    // coverage gap — never a silent pass.
    let gaps = report["gaps"].as_array().unwrap();
    assert!(!gaps.is_empty(), "缺分析器必须留下缺口：{report}");
    assert!(
        gaps.iter().any(|gap| {
            let text = gap.as_str().unwrap_or_default();
            text.starts_with("analyzer_missing") || text.starts_with("rule_pack_missing")
        }),
        "缺口要说明是哪一项能力缺失：{gaps:?}"
    );
    assert!(
        report["analyzers"].as_array().unwrap().len()
            >= report["analyzerProvenance"].as_array().unwrap().len(),
        "只有真正跑出结果的分析器才带得上 provenance：{report}"
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM analyzer_runs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
        "没有可用分析器时不得留下执行记录"
    );

    // The freeze itself is durable: plan and snapshot are both readable back.
    let plan = crate::native_pipeline::NativeSourcePlan::load(&connection, "route-code", 1)
        .unwrap()
        .expect("源码扫描必须留下 Native 冻结计划");
    assert_eq!(plan.backend, "native");
    assert_eq!(plan.scan_type, "code");
    assert_eq!(plan.tree_hash, report["treeHash"].as_str().unwrap());
    assert_eq!(plan.tools, crate::native_pipeline::tools::SOURCE_TOOLS.to_vec());
    let restored = crate::native_pipeline::snapshot::RepositorySnapshot::restore(
        &connection,
        "route-code",
        1,
    )
    .unwrap()
    .expect("快照清单必须能读回来");
    assert_eq!(restored.tree_hash, plan.tree_hash);
    restored.verify_unchanged().unwrap();

    // And the whole run never needed a sandboxed engine: the source tree is untouched
    // and no analyzer result was written straight into the live views. The
    // publication's informational inventory remains byte-for-byte unchanged.
    assert_eq!(finding_snapshot(),published_findings);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stage_4_ci_scans_carry_the_gate_and_the_freeze_into_the_scan_record() {
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!("oviraptor-native-ci-route-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let repository = root.join("repo");
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::write(repository.join("src/main.go"), "package main\n").unwrap();
    fs::write(repository.join("go.mod"), "module example.invalid/x\n\ngo 1.21\n").unwrap();
    let db_path = db::initialize(&root.join("app")).unwrap();
    let connection = db::open(&db_path).unwrap();
    let work_dir = root.join("work");
    fs::create_dir_all(&work_dir).unwrap();

    source_runtime_publication_fixture(&connection,"route-ci",&work_dir,&repository,"cicd","diff","main");

    let report = run_native_source_scan(
        &db_path,
        &root.join("app"),
        "route-ci",
        1,
        &work_dir,
        &repository.to_string_lossy(),
        "cicd",
        "main",
    )
    .unwrap();
    let gate = &report["gate"];
    assert!(gate.is_object(), "CI 任务必须带上门禁结论：{report}");
    let exit_code = gate["exitCode"].as_i64().unwrap();
    assert!(
        matches!(exit_code, 0 | 2 | 3 | 4),
        "退出码只能是 0/2/3/4：{exit_code}"
    );
    assert_eq!(
        exit_code, 3,
        "覆盖不完整或范围不可信都不能算通过：{gate}"
    );
    let (status, reason): (String, String) = connection
        .query_row(
            "SELECT gate_status,gate_reason FROM sentinel_scan_contexts WHERE scan_id='route-ci'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    // This fixture has no git repository, so the requested diff base is unusable: the
    // gate has to say "not known" rather than "clean".
    assert_eq!(status, "inconclusive");
    assert!(reason.contains("diff base"), "{reason}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stage_4_source_tools_are_offered_only_to_a_run_with_a_frozen_snapshot() {
    let root = std::env::temp_dir().join(format!("oviraptor-native-tools-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let db_path = db::initialize(&root.join("app")).unwrap();
    let context = crate::scanning::agent_tests::test_context(
        &db_path,
        "https://app.example.invalid",
        Vec::new(),
    );
    assert!(
        !agent_run_has_frozen_source(&context),
        "没有快照的运行不能拿到源码工具"
    );
    let advertised = agent_tool_specs_for(&context);
    assert!(
        !advertised
            .iter()
            .any(|spec| agent_is_source_tool(spec.name)),
        "URL 运行看到的工具集不能因为 Stage 4 变大"
    );
    assert_eq!(advertised.len(), 7);

    // A historical snapshot alone is not an execution grant. This context has
    // neither a published scope/view nor a real run identity.
    let repository = root.join("repo");
    fs::create_dir_all(repository.join("src")).unwrap();
    fs::write(repository.join("src/a.py"), "x = 1\n").unwrap();
    let connection = db::open(&db_path).unwrap();
    let snapshot = crate::native_pipeline::snapshot::RepositorySnapshot::capture(
        &repository,
        &root.join("scratch"),
        None,
    )
    .unwrap();
    snapshot
        .store(&connection, &context.scan_id, context.attempt_number)
        .unwrap();
    drop(connection);
    assert!(!agent_run_has_frozen_source(&context));
    let context = crate::scanning::agent_tests::test_context(
        &db_path,
        "https://app.example.invalid",
        Vec::new(),
    );
    let advertised = agent_tool_specs_for(&context);
    assert_eq!(advertised.len(), 7, "未发布范围与身份的历史快照不能扩权");
    assert!(!advertised
        .iter()
        .any(|spec| spec.name == "repo.read_slice"));

    // A call without a snapshot is refused with a stable code, not by falling back.
    let connection = db::open(&db_path).unwrap();
    connection
        .execute(
            "DELETE FROM source_snapshots WHERE scan_id=?1",
            [context.scan_id.clone()],
        )
        .unwrap();
    drop(connection);
    let answer = agent_execute_source_tool(
        &context,
        "repo.inventory",
        &serde_json::json!({}),
    );
    assert_eq!(answer["code"], json!("source_snapshot_unavailable"));
    let _ = fs::remove_dir_all(root);
}
