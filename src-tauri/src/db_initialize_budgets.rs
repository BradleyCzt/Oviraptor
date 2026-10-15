// Launch migrations over the asset table and the stored budget defaults. Included from
// db_initialize.rs.

/// One-time repair over the complete asset table: duplicate assets, and the probe classification version that decides which findings are re-derived.
fn repair_asset_duplicates_and_probe_labels(connection: &mut Connection) -> Result<(), String> {
    // This is a one-time repair over the complete asset table. Running the
    // GROUP BY on every launch becomes noticeable once the DB reaches hundreds
    // of MB; normal imports already enforce canonical-key deduplication.
    let dedupe_migration: i64 = connection
        .query_row(
            "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM app_settings WHERE key='canonical_dedupe_migration'),0)",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if dedupe_migration < 1 {
        let deduplicated = deduplicate_project_assets(&mut *connection)?;
        connection
            .execute(
                "INSERT INTO app_settings(key,value) VALUES('canonical_dedupe_migration','1') ON CONFLICT(key) DO UPDATE SET value='1'",
                [],
            )
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO app_settings(key,value) VALUES('last_deduplicated',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [deduplicated.to_string()],
            )
            .map_err(|error| error.to_string())?;
    } else {
        connection
            .execute(
                "INSERT OR IGNORE INTO app_settings(key,value) VALUES('last_deduplicated','0')",
                [],
            )
            .map_err(|error| error.to_string())?;
    }
    // 0.5.4 将旧版 alive_clean 中混入的 TCP 与异常 HTTP 结果拆开。
    // 只使用已保存的探测证据迁移分类；之后的“复测现有资产”会刷新实时状态。
    let probe_classification_version: i64 = connection.query_row(
        "SELECT COALESCE((SELECT CAST(value AS INTEGER) FROM app_settings WHERE key='probe_classification_version'),0)",
        [], |row| row.get(0),
    ).unwrap_or(0);
    if probe_classification_version < 3 {
        connection.execute_batch(r#"
        UPDATE assets SET probe_outcome=CASE
          WHEN probe_entry_state='tcp_alive_non_http' THEN 'tcp_alive_non_http'
          WHEN CAST(status_code AS INTEGER) IN (401,403,407,429) THEN 'web_restricted'
          WHEN probe_entry_state IN ('reachable_but_path_missing','reachable_client_error','reachable_server_error','reachable_other','empty_response') THEN 'web_abnormal'
          WHEN CAST(status_code AS INTEGER) BETWEEN 200 AND 399 THEN 'web_alive'
          ELSE probe_outcome END
        WHERE probe_outcome='alive_clean';
        UPDATE project_assets SET decision='not_applicable',note='系统自动Web分类：当前不进入浏览器人工队列'
        WHERE decision IN ('pending','uncertain','') AND is_deleted=0
          AND (note='' OR note LIKE '系统自动%')
          AND NOT EXISTS(SELECT 1 FROM asset_events e WHERE e.project_id=project_assets.project_id AND e.asset_id=project_assets.asset_id AND e.event_type IN ('decision','archived','restored'))
          AND NOT EXISTS(SELECT 1 FROM asset_ownership_decisions o WHERE o.project_id=project_assets.project_id AND o.asset_id=project_assets.asset_id AND (o.source<>'auto' OR o.exposure_eligible=1))
          AND NOT EXISTS(SELECT 1 FROM sentinel_targets st JOIN assets a ON a.id=project_assets.asset_id WHERE st.project_id=project_assets.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host))))
          AND asset_id IN (
          SELECT id FROM assets WHERE probe_outcome IN ('tcp_alive_non_http','web_abnormal','unreachable','skipped')
        );
        UPDATE project_assets SET decision='rejected',note='系统自动Web分类：违规内容隔离'
        WHERE decision IN ('pending','uncertain','') AND is_deleted=0
          AND (note='' OR note LIKE '系统自动%')
          AND NOT EXISTS(SELECT 1 FROM asset_events e WHERE e.project_id=project_assets.project_id AND e.asset_id=project_assets.asset_id AND e.event_type IN ('decision','archived','restored'))
          AND NOT EXISTS(SELECT 1 FROM asset_ownership_decisions o WHERE o.project_id=project_assets.project_id AND o.asset_id=project_assets.asset_id AND (o.source<>'auto' OR o.exposure_eligible=1))
          AND NOT EXISTS(SELECT 1 FROM sentinel_targets st JOIN assets a ON a.id=project_assets.asset_id WHERE st.project_id=project_assets.project_id AND (st.asset_id=a.id OR (st.asset_id IS NULL AND st.url=COALESCE(NULLIF(a.link,''),a.host))))
          AND asset_id IN (
          SELECT id FROM assets WHERE probe_outcome='blocked_content'
        );
        INSERT INTO app_settings(key,value) VALUES('probe_classification_version','3')
          ON CONFLICT(key) DO UPDATE SET value='3';
        "#).map_err(|error| error.to_string())?;
    }
    let _ = connection.execute(
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.fofaEmail','') WHERE json_valid(settings_json) AND json_type(settings_json,'$.fofaEmail') IS NULL",
        [],
    );
    let _ = connection.execute(
        "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.fofaKey','') WHERE json_valid(settings_json) AND json_type(settings_json,'$.fofaKey') IS NULL",
        [],
    );
    for (key, value) in [
        ("hackerOneUsername", ""),
        ("hackerOneToken", ""),
        ("proxyUrl", ""),
        ("noProxy", "127.0.0.1,localhost"),
    ] {
        let sql = format!(
            "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.{key}',?1) WHERE json_valid(settings_json) AND json_type(settings_json,'$.{key}') IS NULL"
        );
        let _ = connection.execute(&sql, [value]);
    }

    // A scan is complete when its bounded queue is exhausted, even when every
    // target legitimately ends at deterministic reconnaissance. Target rows
    // still preserve `recon_only`; the task-level status represents lifecycle
    // completion and must not look like an interruption requiring a retry.
    let _ = connection.execute_batch(
        r#"
        UPDATE sentinel_scans
        SET status='completed',
            current_checkpoint=CASE
              WHEN trim(current_checkpoint)='' THEN '扫描完成：目标均由确定性侦察正常收口，未发生异常中断'
              ELSE current_checkpoint
            END,
            updated_at=datetime('now','localtime')
        WHERE scan_type='web'
          AND status='recon_only'
          AND EXISTS(
            SELECT 1 FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id
          )
          AND NOT EXISTS(
            SELECT 1 FROM sentinel_targets t
            WHERE t.scan_id=sentinel_scans.id AND t.status NOT IN ('recon_only','manual_review')
          );
        "#,
    );

    // Older investigation-gate runs used `partial` for a deterministic
    // no-high-value stop. That state is not a pause or a failed Agent run:
    // the local evidence is complete and the target is recon-only. Repair the
    // persisted classification once so the queue and resume UI are truthful.
    let _ = connection.execute_batch(
        r#"
        UPDATE sentinel_targets
        SET status='recon_only',
            scan_mode=CASE WHEN scan_mode IN ('', 'quick', 'standard', 'deep') THEN 'skip' ELSE scan_mode END,
            updated_at=datetime('now','localtime')
        WHERE status='partial'
          AND routing_reason LIKE '%no_high_value_hypothesis%'
          AND NOT EXISTS (
            SELECT 1 FROM investigation_metrics im
            WHERE im.scan_id=sentinel_targets.scan_id
              AND im.target_url=sentinel_targets.url
              AND COALESCE(json_extract(im.decision_json,'$.baselineInvestigationAllowed'),0)=1
          );

        UPDATE sentinel_scans
        SET status='recon_only',
            current_checkpoint=CASE
              WHEN trim(current_checkpoint)='' OR current_checkpoint LIKE '%流水线%' THEN '本地前端调查完成：没有达到高价值假设门禁；已保留全部证据'
              ELSE current_checkpoint
            END,
            updated_at=datetime('now','localtime')
        WHERE status='partial'
          AND scan_type='web'
          AND EXISTS(SELECT 1 FROM sentinel_targets t WHERE t.scan_id=sentinel_scans.id)
          AND NOT EXISTS(
            SELECT 1 FROM sentinel_targets t
            WHERE t.scan_id=sentinel_scans.id
              AND t.status NOT IN ('recon_only','manual_review')
          )
          AND EXISTS(
            SELECT 1 FROM sentinel_targets t
            WHERE t.scan_id=sentinel_scans.id
              AND t.routing_reason LIKE '%no_high_value_hypothesis%'
          );
        "#,
    );

    // MLX owns local-model context and generation limits. Remove the retired
    // per-profile overrides so old profiles cannot silently reintroduce them.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles
        SET settings_json=json_set(
            settings_json,
            '$.modelProfiles',
            json(COALESCE((
                SELECT json_group_array(json_remove(value,'$.contextWindow','$.maxOutputTokens'))
                FROM json_each(settings_json,'$.modelProfiles')
            ),'[]'))
        )
        WHERE json_valid(settings_json)
          AND json_type(settings_json,'$.modelProfiles')='array';
        "#,
    );

    // Version 5 adds an explicit frontend packet budget so low-context local
    // models do not receive the old multi-file 40KB+ evidence packet.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.agentFrontendPacketMode',COALESCE(json_extract(settings_json,'$.agentFrontendPacketMode'),'balanced'),
            '$.agentFrontendPacketBudgetKb',CASE
              WHEN json_type(settings_json,'$.agentFrontendPacketBudgetKb') IN ('integer','real')
                THEN MIN(MAX(CAST(json_extract(settings_json,'$.agentFrontendPacketBudgetKb') AS INTEGER),4),64)
              ELSE 12
            END
        ) WHERE json_valid(settings_json);
        "#,
    );
    // Version 3 repairs the exact policy signature written by the old
    // "local full power" UI watcher. That watcher permanently replaced the
    // user's governed-mode values with max/zero settings even after the
    // switch was turned off.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.agentBatchSize',15,
            '$.agentQuickScore',30,
            '$.agentStandardScore',55,
            '$.agentDeepScore',80,
            '$.agentQuickTimeout',120,
            '$.agentStandardTimeout',300,
            '$.agentDeepTimeout',600,
            '$.agentQuickTokenLimit',50000,
            '$.agentStandardTokenLimit',120000,
            '$.agentDeepTokenLimit',250000,
            '$.agentQuickRequestLimit',4,
            '$.agentStandardRequestLimit',8,
            '$.agentDeepRequestLimit',12,
            '$.agentNoToolTurnLimit',2,
            '$.agentBudgetPolicyVersion',3
        )
        WHERE json_valid(settings_json)
          AND COALESCE(json_extract(settings_json,'$.agentBudgetPolicyVersion'),0)<3
          AND json_extract(settings_json,'$.agentQuickScore')=1
          AND json_extract(settings_json,'$.agentStandardScore')=2
          AND json_extract(settings_json,'$.agentDeepScore')=3
          AND json_extract(settings_json,'$.agentQuickTimeout')=3600
          AND json_extract(settings_json,'$.agentStandardTimeout')=7200
          AND json_extract(settings_json,'$.agentDeepTimeout')=14400
          AND json_extract(settings_json,'$.agentQuickTokenLimit')=0
          AND json_extract(settings_json,'$.agentStandardTokenLimit')=0
          AND json_extract(settings_json,'$.agentDeepTokenLimit')=0
          AND json_extract(settings_json,'$.agentQuickRequestLimit')=100
          AND json_extract(settings_json,'$.agentStandardRequestLimit')=200
          AND json_extract(settings_json,'$.agentDeepRequestLimit')=300
          AND json_extract(settings_json,'$.agentNoToolTurnLimit')=100;
        UPDATE config_profiles
          SET settings_json=json_set(settings_json,'$.agentBudgetPolicyVersion',3)
          WHERE json_valid(settings_json)
            AND COALESCE(json_extract(settings_json,'$.agentBudgetPolicyVersion'),0)<3;
        "#,
    );
    // Adaptive Agent limits are migrated field-by-field so an explicit 0
    // remains a user-selected disabled uncached-token budget.
    for (key, value) in [
        ("agentQuickTimeout", "120"),
        ("agentStandardTimeout", "300"),
        ("agentDeepTimeout", "600"),
        ("agentQuickTokenLimit", "50000"),
        ("agentStandardTokenLimit", "120000"),
        ("agentDeepTokenLimit", "250000"),
        ("agentQuickRequestLimit", "4"),
        ("agentStandardRequestLimit", "8"),
        ("agentDeepRequestLimit", "12"),
        ("agentNoToolTurnLimit", "4"),
    ] {
        let sql = format!(
            "UPDATE config_profiles SET settings_json=json_set(settings_json,'$.{key}',json(?1)) WHERE json_valid(settings_json) AND json_type(settings_json,'$.{key}') IS NULL"
        );
        let _ = connection.execute(&sql, [value]);
    }
    Ok(())
}

