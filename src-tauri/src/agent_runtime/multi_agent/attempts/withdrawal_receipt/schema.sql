-- Audit of a new protected withdrawal, never SDK exit or execution authority.
CREATE TABLE IF NOT EXISTS agent_worker_expiry_receipts (
 receipt_id TEXT PRIMARY KEY,
 worker_attempt_id TEXT NOT NULL UNIQUE,
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 child_run_id TEXT NOT NULL,
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64),
 created_at TEXT NOT NULL DEFAULT(strftime('%Y-%m-%d %H:%M:%f','now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS worker_expiry_receipt_no_update BEFORE UPDATE ON agent_worker_expiry_receipts
 BEGIN SELECT RAISE(ABORT,'worker_expiry_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS worker_expiry_receipt_no_delete BEFORE DELETE ON agent_worker_expiry_receipts
 BEGIN SELECT RAISE(ABORT,'worker_expiry_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS worker_expiry_receipt_no_replace BEFORE INSERT ON agent_worker_expiry_receipts
 WHEN EXISTS(SELECT 1 FROM agent_worker_expiry_receipts
 WHERE rowid=NEW.rowid OR receipt_id=NEW.receipt_id OR worker_attempt_id=NEW.worker_attempt_id)
 BEGIN SELECT RAISE(ABORT,'worker_expiry_receipt_immutable'); END;

-- Companion seal leaves pre-existing unsigned audit rows byte-for-byte intact.
CREATE TABLE IF NOT EXISTS agent_worker_expiry_issuance_seals (
 receipt_id TEXT PRIMARY KEY,
 root_run_id TEXT NOT NULL,
 binding_json TEXT NOT NULL CHECK(json_valid(binding_json)),
 signature BLOB NOT NULL CHECK(length(signature)=32)
);
CREATE TRIGGER IF NOT EXISTS worker_expiry_seal_no_update BEFORE UPDATE ON agent_worker_expiry_issuance_seals BEGIN SELECT RAISE(ABORT,'worker_expiry_seal_immutable'); END;
CREATE TRIGGER IF NOT EXISTS worker_expiry_seal_no_delete BEFORE DELETE ON agent_worker_expiry_issuance_seals BEGIN SELECT RAISE(ABORT,'worker_expiry_seal_immutable'); END;
CREATE TRIGGER IF NOT EXISTS worker_expiry_seal_no_replace BEFORE INSERT ON agent_worker_expiry_issuance_seals
 WHEN EXISTS(SELECT 1 FROM agent_worker_expiry_issuance_seals WHERE receipt_id=NEW.receipt_id OR rowid=NEW.rowid) BEGIN SELECT RAISE(ABORT,'worker_expiry_seal_immutable'); END;
