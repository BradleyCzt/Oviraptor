-- Exact historical table statements from 525f1283adae79a13bcf0e07cf50772841911fb0:src-tauri/src/db_schema.rs
-- Git source blob: 8ff5a1e8b5f5c9f7dc7ec85f88419a9ee9fbdc75. Synthetic test rows are inserted by the Rust tests.
-- Other application tables are created by the real db::initialize path.

CREATE TABLE IF NOT EXISTS projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'active',
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

CREATE TABLE IF NOT EXISTS asset_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    run_id INTEGER REFERENCES runs(id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
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
