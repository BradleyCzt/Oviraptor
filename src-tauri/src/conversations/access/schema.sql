-- Explicit, local UI grants. A model tool can never create or expand these.
CREATE TABLE IF NOT EXISTS conversation_access (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision BETWEEN 1 AND 9007199254740991),
    agent_enabled INTEGER NOT NULL DEFAULT 1 CHECK(agent_enabled IN (0,1)),
    read_assets INTEGER NOT NULL DEFAULT 0 CHECK(read_assets IN (0,1)),
    read_scans INTEGER NOT NULL DEFAULT 0 CHECK(read_scans IN (0,1)),
    selected_skill_ids TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(selected_skill_ids)),
    provider_disclosure_accepted INTEGER NOT NULL DEFAULT 0 CHECK(provider_disclosure_accepted IN (0,1)),
    provider_identity TEXT NOT NULL DEFAULT '',
    selected_skill_fingerprints TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(selected_skill_fingerprints))
);
CREATE TABLE IF NOT EXISTS conversation_directory_grants (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    canonical_root TEXT NOT NULL,
    root_device TEXT NOT NULL,
    root_inode TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(conversation_id,canonical_root)
);
