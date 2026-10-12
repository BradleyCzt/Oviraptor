// The whole SQLite schema: every table, index and view the app creates on open.
// Included from db.rs so the schema text stays one contiguous block (§12).

const SCHEMA: &str = concat!(r#"
CREATE TABLE IF NOT EXISTS projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS config_profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    is_default INTEGER NOT NULL DEFAULT 0,
    settings_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS app_settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS worker_nodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    endpoint TEXT NOT NULL UNIQUE,
    access_token TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_seen_at TEXT,
    last_sync_at TEXT,
    last_error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS content_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    keyword TEXT NOT NULL,
    normalized_keyword TEXT NOT NULL UNIQUE,
    category TEXT NOT NULL DEFAULT 'custom_rule',
    source_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_content_rules_enabled ON content_rules(enabled,normalized_keyword);

CREATE TABLE IF NOT EXISTS hackerone_programs (
    id TEXT PRIMARY KEY,
    handle TEXT UNIQUE NOT NULL,
    name TEXT NOT NULL,
    icon_url TEXT NOT NULL DEFAULT '',
    policy TEXT NOT NULL DEFAULT '',
    policy_hash TEXT NOT NULL DEFAULT '',
    submission_state TEXT NOT NULL DEFAULT '',
    program_state TEXT NOT NULL DEFAULT '',
    offers_bounties INTEGER NOT NULL DEFAULT 0,
    open_scope INTEGER NOT NULL DEFAULT 0,
    fast_payments INTEGER NOT NULL DEFAULT 0,
    safe_harbor INTEGER NOT NULL DEFAULT 0,
    collaboration INTEGER NOT NULL DEFAULT 0,
    started_accepting_at TEXT,
    last_synced_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    custom_industry TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS hackerone_scopes (
    id TEXT PRIMARY KEY,
    program_handle TEXT NOT NULL,
    asset_type TEXT NOT NULL DEFAULT '',
    asset_identifier TEXT NOT NULL DEFAULT '',
    eligible_for_submission INTEGER NOT NULL DEFAULT 0,
    eligible_for_bounty INTEGER NOT NULL DEFAULT 0,
    max_severity TEXT NOT NULL DEFAULT '',
    instruction TEXT NOT NULL DEFAULT '',
    reference TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS hackerone_exclusions (
    id TEXT PRIMARY KEY,
    program_handle TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    details TEXT NOT NULL DEFAULT '',
    updated_at TEXT,
    active INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS hackerone_notes (
    program_handle TEXT PRIMARY KEY,
    bookmarked INTEGER NOT NULL DEFAULT 0,
    tags TEXT NOT NULL DEFAULT '',
    notes TEXT NOT NULL DEFAULT '',
    last_tested_at TEXT
);

CREATE TABLE IF NOT EXISTS hackerone_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    program_handle TEXT NOT NULL,
    event_type TEXT NOT NULL,
    summary TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE INDEX IF NOT EXISTS idx_h1_program_handle ON hackerone_programs(handle);
CREATE INDEX IF NOT EXISTS idx_h1_scope_program ON hackerone_scopes(program_handle,active);
CREATE INDEX IF NOT EXISTS idx_h1_event_program ON hackerone_events(program_handle,created_at);

CREATE TABLE IF NOT EXISTS sentinel_targets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    company TEXT NOT NULL DEFAULT '',
    url TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    value_score INTEGER NOT NULL DEFAULT 0,
    scan_mode TEXT NOT NULL DEFAULT '',
    routing_reason TEXT NOT NULL DEFAULT '',
    last_attempt_number INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,scan_id,url)
);
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
-- An administrator's environment preparation and a scan entering the active
-- queue must be mutually exclusive across connections and processes.
CREATE TABLE IF NOT EXISTS environment_preparation_lease (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    owner TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS environment_preparation_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner TEXT NOT NULL,
    event TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
-- Display-only administrator installation output. Persist redacted records
-- before notifying the UI so reconnects can read authoritative rows.
CREATE TABLE IF NOT EXISTS environment_install_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stage TEXT NOT NULL,
    stream TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS sentinel_scans (
    id TEXT PRIMARY KEY,
    project_id INTEGER REFERENCES projects(id) ON DELETE SET NULL,
    project_name TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'queued',
    current_checkpoint TEXT NOT NULL DEFAULT '',
    task_path TEXT NOT NULL DEFAULT '',
    previous_scan_id TEXT NOT NULL DEFAULT '',
    llm_requests INTEGER NOT NULL DEFAULT 0,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cached_tokens INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    scan_type TEXT NOT NULL DEFAULT 'web',
    task_name TEXT NOT NULL DEFAULT '',
    source_path TEXT NOT NULL DEFAULT '',
    skill_names TEXT NOT NULL DEFAULT '',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    archived_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS environment_preparation_refuse_scan_insert
BEFORE INSERT ON sentinel_scans
WHEN NEW.status IN ('queued','scanning','pausing')
 AND EXISTS(SELECT 1 FROM environment_preparation_lease WHERE singleton=1)
BEGIN
    SELECT RAISE(ABORT,'environment_preparation_active');
END;
CREATE TRIGGER IF NOT EXISTS environment_preparation_refuse_scan_activation
BEFORE UPDATE OF status ON sentinel_scans
WHEN NEW.status IN ('queued','scanning','pausing')
 AND EXISTS(SELECT 1 FROM environment_preparation_lease WHERE singleton=1)
BEGIN
    SELECT RAISE(ABORT,'environment_preparation_active');
END;
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
    backend_plan_json TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(scan_id,attempt_number)
);
-- Local operator presentation state only. It never grants execution authority.
CREATE TABLE IF NOT EXISTS agent_dialog_views (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    revision INTEGER NOT NULL CHECK(revision > 0),
    selected_thread TEXT NOT NULL DEFAULT '',
    all_read_sequence INTEGER NOT NULL DEFAULT 0 CHECK(all_read_sequence >= 0),
    thread_read_sequences_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(thread_read_sequences_json)),
    PRIMARY KEY(scan_id,attempt_number)
);
CREATE TABLE IF NOT EXISTS agent_dialog_selections (
    scope_key TEXT PRIMARY KEY NOT NULL,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    scan_id TEXT REFERENCES sentinel_scans(id) ON DELETE SET NULL,
    CHECK((project_id IS NULL AND scope_key='all') OR
          (project_id IS NOT NULL AND project_id > 0 AND scope_key='project:' || CAST(project_id AS TEXT)))
);
CREATE TABLE IF NOT EXISTS sentinel_deleted_scans (
    scan_id TEXT PRIMARY KEY,
    deleted_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS sentinel_processes (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    process_id INTEGER NOT NULL DEFAULT 0,
    engine TEXT NOT NULL DEFAULT '',
    work_dir TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,process_id)
);
CREATE TABLE IF NOT EXISTS sentinel_checkpoints (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    stage TEXT NOT NULL,
    raw_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,url,stage)
);
CREATE TABLE IF NOT EXISTS sentinel_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL DEFAULT '',
    stage TEXT NOT NULL,
    kind TEXT NOT NULL,
    record_key TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    severity TEXT NOT NULL DEFAULT '',
    record_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,stage,kind,record_key)
);
CREATE INDEX IF NOT EXISTS idx_sentinel_findings_scan ON sentinel_findings(scan_id,stage,kind);
CREATE TABLE IF NOT EXISTS sentinel_opportunities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL DEFAULT '',
    opportunity_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    score INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'queued',
    confidence TEXT NOT NULL DEFAULT '',
    why_json TEXT NOT NULL DEFAULT '[]',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    recommended_action_json TEXT NOT NULL DEFAULT '{}',
    source TEXT NOT NULL DEFAULT '',
    record_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,opportunity_key)
);
CREATE INDEX IF NOT EXISTS idx_sentinel_opportunities_inbox ON sentinel_opportunities(project_id,status,score DESC,last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_opportunities_scan ON sentinel_opportunities(scan_id,target_url,score DESC);
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
CREATE TABLE IF NOT EXISTS sentinel_validations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    finding_key TEXT NOT NULL DEFAULT 'url-summary',
    finding_kind TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL DEFAULT 'pending',
    severity TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    evidence TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,url,finding_key)
);
CREATE INDEX IF NOT EXISTS idx_sentinel_scan_updated ON sentinel_scans(updated_at);
CREATE INDEX IF NOT EXISTS idx_sentinel_scan_project_updated ON sentinel_scans(project_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_attempt_scan ON sentinel_scan_attempts(scan_id,attempt_number DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_targets_project_updated ON sentinel_targets(project_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sentinel_targets_scan ON sentinel_targets(scan_id);
CREATE INDEX IF NOT EXISTS idx_sentinel_validation_scan ON sentinel_validations(scan_id,updated_at);
CREATE TABLE IF NOT EXISTS investigation_validations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    opportunity_id INTEGER REFERENCES sentinel_opportunities(id) ON DELETE SET NULL,
    hypothesis_id INTEGER REFERENCES investigation_hypotheses(id) ON DELETE SET NULL,
    api_key TEXT NOT NULL DEFAULT '',
    identity_id TEXT NOT NULL DEFAULT '',
    method TEXT NOT NULL DEFAULT 'GET',
    request_url TEXT NOT NULL DEFAULT '',
    request_headers_json TEXT NOT NULL DEFAULT '{}',
    request_body TEXT NOT NULL DEFAULT '',
    response_status INTEGER NOT NULL DEFAULT 0,
    response_status_text TEXT NOT NULL DEFAULT '',
    response_headers_json TEXT NOT NULL DEFAULT '{}',
    response_body TEXT NOT NULL DEFAULT '',
    decoded_body TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL DEFAULT 'needs_more_evidence',
    severity TEXT NOT NULL DEFAULT 'info',
    confidence TEXT NOT NULL DEFAULT 'low',
    ai_assessment TEXT NOT NULL DEFAULT '',
    note TEXT NOT NULL DEFAULT '',
    next_action TEXT NOT NULL DEFAULT '',
    evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_investigation_validations_scan ON investigation_validations(scan_id,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_investigation_validations_opportunity ON investigation_validations(opportunity_id,updated_at DESC);

-- Investigation graph: deterministic browser/AST evidence is kept as first-class
-- data instead of being flattened into generic findings.  The graph is rebuilt
-- idempotently for each scan target, while baselines and learned layers retain
-- cross-scan history.
CREATE TABLE IF NOT EXISTS investigation_nodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    node_key TEXT NOT NULL,
    node_type TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    confidence TEXT NOT NULL DEFAULT '',
    value_score INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'observed',
    payload_json TEXT NOT NULL DEFAULT '{}',
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,node_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_nodes_scan ON investigation_nodes(scan_id,target_url,node_type);
CREATE INDEX IF NOT EXISTS idx_investigation_nodes_project ON investigation_nodes(project_id,node_type,last_seen DESC);

CREATE TABLE IF NOT EXISTS investigation_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    source_key TEXT NOT NULL,
    relation TEXT NOT NULL,
    target_key TEXT NOT NULL,
    confidence TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,source_key,relation,target_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_edges_scan ON investigation_edges(scan_id,target_url,source_key);

CREATE TABLE IF NOT EXISTS investigation_actions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    action_key TEXT NOT NULL,
    state_key TEXT NOT NULL DEFAULT '',
    action_type TEXT NOT NULL DEFAULT 'interaction',
    label TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL DEFAULT '',
    value_score INTEGER NOT NULL DEFAULT 0,
    protocol_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,action_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_actions_scan ON investigation_actions(scan_id,target_url,value_score DESC);

CREATE TABLE IF NOT EXISTS investigation_api_models (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    method TEXT NOT NULL DEFAULT 'UNKNOWN',
    url TEXT NOT NULL DEFAULT '',
    normalized_path TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT '',
    confidence TEXT NOT NULL DEFAULT '',
    auth_scope TEXT NOT NULL DEFAULT 'unknown',
    parameters_json TEXT NOT NULL DEFAULT '[]',
    request_schema_json TEXT NOT NULL DEFAULT '{}',
    response_schema_json TEXT NOT NULL DEFAULT '{}',
    state_keys_json TEXT NOT NULL DEFAULT '[]',
    action_keys_json TEXT NOT NULL DEFAULT '[]',
    identity_keys_json TEXT NOT NULL DEFAULT '[]',
    observed_count INTEGER NOT NULL DEFAULT 1,
    baseline_status TEXT NOT NULL DEFAULT 'new',
    payload_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,api_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_api_scan ON investigation_api_models(scan_id,target_url,baseline_status);
CREATE INDEX IF NOT EXISTS idx_investigation_api_project ON investigation_api_models(project_id,normalized_path,method);

CREATE TABLE IF NOT EXISTS investigation_hypotheses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    hypothesis_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'candidate',
    score INTEGER NOT NULL DEFAULT 0,
    confidence TEXT NOT NULL DEFAULT '',
    contract_json TEXT NOT NULL DEFAULT '{}',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    decision_json TEXT NOT NULL DEFAULT '{}',
    source_opportunity_key TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,hypothesis_key)
);
CREATE INDEX IF NOT EXISTS idx_investigation_hypotheses_queue ON investigation_hypotheses(project_id,status,score DESC,updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_investigation_hypotheses_scan ON investigation_hypotheses(scan_id,target_url,score DESC);

-- A mutation-capable verification is disabled by default. Approval is scoped
-- to one hypothesis, endpoint/method contract, a small attempt budget and an
-- expiry time so entering the validation queue never implies broad consent.
CREATE TABLE IF NOT EXISTS investigation_mutation_approvals (
    hypothesis_id INTEGER PRIMARY KEY REFERENCES investigation_hypotheses(id) ON DELETE CASCADE,
    approved INTEGER NOT NULL DEFAULT 0,
    scope_json TEXT NOT NULL DEFAULT '{}',
    max_attempts INTEGER NOT NULL DEFAULT 1,
    note TEXT NOT NULL DEFAULT '',
    expires_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_investigation_mutation_expiry ON investigation_mutation_approvals(approved,expires_at);

CREATE TABLE IF NOT EXISTS investigation_identity_diffs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    left_identity_key TEXT NOT NULL,
    right_identity_key TEXT NOT NULL,
    difference_type TEXT NOT NULL DEFAULT '',
    risk_score INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'observed',
    matrix_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,api_key,left_identity_key,right_identity_key,difference_type)
);
CREATE INDEX IF NOT EXISTS idx_investigation_identity_scan ON investigation_identity_diffs(scan_id,target_url,risk_score DESC);

CREATE TABLE IF NOT EXISTS investigation_metrics (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    node_count INTEGER NOT NULL DEFAULT 0,
    edge_count INTEGER NOT NULL DEFAULT 0,
    state_count INTEGER NOT NULL DEFAULT 0,
    action_count INTEGER NOT NULL DEFAULT 0,
    api_count INTEGER NOT NULL DEFAULT 0,
    parameter_count INTEGER NOT NULL DEFAULT 0,
    hypothesis_count INTEGER NOT NULL DEFAULT 0,
    added_count INTEGER NOT NULL DEFAULT 0,
    changed_count INTEGER NOT NULL DEFAULT 0,
    removed_count INTEGER NOT NULL DEFAULT 0,
    duplicate_count INTEGER NOT NULL DEFAULT 0,
    information_gain INTEGER NOT NULL DEFAULT 0,
    token_worthy INTEGER NOT NULL DEFAULT 0,
    stop_reason TEXT NOT NULL DEFAULT '',
    decision_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,target_url)
);
CREATE INDEX IF NOT EXISTS idx_investigation_metrics_project ON investigation_metrics(project_id,information_gain DESC,updated_at DESC);

CREATE TABLE IF NOT EXISTS investigation_baselines (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    identity_key TEXT NOT NULL DEFAULT 'anonymous',
    source_scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    signature TEXT NOT NULL,
    api_signatures_json TEXT NOT NULL DEFAULT '[]',
    parameter_signatures_json TEXT NOT NULL DEFAULT '[]',
    metrics_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,target_url,identity_key,source_scan_id)
);
CREATE INDEX IF NOT EXISTS idx_investigation_baseline_lookup ON investigation_baselines(project_id,target_url,identity_key,created_at DESC);

CREATE TABLE IF NOT EXISTS knowledge_facts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    fact_key TEXT NOT NULL,
    fact_type TEXT NOT NULL,
    subject TEXT NOT NULL DEFAULT '',
    predicate TEXT NOT NULL DEFAULT '',
    object_json TEXT NOT NULL DEFAULT '{}',
    confidence TEXT NOT NULL DEFAULT '',
    source_scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL DEFAULT '',
    evidence_hash TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,fact_key,source_scan_id,target_url)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_facts_subject ON knowledge_facts(project_id,fact_type,subject,last_seen DESC);

