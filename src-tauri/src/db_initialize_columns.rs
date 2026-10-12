// Launch migrations that only add or verify columns: the agent-runtime fields, the
// platform schema check and the fuse-entry status repair. Included from db_initialize.rs.

fn suspend_collaboration_event_triggers(connection: &Connection) -> Result<(), String> {
    // Some legacy migrations rebuild agent_runs with ALTER TABLE/rename. SQLite
    // validates every trigger during that window, so derived event triggers must
    // be absent until the canonical tables are back in place. The durable event
    // rows remain untouched and the triggers are recreated at the end of launch.
    connection
        .execute_batch(
            r#"
            DROP TRIGGER IF EXISTS agent_collaboration_draft_insert;
            DROP TRIGGER IF EXISTS agent_collaboration_draft_update;
            DROP TRIGGER IF EXISTS agent_collaboration_directive_insert;
            DROP TRIGGER IF EXISTS agent_collaboration_directive_update;
            DROP TRIGGER IF EXISTS agent_collaboration_message_insert;
            DROP TRIGGER IF EXISTS agent_collaboration_message_update;
            DROP TRIGGER IF EXISTS agent_collaboration_run_insert;
            DROP TRIGGER IF EXISTS agent_collaboration_run_update;
            DROP TRIGGER IF EXISTS agent_collaboration_assignment_insert;
            DROP TRIGGER IF EXISTS agent_collaboration_assignment_update;
            DROP TRIGGER IF EXISTS agent_collaboration_review_insert;
            DROP TRIGGER IF EXISTS agent_collaboration_review_update;
            DROP TRIGGER IF EXISTS agent_collaboration_request_review_insert;
            "#,
        )
        .map_err(|error| format!("暂停协作事件触发器失败：{error}"))
}

/// Agent-runtime columns on a development database, the light per-table additions older releases are missing, and retry children whose parent scan was deleted.
fn migrate_legacy_agent_and_retry_columns(connection: &mut Connection) -> Result<(), String> {
    ensure_column(connection, "source_snapshots", "frozen_root", "TEXT NOT NULL DEFAULT ''")?;
    ensure_column(connection, "source_snapshots", "changed_files_json", "TEXT NOT NULL DEFAULT 'null'")?;
    ensure_column(connection, "source_scope_contracts", "analysis_policy_version", "INTEGER NOT NULL DEFAULT 0 CHECK(analysis_policy_version IN (0,1))")?;
    // The agent runtime tables are new; this keeps a development database that
    // already created them before the state column existed usable.
    let _ = ensure_column(
        &*connection,
        "agent_runs",
        "terminal_state",
        "TEXT NOT NULL DEFAULT ''",
    );
    let _ = connection.execute("ALTER TABLE worker_nodes ADD COLUMN last_sync_at TEXT", []);
    let _ = connection.execute(
        "ALTER TABLE asset_ownership_profiles ADD COLUMN jurisdictions_json TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE asset_ownership_profiles ADD COLUMN excluded_jurisdictions_json TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE asset_ownership_profiles ADD COLUMN shared_domains_json TEXT NOT NULL DEFAULT '[]'",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE exposure_runs ADD COLUMN stage TEXT NOT NULL DEFAULT 'queued'",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE exposure_runs ADD COLUMN current_source TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE exposure_runs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "UPDATE exposure_runs SET status='interrupted',stage='interrupted',current_source='',error=CASE WHEN error='' THEN '应用上次退出时采集尚未结束；已发现结果已保留，可重新开始采集' ELSE error END,completed_at=datetime('now','localtime') WHERE status IN ('queued','running')",
        [],
    );
    // 轻量迁移：旧版数据库缺少该列时添加，已存在时忽略 duplicate column。
    let _ = connection.execute(
        "ALTER TABLE assets ADD COLUMN probe_hash TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE assets ADD COLUMN canonical_key TEXT NOT NULL DEFAULT ''",
        [],
    );
    // -1 means the probe has not classified this asset. 0/1 are written only after a body was read.
    ensure_column(connection, "assets", "has_login", "INTEGER NOT NULL DEFAULT -1")?;
    ensure_column(connection, "assets", "has_register", "INTEGER NOT NULL DEFAULT -1")?;
    ensure_column(connection, "project_assets", "entry_opened_at", "TEXT")?;
    let _ = connection.execute("ALTER TABLE sentinel_targets ADD COLUMN scan_id TEXT", []);
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN value_score INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN scan_mode TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN routing_reason TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_targets ADD COLUMN last_attempt_number INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN previous_scan_id TEXT NOT NULL DEFAULT ''",
        [],
    );
    // Retries created by older releases copied their own targets/checkpoints
    // but still pointed at the first scan for future execution. If that parent
    // was deleted, the dangling comparison link made an otherwise complete
    // child impossible to retry. New retries run in place; detach legacy links
    // whose parent no longer exists.
    let _ = connection.execute(
        "UPDATE sentinel_scans SET previous_scan_id='' WHERE trim(previous_scan_id)<>'' AND NOT EXISTS (SELECT 1 FROM sentinel_scans parent WHERE parent.id=sentinel_scans.previous_scan_id)",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN llm_requests INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN input_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN output_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN cached_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN total_tokens INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN scan_type TEXT NOT NULL DEFAULT 'web'",
        [],
    );
    let _ = connection.execute(
        "ALTER TABLE sentinel_scans ADD COLUMN task_name TEXT NOT NULL DEFAULT ''",
        [],
    );
    Ok(())
}

