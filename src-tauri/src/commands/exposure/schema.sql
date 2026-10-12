-- §Q3/Q4/Q5/Q6 — Asset 暴露面的持久结构。全部是新增表，不改任何历史表；
-- 由 `state::ensure_exposure_schema` 幂等建表，主控审查后可把同样内容并入 db_schema.rs。
-- 生产采集/恢复只依赖这些表和现有 exposure_runs/exposure_source_results/exposure_findings。

CREATE TABLE IF NOT EXISTS exposure_run_plans (
    run_id INTEGER PRIMARY KEY REFERENCES exposure_runs(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    scope_version INTEGER NOT NULL DEFAULT 1,
    scope_hash TEXT NOT NULL DEFAULT '',
    options_hash TEXT NOT NULL DEFAULT '',
    sources_json TEXT NOT NULL DEFAULT '[]',
    budget_requests INTEGER NOT NULL DEFAULT 0,
    budget_bytes INTEGER NOT NULL DEFAULT 0,
    requests_used INTEGER NOT NULL DEFAULT 0,
    bytes_used INTEGER NOT NULL DEFAULT 0,
    planned_items INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS exposure_work_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES exposure_runs(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    work_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    source_key TEXT NOT NULL DEFAULT '',
    asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    origin TEXT NOT NULL DEFAULT '',
    entity TEXT NOT NULL DEFAULT '',
    input_hash TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'pending',
    reason TEXT NOT NULL DEFAULT '',
    owner TEXT NOT NULL DEFAULT '',
    claim_version INTEGER NOT NULL DEFAULT 0,
    claimed_at TEXT NOT NULL DEFAULT '',
    heartbeat_at TEXT NOT NULL DEFAULT '',
    cursor TEXT NOT NULL DEFAULT '',
    attempts INTEGER NOT NULL DEFAULT 0,
    observations INTEGER NOT NULL DEFAULT 0,
    requests INTEGER NOT NULL DEFAULT 0,
    bytes INTEGER NOT NULL DEFAULT 0,
    finished_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(run_id, work_key)
);
CREATE INDEX IF NOT EXISTS idx_exposure_work_items_claim ON exposure_work_items(run_id, status, work_key);
CREATE INDEX IF NOT EXISTS idx_exposure_work_items_project ON exposure_work_items(project_id, run_id, status);

-- 不可变观察：某一轮某一来源到底看见了什么，只能追加，不能改写。
CREATE TABLE IF NOT EXISTS exposure_observations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES exposure_runs(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    work_item_id INTEGER REFERENCES exposure_work_items(id) ON DELETE SET NULL,
    source_key TEXT NOT NULL,
    asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    kind TEXT NOT NULL,
    url TEXT NOT NULL,
    final_url TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL DEFAULT 'GET',
    status_code INTEGER NOT NULL DEFAULT 0,
    content_type TEXT NOT NULL DEFAULT '',
    content_hash TEXT NOT NULL DEFAULT '',
    headers_hash TEXT NOT NULL DEFAULT '',
    body_bytes INTEGER NOT NULL DEFAULT 0,
    truncated INTEGER NOT NULL DEFAULT 0,
    rule_version TEXT NOT NULL DEFAULT '',
    redaction_version TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    boundary TEXT NOT NULL DEFAULT '',
    current_accessible INTEGER NOT NULL DEFAULT -1,
    indexed_at TEXT NOT NULL DEFAULT '',
    queried_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    canonical_url TEXT NOT NULL DEFAULT '',
    root_domain TEXT NOT NULL DEFAULT '',
    UNIQUE(run_id, source_key, kind, url)
);
CREATE INDEX IF NOT EXISTS idx_exposure_observations_lookup ON exposure_observations(project_id, kind, url, queried_at);

-- 冻结的授权收据：本轮到底按哪个范围发过请求，撤销/改版本后要能自证。
CREATE TABLE IF NOT EXISTS exposure_scope_receipts (
    run_id INTEGER PRIMARY KEY REFERENCES exposure_runs(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    scope_version INTEGER NOT NULL DEFAULT 1,
    scope_hash TEXT NOT NULL DEFAULT '',
    scope_json TEXT NOT NULL DEFAULT '{}',
    coarse_legacy INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- 线索与观察的引用关系：exposure_findings 保持原形状（不加列、不改历史），
-- “这一轮看见了什么”由这张表回答。
CREATE TABLE IF NOT EXISTS exposure_finding_observations (
    finding_id INTEGER NOT NULL REFERENCES exposure_findings(id) ON DELETE CASCADE,
    observation_id INTEGER NOT NULL REFERENCES exposure_observations(id) ON DELETE CASCADE,
    run_id INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(finding_id, observation_id)
);
CREATE INDEX IF NOT EXISTS idx_exposure_finding_observations_run ON exposure_finding_observations(run_id, finding_id);

-- 内容与规则变化产生的版本；旧人工结论保留在 review 事件里。
CREATE TABLE IF NOT EXISTS exposure_finding_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    finding_id INTEGER NOT NULL REFERENCES exposure_findings(id) ON DELETE CASCADE,
    run_id INTEGER NOT NULL DEFAULT 0,
    revision INTEGER NOT NULL,
    content_hash TEXT NOT NULL DEFAULT '',
    rule_version TEXT NOT NULL DEFAULT '',
    redaction_version TEXT NOT NULL DEFAULT '',
    evidence_excerpt TEXT NOT NULL DEFAULT '',
    severity TEXT NOT NULL DEFAULT '',
    confidence INTEGER NOT NULL DEFAULT 0,
    manual_status TEXT NOT NULL DEFAULT '',
    review_needed INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(finding_id, revision)
);
-- FIX2：按轮次找“内容/规则变化”的线索版本，不扫全表。
CREATE INDEX IF NOT EXISTS idx_exposure_finding_versions_run ON exposure_finding_versions(run_id, finding_id);

CREATE TABLE IF NOT EXISTS exposure_review_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    finding_id INTEGER NOT NULL REFERENCES exposure_findings(id) ON DELETE CASCADE,
    from_status TEXT NOT NULL DEFAULT '',
    to_status TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    actor TEXT NOT NULL DEFAULT 'user',
    expected_revision INTEGER NOT NULL DEFAULT 0,
    references_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- §Q6 持久簇：证书/名称聚合结果落成可继承的图，不代表归属也不代表授权。
-- §E3 生产簇还记录“代表是哪一次观察、哪一轮、按哪个配置版本”，继承必须能追溯到依据。
CREATE TABLE IF NOT EXISTS exposure_clusters (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    cluster_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    representative_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    rule_version TEXT NOT NULL DEFAULT '',
    member_count INTEGER NOT NULL DEFAULT 0,
    direct_count INTEGER NOT NULL DEFAULT 0,
    inherited_count INTEGER NOT NULL DEFAULT 0,
    representative_observation_id INTEGER REFERENCES exposure_observations(id) ON DELETE SET NULL,
    representative_run_id INTEGER NOT NULL DEFAULT 0,
    settings_version INTEGER NOT NULL DEFAULT 0,
    gap_note TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id, cluster_key)
);

CREATE TABLE IF NOT EXISTS exposure_cluster_members (
    cluster_id INTEGER NOT NULL REFERENCES exposure_clusters(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL,
    membership TEXT NOT NULL DEFAULT 'member',
    evidence_source TEXT NOT NULL DEFAULT 'direct',
    inherited_from_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    inherited_run_id INTEGER NOT NULL DEFAULT 0,
    observation_id INTEGER REFERENCES exposure_observations(id) ON DELETE SET NULL,
    rule_version TEXT NOT NULL DEFAULT '',
    verified_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(cluster_id, asset_id)
);

-- 分析缓存：键绑定项目/范围/隐私/内容/规则/版本，内容变化只失效对应条目。
-- §E3 再绑定来源、规范化 URL 与配置版本，并带期限：过期或撤权后不得按命中处理。
CREATE TABLE IF NOT EXISTS exposure_analysis_cache (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    cache_key TEXT NOT NULL UNIQUE,
    project_id INTEGER NOT NULL,
    scope_version INTEGER NOT NULL DEFAULT 1,
    privacy_class TEXT NOT NULL DEFAULT 'public',
    content_hash TEXT NOT NULL DEFAULT '',
    rule_version TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    source_key TEXT NOT NULL DEFAULT '',
    canonical_url TEXT NOT NULL DEFAULT '',
    settings_version INTEGER NOT NULL DEFAULT 0,
    result_json TEXT NOT NULL DEFAULT '{}',
    hits INTEGER NOT NULL DEFAULT 0,
    expires_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_exposure_cache_content ON exposure_analysis_cache(project_id, content_hash, kind);


-- §Q4 待归并发现：证书透明度/历史归档带回的新名字只进入这个队列，
-- 不自动成为资产、不自动通过归属门禁，也不自动获得授权。
CREATE TABLE IF NOT EXISTS exposure_pending_discoveries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    run_id INTEGER NOT NULL REFERENCES exposure_runs(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    entity TEXT NOT NULL,
    canonical_key TEXT NOT NULL,
    evidence_url TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'unreviewed',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,source_key,kind,canonical_key)
);
CREATE INDEX IF NOT EXISTS idx_exposure_pending_project ON exposure_pending_discoveries(project_id,status,id);


-- §E2 暴露面专属采集配置：当前值一份，历史每版一份；不改共享 config_profiles，也不进全局设置。
CREATE TABLE IF NOT EXISTS exposure_settings (
    project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    version INTEGER NOT NULL DEFAULT 1,
    settings_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS exposure_setting_versions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    settings_json TEXT NOT NULL DEFAULT '{}',
    actor TEXT NOT NULL DEFAULT 'user',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,version)
);

-- 同一项目最多一个进行中的采集任务：并发启动由数据库拒绝第二条。
CREATE UNIQUE INDEX IF NOT EXISTS ux_exposure_runs_single_active
    ON exposure_runs(project_id) WHERE status IN ('queued','running');

-- §E5 整改记录绑定原 finding / observation / run，复测另起一轮并重新验权。
CREATE TABLE IF NOT EXISTS exposure_remediations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    finding_id INTEGER NOT NULL REFERENCES exposure_findings(id) ON DELETE CASCADE,
    observation_id INTEGER REFERENCES exposure_observations(id) ON DELETE SET NULL,
    run_id INTEGER NOT NULL DEFAULT 0,
    actor TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open',
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    closed_at TEXT NOT NULL DEFAULT '',
    UNIQUE(project_id, finding_id, observation_id)
);
CREATE INDEX IF NOT EXISTS idx_exposure_remediation_project ON exposure_remediations(project_id,status,id);

-- 复测轮次只是关联：它不改写旧观察，也不把旧观察说成"已消失"。
CREATE TABLE IF NOT EXISTS exposure_remediation_retests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    remediation_id INTEGER NOT NULL REFERENCES exposure_remediations(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    run_id INTEGER NOT NULL REFERENCES exposure_runs(id) ON DELETE CASCADE,
    finding_id INTEGER NOT NULL DEFAULT 0,
    observation_url TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(remediation_id, run_id)
);

-- G1-3 信息分类：每次观察按策略版本留一行，旧行不改写。
CREATE TABLE IF NOT EXISTS exposure_classifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    finding_id INTEGER NOT NULL REFERENCES exposure_findings(id) ON DELETE CASCADE,
    observation_id INTEGER REFERENCES exposure_observations(id) ON DELETE SET NULL,
    run_id INTEGER NOT NULL DEFAULT 0,
    policy_version TEXT NOT NULL,
    info_category TEXT NOT NULL,
    source_channel TEXT NOT NULL,
    disposition TEXT NOT NULL CHECK(disposition IN ('risk_candidate','classification_pending','excluded_public_procurement','excluded_public_material')),
    reason TEXT NOT NULL,
    markers_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    UNIQUE(finding_id, observation_id, policy_version)
);
CREATE INDEX IF NOT EXISTS idx_exposure_classifications_finding ON exposure_classifications(finding_id,id);

