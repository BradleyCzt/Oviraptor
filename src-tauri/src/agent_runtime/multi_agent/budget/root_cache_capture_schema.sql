-- Fresh capture authentication. Never backfill historical signatures.
CREATE TABLE IF NOT EXISTS agent_root_model_cache_capture (
 call_id TEXT PRIMARY KEY,
 signature BLOB NOT NULL CHECK(length(signature)=32)
);
CREATE TRIGGER IF NOT EXISTS root_cache_capture_no_update BEFORE UPDATE ON agent_root_model_cache_capture
 BEGIN SELECT RAISE(ABORT,'root_cache_capture_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_cache_capture_no_delete BEFORE DELETE ON agent_root_model_cache_capture
 BEGIN SELECT RAISE(ABORT,'root_cache_capture_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_cache_capture_no_replace BEFORE INSERT ON agent_root_model_cache_capture
 WHEN EXISTS(SELECT 1 FROM agent_root_model_cache_capture WHERE call_id=NEW.call_id)
 BEGIN SELECT RAISE(ABORT,'root_cache_capture_immutable'); END;
