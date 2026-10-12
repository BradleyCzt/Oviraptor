-- First actual protection response only. No execution, refund or repair grant.
CREATE TABLE IF NOT EXISTS agent_protection_response_receipts (
 storage_id INTEGER PRIMARY KEY CHECK(storage_id>0),
 receipt_id TEXT NOT NULL UNIQUE CHECK(length(receipt_id)>0),
 root_run_id TEXT NOT NULL CHECK(length(root_run_id)>0),
 scan_id TEXT NOT NULL CHECK(length(scan_id)>0),
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 target_url TEXT NOT NULL CHECK(length(target_url)>0),
 run_id TEXT NOT NULL CHECK(length(run_id)>0),
 claim_rowid INTEGER NOT NULL CHECK(claim_rowid>0),
 worker_attempt_id TEXT NOT NULL CHECK(length(worker_attempt_id)>0),
 assignment_id TEXT NOT NULL CHECK(length(assignment_id)>0),
 artifact_id TEXT NOT NULL CHECK(length(artifact_id)>0),
 response_status INTEGER NOT NULL CHECK(response_status BETWEEN 100 AND 599 AND response_status<>404),
 protection_code TEXT NOT NULL CHECK(protection_code='confirmed_waf_or_challenge'),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64),
 signature BLOB NOT NULL CHECK(length(signature)=32),
 created_at TEXT NOT NULL CHECK(length(created_at)>0),
 UNIQUE(scan_id,target_url),
 UNIQUE(root_run_id,claim_rowid)
);
CREATE TRIGGER IF NOT EXISTS protection_response_receipt_no_update
 BEFORE UPDATE ON agent_protection_response_receipts
 BEGIN SELECT RAISE(ABORT,'protection_response_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_response_receipt_no_delete
 BEFORE DELETE ON agent_protection_response_receipts
 BEGIN SELECT RAISE(ABORT,'protection_response_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_response_receipt_no_replace
 BEFORE INSERT ON agent_protection_response_receipts
 WHEN EXISTS(SELECT 1 FROM agent_protection_response_receipts old
  WHERE old.storage_id=NEW.storage_id OR old.rowid=NEW.rowid OR old.receipt_id=NEW.receipt_id
   OR (old.scan_id=NEW.scan_id AND old.target_url=NEW.target_url)
   OR (old.root_run_id=NEW.root_run_id AND old.claim_rowid=NEW.claim_rowid))
 BEGIN SELECT RAISE(ABORT,'protection_response_receipt_immutable'); END;