/// Forward-compatible field-level migration of the stored budget defaults.
fn migrate_budget_defaults(connection: &mut Connection) -> Result<(), String> {
    // Version 2 replaces the legacy high-token defaults. Explicit numeric
    // uncached-token limits, including 0 (disabled layer), remain user-controlled.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentQuickTokenLimit',50000)
          WHERE json_valid(settings_json) AND COALESCE(json_extract(settings_json,'$.agentBudgetPolicyVersion'),0)<2
            AND (json_type(settings_json,'$.agentQuickTokenLimit') NOT IN ('integer','real') OR json_extract(settings_json,'$.agentQuickTokenLimit')=100000);
        UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentStandardTokenLimit',120000)
          WHERE json_valid(settings_json) AND COALESCE(json_extract(settings_json,'$.agentBudgetPolicyVersion'),0)<2
            AND (json_type(settings_json,'$.agentStandardTokenLimit') NOT IN ('integer','real') OR json_extract(settings_json,'$.agentStandardTokenLimit')=250000);
        UPDATE config_profiles SET settings_json=json_set(settings_json,'$.agentDeepTokenLimit',250000)
          WHERE json_valid(settings_json) AND COALESCE(json_extract(settings_json,'$.agentBudgetPolicyVersion'),0)<2
            AND (json_type(settings_json,'$.agentDeepTokenLimit') NOT IN ('integer','real') OR json_extract(settings_json,'$.agentDeepTokenLimit')=500000);
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.agentQuickTimeout',MIN(COALESCE(CAST(json_extract(settings_json,'$.agentQuickTimeout') AS INTEGER),120),120),
            '$.agentStandardTimeout',MIN(COALESCE(CAST(json_extract(settings_json,'$.agentStandardTimeout') AS INTEGER),300),300),
            '$.agentDeepTimeout',MIN(COALESCE(CAST(json_extract(settings_json,'$.agentDeepTimeout') AS INTEGER),600),600),
            '$.agentQuickRequestLimit',MIN(COALESCE(CAST(json_extract(settings_json,'$.agentQuickRequestLimit') AS INTEGER),4),4),
            '$.agentStandardRequestLimit',MIN(COALESCE(CAST(json_extract(settings_json,'$.agentStandardRequestLimit') AS INTEGER),8),8),
            '$.agentDeepRequestLimit',MIN(COALESCE(CAST(json_extract(settings_json,'$.agentDeepRequestLimit') AS INTEGER),12),12),
            '$.agentNoToolTurnLimit',MIN(COALESCE(CAST(json_extract(settings_json,'$.agentNoToolTurnLimit') AS INTEGER),2),2),
            '$.agentBudgetPolicyVersion',2
        ) WHERE json_valid(settings_json) AND COALESCE(json_extract(settings_json,'$.agentBudgetPolicyVersion'),0)<2;
        "#,
    );
    // Version 4 raises the cloud no-progress default from two to four model
    // turns. Local full-power scans bypass this fuse at runtime.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.agentNoToolTurnLimit',CASE
              WHEN json_extract(settings_json,'$.agentNoToolTurnLimit')=2 THEN 4
              ELSE json_extract(settings_json,'$.agentNoToolTurnLimit')
            END,
            '$.agentBudgetPolicyVersion',4
        )
        WHERE json_valid(settings_json)
          AND COALESCE(json_extract(settings_json,'$.agentBudgetPolicyVersion'),0)<4;
        "#,
    );
    // Version 5 gives cloud models enough time and request budget to consume
    // the richer browser/AST evidence. Local deployments keep the former
    // conservative values at runtime until their policy is tuned separately.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.agentFrontendPacketBudgetKb',CASE WHEN json_extract(settings_json,'$.agentFrontendPacketBudgetKb')=12 THEN 24 ELSE json_extract(settings_json,'$.agentFrontendPacketBudgetKb') END,
            '$.agentQuickTimeout',CASE WHEN json_extract(settings_json,'$.agentQuickTimeout')=120 THEN 240 ELSE json_extract(settings_json,'$.agentQuickTimeout') END,
            '$.agentStandardTimeout',CASE WHEN json_extract(settings_json,'$.agentStandardTimeout')=300 THEN 600 ELSE json_extract(settings_json,'$.agentStandardTimeout') END,
            '$.agentDeepTimeout',CASE WHEN json_extract(settings_json,'$.agentDeepTimeout')=600 THEN 1200 ELSE json_extract(settings_json,'$.agentDeepTimeout') END,
            '$.agentQuickTokenLimit',CASE WHEN json_extract(settings_json,'$.agentQuickTokenLimit')=50000 THEN 100000 ELSE json_extract(settings_json,'$.agentQuickTokenLimit') END,
            '$.agentStandardTokenLimit',CASE WHEN json_extract(settings_json,'$.agentStandardTokenLimit')=120000 THEN 300000 ELSE json_extract(settings_json,'$.agentStandardTokenLimit') END,
            '$.agentDeepTokenLimit',CASE WHEN json_extract(settings_json,'$.agentDeepTokenLimit')=250000 THEN 700000 ELSE json_extract(settings_json,'$.agentDeepTokenLimit') END,
            '$.agentQuickRequestLimit',CASE WHEN json_extract(settings_json,'$.agentQuickRequestLimit')=4 THEN 6 ELSE json_extract(settings_json,'$.agentQuickRequestLimit') END,
            '$.agentStandardRequestLimit',CASE WHEN json_extract(settings_json,'$.agentStandardRequestLimit')=8 THEN 14 ELSE json_extract(settings_json,'$.agentStandardRequestLimit') END,
            '$.agentDeepRequestLimit',CASE WHEN json_extract(settings_json,'$.agentDeepRequestLimit')=12 THEN 24 ELSE json_extract(settings_json,'$.agentDeepRequestLimit') END,
            '$.agentNoToolTurnLimit',CASE WHEN json_extract(settings_json,'$.agentNoToolTurnLimit')=4 THEN 6 ELSE json_extract(settings_json,'$.agentNoToolTurnLimit') END,
            '$.agentBudgetPolicyVersion',5
        )
        WHERE json_valid(settings_json)
          AND COALESCE(json_extract(settings_json,'$.agentBudgetPolicyVersion'),0)<5;
        "#,
    );
    // Version 6 removes the accidental local/frontend 50k effective ceiling.
    // Only migrate the shipped defaults; explicit user budgets (including 0)
    // remain untouched.
    let _ = connection.execute_batch(
        r#"
        UPDATE config_profiles SET settings_json=json_set(
            settings_json,
            '$.agentQuickTimeout',CASE WHEN json_extract(settings_json,'$.agentQuickTimeout')=240 THEN 300 ELSE json_extract(settings_json,'$.agentQuickTimeout') END,
            '$.agentStandardTimeout',CASE WHEN json_extract(settings_json,'$.agentStandardTimeout')=600 THEN 480 ELSE json_extract(settings_json,'$.agentStandardTimeout') END,
            '$.agentDeepTimeout',CASE WHEN json_extract(settings_json,'$.agentDeepTimeout')=1200 THEN 900 ELSE json_extract(settings_json,'$.agentDeepTimeout') END,
            '$.agentQuickTokenLimit',CASE WHEN json_extract(settings_json,'$.agentQuickTokenLimit') IN (50000,100000) THEN 200000 ELSE json_extract(settings_json,'$.agentQuickTokenLimit') END,
            '$.agentStandardTokenLimit',CASE WHEN json_extract(settings_json,'$.agentStandardTokenLimit')=300000 THEN 400000 ELSE json_extract(settings_json,'$.agentStandardTokenLimit') END,
            '$.agentDeepTokenLimit',CASE WHEN json_extract(settings_json,'$.agentDeepTokenLimit')=700000 THEN 800000 ELSE json_extract(settings_json,'$.agentDeepTokenLimit') END,
            '$.agentQuickRequestLimit',CASE WHEN json_extract(settings_json,'$.agentQuickRequestLimit')=6 THEN 8 ELSE json_extract(settings_json,'$.agentQuickRequestLimit') END,
            '$.agentStandardRequestLimit',CASE WHEN json_extract(settings_json,'$.agentStandardRequestLimit')=14 THEN 12 ELSE json_extract(settings_json,'$.agentStandardRequestLimit') END,
            '$.agentDeepRequestLimit',CASE WHEN json_extract(settings_json,'$.agentDeepRequestLimit')=24 THEN 16 ELSE json_extract(settings_json,'$.agentDeepRequestLimit') END,
            '$.agentBudgetPolicyVersion',6
        )
        WHERE json_valid(settings_json)
          AND COALESCE(json_extract(settings_json,'$.agentBudgetPolicyVersion'),0)<6;
        "#,
    );

    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM config_profiles", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if count == 0 {
        let defaults = json!({
            "modelProfiles": [],
            "activeModelProfileId": "",
            "modelDeployment": "cloud",
            "modelApiBase": "",
            "modelApiKey": "",
            "localApiKey": "",
            "agentLocalFullPower": false,
            "agentPromptAuditMode": "off",
            "agentFrontendPacketMode": "balanced",
            "agentFrontendPacketBudgetKb": 24,
            "agentBatchSize": 15,
            "agentQuickScore": 30,
            "agentStandardScore": 55,
            "agentDeepScore": 80,
            "agentQuickTimeout": 300,
            "agentStandardTimeout": 480,
            "agentDeepTimeout": 900,
            "agentQuickTokenLimit": 200000,
            "agentStandardTokenLimit": 400000,
            "agentDeepTokenLimit": 800000,
            "agentQuickRequestLimit": 8,
            "agentStandardRequestLimit": 12,
            "agentDeepRequestLimit": 16,
            "agentNoToolTurnLimit": 6,
            "agentBudgetPolicyVersion": 6,
            "agentProxyEnabled": false,
            "authorizedProxyPool": [],
            "fofaEmail": "",
            "fofaKey": "",
            "hackerOneUsername": "",
            "hackerOneToken": "",
            "proxyUrl": "",
            "noProxy": "127.0.0.1,localhost",
            "scriptsDirectory": "",
            "configPath": "",
            "collectionMode": "all",
            "fofaProfile": "professional",
            "pageSize": 500,
            "maxPages": 0,
            "interval": 6.0,
            "collectionTimeout": 45,
            "fullHistory": false,
            "enableCidr24": false,
            "includeWeakFingerprints": false,
            "runRefine": true,
            "runProbe": true,
            "includeOther": true,
            "includeWeak": false,
            "priorityRate": 20.0,
            "otherRate": 10.0,
            "workers": 96,
            "probeTimeout": 6,
            "probeTotalTimeout": 8,
            "probeRetries": 0,
            "contentThreshold": 12,
            "gamblingKeywords": ["在线赌博", "博彩平台", "真人视讯", "体育投注"],
            "pornKeywords": ["色情网站", "成人网站", "成人视频", "情色直播"],
            "negativeKeywords": ["打击赌博", "扫黄打非", "反诈", "公安", "法院"],
            "replaceDefaultContentRules": false
        });
        connection.execute(
            "INSERT INTO config_profiles(name, description, is_default, settings_json) VALUES(?1, ?2, 1, ?3)",
            ("默认配置", "安全、完整度与速度平衡的默认配置", defaults.to_string()),
        ).map_err(|error| error.to_string())?;
    }
    Ok(())
}