/// Fields that first shipped on macOS. A locked or interrupted Windows upgrade must not keep an older schema, so the columns are verified instead of trusted.
fn verify_locked_platform_schema(connection: &mut Connection) -> Result<(), String> {
    ensure_column(
        &*connection,
        "sentinel_scans",
        "archived_at",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    // These two fields were introduced on macOS first. Verify the migration so a
    // locked or interrupted Windows upgrade cannot silently keep an older schema.
    ensure_column(
        &*connection,
        "sentinel_scans",
        "source_path",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &*connection,
        "sentinel_scans",
        "skill_names",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &*connection,
        "sentinel_scans",
        "attempt_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    ensure_column(
        &*connection,
        "browser_auth_sessions",
        "capture_previous_status",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &*connection,
        "browser_auth_sessions",
        "owner_scan_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    ensure_column(
        &*connection,
        "browser_auth_sessions",
        "draft_scope_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    connection
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_task_scope ON browser_auth_sessions(owner_scan_id,draft_scope_id,project_id,updated_at DESC)",
            [],
        )
        .map_err(|error| format!("创建任务会话作用域索引失败：{error}"))?;
    let _ = connection.execute(
        "UPDATE sentinel_scans SET attempt_count=1 WHERE attempt_count=0 AND status<>'draft'",
        [],
    );
    for sql in [
        "ALTER TABLE security_rule_packs ADD COLUMN progress INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN progress_stage TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE security_rule_packs ADD COLUMN progress_message TEXT NOT NULL DEFAULT ''",
    ] {
        let _ = connection.execute(sql, []);
    }
    connection
        .execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sentinel_processes (
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                process_id INTEGER NOT NULL DEFAULT 0,
                engine TEXT NOT NULL DEFAULT '',
                work_dir TEXT NOT NULL DEFAULT '',
                started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                PRIMARY KEY(scan_id,process_id)
            );
            CREATE TABLE IF NOT EXISTS sentinel_scan_attempts (
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                attempt_number INTEGER NOT NULL,
                execution_mode TEXT NOT NULL DEFAULT 'initial',
                status TEXT NOT NULL DEFAULT 'scanning',
                stage TEXT NOT NULL DEFAULT 'initializing',
                checkpoint TEXT NOT NULL DEFAULT '',
                stop_reason TEXT NOT NULL DEFAULT '',
                work_dir TEXT NOT NULL DEFAULT '',
                llm_requests_start INTEGER NOT NULL DEFAULT 0,
                input_tokens_start INTEGER NOT NULL DEFAULT 0,
                output_tokens_start INTEGER NOT NULL DEFAULT 0,
                cached_tokens_start INTEGER NOT NULL DEFAULT 0,
                total_tokens_start INTEGER NOT NULL DEFAULT 0,
                llm_requests_delta INTEGER NOT NULL DEFAULT 0,
                input_tokens_delta INTEGER NOT NULL DEFAULT 0,
                output_tokens_delta INTEGER NOT NULL DEFAULT 0,
                cached_tokens_delta INTEGER NOT NULL DEFAULT 0,
                total_tokens_delta INTEGER NOT NULL DEFAULT 0,
                started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                finished_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                PRIMARY KEY(scan_id,attempt_number)
            );
            CREATE INDEX IF NOT EXISTS idx_sentinel_attempt_scan ON sentinel_scan_attempts(scan_id,attempt_number DESC);
            CREATE TABLE IF NOT EXISTS security_rule_packs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                key TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                engine TEXT NOT NULL,
                repository TEXT NOT NULL,
                reference TEXT NOT NULL DEFAULT 'main',
                local_path TEXT NOT NULL DEFAULT '',
                previous_version TEXT NOT NULL DEFAULT '',
                version TEXT NOT NULL DEFAULT '',
                enabled INTEGER NOT NULL DEFAULT 1,
                builtin INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'not_installed',
                last_sync_at TEXT NOT NULL DEFAULT '',
                error TEXT NOT NULL DEFAULT '',
                added_count INTEGER NOT NULL DEFAULT 0,
                modified_count INTEGER NOT NULL DEFAULT 0,
                deleted_count INTEGER NOT NULL DEFAULT 0,
                change_summary TEXT NOT NULL DEFAULT '[]',
                progress INTEGER NOT NULL DEFAULT 0,
                progress_stage TEXT NOT NULL DEFAULT '',
                progress_message TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_security_rule_packs_enabled ON security_rule_packs(enabled,engine);
            CREATE TABLE IF NOT EXISTS sentinel_scan_contexts (
                scan_id TEXT PRIMARY KEY REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                environment TEXT NOT NULL DEFAULT '',
                auth_profile_name TEXT NOT NULL DEFAULT '',
                auth_type TEXT NOT NULL DEFAULT 'none',
                authenticated INTEGER NOT NULL DEFAULT 0,
                ci_provider TEXT NOT NULL DEFAULT '',
                repository_url TEXT NOT NULL DEFAULT '',
                branch TEXT NOT NULL DEFAULT '',
                commit_sha TEXT NOT NULL DEFAULT '',
                build_id TEXT NOT NULL DEFAULT '',
                policy_json TEXT NOT NULL DEFAULT '{}',
                gate_status TEXT NOT NULL DEFAULT 'not_evaluated',
                gate_reason TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE TABLE IF NOT EXISTS browser_auth_sessions (
                id TEXT PRIMARY KEY,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                owner_scan_id TEXT NOT NULL DEFAULT '',
                draft_scope_id TEXT NOT NULL DEFAULT '',
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
                capture_previous_status TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
            );
            CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_project ON browser_auth_sessions(project_id,status,updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_browser_auth_sessions_task_scope ON browser_auth_sessions(owner_scan_id,draft_scope_id,project_id,updated_at DESC);
            CREATE TABLE IF NOT EXISTS appsec_vulnerabilities (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                fingerprint TEXT NOT NULL,
                title TEXT NOT NULL,
                vulnerability_type TEXT NOT NULL DEFAULT '',
                severity TEXT NOT NULL DEFAULT 'info',
                status TEXT NOT NULL DEFAULT 'open',
                confidence TEXT NOT NULL DEFAULT '',
                asset TEXT NOT NULL DEFAULT '',
                environment TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL DEFAULT '',
                http_method TEXT NOT NULL DEFAULT '',
                parameter TEXT NOT NULL DEFAULT '',
                file TEXT NOT NULL DEFAULT '',
                symbol TEXT NOT NULL DEFAULT '',
                start_line INTEGER NOT NULL DEFAULT 0,
                correlation_score INTEGER NOT NULL DEFAULT 0,
                correlation_json TEXT NOT NULL DEFAULT '{}',
                first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                owner TEXT NOT NULL DEFAULT '',
                UNIQUE(project_id,fingerprint)
            );
            CREATE TABLE IF NOT EXISTS appsec_vulnerability_sources (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                vulnerability_id INTEGER NOT NULL REFERENCES appsec_vulnerabilities(id) ON DELETE CASCADE,
                scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                finding_id INTEGER REFERENCES sentinel_findings(id) ON DELETE CASCADE,
                source_type TEXT NOT NULL,
                source_key TEXT NOT NULL,
                engine TEXT NOT NULL DEFAULT '',
                evidence_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(vulnerability_id,scan_id,source_type,source_key)
            );
            CREATE INDEX IF NOT EXISTS idx_appsec_vuln_project ON appsec_vulnerabilities(project_id,last_seen);
            CREATE INDEX IF NOT EXISTS idx_appsec_source_scan ON appsec_vulnerability_sources(scan_id,vulnerability_id);
            CREATE TABLE IF NOT EXISTS sentinel_fuse_zone (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
                company TEXT NOT NULL DEFAULT '',
                url TEXT NOT NULL,
                normalized_url TEXT NOT NULL,
                source_scan_id TEXT NOT NULL DEFAULT '',
                reason TEXT NOT NULL DEFAULT '',
                verdict TEXT NOT NULL DEFAULT 'pending',
                note TEXT NOT NULL DEFAULT '',
                evidence TEXT NOT NULL DEFAULT '',
                archived INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                UNIQUE(project_id,normalized_url)
            );
            CREATE INDEX IF NOT EXISTS idx_sentinel_fuse_project ON sentinel_fuse_zone(project_id,archived,updated_at);
            "#,
        )
        .map_err(|error| error.to_string())?;
    ensure_column(
        &*connection,
        "sentinel_scan_attempts",
        "execution_mode",
        "TEXT NOT NULL DEFAULT 'initial'",
    )?;
    Ok(())
}

/// Runtime orchestration columns on `agent_runs`: invocation provenance, the
/// per-attempt backend plan, the frozen plan and the fields used by live child runs.
/// Old installations retain the same-revision edge triggers because `IF NOT
/// EXISTS` never upgrades their bodies. Rebuild both guards in one transaction
/// after importing their existing node revisions into a linear ancestry chain.
fn migrate_evidence_revision_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "BEGIN IMMEDIATE;
             INSERT OR IGNORE INTO agent_evidence_revisions
                 (root_run_id,revision,parent_revision,cause_event_id,manifest_hash)
             SELECT n.root_run_id,n.revision,
                    (SELECT MAX(prior.revision) FROM agent_evidence_nodes prior
                     WHERE prior.root_run_id=n.root_run_id AND prior.revision<n.revision),
                    'legacy-backfill',''
             FROM agent_evidence_nodes n WHERE n.revision>0
             GROUP BY n.root_run_id,n.revision ORDER BY n.root_run_id,n.revision;
             DROP TRIGGER IF EXISTS agent_evidence_edges_endpoints_insert;
             DROP TRIGGER IF EXISTS agent_evidence_edges_endpoints_update;
             CREATE TRIGGER IF NOT EXISTS agent_evidence_nodes_revision_insert
             AFTER INSERT ON agent_evidence_nodes
             BEGIN
                 INSERT OR IGNORE INTO agent_evidence_revisions
                     (root_run_id,revision,parent_revision,cause_event_id,manifest_hash)
                 VALUES(NEW.root_run_id,NEW.revision,
                     (SELECT MAX(revision) FROM agent_evidence_revisions
                      WHERE root_run_id=NEW.root_run_id AND revision<NEW.revision),
                     'node:' || NEW.id,'');
             END;",
        )
        .map_err(|error| format!("无法迁移 evidence revision：{error}"))?;
    let install = (|| -> Result<(), String> {
        for (name, operation) in [
            ("agent_evidence_edges_endpoints_insert", "INSERT"),
            (
                "agent_evidence_edges_endpoints_update",
                "UPDATE OF root_run_id,revision,from_node_id,to_node_id",
            ),
        ] {
            connection.execute_batch(&format!(
                "CREATE TRIGGER {name} BEFORE {operation} ON agent_evidence_edges
                 BEGIN
                     SELECT CASE WHEN NOT EXISTS (
                         WITH RECURSIVE ancestors(revision) AS (
                             SELECT revision FROM agent_evidence_revisions
                             WHERE root_run_id=NEW.root_run_id AND revision=NEW.revision
                             UNION ALL
                             SELECT parent_revision FROM agent_evidence_revisions r
                             JOIN ancestors a ON r.root_run_id=NEW.root_run_id
                               AND r.revision=a.revision WHERE parent_revision IS NOT NULL
                         )
                         SELECT 1 FROM agent_evidence_nodes n JOIN ancestors a
                           ON n.revision=a.revision
                         WHERE n.id=NEW.from_node_id AND n.root_run_id=NEW.root_run_id
                     ) THEN RAISE(ABORT, 'evidence 边的 from_node_id 不属于本 root/祖先 revision') END;
                     SELECT CASE WHEN NOT EXISTS (
                         WITH RECURSIVE ancestors(revision) AS (
                             SELECT revision FROM agent_evidence_revisions
                             WHERE root_run_id=NEW.root_run_id AND revision=NEW.revision
                             UNION ALL
                             SELECT parent_revision FROM agent_evidence_revisions r
                             JOIN ancestors a ON r.root_run_id=NEW.root_run_id
                               AND r.revision=a.revision WHERE parent_revision IS NOT NULL
                         )
                         SELECT 1 FROM agent_evidence_nodes n JOIN ancestors a
                           ON n.revision=a.revision
                         WHERE n.id=NEW.to_node_id AND n.root_run_id=NEW.root_run_id
                     ) THEN RAISE(ABORT, 'evidence 边的 to_node_id 不属于本 root/祖先 revision') END;
                 END;"
            )).map_err(|error| format!("无法安装 evidence 端点约束：{error}"))?;
        }
        connection
            .execute_batch("COMMIT;")
            .map_err(|error| format!("无法提交 evidence revision 迁移：{error}"))
    })();
    if install.is_err() {
        let _ = connection.execute_batch("ROLLBACK;");
    }
    install
}

