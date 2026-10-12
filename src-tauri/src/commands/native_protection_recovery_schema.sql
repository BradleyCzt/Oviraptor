-- Independent explicit new-work approval. No old stop, bill, C or task update.
CREATE TABLE IF NOT EXISTS native_protection_recovery_approvals (
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
CREATE TRIGGER IF NOT EXISTS protection_recovery_approval_no_update
 BEFORE UPDATE ON native_protection_recovery_approvals
 BEGIN SELECT RAISE(ABORT,'protection_recovery_approval_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_approval_no_delete
 BEFORE DELETE ON native_protection_recovery_approvals
 BEGIN SELECT RAISE(ABORT,'protection_recovery_approval_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_approval_no_replace
 BEFORE INSERT ON native_protection_recovery_approvals
 WHEN EXISTS(SELECT 1 FROM native_protection_recovery_approvals
  WHERE rowid=NEW.rowid OR approval_id=NEW.approval_id OR request_id=NEW.request_id OR proposal_hash=NEW.proposal_hash
   OR (source_scan_id=NEW.source_scan_id AND source_root_run_id=NEW.source_root_run_id
       AND protection_receipt_id=NEW.protection_receipt_id))
 BEGIN SELECT RAISE(ABORT,'protection_recovery_approval_immutable'); END;

-- One approval can stage exactly one independent draft. This relation never
-- grants target execution or consumes/changes the original stop or budget.
CREATE TABLE IF NOT EXISTS native_protection_recovery_drafts (
 approval_id TEXT PRIMARY KEY CHECK(length(approval_id)=64),
 request_id TEXT NOT NULL UNIQUE,
 scan_id TEXT NOT NULL UNIQUE,
 draft_id TEXT NOT NULL UNIQUE,
 source_scan_id TEXT NOT NULL,
 proposal_hash TEXT NOT NULL CHECK(length(proposal_hash)=64),
 draft_json TEXT NOT NULL CHECK(json_valid(draft_json)),
 signature BLOB NOT NULL CHECK(length(signature)=32)
);
CREATE TRIGGER IF NOT EXISTS protection_recovery_draft_no_update
 BEFORE UPDATE ON native_protection_recovery_drafts
 BEGIN SELECT RAISE(ABORT,'protection_recovery_draft_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_draft_no_delete
 BEFORE DELETE ON native_protection_recovery_drafts
 BEGIN SELECT RAISE(ABORT,'protection_recovery_draft_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_draft_no_replace
 BEFORE INSERT ON native_protection_recovery_drafts
 WHEN EXISTS(SELECT 1 FROM native_protection_recovery_drafts
  WHERE rowid=NEW.rowid OR approval_id=NEW.approval_id OR request_id=NEW.request_id
   OR scan_id=NEW.scan_id OR draft_id=NEW.draft_id)
 BEGIN SELECT RAISE(ABORT,'protection_recovery_draft_immutable'); END;

-- This pending total is covered by the original draft signature. No clock,
-- Root allocation, reservation, SDK or target grant is created by this row.
CREATE TABLE IF NOT EXISTS native_protection_recovery_scope_budgets (
 owner_id TEXT PRIMARY KEY CHECK(length(owner_id)=64),
 scan_id TEXT NOT NULL UNIQUE,
 draft_id TEXT NOT NULL UNIQUE,
 contract_json TEXT NOT NULL CHECK(json_valid(contract_json)),
 contract_hash TEXT NOT NULL CHECK(length(contract_hash)=64)
);
CREATE TRIGGER IF NOT EXISTS protection_recovery_scope_budget_no_update
 BEFORE UPDATE ON native_protection_recovery_scope_budgets
 BEGIN SELECT RAISE(ABORT,'protection_recovery_scope_budget_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_scope_budget_no_delete
 BEFORE DELETE ON native_protection_recovery_scope_budgets
 BEGIN SELECT RAISE(ABORT,'protection_recovery_scope_budget_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_scope_budget_no_replace
 BEFORE INSERT ON native_protection_recovery_scope_budgets
 WHEN EXISTS(SELECT 1 FROM native_protection_recovery_scope_budgets
  WHERE rowid=NEW.rowid OR owner_id=NEW.owner_id OR scan_id=NEW.scan_id OR draft_id=NEW.draft_id)
 BEGIN SELECT RAISE(ABORT,'protection_recovery_scope_budget_immutable'); END;

-- Retain a cancellation of unconsumed recovery authority; never delete its
-- signed draft, targets, approval or original paid history.
CREATE TABLE IF NOT EXISTS native_protection_recovery_cancellations (
 scan_id TEXT PRIMARY KEY,
 cancellation_id TEXT NOT NULL UNIQUE,
 approval_id TEXT NOT NULL UNIQUE CHECK(length(approval_id)=64),
 request_id TEXT NOT NULL UNIQUE,
 draft_hash TEXT NOT NULL CHECK(length(draft_hash)=64),
 cancellation_json TEXT NOT NULL CHECK(json_valid(cancellation_json)),
 signature BLOB NOT NULL CHECK(length(signature)=32)
);
CREATE TRIGGER IF NOT EXISTS protection_recovery_cancellation_no_update
 BEFORE UPDATE ON native_protection_recovery_cancellations
 BEGIN SELECT RAISE(ABORT,'protection_recovery_cancellation_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_cancellation_no_delete
 BEFORE DELETE ON native_protection_recovery_cancellations
 BEGIN SELECT RAISE(ABORT,'protection_recovery_cancellation_immutable'); END;
CREATE TRIGGER IF NOT EXISTS protection_recovery_cancellation_no_replace
 BEFORE INSERT ON native_protection_recovery_cancellations
 WHEN EXISTS(SELECT 1 FROM native_protection_recovery_cancellations
  WHERE rowid=NEW.rowid OR scan_id=NEW.scan_id OR cancellation_id=NEW.cancellation_id
   OR approval_id=NEW.approval_id OR request_id=NEW.request_id)
 BEGIN SELECT RAISE(ABORT,'protection_recovery_cancellation_immutable'); END;
