CREATE TABLE IF NOT EXISTS agent_mapper_loop_final_receipts (
 receipt_id TEXT NOT NULL PRIMARY KEY CHECK(length(receipt_id)=64),
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 child_run_id TEXT NOT NULL,
 worker_attempt_id TEXT NOT NULL UNIQUE,
 message_id TEXT NOT NULL UNIQUE,
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64),
 signature BLOB NOT NULL CHECK(length(signature)=32)
);
CREATE TRIGGER IF NOT EXISTS mapper_loop_final_no_update BEFORE UPDATE ON agent_mapper_loop_final_receipts BEGIN SELECT RAISE(ABORT,'mapper_loop_final_immutable'); END;
CREATE TRIGGER IF NOT EXISTS mapper_loop_final_no_delete BEFORE DELETE ON agent_mapper_loop_final_receipts BEGIN SELECT RAISE(ABORT,'mapper_loop_final_immutable'); END;
CREATE TRIGGER IF NOT EXISTS mapper_loop_final_no_replace BEFORE INSERT ON agent_mapper_loop_final_receipts WHEN EXISTS(SELECT 1 FROM agent_mapper_loop_final_receipts WHERE receipt_id=NEW.receipt_id OR worker_attempt_id=NEW.worker_attempt_id OR message_id=NEW.message_id) BEGIN SELECT RAISE(ABORT,'mapper_loop_final_immutable'); END;
