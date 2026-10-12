CREATE TABLE IF NOT EXISTS agent_public_surface_response_receipts (
 receipt_id TEXT PRIMARY KEY CHECK(length(receipt_id)=64),
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL UNIQUE,
 child_run_id TEXT NOT NULL UNIQUE,
 worker_attempt_id TEXT NOT NULL UNIQUE,
 claim_rowid INTEGER NOT NULL UNIQUE CHECK(claim_rowid>0),
 artifact_id TEXT NOT NULL,
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64),
 signature BLOB NOT NULL CHECK(length(signature)=32)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS public_surface_response_no_update BEFORE UPDATE ON agent_public_surface_response_receipts BEGIN SELECT RAISE(ABORT,'public_surface_response_immutable'); END;
CREATE TRIGGER IF NOT EXISTS public_surface_response_no_delete BEFORE DELETE ON agent_public_surface_response_receipts BEGIN SELECT RAISE(ABORT,'public_surface_response_immutable'); END;
CREATE TRIGGER IF NOT EXISTS public_surface_response_no_replace BEFORE INSERT ON agent_public_surface_response_receipts WHEN EXISTS(SELECT 1 FROM agent_public_surface_response_receipts WHERE receipt_id=NEW.receipt_id OR assignment_id=NEW.assignment_id OR child_run_id=NEW.child_run_id OR worker_attempt_id=NEW.worker_attempt_id OR claim_rowid=NEW.claim_rowid) BEGIN SELECT RAISE(ABORT,'public_surface_response_immutable'); END;
