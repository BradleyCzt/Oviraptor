-- Reserved for the actual new-scan private issuer. This leaf cannot mint rows
-- merely from caller-declared scope or the absence of an existing Root.
CREATE TABLE IF NOT EXISTS native_tool_selection_requests (
 request_id TEXT PRIMARY KEY NOT NULL CHECK(length(request_id)=36),
 scan_id TEXT NOT NULL,
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 scope_json TEXT NOT NULL,
 selection_json TEXT NOT NULL,
 inventory_json TEXT NOT NULL,
 configuration_digest TEXT NOT NULL CHECK(length(configuration_digest)=64),
 request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
 draft_id TEXT NOT NULL CHECK(length(draft_id)=36),
 stage TEXT NOT NULL CHECK(stage='draft_pending_original_mode_signature'),
 snapshot_json TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime')),
 UNIQUE(scan_id,attempt_number)
);
CREATE TRIGGER IF NOT EXISTS tool_selection_no_update BEFORE UPDATE ON native_tool_selection_requests BEGIN SELECT RAISE(ABORT,'tool_selection_request_immutable'); END;
CREATE TRIGGER IF NOT EXISTS tool_selection_no_delete BEFORE DELETE ON native_tool_selection_requests BEGIN SELECT RAISE(ABORT,'tool_selection_request_immutable'); END;
CREATE TRIGGER IF NOT EXISTS tool_selection_no_replace BEFORE INSERT ON native_tool_selection_requests WHEN EXISTS(SELECT 1 FROM native_tool_selection_requests WHERE request_id=NEW.request_id OR (scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number)) BEGIN SELECT RAISE(ABORT,'tool_selection_request_immutable'); END;
