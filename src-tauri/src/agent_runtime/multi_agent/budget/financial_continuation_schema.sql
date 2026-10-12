CREATE TABLE IF NOT EXISTS agent_financial_continuations (
 approval_id TEXT PRIMARY KEY,
 proposal_hash TEXT NOT NULL UNIQUE CHECK(length(proposal_hash)=64),
 scan_id TEXT NOT NULL,
 source_attempt_number INTEGER NOT NULL CHECK(source_attempt_number>0),
 new_attempt_number INTEGER NOT NULL CHECK(new_attempt_number>source_attempt_number),
 source_root_run_id TEXT NOT NULL UNIQUE,
 new_root_run_id TEXT NOT NULL UNIQUE,
 source_receipt_id TEXT NOT NULL CHECK(length(source_receipt_id)=64),
 fact_json TEXT NOT NULL,
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64),
 created_at TEXT NOT NULL DEFAULT (datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS financial_continuation_no_update
 BEFORE UPDATE ON agent_financial_continuations
 BEGIN SELECT RAISE(ABORT,'financial_continuation_immutable'); END;
CREATE TRIGGER IF NOT EXISTS financial_continuation_no_delete
 BEFORE DELETE ON agent_financial_continuations
 BEGIN SELECT RAISE(ABORT,'financial_continuation_immutable'); END;
CREATE TRIGGER IF NOT EXISTS financial_continuation_no_replace
 BEFORE INSERT ON agent_financial_continuations
 WHEN EXISTS(SELECT 1 FROM agent_financial_continuations
  WHERE approval_id=NEW.approval_id OR proposal_hash=NEW.proposal_hash
   OR source_root_run_id=NEW.source_root_run_id OR new_root_run_id=NEW.new_root_run_id)
 BEGIN SELECT RAISE(ABORT,'financial_continuation_immutable'); END;
CREATE TABLE IF NOT EXISTS agent_financial_startup_bindings (
 scan_id TEXT NOT NULL,
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 approval_id TEXT NOT NULL UNIQUE,
 binding_tag BLOB NOT NULL CHECK(typeof(binding_tag)='blob' AND length(binding_tag)=32),
 binding_fact_json TEXT NOT NULL CHECK(json_valid(binding_fact_json)),
 PRIMARY KEY(scan_id,attempt_number),
 FOREIGN KEY(scan_id,attempt_number) REFERENCES sentinel_scan_attempts(scan_id,attempt_number)
);
CREATE TRIGGER IF NOT EXISTS financial_startup_no_update BEFORE UPDATE ON agent_financial_startup_bindings
 BEGIN SELECT RAISE(ABORT,'financial_startup_immutable'); END;
CREATE TRIGGER IF NOT EXISTS financial_startup_no_delete BEFORE DELETE ON agent_financial_startup_bindings
 BEGIN SELECT RAISE(ABORT,'financial_startup_immutable'); END;
CREATE TRIGGER IF NOT EXISTS financial_startup_no_replace BEFORE INSERT ON agent_financial_startup_bindings
 WHEN EXISTS(SELECT 1 FROM agent_financial_startup_bindings
  WHERE (scan_id=NEW.scan_id AND attempt_number=NEW.attempt_number) OR approval_id=NEW.approval_id)
 BEGIN SELECT RAISE(ABORT,'financial_startup_immutable'); END;
