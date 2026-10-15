CREATE TABLE IF NOT EXISTS asset_page_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('title_contains','body_sha256')),
    value TEXT NOT NULL,
    source_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK(enabled IN (0,1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now','localtime')),
    UNIQUE(project_id,kind,value)
);
CREATE INDEX IF NOT EXISTS idx_asset_page_rules_project ON asset_page_rules(project_id,enabled);
CREATE TABLE IF NOT EXISTS asset_page_rule_effects (
    project_id INTEGER NOT NULL,
    asset_id INTEGER NOT NULL,
    rule_id INTEGER NOT NULL REFERENCES asset_page_rules(id),
    original_decision TEXT NOT NULL,
    original_note TEXT NOT NULL,
    original_deleted_at TEXT,
    applied_note TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)),
    PRIMARY KEY(project_id,asset_id),
    FOREIGN KEY(project_id,asset_id) REFERENCES project_assets(project_id,asset_id) ON DELETE CASCADE
);
