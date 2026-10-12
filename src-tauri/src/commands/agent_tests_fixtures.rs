    fn test_environment(deployment: &str) -> ModelRuntimeEnv {
        ModelRuntimeEnv {
            llm: "openai/test-model".into(),
            api_key: "test-key".into(),
            api_base: "https://model.invalid/v1".into(),
            deployment: deployment.into(),
            full_power: false,
            prompt_audit_mode: "off".into(),
        }
    }

    fn test_route(mode: &str, surface: &str) -> FrontendRoute {
        FrontendRoute {
            url: "https://app.example.invalid".into(),
            score: 70,
            mode: mode.into(),
            surface: surface.into(),
            reasons: Vec::new(),
        }
    }

    fn test_plan(mode: &str) -> AgentExecutionPlan {
        test_plan_for(mode, "https://app.example.invalid")
    }

    /// Plans are built against a database so the frozen scope comes from the
    /// same authority the app uses; `missing_db_plan` keeps the target domain only.
    fn test_plan_for(mode: &str, url: &str) -> AgentExecutionPlan {
        let mut route = test_route(mode, "framework_application");
        route.url = url.to_string();
        build_agent_execution_plan(
            &AgentBudgetSettings::from_json(&serde_json::json!({})),
            &route,
            &test_environment("cloud"),
            AgentBackendKind::Native,
            Path::new("/nonexistent/oviraptor.sqlite3"),
            "agent-scan",
        )
    }

    pub(super) fn temp_database(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().canonicalize().unwrap()
            .join(format!("oviraptor-{tag}-{}", Uuid::new_v4()));
        let db_path = db::initialize(&root).unwrap();
        (root, db_path)
    }

    pub(super) fn seed_scan(db_path: &Path, scan_id: &str, status: &str) {
        let connection = db::open(db_path).unwrap();
        connection
            .execute("INSERT INTO projects(id,name) VALUES(9001,'Agent tests')", [])
            .unwrap();
        connection.execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,current_checkpoint,scan_type,attempt_count) VALUES(?1,9001,'Agent tests',?2,'测试','web',1)",
            params![scan_id, status],
        )
        .unwrap();
    }

    /// Test-only historical row. Production plan recording must never create
    /// or update a retired backend, so old lineage is seeded directly here.
    fn seed_retired_attempt_plan(
        db_path: &Path,
        scan_id: &str,
        attempt: i64,
        url: &str,
        plan: &AgentExecutionPlan,
    ) {
        assert_eq!(plan.backend, AgentBackendKind::LegacyRemoved);
        let connection = db::open(db_path).unwrap();
        connection.execute(
            "INSERT INTO agent_runs(id,scan_id,attempt_number,target_url,backend,role,plan_hash,plan_json)
             VALUES(?1,?2,?3,?4,?5,'coordinator',?6,?7)",
            params![
                Uuid::new_v4().to_string(), scan_id, attempt, url,
                plan.backend.as_str(), plan.hash(), plan.as_json().to_string()
            ],
        ).unwrap();
    }

    pub(super) fn seed_target(db_path: &Path, url: &str) {
        let connection = db::open(db_path).unwrap();
        connection
            .execute(
                "INSERT OR IGNORE INTO sentinel_targets(project_id,scan_id,company,url,status) VALUES(9001,'agent-scan','Agent tests',?1,'scanning')",
                [url],
            )
            .unwrap();
    }

    pub(super) fn seed_session(db_path: &Path, id: &str, cookie: &str, authorization: &str) {
        let connection = db::open(db_path).unwrap();
        let document = serde_json::json!({
            "schemaVersion": 1,
            "id": id,
            "projectId": 9001,
            "entryUrl": "https://app.example.invalid",
            "scopeHosts": ["app.example.invalid"],
            "cookies": [{"name": "session", "value": cookie}],
            "headers": {"authorization": authorization},
            "identityIsolation": {"sharedWithOtherSessions": false},
        });
        connection.execute(
            "INSERT INTO browser_auth_sessions(id,project_id,owner_scan_id,name,entry_url,status,session_json,expires_at) VALUES(?1,9001,'agent-scan',?2,'https://app.example.invalid','valid',?3,?4)",
            params![id, id, document.to_string(), (chrono::Utc::now() + chrono::Duration::hours(8)).to_rfc3339()],
        )
        .unwrap();
    }

    pub(super) fn test_context(
        db_path: &Path,
        target_url: &str,
        identities: Vec<AgentIdentity>,
    ) -> AgentRunContext {
        let mut route = test_route("standard", "framework_application");
        route.url = target_url.into();
        AgentRunContext {
            tool_configuration: None,
            supervision: None,
            external_surface: false,
            scan_id: "agent-scan".into(),
            attempt_number: 1,
            target_url: target_url.into(),
            target_dir: std::env::temp_dir().canonicalize().unwrap()
                .join(format!("oviraptor-target-{}", Uuid::new_v4())),
            db_path: db_path.to_path_buf(),
            route,
            execution_plan: web_mode_positive_plan_or(db_path,target_url,test_plan_for("standard", target_url)),
            evidence: serde_json::json!({
                "schemaVersion": 2,
                "url": target_url,
                "surface": "framework_application",
                "apiCandidates": [{"path": "/api/orders", "url": format!("{target_url}/api/orders"), "method": "GET", "parameters": ["page"], "source": "browser-runtime"}],
                "investigation": {"actions": [{"key": "open-orders", "type": "link", "url": format!("{target_url}/orders"), "triggeredApis": [{"method": "GET", "path": "/api/orders", "parameters": ["page"]}]}]},
                "verificationPlan": {"strategy": "runtime_api", "maxAttemptsPerCandidate": 2},
            }),
            capabilities: serde_json::json!({"schemaVersion": 0}),
            identities,
            proxy: None,
            log_path: std::env::temp_dir().join("oviraptor-agent-test.log"),
            environment: test_environment("cloud"),
            browser: None,
            resume: false,
            plan_rejection: None,
            run: None,
            run_budget: None,
        }
    }

    /// One request the loop really executed, as the ledger records it.
    fn executed_request(family: &str) -> AgentRequestTrace {
        trace_of("GET", "/api/orders", family, "")
    }

    fn trace_of(
        method: &str,
        path: &str,
        family: &str,
        contract_key: &str,
    ) -> AgentRequestTrace {
        AgentRequestTrace {
            method: method.into(),
            path: path.into(),
            origin: "app.example.invalid".into(),
            identity: "anonymous".into(),
            status: 200,
            family: family.into(),
            contract_key: contract_key.into(),
            tool: "replay_http".into(),
            scope_class: "authorized_business_api".into(),
            ..Default::default()
        }
    }

    /// The shape `agent_http_exchange` hands back for a JSON body.
    fn field_summary(body: &str) -> JsonValue {
        serde_json::json!({
            "status": 200,
            "contentType": "application/json",
            "fields": agent_field_fingerprints(
                body,
                "application/json",
                &crate::agent_runtime::secrets::RedactionContext::default(),
            ),
        })
    }

    fn state(turns: i64, total_tokens: i64, no_progress: i64) -> NativeAgentState {
        let mut state = NativeAgentState::fresh(
            1,
            AgentBackendKind::Native,
            "evidence",
            "plan",
            vec!["family:authorization".into()],
        );
        state.turns = turns;
        state.no_progress_streak = no_progress;
        state.token_usage = AgentTokenUsage {
            input_tokens: total_tokens / 2,
            cached_input_tokens: 0,
            output_tokens: total_tokens / 2,
            total_tokens,
            model_requests: turns,
        };
        state
    }

    // ------------------------------------------------------------------
    // §12 cases 1-12: plan, budget, scope and ledger semantics
    // ------------------------------------------------------------------
