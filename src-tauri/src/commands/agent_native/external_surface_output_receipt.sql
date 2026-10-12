CREATE TABLE IF NOT EXISTS agent_external_surface_output_receipts (
 receipt_id TEXT PRIMARY KEY CHECK(length(receipt_id)=64),
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 child_run_id TEXT NOT NULL,
 worker_attempt_id TEXT NOT NULL UNIQUE,
 message_id TEXT NOT NULL UNIQUE,
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64),
 signature BLOB NOT NULL CHECK(length(signature)=32)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS external_surface_output_no_update BEFORE UPDATE ON agent_external_surface_output_receipts BEGIN SELECT RAISE(ABORT,'external_surface_output_immutable'); END;
CREATE TRIGGER IF NOT EXISTS external_surface_output_no_delete BEFORE DELETE ON agent_external_surface_output_receipts BEGIN SELECT RAISE(ABORT,'external_surface_output_immutable'); END;
CREATE TRIGGER IF NOT EXISTS external_surface_output_no_replace BEFORE INSERT ON agent_external_surface_output_receipts WHEN EXISTS(SELECT 1 FROM agent_external_surface_output_receipts WHERE receipt_id=NEW.receipt_id OR worker_attempt_id=NEW.worker_attempt_id OR message_id=NEW.message_id) BEGIN SELECT RAISE(ABORT,'external_surface_output_immutable'); END;
