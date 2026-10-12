CREATE TABLE IF NOT EXISTS agent_mapper_loop_journal (
 call_id TEXT NOT NULL CHECK(length(call_id)=64),
 phase TEXT NOT NULL CHECK(phase IN ('dispatch','invoice','received','uncertain','unsent','lookup')),
 root_run_id TEXT NOT NULL,
 assignment_id TEXT NOT NULL,
 child_run_id TEXT NOT NULL,
 worker_attempt_id TEXT NOT NULL,
 round_number INTEGER NOT NULL CHECK(round_number BETWEEN 1 AND 2),
 request_hash TEXT NOT NULL CHECK(length(request_hash)=64),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64),
 PRIMARY KEY(call_id,phase),
 UNIQUE(child_run_id,round_number,phase)
);
CREATE TRIGGER IF NOT EXISTS mapper_loop_journal_no_update BEFORE UPDATE ON agent_mapper_loop_journal BEGIN SELECT RAISE(ABORT,'mapper_loop_original_immutable'); END;
CREATE TRIGGER IF NOT EXISTS mapper_loop_journal_no_delete BEFORE DELETE ON agent_mapper_loop_journal BEGIN SELECT RAISE(ABORT,'mapper_loop_original_immutable'); END;
CREATE TRIGGER IF NOT EXISTS mapper_loop_journal_no_replace BEFORE INSERT ON agent_mapper_loop_journal WHEN EXISTS(SELECT 1 FROM agent_mapper_loop_journal WHERE (call_id=NEW.call_id AND phase=NEW.phase) OR (child_run_id=NEW.child_run_id AND round_number=NEW.round_number AND phase=NEW.phase)) BEGIN SELECT RAISE(ABORT,'mapper_loop_original_immutable'); END;