CREATE TABLE IF NOT EXISTS knowledge_strategies (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    strategy_key TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    conditions_json TEXT NOT NULL DEFAULT '{}',
    playbook_json TEXT NOT NULL DEFAULT '{}',
    support_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    promoted INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,strategy_key)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_strategies_project ON knowledge_strategies(project_id,promoted,support_count DESC);

CREATE TABLE IF NOT EXISTS knowledge_outcomes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    hypothesis_key TEXT NOT NULL DEFAULT '',
    strategy_key TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL DEFAULT '',
    stop_reason TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,target_url,hypothesis_key,strategy_key)
);
CREATE INDEX IF NOT EXISTS idx_knowledge_outcomes_strategy ON knowledge_outcomes(project_id,strategy_key,outcome);

CREATE TABLE IF NOT EXISTS targets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    target_type TEXT NOT NULL,
    value TEXT NOT NULL,
    normalized_value TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id, target_type, normalized_value)
);

CREATE TABLE IF NOT EXISTS runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    profile_id INTEGER REFERENCES config_profiles(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    pipeline TEXT NOT NULL DEFAULT 'collect',
    status TEXT NOT NULL DEFAULT 'queued',
    stage TEXT NOT NULL DEFAULT 'queued',
    progress REAL NOT NULL DEFAULT 0,
    processed INTEGER NOT NULL DEFAULT 0,
    total INTEGER NOT NULL DEFAULT 0,
    config_snapshot TEXT NOT NULL DEFAULT '{}',
    output_dir TEXT NOT NULL DEFAULT '',
    error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    started_at TEXT,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS assets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_key TEXT NOT NULL UNIQUE,
    company TEXT NOT NULL DEFAULT '',
    host TEXT NOT NULL DEFAULT '',
    link TEXT NOT NULL DEFAULT '',
    ip TEXT NOT NULL DEFAULT '',
    port TEXT NOT NULL DEFAULT '',
    protocol TEXT NOT NULL DEFAULT '',
    domain TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    status_code TEXT NOT NULL DEFAULT '',
    probe_outcome TEXT NOT NULL DEFAULT '',
    probe_entry_state TEXT NOT NULL DEFAULT '',
    review_tier TEXT NOT NULL DEFAULT '',
    content_category TEXT NOT NULL DEFAULT '',
    score TEXT NOT NULL DEFAULT '',
    state_hash TEXT NOT NULL DEFAULT '',
    probe_hash TEXT NOT NULL DEFAULT '',
    canonical_key TEXT NOT NULL DEFAULT '',
    extra_json TEXT NOT NULL DEFAULT '{}',
    has_login INTEGER NOT NULL DEFAULT -1,
    has_register INTEGER NOT NULL DEFAULT -1,
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_alive TEXT
);

CREATE TABLE IF NOT EXISTS project_assets (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    decision TEXT NOT NULL DEFAULT 'pending',
    note TEXT NOT NULL DEFAULT '',
    is_deleted INTEGER NOT NULL DEFAULT 0,
    deleted_at TEXT,
    first_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    entry_opened_at TEXT,
    PRIMARY KEY(project_id, asset_id)
);

CREATE TABLE IF NOT EXISTS asset_ownership_profiles (
    project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    legal_name TEXT NOT NULL DEFAULT '',
    jurisdiction TEXT NOT NULL DEFAULT '',
    jurisdictions_json TEXT NOT NULL DEFAULT '[]',
    excluded_jurisdictions_json TEXT NOT NULL DEFAULT '[]',
    aliases_json TEXT NOT NULL DEFAULT '[]',
    approved_domains_json TEXT NOT NULL DEFAULT '[]',
    shared_domains_json TEXT NOT NULL DEFAULT '[]',
    excluded_names_json TEXT NOT NULL DEFAULT '[]',
    excluded_domains_json TEXT NOT NULL DEFAULT '[]',
    notes TEXT NOT NULL DEFAULT '',
    policy_version INTEGER NOT NULL DEFAULT 1,
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS asset_ownership_decisions (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'unreviewed',
    confidence INTEGER NOT NULL DEFAULT 0,
    authorization_status TEXT NOT NULL DEFAULT 'unknown',
    exposure_eligible INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL DEFAULT 'auto',
    reason TEXT NOT NULL DEFAULT '',
    evidence_json TEXT NOT NULL DEFAULT '[]',
    negative_evidence_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(project_id,asset_id)
);
CREATE INDEX IF NOT EXISTS idx_asset_ownership_queue ON asset_ownership_decisions(project_id,status,exposure_eligible,confidence DESC);

CREATE TABLE IF NOT EXISTS asset_ownership_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    rule_type TEXT NOT NULL,
    pattern TEXT NOT NULL,
    action TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'manual',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,rule_type,pattern,action)
);
CREATE INDEX IF NOT EXISTS idx_asset_ownership_rules_project ON asset_ownership_rules(project_id,enabled,rule_type);

-- O1 shared organization directory (the single directory; exposure must reference
-- these entity ids instead of creating a second one). It records organization
-- facts only and never carries or copies authorization between projects.
CREATE TABLE IF NOT EXISTS org_directory_state (
    id INTEGER PRIMARY KEY CHECK(id=1),
    version INTEGER NOT NULL DEFAULT 0 CHECK(version>=0),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS org_entities (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    group_id INTEGER REFERENCES org_entities(id) ON DELETE RESTRICT,
    parent_id INTEGER REFERENCES org_entities(id) ON DELETE RESTRICT,
    entity_kind TEXT NOT NULL CHECK(entity_kind IN ('group','headquarters','province_company','city_branch','county_branch','professional_subsidiary','overseas_entity','supplier','unknown')),
    legal_name TEXT NOT NULL CHECK(length(legal_name)>0),
    normalized_name TEXT NOT NULL CHECK(length(normalized_name)>0),
    unified_code TEXT NOT NULL DEFAULT '',
    region_code TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'confirmed' CHECK(status IN ('confirmed','retired')),
    source TEXT NOT NULL DEFAULT 'manual',
    source_ref TEXT NOT NULL DEFAULT '',
    directory_version INTEGER NOT NULL CHECK(directory_version>0),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(normalized_name)
);
CREATE INDEX IF NOT EXISTS idx_org_entities_group ON org_entities(group_id,entity_kind,status);
CREATE TABLE IF NOT EXISTS org_entity_identifiers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_id INTEGER NOT NULL REFERENCES org_entities(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('short_name','english_name','historical_name','brand','official_domain','shared_domain')),
    value TEXT NOT NULL CHECK(length(value)>0),
    normalized_value TEXT NOT NULL CHECK(length(normalized_value)>0),
    source TEXT NOT NULL DEFAULT 'manual',
    source_ref TEXT NOT NULL DEFAULT '',
    directory_version INTEGER NOT NULL CHECK(directory_version>0),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(entity_id,kind,normalized_value)
);
CREATE INDEX IF NOT EXISTS idx_org_identifiers_value ON org_entity_identifiers(kind,normalized_value);
-- Per-project binding to the shared directory. Binding a project grants nothing:
-- authorization stays in that project's own asset_ownership_decisions.
CREATE TABLE IF NOT EXISTS project_org_bindings (
    project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    project_kind TEXT NOT NULL CHECK(project_kind IN ('group_ledger','unit')),
    group_entity_id INTEGER NOT NULL REFERENCES org_entities(id) ON DELETE RESTRICT,
    unit_entity_id INTEGER REFERENCES org_entities(id) ON DELETE RESTRICT,
    binding_version INTEGER NOT NULL CHECK(binding_version>0),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
-- Directory audit: which version added what, and from which page (via_project_id
-- is provenance only; the directory itself is not project-owned data).
CREATE TABLE IF NOT EXISTS org_directory_changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    directory_version INTEGER NOT NULL,
    via_project_id INTEGER,
    change_type TEXT NOT NULL,
    entity_id INTEGER,
    payload_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(payload_json)),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- O2 ownership review (design 2026-10-08 §6–§10). Analysis runs are durable and bound to
