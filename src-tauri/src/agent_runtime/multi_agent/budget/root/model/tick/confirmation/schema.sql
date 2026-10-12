-- Independent human confirmation of original Root fees; grants no execution.
CREATE TABLE IF NOT EXISTS agent_root_cost_reconciliation_receipts (
 receipt_id TEXT PRIMARY KEY CHECK(length(receipt_id)=64),
 scan_id TEXT NOT NULL,
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 root_run_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(root_run_id) ON DELETE RESTRICT,
 call_id TEXT NOT NULL UNIQUE,
 lease_attempt_id TEXT NOT NULL REFERENCES agent_root_budget_attempts(id) ON DELETE RESTRICT,
 invoice_hash TEXT NOT NULL CHECK(length(invoice_hash)=64),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 created_at TEXT NOT NULL DEFAULT(datetime('now','localtime'))
);
CREATE TRIGGER IF NOT EXISTS root_cost_reconciliation_no_update BEFORE UPDATE ON agent_root_cost_reconciliation_receipts
 BEGIN SELECT RAISE(ABORT,'root_cost_reconciliation_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_cost_reconciliation_no_delete BEFORE DELETE ON agent_root_cost_reconciliation_receipts
 BEGIN SELECT RAISE(ABORT,'root_cost_reconciliation_immutable'); END;
CREATE TRIGGER IF NOT EXISTS root_cost_reconciliation_no_replace BEFORE INSERT ON agent_root_cost_reconciliation_receipts
 WHEN EXISTS(SELECT 1 FROM agent_root_cost_reconciliation_receipts WHERE receipt_id=NEW.receipt_id OR call_id=NEW.call_id)
 BEGIN SELECT RAISE(ABORT,'root_cost_reconciliation_immutable'); END;