fn migrate_agent_run_orchestration_columns(connection: &mut Connection) -> Result<(), String> {
    // Coverage entries cite a tool invocation by the id the runtime handed out, so
    // the audit row and the coverage ledger agree (§9.2).
    ensure_column(
        &*connection,
        "tool_invocations",
        "invocation_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    // Phase 2 §3.2: the backend matrix is decided once per attempt, before any
    // dependency is resolved, and is never recomputed from settings that changed
    // while the attempt was running.
    ensure_column(
        &*connection,
        "sentinel_scan_attempts",
        "backend_plan_json",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    // Phase 2 §2.2: the frozen plan belongs to an attempt, not to a URL. `agent_runs`
    // already carries the attempt dimension, so the plan lives beside it and the old
    // per-URL checkpoint row stays a compatibility projection for the UI.
    ensure_column(
        &*connection,
        "agent_runs",
        "plan_json",
        "TEXT NOT NULL DEFAULT '{}'",
    )?;
    // Orchestration columns belong to `agent_runs` directly. Existing history stays
    // single-run unless a live Coordinator explicitly activates the multi-agent path.
    for (column, definition) in [
        ("root_run_id", "TEXT NOT NULL DEFAULT ''"),
        ("assignment_id", "TEXT NOT NULL DEFAULT ''"),
        ("lane", "TEXT NOT NULL DEFAULT ''"),
        ("orchestration_policy", "TEXT NOT NULL DEFAULT 'single'"),
        // DESIGN §0.1：任务级调度模式开关。历史行一律落在 legacy 上，
        // 只有显式写成 homogeneous_workers 的 run 才走新的主管闭环。
        ("scheduling_model", "TEXT NOT NULL DEFAULT 'legacy_role_pipeline'"),
        ("capability_lease_json", "TEXT NOT NULL DEFAULT '[]'"),
        ("reserved_tokens", "INTEGER NOT NULL DEFAULT 0"),
        ("reserved_requests", "INTEGER NOT NULL DEFAULT 0"),
        ("heartbeat_at", "TEXT NOT NULL DEFAULT ''"),
        ("cancel_requested_at", "TEXT NOT NULL DEFAULT ''"),
    ] {
        ensure_column(&*connection, "agent_runs", column, definition)?;
    }
    for (column, definition) in [
        ("root_run_id", "TEXT NOT NULL DEFAULT ''"),
        ("from_run_id", "TEXT NOT NULL DEFAULT ''"),
        ("to_run_id", "TEXT NOT NULL DEFAULT ''"),
        ("assignment_id", "TEXT NOT NULL DEFAULT ''"),
        ("evidence_revision", "INTEGER NOT NULL DEFAULT 0"),
        ("delivery_attempts", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        ensure_column(&*connection, "agent_messages", column, definition)?;
    }
    for (column, definition) in [
        ("lease_epoch", "INTEGER NOT NULL DEFAULT 0"),
        ("fencing_token", "TEXT NOT NULL DEFAULT ''"),
        ("lease_expires_at", "TEXT NOT NULL DEFAULT ''"),
        ("budget_settled_at", "TEXT NOT NULL DEFAULT ''"),
        // DESIGN §4.3：Phase 1 同质 Worker 的切片身份、调度模式与 attempt。
        // 旧行的默认值保持 legacy vocabulary，读写新列前必须先判 scheduling_model。
        ("slice_id", "TEXT NOT NULL DEFAULT ''"),
        ("scheduling_model", "TEXT NOT NULL DEFAULT 'legacy_role_pipeline'"),
        ("worker_profile_id", "TEXT NOT NULL DEFAULT ''"),
        ("homogeneous_worker_id", "TEXT NOT NULL DEFAULT ''"),
        ("attempt", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        ensure_column(&*connection, "agent_assignments", column, definition)?;
    }
    // DESIGN §4.3 的索引原写作 agent_assignments.attempt_number（现状该列不存在，
    // attempts 表用 lease_epoch 递增），因此这里按 SPEC §1.4 Assignment.attempt 建索引。
    connection
        .execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS ux_homo_slice_attempt ON agent_assignments(slice_id,attempt) WHERE scheduling_model='homogeneous_workers' AND slice_id<>'';",
            [],
        )
        .map_err(|error| format!("创建同质切片索引失败：{error}"))?;
    for (column, definition) in [
        ("source_draft_id", "TEXT NOT NULL DEFAULT ''"),
        ("confirmed_revision", "INTEGER NOT NULL DEFAULT 0"),
        ("confirmed_hash", "TEXT NOT NULL DEFAULT ''"),
        ("confirmation_at", "TEXT NOT NULL DEFAULT ''"),
        ("thread_key", "TEXT NOT NULL DEFAULT 'team'"),
    ] {
        ensure_column(&*connection, "agent_user_directives", column, definition)?;
    }
    ensure_column(&*connection, "agent_directive_drafts", "thread_key", "TEXT NOT NULL DEFAULT 'team'")?;
    connection
        .execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_agent_messages_recipient ON agent_messages(to_run_id,delivered_at,created_at);
            CREATE INDEX IF NOT EXISTS idx_agent_directives_scope ON agent_user_directives(scan_id,attempt_number,status,created_at);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_directives_source_draft ON agent_user_directives(source_draft_id) WHERE source_draft_id<>'';
            CREATE INDEX IF NOT EXISTS idx_agent_directive_drafts_scope ON agent_directive_drafts(scan_id,attempt_number,status,created_at);
            CREATE INDEX IF NOT EXISTS idx_agent_review_root ON agent_review_requests(root_run_id,status,created_at);
            CREATE INDEX IF NOT EXISTS idx_agent_capability_child ON agent_capability_leases(child_run_id,revoked_at,lease_expires_at);
            "#,
        )
        .map_err(|error| format!("创建多智能体运行索引失败：{error}"))?;
    let _ = connection.execute(
        "INSERT OR IGNORE INTO sentinel_scan_attempts(scan_id,attempt_number,status,stage,checkpoint,stop_reason,llm_requests_delta,input_tokens_delta,output_tokens_delta,cached_tokens_delta,total_tokens_delta,started_at,finished_at,updated_at) SELECT id,MAX(attempt_count,1),status,CASE WHEN status IN ('completed','partial') THEN 'complete' WHEN status IN ('failed','cancelled') THEN 'stopped' WHEN status IN ('paused','pausing') THEN 'paused' ELSE 'unknown' END,current_checkpoint,CASE WHEN status IN ('completed','partial','failed','cancelled','paused') THEN current_checkpoint ELSE '' END,llm_requests,input_tokens,output_tokens,cached_tokens,total_tokens,created_at,CASE WHEN status IN ('completed','partial','failed','cancelled','paused') THEN updated_at ELSE '' END,updated_at FROM sentinel_scans WHERE attempt_count>0",
        [],
    );
    let old_process_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='sentinel_processes'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if old_process_schema.contains("scan_id TEXT PRIMARY KEY") {
        connection
            .execute_batch(
                r#"
                ALTER TABLE sentinel_processes RENAME TO sentinel_processes_legacy;
                CREATE TABLE sentinel_processes (
                    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
                    process_id INTEGER NOT NULL DEFAULT 0,
                    engine TEXT NOT NULL DEFAULT '',
                    work_dir TEXT NOT NULL DEFAULT '',
                    started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                    PRIMARY KEY(scan_id,process_id)
                );
                INSERT OR IGNORE INTO sentinel_processes(scan_id,process_id,engine,work_dir,started_at)
                  SELECT scan_id,process_id,engine,work_dir,started_at FROM sentinel_processes_legacy;
                DROP TABLE sentinel_processes_legacy;
                "#,
            )
            .map_err(|error| format!("升级扫描进程表失败：{error}"))?;
    }
    for statement in [
        "ALTER TABLE security_rule_packs ADD COLUMN added_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN modified_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN deleted_count INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE security_rule_packs ADD COLUMN change_summary TEXT NOT NULL DEFAULT '[]'",
        "ALTER TABLE security_rule_packs ADD COLUMN previous_version TEXT NOT NULL DEFAULT ''",
    ] {
        let _ = connection.execute(statement, []);
    }
    connection.execute_batch(
        r#"
        INSERT OR IGNORE INTO security_rule_packs(key,name,engine,repository,reference,enabled,builtin)
        VALUES
          ('semgrep-rules','Semgrep Rules','semgrep','https://github.com/semgrep/semgrep-rules.git','develop',1,1),
          ('codeql-queries','CodeQL queries','codeql','https://github.com/github/codeql.git','main',1,1),
          ('owasp-benchmark','OWASP Benchmark','benchmark','https://github.com/OWASP/Benchmark.git','master',1,1);
        "#,
    ).map_err(|error| error.to_string())?;
    Ok(())
}

/// New reviewer decisions use `insufficient_evidence`. The retired
/// `needs_evidence` spelling remains readable for existing rows and imported
/// history, but upgraded databases must accept the canonical value for new
/// review requests and finding candidates.
fn migrate_agent_review_verdict_vocabulary(connection: &mut Connection) -> Result<(), String> {
    let review_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_review_requests'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if !review_schema.contains("insufficient_evidence") {
        connection
            .execute_batch(
                r#"
                ALTER TABLE agent_review_requests RENAME TO agent_review_requests_legacy_verdict;
                CREATE TABLE agent_review_requests (
                    id TEXT PRIMARY KEY,
                    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
                    assignment_id TEXT NOT NULL REFERENCES agent_assignments(id) ON DELETE CASCADE,
                    reviewer_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
                    candidate_id TEXT NOT NULL,
                    candidate_revision INTEGER NOT NULL,
                    candidate_json TEXT NOT NULL DEFAULT '{}',
                    status TEXT NOT NULL DEFAULT 'pending'
                      CHECK(status IN ('pending','running','confirmed','rejected','insufficient_evidence','needs_evidence','failed','superseded')),
                    decision_id INTEGER,
                    lease_epoch INTEGER NOT NULL,
                    fencing_token TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                    finished_at TEXT NOT NULL DEFAULT '',
                    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                    UNIQUE(candidate_id,candidate_revision)
                );
                INSERT INTO agent_review_requests(
                    id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,
                    candidate_json,status,decision_id,lease_epoch,fencing_token,created_at,finished_at,updated_at
                ) SELECT
                    id,root_run_id,assignment_id,reviewer_run_id,candidate_id,candidate_revision,
                    candidate_json,status,decision_id,lease_epoch,fencing_token,created_at,finished_at,updated_at
                  FROM agent_review_requests_legacy_verdict;
                DROP TABLE agent_review_requests_legacy_verdict;
                CREATE INDEX IF NOT EXISTS idx_agent_review_root
                  ON agent_review_requests(root_run_id,status,created_at);
                "#,
            )
            .map_err(|error| format!("升级 Reviewer verdict 词汇失败：{error}"))?;
    }

    let candidate_schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='agent_finding_candidates'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();
    if !candidate_schema.contains("insufficient_evidence") {
        connection
            .execute_batch(
                r#"
                ALTER TABLE agent_finding_candidates RENAME TO agent_finding_candidates_legacy_verdict;
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
                      CHECK(status IN ('pending','published','rejected','insufficient_evidence','needs_evidence','superseded')),
                    reviewer_run_id TEXT NOT NULL DEFAULT '',
                    published_at TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
                    UNIQUE(root_run_id,scan_id,target_url,stage,kind,record_key)
                );
                INSERT INTO agent_finding_candidates(
                    id,root_run_id,candidate_revision,scan_id,target_url,stage,kind,record_key,title,
                    severity,record_json,status,reviewer_run_id,published_at,created_at,updated_at
                ) SELECT
                    id,root_run_id,candidate_revision,scan_id,target_url,stage,kind,record_key,title,
                    severity,record_json,status,reviewer_run_id,published_at,created_at,updated_at
                  FROM agent_finding_candidates_legacy_verdict;
                DROP TABLE agent_finding_candidates_legacy_verdict;
                CREATE INDEX IF NOT EXISTS idx_agent_finding_candidates_review
                  ON agent_finding_candidates(root_run_id,candidate_revision,status);
                CREATE INDEX IF NOT EXISTS idx_agent_finding_candidates_projection
                  ON agent_finding_candidates(scan_id,target_url,stage,kind,record_key,status);
                "#,
            )
            .map_err(|error| format!("升级 finding candidate verdict 词汇失败：{error}"))?;
    }
    // Older review rows remain readable history, but an unsealed row cannot
    // be replayed or published as a new decision after this migration.
    ensure_column(connection, "agent_review_requests", "evidence_revision", "INTEGER NOT NULL DEFAULT 0")?;
    ensure_column(connection, "agent_review_requests", "manifest_hash", "TEXT NOT NULL DEFAULT ''")?;
    connection.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_agent_finding_candidates_projection \
         ON agent_finding_candidates(scan_id,target_url,stage,kind,record_key,status)",
    ).map_err(|error| format!("创建 finding provenance 索引失败：{error}"))?;
    Ok(())
}