-- the project/scope/data/policy/directory/binding/algorithm versions they used; at most one
-- queued/running run per project. Group membership is per snapshot: PRIMARY KEY(run_id,asset_id)
-- means an asset belongs to exactly one primary review group in a snapshot. Nothing here
-- grants authorization; group operations only write ownership facts through
-- asset_ownership_decisions and audit every row in ownership_operation_items.
CREATE TABLE IF NOT EXISTS ownership_analysis_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'queued' CHECK(status IN ('queued','running','completed','cancelled','failed')),
    scope_version TEXT NOT NULL,
    data_revision TEXT NOT NULL,
    data_revision_at_finish TEXT NOT NULL DEFAULT '',
    policy_version INTEGER NOT NULL,
    directory_version INTEGER NOT NULL,
    binding_version INTEGER NOT NULL,
    algorithm_version TEXT NOT NULL,
    cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0,1)),
    worker_token TEXT NOT NULL DEFAULT '',
    heartbeat_epoch INTEGER NOT NULL DEFAULT 0,
    total_assets INTEGER NOT NULL DEFAULT 0,
    processed_assets INTEGER NOT NULL DEFAULT 0,
    last_asset_id INTEGER NOT NULL DEFAULT 0,
    group_count INTEGER NOT NULL DEFAULT 0,
    member_count INTEGER NOT NULL DEFAULT 0,
    members_pruned INTEGER NOT NULL DEFAULT 0 CHECK(members_pruned IN (0,1)),
    error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    started_at TEXT NOT NULL DEFAULT '',
    finished_at TEXT NOT NULL DEFAULT '',
    -- O3: rules are part of the bound versions; incremental runs reuse an unchanged base
    -- snapshot's classification for rows below the data-change watermark.
    rules_version TEXT NOT NULL DEFAULT '',
    run_kind TEXT NOT NULL DEFAULT 'full' CHECK(run_kind IN ('full','incremental')),
    base_run_id INTEGER,
    mode_reason TEXT NOT NULL DEFAULT '',
    watermark_at TEXT NOT NULL DEFAULT '',
    reclassified_assets INTEGER NOT NULL DEFAULT 0,
    carried_assets INTEGER NOT NULL DEFAULT 0,
    resume_count INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_ownership_runs_project ON ownership_analysis_runs(project_id,status,id);
CREATE UNIQUE INDEX IF NOT EXISTS ux_ownership_runs_active ON ownership_analysis_runs(project_id) WHERE status IN ('queued','running');
CREATE TABLE IF NOT EXISTS ownership_review_groups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES ownership_analysis_runs(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL,
    group_key TEXT NOT NULL,
    lane TEXT NOT NULL CHECK(lane IN ('consistent','insufficient','counter_evidence','conflict','unknown','manual')),
    nav_bucket TEXT NOT NULL,
    candidate_entity_id INTEGER,
    candidate_label TEXT NOT NULL DEFAULT '',
    candidate_source TEXT NOT NULL DEFAULT '',
    relation_kind TEXT NOT NULL DEFAULT '',
    region_clue TEXT NOT NULL DEFAULT '',
    boundary_kind TEXT NOT NULL,
    boundary_value TEXT NOT NULL DEFAULT '',
    support_kind TEXT NOT NULL DEFAULT '',
    counter_kind TEXT NOT NULL DEFAULT '',
    decision_status TEXT NOT NULL,
    decision_source TEXT NOT NULL,
    authorization_status TEXT NOT NULL,
    exposure_eligible INTEGER NOT NULL,
    member_count INTEGER NOT NULL DEFAULT 0 CHECK(member_count>=0),
    distinct_hosts INTEGER NOT NULL DEFAULT 0,
    sample_asset_ids_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(sample_asset_ids_json)),
    sample_rule TEXT NOT NULL DEFAULT '',
    reason TEXT NOT NULL DEFAULT '',
    priority INTEGER NOT NULL DEFAULT 50,
    sort_rank INTEGER NOT NULL DEFAULT 0,
    group_version INTEGER NOT NULL DEFAULT 1 CHECK(group_version>0),
    applied_operations INTEGER NOT NULL DEFAULT 0,
    -- O3 display index: members whose authorization was revoked after this snapshot.
    authorization_revoked INTEGER NOT NULL DEFAULT 0,
    UNIQUE(run_id,group_key)
);
CREATE INDEX IF NOT EXISTS idx_ownership_groups_rank ON ownership_review_groups(run_id,sort_rank);
CREATE INDEX IF NOT EXISTS idx_ownership_groups_lane ON ownership_review_groups(run_id,lane,sort_rank);
CREATE TABLE IF NOT EXISTS ownership_review_group_members (
    run_id INTEGER NOT NULL REFERENCES ownership_analysis_runs(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL,
    project_id INTEGER NOT NULL,
    group_id INTEGER NOT NULL,
    host TEXT NOT NULL DEFAULT '',
    registrable_domain TEXT NOT NULL DEFAULT '',
    suffix_section TEXT NOT NULL DEFAULT '',
    company TEXT NOT NULL DEFAULT '',
    decision_state TEXT NOT NULL,
    evidence_hash TEXT NOT NULL,
    PRIMARY KEY(run_id,asset_id)
);
CREATE INDEX IF NOT EXISTS idx_ownership_members_group ON ownership_review_group_members(run_id,group_id,asset_id);
-- O3: operations are a state machine. Group decisions/reverts stage their frozen plan in
-- ownership_operation_staging and apply it in bounded chunks (applying -> completed);
-- authorization revocation blocks downstream first (blocked) and only then updates the
-- display index (completed). At most one applying decision/revert per group; a decision
-- operation can be reverted at most once.
CREATE TABLE IF NOT EXISTS ownership_operations (
    operation_id TEXT PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    run_id INTEGER NOT NULL,
    group_id INTEGER NOT NULL,
    kind TEXT NOT NULL DEFAULT 'group_decision' CHECK(kind IN ('group_decision','group_revert','authorization_revoke')),
    action TEXT NOT NULL CHECK(action IN ('confirm_ownership','related','third_party','excluded','revert','revoke_authorization')),
    status TEXT NOT NULL DEFAULT 'completed' CHECK(status IN ('applying','blocked','completed','failed')),
    target_operation_id TEXT NOT NULL DEFAULT '',
    payload_hash TEXT NOT NULL,
    expected_action_hash TEXT NOT NULL,
    total_items INTEGER NOT NULL DEFAULT 0,
    done_items INTEGER NOT NULL DEFAULT 0,
    last_seq INTEGER NOT NULL DEFAULT 0,
    worker_token TEXT NOT NULL DEFAULT '',
    heartbeat_epoch INTEGER NOT NULL DEFAULT 0,
    resume_count INTEGER NOT NULL DEFAULT 0,
    applied INTEGER NOT NULL DEFAULT 0,
    skipped INTEGER NOT NULL DEFAULT 0,
    conflicted INTEGER NOT NULL DEFAULT 0,
    failed INTEGER NOT NULL DEFAULT 0,
    result_json TEXT NOT NULL CHECK(json_valid(result_json)),
    error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_ownership_operations_run ON ownership_operations(run_id,group_id);
CREATE INDEX IF NOT EXISTS idx_ownership_operations_status ON ownership_operations(status,project_id);
CREATE UNIQUE INDEX IF NOT EXISTS ux_ownership_operations_group_active ON ownership_operations(run_id,group_id) WHERE status='applying' AND kind IN ('group_decision','group_revert');
CREATE UNIQUE INDEX IF NOT EXISTS ux_ownership_operations_revert_once ON ownership_operations(target_operation_id) WHERE kind='group_revert';
CREATE TABLE IF NOT EXISTS ownership_operation_staging (
    operation_id TEXT NOT NULL REFERENCES ownership_operations(operation_id) ON DELETE CASCADE,
    seq INTEGER NOT NULL CHECK(seq>0),
    asset_id INTEGER NOT NULL,
    planned TEXT NOT NULL CHECK(planned IN ('apply','skipped','conflicted')),
    reason TEXT NOT NULL DEFAULT '',
    expected_state TEXT NOT NULL,
    evidence_hash TEXT NOT NULL DEFAULT '',
    old_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(old_json)),
    restore_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(restore_json)),
    evidence_json TEXT NOT NULL DEFAULT '[]',
    evidence_ref TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(operation_id,seq),
    UNIQUE(operation_id,asset_id)
);
CREATE TABLE IF NOT EXISTS ownership_operation_items (
    operation_id TEXT NOT NULL REFERENCES ownership_operations(operation_id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL,
    outcome TEXT NOT NULL CHECK(outcome IN ('applied','skipped','conflicted','failed')),
    reason TEXT NOT NULL DEFAULT '',
    old_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(old_json)),
    new_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(new_json)),
    evidence_ref TEXT NOT NULL DEFAULT '',
    before_state TEXT NOT NULL DEFAULT '',
    after_state TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(operation_id,asset_id)
);
CREATE INDEX IF NOT EXISTS idx_ownership_operation_items_asset ON ownership_operation_items(asset_id,operation_id);

CREATE TABLE IF NOT EXISTS exposure_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    category TEXT NOT NULL,
    title TEXT NOT NULL,
    source_type TEXT NOT NULL DEFAULT 'manual',
    source_url TEXT NOT NULL DEFAULT '',
    evidence_excerpt TEXT NOT NULL DEFAULT '',
    severity TEXT NOT NULL DEFAULT 'medium',
    confidence INTEGER NOT NULL DEFAULT 50,
    status TEXT NOT NULL DEFAULT 'new',
    fingerprint TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    first_seen_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,fingerprint)
);
CREATE INDEX IF NOT EXISTS idx_exposure_findings_queue ON exposure_findings(project_id,status,severity,confidence DESC);
CREATE INDEX IF NOT EXISTS idx_exposure_findings_asset ON exposure_findings(project_id,asset_id);

CREATE TABLE IF NOT EXISTS exposure_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'queued',
    eligible_assets INTEGER NOT NULL DEFAULT 0,
    scanned_assets INTEGER NOT NULL DEFAULT 0,
    fetched_resources INTEGER NOT NULL DEFAULT 0,
    findings INTEGER NOT NULL DEFAULT 0,
    stage TEXT NOT NULL DEFAULT 'queued',
    current_source TEXT NOT NULL DEFAULT '',
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    error TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    started_at TEXT NOT NULL DEFAULT '',
    completed_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_exposure_runs_project ON exposure_runs(project_id,id DESC);

CREATE TABLE IF NOT EXISTS exposure_source_results (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES exposure_runs(id) ON DELETE CASCADE,
    source_key TEXT NOT NULL,
    status TEXT NOT NULL,
    item_count INTEGER NOT NULL DEFAULT 0,
    error TEXT NOT NULL DEFAULT '',
    completed_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(run_id,source_key)
);

