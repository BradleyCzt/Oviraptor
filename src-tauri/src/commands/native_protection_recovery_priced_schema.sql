-- New priced approval protocol. Old unpriced approvals are never upgraded.
-- This approval alone creates no draft, clock, Root, balance or target grant.
CREATE TABLE IF NOT EXISTS native_protection_recovery_priced_approvals (
 approval_id TEXT PRIMARY KEY CHECK(length(approval_id)=64),
 request_id TEXT NOT NULL UNIQUE,
 source_scan_id TEXT NOT NULL,
 source_attempt_number INTEGER NOT NULL CHECK(source_attempt_number>0),
 source_root_run_id TEXT NOT NULL,
 protection_receipt_id TEXT NOT NULL,
 proposal_hash TEXT NOT NULL UNIQUE CHECK(length(proposal_hash)=64),
 approval_json TEXT NOT NULL CHECK(json_valid(approval_json)),
 signature BLOB NOT NULL CHECK(length(signature)=32),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime')),
 UNIQUE(source_scan_id,source_root_run_id,protection_receipt_id)
);
CREATE TRIGGER IF NOT EXISTS protection_recovery_priced_approval_no_update
 BEFORE UPDATE ON native_protection_recovery_priced_approvals
 BEGIN SELECT RAISE(ABORT,'protection_recovery_priced_approval_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_priced_approval_no_delete
 BEFORE DELETE ON native_protection_recovery_priced_approvals
 BEGIN SELECT RAISE(ABORT,'protection_recovery_priced_approval_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_priced_approval_no_replace
 BEFORE INSERT ON native_protection_recovery_priced_approvals
 WHEN EXISTS(SELECT 1 FROM native_protection_recovery_priced_approvals
  WHERE rowid=NEW.rowid OR approval_id=NEW.approval_id OR request_id=NEW.request_id
   OR proposal_hash=NEW.proposal_hash OR (source_scan_id=NEW.source_scan_id
    AND source_root_run_id=NEW.source_root_run_id AND protection_receipt_id=NEW.protection_receipt_id))
 BEGIN SELECT RAISE(ABORT,'protection_recovery_priced_approval_immutable'); END;

-- Separate immutable revocation; the original approval remains byte-for-byte.
CREATE TABLE IF NOT EXISTS native_protection_recovery_priced_cancellations (
 cancellation_id TEXT PRIMARY KEY CHECK(length(cancellation_id)=64),
 request_id TEXT NOT NULL UNIQUE,
 approval_id TEXT NOT NULL UNIQUE CHECK(length(approval_id)=64),
 cancellation_json TEXT NOT NULL CHECK(json_valid(cancellation_json)),
 signature BLOB NOT NULL CHECK(length(signature)=32),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS protection_recovery_priced_cancellation_no_update
 BEFORE UPDATE ON native_protection_recovery_priced_cancellations
 BEGIN SELECT RAISE(ABORT,'protection_recovery_priced_cancellation_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_priced_cancellation_no_delete
 BEFORE DELETE ON native_protection_recovery_priced_cancellations
 BEGIN SELECT RAISE(ABORT,'protection_recovery_priced_cancellation_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_priced_cancellation_no_replace
 BEFORE INSERT ON native_protection_recovery_priced_cancellations
 WHEN EXISTS(SELECT 1 FROM native_protection_recovery_priced_cancellations
  WHERE rowid=NEW.rowid OR cancellation_id=NEW.cancellation_id
   OR request_id=NEW.request_id OR approval_id=NEW.approval_id)
 BEGIN SELECT RAISE(ABORT,'protection_recovery_priced_cancellation_immutable'); END;