/// Only confirmed protection-system blocks become persistent fuse entries; budget, context and no-progress stops remain retryable checkpoints.
fn migrate_fuse_entry_status(connection: &mut Connection) -> Result<(), String> {
    // Only confirmed protection-system blocks become persistent fuse entries.
    // Budget, context and no-progress stops remain retryable checkpoints.
    connection.execute(
        "INSERT OR IGNORE INTO sentinel_fuse_zone(project_id,asset_id,company,url,normalized_url,source_scan_id,reason) SELECT project_id,asset_id,company,url,lower(rtrim(trim(url),'/')),COALESCE(scan_id,''),routing_reason FROM sentinel_targets WHERE status='limited' AND trim(url)<>'' AND (lower(routing_reason) LIKE '%waf%' OR lower(routing_reason) LIKE '%captcha%' OR lower(routing_reason) LIKE '%cloudflare%' OR lower(routing_reason) LIKE '%rate limit%' OR routing_reason LIKE '%验证码%' OR routing_reason LIKE '%人机验证%' OR routing_reason LIKE '%持续限流%')",
        [],
    ).map_err(|error| error.to_string())?;
    if migration_version(&*connection, "soft_fuse_cleanup_version") < 1 {
        connection.execute(
            "UPDATE sentinel_fuse_zone SET archived=1,note='旧版将预算、上下文或无进展软暂停误记为熔断；现已恢复为可继续任务',updated_at=datetime('now','localtime') WHERE archived=0 AND verdict='pending' AND trim(evidence)='' AND NOT (lower(reason) LIKE '%waf%' OR lower(reason) LIKE '%captcha%' OR lower(reason) LIKE '%cloudflare%' OR lower(reason) LIKE '%rate limit%' OR reason LIKE '%验证码%' OR reason LIKE '%人机验证%' OR reason LIKE '%持续限流%')",
            [],
        ).map_err(|error| error.to_string())?;
        finish_migration(&*connection, "soft_fuse_cleanup_version", 1)?;
    }
    Ok(())
}