CREATE TABLE IF NOT EXISTS asset_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER REFERENCES runs(id) ON DELETE CASCADE,
    level TEXT NOT NULL DEFAULT 'info',
    stage TEXT NOT NULL DEFAULT '',
    message TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE TABLE IF NOT EXISTS saved_views (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    columns_json TEXT NOT NULL DEFAULT '[]',
    filters_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE INDEX IF NOT EXISTS idx_targets_project ON targets(project_id);
CREATE INDEX IF NOT EXISTS idx_runs_project_created ON runs(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_assets_ip ON assets(ip);
CREATE INDEX IF NOT EXISTS idx_assets_domain ON assets(domain);
CREATE INDEX IF NOT EXISTS idx_assets_host ON assets(host);
CREATE INDEX IF NOT EXISTS idx_assets_probe ON assets(probe_outcome);
CREATE INDEX IF NOT EXISTS idx_assets_tier ON assets(review_tier);
CREATE INDEX IF NOT EXISTS idx_assets_review_order ON assets(review_tier, score, last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_project_assets_project_deleted ON project_assets(project_id, is_deleted);
CREATE INDEX IF NOT EXISTS idx_project_assets_project_decision ON project_assets(project_id, is_deleted, decision, last_seen DESC);
CREATE INDEX IF NOT EXISTS idx_project_assets_asset ON project_assets(asset_id);
CREATE INDEX IF NOT EXISTS idx_events_project_created ON asset_events(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_logs_run_created ON logs(run_id, created_at DESC);

CREATE TABLE IF NOT EXISTS agent_runs (
    id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    target_url TEXT NOT NULL DEFAULT '',
    -- §Stage 3: a new run is Native unless the row says otherwise. Historical rows keep
    -- whatever backend they were written with; nothing here rewrites them.
    backend TEXT NOT NULL DEFAULT 'native',
    role TEXT NOT NULL DEFAULT 'coordinator',
    parent_run_id TEXT REFERENCES agent_runs(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'prepared',
    plan_hash TEXT NOT NULL DEFAULT '',
    evidence_hash TEXT NOT NULL DEFAULT '',
    soft_token_budget INTEGER NOT NULL DEFAULT 0,
    hard_token_budget INTEGER NOT NULL DEFAULT 0,
    used_tokens INTEGER NOT NULL DEFAULT 0,
    used_cached_tokens INTEGER NOT NULL DEFAULT 0,
    soft_request_budget INTEGER NOT NULL DEFAULT 0,
    hard_request_budget INTEGER NOT NULL DEFAULT 0,
    used_requests INTEGER NOT NULL DEFAULT 0,
    lease_expires_at TEXT NOT NULL DEFAULT '',
    terminal_reason TEXT NOT NULL DEFAULT '',
    terminal_code TEXT NOT NULL DEFAULT '',
    terminal_state TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    started_at TEXT NOT NULL DEFAULT '',
    finished_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- A historical target position must never be reconstructed from today's URL list.
CREATE TABLE IF NOT EXISTS agent_evidence_locations (
    root_run_id TEXT PRIMARY KEY REFERENCES agent_runs(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    target_url TEXT NOT NULL,
    attempt_dir TEXT NOT NULL,
    target_dir TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS agent_evidence_locations_immutable
BEFORE UPDATE ON agent_evidence_locations BEGIN
    SELECT RAISE(ABORT, 'evidence_location_immutable');
END;

CREATE TABLE IF NOT EXISTS agent_gap_followups (
    scan_id TEXT PRIMARY KEY REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    request_id TEXT NOT NULL UNIQUE,
    request_hash TEXT NOT NULL,
    source_scan_id TEXT NOT NULL,
    assessment_message_id TEXT NOT NULL,
    source_hash TEXT NOT NULL,
    source_preview_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_agent_gap_followup_source ON agent_gap_followups(source_scan_id);
CREATE TABLE IF NOT EXISTS agent_gap_followup_submissions (
    request_id TEXT PRIMARY KEY,
    source_scan_id TEXT NOT NULL,
    assessment_message_id TEXT NOT NULL,
    input_json TEXT NOT NULL CHECK(json_valid(input_json)),
    input_hash TEXT NOT NULL,
    created_scan_id TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','released')),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_gap_submission_pending
ON agent_gap_followup_submissions(source_scan_id,assessment_message_id) WHERE state='pending';
CREATE TRIGGER IF NOT EXISTS agent_gap_submission_immutable
BEFORE UPDATE ON agent_gap_followup_submissions
WHEN NEW.request_id<>OLD.request_id OR NEW.source_scan_id<>OLD.source_scan_id
 OR NEW.assessment_message_id<>OLD.assessment_message_id OR NEW.input_json<>OLD.input_json
 OR NEW.input_hash<>OLD.input_hash OR NEW.created_at<>OLD.created_at
 OR (OLD.created_scan_id<>'' AND NEW.created_scan_id<>OLD.created_scan_id)
 OR (OLD.state='released' AND NEW.state<>'released') BEGIN
    SELECT RAISE(ABORT, 'followup_submission_immutable');
END;
CREATE TRIGGER IF NOT EXISTS agent_gap_followups_immutable
BEFORE UPDATE ON agent_gap_followups BEGIN
    SELECT RAISE(ABORT, 'followup_provenance_immutable');
END;

CREATE TABLE IF NOT EXISTS agent_gap_review_receipts (
    request_id TEXT PRIMARY KEY REFERENCES agent_review_requests(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL REFERENCES agent_gap_followups(scan_id) ON DELETE CASCADE,
    source_hash TEXT NOT NULL,
    assessment_json TEXT NOT NULL CHECK(json_valid(assessment_json)),
    resolved INTEGER NOT NULL CHECK(resolved IN (0,1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_agent_gap_review_scan ON agent_gap_review_receipts(scan_id);
CREATE TRIGGER IF NOT EXISTS agent_gap_review_immutable
BEFORE UPDATE ON agent_gap_review_receipts BEGIN
    SELECT RAISE(ABORT, 'followup_review_immutable');
END;

CREATE TABLE IF NOT EXISTS agent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    artifact_refs_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(run_id, sequence)
);

CREATE TABLE IF NOT EXISTS agent_messages (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    from_agent TEXT NOT NULL DEFAULT '',
    to_agent TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    correlation_id TEXT NOT NULL DEFAULT '',
    dedup_key TEXT NOT NULL DEFAULT '',
    payload_json TEXT NOT NULL DEFAULT '{}',
    artifact_refs_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    delivered_at TEXT NOT NULL DEFAULT '',
    acknowledged_at TEXT NOT NULL DEFAULT '',
    root_run_id TEXT NOT NULL DEFAULT '',
    from_run_id TEXT NOT NULL DEFAULT '',
    to_run_id TEXT NOT NULL DEFAULT '',
    assignment_id TEXT NOT NULL DEFAULT '',
    evidence_revision INTEGER NOT NULL DEFAULT 0,
    delivery_attempts INTEGER NOT NULL DEFAULT 0,
    UNIQUE(run_id, dedup_key)
);

CREATE TABLE IF NOT EXISTS tool_invocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    tool_name TEXT NOT NULL,
    tool_version INTEGER NOT NULL DEFAULT 1,
    contract_key TEXT NOT NULL DEFAULT '',
    identity_handle TEXT NOT NULL DEFAULT '',
    input_hash TEXT NOT NULL DEFAULT '',
    input_summary_json TEXT NOT NULL DEFAULT '{}',
    policy_decision TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'running',
    progress_signature TEXT NOT NULL DEFAULT '',
    request_artifact_id TEXT NOT NULL DEFAULT '',
    response_artifact_id TEXT NOT NULL DEFAULT '',
    error_class TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    finished_at TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS agent_snapshots (
    run_id TEXT PRIMARY KEY REFERENCES agent_runs(id) ON DELETE CASCADE,
    last_sequence INTEGER NOT NULL DEFAULT 0,
    schema_version INTEGER NOT NULL DEFAULT 1,
    snapshot_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- Native multi-agent control plane. Coordinator leases fence every orchestration
-- write; assignments own child runs, capability leases and reserved budget.
CREATE TABLE IF NOT EXISTS agent_assignments (
    id TEXT PRIMARY KEY,
    coordinator_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    child_run_id TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT 'coordinator',
    lane TEXT NOT NULL DEFAULT 'read_only_analysis',
    target_key TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'prepared',
    dedup_key TEXT NOT NULL,
    trigger_code TEXT NOT NULL DEFAULT '',
    task_slice_json TEXT NOT NULL DEFAULT '{}',
    evidence_revision INTEGER NOT NULL DEFAULT 0,
    contract_keys_json TEXT NOT NULL DEFAULT '[]',
    identity_handles_json TEXT NOT NULL DEFAULT '[]',
    reserved_tokens INTEGER NOT NULL DEFAULT 0,
    reserved_requests INTEGER NOT NULL DEFAULT 0,
    budget_settled_at TEXT NOT NULL DEFAULT '',
    capability_lease_json TEXT NOT NULL DEFAULT '[]',
    deadline_at TEXT NOT NULL DEFAULT '',
    failure_class TEXT NOT NULL DEFAULT '',
    lease_epoch INTEGER NOT NULL DEFAULT 0,
    fencing_token TEXT NOT NULL DEFAULT '',
    lease_expires_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    leased_at TEXT NOT NULL DEFAULT '',
    started_at TEXT NOT NULL DEFAULT '',
    finished_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(coordinator_run_id, dedup_key)
);

CREATE TABLE IF NOT EXISTS agent_coordinator_leases (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL,
    target_key TEXT NOT NULL DEFAULT '',
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    lease_epoch INTEGER NOT NULL DEFAULT 1,
    fencing_token TEXT NOT NULL,
    lease_expires_at TEXT NOT NULL,
    heartbeat_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number,target_key)
);

-- One assignment per scan/attempt/target/lane. Expiry of a coordinator or
-- capability lease does not free this slot: an interrupted target request may
-- already have reached the server. Only a fenced terminal transition releases it.
CREATE TABLE IF NOT EXISTS agent_lane_leases (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL,
    target_key TEXT NOT NULL,
    lane TEXT NOT NULL CHECK(lane IN ('target_touching','read_only_analysis','review')),
    assignment_id TEXT NOT NULL UNIQUE REFERENCES agent_assignments(id) ON DELETE CASCADE,
    acquired_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number,target_key,lane)
);

CREATE TABLE IF NOT EXISTS agent_budget_ledger (
    root_run_id TEXT PRIMARY KEY REFERENCES agent_runs(id) ON DELETE CASCADE,
    total_tokens INTEGER NOT NULL DEFAULT 0,
    total_requests INTEGER NOT NULL DEFAULT 0,
    reserved_tokens INTEGER NOT NULL DEFAULT 0,
    reserved_requests INTEGER NOT NULL DEFAULT 0,
    spent_tokens INTEGER NOT NULL DEFAULT 0,
    spent_requests INTEGER NOT NULL DEFAULT 0,
    lease_epoch INTEGER NOT NULL DEFAULT 0,
    fencing_token TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    CHECK(reserved_tokens >= 0 AND reserved_requests >= 0),
    CHECK(spent_tokens >= 0 AND spent_requests >= 0),
    CHECK(reserved_tokens + spent_tokens <= total_tokens OR total_tokens = 0),
    CHECK(reserved_requests + spent_requests <= total_requests OR total_requests = 0)
);

CREATE TABLE IF NOT EXISTS agent_capability_leases (
    id TEXT PRIMARY KEY,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    assignment_id TEXT NOT NULL REFERENCES agent_assignments(id) ON DELETE CASCADE,
    child_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    capability TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    fencing_token TEXT NOT NULL,
    lease_expires_at TEXT NOT NULL,
    revoked_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(assignment_id,child_run_id,capability)
);

-- §5.3 Loop5: one owner per contract across all child runs of a root.
-- `assignment_id` is intentionally not a foreign key: the owner is acquired in
-- the same scheduling transaction that inserts the assignment, so the row may
-- not exist yet when this row is written. Stale fencing must fail closed.
CREATE TABLE IF NOT EXISTS agent_contract_owners (
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    contract_key TEXT NOT NULL,
    assignment_id TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    fencing_token TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'held'
      CHECK(state IN ('held','released')),
    result_node_id TEXT NOT NULL DEFAULT '',
    acquired_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    released_at TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(root_run_id, contract_key)
);

CREATE TABLE IF NOT EXISTS agent_user_directives (
    id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL,
    root_run_id TEXT NOT NULL DEFAULT '',
    target_key TEXT NOT NULL DEFAULT '',
    recipient_role TEXT NOT NULL DEFAULT 'coordinator',
    thread_key TEXT NOT NULL DEFAULT 'team',
    text_redacted TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'pending'
      CHECK(status IN ('pending','claimed','accepted','rejected','deferred','assigned','applied','completed','failed')),
    claim_run_id TEXT NOT NULL DEFAULT '',
    claim_lease_epoch INTEGER NOT NULL DEFAULT 0,
    claim_fencing_token TEXT NOT NULL DEFAULT '',
    rejection_code TEXT NOT NULL DEFAULT '',
    source_draft_id TEXT NOT NULL DEFAULT '',
    confirmed_revision INTEGER NOT NULL DEFAULT 0,
    confirmed_hash TEXT NOT NULL DEFAULT '',
    confirmation_at TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    claimed_at TEXT NOT NULL DEFAULT '',
    accepted_at TEXT NOT NULL DEFAULT '',
    assigned_at TEXT NOT NULL DEFAULT '',
    applied_at TEXT NOT NULL DEFAULT '',
    finished_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- Human chat never enters the executable directive queue directly.  It is
-- interpreted and frozen here first; only a matching, explicitly confirmed
-- revision may create an agent_user_directives row.
CREATE TABLE IF NOT EXISTS agent_directive_drafts (
    id TEXT PRIMARY KEY,
    source_message_id TEXT NOT NULL UNIQUE,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL,
    root_run_id TEXT NOT NULL DEFAULT '',
    target_key TEXT NOT NULL DEFAULT '',
    recipient_role TEXT NOT NULL DEFAULT 'coordinator',
    thread_key TEXT NOT NULL DEFAULT 'team',
    text_redacted TEXT NOT NULL,
    intent TEXT NOT NULL DEFAULT 'priority_adjustment',
    requested_roles_json TEXT NOT NULL DEFAULT '[]',
    referenced_fact_ids_json TEXT NOT NULL DEFAULT '[]',
    requested_contracts_json TEXT NOT NULL DEFAULT '[]',
    priority_changes_json TEXT NOT NULL DEFAULT '[]',
    proposed_scope_change_json TEXT NOT NULL DEFAULT 'null',
    estimated_tokens INTEGER NOT NULL DEFAULT 0,
    estimated_requests INTEGER NOT NULL DEFAULT 0,
    side_effect_class TEXT NOT NULL DEFAULT 'read_only',
    required_approvals_json TEXT NOT NULL DEFAULT '[]',
    validation_result TEXT NOT NULL DEFAULT 'valid',
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    coordinator_decision TEXT NOT NULL DEFAULT 'accept',
    confirmation_required INTEGER NOT NULL DEFAULT 1,
    safe_execution_text TEXT NOT NULL DEFAULT '',
    revision INTEGER NOT NULL DEFAULT 1,
    draft_hash TEXT NOT NULL,
    bound_lease_epoch INTEGER NOT NULL DEFAULT 0,
    bound_fencing_token TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'drafted'
      CHECK(status IN ('drafted','need_confirmation','rejected','confirmed','cancelled','expired')),
    confirmed_directive_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    confirmed_at TEXT NOT NULL DEFAULT '',
    cancelled_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

-- Human guidance is frozen before each source phase. It never grants tools,
-- changes a previous model request, or inherits a replacement worker's lease.
CREATE TABLE IF NOT EXISTS agent_source_guidance (
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    phase_key TEXT NOT NULL,
    role TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    fencing_token TEXT NOT NULL,
    guidance_json TEXT NOT NULL,
    guidance_hash TEXT NOT NULL,
    PRIMARY KEY(root_run_id,phase_key)
);
CREATE TRIGGER IF NOT EXISTS agent_source_guidance_immutable
BEFORE UPDATE ON agent_source_guidance
BEGIN SELECT RAISE(ABORT,'source_guidance_immutable'); END;
CREATE TRIGGER IF NOT EXISTS agent_source_guidance_no_delete
BEFORE DELETE ON agent_source_guidance
WHEN EXISTS(SELECT 1 FROM agent_runs WHERE id=OLD.root_run_id)
BEGIN SELECT RAISE(ABORT,'source_guidance_immutable'); END;

-- One provider dispatch per readonly specialist worker. Unknown dispatches
-- are not retryable; received receipts are immutable accounting evidence.
CREATE TABLE IF NOT EXISTS agent_specialist_calls (
    assignment_id TEXT NOT NULL REFERENCES agent_assignments(id) ON DELETE CASCADE,
    child_run_id TEXT NOT NULL PRIMARY KEY REFERENCES agent_runs(id) ON DELETE CASCADE,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    lease_epoch INTEGER NOT NULL,
    fencing_token TEXT NOT NULL,
    request_json TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('executing','received','uncertain')),
    response_json TEXT NOT NULL DEFAULT '{}',
    usage_json TEXT NOT NULL DEFAULT '{}',
    response_hash TEXT NOT NULL DEFAULT '',
    event_sequence INTEGER NOT NULL DEFAULT 0,
    failure_code TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    finished_at TEXT NOT NULL DEFAULT ''
);
CREATE TRIGGER IF NOT EXISTS agent_specialist_call_immutable
BEFORE UPDATE ON agent_specialist_calls
WHEN OLD.state IN ('received','uncertain')
 OR NEW.assignment_id<>OLD.assignment_id OR NEW.child_run_id<>OLD.child_run_id
 OR NEW.root_run_id<>OLD.root_run_id OR NEW.role<>OLD.role
 OR NEW.lease_epoch<>OLD.lease_epoch OR NEW.fencing_token<>OLD.fencing_token
 OR NEW.request_json<>OLD.request_json OR NEW.request_hash<>OLD.request_hash
 OR NEW.created_at<>OLD.created_at
BEGIN SELECT RAISE(ABORT,'specialist_call_immutable'); END;

-- Canonical source decisions use typed identities, never Web numeric revisions.
-- A single FK cascade path avoids dependence on child/mailbox deletion order.
CREATE TABLE IF NOT EXISTS agent_source_review_decisions (
    id TEXT PRIMARY KEY,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    candidate_identity_json TEXT NOT NULL CHECK(json_valid(candidate_identity_json)),
    material_digest TEXT NOT NULL CHECK(length(material_digest)=64),
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    record_digest TEXT NOT NULL CHECK(length(record_digest)=64),
    UNIQUE(root_run_id,material_digest,candidate_identity_json)
);
CREATE TRIGGER IF NOT EXISTS source_decision_no_update BEFORE UPDATE ON agent_source_review_decisions
BEGIN SELECT RAISE(ABORT,'source_decision_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_decision_no_replace BEFORE INSERT ON agent_source_review_decisions
WHEN EXISTS(SELECT 1 FROM agent_source_review_decisions WHERE id=NEW.id OR
    (root_run_id=NEW.root_run_id AND material_digest=NEW.material_digest AND candidate_identity_json=NEW.candidate_identity_json))
BEGIN SELECT RAISE(ABORT,'source_decision_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_decision_no_delete BEFORE DELETE ON agent_source_review_decisions
WHEN EXISTS(SELECT 1 FROM sentinel_scans WHERE id=OLD.scan_id)
BEGIN SELECT RAISE(ABORT,'source_decision_immutable'); END;

-- Coverage has a separate subject and one immutable publication per root.
CREATE TABLE IF NOT EXISTS agent_source_coverage_decisions (
    id TEXT PRIMARY KEY,
    root_run_id TEXT NOT NULL UNIQUE REFERENCES agent_runs(id) ON DELETE CASCADE,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    material_digest TEXT NOT NULL CHECK(length(material_digest)=64),
    record_json TEXT NOT NULL CHECK(json_valid(record_json)),
    record_digest TEXT NOT NULL CHECK(length(record_digest)=64)
);
CREATE TRIGGER IF NOT EXISTS source_coverage_decision_no_update BEFORE UPDATE ON agent_source_coverage_decisions
BEGIN SELECT RAISE(ABORT,'source_coverage_decision_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_coverage_decision_no_replace BEFORE INSERT ON agent_source_coverage_decisions
WHEN EXISTS(SELECT 1 FROM agent_source_coverage_decisions WHERE id=NEW.id OR root_run_id=NEW.root_run_id)
BEGIN SELECT RAISE(ABORT,'source_coverage_decision_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_coverage_decision_no_delete BEFORE DELETE ON agent_source_coverage_decisions
WHEN EXISTS(SELECT 1 FROM sentinel_scans WHERE id=OLD.scan_id)
BEGIN SELECT RAISE(ABORT,'source_coverage_decision_immutable'); END;

-- Source tool phases have their own ordered journal. Do not weaken the
-- single, tool-free specialist receipt to accommodate additional rounds.
CREATE TABLE IF NOT EXISTS agent_source_model_rounds (
    assignment_id TEXT NOT NULL REFERENCES agent_assignments(id) ON DELETE CASCADE,
    round_number INTEGER NOT NULL CHECK(round_number > 0),
    child_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK(role IN ('repo_mapper','source_analyst')),
    lease_epoch INTEGER NOT NULL,
    fencing_token TEXT NOT NULL,
    request_json TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    reserved_tokens INTEGER NOT NULL CHECK(reserved_tokens > 0),
    state TEXT NOT NULL CHECK(state IN ('executing','received','uncertain')),
    response_json TEXT NOT NULL DEFAULT '{}',
    usage_json TEXT NOT NULL DEFAULT '{}',
    response_hash TEXT NOT NULL DEFAULT '',
    event_sequence INTEGER NOT NULL DEFAULT 0,
    failure_code TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    finished_at TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(assignment_id,child_run_id,round_number),
    UNIQUE(child_run_id,round_number)
);
CREATE TRIGGER IF NOT EXISTS agent_source_round_immutable
BEFORE UPDATE ON agent_source_model_rounds
WHEN OLD.state <> 'executing' OR NEW.state NOT IN ('received','uncertain')
 OR NEW.assignment_id<>OLD.assignment_id OR NEW.round_number<>OLD.round_number
 OR NEW.child_run_id<>OLD.child_run_id OR NEW.root_run_id<>OLD.root_run_id
 OR NEW.role<>OLD.role OR NEW.lease_epoch<>OLD.lease_epoch OR NEW.fencing_token<>OLD.fencing_token
 OR NEW.request_json<>OLD.request_json OR NEW.request_hash<>OLD.request_hash
 OR NEW.reserved_tokens<>OLD.reserved_tokens OR NEW.created_at<>OLD.created_at
BEGIN SELECT RAISE(ABORT,'source_round_immutable'); END;

CREATE TABLE IF NOT EXISTS agent_source_tool_receipts (
    assignment_id TEXT NOT NULL,
    child_run_id TEXT NOT NULL,
    round_number INTEGER NOT NULL,
    call_index INTEGER NOT NULL CHECK(call_index >= 0),
    call_id TEXT NOT NULL CHECK(length(call_id)>0),
    tool_name TEXT NOT NULL,
    arguments_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('planned','completed')),
    output_json TEXT NOT NULL DEFAULT '{}',
    receipt_hash TEXT NOT NULL DEFAULT '',
    event_sequence INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(assignment_id,child_run_id,round_number,call_index),
    UNIQUE(assignment_id,child_run_id,call_id),
    FOREIGN KEY(assignment_id,child_run_id,round_number) REFERENCES agent_source_model_rounds(assignment_id,child_run_id,round_number) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS agent_source_tool_receipt_immutable
BEFORE UPDATE ON agent_source_tool_receipts
WHEN OLD.state <> 'planned' OR NEW.state <> 'completed'
 OR NEW.assignment_id<>OLD.assignment_id OR NEW.round_number<>OLD.round_number
 OR NEW.child_run_id<>OLD.child_run_id
 OR NEW.call_index<>OLD.call_index OR NEW.call_id<>OLD.call_id
 OR NEW.tool_name<>OLD.tool_name OR NEW.arguments_json<>OLD.arguments_json
BEGIN SELECT RAISE(ABORT,'source_tool_receipt_immutable'); END;

CREATE TABLE IF NOT EXISTS agent_review_requests (
    id TEXT PRIMARY KEY,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    assignment_id TEXT NOT NULL REFERENCES agent_assignments(id) ON DELETE CASCADE,
    reviewer_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    candidate_id TEXT NOT NULL,
    candidate_revision INTEGER NOT NULL,
    candidate_json TEXT NOT NULL DEFAULT '{}',
    evidence_revision INTEGER NOT NULL DEFAULT 0,
    manifest_hash TEXT NOT NULL DEFAULT '',
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

CREATE TABLE IF NOT EXISTS agent_finding_candidates (
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

-- Operator-authored authorization controls are task/attempt scoped. Model
-- output and imported historical artifacts must never populate this table.
-- Rows are immutable: changing ownership requires a fresh attempt, so a
-- running verifier cannot silently inherit a rewritten control group.
CREATE TABLE IF NOT EXISTS agent_authorization_controls (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    target_url TEXT NOT NULL,
    contract_key TEXT NOT NULL,
    method TEXT NOT NULL CHECK(method = 'GET'),
    owner_object_url TEXT NOT NULL,
    tester_control_url TEXT NOT NULL,
    object_query_key TEXT NOT NULL,
    owner_object_value TEXT NOT NULL,
    tester_object_value TEXT NOT NULL,
    response_object_pointer TEXT NOT NULL,
    owner_identity TEXT NOT NULL,
    tester_identity TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number,target_url,contract_key)
);

-- A public entry claim is committed before network dispatch. A received
-- observation and its scope binding are immutable through ordinary writes.
-- Generic HTTP accounting has a frozen historical baseline, followed by
-- individually committed claims. Checkpoints are projections, not a second
-- source of spend after this boundary has been installed.
CREATE TABLE IF NOT EXISTS agent_http_budget_origins (
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    target_url TEXT NOT NULL,
    budget_attempt INTEGER NOT NULL CHECK(budget_attempt > 0),
    baseline_requests INTEGER NOT NULL CHECK(baseline_requests >= 0),
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK(schema_version=1),
    PRIMARY KEY(scan_id,target_url,budget_attempt)
);
CREATE TRIGGER IF NOT EXISTS immutable_http_budget_origin
BEFORE UPDATE ON agent_http_budget_origins
BEGIN SELECT RAISE(ABORT,'immutable_http_budget_origin'); END;
CREATE TABLE IF NOT EXISTS agent_http_request_claims (
    scan_id TEXT NOT NULL,
    target_url TEXT NOT NULL,
    budget_attempt INTEGER NOT NULL,
    ordinal INTEGER NOT NULL CHECK(ordinal > 0),
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    invocation_id TEXT NOT NULL,
    request_index INTEGER NOT NULL CHECK(request_index > 0),
    tool_name TEXT NOT NULL,
    identity_handle TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    response_status INTEGER NOT NULL DEFAULT 0 CHECK(response_status=0 OR response_status BETWEEN 100 AND 599),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    received_at TEXT NOT NULL DEFAULT '',
    PRIMARY KEY(scan_id,target_url,budget_attempt,ordinal),
    UNIQUE(run_id,invocation_id,request_index),
    FOREIGN KEY(scan_id,target_url,budget_attempt)
      REFERENCES agent_http_budget_origins(scan_id,target_url,budget_attempt) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS immutable_http_request_claim
BEFORE UPDATE ON agent_http_request_claims
WHEN OLD.response_status<>0 OR NEW.response_status=0 OR NEW.received_at=''
 OR NEW.scan_id<>OLD.scan_id OR NEW.target_url<>OLD.target_url
 OR NEW.budget_attempt<>OLD.budget_attempt OR NEW.ordinal<>OLD.ordinal
 OR NEW.attempt_number<>OLD.attempt_number OR NEW.run_id<>OLD.run_id
 OR NEW.invocation_id<>OLD.invocation_id OR NEW.request_index<>OLD.request_index
 OR NEW.tool_name<>OLD.tool_name OR NEW.identity_handle<>OLD.identity_handle
 OR NEW.request_hash<>OLD.request_hash OR NEW.created_at<>OLD.created_at
BEGIN SELECT RAISE(ABORT,'immutable_http_request_claim'); END;

-- Deny-only target fuse. It survives later attempts of this same scan; it
-- confers no original response signature, SDK permission or recovery grant.
CREATE TABLE IF NOT EXISTS agent_protection_stops (
    scan_id TEXT NOT NULL CHECK(length(scan_id)>0),
    target_url TEXT NOT NULL CHECK(length(target_url)>0),
    source_attempt_number INTEGER NOT NULL CHECK(source_attempt_number>0),
    source_run_id TEXT NOT NULL CHECK(length(source_run_id)>0),
    stop_code TEXT NOT NULL CHECK(stop_code IN ('confirmed_waf_or_challenge','persistent_rate_limit')),
    reason TEXT NOT NULL CHECK(length(reason)>0 AND length(reason)<=2000),
    claim_json TEXT NOT NULL CHECK(json_valid(claim_json)),
    claim_hash TEXT NOT NULL CHECK(length(claim_hash)=64),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,target_url)
);
CREATE TRIGGER IF NOT EXISTS protection_stop_no_update
BEFORE UPDATE ON agent_protection_stops
BEGIN SELECT RAISE(ABORT,'protection_stop_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_stop_no_delete
BEFORE DELETE ON agent_protection_stops
BEGIN SELECT RAISE(ABORT,'protection_stop_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_stop_no_replace
BEFORE INSERT ON agent_protection_stops
WHEN EXISTS(SELECT 1 FROM agent_protection_stops old WHERE old.rowid=NEW.rowid
    OR (old.scan_id=NEW.scan_id AND old.target_url=NEW.target_url))
BEGIN SELECT RAISE(ABORT,'protection_stop_immutable'); END;

CREATE TABLE IF NOT EXISTS agent_external_surface_captures (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    target_url TEXT NOT NULL,
    assignment_id TEXT NOT NULL UNIQUE REFERENCES agent_assignments(id) ON DELETE CASCADE,
    child_run_id TEXT NOT NULL UNIQUE REFERENCES agent_runs(id) ON DELETE CASCADE,
    state TEXT NOT NULL CHECK(state IN ('claimed','received')),
    artifact_id TEXT NOT NULL DEFAULT '',
    response_json TEXT NOT NULL DEFAULT '{}',
    response_hash TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number,target_url)
);
CREATE TRIGGER IF NOT EXISTS immutable_external_surface_capture
BEFORE UPDATE ON agent_external_surface_captures
WHEN OLD.state='received' OR NEW.scan_id<>OLD.scan_id OR NEW.attempt_number<>OLD.attempt_number
 OR NEW.target_url<>OLD.target_url OR NEW.assignment_id<>OLD.assignment_id
 OR NEW.child_run_id<>OLD.child_run_id OR NEW.created_at<>OLD.created_at
BEGIN SELECT RAISE(ABORT,'immutable_external_surface_capture'); END;

-- A claim is committed before a target request. A crashed or fenced child
-- cannot silently repeat a side and spend the same authorization budget twice.
CREATE TABLE IF NOT EXISTS agent_authorization_probe_claims (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    target_url TEXT NOT NULL,
    contract_key TEXT NOT NULL,
    side TEXT NOT NULL CHECK(side IN ('owner','cross','tester')),
    child_run_id TEXT NOT NULL,
    assignment_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number,target_url,contract_key,side),
    FOREIGN KEY(scan_id,attempt_number,target_url,contract_key)
      REFERENCES agent_authorization_controls(scan_id,attempt_number,target_url,contract_key)
);

-- First-response Authorization seals are append-only. This table grants no
-- authority; the private issuer must validate and sign the original facts.
-- No migration/backfill turns existing artifacts into first-write receipts.
CREATE TABLE IF NOT EXISTS agent_authorization_side_receipts (
    storage_id INTEGER PRIMARY KEY CHECK(storage_id>0),
    receipt_id TEXT NOT NULL UNIQUE CHECK(length(receipt_id)>0),
    root_run_id TEXT NOT NULL CHECK(length(root_run_id)>0),
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    target_url TEXT NOT NULL,
    contract_key TEXT NOT NULL,
    side TEXT NOT NULL CHECK(side IN ('owner','cross','tester')),
    claim_rowid INTEGER NOT NULL CHECK(claim_rowid>0),
    assignment_id TEXT NOT NULL,
    child_run_id TEXT NOT NULL,
    worker_attempt_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL CHECK(length(artifact_id)>0),
    fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
    fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64),
    signature BLOB NOT NULL CHECK(typeof(signature)='blob' AND length(signature)=32),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,attempt_number,target_url,contract_key,side),
    UNIQUE(claim_rowid),
    UNIQUE(root_run_id,artifact_id)
);
CREATE TRIGGER IF NOT EXISTS authorization_side_receipt_no_update
BEFORE UPDATE ON agent_authorization_side_receipts
BEGIN SELECT RAISE(ABORT,'authorization_side_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS authorization_side_receipt_no_delete
BEFORE DELETE ON agent_authorization_side_receipts
BEGIN SELECT RAISE(ABORT,'authorization_side_receipt_immutable'); END;
-- REPLACE may bypass delete triggers when recursive_triggers is off. Reject
-- every conflicting physical or logical identity before conflict resolution.
CREATE TRIGGER IF NOT EXISTS authorization_side_receipt_no_replace
BEFORE INSERT ON agent_authorization_side_receipts
WHEN EXISTS(SELECT 1 FROM agent_authorization_side_receipts old
    WHERE old.rowid=NEW.rowid OR old.receipt_id=NEW.receipt_id
       OR old.claim_rowid=NEW.claim_rowid
       OR (old.root_run_id=NEW.root_run_id AND old.artifact_id=NEW.artifact_id)
       OR (old.scan_id=NEW.scan_id AND old.attempt_number=NEW.attempt_number
           AND old.target_url=NEW.target_url AND old.contract_key=NEW.contract_key AND old.side=NEW.side))
BEGIN SELECT RAISE(ABORT,'authorization_side_receipt_immutable'); END;

-- Human attestations never replace wire receipts or authorize replay. Revisions
-- are append-only; the source snapshot remains available after a late receipt.
CREATE TABLE IF NOT EXISTS agent_request_reviews (
    id TEXT PRIMARY KEY,
    scan_id TEXT NOT NULL REFERENCES sentinel_scans(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL CHECK(attempt_number > 0),
    target_url TEXT NOT NULL,
    request_key TEXT NOT NULL,
    previous_review_id TEXT NOT NULL DEFAULT '',
    source_snapshot_json TEXT NOT NULL CHECK(json_valid(source_snapshot_json)),
    source_snapshot_hash TEXT NOT NULL,
    disposition TEXT NOT NULL CHECK(disposition IN ('effect_observed','not_sent_attested','still_unknown')),
    note TEXT NOT NULL,
    actor TEXT NOT NULL CHECK(actor='local_operator'),
    created_at TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    UNIQUE(scan_id,target_url,request_key,previous_review_id)
);
CREATE TRIGGER IF NOT EXISTS immutable_agent_request_review
BEFORE UPDATE ON agent_request_reviews
BEGIN SELECT RAISE(ABORT,'immutable_request_review'); END;

CREATE TABLE IF NOT EXISTS agent_evidence_nodes (
    id TEXT PRIMARY KEY,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 1,
    kind TEXT NOT NULL DEFAULT 'target',
    provenance TEXT NOT NULL DEFAULT 'observed',
    natural_key_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL DEFAULT '{}',
    artifact_refs_json TEXT NOT NULL DEFAULT '[]',
    created_by_run_id TEXT NOT NULL DEFAULT '',
    supersedes_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(root_run_id, revision, natural_key_hash)
);

CREATE TABLE IF NOT EXISTS agent_evidence_edges (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 1,
    from_node_id TEXT NOT NULL REFERENCES agent_evidence_nodes(id) ON DELETE CASCADE,
    to_node_id TEXT NOT NULL REFERENCES agent_evidence_nodes(id) ON DELETE CASCADE,
    kind TEXT NOT NULL DEFAULT 'derived_from',
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_by_run_id TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(root_run_id, revision, from_node_id, to_node_id, kind)
);

-- Incremental, non-copying ancestry. Review-specific snapshots are sealed in
-- agent_review_requests: several candidates may share this evidence head.
-- The revision-level manifest_hash is reserved for a later graph-only seal.
CREATE TABLE IF NOT EXISTS agent_evidence_revisions (
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    parent_revision INTEGER CHECK(parent_revision IS NULL OR parent_revision < revision),
    cause_event_id TEXT NOT NULL DEFAULT '',
    manifest_hash TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(root_run_id, revision),
    FOREIGN KEY(root_run_id, parent_revision)
      REFERENCES agent_evidence_revisions(root_run_id, revision) DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS agent_review_decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_run_id TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    candidate_id TEXT NOT NULL,
    candidate_revision INTEGER NOT NULL DEFAULT 1,
    reviewer_run_id TEXT NOT NULL,
    verdict TEXT NOT NULL DEFAULT 'insufficient_evidence',
    reason_codes_json TEXT NOT NULL DEFAULT '[]',
    evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    counter_evidence_refs_json TEXT NOT NULL DEFAULT '[]',
    missing_evidence_json TEXT NOT NULL DEFAULT '[]',
    confidence REAL NOT NULL DEFAULT 0,
    severity TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(candidate_id, candidate_revision, reviewer_run_id)
);

CREATE TABLE IF NOT EXISTS agent_role_config_drafts (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT 'coordinator',
    objective TEXT NOT NULL DEFAULT '',
    capability_bundle_ids_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'draft',
    status_label TEXT NOT NULL DEFAULT '配置草稿，尚不能执行',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);

CREATE INDEX IF NOT EXISTS idx_agent_role_config_drafts_updated
    ON agent_role_config_drafts(updated_at DESC, id);

CREATE INDEX IF NOT EXISTS idx_agent_runs_scan ON agent_runs(scan_id,attempt_number,target_url);
CREATE INDEX IF NOT EXISTS idx_agent_runs_parent ON agent_runs(parent_run_id,status);
CREATE INDEX IF NOT EXISTS idx_agent_runs_lease ON agent_runs(status,lease_expires_at);
CREATE INDEX IF NOT EXISTS idx_agent_messages_run_delivered ON agent_messages(run_id,delivered_at,created_at);
-- Installed mailboxes can predate to_run_id. The orchestration migration
-- creates this index only after ensuring the recipient columns exist.
CREATE INDEX IF NOT EXISTS idx_tool_invocations_run_status ON tool_invocations(run_id,status,started_at DESC);
CREATE INDEX IF NOT EXISTS idx_tool_invocations_contract ON tool_invocations(run_id,contract_key);
CREATE INDEX IF NOT EXISTS idx_agent_assignments_coordinator ON agent_assignments(coordinator_run_id,state,lane);
CREATE INDEX IF NOT EXISTS idx_agent_assignments_child ON agent_assignments(child_run_id);
CREATE INDEX IF NOT EXISTS idx_agent_directives_scope ON agent_user_directives(scan_id,attempt_number,status,created_at);
CREATE INDEX IF NOT EXISTS idx_agent_directive_drafts_scope ON agent_directive_drafts(scan_id,attempt_number,status,created_at);
CREATE INDEX IF NOT EXISTS idx_agent_review_root ON agent_review_requests(root_run_id,status,created_at);
CREATE INDEX IF NOT EXISTS idx_agent_capability_child ON agent_capability_leases(child_run_id,revoked_at,lease_expires_at);
CREATE INDEX IF NOT EXISTS idx_agent_finding_candidates_review ON agent_finding_candidates(root_run_id,candidate_revision,status);
CREATE INDEX IF NOT EXISTS idx_agent_finding_candidates_projection ON agent_finding_candidates(scan_id,target_url,stage,kind,record_key,status);
-- The two endpoint triggers are installed by migrate_evidence_revision_schema
-- on every open. CREATE TRIGGER IF NOT EXISTS would leave an upgraded database
-- with its old same-revision-only guards, so installation is transactional.

CREATE INDEX IF NOT EXISTS idx_agent_evidence_nodes_revision ON agent_evidence_nodes(root_run_id,revision,kind);
CREATE INDEX IF NOT EXISTS idx_agent_evidence_edges_revision ON agent_evidence_edges(root_run_id,revision,from_node_id);
CREATE INDEX IF NOT EXISTS idx_agent_evidence_revisions_parent ON agent_evidence_revisions(root_run_id,parent_revision);
CREATE INDEX IF NOT EXISTS idx_agent_review_candidate ON agent_review_decisions(candidate_id,candidate_revision);
CREATE TABLE IF NOT EXISTS artifact_objects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    content_hash TEXT NOT NULL UNIQUE,
    bytes INTEGER NOT NULL DEFAULT 0,
    storage_path TEXT NOT NULL DEFAULT '',
    display_text TEXT NOT NULL DEFAULT '',
    secret_material INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS import_sources (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    root_path TEXT NOT NULL UNIQUE,
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS import_bundles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bundle_id TEXT NOT NULL UNIQUE,
    source_path TEXT NOT NULL,
    root_path TEXT NOT NULL,
    attempt_key TEXT NOT NULL DEFAULT '',
    attempt_number INTEGER NOT NULL DEFAULT 0,
    signature TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'imported',
    record_count INTEGER NOT NULL DEFAULT 0,
    revision_count INTEGER NOT NULL DEFAULT 0,
    first_seen_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    last_import_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TABLE IF NOT EXISTS import_bundle_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bundle_row_id INTEGER NOT NULL REFERENCES import_bundles(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    bytes INTEGER NOT NULL DEFAULT 0,
    artifact_object_id INTEGER REFERENCES artifact_objects(id),
    UNIQUE(bundle_row_id,relative_path)
);
CREATE TABLE IF NOT EXISTS import_record_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    record_key TEXT NOT NULL,
    logical_key TEXT NOT NULL,
    record_kind TEXT NOT NULL,
    revision_hash TEXT NOT NULL,
    adapter TEXT NOT NULL,
    envelope_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(record_key,revision_hash)
);
CREATE TABLE IF NOT EXISTS import_projection_memberships (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scope_key TEXT NOT NULL,
    source_path TEXT NOT NULL DEFAULT '',
    record_key TEXT NOT NULL,
    logical_key TEXT NOT NULL,
    revision_id INTEGER NOT NULL REFERENCES import_record_revisions(id),
    bundle_row_id INTEGER NOT NULL REFERENCES import_bundles(id) ON DELETE CASCADE,
    attempt_number INTEGER NOT NULL DEFAULT 0,
    current INTEGER NOT NULL DEFAULT 1,
    tombstone INTEGER NOT NULL DEFAULT 0,
    adopted_from TEXT NOT NULL DEFAULT '',
    added_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    removed_at TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS import_diagnostics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bundle_row_id INTEGER REFERENCES import_bundles(id) ON DELETE CASCADE,
    root_path TEXT NOT NULL DEFAULT '',
    source_path TEXT NOT NULL DEFAULT '',
    code TEXT NOT NULL,
    severity TEXT NOT NULL,
    record_pointer TEXT NOT NULL DEFAULT '',
    detail TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_import_bundle_files_bundle ON import_bundle_files(bundle_row_id,relative_path);
CREATE INDEX IF NOT EXISTS idx_import_revisions_record ON import_record_revisions(record_key,revision_hash);
CREATE INDEX IF NOT EXISTS idx_import_memberships_scope ON import_projection_memberships(scope_key,current,logical_key);
CREATE INDEX IF NOT EXISTS idx_import_diagnostics_bundle ON import_diagnostics(bundle_row_id,code);
-- §9.9: original revisions are append-only. The database itself refuses a rewrite or
-- a delete, so "history is kept" is not a convention a caller can forget.
CREATE TRIGGER IF NOT EXISTS import_revisions_are_append_only
BEFORE UPDATE ON import_record_revisions
BEGIN
    SELECT RAISE(ABORT, 'import_record_revisions 只允许追加，不得改写历史修订');
END;
CREATE TRIGGER IF NOT EXISTS import_revisions_are_undeletable
BEFORE DELETE ON import_record_revisions
BEGIN
    SELECT RAISE(ABORT, 'import_record_revisions 只允许追加，不得删除历史修订');
END;
-- §10.2: the frozen snapshot a native code/greybox/CI run was bound to. Every result
-- cites the tree hash, so a resume can prove it is looking at the same source.
CREATE TABLE IF NOT EXISTS native_scan_branches (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    branch TEXT NOT NULL CHECK(branch IN ('source','web')),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','completed','completed_with_gaps','partial','failed')),
    checkpoint TEXT NOT NULL DEFAULT '',
    report_json TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number,branch)
);
-- Never backfill old pending branches as unclaimed: an earlier process may
-- already have sent requests. This receipt precedes every new branch effect.
CREATE TABLE IF NOT EXISTS native_branch_dispatches (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    branch TEXT NOT NULL CHECK(branch IN ('source','web')),
    claim_id TEXT NOT NULL DEFAULT '',
    claimed_at TEXT NOT NULL DEFAULT '',
    CHECK((claim_id='' AND claimed_at='') OR (claim_id<>'' AND claimed_at<>'')),
    PRIMARY KEY(scan_id,attempt_number,branch),
    FOREIGN KEY(scan_id,attempt_number,branch)
        REFERENCES native_scan_branches(scan_id,attempt_number,branch) ON DELETE CASCADE
);
-- Startup configuration binding, not a replay authorization or a liveness
-- record. Never synthesize a binding for a historical attempt.
CREATE TABLE IF NOT EXISTS native_web_dispatch_bindings (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    branch TEXT NOT NULL DEFAULT 'web' CHECK(branch='web'),
    schema_version INTEGER NOT NULL CHECK(schema_version=1),
    binding_tag BLOB NOT NULL CHECK(typeof(binding_tag)='blob' AND length(binding_tag)=32),
    PRIMARY KEY(scan_id,attempt_number),
    FOREIGN KEY(scan_id,attempt_number,branch)
        REFERENCES native_branch_dispatches(scan_id,attempt_number,branch) ON DELETE CASCADE
);
-- Explicit operator closure is separate from dispatch ownership. Never turn a
-- closed attempt back into an executable one or erase its empty/claimed receipt.
CREATE TABLE IF NOT EXISTS native_web_attempt_closures (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    branch TEXT NOT NULL DEFAULT 'web' CHECK(branch='web'),
    id TEXT NOT NULL UNIQUE,
    closed_at TEXT NOT NULL CHECK(closed_at<>''),
    terminal_timestamp TEXT NOT NULL CHECK(terminal_timestamp<>''),
    preservation_hash TEXT NOT NULL CHECK(length(preservation_hash)=64),
    reason TEXT NOT NULL CHECK(reason='operator_closed_before_dispatch'),
    PRIMARY KEY(scan_id,attempt_number),
    FOREIGN KEY(scan_id,attempt_number,branch)
        REFERENCES native_branch_dispatches(scan_id,attempt_number,branch) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS native_web_attempt_closures_immutable
BEFORE UPDATE ON native_web_attempt_closures BEGIN
    SELECT RAISE(ABORT,'web_closure_receipt_immutable');
END;
-- Administrative closure is NOT a machine execution result. Preserve the
-- original attempts, requests and reservations, and seal this scan identity.
CREATE TABLE IF NOT EXISTS native_web_administrative_closures (
    scan_id TEXT PRIMARY KEY REFERENCES sentinel_scans(id) ON DELETE RESTRICT,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    id TEXT NOT NULL UNIQUE,
    closed_at TEXT NOT NULL,
    previous_status TEXT NOT NULL,
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    snapshot_hash TEXT NOT NULL CHECK(length(snapshot_hash)=64),
    actor TEXT NOT NULL CHECK(actor='local_operator'),
    FOREIGN KEY(scan_id,attempt_number) REFERENCES sentinel_scan_attempts(scan_id,attempt_number)
);
CREATE TRIGGER IF NOT EXISTS administrative_closure_immutable
BEFORE UPDATE ON native_web_administrative_closures BEGIN
    SELECT RAISE(ABORT,'administrative_closure_immutable');
END;
CREATE TRIGGER IF NOT EXISTS administrative_closure_no_replace
BEFORE INSERT ON native_web_administrative_closures
WHEN EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=NEW.scan_id OR id=NEW.id)
BEGIN SELECT RAISE(ABORT,'administrative_closure_immutable'); END;
CREATE TRIGGER IF NOT EXISTS administrative_closure_retained
BEFORE DELETE ON native_web_administrative_closures BEGIN
    SELECT RAISE(ABORT,'administrative_closure_retained');
END;
CREATE TRIGGER IF NOT EXISTS administrative_closure_scan_sealed
BEFORE UPDATE ON sentinel_scans
WHEN EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=OLD.id)
 AND (NEW.id<>OLD.id OR NEW.status<>'cancelled' OR NEW.attempt_count<>OLD.attempt_count
      OR NEW.project_id<>OLD.project_id OR NEW.scan_type<>OLD.scan_type OR NEW.source_path<>OLD.source_path)
BEGIN SELECT RAISE(ABORT,'administrative_closure_requires_independent_task'); END;
CREATE TRIGGER IF NOT EXISTS administrative_closure_attempt_insert
BEFORE INSERT ON sentinel_scan_attempts
WHEN EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=NEW.scan_id)
BEGIN SELECT RAISE(ABORT,'administrative_closure_requires_independent_task'); END;
CREATE TRIGGER IF NOT EXISTS administrative_closure_attempt_update
BEFORE UPDATE ON sentinel_scan_attempts
WHEN EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=OLD.scan_id OR scan_id=NEW.scan_id)
BEGIN SELECT RAISE(ABORT,'administrative_closure_attempt_retained'); END;
CREATE TRIGGER IF NOT EXISTS administrative_closure_attempt_delete
BEFORE DELETE ON sentinel_scan_attempts
WHEN EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=OLD.scan_id)
BEGIN SELECT RAISE(ABORT,'administrative_closure_attempt_retained'); END;
CREATE TRIGGER IF NOT EXISTS administrative_closure_run_insert
BEFORE INSERT ON agent_runs
WHEN EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=NEW.scan_id)
BEGIN SELECT RAISE(ABORT,'administrative_closure_requires_independent_task'); END;
CREATE TRIGGER IF NOT EXISTS administrative_closure_coordinator_insert
BEFORE INSERT ON agent_coordinator_leases
WHEN EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=NEW.scan_id)
BEGIN SELECT RAISE(ABORT,'administrative_closure_requires_independent_task'); END;
CREATE TRIGGER IF NOT EXISTS administrative_closure_coordinator_update
BEFORE UPDATE ON agent_coordinator_leases
WHEN EXISTS(SELECT 1 FROM native_web_administrative_closures WHERE scan_id=OLD.scan_id OR scan_id=NEW.scan_id)
BEGIN SELECT RAISE(ABORT,'administrative_closure_requires_independent_task'); END;
-- One canonical handoff per sealed task. This is provenance, not an execution
-- grant. Keeping both endpoints prevents response-loss retries from recreating
-- a deleted draft and preserves unresolved source obligations.
CREATE TABLE IF NOT EXISTS native_web_closure_handoffs (
    source_scan_id TEXT PRIMARY KEY REFERENCES native_web_administrative_closures(scan_id) ON DELETE RESTRICT,
    closure_id TEXT NOT NULL UNIQUE REFERENCES native_web_administrative_closures(id),
    scan_id TEXT NOT NULL UNIQUE REFERENCES sentinel_scans(id) ON DELETE RESTRICT,
    request_id TEXT NOT NULL UNIQUE,
    input_json TEXT NOT NULL CHECK(json_valid(input_json)),
    input_hash TEXT NOT NULL CHECK(length(input_hash)=64),
    preview_json TEXT NOT NULL CHECK(json_valid(preview_json)),
    created_at TEXT NOT NULL
);
CREATE TRIGGER IF NOT EXISTS closure_handoff_immutable
BEFORE UPDATE ON native_web_closure_handoffs BEGIN
    SELECT RAISE(ABORT,'closure_handoff_immutable');
END;
CREATE TRIGGER IF NOT EXISTS closure_handoff_retained
BEFORE DELETE ON native_web_closure_handoffs BEGIN
    SELECT RAISE(ABORT,'closure_handoff_retained');
END;
CREATE TRIGGER IF NOT EXISTS closure_handoff_no_replace
BEFORE INSERT ON native_web_closure_handoffs
WHEN EXISTS(SELECT 1 FROM native_web_closure_handoffs WHERE source_scan_id=NEW.source_scan_id
    OR closure_id=NEW.closure_id OR scan_id=NEW.scan_id OR request_id=NEW.request_id)
BEGIN SELECT RAISE(ABORT,'closure_handoff_immutable'); END;
CREATE TABLE IF NOT EXISTS source_scope_contracts (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    source_path TEXT NOT NULL CHECK(length(source_path)>0),
    canonical_root TEXT NOT NULL CHECK(length(canonical_root)>0),
    scan_type TEXT NOT NULL CHECK(scan_type IN ('code','greybox','cicd')),
    scope_mode TEXT NOT NULL CHECK(scope_mode IN ('full','diff','auto')),
    diff_base TEXT NOT NULL CHECK(length(diff_base)<=1024),
    analysis_policy_version INTEGER NOT NULL DEFAULT 0 CHECK(analysis_policy_version IN (0,1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number),
    FOREIGN KEY(scan_id,attempt_number) REFERENCES sentinel_scan_attempts(scan_id,attempt_number) ON DELETE CASCADE,
    CHECK(scope_mode<>'full' OR diff_base=''),
    CHECK(scope_mode<>'diff' OR length(trim(diff_base))>0)
);
CREATE TRIGGER IF NOT EXISTS source_scope_no_update
BEFORE UPDATE ON source_scope_contracts
BEGIN SELECT RAISE(ABORT,'source_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_scope_no_replace
BEFORE INSERT ON source_scope_contracts
WHEN EXISTS(SELECT 1 FROM source_scope_contracts WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)
BEGIN SELECT RAISE(ABORT,'source_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_scope_no_delete
BEFORE DELETE ON source_scope_contracts
WHEN EXISTS(SELECT 1 FROM sentinel_scan_attempts WHERE scan_id=OLD.scan_id AND attempt_number=OLD.attempt_number)
BEGIN SELECT RAISE(ABORT,'source_scope_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_scope_current_attempt
BEFORE INSERT ON source_scope_contracts
WHEN NOT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=NEW.scan_id AND attempt_count=NEW.attempt_number
    AND status='scanning' AND scan_type=NEW.scan_type AND source_path=NEW.source_path)
BEGIN SELECT RAISE(ABORT,'source_scope_binding_mismatch'); END;
CREATE TABLE IF NOT EXISTS source_runtime_contracts (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    schema_version INTEGER NOT NULL CHECK(schema_version=1),
    contract_json TEXT NOT NULL CHECK(json_valid(contract_json) AND json_type(contract_json)='object'),
    binding_tag BLOB NOT NULL CHECK(length(binding_tag)=32),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number),
    FOREIGN KEY(scan_id,attempt_number) REFERENCES source_scope_contracts(scan_id,attempt_number) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS source_runtime_no_update
BEFORE UPDATE ON source_runtime_contracts
BEGIN SELECT RAISE(ABORT,'source_runtime_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_runtime_no_replace
BEFORE INSERT ON source_runtime_contracts
WHEN EXISTS(SELECT 1 FROM source_runtime_contracts WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)
BEGIN SELECT RAISE(ABORT,'source_runtime_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_runtime_no_delete
BEFORE DELETE ON source_runtime_contracts
WHEN EXISTS(SELECT 1 FROM source_scope_contracts WHERE scan_id=OLD.scan_id AND attempt_number=OLD.attempt_number)
BEGIN SELECT RAISE(ABORT,'source_runtime_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_runtime_current_attempt
BEFORE INSERT ON source_runtime_contracts
WHEN NOT EXISTS(SELECT 1 FROM sentinel_scans s JOIN native_branch_dispatches d ON d.scan_id=s.id AND d.attempt_number=s.attempt_count
    WHERE s.id=NEW.scan_id AND s.attempt_count=NEW.attempt_number AND s.status='scanning'
    AND d.branch='source' AND d.claim_id='' AND d.claimed_at='')
BEGIN SELECT RAISE(ABORT,'source_runtime_scope_mismatch'); END;
CREATE TABLE IF NOT EXISTS source_analysis_views (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    manifest_json TEXT NOT NULL CHECK(json_valid(manifest_json)),
    manifest_digest TEXT NOT NULL CHECK(length(manifest_digest)=64),
    view_root TEXT NOT NULL CHECK(length(view_root)>0),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number),
    FOREIGN KEY(scan_id,attempt_number) REFERENCES source_scope_contracts(scan_id,attempt_number) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS analysis_view_no_update BEFORE UPDATE ON source_analysis_views
BEGIN SELECT RAISE(ABORT,'analysis_view_immutable'); END;
CREATE TRIGGER IF NOT EXISTS analysis_view_no_replace BEFORE INSERT ON source_analysis_views
WHEN EXISTS(SELECT 1 FROM source_analysis_views WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)
BEGIN SELECT RAISE(ABORT,'analysis_view_immutable'); END;
CREATE TRIGGER IF NOT EXISTS analysis_view_no_delete BEFORE DELETE ON source_analysis_views
WHEN EXISTS(SELECT 1 FROM source_scope_contracts WHERE scan_id=OLD.scan_id AND attempt_number=OLD.attempt_number)
BEGIN SELECT RAISE(ABORT,'analysis_view_immutable'); END;
CREATE TABLE IF NOT EXISTS source_analysis_results (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    analysis_digest TEXT NOT NULL CHECK(length(analysis_digest)=64),
    receipt_json TEXT NOT NULL CHECK(json_valid(receipt_json)),
    receipt_digest TEXT NOT NULL CHECK(length(receipt_digest)=64),
    PRIMARY KEY(scan_id,attempt_number),
    FOREIGN KEY(scan_id,attempt_number) REFERENCES source_analysis_views(scan_id,attempt_number) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS analysis_results_no_update BEFORE UPDATE ON source_analysis_results
BEGIN SELECT RAISE(ABORT,'analysis_results_immutable'); END;
CREATE TRIGGER IF NOT EXISTS analysis_results_no_replace BEFORE INSERT ON source_analysis_results
WHEN EXISTS(SELECT 1 FROM source_analysis_results WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)
BEGIN SELECT RAISE(ABORT,'analysis_results_immutable'); END;
CREATE TRIGGER IF NOT EXISTS analysis_results_no_delete BEFORE DELETE ON source_analysis_results
WHEN EXISTS(SELECT 1 FROM source_analysis_views WHERE scan_id=OLD.scan_id AND attempt_number=OLD.attempt_number)
BEGIN SELECT RAISE(ABORT,'analysis_results_immutable'); END;
CREATE TABLE IF NOT EXISTS source_ci_policies (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
    max_critical INTEGER NOT NULL CHECK(typeof(max_critical)='integer' AND max_critical BETWEEN 0 AND 10000),
    max_high INTEGER NOT NULL CHECK(typeof(max_high)='integer' AND max_high BETWEEN 0 AND 10000),
    block_release INTEGER NOT NULL CHECK(block_release IN (0,1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number),
    FOREIGN KEY(scan_id,attempt_number) REFERENCES sentinel_scan_attempts(scan_id,attempt_number) ON DELETE CASCADE
);
CREATE TRIGGER IF NOT EXISTS source_ci_policy_no_update
BEFORE UPDATE ON source_ci_policies
BEGIN SELECT RAISE(ABORT,'source_ci_policy_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_ci_policy_no_replace
BEFORE INSERT ON source_ci_policies
WHEN EXISTS(SELECT 1 FROM source_ci_policies WHERE scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)
BEGIN SELECT RAISE(ABORT,'source_ci_policy_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_ci_policy_no_delete
BEFORE DELETE ON source_ci_policies
WHEN EXISTS(SELECT 1 FROM sentinel_scan_attempts WHERE scan_id=OLD.scan_id AND attempt_number=OLD.attempt_number)
BEGIN SELECT RAISE(ABORT,'source_ci_policy_immutable'); END;
CREATE TRIGGER IF NOT EXISTS source_ci_policy_current_attempt
BEFORE INSERT ON source_ci_policies
WHEN NOT EXISTS(SELECT 1 FROM sentinel_scans WHERE id=NEW.scan_id AND attempt_count=NEW.attempt_number
    AND status='scanning' AND scan_type='cicd')
BEGIN SELECT RAISE(ABORT,'source_ci_policy_scope_mismatch'); END;
CREATE TABLE IF NOT EXISTS source_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    root_path TEXT NOT NULL DEFAULT '',
    frozen_root TEXT NOT NULL DEFAULT '',
    changed_files_json TEXT NOT NULL DEFAULT 'null',
    scratch_dir TEXT NOT NULL DEFAULT '',
    commit_sha TEXT NOT NULL DEFAULT '',
    base_sha TEXT NOT NULL DEFAULT '',
    diff_base TEXT NOT NULL DEFAULT '',
    tree_hash TEXT NOT NULL DEFAULT '',
    file_count INTEGER NOT NULL DEFAULT 0,
    manifest_json TEXT NOT NULL DEFAULT '[]',
    languages_json TEXT NOT NULL DEFAULT '[]',
    gaps_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(scan_id,attempt_number,tree_hash)
);
CREATE INDEX IF NOT EXISTS idx_source_snapshots_scan ON source_snapshots(scan_id,attempt_number);
-- §10.2 items 5-10: one row per (engine, rule pack, snapshot, attempt) execution. The
-- unique invocation key is what keeps a crash recovery from paying for the same
-- analyzer run twice.
CREATE TABLE IF NOT EXISTS analyzer_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invocation_key TEXT NOT NULL UNIQUE,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL DEFAULT 1,
    engine TEXT NOT NULL,
    status TEXT NOT NULL,
    gap_code TEXT NOT NULL DEFAULT '',
    version TEXT NOT NULL DEFAULT '',
    rule_pack_digest TEXT NOT NULL DEFAULT '',
    image_digest TEXT NOT NULL DEFAULT '',
    network_disabled INTEGER NOT NULL DEFAULT 1,
    repository_read_only INTEGER NOT NULL DEFAULT 1,
    sarif_path TEXT NOT NULL DEFAULT '',
    stdout_truncated INTEGER NOT NULL DEFAULT 0,
    duration_millis INTEGER NOT NULL DEFAULT 0,
    args_json TEXT NOT NULL DEFAULT '[]',
    evidence_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_analyzer_runs_scan ON analyzer_runs(scan_id,attempt_number,engine);
-- Persist ownership BEFORE talking to the daemon. An unresolved receipt blocks a
-- retry; killing a client is not proof that its container has stopped.
CREATE TABLE IF NOT EXISTS analyzer_container_receipts (
    receipt_id TEXT PRIMARY KEY,
    invocation_key TEXT NOT NULL,
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    purpose TEXT NOT NULL,
    container_name TEXT NOT NULL UNIQUE,
    container_id TEXT NOT NULL DEFAULT '',
    owner_token TEXT NOT NULL,
    create_args_json TEXT NOT NULL,
    cleanup_status TEXT NOT NULL DEFAULT 'pending'
        CHECK(cleanup_status IN ('pending','confirmed','unconfirmed')),
    detail TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE INDEX IF NOT EXISTS idx_analyzer_container_receipts_scan
ON analyzer_container_receipts(scan_id,cleanup_status);
-- §10.2 item 1: the Native frozen plan, one per attempt. Rewriting it for an attempt
-- that already has one is refused in code, not silently overwritten.
CREATE TABLE IF NOT EXISTS native_scan_plans (
    scan_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    scan_type TEXT NOT NULL DEFAULT '',
    backend TEXT NOT NULL DEFAULT 'native',
    plan_json TEXT NOT NULL,
    plan_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    PRIMARY KEY(scan_id,attempt_number)
);
"#, include_str!("agent_runtime/web_mode/protection_receipt.sql"), include_str!("agent_runtime/web_mode/http_budget_receipt.sql"), include_str!("agent_runtime/web_mode/web_expiry_delivery.sql"), include_str!("agent_runtime/web_mode/capability_transition_receipt.sql"), include_str!("commands/agent_native/coordinator_authorization_output_receipt.sql"), include_str!("commands/agent_native/external_surface_response_receipt.sql"), include_str!("commands/agent_native/external_surface_output_receipt.sql"), include_str!("commands/agent_native/business_logic_output_receipt.sql"), include_str!("commands/agent_native/business_logic_dispatch_receipt.sql"), include_str!("agent_runtime/multi_agent/homogeneous/schema.sql"),include_str!("agent_runtime/multi_agent/homogeneous/parser_execution.sql"),include_str!("agent_runtime/multi_agent/homogeneous/execution_facts.sql"));
