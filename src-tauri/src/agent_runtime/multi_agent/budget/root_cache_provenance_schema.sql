-- Additive original transport metadata. No historical backfill or fee grant.
CREATE TABLE IF NOT EXISTS agent_root_model_cache_provenance (
 call_id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL,
 lease_attempt_id TEXT NOT NULL,
 round INTEGER NOT NULL CHECK(round>0),
 request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
 response_hash TEXT NOT NULL CHECK(length(response_hash)=64),
 cached_usage_reported INTEGER NOT NULL CHECK(cached_usage_reported IN (0,1)),
 UNIQUE(root_run_id,round)
);
CREATE TRIGGER IF NOT EXISTS root_cache_provenance_no_update BEFORE UPDATE ON agent_root_model_cache_provenance
 BEGIN SELECT RAISE(ABORT,'root_cache_provenance_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_cache_provenance_no_delete BEFORE DELETE ON agent_root_model_cache_provenance
 BEGIN SELECT RAISE(ABORT,'root_cache_provenance_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_cache_provenance_no_replace BEFORE INSERT ON agent_root_model_cache_provenance
 WHEN EXISTS(SELECT 1 FROM agent_root_model_cache_provenance WHERE call_id=NEW.call_id OR (root_run_id=NEW.root_run_id AND round=NEW.round))
 BEGIN SELECT RAISE(ABORT,'root_cache_provenance_immutable'); END;
