-- Human conversations have their own identity and lifecycle. These tables do
-- not create a scan/run, and no FK points from a conversation into evidence.
CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE RESTRICT,
    create_operation_id TEXT NOT NULL,
    create_title TEXT NOT NULL,
    title TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0,1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision BETWEEN 1 AND 9007199254740991),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(project_id,create_operation_id)
);
CREATE INDEX IF NOT EXISTS idx_conversations_project ON conversations(project_id,archived,updated_at DESC,id);
CREATE TABLE IF NOT EXISTS conversation_messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL CHECK(sequence BETWEEN 1 AND 9007199254740991),
    role TEXT NOT NULL CHECK(role IN ('user','assistant')),
    content TEXT NOT NULL CHECK(length(content)>0),
    operation_id TEXT,
    request_reply INTEGER NOT NULL DEFAULT 0 CHECK(request_reply IN (0,1)),
    reply_to_message_id TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(conversation_id,sequence),
    UNIQUE(conversation_id,operation_id)
);
CREATE TABLE IF NOT EXISTS conversation_links (
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('scan','asset')),
    target_id TEXT NOT NULL,
    PRIMARY KEY(conversation_id,kind,target_id)
);
CREATE TABLE IF NOT EXISTS conversation_replies (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    user_message_id TEXT NOT NULL UNIQUE REFERENCES conversation_messages(id) ON DELETE CASCADE,
    base_revision INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('queued','running','completed','unavailable','failed','cancel_requested','cancelled','interrupted')),
    code TEXT NOT NULL DEFAULT '',
    message TEXT NOT NULL DEFAULT '',
    assistant_message_id TEXT,
    model TEXT NOT NULL DEFAULT '',
    usage_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(usage_json)),
    context_hash TEXT NOT NULL DEFAULT '',
    outcome_unknown INTEGER NOT NULL DEFAULT 0 CHECK(outcome_unknown IN (0,1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_conversation_one_active_reply ON conversation_replies(conversation_id)
    WHERE status IN ('queued','running','cancel_requested');
CREATE INDEX IF NOT EXISTS idx_conversation_reply_history ON conversation_replies(conversation_id,created_at,id);
