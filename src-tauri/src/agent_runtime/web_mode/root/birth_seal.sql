-- Issued only during a new Root birth, never reconstructed from history.
CREATE TABLE IF NOT EXISTS agent_root_mode_birth_seals (
 root_run_id TEXT PRIMARY KEY,
 binding_json TEXT NOT NULL CHECK(json_valid(binding_json)),
 signature BLOB NOT NULL CHECK(length(signature)=32)
);
CREATE TRIGGER IF NOT EXISTS root_mode_birth_no_update BEFORE UPDATE ON agent_root_mode_birth_seals BEGIN SELECT RAISE(ABORT,'root_mode_birth_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_mode_birth_no_delete BEFORE DELETE ON agent_root_mode_birth_seals BEGIN SELECT RAISE(ABORT,'root_mode_birth_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_mode_birth_no_replace BEFORE INSERT ON agent_root_mode_birth_seals
 WHEN EXISTS(SELECT 1 FROM agent_root_mode_birth_seals WHERE root_run_id=NEW.root_run_id OR rowid=NEW.rowid) BEGIN SELECT RAISE(ABORT,'root_mode_birth_immutable'); END;
