use super::*;
use uuid::Uuid;

#[test]
fn initializes_phase_one_homogeneous_control_plane() {
    // DESIGN §4：Phase 1 新表与 agent_assignments 增列必须在一次 initialize 后齐备。
    let root = std::env::temp_dir().join(format!("oviraptor-homogeneous-schema-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = Connection::open(path).unwrap();
    for table in [
        "agent_homogeneous_tasks",
        "agent_task_slices",
        "agent_lane_quotas",
        "agent_lane_slots",
        "agent_acceptance_verdicts",
        "agent_human_directive_gated",
        "agent_worker_instances",
        "agent_worker_reports",
        "agent_evidence_aliases",
    ] {
        let present: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(present, 1, "缺少 Phase 1 新表：{table}");
    }
    for column in [
        "slice_id",
        "scheduling_model",
        "worker_profile_id",
        "homogeneous_worker_id",
        "attempt",
    ] {
        assert!(
            column_exists(&connection, "agent_assignments", column).unwrap(),
            "agent_assignments 缺少增列：{column}"
        );
    }
    assert!(column_exists(&connection, "agent_runs", "scheduling_model").unwrap());
    // 旧 lane 互斥表结构必须一字不变（DESIGN §5.1：homogeneous 不写它）。
    let pk: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_lane_leases'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        pk.contains("PRIMARY KEY(scan_id,attempt_number,target_key,lane)"),
        "agent_lane_leases 主键被改动：{pk}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn initializes_complete_exposure_workflow_schema() {
    let root = std::env::temp_dir().join(format!("oviraptor-exposure-schema-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = Connection::open(path).unwrap();
    for column in ["stage", "current_source", "cancel_requested"] {
        assert!(column_exists(&connection, "exposure_runs", column).unwrap());
    }
    let source_table:i64=connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='exposure_source_results'",[],|row|row.get(0)).unwrap();
    assert_eq!(source_table, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn migration_adds_chat_thread_binding_to_existing_directives() {
    let root = std::env::temp_dir().join(format!("oviraptor-thread-migration-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    connection.execute_batch(
        "ALTER TABLE agent_user_directives DROP COLUMN thread_key; \
         ALTER TABLE agent_directive_drafts DROP COLUMN thread_key;",
    ).unwrap();
    drop(connection);
    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    for table in ["agent_user_directives", "agent_directive_drafts"] {
        assert!(column_exists(&connection, table, "thread_key").unwrap());
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn upgrades_existing_database_with_non_executable_host_boundary_candidates() {
    let root = std::env::temp_dir().join(format!("oviraptor-host-boundary-migration-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    connection.execute_batch("DROP TABLE agent_host_boundary_candidates;").unwrap();
    drop(connection);
    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let table_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='agent_host_boundary_candidates'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(table_count, 1);
    let schema: String = connection.query_row(
        "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_host_boundary_candidates'",
        [], |row| row.get(0),
    ).unwrap();
    assert!(schema.contains("CHECK(execution_eligible=0)"));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn migrates_legacy_browser_sessions_before_creating_task_scope_index() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-legacy-auth-session-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("oviraptor.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            r#"
                CREATE TABLE browser_auth_sessions (
                    id TEXT PRIMARY KEY,
                    project_id INTEGER NOT NULL,
                    name TEXT NOT NULL DEFAULT '',
                    entry_url TEXT NOT NULL,
                    final_url TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL DEFAULT 'capturing',
                    scope_hosts_json TEXT NOT NULL DEFAULT '[]',
                    cookie_count INTEGER NOT NULL DEFAULT 0,
                    header_count INTEGER NOT NULL DEFAULT 0,
                    storage_count INTEGER NOT NULL DEFAULT 0,
                    captured_request_count INTEGER NOT NULL DEFAULT 0,
                    session_json TEXT NOT NULL DEFAULT '{}',
                    last_validated_at TEXT NOT NULL DEFAULT '',
                    expires_at TEXT NOT NULL DEFAULT '',
                    last_error TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL DEFAULT '',
                    updated_at TEXT NOT NULL DEFAULT ''
                );
                "#,
        )
        .unwrap();
    drop(connection);

    let migrated = initialize(&root).unwrap();
    let connection = open(&migrated).unwrap();
    for column in ["capture_previous_status", "owner_scan_id", "draft_scope_id"] {
        assert!(column_exists(&connection, "browser_auth_sessions", column).unwrap());
    }
    let index: String = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type='index' AND name='idx_browser_auth_sessions_task_scope'",
                [],
                |row| row.get(0),
            )
            .unwrap();
    assert_eq!(index, "idx_browser_auth_sessions_task_scope");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn initializes_security_opportunity_inbox() {
    let root = std::env::temp_dir().join(format!("oviraptor-opportunity-inbox-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let columns: Vec<String> = connection
        .prepare("PRAGMA table_info(sentinel_opportunities)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for expected in [
        "opportunity_key",
        "score",
        "status",
        "why_json",
        "evidence_json",
        "recommended_action_json",
        "record_json",
    ] {
        assert!(
            columns.iter().any(|column| column == expected),
            "missing {expected}"
        );
    }
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn migration_downgrades_inferred_opportunities_from_agent_queue() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-opportunity-readiness-{}",
        Uuid::new_v4()
    ));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    connection
        .execute("INSERT INTO projects(name) VALUES('Readiness')", [])
        .unwrap();
    let project_id = connection.last_insert_rowid();
    connection.execute("INSERT INTO sentinel_scans(id,project_id,project_name) VALUES('readiness-scan',?1,'Readiness')", [project_id]).unwrap();
    connection.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,source,record_json) VALUES(?1,'readiness-scan','https://example.test','inferred-login','identity_surface','inferred',88,'ready','high','evidence-reconstruction','{\"score\":88,\"method\":\"UNKNOWN\"}')", [project_id]).unwrap();
    connection.execute("INSERT INTO investigation_hypotheses(project_id,scan_id,target_url,hypothesis_key,category,title,status,score,confidence,decision_json,source_opportunity_key) VALUES(?1,'readiness-scan','https://example.test','hypothesis','identity_surface','inferred','ready',88,'high','{\"eligibleForModel\":true}','inferred-login')", [project_id]).unwrap();
    connection.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,source,record_json) VALUES(?1,'readiness-scan','https://example.test','ordinary-session','identity_surface','session restore',86,'ready','high','runtime-request','{\"score\":86,\"method\":\"GET\",\"endpoint\":\"/account/restore_login\",\"readiness\":{\"stage\":\"agent_ready\"}}')", [project_id]).unwrap();
    connection.execute("INSERT INTO sentinel_opportunities(project_id,scan_id,target_url,opportunity_key,category,title,score,status,confidence,source,record_json) VALUES(?1,'readiness-scan','https://example.test','transport-device','identity_surface','session restore with device id',100,'ready','high','runtime-request','{\"score\":100,\"method\":\"GET\",\"endpoint\":\"/account/restore_login\",\"riskEvidence\":{\"present\":true,\"signalCount\":1,\"signals\":[{\"type\":\"object_boundary_parameter\",\"fields\":[\"device_id\"]}]}}')", [project_id]).unwrap();
    connection
        .execute(
            "DELETE FROM app_settings WHERE key='opportunity_readiness_gate_version'",
            [],
        )
        .unwrap();
    drop(connection);

    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let opportunity_status: String = connection
        .query_row(
            "SELECT status FROM sentinel_opportunities WHERE opportunity_key='inferred-login'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let (hypothesis_status, eligible): (String, i64) = connection
            .query_row(
                "SELECT status,COALESCE(json_extract(decision_json,'$.eligibleForModel'),1) FROM investigation_hypotheses WHERE hypothesis_key='hypothesis'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
    let (ordinary_status, ordinary_disposition): (String, String) = connection
            .query_row(
                "SELECT status,json_extract(record_json,'$.disposition') FROM sentinel_opportunities WHERE opportunity_key='ordinary-session'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
    let (transport_status, transport_disposition): (String, String) = connection
            .query_row(
                "SELECT status,json_extract(record_json,'$.disposition') FROM sentinel_opportunities WHERE opportunity_key='transport-device'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
    assert_eq!(opportunity_status, "dismissed");
    assert_eq!(hypothesis_status, "rejected");
    assert_eq!(eligible, 0);
    assert_eq!(ordinary_status, "dismissed");
    assert_eq!(ordinary_disposition, "api_inventory_only");
    assert_eq!(transport_status, "dismissed");
    assert_eq!(transport_disposition, "transport_identifier_only");
    assert_eq!(
        migration_version(&connection, "opportunity_readiness_gate_version"),
        7
    );
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn initializes_learning_candidate_lifecycle_table() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-learning-candidate-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let table: String = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='agent_learning_candidates'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table, "agent_learning_candidates");
    let columns: Vec<String> = connection
        .prepare("PRAGMA table_info(agent_learning_candidates)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for expected in [
        "scan_id",
        "candidate_json",
        "status",
        "target_skill_id",
        "source_hash",
    ] {
        assert!(
            columns.iter().any(|column| column == expected),
            "missing {expected}"
        );
    }
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn target_asset_backfill_is_indexed_and_runs_only_once() {
    let root = std::env::temp_dir().join(format!("oviraptor-db-migration-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    connection
        .execute("INSERT INTO projects(name) VALUES('Migration')", [])
        .unwrap();
    let project_id = connection.last_insert_rowid();
    connection
            .execute(
                "INSERT INTO assets(asset_key,link,canonical_key) VALUES('asset','https://matched.invalid/','https://matched.invalid')",
                [],
            )
            .unwrap();
    let asset_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO project_assets(project_id,asset_id) VALUES(?1,?2)",
            params![project_id, asset_id],
        )
        .unwrap();
    connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name) VALUES('migration-scan',?1,'Migration')",
                [project_id],
            )
            .unwrap();
    for url in ["https://matched.invalid/", "https://url-only.invalid/"] {
        connection
                .execute(
                    "INSERT INTO sentinel_targets(project_id,scan_id,url) VALUES(?1,'migration-scan',?2)",
                    params![project_id, url],
                )
                .unwrap();
    }
    connection
        .execute(
            "DELETE FROM app_settings WHERE key='sentinel_asset_backfill_version'",
            [],
        )
        .unwrap();
    drop(connection);

    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let matched: Option<i64> = connection
        .query_row(
            "SELECT asset_id FROM sentinel_targets WHERE url='https://matched.invalid/'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let unmatched: Option<i64> = connection
        .query_row(
            "SELECT asset_id FROM sentinel_targets WHERE url='https://url-only.invalid/'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(matched, Some(asset_id));
    assert_eq!(unmatched, None);
    assert_eq!(
        migration_version(&connection, "sentinel_asset_backfill_version"),
        1
    );

    connection
        .execute(
            "UPDATE sentinel_targets SET asset_id=NULL WHERE url='https://matched.invalid/'",
            [],
        )
        .unwrap();
    drop(connection);
    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let not_repeated: Option<i64> = connection
        .query_row(
            "SELECT asset_id FROM sentinel_targets WHERE url='https://matched.invalid/'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(not_repeated, None);
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn relabels_recon_only_task_as_completed_when_queue_is_exhausted() {
    let root = std::env::temp_dir().join(format!(
        "oviraptor-recon-only-status-test-{}",
        Uuid::new_v4()
    ));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    connection
        .execute("INSERT INTO projects(name) VALUES('Recon only')", [])
        .unwrap();
    let project_id = connection.last_insert_rowid();
    connection
            .execute(
                "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type) VALUES('recon-only-scan',?1,'Recon only','recon_only','web')",
                [project_id],
            )
            .unwrap();
    connection
            .execute(
                "INSERT INTO sentinel_targets(project_id,scan_id,url,status,scan_mode) VALUES(?1,'recon-only-scan','https://static.invalid','recon_only','skip')",
                [project_id],
            )
            .unwrap();
    drop(connection);

    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let status: String = connection
        .query_row(
            "SELECT status FROM sentinel_scans WHERE id='recon-only-scan'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "completed");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

/// Retired settings must not activate a Native model. Explicit current settings
/// must survive subsequent application initialization without old credentials.
#[test]
fn retired_model_settings_are_preserved_without_activating_native_profiles() {
    let root = std::env::temp_dir().join(format!("oviraptor-neutral-model-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    connection
            .execute(
                r#"UPDATE config_profiles SET settings_json=json_remove(
                    json_set(
                        json_object(
                            'strixLlm','openai/legacy-model',
                            'strixApiBase','https://legacy.example.invalid/v1',
                            'strixApiKey','legacy-key'
                        ),
                        '$.strixLlmProfiles',
                        json('[{"id":"p-2","name":"当前","llm":"openai/current","apiBase":"https://current.example.invalid/v1","apiKey":"current-key","localApiKey":"","deployment":"cloud"}]'),
                        '$.strixActiveLlmProfileId','p-2'
                    ),
                    '$.modelProfiles','$.activeModelProfileId','$.modelDeployment','$.modelApiBase','$.modelApiKey','$.localApiKey')"#,
                [],
            )
            .unwrap();
    drop(connection);

    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let raw: String = connection
        .query_row(
            "SELECT settings_json FROM config_profiles LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&raw).unwrap();
    for current_key in [
        "modelProfiles", "activeModelProfileId", "modelName", "modelApiBase",
        "modelApiKey", "modelDeployment", "localApiKey",
    ] {
        assert!(settings.get(current_key).is_none(), "retired configuration activated {current_key}");
    }
    for legacy_key in [
        "strixLlm",
        "strixApiBase",
        "strixApiKey",
        "strixLlmProfiles",
        "strixActiveLlmProfileId",
    ] {
        assert!(settings.get(legacy_key).is_some(), "startup deleted {legacy_key}");
    }
    drop(connection);

    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let repeated: String = connection.query_row(
        "SELECT settings_json FROM config_profiles LIMIT 1", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&repeated).unwrap(), settings);
    drop(connection);

    // A later edit in the new shape must survive a restart.
    let connection = open(&path).unwrap();
    connection
            .execute(
                "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.modelApiBase','https://edited.example.invalid/v1')",
                [],
            )
            .unwrap();
    drop(connection);
    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let raw: String = connection
        .query_row(
            "SELECT settings_json FROM config_profiles LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let settings: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        settings
            .get("modelApiBase")
            .and_then(serde_json::Value::as_str),
        Some("https://edited.example.invalid/v1"),
        "explicit current configuration survives restart"
    );
    let settings = normalize_settings(&settings);
    assert_eq!(settings["modelProfiles"][0]["apiBase"], "https://edited.example.invalid/v1");
    assert_eq!(settings["modelProfiles"][0]["apiKey"], "");
    assert_eq!(settings["modelProfiles"][0]["llm"], "");
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn restores_latest_attempt_scope_without_counting_historical_targets() {
    let root =
        std::env::temp_dir().join(format!("oviraptor-attempt-scope-test-{}", Uuid::new_v4()));
    let path = initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    connection
        .execute("INSERT INTO projects(name) VALUES('Attempt scope')", [])
        .unwrap();
    let project_id = connection.last_insert_rowid();
    connection.execute(
            "INSERT INTO sentinel_scans(id,project_id,project_name,status,scan_type,attempt_count,current_checkpoint) VALUES('attempt-scope',?1,'Attempt scope','partial','web',2,'任务累计状态：待补充验证 1，确定性侦察收口 1')",
            [project_id],
        ).unwrap();
    connection.execute(
            "INSERT INTO sentinel_targets(project_id,scan_id,url,status,scan_mode,routing_reason) VALUES(?1,'attempt-scope','https://historical.invalid','recon_only','skip','历史确定性收口'),(?1,'attempt-scope','https://current.invalid','partial','standard','本轮没有形成任何工具证据')",
            [project_id],
        ).unwrap();
    let first = root.join("strix-jobs/attempt-scope/attempt-0001");
    let second = root.join("strix-jobs/attempt-scope/attempt-0002");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::write(
        first.join("targets.json"),
        br#"[{"url":"https://historical.invalid"}]"#,
    )
    .unwrap();
    fs::write(
        second.join("targets.json"),
        br#"[{"url":"https://current.invalid"}]"#,
    )
    .unwrap();
    connection.execute(
            "INSERT INTO sentinel_scan_attempts(scan_id,attempt_number,status,stage,checkpoint,stop_reason,work_dir) VALUES('attempt-scope',1,'completed','complete','旧轮次','旧轮次',?1),('attempt-scope',2,'partial','complete','待补充验证 1，仅侦察收口 1','待补充验证 1，仅侦察收口 1',?2)",
            params![first.to_string_lossy(), second.to_string_lossy()],
        ).unwrap();
    connection.execute(
            "DELETE FROM app_settings WHERE key IN ('sentinel_target_attempt_version','sentinel_attempt_scope_summary_version')",
            [],
        ).unwrap();
    drop(connection);

    initialize(&root).unwrap();
    let connection = open(&path).unwrap();
    let attempts: Vec<(String, i64)> = connection.prepare(
            "SELECT url,last_attempt_number FROM sentinel_targets WHERE scan_id='attempt-scope' ORDER BY url",
        ).unwrap().query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(
        attempts,
        vec![
            ("https://current.invalid".into(), 2),
            ("https://historical.invalid".into(), 1),
        ]
    );
    let summary: String = connection.query_row(
            "SELECT stop_reason FROM sentinel_scan_attempts WHERE scan_id='attempt-scope' AND attempt_number=2",
            [],
            |row| row.get(0),
        ).unwrap();
    assert!(summary.contains("待补充验证 1"));
    assert!(summary.contains("确定性侦察收口 0"));
    assert!(!summary.contains("确定性侦察收口 1"));
    drop(connection);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn migrates_review_verdict_vocabulary_without_losing_history_or_constraints() {
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
    connection
        .execute_batch(
            r#"
            CREATE TABLE agent_runs(id TEXT PRIMARY KEY);
            CREATE TABLE agent_assignments(id TEXT PRIMARY KEY);
            CREATE TABLE sentinel_scans(id TEXT PRIMARY KEY);
            INSERT INTO agent_runs(id) VALUES('root'),('reviewer');
            INSERT INTO agent_assignments(id) VALUES('assignment');
            INSERT INTO sentinel_scans(id) VALUES('scan');

            CREATE TABLE agent_review_requests (
                id TEXT PRIMARY KEY,
                root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
                assignment_id TEXT NOT NULL REFERENCES agent_assignments(id) ON DELETE CASCADE,
                reviewer_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
                candidate_id TEXT NOT NULL,
                candidate_revision INTEGER NOT NULL,
                candidate_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending'
                  CHECK(status IN ('pending','running','confirmed','rejected','needs_evidence','failed','superseded')),
                decision_id INTEGER,
                lease_epoch INTEGER NOT NULL,
                fencing_token TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                finished_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(candidate_id,candidate_revision)
            );
            CREATE INDEX idx_agent_review_root
              ON agent_review_requests(root_run_id,status,created_at);

            CREATE TABLE agent_finding_candidates (
                id TEXT PRIMARY KEY,
                root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
                candidate_revision INTEGER NOT NULL DEFAULT 0,
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                target_url TEXT NOT NULL DEFAULT '',
                stage TEXT NOT NULL,
                kind TEXT NOT NULL,
                record_key TEXT NOT NULL DEFAULT '',
                title TEXT NOT NULL DEFAULT '',
                severity TEXT NOT NULL DEFAULT '',
                record_json TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'pending'
                  CHECK(status IN ('pending','published','rejected','needs_evidence','superseded')),
                reviewer_run_id TEXT NOT NULL DEFAULT '',
                published_at TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(root_run_id,scan_id,target_url,stage,kind,record_key)
            );
            CREATE INDEX idx_agent_finding_candidates_review
              ON agent_finding_candidates(root_run_id,candidate_revision,status);

            INSERT INTO agent_review_requests(
                id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,
                candidate_json,status,lease_epoch,fencing_token
            ) VALUES(
                'legacy-review','root','assignment','reviewer','legacy-candidate',1,
                '{}','needs_evidence',1,'legacy-token'
            );
            INSERT INTO agent_finding_candidates(
                id,root_run_id,candidate_revision,scan_id,target_url,stage,kind,record_key,status
            ) VALUES(
                'legacy-finding','root',1,'scan','https://example.test','agent','test','legacy',
                'needs_evidence'
            );
            "#,
        )
        .unwrap();

    migrate_agent_review_verdict_vocabulary(&mut connection).unwrap();

    let legacy_review: String = connection
        .query_row(
            "SELECT status FROM agent_review_requests WHERE id='legacy-review'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let legacy_finding: String = connection
        .query_row(
            "SELECT status FROM agent_finding_candidates WHERE id='legacy-finding'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_review, "needs_evidence");
    assert_eq!(legacy_finding, "needs_evidence");

    connection
        .execute(
            "INSERT INTO agent_review_requests(
                id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,
                candidate_json,status,lease_epoch,fencing_token
             ) VALUES('canonical-review','root','assignment','reviewer','canonical-candidate',1,
                '{}','insufficient_evidence',2,'canonical-token')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO agent_finding_candidates(
                id,root_run_id,candidate_revision,scan_id,target_url,stage,kind,record_key,status
             ) VALUES('canonical-finding','root',2,'scan','https://example.test','agent','test',
                'canonical','insufficient_evidence')",
            [],
        )
        .unwrap();

    assert!(connection
        .execute(
            "INSERT INTO agent_review_requests(
                id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,
                candidate_json,status,lease_epoch,fencing_token
             ) VALUES('duplicate-review','root','assignment','reviewer','canonical-candidate',1,
                '{}','running',2,'canonical-token')",
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO agent_finding_candidates(
                id,root_run_id,candidate_revision,scan_id,target_url,stage,kind,record_key,status
             ) VALUES('duplicate-finding','root',3,'scan','https://example.test','agent','test',
                'canonical','pending')",
            [],
        )
        .is_err());

    for index in [
        "idx_agent_review_root",
        "idx_agent_finding_candidates_review",
    ] {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='index' AND name=?1)",
                [index],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "migration dropped index {index}");
    }
    for table in ["agent_review_requests", "agent_finding_candidates"] {
        let foreign_keys: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM pragma_foreign_key_list('{table}')"),
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(foreign_keys > 0, "migration dropped foreign keys for {table}");
    }
}
