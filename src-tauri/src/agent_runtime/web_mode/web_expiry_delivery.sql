-- Original paused Web expiry delivery only. Never ordinary completion.
CREATE TABLE IF NOT EXISTS agent_web_expiry_deliveries (
 receipt_id TEXT PRIMARY KEY NOT NULL CHECK(length(receipt_id)=64),
 root_run_id TEXT NOT NULL,
 worker_attempt_id TEXT NOT NULL UNIQUE,
 assignment_id TEXT NOT NULL,
 child_run_id TEXT NOT NULL,
 expiry_receipt_id TEXT NOT NULL UNIQUE,
 message_id TEXT NOT NULL UNIQUE,
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64 AND receipt_id=fact_hash),
 signature BLOB NOT NULL CHECK(typeof(signature)='blob' AND length(signature)=32)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS web_expiry_delivery_no_update BEFORE UPDATE ON agent_web_expiry_deliveries BEGIN SELECT RAISE(ABORT,'web_expiry_delivery_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_expiry_delivery_no_delete BEFORE DELETE ON agent_web_expiry_deliveries BEGIN SELECT RAISE(ABORT,'web_expiry_delivery_immutable'); END;
CREATE TRIGGER IF NOT EXISTS web_expiry_delivery_no_replace BEFORE INSERT ON agent_web_expiry_deliveries WHEN EXISTS(SELECT 1 FROM agent_web_expiry_deliveries old WHERE old.receipt_id=NEW.receipt_id OR old.worker_attempt_id=NEW.worker_attempt_id OR old.expiry_receipt_id=NEW.expiry_receipt_id OR old.message_id=NEW.message_id) BEGIN SELECT RAISE(ABORT,'web_expiry_delivery_immutable'); END;