-- G1-3 线索当前处置：排除粘性，冲突只置标记，等人工改判（IR-07）。
CREATE TABLE IF NOT EXISTS exposure_finding_dispositions (
    finding_id INTEGER PRIMARY KEY REFERENCES exposure_findings(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    disposition TEXT NOT NULL CHECK(disposition IN ('risk_candidate','classification_pending','excluded_public_procurement','excluded_public_material')),
    info_category TEXT NOT NULL,
    source_channel TEXT NOT NULL,
    reason TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    classification_id INTEGER NOT NULL,
    conflict INTEGER NOT NULL DEFAULT 0,
    conflict_note TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_exposure_dispositions_project ON exposure_finding_dispositions(project_id,disposition);

-- G3 导入批次：来源声明、操作者、文件 hash 与格式、幂等 operationId。正文不入库，
-- 观察里只留脱敏摘录与哈希。同一项目同一 operationId 只能对应一份载荷。
CREATE TABLE IF NOT EXISTS exposure_import_batches (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    run_id INTEGER REFERENCES exposure_runs(id) ON DELETE SET NULL,
    operation_id TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    channel TEXT NOT NULL,
    operator TEXT NOT NULL,
    source_declaration TEXT NOT NULL,
    declared_url TEXT NOT NULL DEFAULT '',
    batch_label TEXT NOT NULL DEFAULT '',
    import_version TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',
    files_json TEXT NOT NULL DEFAULT '[]',
    report_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    completed_at TEXT NOT NULL DEFAULT '',
    UNIQUE(project_id, operation_id)
);

-- G3 情报等级的人工变更（只追加）。导入时的等级写在观察里，这里只记人工改动及其依据。
CREATE TABLE IF NOT EXISTS exposure_intel_grades (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    finding_id INTEGER NOT NULL REFERENCES exposure_findings(id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    previous_grade TEXT NOT NULL,
    grade TEXT NOT NULL,
    actor TEXT NOT NULL,
    reason TEXT NOT NULL,
    sample_observation_id INTEGER,
    entity_id INTEGER,
    created_at TEXT NOT NULL,
    UNIQUE(project_id, operation_id)
);
CREATE INDEX IF NOT EXISTS idx_exposure_intel_grades_finding ON exposure_intel_grades(finding_id, id);

-- G4 泄漏事件：同一泄漏/信息事件的人工可复核主体。自动只生成 candidate 并附上严格依据
-- （规范化全文或完整响应体哈希相同），状态只由人工改，原观察与线索不改写。
-- status 与旧 finding.status 的映射：candidate→new，reviewing→reviewing，verified→confirmed，
-- dismissed→dismissed，excluded_public→dismissed（带排除理由），merged 表示已并入另一事件。
CREATE TABLE IF NOT EXISTS exposure_incidents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    schema_version INTEGER NOT NULL DEFAULT 1,
    stable_key TEXT NOT NULL,
    title TEXT NOT NULL,
    info_category TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL CHECK(status IN ('candidate','reviewing','verified','dismissed','excluded_public','merged')),
    revision INTEGER NOT NULL DEFAULT 1,
    basis TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    review_needed INTEGER NOT NULL DEFAULT 0,
    review_needed_reason TEXT NOT NULL DEFAULT '',
    merged_into INTEGER REFERENCES exposure_incidents(id) ON DELETE SET NULL,
    handling_state TEXT NOT NULL DEFAULT 'none',
    first_run_id INTEGER NOT NULL DEFAULT 0,
    last_run_id INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(project_id, stable_key)
);
CREATE INDEX IF NOT EXISTS idx_exposure_incidents_page ON exposure_incidents(project_id, status, id);

-- G4 传播位置：事件出现在哪个地址/仓库文件/导入文件/索引。一条线索在同一时刻只属于一个
-- 有效事件，拆分与合并只改位置状态，不删行。
CREATE TABLE IF NOT EXISTS exposure_incident_locations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    incident_id INTEGER NOT NULL REFERENCES exposure_incidents(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    finding_id INTEGER NOT NULL REFERENCES exposure_findings(id) ON DELETE CASCADE,
    observation_id INTEGER REFERENCES exposure_observations(id) ON DELETE SET NULL,
    run_id INTEGER NOT NULL DEFAULT 0,
    location_kind TEXT NOT NULL,
    canonical_url TEXT NOT NULL,
    source_key TEXT NOT NULL,
    collection_mode TEXT NOT NULL,
    relation TEXT NOT NULL,
    content_key TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','split_out','merged')),
    created_at TEXT NOT NULL,
    UNIQUE(incident_id, finding_id)
);
CREATE INDEX IF NOT EXISTS idx_exposure_incident_locations_finding ON exposure_incident_locations(project_id, finding_id, status);
CREATE INDEX IF NOT EXISTS idx_exposure_incident_locations_page ON exposure_incident_locations(incident_id, status, id);
CREATE INDEX IF NOT EXISTS idx_exposure_incident_locations_content ON exposure_incident_locations(project_id, content_key);

-- G4 事件涉及单位：只引用组织目录里已存在的实体，不改资产归属。confirmed_link 只来自
-- 单位项目绑定加已确认的资产归属，或人工带证据的确认。
CREATE TABLE IF NOT EXISTS exposure_incident_entities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    incident_id INTEGER NOT NULL REFERENCES exposure_incidents(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    entity_id INTEGER NOT NULL,
    link_level TEXT NOT NULL CHECK(link_level IN ('confirmed_link','candidate_link','unknown','conflict')),
    origin TEXT NOT NULL,
    basis TEXT NOT NULL,
    evidence_json TEXT NOT NULL DEFAULT '[]',
    directory_version INTEGER NOT NULL DEFAULT 0,
    binding_version INTEGER NOT NULL DEFAULT 0,
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_exposure_incident_entities_incident ON exposure_incident_entities(incident_id, active);
CREATE INDEX IF NOT EXISTS idx_exposure_incident_entities_project ON exposure_incident_entities(project_id, active, entity_id);

-- G4 事件动作（只追加）：自动建议、人工复核、合并、拆分、单位关联、处置登记。
-- 通知/下架只登记人工已做的动作，应用不对外联系任何人或平台。同项目同 operationId 只对应一份载荷。
CREATE TABLE IF NOT EXISTS exposure_incident_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    incident_id INTEGER NOT NULL REFERENCES exposure_incidents(id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    action TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    actor TEXT NOT NULL,
    reason TEXT NOT NULL DEFAULT '',
    expected_revision INTEGER NOT NULL DEFAULT 0,
    from_status TEXT NOT NULL DEFAULT '',
    to_status TEXT NOT NULL DEFAULT '',
    refs_json TEXT NOT NULL DEFAULT '{}',
    result_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    UNIQUE(project_id, operation_id)
);
CREATE INDEX IF NOT EXISTS idx_exposure_incident_events_incident ON exposure_incident_events(incident_id, id);

-- G4 事件建议按内容找同项目的相同全文：完整响应体哈希与规范化全文哈希都要能走索引。
CREATE INDEX IF NOT EXISTS idx_exposure_observations_content ON exposure_observations(project_id, content_hash);
CREATE INDEX IF NOT EXISTS idx_exposure_observations_normalized ON exposure_observations(project_id, json_extract(evidence_json,'$.detail.normalizedTextSha256'));

-- IR-07 人工改判审计。operationId 幂等；result_json 是首次结果，重放原样返回。
CREATE TABLE IF NOT EXISTS exposure_reclassifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    operation_id TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    finding_id INTEGER NOT NULL,
    classification_id INTEGER NOT NULL,
    actor TEXT NOT NULL,
    reason TEXT NOT NULL,
    disposition TEXT NOT NULL,
    info_category TEXT NOT NULL,
    result_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(project_id, operation_id)
);

-- FIX2-3：事件单位关联随归属投影刷新的水位（每项目一行）。只记读到哪里，不是归属数据本身。
CREATE TABLE IF NOT EXISTS exposure_unit_link_watermarks (
    project_id INTEGER PRIMARY KEY,
    asset_event_id INTEGER NOT NULL DEFAULT 0,
    item_rowid INTEGER NOT NULL DEFAULT 0,
    directory_version INTEGER NOT NULL DEFAULT 0,
    binding_version INTEGER NOT NULL DEFAULT 0,
    binding_kind TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL
);

-- FIX2：归属模块的逐资产事件查询（单位投影）按资产走索引。asset_events 由主库结构建立，
-- 暴露面结构总在主库初始化之后执行；表不存在时 ensure_exposure_schema 只记警告并跳过这一条。
CREATE INDEX IF NOT EXISTS idx_asset_events_project_asset ON asset_events(project_id, asset_id, event_type);
